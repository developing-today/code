# Metadata version conformance fixtures (#9095)

The conformance fixture set for the Metadata schema v1 + evolution
doctrine specified in [`../note.md`](../note.md). Acceptance item 4 of
#9095: _metadata at every historical version parses and is usable by
the current reader_, in the fixture-family practice of #8525–#8528 —
hand-authored cases with recorded expectations, consumed by tests in
the crates that own the readers.

Only one version of each surface has ever been published (PM
`pm_meta: 1`; Atlas `schema_version: 1`), so "every historical
version" is v1 in all of its historical _shapes_: the canonical form,
the pre-doctrine producer shapes still carried by live data, and the
doctrine edge cases those shapes must survive.

## Files

- **`pm-metadata-v1.json`** — PM issue/PR Metadata cases. Each case
  carries a full issue body, a `reader_parse` expectation (`ok` /
  `reject`), and the top-level keys the reader must preserve.
  Asserted executably by the fleet crate conformance test
  (`pkgs/fleet/crates/fleet/src/gh_metadata.rs`,
  `pm_metadata_version_conformance_fixtures`), which runs every case
  through `parse_metadata_block` and the stamp assembly round-trip.
- **`atlas-metadata-v1.json`** — Atlas corpus metadata cases. Cases
  marked `executable: true` carry a `stage_row` and are asserted by
  the atlas-core conformance test
  (`pkgs/atlas-search/crates/atlas-core/tests/metadata_conformance.rs`):
  the row is inserted into a provisioned `atlas_stage` v1 database
  and read back through `list_staged_items`; `meta` must round-trip
  byte-exact and the database schema version must be 1.
- **`digest-vectors.json`** — exact-byte identity vectors for the
  design §2 rule (`sha256:` + lowercase hex of the SHA-256 of the
  exact metadata bytes). Each vector records its input bytes and the
  expected artifact ID, so any implementation can pin the rule
  byte-exactly.

## Doctrine-projection cases

Some cases carry a `doctrine_projection` (Atlas cases with
`executable: false`, PM cases with the field present). These record
the expected _projection_ under the doctrine — extras relocation,
lossless-only coercion, fail-closed sensitivity fallback,
unknown-closed-enum preservation (§3.1–3.2 of the design). No current
reader projects; projection is design §7 slice 2/6 work. These cases
are the oracle those implementations must satisfy. They are
deliberately **not** asserted against today's readers, whose
conforming behavior is parse-and-preserve (PM) and byte-exact
round-trip (Atlas staging).

## Rules for this set

- **Additive-only, like the schema.** A new schema version appends
  new case files (`pm-metadata-v2.json`, …) and new cases; historical
  cases are never edited or removed. If a case's expectation must
  change, that is a doctrine event: record it in the design note's
  change record and in the PR that changes it.
- Every case names its `historical_source` — the real shape, test, or
  doctrine rule it comes from. Synthetic cases with no source do not
  belong here.
- Rejection cases are part of conformance: a block with no version
  key, or a version newer than the reader, must fail closed.

## CEN-04 conformance

This set introduces no persisted field and writes no store, so it
needs no new field-admission row under CEN-04 §5
(`doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`).
The facts it exercises already have owners there: the pm-metadata
block in the GitHub body is the record and `pm_metadata_items` is its
rebuildable projection (CEN-04 §3.10); Atlas staging rows are owned
by the Atlas producer profile (design §3.1). Fixtures are test data
about those facts, not a new home for them.
