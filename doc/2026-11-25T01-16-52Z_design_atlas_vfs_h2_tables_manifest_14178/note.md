# H2 — VFS serving tables and sidecar manifest

Date: 2026-11-25
Status: design of record for [#14178](https://github.com/developing-today/code/issues/14178)
(workstream H, parent [#9100](https://github.com/developing-today/code/issues/9100),
parent design `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md` §4 and §12).
Design only. No implementation in this record's PR.

This record specifies three things the parent design left open: where the H1
presentation fields travel, the `vfs_locations` tables and their four indexes,
and the sidecar manifest that lets a bucket rebuild the tables on its own. It
builds on the H1 profile
(`doc/2026-11-25T01-14-10Z_design_atlas_vfs_h1_location_profile_14177/note.md`,
called "H1" below) and the landed #9115 code
(`pkgs/id/crates/id-store/src/location_index.rs` and `sidecar_snapshot.rs`),
which it does not change. H3
([#14179](https://github.com/developing-today/code/issues/14179)) wires the
write path; H4 (`doc/2026-11-24_design_atlas_vfs_h4_offline_tier_14180.md`)
consumes the manifest defined here.

## 1. Facts this design rests on

Read from `main` at 152b4c3:

- A sidecar snapshot (`SidecarEnvelopeV1`) carries a complete view: a canonical
  CBOR map with text keys. The harvester reads only the `locations` key
  (`LOCATIONS_VIEW_KEY`) and ignores the others (`decode_assertions`).
- Each scope (one namespace, one tenant scope) has one snapshot chain. Genesis
  is generation 0 and each successor has generation + 1 and names its parent.
- A snapshot folds at order `(g, 0, 0)`. A tombstone with basis generation `g`
  and per-scope sequence `s` folds at `(g, 1, s)`. `ScopeCursor` records the
  chain head and the last tombstone order.
- Folding a snapshot inserts every assertion in its view, fresh or
  unverified, even over a deleted entry. A later snapshot that omits a
  location changes nothing. Only a tombstone moves a location to stale or
  deleted. A tombstone for a location never asserted creates an entry with no
  access class.
- `LocationIndex::rebuild` from the same validated snapshots and tombstones
  equals the incrementally folded index at the same cursor.

## 2. Decision 1 — presentation travels in the same signed view

**Decided:** a snapshot's complete view may carry a second key,
`vfs_metadata`, next to `locations`. Its value is an array of byte strings.
Each byte string is the exact bytes of one H1 Metadata object
(`atlas.vfs-location/v1`), so its #9095 artifact id is the SHA-256 of those
bytes, unchanged by the CBOR wrapper.

Rules:

- **Core binds presentation.** A Metadata object is attached to the assertion
  in the same view whose identity (`namespace`, `tenant_scope`, `content_id`,
  `locator`) it names, and only when all of its H1 core fields equal that
  assertion's (same values, same encodings). An object that names no
  assertion in the view, matches more than one, differs in any core field, or
  is not projectable under H1 is reported and ignored. It never changes the
  assertion or the index.
- **Insert-all is untouched.** An assertion with no attached object still
  becomes a row. Its presentation is the H1 projection of a Metadata object
  holding only its core fields, so `host`, `display_path` and `filename` come
  from the H1 derivations.
- **Latest assertion wins.** A row's presentation comes from the snapshot
  that last asserted the location (`LocationEntry.asserted_by`). A
  presentation-only rename is a new snapshot that re-asserts the location with
  a new object. Tombstones never change presentation.
- **Producers own completeness.** A complete view lists every location the
  producer currently asserts and leaves out tombstoned ones unless they are
  really observed again, because re-asserting a deleted location revives it.

**Rejected alternatives:**

- _A separate presentation sidecar chain:_ two chains per scope would need a
  cross-chain order, and a presentation could arrive before or after its
  assertion. One view keeps both under one signature and one fold order.
- _Widening `LocationAssertion` with presentation fields:_ it would change the
  strict #9115 record that H1 keeps normative and every existing signature
  vector.

## 3. Decision 2 — `vfs_locations` tables

**Decided:** two tables in the existing Atlas serving Turso database (parent
§4). SQLite dialect:

```sql
CREATE TABLE vfs_locations (
  namespace            TEXT    NOT NULL,
  tenant_scope         TEXT    NOT NULL,
  content_id           TEXT    NOT NULL,  -- ContentId text form
  locator              TEXT    NOT NULL,  -- tagged locator text
  locator_type         TEXT    NOT NULL,  -- tag; open registry, so no CHECK
  display_path         TEXT,              -- NULL only for drive with no supplied path
  filename             TEXT,
  host                 TEXT    NOT NULL,
  produced_by          TEXT,
  archive_member       TEXT,              -- canonical JSON {path, offset, length}
  observed_at          INTEGER NOT NULL,  -- unix ms, latest assertion or tombstone
  access_class         TEXT    NOT NULL
    CHECK (access_class IN ('public', 'scope', 'stored_unverified')),
  state                TEXT    NOT NULL
    CHECK (state IN ('fresh', 'stale', 'deleted', 'unverified')),
  occurrence_ref       TEXT,
  source_version_ref   TEXT,
  signer               TEXT    NOT NULL,  -- signer of the record that last set the state
  metadata_artifact_id TEXT,              -- sha256:… of the attached H1 object; NULL if derived
  sidecar_ref          TEXT    NOT NULL,  -- snapshot id that last asserted the location
  asserted_generation  INTEGER NOT NULL,
  tombstone_ref        TEXT,              -- tombstone id that last set the state, if any
  PRIMARY KEY (namespace, tenant_scope, content_id, locator)
) WITHOUT ROWID;

CREATE INDEX vfs_by_content  ON vfs_locations (namespace, tenant_scope, content_id);
CREATE INDEX vfs_by_filename ON vfs_locations (namespace, tenant_scope, filename);
CREATE INDEX vfs_by_path     ON vfs_locations (namespace, tenant_scope, display_path);
CREATE INDEX vfs_by_host     ON vfs_locations (namespace, tenant_scope, host);

CREATE TABLE vfs_scope_cursor (
  namespace               TEXT    NOT NULL,
  tenant_scope            TEXT    NOT NULL,
  head_generation         INTEGER NOT NULL,
  head_snapshot_id        TEXT    NOT NULL,
  last_tombstone_basis    INTEGER,
  last_tombstone_sequence INTEGER,
  manifest_id             TEXT,             -- latest manifest written at or before this cursor
  PRIMARY KEY (namespace, tenant_scope)
) WITHOUT ROWID;
```

Rules:

- **Projection, not a store.** Rows are the H1 projection of the
  `LocationIndex` entries for a scope, joined with the presentation chosen in
  §2. Every row and the scope's cursor row change in one transaction, so a
  reader never sees rows ahead of or behind the cursor.
- **Tombstone-only entries are not projected.** An index entry with no access
  class (a tombstone that arrived before any assertion) gets no row: its
  visibility cannot be decided, so it fails closed. It gets a row when a
  later snapshot asserts it.
- **Index keys lead with scope.** Every index starts with `namespace,
tenant_scope`, so the authorization filter is part of the index seek and a
  query never touches another scope's rows (A-9115.2 privacy rule).
  `access_class` is filtered in the same statement, before any `LIMIT` or
  count.
- **Byte-exact matching.** Columns use SQLite's default `BINARY` collation.
  Filename and host lookups are equality. A path prefix lookup is a half-open
  range, not `LIKE` (which folds ASCII case and treats `%` and `_` as
  wildcards):

  ```sql
  WHERE namespace = ?1 AND tenant_scope = ?2
    AND display_path >= ?3 AND display_path < ?4
    AND state != 'deleted' AND access_class IN (/* caller's visible classes */)
  ORDER BY display_path, content_id, locator
  LIMIT ?5
  ```

  `?4` is the prefix's code-point successor: the prefix with its last code
  point replaced by the next one (a trailing U+10FFFF is dropped and the step
  repeats). UTF-8 `BINARY` order equals code-point order, so the range holds
  exactly the paths that start with the prefix.

- **Default queries exclude deleted rows** and return the state of every other
  row (H1 §8). Pages continue from the last `(display_path, content_id,
locator)` or primary key seen; the opaque, scope-bound cursor contract is
  #9119's (A-9119.2), not this table's.
- `vfs_scope_cursor` mirrors `ScopeCursor`. It is how H3 resumes a fold and
  how a rebuild proves it reached the same epoch.

**Rejected:** a `CHECK` on `locator_type`. Locator tags are an open registry
(H1 §4), so a new store would need a table migration for a value the profile
already allows.

## 4. Decision 3 — the sidecar manifest

### 4.1 Epochs and generations

An **epoch** is a fold position in one scope: the order `(generation, rank,
sequence)` of the last folded record, as in `location_index.rs`. Its text key
is `<generation>.<rank>.<sequence>` with generation and sequence written as
20-digit zero-padded decimals, so keys sort in fold order:
`00000000000000000002.1.00000000000000000002`.

A **bucket generation**, the unit H4 snapshots, is one manifest: the records
of one scope folded after the previous manifest's epoch, up to and including
this manifest's epoch. Generations are append-only. A new record always lands
in a later generation, and a written manifest is never rewritten.

### 4.2 Layout beside the bucket

```text
<bucket>/vfs-snapshots/<namespace>/<tenant_scope>/<epoch-key>/manifest.json
<bucket>/vfs-snapshots/<namespace>/<tenant_scope>/<epoch-key>/vfs_locations-<epoch-key>.parquet   # H4, optional
<bucket>/vfs-records/<namespace>/<tenant_scope>/snapshots/<snapshot-id>.cbor
<bucket>/vfs-records/<namespace>/<tenant_scope>/tombstones/<tombstone-id>.cbor
```

- The H4 directory name (`vfs-snapshots/<generation>/`) is kept, with the
  scope added in front, because a bucket can hold several scopes. This is the
  editorial reconciliation H4's header asks for.
- Record objects hold the canonical bytes of each signed record
  (`SidecarEnvelopeV1` and `LocationTombstoneV1`). Where a snapshot already
  lives in the #9114 store (`hash.metadata.<snapshot-digest>`), the manifest
  points at that key instead of storing a second copy.
- Namespace and tenant scope are labels (H1 §3). Path segments use them
  percent-encoded (RFC 3986 unreserved characters kept, every other byte
  `%XX`), so `tenant:atlas-home` becomes `tenant%3Aatlas-home`.

### 4.3 Manifest format

The manifest is canonical JSON (sorted keys, no insignificant whitespace,
UTF-8, no ASCII escaping), the same encoding as the H1 vectors:

```json
{
  "cursor": {
    "head_generation": 2,
    "head_snapshot_id": "<snapshot id>",
    "last_tombstone_basis": 2,
    "last_tombstone_sequence": 2
  },
  "epoch": "00000000000000000002.1.00000000000000000002",
  "format": "atlas.vfs-manifest/v1",
  "namespace": "atlas",
  "offline": null,
  "previous": { "epoch": "00000000000000000000.0.00000000000000000000", "manifest_id": "sha256:…" },
  "projection": { "digest": "sha256:…", "rows": 7 },
  "records": [
    { "id": "<snapshot id>", "key": "vfs-records/…/snapshots/<id>.cbor", "kind": "snapshot", "order": [1, 0, 0] },
    { "id": "<tombstone id>", "key": "vfs-records/…/tombstones/<id>.cbor", "kind": "tombstone", "order": [2, 1, 0] }
  ],
  "signature": { "key": "<hex ed25519 public key>", "signer": "atlas-ingest", "suite": "ed25519", "value": "<hex>" },
  "tenant_scope": "tenant:atlas-home"
}
```

- `records` lists, in fold order, every record after `previous.epoch` up to
  `epoch`. The genesis manifest has `"previous": null`.
- `projection` is the row count and digest of the whole `vfs_locations`
  projection of the scope at `epoch`, deleted rows included. The digest is the
  SHA-256 of the canonical JSON array of rows sorted by primary key, each row
  an object of all §3 columns, with SQL `NULL` as JSON `null`.
- `offline` is `null`, or H4's snapshot linkage `{"object", "rows",
"digest"}`. When H4 export is on for a scope, the writer exports first and
  writes the manifest only after the snapshot verifies, which keeps manifests
  immutable.
- **Signature.** `value` is Ed25519 over the domain string
  `atlas.vfs-manifest/v1`, a zero byte, then the canonical bytes of the
  manifest with `signature` removed. The manifest id is
  `"sha256:" + hex(SHA-256(exact stored bytes))`, signature included, so it is
  never inside the bytes it names.
- The manifest is authority only for _which records, in which order_. The
  records carry their own signatures, so a manifest cannot forge a location;
  at worst a bad manifest leaves one out, and rebuild detects that through
  the projection digest and the cursor.

### 4.4 Writer cadence

The writer seals a generation at the end of each batch fold and at most every
N interactive folds or T minutes per scope, whichever comes first. N and T are
operator settings with defaults of 1,000 records and 15 minutes. It never
seals an empty generation. Cadence is not part of the format: readers depend
only on the chain.

## 5. Rebuild from the bucket alone

Input: the bucket. No serving database, no laptop, no other store.

1. **Find the head.** List `vfs-snapshots/<ns>/<scope>/` and take the greatest
   epoch key. Read its `manifest.json`.
2. **Walk the chain.** Verify each manifest's signature and id, then follow
   `previous` to genesis. Each `previous.manifest_id` must equal the id of the
   manifest at `previous.epoch`, each manifest's first record must fold after
   its predecessor's epoch, and its last record's order must equal its own
   epoch. Any gap, fork or mismatch stops the rebuild for that scope and is
   reported. Nothing is written.
3. **Fetch records.** Read every listed object, check that its id is the
   envelope id of the exact bytes, and decode it. A missing or mismatched
   object stops the rebuild.
4. **Fold.** `LocationIndex::rebuild(namespace, tenant_scope, snapshots,
tombstones, dispatch)`. Its `cursor()` must equal the head manifest's
   `cursor`.
5. **Project.** For each entry with an access class, take the presentation
   from its `asserted_by` snapshot's `vfs_metadata` under §2, and project with
   H1.
6. **Check.** The row count and digest must equal the head manifest's
   `projection`.
7. **Swap.** Write the rows and the cursor row in one transaction, replacing
   the scope's previous rows. Rollback is to keep the old rows: the bucket is
   never written by a rebuild.

A partial rebuild up to an older manifest works the same way, starting at
that manifest. That is how a reader pins an answer to a generation (H4's "which
generation did this answer come from").

## 6. Paper check against the parent §9 example

Scope `atlas` / `tenant:atlas-home`, using the H1 vector records. Each
snapshot's view carries `locations` and the matching `vfs_metadata` objects.

| Record | Epoch key (short) | What it carries                                                           | Rows after fold                                                                                |
| ------ | ----------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| S0     | `0.0.0`           | genesis; the five `geordi.png` locations plus `laptop-other`              | 6: five fresh, Drive `unverified`                                                              |
| S1     | `1.0.0`           | all six again; the r2 location now carries the `r2-upload-renamed` object | 6; the r2 row's `display_path` is `/uploads/2026/geordi-portrait.png`, `sidecar_ref` S1 on all |
| S2     | `2.0.0`           | all six, with `laptop-incoming` replaced by `laptop-incoming-moved`       | 7; the old incoming row stays `fresh` with `sidecar_ref` S1, since omission changes nothing    |
| T0     | `2.1.0`           | delete tombstone on `laptop-incoming`                                     | 7; old incoming row `deleted`, `tombstone_ref` T0                                              |
| T1     | `2.1.1`           | delete tombstone on `drive-shared`                                        | 7; Drive row `deleted`                                                                         |
| T2     | `2.1.2`           | stale tombstone on `tar-member`                                           | 7; tar-member row `stale`                                                                      |

Manifests: M0 at `0.0.0` lists S0 (`previous` null). M1 at `2.1.2` lists S1,
S2, T0, T1 and T2 with `previous` M0.

Rebuild from the bucket finds M1 at the greatest epoch, walks to M0, fetches
six records, and folds them. The cursor is head generation 2, last tombstone
(2, 2), which equals M1's. The rows match H1's `delete-and-stale` step:

- By content id: `laptop-incoming-moved`, `laptop-photos`, `r2-upload-renamed`
  and `tar-member`. The two deleted rows are excluded.
- By filename `geordi.png`: `laptop-photos` and `tar-member`.
- By path prefix `/home/user/photos/`: `laptop-incoming-moved`,
  `laptop-other` and `laptop-photos`.
- By host `atlas-01`: `laptop-other` and `laptop-photos`.

Rebuilding from M0 alone gives H1's `initial` step. Dropping T1 from M1 leaves the
folded cursor unchanged, but the Drive row stays `unverified`, so the
projection digest no longer matches and the rebuild stops instead of serving
a location that was deleted.

## 7. What H2 leaves to others

- **H3 (#14179):** calls the fold and the projection in the serving write
  path, seals generations per §4.4, and emits the reference-release signal to
  #9084 on delete tombstones.
- **H4 (#14180):** fills `offline` and reads the layout in §4.2 instead of its
  provisional `<bucket>/vfs-snapshots/<generation>/`.
- **#9119:** the opaque, authenticated page cursor for these queries.
- **Retention:** hard purge of deleted rows and old generations stays with the
  #9084 and #3125 lineage owners (parent §11).

## 8. Verification for the implementation slice

- **Golden:** the §6 walk as a fixture. Build the six records with test keys,
  write M0 and M1, rebuild, and compare with the H1 vectors.
- **Property:** for random fold streams, the incremental projection equals
  rebuild-from-manifests at every sealed epoch, row for row and digest for
  digest.
- **Adversarial:** a dropped record, a reordered record, a swapped
  `previous`, a forged manifest signature, a record object whose bytes do not
  match its id, and a `vfs_metadata` object whose core disagrees with its
  assertion. Each is rejected or ignored as §2 and §5 say, and none changes a
  row.
- **Privacy:** a query from another scope returns no rows and no count. A
  caller without the `stored_unverified` grant sees no unverified rows.
- **Range:** prefixes ending in U+10FFFF, prefixes containing `%` and `_`, and
  mixed-case paths match byte-exactly.

## 9. Open questions

| Question                                                  | Default here                                          | Decider                                       |
| --------------------------------------------------------- | ----------------------------------------------------- | --------------------------------------------- |
| Manifest signer: the producer key or a separate key       | the scope's ingest signer (same key as its snapshots) | ID/location owners, with key rotation (#9114) |
| Seal cadence defaults (1,000 records, 15 minutes)         | as stated; tune from H4's per-generation measurements | owning namespace operator                     |
| Whether `vfs_metadata` should also be legal in tombstones | no; tombstones never change presentation              | Atlas VFS owner, only if a use appears        |

## 10. Evidence checked (2026-11-25)

- #14178 issue body and comments; #14179 and #14180 bodies; parent design
  §§3–4, 6, 9, 11 and 12.
- H1 note and vectors (merged in PR #14706); H4 spec §3–§4 and its open
  questions.
- `location_index.rs` (module doc, `apply_snapshot`, `apply_tombstone`,
  `rebuild`, `ScopeCursor`, `LocationEntry`, `decode_assertions`) and
  `sidecar_snapshot.rs` (`SidecarBodyV1`, `view`, `decode_view`) on `main` at
  152b4c3.
- A-9114.2, A-9115.2 and A-9119.2 in
  `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`.
