use super::*;
use crate::{
    ipc::ErrorId,
    storage::{AppPaths, Repository, Storage, StorageReason, StoredOnboardingDocument},
};
use serde_json::json;
use uuid::Uuid;

fn temporary_paths() -> (tempfile::TempDir, AppPaths) {
    let temp = tempfile::tempdir().expect("temporary root");
    let paths = AppPaths::create(
        temp.path().join("data"),
        temp.path().join("cache"),
        temp.path().join("logs"),
    )
    .expect("scoped paths");
    (temp, paths)
}

async fn add_authorized_library_root(temp: &tempfile::TempDir, repository: &Repository) {
    let selected_directory = temp.path().join("authorized-music");
    std::fs::create_dir(&selected_directory).expect("real selected directory");
    repository
        .add_library_root(&selected_directory, 1)
        .await
        .expect("authorized local root");
}

fn request(
    expected_revision: u64,
    submission: OnboardingStepSubmission,
) -> SaveOnboardingStepRequest {
    SaveOnboardingStepRequest {
        client_request_id: Uuid::now_v7(),
        expected_revision,
        submission,
    }
}

fn neutral_profile() -> OnboardingProfile {
    OnboardingProfile::default()
}

fn full_sequence() -> Vec<OnboardingStepSubmission> {
    vec![
        OnboardingStepSubmission::Welcome {},
        OnboardingStepSubmission::MusicSource {
            sources: vec![MusicSourceKind::Local, MusicSourceKind::AppleMusic],
        },
        OnboardingStepSubmission::OpenaiKey {
            mode: OnboardingAiMode::LocalOnly,
        },
        OnboardingStepSubmission::Voice {
            mode: OnboardingVoiceMode::TextOnly,
        },
        OnboardingStepSubmission::Profile {
            profile: neutral_profile(),
        },
        OnboardingStepSubmission::CitySchedule {
            mode: CityScheduleMode::NotNow,
        },
        OnboardingStepSubmission::Privacy {
            confirmations: PrivacyConfirmations {
                explicit_sound: true,
                raw_conversation_retention: true,
            },
        },
    ]
}

