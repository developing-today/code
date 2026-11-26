-- fleet-kv central plane, migration 2: the survival archive (#13316,
-- design §1 and §7).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- The latest archived dirty set of each full-identity partition. A
-- checkpoint replaces the partition's rows in one transaction. Only
-- inline payloads up to the effective threshold are archived; a key
-- whose bytes are over it is recorded as 'unprotected' with no bytes,
-- so restore can say what the archive could not keep. Restore and
-- audit only: nothing reads these rows as key-value heads.
CREATE TABLE IF NOT EXISTS kv_archive (
    owner_user_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    partition_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    state TEXT NOT NULL CHECK (state IN ('live', 'tombstone', 'unprotected')),
    value BLOB NOT NULL,
    host_incarnation TEXT NOT NULL,
    outbox_id INTEGER NOT NULL,
    archived_at_ms INTEGER NOT NULL,
    PRIMARY KEY (owner_user_id, context_id, partition_id, namespace, key)
) WITHOUT ROWID;

-- One row per partition: when its last checkpoint landed and what it held.
CREATE TABLE IF NOT EXISTS kv_archive_checkpoints (
    owner_user_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    partition_id TEXT NOT NULL,
    host_incarnation TEXT NOT NULL,
    checkpoint_at_ms INTEGER NOT NULL,
    archived INTEGER NOT NULL CHECK (archived >= 0),
    unprotected INTEGER NOT NULL CHECK (unprotected >= 0),
    PRIMARY KEY (owner_user_id, context_id, partition_id)
) WITHOUT ROWID;
