-- fleet-kv central plane, migration 4: each namespace's blob placement
-- policy (#13318, design §8 to §10, acceptance item 6).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- The dedup partition (jurisdiction, sensitivity tier, #3401) that every
-- blob of a namespace is placed in. A namespace with no row has no
-- placement: its over-threshold values are refused at the upload door and
-- wait on the host. There is no default partition. A row exists only
-- beside its namespace's definition, and changing it moves new content
-- only, never a published reference.
CREATE TABLE IF NOT EXISTS kv_namespace_placement (
    registry_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    jurisdiction TEXT NOT NULL,
    tier TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (registry_id, namespace)
) WITHOUT ROWID;
