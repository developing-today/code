-- fleet-kv central plane, migration 3: the blob plane's central rows
-- (#13318, design §8 to §10).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- Published objects, per dedup partition (jurisdiction, tier). Only the
-- service's verification step writes a row here, and only as published:
-- staged and verified bytes live in the endpoint's staging area and never
-- reach central. A fenced GC delete removes the row.
CREATE TABLE IF NOT EXISTS kv_blobs (
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    content_id TEXT NOT NULL,
    bytes_len INTEGER NOT NULL CHECK (bytes_len >= 0),
    locator TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state = 'published'),
    published_at_ms INTEGER NOT NULL,
    PRIMARY KEY (jurisdiction, tier, content_id)
) WITHOUT ROWID;

-- The GC roots: one row for every publish-CAS that named a blob, whether
-- it committed (a head and a retained version) or conflicted (a proposed
-- ref kept in kv_conflicts). Written in the same transaction as the CAS.
CREATE TABLE IF NOT EXISTS kv_blob_refs (
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    content_id TEXT NOT NULL,
    writer TEXT NOT NULL,
    host_incarnation TEXT NOT NULL,
    outbox_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    registered_at_ms INTEGER NOT NULL,
    PRIMARY KEY (jurisdiction, tier, content_id, writer, host_incarnation, outbox_id)
) WITHOUT ROWID;

-- The delete fence and its audit trail: one row per fenced delete. The
-- epoch rises by one per partition, and the row is written in the same
-- transaction as the delete, after re-verifying that no root names the
-- content.
CREATE TABLE IF NOT EXISTS kv_blob_deletions (
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 1),
    content_id TEXT NOT NULL,
    bytes_len INTEGER NOT NULL CHECK (bytes_len >= 0),
    candidate_since_ms INTEGER NOT NULL,
    deleted_at_ms INTEGER NOT NULL,
    attempt TEXT NOT NULL UNIQUE,
    PRIMARY KEY (jurisdiction, tier, epoch)
) WITHOUT ROWID;
