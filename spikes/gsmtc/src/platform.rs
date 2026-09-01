use crate::{
    CapabilitySnapshot, FieldReadError, MediaSnapshot, ProbeReport, REPORT_SCHEMA_VERSION,
    SessionSnapshot, TimelineSnapshot, now_unix_ms, optional_text, windows_ticks_to_ms,
};
use std::error::Error;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

/// Reads one point-in-time snapshot from the system media session manager.
///
/// # Errors
///
/// Returns a Windows error when the manager or session collection cannot be
/// obtained. Per-session optional field failures are retained in `read_errors`.
pub fn snapshot() -> Result<ProbeReport, Box<dyn Error>> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
    let current_session_app_id = manager
        .GetCurrentSession()
        .ok()
        .and_then(|session| session.SourceAppUserModelId().ok())
        .map(|value| value.to_string_lossy());
    let sessions = manager.GetSessions()?;
    let mut snapshots = Vec::with_capacity(sessions.Size()? as usize);

    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        snapshots.push(read_session(&session, current_session_app_id.as_deref())?);
    }

    snapshots.sort_by(|left, right| {
        left.source_app_user_model_id
            .cmp(&right.source_app_user_model_id)
    });

    Ok(ProbeReport {
        schema_version: REPORT_SCHEMA_VERSION.to_owned(),
        captured_at_unix_ms: now_unix_ms(),
        mode: "read_only".to_owned(),
        current_session_app_id,
        sessions: snapshots,
        warnings: vec![
            "Media text is local diagnostic data; do not commit raw output.".to_owned(),
            "No playback-control method is called by this probe.".to_owned(),
        ],
    })
}

fn read_session(
    session: &GlobalSystemMediaTransportControlsSession,
    current_session_app_id: Option<&str>,
) -> Result<SessionSnapshot, Box<dyn Error>> {
    let source_app_user_model_id = session.SourceAppUserModelId()?.to_string_lossy();
    let mut read_errors = Vec::new();
    let (playback_status, capabilities) = read_playback(session, &mut read_errors);
    let media = read_media(session, &mut read_errors);
    let timeline = read_timeline(session, &mut read_errors);

    Ok(SessionSnapshot {
        is_current: current_session_app_id == Some(source_app_user_model_id.as_str()),
        source_app_user_model_id,
        playback_status,
        media,
        timeline,
        capabilities,
        read_errors,
    })
}

fn read_playback(
    session: &GlobalSystemMediaTransportControlsSession,
    read_errors: &mut Vec<FieldReadError>,
) -> (Option<String>, Option<CapabilitySnapshot>) {
    match session.GetPlaybackInfo() {
        Ok(playback) => {
            let status = read_field(
                "playback.status",
                playback.PlaybackStatus().map(playback_status_name),
                read_errors,
            );
            let capabilities = match playback.Controls() {
                Ok(controls) => Some(CapabilitySnapshot {
                    play: read_field("capability.play", controls.IsPlayEnabled(), read_errors),
                    pause: read_field("capability.pause", controls.IsPauseEnabled(), read_errors),
                    stop: read_field("capability.stop", controls.IsStopEnabled(), read_errors),
                    next: read_field("capability.next", controls.IsNextEnabled(), read_errors),
                    previous: read_field(
                        "capability.previous",
                        controls.IsPreviousEnabled(),
                        read_errors,
                    ),
                    seek: read_field(
                        "capability.seek",
                        controls.IsPlaybackPositionEnabled(),
                        read_errors,
                    ),
                    play_pause_toggle: read_field(
                        "capability.playPauseToggle",
                        controls.IsPlayPauseToggleEnabled(),
                        read_errors,
                    ),
                    playback_rate: read_field(
                        "capability.playbackRate",
                        controls.IsPlaybackRateEnabled(),
                        read_errors,
                    ),
                    shuffle: read_field(
                        "capability.shuffle",
                        controls.IsShuffleEnabled(),
                        read_errors,
                    ),
                    repeat: read_field(
                        "capability.repeat",
                        controls.IsRepeatEnabled(),
                        read_errors,
                    ),
                }),
                Err(error) => {
                    push_error("playback.controls", &error, read_errors);
                    None
                }
            };
            (status, capabilities)
        }
        Err(error) => {
            push_error("playback", &error, read_errors);
            (None, None)
        }
    }
}

fn read_media(
    session: &GlobalSystemMediaTransportControlsSession,
    read_errors: &mut Vec<FieldReadError>,
) -> Option<MediaSnapshot> {
    match session
        .TryGetMediaPropertiesAsync()
        .and_then(|operation| operation.join())
    {
        Ok(properties) => Some(read_media_properties(&properties, read_errors)),
        Err(error) => {
            push_error("media", &error, read_errors);
            None
        }
    }
}

