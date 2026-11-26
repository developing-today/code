-- fleet-kv central plane, migration 7: purging deleted blob bytes (#13318,
-- design §9, §10).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- A fenced delete retires the central record; the object bytes go later,
-- in the door that holds the bucket. A deletion row is purged once the
-- bytes under its content key are gone.
ALTER TABLE kv_blob_deletions ADD COLUMN purged_at_ms INTEGER;

-- A purge in flight. While a content id is claimed here no upload may
-- publish it again, and a claim is taken only while no kv_blobs row names
-- the content, so a purge never removes bytes a published row needs. A
-- claim older than the purge lease may be taken over; finishing or
-- releasing a claim is gated on its attempt.
CREATE TABLE IF NOT EXISTS kv_blob_purges (
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    content_id TEXT NOT NULL,
    attempt TEXT NOT NULL,
    claimed_ms INTEGER NOT NULL,
    PRIMARY KEY (jurisdiction, tier, content_id)
) WITHOUT ROWID;
