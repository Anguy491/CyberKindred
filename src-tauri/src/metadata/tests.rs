use super::*;
use chrono::{DateTime, Utc};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct FakeClock(Arc<AtomicI64>);

impl FakeClock {
    fn new(now_ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now_ms)))
    }

    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.0.load(Ordering::SeqCst))
            .expect("fake timestamp is valid")
    }
}

struct FakeSleeper {
    clock: FakeClock,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl MetadataSleeper for FakeSleeper {
    fn sleep(&self, duration: Duration) -> MetadataFuture<'_, ()> {
        self.sleeps
            .lock()
            .expect("sleep capture lock")
            .push(duration);
        self.clock
            .advance(i64::try_from(duration.as_millis()).expect("bounded duration"));
        Box::pin(async {})
    }
}

#[derive(Clone, Debug)]
enum CapturedRequest {
    Metadata(MusicBrainzRequest, Duration),
    Cover(CoverArtRequest, Duration),
}

struct FakeTransport {
    metadata: Mutex<VecDeque<Result<MetadataTransportResult, ProviderFailure>>>,
    covers: Mutex<VecDeque<Result<CoverTransportResult, ProviderFailure>>>,
    captured: Mutex<Vec<CapturedRequest>>,
    calls: AtomicUsize,
}

impl FakeTransport {
    fn new(
        metadata: Vec<Result<MetadataTransportResult, ProviderFailure>>,
        covers: Vec<Result<CoverTransportResult, ProviderFailure>>,
    ) -> Self {
        Self {
            metadata: Mutex::new(metadata.into()),
            covers: Mutex::new(covers.into()),
            captured: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }
}

impl MetadataTransport for FakeTransport {
    fn search<'a>(
        &'a self,
        request: &'a MusicBrainzRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<MetadataTransportResult, ProviderFailure>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.captured
            .lock()
            .expect("capture lock")
            .push(CapturedRequest::Metadata(request.clone(), timeout));
        let response = self
            .metadata
            .lock()
            .expect("metadata response lock")
            .pop_front()
            .unwrap_or_else(|| Err(unavailable()));
        Box::pin(async move { response })
    }

    fn fetch_cover<'a>(
        &'a self,
        request: &'a CoverArtRequest,
        timeout: Duration,
    ) -> MetadataFuture<'a, Result<CoverTransportResult, ProviderFailure>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.captured
            .lock()
            .expect("capture lock")
            .push(CapturedRequest::Cover(request.clone(), timeout));
        let response = self
            .covers
            .lock()
            .expect("cover response lock")
            .pop_front()
            .unwrap_or_else(|| Err(unavailable()));
        Box::pin(async move { response })
    }
}

fn query(title: &str) -> MetadataQuery {
    MetadataQuery::new(
        Some(title.to_owned()),
        vec!["Alice / Bob".to_owned()],
        Some("Local Album".to_owned()),
        Some(183_000),
    )
    .expect("valid query")
}

fn candidate(confidence: f64, title: &str, artist: &str) -> MetadataMatch {
    MetadataMatch {
        recording_mbid: Uuid::from_u128(1),
        release_mbid: Some(Uuid::from_u128(2)),
        title: title.to_owned(),
        artists: vec![artist.to_owned()],
        album: Some("Enriched Album".to_owned()),
        tags: vec!["ambient".to_owned()],
        confidence,
    }
}

fn provider(
    transport: Arc<FakeTransport>,
    clock: &FakeClock,
    limiter: Arc<GlobalMetadataRateLimiter>,
    match_capacity: usize,
) -> CachedMetadataProvider {
    CachedMetadataProvider::new(
        transport,
        MetadataUserAgent::new("0.1.0", "https://example.invalid/cyberkindred")
            .expect("valid user agent"),
        limiter,
        Arc::new(MetadataCache::new(match_capacity, 4, 6 * 1_024 * 1_024)),
        Arc::new(clock.clone()),
    )
}

