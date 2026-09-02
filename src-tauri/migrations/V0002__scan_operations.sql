CREATE TABLE scan_operations (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK(status IN ('accepted', 'running', 'completed', 'cancelled', 'failed', 'interrupted')),
    files_seen INTEGER NOT NULL DEFAULT 0 CHECK(files_seen BETWEEN 0 AND 9007199254740991),
    tracks_indexed INTEGER NOT NULL DEFAULT 0 CHECK(tracks_indexed BETWEEN 0 AND 9007199254740991),
    errors_count INTEGER NOT NULL DEFAULT 0 CHECK(errors_count BETWEEN 0 AND 9007199254740991),
    started_at_ms INTEGER CHECK(started_at_ms IS NULL OR started_at_ms >= 0),
    finished_at_ms INTEGER CHECK(finished_at_ms IS NULL OR finished_at_ms >= 0),
    error_code TEXT CHECK(error_code IS NULL OR error_code IN ('scan_failed', 'interrupted')),
    app_version TEXT NOT NULL CHECK(length(app_version) BETWEEN 1 AND 100),
    CHECK (
        (status = 'accepted' AND started_at_ms IS NULL AND finished_at_ms IS NULL AND error_code IS NULL)
        OR (status = 'running' AND started_at_ms IS NOT NULL AND finished_at_ms IS NULL AND error_code IS NULL)
        OR (status = 'completed' AND started_at_ms IS NOT NULL AND finished_at_ms IS NOT NULL AND error_code IS NULL)
        OR (status = 'cancelled' AND finished_at_ms IS NOT NULL AND error_code IS NULL)
        OR (status = 'failed' AND finished_at_ms IS NOT NULL AND error_code = 'scan_failed')
        OR (status = 'interrupted' AND finished_at_ms IS NOT NULL AND error_code = 'interrupted')
    )
) STRICT;
CREATE INDEX ix_scan_operations_status ON scan_operations(status, id);

CREATE UNIQUE INDEX ux_scan_jobs_id_root ON scan_jobs(id, root_id);

CREATE TABLE scan_operation_roots (
    operation_id TEXT NOT NULL REFERENCES scan_operations(id) ON DELETE CASCADE,
    root_id TEXT NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
    scan_job_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY(operation_id, root_id),
    FOREIGN KEY(scan_job_id, root_id) REFERENCES scan_jobs(id, root_id) ON DELETE CASCADE
) STRICT;
CREATE INDEX ix_scan_operation_roots_root ON scan_operation_roots(root_id, operation_id);
