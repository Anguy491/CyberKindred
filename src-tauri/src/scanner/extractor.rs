use std::{
    fs::{self, File},
    path::Path,
    sync::Once,
    time::UNIX_EPOCH,
};

use lofty::{
    config::{GlobalOptions, ParseOptions, apply_global_options},
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey},
};
use sha2::{Digest, Sha256};
use symphonia::core::{
    codecs::audio::AudioDecoderOptions,
    errors::Error as SymphoniaError,
    formats::{FormatOptions, TrackType, probe::Hint},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
};

use super::traversal::{ScanCancellation, TraversalFile};
use crate::storage::resolve_read_only_library_path;

const MAX_TAG_TEXT_CHARS: usize = 1_000;
const MAX_GENRES: usize = 32;
const MAX_TAG_ITEM_BYTES: usize = 4 * 1024 * 1024;
const MAX_INITIAL_PACKETS: usize = 32;
static LOFTY_OPTIONS: Once = Once::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExtractedAvailability {
    Available,
    Corrupt,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AudioFormat {
    Mp3,
    Flac,
    M4a,
    Aac,
    Wav,
    Ogg,
}

impl AudioFormat {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::M4a => "m4a",
            Self::Aac => "aac",
            Self::Wav => "wav",
            Self::Ogg => "ogg",
        }
    }
}

/// Internal-only extracted row. It intentionally implements neither `Debug`
/// nor serialization because it contains a relative local path and raw tags.
pub(super) struct ExtractedTrack {
    pub relative_path: String,
    pub file_identity: Option<String>,
    pub availability: ExtractedAvailability,
    pub format: String,
    pub file_size_bytes: u64,
    pub modified_at_ms: i64,
    pub duration_ms: u64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<u16>,
    pub genres: Vec<String>,
    pub embedded_cover_hash: Option<String>,
}

impl ExtractedTrack {
    pub(super) fn failed(&self) -> bool {
        self.availability != ExtractedAvailability::Available
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExtractError {
    Cancelled,
    PathDenied,
    MetadataUnavailable,
}

pub(super) fn extract_track(
    authorized_root: &Path,
    file: TraversalFile,
    cancellation: &ScanCancellation,
) -> Result<ExtractedTrack, ExtractError> {
    if cancellation.is_cancelled() {
        return Err(ExtractError::Cancelled);
    }
    let resolved = resolve_read_only_library_path(authorized_root, &file.relative_path)
        .map_err(|_| ExtractError::PathDenied)?;
    let metadata = fs::metadata(&resolved).map_err(|_| ExtractError::MetadataUnavailable)?;
    if !metadata.is_file() {
        return Err(ExtractError::PathDenied);
    }
    let file_size_bytes = metadata.len();
    let modified_at_ms = modified_at_ms(&metadata)?;
    let file_identity = stable_file_identity(&metadata);
    let format = detect_format(&resolved);
    let Some(format) = format else {
        return Ok(empty_track(
            file,
            file_identity,
            ExtractedAvailability::Unsupported,
            "unknown",
            file_size_bytes,
            modified_at_ms,
        ));
    };
    if cancellation.is_cancelled() {
        return Err(ExtractError::Cancelled);
    }

    let Ok(duration_ms) = symphonia_probe(&resolved, format) else {
        return Ok(empty_track(
            file,
            file_identity,
            ExtractedAvailability::Corrupt,
            format.as_str(),
            file_size_bytes,
            modified_at_ms,
        ));
    };
    if cancellation.is_cancelled() {
        return Err(ExtractError::Cancelled);
    }

    // Re-resolve immediately before the second open so a path replaced while
    // Symphonia was probing cannot make Lofty cross the authorized boundary.
    let resolved_for_tags = resolve_read_only_library_path(authorized_root, &file.relative_path)
        .map_err(|_| ExtractError::PathDenied)?;
    let tags = read_bounded_tags(&resolved_for_tags);
    let duration_ms = duration_ms.or(tags.duration_ms).unwrap_or(0);
    if duration_ms == 0 {
        return Ok(empty_track(
            file,
            file_identity,
            ExtractedAvailability::Corrupt,
            format.as_str(),
            file_size_bytes,
            modified_at_ms,
        ));
    }
    Ok(ExtractedTrack {
        relative_path: file.relative_path_text,
        file_identity,
        availability: ExtractedAvailability::Available,
        format: format.as_str().to_owned(),
        file_size_bytes,
        modified_at_ms,
        duration_ms,
        title: tags.title,
        artist: tags.artist,
        album: tags.album,
        album_artist: tags.album_artist,
        track_number: tags.track_number,
        disc_number: tags.disc_number,
        year: tags.year,
        genres: tags.genres,
        embedded_cover_hash: tags.embedded_cover_hash,
    })
}

fn empty_track(
    file: TraversalFile,
    file_identity: Option<String>,
    availability: ExtractedAvailability,
    format: &str,
    file_size_bytes: u64,
    modified_at_ms: i64,
) -> ExtractedTrack {
    ExtractedTrack {
        relative_path: file.relative_path_text,
        file_identity,
        availability,
        format: format.to_owned(),
        file_size_bytes,
        modified_at_ms,
        duration_ms: 0,
        title: None,
        artist: None,
        album: None,
        album_artist: None,
        track_number: None,
        disc_number: None,
        year: None,
        genres: Vec::new(),
        embedded_cover_hash: None,
    }
}

fn detect_format(path: &Path) -> Option<AudioFormat> {
    extension_format(path).or_else(|| {
        let kind = infer::get_from_path(path).ok().flatten()?;
        mime_format(kind.mime_type())
    })
}

fn extension_format(path: &Path) -> Option<AudioFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "mp3" => Some(AudioFormat::Mp3),
        "flac" => Some(AudioFormat::Flac),
        "m4a" | "mp4" => Some(AudioFormat::M4a),
        "aac" => Some(AudioFormat::Aac),
        "wav" => Some(AudioFormat::Wav),
        "ogg" | "oga" => Some(AudioFormat::Ogg),
        _ => None,
    }
}

