-- 0001_reference_record.sql
--
-- Reference record schema for the content-addressed media/data plane
-- (component F of #9078, issue #12803, slice 1 of #9084; design record
-- doc/2026-11-23T03-04-00Z_design_content-addressed-media-data-plane,
-- sections 4-6).
--
-- This migration runs in the TENANT and PUBLIC databases of component
-- E's data plane (#9083 design section 2.2) -- one copy per database
-- that can hold references. It is NOT part of the central tenancy
-- schema (tenancy/migrations/, #12796): reference rows are tenant
-- data. The central DB never holds them; its publish_record table is
-- the central-plane projection of publication, not the reference
-- itself. The references ARE the rows in the tenant and public
-- databases; GC reads them (design section 6), there is no separate
-- reference registry.
--
-- A row carries exactly what the design requires (section "For
-- component E"): content ID, partition, class, created-by, tombstone
-- state.
--
--   content_id        the #9106 Part A content address (sha256, 64
--                     lowercase hex, the shape the landed `blobs`
--                     table in src/user-do.ts already uses). An
--                     identifier, never a credential: a row's
--                     existence may be learnable, bytes are served
--                     only after the access check in src/references.ts.
--   jurisdiction +    the dedup partition (#3401). Content never
--   sensitivity_tier  dedups across partitions and a reference never
--                     grants outside its own partition, so both
--                     halves are stored on the row and checked at
--                     resolution time (design section 5).
--   class             what this reference authorizes. `standard` is
--                     the ordinary tenant reference; `restricted`
--                     is the restricted class capability minting
--                     additionally requires for tenant-restricted
--                     content, set only by an authorized writer of
--                     the source content; `publication` marks a
--                     publication reference and is only valid in the
--                     public database at tier `public`.
--   created_by        binding_id of the writer (32 lowercase hex,
--                     the pkgs/id hosted binding shape). References
--                     are written only through the hosting database's
--                     authorized write path (design section 5).
--   tombstone state   tombstoned_at NULL means live. Removal
--                     tombstones; rows are never hard-deleted here,
--                     because the tombstone is the authoritative
--                     state the GC sweep re-verifies against and
--                     the audit trail of who lost access when.
--
-- The outbox is the contract the derived reference/location index
-- (#12805) and the GC worker (#12806) consume: one event per create
-- and per remove, appended with the reference change itself.
-- Consumers page by `seq` and dedupe by `event_id` (at-least-once).
-- The index built from these events is a cache; this table and
-- content_reference are the authority.

CREATE TABLE IF NOT EXISTS content_reference (
  reference_id     TEXT PRIMARY KEY,
  content_id       TEXT NOT NULL,
  jurisdiction     TEXT NOT NULL,
  sensitivity_tier TEXT NOT NULL,
  class            TEXT NOT NULL,
  created_by       TEXT NOT NULL,
  created_at       TEXT NOT NULL,
  tombstoned_at    TEXT,
  CHECK (jurisdiction IN ('us','eu','fedramp')),
  CHECK (sensitivity_tier IN ('public','tenant-private','tenant-restricted','staging')),
  CHECK (class IN ('standard','restricted','publication'))
);

-- Referencing is idempotent (design section 6): at most one LIVE
-- reference per (content, partition, class, writer). Re-creating an
-- existing live reference returns the existing row and emits no
-- second outbox event.
CREATE UNIQUE INDEX IF NOT EXISTS content_reference_one_live
  ON content_reference (content_id, jurisdiction, sensitivity_tier, class, created_by)
  WHERE tombstoned_at IS NULL;

-- The access check and the GC mark phase both scan by content within
-- a partition.
CREATE INDEX IF NOT EXISTS content_reference_by_content
  ON content_reference (content_id, jurisdiction, sensitivity_tier);

CREATE TABLE IF NOT EXISTS reference_outbox (
  seq              INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id         TEXT NOT NULL UNIQUE,
  event_type       TEXT NOT NULL,
  reference_id     TEXT NOT NULL,
  content_id       TEXT NOT NULL,
  jurisdiction     TEXT NOT NULL,
  sensitivity_tier TEXT NOT NULL,
  class            TEXT NOT NULL,
  actor            TEXT NOT NULL,
  occurred_at      TEXT NOT NULL,
  CHECK (event_type IN ('reference.created','reference.removed'))
);
