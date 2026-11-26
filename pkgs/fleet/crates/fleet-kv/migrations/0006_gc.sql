-- fleet-kv central plane, migration 6: GC candidacy (#13318, design §10).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- A published object that no root names, and since when. A GC pass marks
-- unrooted objects here, clears a row as soon as a root names its content
-- again (or the object is gone), and fenced-deletes only rows whose grace
-- has run out. The fenced delete drops the row in its own transaction.
CREATE TABLE IF NOT EXISTS kv_blob_candidates (
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    content_id TEXT NOT NULL,
    since_ms INTEGER NOT NULL,
    PRIMARY KEY (jurisdiction, tier, content_id)
) WITHOUT ROWID;