fn mime_format(mime: &str) -> Option<AudioFormat> {
    match mime {
        "audio/mpeg" => Some(AudioFormat::Mp3),
        "audio/flac" | "audio/x-flac" => Some(AudioFormat::Flac),
        "audio/mp4" | "video/mp4" => Some(AudioFormat::M4a),
        "audio/aac" => Some(AudioFormat::Aac),
        "audio/wav" | "audio/x-wav" => Some(AudioFormat::Wav),
        "audio/ogg" | "application/ogg" => Some(AudioFormat::Ogg),
        _ => None,
    }
}

/// Symphonia 0.6.1 is the authoritative format/codec probe. At least one
/// packet must decode so a header-only or obviously damaged file is isolated.
fn symphonia_probe(path: &Path, format: AudioFormat) -> Result<Option<u64>, ()> {
    let source = File::open(path).map_err(|_| ())?;
    let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    hint.with_extension(format.as_str());
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| ())?;
    let track = reader.default_track(TrackType::Audio).ok_or(())?;
    let track_id = track.id;
    let codec_parameters = track.codec_params.as_ref().ok_or(())?.audio().ok_or(())?;
    let duration_ms = track
        .time_base
        .zip(track.duration)
        .and_then(|(time_base, duration)| time_base.calc_duration(duration))
        .and_then(|duration| u64::try_from(duration.as_millis()).ok());
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_parameters, &AudioDecoderOptions::default())
        .map_err(|_| ())?;
    for _ in 0..MAX_INITIAL_PACKETS {
        let Ok(Some(packet)) = reader.next_packet() else {
            return Err(());
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(_) => return Ok(duration_ms),
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => {}
            Err(_) => return Err(()),
        }
    }
    Err(())
}

#[derive(Default)]
struct BoundedTags {
    duration_ms: Option<u64>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    year: Option<u16>,
    genres: Vec<String>,
    embedded_cover_hash: Option<String>,
}

fn read_bounded_tags(path: &Path) -> BoundedTags {
    LOFTY_OPTIONS.call_once(|| {
        let options = GlobalOptions::new()
            .allocation_limit(MAX_TAG_ITEM_BYTES)
            .use_custom_resolvers(false)
            .preserve_format_specific_items(false);
        apply_global_options(options);
    });
    let parse_options = ParseOptions::new().max_junk_bytes(4_096);
    let Ok(tagged) = Probe::open(path)
        .and_then(|probe| {
            probe
                .options(parse_options)
                .guess_file_type()
                .map_err(Into::into)
        })
        .and_then(Probe::read)
    else {
        return BoundedTags::default();
    };
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let properties_duration = u64::try_from(tagged.properties().duration().as_millis()).ok();
    let Some(tag) = tag else {
        return BoundedTags {
            duration_ms: properties_duration,
            ..BoundedTags::default()
        };
    };
    let genres = tag
        .get_strings(ItemKey::Genre)
        .take(MAX_GENRES)
        .filter_map(bounded_text)
        .collect();
    let embedded_cover_hash = tag.pictures().first().map(|picture| {
        let mut hasher = Sha256::new();
        hasher.update(picture.data());
        hex::encode(hasher.finalize())
    });
    BoundedTags {
        duration_ms: properties_duration,
        title: tag.title().and_then(|value| bounded_text(value.as_ref())),
        artist: tag.artist().and_then(|value| bounded_text(value.as_ref())),
        album: tag.album().and_then(|value| bounded_text(value.as_ref())),
        album_artist: tag.get_string(ItemKey::AlbumArtist).and_then(bounded_text),
        track_number: tag.track().filter(|value| (1..=10_000).contains(value)),
        disc_number: tag.disk().filter(|value| (1..=10_000).contains(value)),
        year: tag
            .date()
            .map(|date| date.year)
            .filter(|value| *value <= 9_999),
        genres,
        embedded_cover_hash,
    }
}

fn bounded_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_TAG_TEXT_CHARS).collect())
}

fn modified_at_ms(metadata: &fs::Metadata) -> Result<i64, ExtractError> {
    let elapsed = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .ok_or(ExtractError::MetadataUnavailable)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ExtractError::MetadataUnavailable)
}

// Rust 1.98 exposes Windows volume/file-index identity only behind the unstable
// `windows_by_handle` feature. This safe opaque fallback is root-scoped by the
// repository and intentionally treated as collision-tolerant, not globally unique.
#[cfg(windows)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the cross-platform identity contract permits unavailable identities"
)]
fn stable_file_identity(metadata: &fs::Metadata) -> Option<String> {
    use std::os::windows::fs::MetadataExt;

    let mut hasher = Sha256::new();
    hasher.update(b"cyberkindred/windows-file-identity/v1\0");
    hasher.update(metadata.creation_time().to_le_bytes());
    hasher.update(metadata.file_size().to_le_bytes());
    Some(format!("win-ct-size-v1:{}", hex::encode(hasher.finalize())))
}

#[cfg(unix)]
fn stable_file_identity(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    Some(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(not(any(unix, windows)))]
fn stable_file_identity(_metadata: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(test)]
pub(super) fn detect_format_for_test(path: &Path) -> Option<AudioFormat> {
    detect_format(path)
}