#[tokio::test]
async fn new_install_returns_the_exact_neutral_state() {
    let (_temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let service = OnboardingService::new(storage.repository());

    let state = service.get_state().await.expect("default state");
    assert_eq!(
        state,
        OnboardingState {
            completed: false,
            completed_steps: Vec::new(),
            source_selection: Vec::new(),
            ai_mode: None,
            voice_mode: None,
            city_schedule_mode: None,
            profile: neutral_profile(),
            privacy_confirmations: PrivacyConfirmations::default(),
            revision: 0,
        }
    );
    assert_eq!(
        serde_json::to_value(state).expect("state JSON"),
        json!({
            "completed": false,
            "completedSteps": [],
            "sourceSelection": [],
            "aiMode": null,
            "voiceMode": null,
            "cityScheduleMode": null,
            "profile": {
                "displayName": "",
                "companionStyle": "quiet_warm",
                "initialPreferences": [],
                "narrationDensity": "balanced"
            },
            "privacyConfirmations": {
                "explicitSound": false,
                "rawConversationRetention": false
            },
            "revision": 0
        })
    );
    storage.close().await;
}

#[tokio::test]
async fn each_step_survives_database_reopen_and_completed_steps_remain_a_prefix() {
    let (temp, paths) = temporary_paths();
    let mut storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    add_authorized_library_root(&temp, &storage.repository()).await;
    let expected_prefix = [
        OnboardingStep::Welcome,
        OnboardingStep::MusicSource,
        OnboardingStep::OpenaiKey,
        OnboardingStep::Voice,
        OnboardingStep::Profile,
        OnboardingStep::CitySchedule,
        OnboardingStep::Privacy,
    ];

    for (index, submission) in full_sequence().into_iter().enumerate() {
        let repository = storage.repository();
        let service = OnboardingService::new(repository);
        let ack = service
            .save_step(request(index as u64, submission))
            .await
            .expect("ordered step");
        assert_eq!(ack.revision, index as u64 + 1);
        drop(service);
        storage.close().await;

        storage = Storage::open(&paths, "0.1.0").await.expect("reopen");
        let restarted = OnboardingService::new(storage.repository());
        let state = restarted.get_state().await.expect("recovered state");
        assert_eq!(state.revision, index as u64 + 1);
        assert_eq!(state.completed_steps, expected_prefix[..=index]);
        assert_eq!(state.completed, index + 1 == expected_prefix.len());
    }

    let repository = storage.repository();
    let service = OnboardingService::new(repository.clone());
    let edited_profile = OnboardingProfile {
        display_name: "阿岚".to_owned(),
        companion_style: CompanionStyle::QuietWarm,
        initial_preferences: vec!["夜间安静陪伴".to_owned()],
        narration_density: NarrationDensity::Frequent,
    };
    let ack = service
        .save_step(request(
            7,
            OnboardingStepSubmission::Profile {
                profile: edited_profile.clone(),
            },
        ))
        .await
        .expect("edit completed step");
    assert_eq!(ack.revision, 8);
    let edited = service.get_state().await.expect("edited state");
    assert!(edited.completed);
    assert_eq!(edited.completed_steps, expected_prefix);
    assert_eq!(edited.profile, edited_profile);

    let (state_json, narration, settings_revision, routine, preferences) = repository
        .onboarding_storage_values()
        .await
        .expect("stored values");
    assert!(!state_json.contains("阿岚"));
    assert!(!state_json.contains("initialPreferences"));
    assert_eq!(narration.as_deref(), Some("\"frequent\""));
    assert_eq!(settings_revision.as_deref(), Some("2"));
    assert!(
        routine
            .as_deref()
            .is_some_and(|value| value.contains("quiet_warm"))
    );
    assert!(
        preferences
            .as_deref()
            .is_some_and(|value| value.contains("夜间安静陪伴"))
    );
    assert_eq!(
        repository
            .onboarding_external_effect_counts()
            .await
            .expect("provider/audio side-effect tables"),
        (0, 0, 0),
        "API-003 must not initiate provider, outbox, or TTS work"
    );
    storage.close().await;
}

#[tokio::test]
async fn ordering_local_authorization_and_revision_conflicts_preserve_state() {
    let (temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    let service = OnboardingService::new(repository.clone());

    let skipped = service
        .save_step(request(
            0,
            OnboardingStepSubmission::MusicSource {
                sources: vec![MusicSourceKind::AppleMusic],
            },
        ))
        .await
        .expect_err("first step cannot be skipped");
    assert_eq!(skipped.error_id, ErrorId::RequestInvalid);
    assert_eq!(service.get_state().await.expect("unchanged").revision, 0);

    service
        .save_step(request(0, OnboardingStepSubmission::Welcome {}))
        .await
        .expect("welcome");
    let no_root = service
        .save_step(request(
            1,
            OnboardingStepSubmission::MusicSource {
                sources: vec![MusicSourceKind::Local],
            },
        ))
        .await
        .expect_err("local requires an authorized root");
    assert_eq!(no_root.error_id, ErrorId::RequestInvalid);
    assert_eq!(service.get_state().await.expect("unchanged").revision, 1);

    add_authorized_library_root(&temp, &repository).await;
    service
        .save_step(request(
            1,
            OnboardingStepSubmission::MusicSource {
                sources: vec![MusicSourceKind::Local],
            },
        ))
        .await
        .expect("local source");
    let stale = service
        .save_step(request(
            1,
            OnboardingStepSubmission::OpenaiKey {
                mode: OnboardingAiMode::LocalOnly,
            },
        ))
        .await
        .expect_err("stale revision");
    assert_eq!(stale.error_id, ErrorId::Conflict);
    assert_eq!(
        stale
            .details
            .as_deref()
            .and_then(|value| value.current_revision),
        Some(2)
    );
    let preserved = service.get_state().await.expect("preserved state");
    assert_eq!(preserved.revision, 2);
    assert_eq!(preserved.completed_steps.len(), 2);
    assert_eq!(preserved.ai_mode, None);
    storage.close().await;
}

#[tokio::test]
async fn idempotency_replays_original_ack_and_rejects_payload_reuse() {
    let (_temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let service = OnboardingService::new(storage.repository());
    let request_id = Uuid::now_v7();
    let original = SaveOnboardingStepRequest {
        client_request_id: request_id,
        expected_revision: 0,
        submission: OnboardingStepSubmission::Welcome {},
    };

    let first = service
        .save_step(original.clone())
        .await
        .expect("first save");
    let replay = service.save_step(original).await.expect("identical replay");
    assert_eq!(replay, first);
    assert_eq!(service.get_state().await.expect("one write").revision, 1);

    let conflict = service
        .save_step(SaveOnboardingStepRequest {
            client_request_id: request_id,
            expected_revision: 1,
            submission: OnboardingStepSubmission::MusicSource {
                sources: vec![MusicSourceKind::AppleMusic],
            },
        })
        .await
        .expect_err("same key with different payload");
    assert_eq!(conflict.error_id, ErrorId::Conflict);
    assert_eq!(
        conflict
            .details
            .as_deref()
            .and_then(|value| value.field.as_deref()),
        Some("clientRequestId")
    );
    assert_eq!(
        service.get_state().await.expect("still one write").revision,
        1
    );
    storage.close().await;
}

#[tokio::test]
async fn onboarding_submission_serde_is_exact_and_rejects_unknown_fields() {
    for exact_submission in [
        json!({ "step": "welcome" }),
        json!({ "step": "music_source", "sources": ["local", "apple_music"] }),
        json!({ "step": "openai_key", "mode": "verified" }),
        json!({ "step": "voice", "mode": "selected" }),
        json!({
            "step": "profile",
            "profile": {
                "displayName": "",
                "companionStyle": "quiet_warm",
                "initialPreferences": [],
                "narrationDensity": "balanced"
            }
        }),
        json!({ "step": "city_schedule", "mode": "configured" }),
        json!({
            "step": "privacy",
            "confirmations": {
                "explicitSound": true,
                "rawConversationRetention": true
            }
        }),
    ] {
        let decoded = serde_json::from_value::<OnboardingStepSubmission>(exact_submission.clone())
            .expect("exact submission");
        assert_eq!(
            serde_json::to_value(decoded).expect("submission JSON"),
            exact_submission
        );
    }

    let unknown_top = json!({
        "clientRequestId": Uuid::now_v7(),
        "expectedRevision": 0,
        "submission": { "step": "welcome" },
        "apiKey": "ck-task010-secret-canary"
    });
    assert!(serde_json::from_value::<SaveOnboardingStepRequest>(unknown_top).is_err());
    let unknown_variant = json!({
        "clientRequestId": Uuid::now_v7(),
        "expectedRevision": 0,
        "submission": { "step": "welcome", "path": "C:/private/canary" }
    });
    assert!(serde_json::from_value::<SaveOnboardingStepRequest>(unknown_variant).is_err());
    let unknown_profile = json!({
        "clientRequestId": Uuid::now_v7(),
        "expectedRevision": 4,
        "submission": {
            "step": "profile",
            "profile": {
                "displayName": "",
                "companionStyle": "quiet_warm",
                "initialPreferences": [],
                "narrationDensity": "balanced",
                "secret": "ck-task010-secret-canary"
            }
        }
    });
    assert!(serde_json::from_value::<SaveOnboardingStepRequest>(unknown_profile).is_err());
}

#[tokio::test]
async fn invalid_profile_source_or_privacy_input_does_not_persist() {
    let (_temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    let service = OnboardingService::new(repository.clone());
    let false_privacy = service
        .save_step(request(
            0,
            OnboardingStepSubmission::Privacy {
                confirmations: PrivacyConfirmations {
                    explicit_sound: false,
                    raw_conversation_retention: true,
                },
            },
        ))
        .await
        .expect_err("privacy literals must both be true");
    assert_eq!(false_privacy.error_id, ErrorId::RequestInvalid);
    let duplicate_sources = service
        .save_step(request(
            0,
            OnboardingStepSubmission::MusicSource {
                sources: vec![MusicSourceKind::Local, MusicSourceKind::Local],
            },
        ))
        .await
        .expect_err("duplicate source selection");
    assert_eq!(duplicate_sources.error_id, ErrorId::RequestInvalid);

    for profile in [
        OnboardingProfile {
            display_name: "x".repeat(81),
            ..neutral_profile()
        },
        OnboardingProfile {
            initial_preferences: vec![String::new()],
            ..neutral_profile()
        },
        OnboardingProfile {
            initial_preferences: vec!["x".to_owned(); 21],
            ..neutral_profile()
        },
        OnboardingProfile {
            initial_preferences: vec!["x".repeat(101)],
            ..neutral_profile()
        },
    ] {
        let error = service
            .save_step(request(0, OnboardingStepSubmission::Profile { profile }))
            .await
            .expect_err("invalid profile");
        assert_eq!(error.error_id, ErrorId::RequestInvalid);
    }
    assert_eq!(service.get_state().await.expect("unchanged").revision, 0);
    let state_count = repository.onboarding_storage_values().await;
    assert!(
        state_count.is_err(),
        "a rejected request must not create state"
    );
    storage.close().await;
}

#[tokio::test]
async fn profile_transaction_bumps_settings_revision_and_stale_settings_write_fails() {
    let (temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    let service = OnboardingService::new(repository.clone());
    let stale_settings = repository
        .load_provider_settings()
        .await
        .expect("settings before profile");

    for (revision, submission) in full_sequence().into_iter().take(5).enumerate() {
        if revision == 1 {
            add_authorized_library_root(&temp, &repository).await;
        }
        service
            .save_step(request(revision as u64, submission))
            .await
            .expect("prefix through profile");
    }
    assert_eq!(service.get_state().await.expect("onboarding").revision, 5);
    let settings = repository
        .load_provider_settings()
        .await
        .expect("settings after profile");
    assert_eq!(settings.revision, 1);
    assert_eq!(settings.narration_density, "balanced");

    let stale = repository
        .save_provider_settings(0, &stale_settings, 10, false, None)
        .await
        .expect_err("API-008 repository CAS must observe profile settings revision");
    assert_eq!(stale.reason(), StorageReason::RevisionConflict);
    assert_eq!(
        repository
            .load_provider_settings()
            .await
            .expect("unchanged settings")
            .revision,
        1
    );
    assert_eq!(
        service
            .get_state()
            .await
            .expect("independent aggregate")
            .revision,
        5
    );
    storage.close().await;
}

#[tokio::test]
async fn profile_storage_failure_rolls_back_onboarding_profile_and_settings_revisions_together() {
    let (temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    add_authorized_library_root(&temp, &repository).await;
    let service = OnboardingService::new(repository.clone());
    for (revision, submission) in full_sequence().into_iter().take(4).enumerate() {
        service
            .save_step(request(revision as u64, submission))
            .await
            .expect("prefix before profile");
    }
    repository
        .reject_onboarding_profile_writes()
        .await
        .expect("failure fixture");

    let error = service
        .save_step(request(
            4,
            OnboardingStepSubmission::Profile {
                profile: OnboardingProfile {
                    display_name: "transaction-canary".to_owned(),
                    companion_style: CompanionStyle::QuietWarm,
                    initial_preferences: vec!["must roll back".to_owned()],
                    narration_density: NarrationDensity::Frequent,
                },
            },
        ))
        .await
        .expect_err("injected profile write failure");
    assert_eq!(error.error_id, ErrorId::StorageFailed);

    let recovered = service.get_state().await.expect("pre-profile state");
    assert_eq!(recovered.revision, 4);
    assert_eq!(recovered.completed_steps.len(), 4);
    assert_eq!(recovered.profile, neutral_profile());
    let settings = repository
        .load_provider_settings()
        .await
        .expect("pre-profile settings");
    assert_eq!(settings.revision, 0);
    assert_eq!(settings.narration_density, "balanced");
    let (state_json, narration, settings_revision, routine, preferences) = repository
        .onboarding_storage_values()
        .await
        .expect("rolled-back storage");
    assert!(!state_json.contains("transaction-canary"));
    assert_eq!(narration, None);
    assert_eq!(settings_revision, None);
    assert_eq!(routine, None);
    assert_eq!(preferences, None);

    repository
        .store_settings_revision_fixture(9_007_199_254_740_992)
        .await
        .expect("corrupt settings revision fixture");
    let corrupt = service
        .save_step(request(
            4,
            OnboardingStepSubmission::Profile {
                profile: neutral_profile(),
            },
        ))
        .await
        .expect_err("persisted over-bound settings revision");
    assert_eq!(corrupt.error_id, ErrorId::StorageFailed);
    assert_eq!(
        corrupt
            .details
            .as_deref()
            .and_then(|details| details.reason.as_deref()),
        Some("storage_integrity_failed")
    );
    assert_eq!(
        service
            .get_state()
            .await
            .expect("still rolled back")
            .revision,
        4
    );
    storage.close().await;
}

#[tokio::test]
async fn revisions_are_bounded_to_the_javascript_safe_integer_contract() {
    const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

    let (_temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    let service = OnboardingService::new(repository.clone());
    let over_bound_request = service
        .save_step(request(
            MAX_JS_SAFE_INTEGER + 1,
            OnboardingStepSubmission::Welcome {},
        ))
        .await
        .expect_err("over-bound expected revision");
    assert_eq!(over_bound_request.error_id, ErrorId::RequestInvalid);
    assert_eq!(
        over_bound_request
            .details
            .as_deref()
            .and_then(|details| details.field.as_deref()),
        Some("expectedRevision")
    );

    let at_limit = StoredOnboardingDocument {
        revision: MAX_JS_SAFE_INTEGER,
        ..StoredOnboardingDocument::default()
    };
    repository
        .store_onboarding_document_fixture(&at_limit)
        .await
        .expect("at-limit fixture");
    assert_eq!(
        service.get_state().await.expect("at-limit state").revision,
        MAX_JS_SAFE_INTEGER
    );
    let exhausted = service
        .save_step(request(
            MAX_JS_SAFE_INTEGER,
            OnboardingStepSubmission::Welcome {},
        ))
        .await
        .expect_err("revision cannot exceed JavaScript safe integer");
    assert_eq!(exhausted.error_id, ErrorId::StorageFailed);
    assert_eq!(
        service
            .get_state()
            .await
            .expect("preserved at-limit state")
            .revision,
        MAX_JS_SAFE_INTEGER
    );

    let over_limit = StoredOnboardingDocument {
        revision: MAX_JS_SAFE_INTEGER + 1,
        ..StoredOnboardingDocument::default()
    };
    repository
        .store_onboarding_document_fixture(&over_limit)
        .await
        .expect("over-limit fixture");
    let integrity = service
        .get_state()
        .await
        .expect_err("persisted over-bound revision");
    assert_eq!(integrity.error_id, ErrorId::StorageFailed);
    assert_eq!(
        integrity
            .details
            .as_deref()
            .and_then(|details| details.reason.as_deref()),
        Some("storage_integrity_failed")
    );
    storage.close().await;
}

#[tokio::test]
async fn profile_save_cannot_overflow_settings_revision_or_partially_commit() {
    const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

    let (temp, paths) = temporary_paths();
    let storage = Storage::open(&paths, "0.1.0").await.expect("storage");
    let repository = storage.repository();
    add_authorized_library_root(&temp, &repository).await;
    let service = OnboardingService::new(repository.clone());
    for (revision, submission) in full_sequence().into_iter().take(4).enumerate() {
        service
            .save_step(request(revision as u64, submission))
            .await
            .expect("prefix before profile");
    }
    repository
        .store_settings_revision_fixture(MAX_JS_SAFE_INTEGER)
        .await
        .expect("settings limit fixture");

    let overflow = service
        .save_step(request(
            4,
            OnboardingStepSubmission::Profile {
                profile: OnboardingProfile {
                    narration_density: NarrationDensity::Frequent,
                    ..neutral_profile()
                },
            },
        ))
        .await
        .expect_err("settings revision overflow");
    assert_eq!(overflow.error_id, ErrorId::StorageFailed);
    assert_eq!(service.get_state().await.expect("rolled back").revision, 4);
    let settings = repository
        .load_provider_settings()
        .await
        .expect("settings preserved");
    assert_eq!(settings.revision, MAX_JS_SAFE_INTEGER);
    assert_eq!(settings.narration_density, "balanced");
    let (_, narration, settings_revision, routine, preferences) = repository
        .onboarding_storage_values()
        .await
        .expect("unchanged profile storage");
    assert_eq!(narration, None);
    assert_eq!(settings_revision.as_deref(), Some("9007199254740991"));
    assert_eq!(routine, None);
    assert_eq!(preferences, None);
    storage.close().await;
}
