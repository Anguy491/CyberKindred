CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    checksum_sha256 TEXT NOT NULL CHECK(length(checksum_sha256) = 64),
    applied_at_ms INTEGER NOT NULL,
    app_version TEXT NOT NULL
) STRICT;

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL CHECK(json_valid(value_json)),
    schema_version INTEGER NOT NULL CHECK(schema_version > 0),
    updated_at_ms INTEGER NOT NULL,
    CHECK (
        (key LIKE 'ui.%' OR key LIKE 'audio.%' OR key LIKE 'program.%' OR key LIKE 'privacy.%'
         OR key IN ('provider.openai.model', 'provider.openai.base_url', 'os.tray', 'os.autostart', 'os.notifications')
         OR key LIKE 'provider.openai.tts_%')
        AND lower(key) NOT LIKE '%credential%'
        AND lower(key) NOT LIKE '%api_key%'
        AND lower(key) NOT LIKE '%apikey%'
        AND lower(key) NOT LIKE '%token%'
        AND lower(key) NOT LIKE '%password%'
        AND lower(key) NOT LIKE '%authorization%'
        AND lower(key) NOT LIKE '%secret%'
    )
) STRICT;

CREATE TABLE user_profile (
    id TEXT PRIMARY KEY CHECK(id = 'current'),
    display_name TEXT,
    locale TEXT NOT NULL DEFAULT 'zh-CN',
    city TEXT,
    region TEXT,
    country TEXT,
    country_code TEXT CHECK(country_code IS NULL OR (length(country_code) = 2 AND country_code = upper(country_code))),
    city_lat REAL CHECK(city_lat IS NULL OR city_lat BETWEEN -90 AND 90),
    city_lon REAL CHECK(city_lon IS NULL OR city_lon BETWEEN -180 AND 180),
    timezone TEXT NOT NULL,
    routine_json TEXT NOT NULL CHECK(json_valid(routine_json)),
    program_preferences_json TEXT NOT NULL CHECK(json_valid(program_preferences_json)),
    profile_revision INTEGER NOT NULL CHECK(profile_revision >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK ((city_lat IS NULL) = (city_lon IS NULL))
) STRICT;

CREATE TABLE library_roots (
    id TEXT PRIMARY KEY,
    canonical_path TEXT NOT NULL,
    path_key TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    last_scan_at_ms INTEGER
) STRICT;

CREATE TABLE scan_jobs (
    id TEXT PRIMARY KEY,
    root_id TEXT NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'completed', 'cancelled', 'failed', 'interrupted')),
    files_seen INTEGER NOT NULL DEFAULT 0 CHECK(files_seen >= 0),
    tracks_indexed INTEGER NOT NULL DEFAULT 0 CHECK(tracks_indexed >= 0),
    errors_count INTEGER NOT NULL DEFAULT 0 CHECK(errors_count >= 0),
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    error_code TEXT,
    app_version TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX ux_scan_one_active_root ON scan_jobs(root_id) WHERE status IN ('queued', 'running');

CREATE TABLE tracks (
    id TEXT PRIMARY KEY,
    root_id TEXT NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    relative_path_key TEXT NOT NULL,
    file_identity TEXT,
    availability TEXT NOT NULL CHECK(availability IN ('available', 'missing', 'corrupt', 'unsupported')),
    format TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL CHECK(file_size_bytes >= 0),
    modified_at_ms INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
    last_seen_scan_id TEXT REFERENCES scan_jobs(id) ON DELETE SET NULL,
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    track_number INTEGER,
    disc_number INTEGER,
    year INTEGER,
    genre_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(genre_json)),
    metadata_confidence REAL NOT NULL CHECK(metadata_confidence BETWEEN 0 AND 1),
    embedded_cover_hash TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_played_at_ms INTEGER,
    UNIQUE(root_id, relative_path_key)
) STRICT;
CREATE INDEX ix_tracks_root_availability ON tracks(root_id, availability);
CREATE INDEX ix_tracks_artist ON tracks(artist);
CREATE INDEX ix_tracks_album ON tracks(album);
CREATE INDEX ix_tracks_last_played ON tracks(last_played_at_ms);
CREATE INDEX ix_tracks_seen_scan ON tracks(last_seen_scan_id);

CREATE TABLE track_external_metadata (
    id TEXT PRIMARY KEY,
    track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK(provider = 'musicbrainz'),
    external_id TEXT,
    normalized_title TEXT,
    normalized_artist TEXT,
    normalized_album TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(tags_json)),
    confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
    match_status TEXT NOT NULL CHECK(match_status IN ('adopted', 'suggested', 'no_match')),
    response_etag TEXT,
    fetched_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    UNIQUE(track_id, provider)
) STRICT;

