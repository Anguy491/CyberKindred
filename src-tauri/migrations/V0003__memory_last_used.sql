ALTER TABLE memories ADD COLUMN last_used_at_ms INTEGER;
ALTER TABLE memory_proposals ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0);
