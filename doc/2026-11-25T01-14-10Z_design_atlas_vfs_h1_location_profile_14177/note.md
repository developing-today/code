# H1 — VFS location-record profile v1

Date: 2026-11-25
Status: design of record for [#14177](https://github.com/developing-today/code/issues/14177)
(workstream H, parent [#9100](https://github.com/developing-today/code/issues/9100),
parent design `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md`).
Spec and golden vectors only. No runtime code changes in this record's PR.

This record freezes the field registry that the parent design sketched in §3:
every field's path, type, whether it is required, how it is indexed, its enum
policy and its safety effect. It also fixes the `extras` behavior for each
failure, the derivation rules for the presentation fields, and a golden vector
set that H2 ([#14178](https://github.com/developing-today/code/issues/14178))
and H3 ([#14179](https://github.com/developing-today/code/issues/14179)) must
reproduce. It consumes the #9095 Metadata doctrine
(`doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/note.md`) and the
landed #9115 code (`pkgs/id/crates/id-core/src/location_ref.rs`,
`pkgs/id/crates/id-store/src/location_index.rs`). It changes neither.

## 1. Profile identity and envelope

- Profile id: `atlas.vfs-location/v1`.
- Envelope: the #9095 Atlas form, `{"schema_version": 1, "fields": {…}, "extras": {…}}`.
  A missing `extras` reads as `{}`. A `schema_version` other than `1` is not
  read by this profile: the record is reported `unsupported-version` and is not
  projectable.
- Artifact id: #9095 exact-byte identity, `"sha256:" + hex(SHA-256(bytes))`
  over the stored bytes. The vectors store objects in canonical JSON (sorted
  keys, no insignificant whitespace, UTF-8, no ASCII escaping) so their ids are
  reproducible. A writer may store other bytes; the id is always of the bytes
  as stored, never of a re-encoding.

## 2. Two layers

The profile has two layers with different authorities.

1. **Assertion core.** Exactly the fields of the strict #9115 sidecar
   assertion (`LocationAssertion` in `location_index.rs`), in the same
   encodings. `location_index.rs` is the normative source for these fields;
   this record restates them so the profile is checkable, and if the two ever
   differ the code wins and this record is corrected. The harvester's strict
   parse (unknown fields reject) is unchanged: the sidecar assertion does not
   carry presentation fields.
2. **Presentation.** Optional VFS fields carried only in the Metadata record.
   They are projections for browsing and lookup. They are never identity and
   never authority. H2 decides where they travel in the sidecar manifest, next
   to the assertion they describe.

## 3. Field registry

All paths are under `fields`. "Index" is the H2 serving-table policy.

| Field                | Layer        | Type (JSON)                          | Required | Index                    | Enum policy | Safety effect                            |
| -------------------- | ------------ | ------------------------------------ | -------- | ------------------------ | ----------- | ---------------------------------------- |
| `content_id`         | core         | string, `content/<profile>.<64 hex>` | yes      | point                    | —           | identity                                 |
| `locator`            | core         | string, tagged locator text (§4)     | yes      | stored, part of identity | open (§4)   | identity; unknown tag blocks projection  |
| `namespace`          | core         | label                                | yes      | filter                   | —           | identity; authorization filter           |
| `tenant_scope`       | core         | label                                | yes      | filter                   | —           | identity; authorization filter           |
| `observed_at`        | core         | integer, unix milliseconds, ≥ 0      | yes      | stored                   | —           | ordering only                            |
| `access_class`       | core         | string                               | yes      | filter                   | closed      | visibility; fail-closed (§6)             |
| `signer`             | core         | label                                | yes      | stored                   | —           | sidecar authority gate (A-9115.2)        |
| `occurrence_ref`     | core         | label                                | no       | none                     | —           | #3125 carry-over pointer                 |
| `source_version_ref` | core         | label                                | no       | none                     | —           | which version of which source            |
| `display_path`       | presentation | string, absolute, no `!/`            | no       | prefix                   | —           | none; default derived (§5)               |
| `filename`           | presentation | string                               | derived  | point                    | —           | none; always derived (§5)                |
| `host`               | presentation | string                               | derived  | point                    | —           | none; always derived (§5)                |
| `produced_by`        | presentation | label                                | no       | none                     | open        | none                                     |
| `archive_member`     | presentation | object `{path, offset, length}`      | no       | none                     | —           | none; only on `r2` and `laptop` locators |

A **label** is a non-empty string, not only whitespace, at most 256 UTF-8
bytes (`MAX_LABEL_BYTES`), with no control characters.

Derived (`locator_type`): the projection also carries the locator's tag
(`r2`, `laptop`, `drive`) as `locator_type`, matching the parent §4 table
column. It is never stored in `fields`.

## 4. Core encodings

- `content_id` uses the `ContentId` text form. The registered profiles are
  `raw-blake3-256` and `atlas-chunk-sha256-utf8`; anything else is a type
  mismatch.
- `locator` uses `TypedLocator::parse_tagged` text:
  `r2/<account>/<bucket>/<key>`, `laptop/<hostname><absolute path>`,
  `drive/<account>/<file-id>/<https url>`. Segments allow no `/`, whitespace
  or control characters; the r2 key and laptop path allow no control
  characters; the drive url must be `https://` with a host and no whitespace.
- The locator tag registry is open in the #9095 sense: new stores add tags
  additively (reviewed by the ID/location owners, parent §11). A reader that
  meets an unknown tag preserves the value in `extras` with reason
  `unknown-enum` and the record is **not projectable**. It never guesses a
  variant and never coerces a local path into a cloud URL. A known tag whose
  body does not parse is `type-mismatch`, also not projectable.
- `observed_at` is an integer in unix milliseconds, as in `location_index.rs`.
  A string date, a float or a boolean is `type-mismatch`; there is no
  coercion, because a date string has no lossless round trip to milliseconds.

## 5. Presentation derivations

These rules are exact, so every implementation projects the same view.

- **`host`** is always derived from the locator: `laptop` gives the hostname;
  `r2` gives `r2:<account>/<bucket>`; `drive` gives `drive:<account>`. A
  supplied `host` that equals the derived value is accepted. A different
  supplied value is relocated to `extras` with reason `unconvertible`, and the
  derived value is used.
- **`display_path`** defaults to: `laptop` → the locator path; `r2` → `"/" + key`;
  `drive` → no default (Drive ids carry no path). A supplied value must be an
  absolute path with no control characters and no `!/`; otherwise it is
  relocated as `type-mismatch` and the default applies. Paths are byte-exact as
  observed: no case folding, no separator rewriting (parent §11 default,
  confirmed here).
- **Archive members.** `archive_member` is `{"path", "offset", "length"}`
  exactly: a relative non-empty path and two non-negative integers. It is valid
  only on `r2` and `laptop` locators (the byte-range fetch pattern from #3125).
  On a `drive` locator, or with any other shape, the whole value is relocated
  as `unconvertible` (or `type-mismatch` if it is not an object). When a member
  is present and `display_path` is not supplied, the default is the archive's
  path followed by `!/` and the member path, for example
  `/2026-11.tar!/photos/geordi.png`. `!/` is the reserved member separator;
  that is why a supplied `display_path` may not contain it.
- **`filename`** is always derived: the last `/` segment of the part after the
  last `!/` of the projected `display_path`. With no `display_path` there is
  no `filename`. A supplied value that differs is relocated as
  `unconvertible`. An r2 record with no supplied `display_path` therefore gets
  its blob key as the filename; producers that know the human name supply
  `display_path`.

## 6. Failures, `extras` and projectability

Relocation follows #9095 §3.2 exactly. `extras` is keyed by the bare field
name, as in the #9095 fixtures. Relocation never overwrites: on a key
collision with a different value, and for any source key beginning with `$`,
the entry is appended to `extras["$relocated"]` as `{path, value, reason}`.
Every relocation is also reported as `{path, reason}`.

| Case                                           | Reason          | Projectable |
| ---------------------------------------------- | --------------- | ----------- |
| unknown field                                  | `unknown-field` | yes         |
| missing required core field                    | (report only)   | no          |
| core field of the wrong type or malformed      | `type-mismatch` | no          |
| unknown `access_class` token                   | `unknown-enum`  | no          |
| unknown locator tag                            | `unknown-enum`  | no          |
| bad optional core field                        | `type-mismatch` | yes         |
| bad presentation field                         | `type-mismatch` | yes         |
| conflicting derived field, or misplaced member | `unconvertible` | yes         |

**`access_class` is closed and fail-closed.** The values are exactly
`public`, `scope` and `stored_unverified`. A missing or unknown value is
preserved in `extras`, the view reads `scope` (the A-9115.2 privacy rule's
default: private within the caller-authorized scope), and the record is not
projectable. The fallback never widens visibility, and because the record is
not projected it cannot be indexed under a guessed class. `stored_unverified`
records project with state `unverified`, never `fresh`.

**A non-projectable record** is retained as a Metadata artifact (its bytes and
`extras` are kept for correction), produces no serving row and no index entry,
and does not count toward any query result or candidate count. Only its
`extras`, reports and projectability are normative; its partial view is not.

## 7. Identity, rename and move

- Identity is (`namespace`, `tenant_scope`, `content_id`, `locator`), the same
  key the #9115 index uses per scope.
- **Presentation-only rename** (new `display_path`, same locator): same
  identity, new Metadata artifact. The projection replaces the row in place;
  the state stays as harvested.
- **Move** (the locator itself changes, for example a laptop file moved to
  another directory): a new identity. The producer asserts the new location and
  writes a `delete` tombstone for the old one. This **refines parent §9**,
  which treated a move as the same lineage: since the locator is identity, a
  moved file cannot keep its old record, and the tombstone keeps the history
  auditable. Linking old and new for lineage is #3125's occurrence registry's
  job (`occurrence_ref`), not the VFS row's.
- **Tombstones**: `delete` sets state `deleted`; `stale` sets `stale`. Rows
  are kept with their tombstone (parent §6).

## 8. Index and query semantics

- H2 indexes `content_id`, `filename`, `display_path` (prefix) and `host`, and
  filters on `namespace`, `tenant_scope` and `access_class`. It does not index
  `produced_by`, `occurrence_ref`, `source_version_ref`, `signer` or
  `archive_member`.
- Authorization filters apply before matching and before counts (A-9115.2).
- The four default queries return `fresh`, `stale` and `unverified` rows and
  exclude `deleted` rows. Each returned row carries its state, so callers can
  tell a stale location from a fresh one. An audit query that includes
  `deleted` rows is explicit, never the default.
- Filename match is exact and byte-wise; prefix match is a plain byte prefix
  on `display_path`.

## 9. Evolution

Additive only, per #9095 §5. New optional presentation fields may be added to
this registry under v1. A new `access_class` value, a new required field, a
change to an encoding, or a change to a derivation rule is a breaking
candidate and goes to the #9095 parking lot and batch gate, not into a VFS
fork. New locator tags follow §4.

## 10. Golden vectors

`fixtures/vfs-location-v1.json` holds the parent §9 example (`geordi.png` in
five places) plus doctrine cases, and `fixtures/check_profile.py` is a
standard-library reference projection that checks them. Run:

```sh
python3 -I doc/2026-11-25T01-14-10Z_design_atlas_vfs_h1_location_profile_14177/fixtures/check_profile.py
```

It checks each record's artifact id and projection (view, `extras`, reports,
projectability), then folds five event streams and checks the four queries
and every row's state.

| Record                  | What it shows                                                                |
| ----------------------- | ---------------------------------------------------------------------------- |
| `laptop-photos`         | laptop locator; derived display path, filename and host                      |
| `r2-upload`             | r2 blob with a supplied human `display_path`                                 |
| `drive-shared`          | drive locator, supplied path, `stored_unverified` → state `unverified`       |
| `laptop-incoming`       | same bytes under another name (`geordi (1).png`)                             |
| `tar-member`            | archive member; `!/` display path; filename from the member                  |
| `laptop-other`          | different bytes on the same host, so host and prefix queries are not trivial |
| `r2-upload-renamed`     | presentation rename: same identity, new artifact                             |
| `laptop-incoming-moved` | move target; the old identity gets a delete tombstone                        |
| `bad-access-class`      | unknown closed enum: preserved, fallback `scope`, not projectable            |
| `bad-locator-tag`       | unknown locator tag: preserved, not projectable                              |
| `bad-observed-at`       | ISO string for milliseconds: no coercion, not projectable                    |
| `conflicting-host`      | supplied host and filename disagree with derivation; r2 key as filename      |
| `drive-with-member`     | archive member on a drive locator: `unconvertible`                           |
| `unknown-field`         | unknown field kept; `$`-prefixed key goes to `$relocated`                    |
| `extras-collision`      | relocation never overwrites an existing `extras` value                       |

Steps: `initial` (all five locations fresh except Drive, which is
`unverified`), `presentation-rename`, `laptop-move`, `delete-and-stale` (Drive
deleted, tar member stale) and `non-projectable-skipped` (bad records add no
rows).

H2 and H3 conformance: an implementation passes when, fed the same records
and event streams, it produces the same views for projectable records, the
same `extras` and reports for every record, and the same query results and
states. The expected values were reviewed by hand against this record; when
they change, the change is to this record first.

## 11. Hand-off

- **H2 (#14178):** `vfs_locations` columns are §3's projected view plus
  `state`, `tombstone_ref` and `sidecar_ref`; indexes per §8; the sidecar
  manifest carries each assertion with its presentation fields as one
  Metadata artifact per location, so a rebuild reproduces the vectors.
- **H3 (#14179):** fold order and tombstone handling must reproduce the §10
  steps; a non-projectable record is reported, never folded.

## 12. Open questions

| Question                                                               | Default here                                    | Decider                                      |
| ---------------------------------------------------------------------- | ----------------------------------------------- | -------------------------------------------- |
| Should a `drive` locator get a default `display_path` from Drive's API | no default; producers supply it                 | Atlas VFS owner, when a Drive producer lands |
| Nested archives (`a.tar!/b.zip!/c.png`)                                | one member level only; deeper nesting relocates | Atlas VFS owner with #3125 owners            |
| Case-insensitive filename lookup                                       | byte-exact only                                 | Atlas VFS owner, on a measured need          |

## 13. Evidence checked (2026-11-25)

- #14177, #14178, #14179, #14180 and #9100 issue bodies; parent design
  `doc/2026-11-24_design_atlas_vfs_workstream_h_9100.md` (§3, §4, §6, §9,
  §11, §12).
- `location_ref.rs` (`TypedLocator`, tagged text form, segment rules) and
  `location_index.rs` (`LocationAssertion` keys and encodings, `AccessClass`,
  states, tombstone kinds, `MAX_LABEL_BYTES`) on `main` at f66f1e5.
- `location_policy.rs` visibility rules for the three access classes.
- #9095 note §3.1–§3.2 and its fixtures (`extras` keyed by bare field name).