CREATE TABLE cover_art_cache (
    cache_key TEXT PRIMARY KEY CHECK(length(cache_key) = 64),
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
    source TEXT NOT NULL CHECK(source IN ('embedded', 'cover_art_archive')),
    external_url TEXT,
    mime_type TEXT NOT NULL,
    width INTEGER CHECK(width IS NULL OR width > 0),
    height INTEGER CHECK(height IS NULL OR height > 0),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    etag TEXT,
    attribution_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(attribution_json)),
    last_accessed_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE cover_art_negative_cache (
    release_mbid TEXT PRIMARY KEY,
    result TEXT NOT NULL CHECK(result = 'not_found'),
    fetched_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    provider_status INTEGER NOT NULL CHECK(provider_status = 404)
) STRICT;

CREATE TABLE tts_cache_entries (
    cache_key TEXT PRIMARY KEY CHECK(length(cache_key) = 64),
    relative_path TEXT NOT NULL UNIQUE,
    model_id TEXT NOT NULL,
    voice_id TEXT NOT NULL,
    speed REAL NOT NULL CHECK(speed BETWEEN 0.75 AND 1.25),
    format TEXT NOT NULL CHECK(format = 'mp3'),
    locale TEXT NOT NULL,
    text_hash TEXT NOT NULL CHECK(length(text_hash) = 64),
    size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 1 AND 20971520),
    duration_ms INTEGER NOT NULL CHECK(duration_ms BETWEEN 1 AND 120000),
    created_at_ms INTEGER NOT NULL,
    last_accessed_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE tts_cache_references (
    cache_key TEXT NOT NULL REFERENCES tts_cache_entries(cache_key) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('program_segment', 'voice_preview')),
    owner_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(cache_key, owner_kind, owner_id)
) STRICT;

CREATE TABLE tts_cache_leases (
    lease_id TEXT PRIMARY KEY,
    cache_key TEXT NOT NULL REFERENCES tts_cache_entries(cache_key) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('program_segment', 'voice_preview')),
    owner_id TEXT NOT NULL,
    process_instance_id TEXT NOT NULL,
    acquired_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms >= acquired_at_ms)
) STRICT;

CREATE TABLE program_runs (
    id TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('local', 'system_session')),
    source_instance_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('planning', 'ready', 'music', 'voice_preparing', 'voice', 'paused', 'degraded', 'completing', 'stopping', 'completed', 'interrupted', 'failed')),
    target_duration_ms INTEGER CHECK(target_duration_ms IS NULL OR target_duration_ms > 0),
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    plan_schema_version INTEGER NOT NULL CHECK(plan_schema_version > 0),
    prompt_version TEXT,
    provider TEXT,
    model TEXT,
    degraded_features_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(degraded_features_json)),
    failure_code TEXT,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX ix_program_runs_started ON program_runs(started_at_ms DESC);
CREATE INDEX ix_program_runs_status ON program_runs(status);

CREATE TABLE program_segments (
    id TEXT PRIMARY KEY,
    program_run_id TEXT NOT NULL REFERENCES program_runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    kind TEXT NOT NULL CHECK(kind IN ('track', 'voice')),
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
    system_media_identity TEXT,
    voice_text TEXT,
    voice_text_hash TEXT,
    tts_cache_key TEXT REFERENCES tts_cache_entries(cache_key) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK(status IN ('planned', 'preparing', 'active', 'completed', 'skipped', 'failed', 'cancelled')),
    started_at_ms INTEGER,
    ended_at_ms INTEGER,
    failure_code TEXT,
    UNIQUE(program_run_id, ordinal),
    CHECK (
        (kind = 'track' AND ((track_id IS NOT NULL AND system_media_identity IS NULL) OR (track_id IS NULL AND system_media_identity IS NOT NULL)) AND voice_text IS NULL)
        OR
        (kind = 'voice' AND track_id IS NULL AND system_media_identity IS NULL AND (voice_text IS NOT NULL OR ended_at_ms IS NOT NULL))
    )
) STRICT;

CREATE TABLE playback_events (
    id TEXT PRIMARY KEY,
    program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('local', 'system_session')),
    source_instance_id TEXT,
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
    system_media_identity TEXT,
    display_title TEXT CHECK(display_title IS NULL OR length(display_title) BETWEEN 1 AND 300),
    display_artist TEXT CHECK(display_artist IS NULL OR length(display_artist) BETWEEN 1 AND 300),
    display_album TEXT CHECK(display_album IS NULL OR length(display_album) BETWEEN 1 AND 300),
    event_type TEXT NOT NULL CHECK(event_type IN ('started', 'paused', 'resumed', 'seeked', 'completed', 'skipped', 'failed', 'source_changed')),
    position_ms INTEGER CHECK(position_ms IS NULL OR position_ms >= 0),
    reason_code TEXT,
    occurred_at_ms INTEGER NOT NULL,
    playback_revision INTEGER NOT NULL CHECK(playback_revision >= 0)
) STRICT;
CREATE INDEX ix_playback_events_run_time ON playback_events(program_run_id, occurred_at_ms);
CREATE INDEX ix_playback_events_track_time ON playback_events(track_id, occurred_at_ms DESC);
CREATE INDEX ix_playback_events_media_time ON playback_events(system_media_identity, occurred_at_ms DESC);