fn limiter(clock: &FakeClock) -> (Arc<GlobalMetadataRateLimiter>, Arc<Mutex<Vec<Duration>>>) {
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let sleeper = FakeSleeper {
        clock: clock.clone(),
        sleeps: Arc::clone(&sleeps),
    };
    (
        Arc::new(GlobalMetadataRateLimiter::new(
            Arc::new(clock.clone()),
            Arc::new(sleeper),
        )),
        sleeps,
    )
}

#[tokio::test]
async fn metadata_provider_allowlists_normalized_text_and_explicit_user_agent() {
    let clock = FakeClock::new(1_700_000_000_000);
    let (limiter, _) = limiter(&clock);
    let transport = Arc::new(FakeTransport::new(
        vec![Ok(MetadataTransportResult::NotFound)],
        vec![],
    ));
    let provider = provider(Arc::clone(&transport), &clock, limiter, 8);
    let input = MetadataQuery::new(
        Some("  AC/DC   Live  ".to_owned()),
        vec!["  Artist One ".to_owned()],
        None,
        Some(123_000),
    )
    .expect("slash is valid metadata text");

    let result = provider
        .lookup_recording(input, &ProviderCallContext::new(Duration::from_secs(5)))
        .await
        .expect("hermetic lookup succeeds");

    assert!(result.matches.is_empty());
    {
        let captured = transport.captured.lock().expect("capture lock");
        let CapturedRequest::Metadata(request, timeout) = &captured[0] else {
            panic!("expected metadata request")
        };
        assert_eq!(request.query().title(), Some("AC/DC Live"));
        assert_eq!(request.query().artists(), &["Artist One"]);
        assert_eq!(request.query().album(), None);
        assert_eq!(request.query().duration_ms(), Some(123_000));
        assert_eq!(
            request.user_agent().as_str(),
            "CyberKindred/0.1.0 (https://example.invalid/cyberkindred)"
        );
        assert!(*timeout <= METADATA_DEADLINE);
    }
    assert!(MetadataQuery::new(Some("bad\npath".into()), vec![], None, None).is_err());
    assert!(MetadataQuery::new(None, vec![], Some("album only".into()), None).is_err());
    assert!(MetadataUserAgent::new("0.1.0", "placeholder").is_err());

    let cancelled = ProviderCallContext::new(Duration::from_secs(5));
    cancelled.cancellation.cancel();
    assert!(
        provider
            .lookup_recording(query("Cancelled"), &cancelled)
            .await
            .is_err()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn metadata_provider_global_limiter_serializes_across_instances() {
    let clock = FakeClock::new(10_000);
    let (limiter, sleeps) = limiter(&clock);
    let first_transport = Arc::new(FakeTransport::new(
        vec![Ok(MetadataTransportResult::NotFound)],
        vec![],
    ));
    let second_transport = Arc::new(FakeTransport::new(
        vec![Ok(MetadataTransportResult::NotFound)],
        vec![],
    ));
    let first = provider(
        Arc::clone(&first_transport),
        &clock,
        Arc::clone(&limiter),
        8,
    );
    let second = provider(Arc::clone(&second_transport), &clock, limiter, 8);

    first
        .lookup_recording(
            query("First"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("first lookup");
    second
        .lookup_recording(
            query("Second"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("second lookup");

    assert_eq!(
        sleeps.lock().expect("sleep lock").as_slice(),
        &[Duration::from_secs(1)]
    );
}

#[tokio::test]
async fn metadata_provider_positive_and_negative_cache_policies_are_bounded() {
    let clock = FakeClock::new(100);
    let (limiter, _) = limiter(&clock);
    let transport = Arc::new(FakeTransport::new(
        vec![
            Ok(MetadataTransportResult::Matches(vec![candidate(
                0.95,
                "Permanent",
                "Alice / Bob",
            )])),
            Ok(MetadataTransportResult::NotFound),
            Ok(MetadataTransportResult::NotFound),
            Ok(MetadataTransportResult::NotFound),
        ],
        vec![],
    ));
    let provider = provider(Arc::clone(&transport), &clock, limiter, 2);

    provider
        .lookup_recording(
            query("Permanent"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("positive lookup");
    clock.advance(40 * 24 * 60 * 60 * 1_000);
    let positive = provider
        .lookup_recording(
            query("Permanent"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("permanent cache lookup");
    assert_eq!(positive.source, CacheSource::Cache);

    provider
        .lookup_recording(
            query("Missing"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("negative lookup");
    clock.advance(NEGATIVE_CACHE_TTL_MS - 1);
    let negative = provider
        .lookup_recording(
            query("Missing"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("fresh negative cache");
    assert_eq!(negative.source, CacheSource::Cache);
    clock.advance(1);
    let expired = provider
        .lookup_recording(
            query("Missing"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("expired negative is refreshed");
    assert_eq!(expired.source, CacheSource::Live);

    provider
        .lookup_recording(
            query("Evicts oldest"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("bounded insertion");
    assert_eq!(transport.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn enrichment_low_confidence_never_overwrites_original_and_offline_uses_provenance() {
    let original = OriginalMetadata {
        title: Some("Original Title".to_owned()),
        artist: Some("Original Artist".to_owned()),
        album: Some("Original Album".to_owned()),
    };
    let low = MetadataLookup {
        matches: vec![candidate(0.69, "Wrong", "Wrong")],
        fetched_at_ms: 123,
        source: CacheSource::Live,
    };
    let low_view = build_enrichment_view(&original, &low);
    assert_eq!(low_view.original, original);
    assert_eq!(low_view.enriched, None);
    assert_eq!(low_view.match_status, MatchStatus::Unmatched);

    let suggestion = MetadataLookup {
        matches: vec![candidate(0.80, "Suggestion", "Candidate")],
        fetched_at_ms: 456,
        source: CacheSource::Cache,
    };
    let suggestion_view = build_enrichment_view(&original, &suggestion);
    assert_eq!(
        suggestion_view.original.title.as_deref(),
        Some("Original Title")
    );
    assert_eq!(suggestion_view.match_status, MatchStatus::Review);
    assert_eq!(suggestion_view.provenance.provider, "musicbrainz");
    assert_eq!(suggestion_view.provenance.fetched_at_ms, 456);
    assert_eq!(suggestion_view.provenance.source, CacheSource::Cache);
    let serialized = serde_json::to_value(&suggestion_view).expect("view serializes");
    assert_eq!(
        serialized.pointer("/enriched/fetchedAt"),
        Some(&serde_json::Value::String(
            "1970-01-01T00:00:00.456Z".to_owned()
        ))
    );

    let exact = MetadataLookup {
        matches: vec![candidate(0.95, " original   title ", "ORIGINAL ARTIST")],
        fetched_at_ms: 789,
        source: CacheSource::Live,
    };
    assert_eq!(
        build_enrichment_view(&original, &exact).match_status,
        MatchStatus::Matched
    );
    let whitespace_collision = MetadataLookup {
        matches: vec![candidate(0.99, "OriginalTitle", "Original Artist")],
        fetched_at_ms: 790,
        source: CacheSource::Live,
    };
    assert_eq!(
        build_enrichment_view(&original, &whitespace_collision).match_status,
        MatchStatus::Review
    );

    let clock = FakeClock::new(1_000);
    let (limiter, _) = limiter(&clock);
    let transport = Arc::new(FakeTransport::new(
        vec![
            Ok(MetadataTransportResult::Matches(vec![candidate(
                0.80,
                "Cached",
                "Alice / Bob",
            )])),
            Err(unavailable()),
        ],
        vec![],
    ));
    let provider = provider(transport, &clock, limiter, 8);
    provider
        .lookup_recording(
            query("Cached"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("prime cache");
    let fallback = provider
        .refresh_recording(
            query("Cached"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("provider failure falls back to fresh positive cache");
    assert_eq!(fallback.source, CacheSource::CacheFallback);
    assert_eq!(fallback.fetched_at_ms, 1_000);
}

#[derive(Default)]
struct VecSink(Vec<u8>);

impl CoverSink for VecSink {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ProviderFailure> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn small_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes
}

#[tokio::test]
async fn metadata_provider_cover_uses_only_valid_mbid_and_caches_success_and_404() {
    let clock = FakeClock::new(1_000);
    let (limiter, _) = limiter(&clock);
    let release = Uuid::from_u128(99);
    let transport = Arc::new(FakeTransport::new(
        vec![],
        vec![
            Ok(CoverTransportResult::Found {
                bytes: small_png(500, 500),
                declared_mime: "image/png".to_owned(),
                source_url: "https://ia.example.archive.org/cover.png".to_owned(),
                etag: Some("safe-etag".to_owned()),
                attribution: Some("Cover Art Archive".to_owned()),
            }),
            Ok(CoverTransportResult::NotFound),
        ],
    ));
    let provider = provider(Arc::clone(&transport), &clock, limiter, 8);
    let mut first = VecSink::default();
    let artifact = provider
        .fetch_cover(
            release,
            CoverSize::Px500,
            &mut first,
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("valid cover")
        .expect("cover exists");
    assert_eq!(artifact.byte_length, first.0.len());
    let mut cached = VecSink::default();
    provider
        .fetch_cover(
            release,
            CoverSize::Px500,
            &mut cached,
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("cached cover")
        .expect("cached cover exists");
    assert_eq!(first.0, cached.0);

    let missing_release = Uuid::from_u128(100);
    let mut missing = VecSink::default();
    assert!(
        provider
            .fetch_cover(
                missing_release,
                CoverSize::Px500,
                &mut missing,
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .expect("404 is a normal miss")
            .is_none()
    );
    assert!(
        provider
            .fetch_cover(
                missing_release,
                CoverSize::Px500,
                &mut missing,
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .expect("404 cache")
            .is_none()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    {
        let captured = transport.captured.lock().expect("capture lock");
        let CapturedRequest::Cover(request, timeout) = &captured[0] else {
            panic!("expected cover request")
        };
        assert_eq!(request.release_mbid(), release);
        assert_eq!(request.size(), CoverSize::Px500);
        assert!(*timeout <= COVER_DEADLINE);
    }

    let before = transport.calls.load(Ordering::SeqCst);
    assert!(
        provider
            .fetch_cover(
                Uuid::nil(),
                CoverSize::Px500,
                &mut missing,
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .is_err()
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn metadata_provider_rejects_untrusted_cover_origin_and_image_dimensions() {
    let clock = FakeClock::new(1_000);
    let (limiter, _) = limiter(&clock);
    let transport = Arc::new(FakeTransport::new(
        vec![],
        vec![
            Ok(CoverTransportResult::Found {
                bytes: small_png(500, 500),
                declared_mime: "image/png".to_owned(),
                source_url: "https://evil.invalid/cover.png".to_owned(),
                etag: None,
                attribution: None,
            }),
            Ok(CoverTransportResult::Found {
                bytes: small_png(5_000, 500),
                declared_mime: "image/png".to_owned(),
                source_url: "https://archive.org/cover.png".to_owned(),
                etag: None,
                attribution: None,
            }),
        ],
    ));
    let provider = provider(transport, &clock, limiter, 8);
    let mut sink = VecSink::default();

    for release in [Uuid::from_u128(8), Uuid::from_u128(9)] {
        let error = provider
            .fetch_cover(
                release,
                CoverSize::Px500,
                &mut sink,
                &ProviderCallContext::new(Duration::from_secs(5)),
            )
            .await
            .expect_err("unsafe cover must be rejected");
        assert_eq!(error.category, ProviderFailureCategory::InvalidResponse);
    }
    assert!(sink.0.is_empty());
}

#[test]
fn metadata_provider_production_parser_validates_identifiers_and_builds_fixed_query() {
    ReqwestMetadataTransport::new().expect("production TLS client can be constructed");
    let parsed = parse_musicbrainz_response(
        br#"{
            "recordings": [{
                "id": "00000000-0000-0000-0000-000000000001",
                "title": "A Result",
                "artist-credit": [{"name": "Artist"}],
                "releases": [{
                    "id": "00000000-0000-0000-0000-000000000002",
                    "title": "Release"
                }],
                "tags": [{"name": "ambient"}],
                "score": 91
            }]
        }"#,
    )
    .expect("bounded MusicBrainz shape parses");
    let MetadataTransportResult::Matches(matches) = parsed else {
        panic!("expected parsed candidate")
    };
    assert!((matches[0].confidence - 0.91).abs() < f64::EPSILON);

    assert!(
        parse_musicbrainz_response(
            br#"{"recordings":[{"id":"not-an-mbid","title":"x","artist-credit":[{"name":"a"}],"score":99}]}"#
        )
        .is_err()
    );
    let escaped = musicbrainz_query(
        &MetadataQuery::new(
            Some("quoted \"title\"".to_owned()),
            vec!["AC/DC".to_owned()],
            None,
            None,
        )
        .expect("valid query"),
    );
    assert_eq!(
        escaped,
        "recording:\"quoted \\\"title\\\"\" AND artist:\"AC/DC\""
    );

    assert!(validate_cover_url(&Url::parse("https://archive.org/file").expect("url")).is_ok());
    assert!(validate_cover_url(&Url::parse("http://archive.org/file").expect("url")).is_err());
    assert!(
        validate_cover_url(&Url::parse("https://archive.org.evil.invalid/file").expect("url"))
            .is_err()
    );
}

#[tokio::test]
async fn metadata_provider_cache_facts_are_serializable_validated_and_no_call_on_restore() {
    let clock = FakeClock::new(5_000);
    let (shared_limiter, _) = limiter(&clock);
    let source_transport = Arc::new(FakeTransport::new(
        vec![Ok(MetadataTransportResult::Matches(vec![candidate(
            0.85,
            "Persisted",
            "Alice / Bob",
        )]))],
        vec![],
    ));
    let source_cache = Arc::new(MetadataCache::new(4, 4, 1_024));
    let source = CachedMetadataProvider::new(
        source_transport,
        MetadataUserAgent::new("0.1.0", "mailto:maintainer@example.invalid")
            .expect("valid user agent"),
        Arc::clone(&shared_limiter),
        Arc::clone(&source_cache),
        Arc::new(clock.clone()),
    );
    source
        .lookup_recording(
            query("Persisted"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("prime persisted cache fact");
    let encoded =
        serde_json::to_string(&source_cache.snapshot_match_facts()).expect("cache facts serialize");
    assert!(encoded.contains("\"provider\":\"musicbrainz\""));
    assert!(!encoded.contains("path"));
    assert!(!encoded.contains("audio"));
    let decoded: Vec<MetadataCacheFact> =
        serde_json::from_str(&encoded).expect("cache facts deserialize");

    let restored_cache = Arc::new(MetadataCache::new(4, 4, 1_024));
    restored_cache
        .restore_match_facts(decoded)
        .expect("validated cache facts restore");
    let no_call_transport = Arc::new(FakeTransport::new(vec![], vec![]));
    let restored = CachedMetadataProvider::new(
        Arc::clone(&no_call_transport) as Arc<dyn MetadataTransport>,
        MetadataUserAgent::new("0.1.0", "mailto:maintainer@example.invalid")
            .expect("valid user agent"),
        shared_limiter,
        restored_cache,
        Arc::new(clock),
    );
    let lookup = restored
        .lookup_recording(
            query("Persisted"),
            &ProviderCallContext::new(Duration::from_secs(5)),
        )
        .await
        .expect("restored positive cache is usable offline");
    assert_eq!(lookup.source, CacheSource::Cache);
    assert_eq!(lookup.fetched_at_ms, 5_000);
    assert_eq!(no_call_transport.calls.load(Ordering::SeqCst), 0);

    let mut valid_fact = source_cache.snapshot_match_facts().pop().expect("one fact");
    let mut invalid_fact = valid_fact.clone();
    invalid_fact.provider = "unexpected-provider".to_owned();
    valid_fact.title = Some("Would partially restore".to_owned());
    let atomic_cache = MetadataCache::new(4, 4, 1_024);
    assert!(
        atomic_cache
            .restore_match_facts(vec![valid_fact, invalid_fact])
            .is_err()
    );
    assert!(atomic_cache.snapshot_match_facts().is_empty());
}