fn read_media_properties(
    properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
    read_errors: &mut Vec<FieldReadError>,
) -> MediaSnapshot {
    MediaSnapshot {
        title: read_text("media.title", properties.Title(), read_errors),
        subtitle: read_text("media.subtitle", properties.Subtitle(), read_errors),
        artist: read_text("media.artist", properties.Artist(), read_errors),
        album_artist: read_text("media.albumArtist", properties.AlbumArtist(), read_errors),
        album_title: read_text("media.albumTitle", properties.AlbumTitle(), read_errors),
        track_number: read_positive_i32("media.trackNumber", properties.TrackNumber(), read_errors),
        album_track_count: read_positive_i32(
            "media.albumTrackCount",
            properties.AlbumTrackCount(),
            read_errors,
        ),
        genres: read_genres(properties, read_errors),
        thumbnail_available: properties.Thumbnail().is_ok(),
    }
}

fn read_genres(
    properties: &GlobalSystemMediaTransportControlsSessionMediaProperties,
    read_errors: &mut Vec<FieldReadError>,
) -> Vec<String> {
    let Ok(values) = properties.Genres() else {
        read_errors.push(FieldReadError {
            field: "media.genres".to_owned(),
            message: "genre collection unavailable".to_owned(),
        });
        return Vec::new();
    };
    let Ok(size) = values.Size() else {
        read_errors.push(FieldReadError {
            field: "media.genres".to_owned(),
            message: "genre collection size unavailable".to_owned(),
        });
        return Vec::new();
    };

    let mut result = Vec::with_capacity(size as usize);
    for index in 0..size {
        match values.GetAt(index) {
            Ok(value) => result.push(value.to_string_lossy()),
            Err(error) => {
                push_error("media.genres", &error, read_errors);
                break;
            }
        }
    }
    result
}

fn read_timeline(
    session: &GlobalSystemMediaTransportControlsSession,
    read_errors: &mut Vec<FieldReadError>,
) -> Option<TimelineSnapshot> {
    match session.GetTimelineProperties() {
        Ok(value) => {
            if let (
                Ok(start),
                Ok(end),
                Ok(min_seek),
                Ok(max_seek),
                Ok(position),
                Ok(last_updated),
            ) = (
                value.StartTime(),
                value.EndTime(),
                value.MinSeekTime(),
                value.MaxSeekTime(),
                value.Position(),
                value.LastUpdatedTime(),
            ) {
                Some(TimelineSnapshot {
                    start_ms: windows_ticks_to_ms(start.Duration),
                    end_ms: windows_ticks_to_ms(end.Duration),
                    min_seek_ms: windows_ticks_to_ms(min_seek.Duration),
                    max_seek_ms: windows_ticks_to_ms(max_seek.Duration),
                    position_ms: windows_ticks_to_ms(position.Duration),
                    last_updated_windows_ticks: last_updated.UniversalTime,
                })
            } else {
                read_errors.push(FieldReadError {
                    field: "timeline".to_owned(),
                    message: "one or more timeline fields could not be read".to_owned(),
                });
                None
            }
        }
        Err(error) => {
            push_error("timeline", &error, read_errors);
            None
        }
    }
}

fn read_positive_i32(
    field: &str,
    result: windows::core::Result<i32>,
    errors: &mut Vec<FieldReadError>,
) -> Option<i32> {
    read_field(field, result, errors).filter(|value| *value > 0)
}

fn read_text(
    field: &str,
    result: windows::core::Result<windows::core::HSTRING>,
    errors: &mut Vec<FieldReadError>,
) -> Option<String> {
    read_field(field, result.map(|value| value.to_string_lossy()), errors).and_then(optional_text)
}

fn read_field<T>(
    field: &str,
    result: windows::core::Result<T>,
    errors: &mut Vec<FieldReadError>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            push_error(field, &error, errors);
            None
        }
    }
}

fn push_error(field: &str, error: &windows::core::Error, errors: &mut Vec<FieldReadError>) {
    errors.push(FieldReadError {
        field: field.to_owned(),
        message: format!("HRESULT {}", error.code()),
    });
}

fn playback_status_name(status: GlobalSystemMediaTransportControlsSessionPlaybackStatus) -> String {
    match status {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed => "closed",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Opened => "opened",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Changing => "changing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped => "stopped",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => "playing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => "paused",
        _ => "unknown",
    }
    .to_owned()
}