CREATE TABLE feedback (
    id TEXT PRIMARY KEY,
    program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL,
    target_kind TEXT NOT NULL CHECK(target_kind IN ('track', 'voice', 'program')),
    track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,
    system_media_identity TEXT,
    feedback_type TEXT NOT NULL CHECK(feedback_type IN ('like', 'skip', 'less_talk', 'more_like_this')),
    value_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(value_json)),
    created_at_ms INTEGER NOT NULL,
    revoked_at_ms INTEGER
) STRICT;
CREATE INDEX ix_feedback_track_active ON feedback(track_id, feedback_type, revoked_at_ms);
CREATE INDEX ix_feedback_time ON feedback(created_at_ms DESC);

CREATE TABLE chat_sessions (
    id TEXT PRIMARY KEY,
    program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    covered_from_ms INTEGER,
    covered_to_ms INTEGER,
    user_message_count INTEGER NOT NULL DEFAULT 0 CHECK(user_message_count >= 0),
    assistant_message_count INTEGER NOT NULL DEFAULT 0 CHECK(assistant_message_count >= 0),
    status TEXT NOT NULL CHECK(status IN ('active', 'completed', 'interrupted')),
    CHECK ((covered_from_ms IS NULL AND covered_to_ms IS NULL) OR (covered_from_ms IS NOT NULL AND covered_to_ms IS NOT NULL AND covered_from_ms <= covered_to_ms))
) STRICT;
CREATE UNIQUE INDEX ux_chat_session_program ON chat_sessions(program_run_id) WHERE program_run_id IS NOT NULL;

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    chat_session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
    content_text TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
    provider TEXT,
    model TEXT,
    prompt_version TEXT,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    deletion_reason TEXT,
    CHECK(expires_at_ms = created_at_ms + 2592000000)
) STRICT;
CREATE INDEX ix_messages_session_time ON messages(chat_session_id, created_at_ms);
CREATE INDEX ix_messages_expiry ON messages(expires_at_ms);

CREATE TABLE session_summaries (
    id TEXT PRIMARY KEY,
    chat_session_id TEXT NOT NULL UNIQUE REFERENCES chat_sessions(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK(status IN ('active', 'deleted')),
    summary_text TEXT,
    source_from_ms INTEGER,
    source_to_ms INTEGER,
    source_hash TEXT,
    generation_kind TEXT CHECK(generation_kind IN ('llm', 'deterministic')),
    preference_signals_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(preference_signals_json)),
    provider TEXT,
    model TEXT,
    prompt_version TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER,
    CHECK (
        (status = 'active' AND length(summary_text) BETWEEN 1 AND 1000 AND source_from_ms IS NOT NULL AND source_to_ms IS NOT NULL AND source_from_ms <= source_to_ms AND source_hash IS NOT NULL AND generation_kind IS NOT NULL AND prompt_version IS NOT NULL AND deleted_at_ms IS NULL)
        OR
        (status = 'deleted' AND summary_text IS NULL AND source_hash IS NULL AND preference_signals_json = '[]' AND provider IS NULL AND model IS NULL AND prompt_version IS NULL AND deleted_at_ms IS NOT NULL)
    )
) STRICT;

CREATE TABLE memory_proposals (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL CHECK(category IN ('preference', 'routine', 'boundary', 'biographical')),
    statement_text TEXT,
    statement_hash TEXT NOT NULL CHECK(length(statement_hash) = 64),
    reason_text TEXT,
    confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
    status TEXT NOT NULL CHECK(status IN ('proposed', 'approved', 'rejected', 'superseded')),
    created_at_ms INTEGER NOT NULL,
    decided_at_ms INTEGER,
    prompt_version TEXT NOT NULL,
    model TEXT,
    CHECK(status NOT IN ('proposed', 'approved') OR statement_text IS NOT NULL)
) STRICT;

CREATE TABLE memory_proposal_sources (
    proposal_id TEXT NOT NULL REFERENCES memory_proposals(id) ON DELETE CASCADE,
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    source_content_hash TEXT NOT NULL CHECK(length(source_content_hash) = 64),
    PRIMARY KEY(proposal_id, source_content_hash)
) STRICT;

CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL CHECK(category IN ('preference', 'routine', 'boundary', 'biographical')),
    current_revision INTEGER NOT NULL CHECK(current_revision > 0),
    status TEXT NOT NULL CHECK(status IN ('approved', 'disabled', 'deleted')),
    pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0, 1)),
    created_from_proposal_id TEXT REFERENCES memory_proposals(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER
) STRICT;

CREATE TABLE memory_revisions (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    statement_text TEXT,
    statement_hash TEXT NOT NULL CHECK(length(statement_hash) = 64),
    change_kind TEXT NOT NULL CHECK(change_kind IN ('approved', 'edited', 'disabled', 'reenabled', 'deleted')),
    changed_at_ms INTEGER NOT NULL,
    PRIMARY KEY(memory_id, revision)
) STRICT;

CREATE TABLE schedule_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    timezone TEXT NOT NULL,
    days_of_week_json TEXT NOT NULL CHECK(json_valid(days_of_week_json)),
    local_time TEXT NOT NULL CHECK(length(local_time) = 5 AND local_time GLOB '[0-2][0-9]:[0-5][0-9]' AND substr(local_time, 1, 2) < '24'),
    notification_only INTEGER NOT NULL DEFAULT 1 CHECK(notification_only = 1),
    next_occurrence_at_ms INTEGER,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE schedule_occurrences (
    id TEXT PRIMARY KEY,
    rule_id TEXT NOT NULL REFERENCES schedule_rules(id) ON DELETE CASCADE,
    occurrence_key TEXT NOT NULL,
    due_at_ms INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('due', 'awaiting_user', 'snoozed', 'starting', 'consumed', 'dismissed', 'missed')),
    snooze_minutes INTEGER CHECK(snooze_minutes IN (10, 30, 60)),
    snoozed_until_ms INTEGER,
    notification_id TEXT,
    program_run_id TEXT REFERENCES program_runs(id) ON DELETE SET NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(rule_id, occurrence_key)
) STRICT;
CREATE INDEX ix_occurrence_status_due ON schedule_occurrences(status, due_at_ms);

CREATE TABLE os_integration_state (
    id TEXT PRIMARY KEY CHECK(id = 'current'),
    notifications_permission TEXT NOT NULL CHECK(notifications_permission IN ('unknown', 'granted', 'denied', 'unavailable')),
    autostart_enabled INTEGER NOT NULL CHECK(autostart_enabled IN (0, 1)),
    tray_enabled INTEGER NOT NULL CHECK(tray_enabled IN (0, 1)),
    last_suspend_at_ms INTEGER,
    last_resume_at_ms INTEGER,
    scheduler_checked_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE weather_cache (
    cache_key TEXT PRIMARY KEY,
    city TEXT NOT NULL,
    region TEXT,
    country TEXT NOT NULL,
    country_code TEXT NOT NULL CHECK(length(country_code) = 2 AND country_code = upper(country_code)),
    latitude REAL NOT NULL CHECK(latitude BETWEEN -90 AND 90),
    longitude REAL NOT NULL CHECK(longitude BETWEEN -180 AND 180),
    timezone TEXT NOT NULL,
    request_shape_hash TEXT NOT NULL CHECK(length(request_shape_hash) = 64),
    weather_json TEXT NOT NULL CHECK(json_valid(weather_json)),
    fetched_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    delete_after_ms INTEGER NOT NULL CHECK(delete_after_ms >= expires_at_ms)
) STRICT;
CREATE INDEX ix_weather_expiry ON weather_cache(expires_at_ms);

CREATE TABLE provider_usage (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    request_kind TEXT NOT NULL,
    model TEXT,
    input_units INTEGER CHECK(input_units IS NULL OR input_units >= 0),
    output_units INTEGER CHECK(output_units IS NULL OR output_units >= 0),
    audio_seconds REAL CHECK(audio_seconds IS NULL OR audio_seconds >= 0),
    latency_ms INTEGER NOT NULL CHECK(latency_ms >= 0),
    status_class TEXT NOT NULL,
    correlation_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX ix_usage_provider_time ON provider_usage(provider, created_at_ms);

CREATE TABLE outbox_events (
    id TEXT PRIMARY KEY,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    aggregate_revision INTEGER NOT NULL CHECK(aggregate_revision >= 0),
    event_type TEXT NOT NULL CHECK(event_type != 'cyberkindred://v1/chat/message'),
    payload_json TEXT NOT NULL CHECK(
        json_valid(payload_json)
        AND json_type(payload_json, '$.text') IS NULL
        AND json_type(payload_json, '$.content') IS NULL
        AND json_type(payload_json, '$.summary') IS NULL
        AND json_type(payload_json, '$.secret') IS NULL
        AND json_type(payload_json, '$.path') IS NULL
        AND json_type(payload_json, '$.statement') IS NULL
    ),
    created_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER,
    UNIQUE(aggregate_type, aggregate_id, aggregate_revision, event_type)
) STRICT;
