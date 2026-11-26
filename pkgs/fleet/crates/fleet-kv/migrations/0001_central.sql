-- fleet-kv central plane, migration 1 (#13316, design D007/D014).
--
-- Applied by `central::migrate` one statement at a time inside one
-- transaction, together with its `kv_central_migrations` row. Every
-- statement ends with a semicolon at the end of a line and no statement
-- contains a semicolon anywhere else: the loader splits on that.

-- Namespace definitions under one registry handle (`fleet-kv-central`).
-- A shared namespace names its one shared scope; a private namespace
-- resolves to a per-(owner, context) scope at the caller, so its
-- shared_scope_id is NULL. policy_epoch only grows.
CREATE TABLE IF NOT EXISTS kv_namespaces (
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('shared', 'private')),
    shared_scope_id TEXT,
    core_binding TEXT,
    global_flag INTEGER NOT NULL DEFAULT 0 CHECK (global_flag IN (0, 1)),
    blob_threshold INTEGER NOT NULL CHECK (blob_threshold >= 0),
    policy_epoch INTEGER NOT NULL CHECK (policy_epoch >= 1),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (registry_id, namespace),
    CHECK ((scope_kind = 'shared') = (shared_scope_id IS NOT NULL))
) WITHOUT ROWID;

-- The current head of every central key. Version 0 means absent and is
-- never stored; the first committed write is version 1.
CREATE TABLE IF NOT EXISTS kv_heads (
    scope_id TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    state TEXT NOT NULL CHECK (state IN ('live', 'tombstone')),
    value BLOB NOT NULL,
    writer TEXT NOT NULL,
    written_at_ms INTEGER NOT NULL,
    PRIMARY KEY (scope_id, registry_id, namespace, key)
) WITHOUT ROWID;

-- Append-only history: one row per committed CAS.
CREATE TABLE IF NOT EXISTS kv_versions (
    scope_id TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 1),
    parent_version INTEGER NOT NULL CHECK (parent_version = version - 1),
    state TEXT NOT NULL CHECK (state IN ('live', 'tombstone')),
    value BLOB NOT NULL,
    writer TEXT NOT NULL,
    host_incarnation TEXT NOT NULL,
    outbox_id TEXT NOT NULL,
    written_at_ms INTEGER NOT NULL,
    PRIMARY KEY (scope_id, registry_id, namespace, key, version)
) WITHOUT ROWID;

-- Append-only: one row per failed CAS. The proposed body stays
-- retrievable beside the winner's version.
CREATE TABLE IF NOT EXISTS kv_conflicts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_id TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    expected_version INTEGER NOT NULL,
    actual_version INTEGER NOT NULL,
    proposed_state TEXT NOT NULL CHECK (proposed_state IN ('live', 'tombstone')),
    proposed_value BLOB NOT NULL,
    writer TEXT NOT NULL,
    host_incarnation TEXT NOT NULL,
    outbox_id TEXT NOT NULL,
    written_at_ms INTEGER NOT NULL,
    UNIQUE (writer, host_incarnation, outbox_id)
);

CREATE INDEX IF NOT EXISTS kv_conflicts_by_key
    ON kv_conflicts (scope_id, registry_id, namespace, key, id);

-- Idempotency receipts, written in the same transaction as the CAS.
-- `attempt` is a fresh nonce per transaction: only the transaction that
-- inserted the receipt applies its outcome, so a retransmit can never
-- apply twice.
CREATE TABLE IF NOT EXISTS idem_receipts (
    writer TEXT NOT NULL,
    host_incarnation TEXT NOT NULL,
    outbox_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    attempt TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    expected_version INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('committed', 'conflict')),
    version INTEGER NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    PRIMARY KEY (writer, host_incarnation, outbox_id)
) WITHOUT ROWID;

-- Explicit grants for group databases: a group database exists only if
-- a row here authorizes it, one row per allowed namespace.
CREATE TABLE IF NOT EXISTS kv_grants (
    group_id TEXT NOT NULL,
    database_name TEXT NOT NULL,
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    allowed_ops TEXT NOT NULL,
    policy_epoch INTEGER NOT NULL CHECK (policy_epoch >= 1),
    PRIMARY KEY (group_id, database_name, registry_id, namespace)
) WITHOUT ROWID;
