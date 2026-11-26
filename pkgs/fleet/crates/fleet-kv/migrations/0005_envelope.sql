-- fleet-kv central plane, migration 5: the row envelope (#13318, design
-- §11, D015).
--
-- Same loader rules as migration 1: every statement ends with a
-- semicolon at the end of a line, and no statement holds one elsewhere.

-- Every committed version names its envelope: inline bytes (kv:raw-v1)
-- or a published blob reference (kv:ref-v1). Rows from before this
-- migration and every plain CAS are inline bytes; only a publish-CAS
-- writes kv:ref-v1. A head's envelope is the envelope of its version row,
-- so a head written by any client, old or new, reads with the right one.
-- Readers refuse an envelope they do not know.
ALTER TABLE kv_versions ADD COLUMN envelope TEXT NOT NULL DEFAULT 'kv:raw-v1';

-- A kept conflict body names the envelope its writer proposed.
ALTER TABLE kv_conflicts ADD COLUMN proposed_envelope TEXT NOT NULL DEFAULT 'kv:raw-v1';
