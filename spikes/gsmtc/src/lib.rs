use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const REPORT_SCHEMA_VERSION: &str = "1.0.0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub mode: String,
    pub current_session_app_id: Option<String>,
    pub sessions: Vec<SessionSnapshot>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub source_app_user_model_id: String,
    pub is_current: bool,
    pub playback_status: Option<String>,
    pub media: Option<MediaSnapshot>,
    pub timeline: Option<TimelineSnapshot>,
    pub capabilities: Option<CapabilitySnapshot>,
    pub read_errors: Vec<FieldReadError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSnapshot {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album_title: Option<String>,
    pub track_number: Option<i32>,
    pub album_track_count: Option<i32>,
    pub genres: Vec<String>,
    pub thumbnail_available: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSnapshot {
    pub start_ms: i64,
    pub end_ms: i64,
    pub min_seek_ms: i64,
    pub max_seek_ms: i64,
    pub position_ms: i64,
    pub last_updated_windows_ticks: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshot {
    pub play: Option<bool>,
    pub pause: Option<bool>,
    pub stop: Option<bool>,
    pub next: Option<bool>,
    pub previous: Option<bool>,
    pub seek: Option<bool>,
    pub play_pause_toggle: Option<bool>,
    pub playback_rate: Option<bool>,
    pub shuffle: Option<bool>,
    pub repeat: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldReadError {
    pub field: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeObservation {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub mode: String,
    pub sessions: Vec<ObservedSession>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedSession {
    pub source_app_user_model_id: String,
    pub is_current: bool,
    pub playback_status: Option<String>,
    pub media_presence: Option<MediaPresence>,
    pub timeline: Option<TimelineSnapshot>,
    pub capabilities: Option<CapabilitySnapshot>,
    pub read_error_fields: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPresence {
    pub present_fields: Vec<String>,
    pub genre_count: usize,
    pub thumbnail_available: bool,
}

impl From<&ProbeReport> for ProbeObservation {
    fn from(report: &ProbeReport) -> Self {
        Self {
            schema_version: report.schema_version.clone(),
            captured_at_unix_ms: report.captured_at_unix_ms,
            mode: "read_only_redacted_watch".to_owned(),
            sessions: report.sessions.iter().map(ObservedSession::from).collect(),
        }
    }
}

impl From<&SessionSnapshot> for ObservedSession {
    fn from(session: &SessionSnapshot) -> Self {
        Self {
            source_app_user_model_id: session.source_app_user_model_id.clone(),
            is_current: session.is_current,
            playback_status: session.playback_status.clone(),
            media_presence: session.media.as_ref().map(MediaPresence::from),
            timeline: session.timeline.clone(),
            capabilities: session.capabilities.clone(),
            read_error_fields: session
                .read_errors
                .iter()
                .map(|error| error.field.clone())
                .collect(),
        }
    }
}

impl From<&MediaSnapshot> for MediaPresence {
    fn from(media: &MediaSnapshot) -> Self {
        let fields = [
            ("title", media.title.is_some()),
            ("subtitle", media.subtitle.is_some()),
            ("artist", media.artist.is_some()),
            ("albumArtist", media.album_artist.is_some()),
            ("albumTitle", media.album_title.is_some()),
            ("trackNumber", media.track_number.is_some()),
            ("albumTrackCount", media.album_track_count.is_some()),
        ];
        Self {
            present_fields: fields
                .into_iter()
                .filter(|(_, present)| *present)
                .map(|(field, _)| field.to_owned())
                .collect(),
            genre_count: media.genres.len(),
            thumbnail_available: media.thumbnail_available,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Snapshot,
    Watch { seconds: u64, interval_ms: u64 },
}

/// Parses the dependency-free probe command line.
///
/// # Errors
///
/// Returns a bounded, user-safe message for an unknown command, option, missing
/// option value, or numeric value outside the documented range.
pub fn parse_command<I, S>(arguments: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let arguments = arguments.into_iter().map(Into::into).collect::<Vec<_>>();
    let Some(command) = arguments.first().map(String::as_str) else {
        return Ok(Command::Snapshot);
    };

    match command {
        "snapshot" => {
            if arguments.len() == 1 {
                Ok(Command::Snapshot)
            } else {
                Err("snapshot does not accept options".to_owned())
            }
        }
        "watch" => parse_watch_options(&arguments[1..]),
        "help" | "--help" | "-h" => Err(usage()),
        other => Err(format!("unknown command '{other}'\n{}", usage())),
    }
}

fn parse_watch_options(arguments: &[String]) -> Result<Command, String> {
    let mut seconds = 60_u64;
    let mut interval_ms = 100_u64;
    let mut index = 0_usize;

    while index < arguments.len() {
        let option = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--seconds" => {
                seconds = parse_bounded(value, "seconds", 1, 3_600)?;
            }
            "--interval-ms" => {
                interval_ms = parse_bounded(value, "interval-ms", 50, 5_000)?;
            }
            _ => return Err(format!("unknown watch option '{option}'")),
        }
        index += 2;
    }

    Ok(Command::Watch {
        seconds,
        interval_ms,
    })
}

fn parse_bounded(value: &str, label: &str, minimum: u64, maximum: u64) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be an integer"))?;
    if (minimum..=maximum).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("{label} must be between {minimum} and {maximum}"))
    }
}

#[must_use]
pub fn usage() -> String {
    "Usage: cyberkindred-gsmtc-probe [snapshot | watch [--seconds 1..3600] [--interval-ms 50..5000]]".to_owned()
}

#[must_use]
pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[must_use]
pub const fn windows_ticks_to_ms(ticks: i64) -> i64 {
    ticks / 10_000
}

#[must_use]
pub fn optional_text(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::snapshot;

#[cfg(not(windows))]
pub fn snapshot() -> Result<ProbeReport, Box<dyn std::error::Error>> {
    Err("GSMTC is available only on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_snapshot() {
        assert_eq!(parse_command(Vec::<String>::new()), Ok(Command::Snapshot));
    }

    #[test]
    fn parses_bounded_watch_options() {
        assert_eq!(
            parse_command(["watch", "--seconds", "10", "--interval-ms", "250"]),
            Ok(Command::Watch {
                seconds: 10,
                interval_ms: 250
            })
        );
    }

    #[test]
    fn rejects_unbounded_polling_options() {
        let result = parse_command(["watch", "--seconds", "0"]);
        assert!(result.is_err());
    }

    #[test]
    fn converts_windows_ticks_to_milliseconds_without_rounding_up() {
        assert_eq!(windows_ticks_to_ms(19_999), 1);
        assert_eq!(windows_ticks_to_ms(-19_999), -1);
    }

    #[test]
    fn empty_text_is_absent_and_non_empty_text_is_preserved() {
        assert_eq!(optional_text(String::new()), None);
        assert_eq!(
            optional_text("  title  ".to_owned()),
            Some("  title  ".to_owned())
        );
    }

    #[test]
    fn watch_observation_redacts_media_text() {
        let report = ProbeReport {
            schema_version: REPORT_SCHEMA_VERSION.to_owned(),
            captured_at_unix_ms: 1,
            mode: "read_only".to_owned(),
            current_session_app_id: Some("app-id".to_owned()),
            sessions: vec![SessionSnapshot {
                source_app_user_model_id: "app-id".to_owned(),
                is_current: true,
                playback_status: Some("playing".to_owned()),
                media: Some(MediaSnapshot {
                    title: Some("private-title-canary".to_owned()),
                    subtitle: None,
                    artist: Some("private-artist-canary".to_owned()),
                    album_artist: None,
                    album_title: None,
                    track_number: None,
                    album_track_count: None,
                    genres: Vec::new(),
                    thumbnail_available: false,
                }),
                timeline: None,
                capabilities: None,
                read_errors: Vec::new(),
            }],
            warnings: Vec::new(),
        };

        let serialized = serde_json::to_string(&ProbeObservation::from(&report))
            .expect("test observation should serialize");
        assert!(!serialized.contains("private-title-canary"));
        assert!(!serialized.contains("private-artist-canary"));
        assert!(serialized.contains("\"presentFields\":[\"title\",\"artist\"]"));
    }
}
