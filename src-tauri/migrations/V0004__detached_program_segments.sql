CREATE TABLE program_segments_v4 (
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
        (kind = 'track' AND voice_text IS NULL AND (
            (track_id IS NOT NULL AND system_media_identity IS NULL)
            OR (track_id IS NULL AND system_media_identity IS NOT NULL)
            OR (track_id IS NULL AND system_media_identity IS NULL AND status IN ('completed', 'skipped', 'failed', 'cancelled'))
        ))
        OR
        (kind = 'voice' AND track_id IS NULL AND system_media_identity IS NULL AND (voice_text IS NOT NULL OR ended_at_ms IS NOT NULL))
    )
) STRICT;

INSERT INTO program_segments_v4 SELECT * FROM program_segments;
DROP TABLE program_segments;
ALTER TABLE program_segments_v4 RENAME TO program_segments;
