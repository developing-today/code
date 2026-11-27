# Muse import contract, `atlas-muse-import/v1`

Date: 2026-11-26
Status: design of record for [#13607](https://github.com/developing-today/code/issues/13607)
(parent [#9097](https://github.com/developing-today/code/issues/9097), programme
[#13241](https://github.com/developing-today/code/issues/13241)). Contract and synthetic
fixture only. No importer, provider access, schedule or real Muse data in this record.

Files in this record:

- `note.md`: this contract.
- `fixture.json`: a synthetic replay fixture of 17 deliveries, each with hand-authored
  expected effects.
- `replay.py`: a standard-library reference replay that reads the fixture, applies the
  rules below and compares the result with each hand-written `expect` block.
  `python3 replay.py` prints one line per step and exits non-zero on any mismatch.

## 1. What this contract is, and what it is not

Atlas has no verified Muse source format. Nothing in the repository establishes an export
file layout, a local database, an API, its scopes, whether an export includes every object,
or whether deletions are marked. The adapter-fixtures chapter of the Atlas book proposed an
Atlas-side shape for a manifest consumer; this record fixes that shape as a versioned
contract and makes it testable.

The contract is the **Atlas side** of the boundary. A future mapper, written once an
authorized export or API sample exists and its completeness can be documented, turns the
provider's real format into deliveries of this shape. Anything the provider does not supply
(stable identity, ordered revisions, attested membership) the mapper must not invent, and
the contract below refuses deliveries that lack it.

| Concern                                         | Exists in Atlas today                                                                                                        | Future adapter work this contract defines                                                                   |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Landing material without serving it             | Per-unit staging databases (`atlas_stage` v1), the stage registry, raw un-embedded chunks                                    | A Muse delivery is staged as one unit per accepted, fully assembled export or delta                         |
| Sensitivity                                     | Classification at staging, boundary-safe chunking, unknown classification fails closed to secret-bearing                     | Source access designations map onto the same four-rung ladder; the classifier verdict still intersects (§6) |
| Promotion                                       | The batch gate's audit record and state machine (#7709), embed-once at promotion under the one model                         | A delivery publishes only after the gate passes, so a refused delivery never reaches the corpus             |
| Content identity                                | Chunk `content_hash` as the cross-store join key; SHA-256 rung of the detection ladder                                       | Per-object `content_sha256` checked on arrival (§5); unchanged revisions are no-ops and add zero vectors    |
| Deletion                                        | Designed only: tombstones that propagate to CAS, FTS, vectors and the recommender as one operation. Nothing propagates today | Ordered, revision-keyed tombstones (§4) and the narrow rule for deletion by absence (§3)                    |
| Source identity, revisions, scopes, checkpoints | None for Muse. No connector, no export mechanism recorded                                                                    | All of §2 to §7                                                                                             |

## 2. A delivery

A delivery is one part of one export: a manifest and the objects it carries. Payload bytes
travel inline as `content` in the fixture; a real mapper may reference local member files,
and the same digest and length rules then apply to each member.

**Manifest** (every field required unless marked):

| Field                        | Meaning                                                                                                                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `schema`                     | Exactly `atlas-muse-import/v1`. Anything else is refused as `malformed`.                                                                                                                   |
| `namespace`                  | Source namespace. Object keys are unique within it.                                                                                                                                        |
| `export_id`, `part`, `parts` | The export and this part's position, `1 <= part <= parts`. `(export_id, part)` names the delivery for duplicate detection.                                                                 |
| `scope_id`                   | The access scope the export was captured under (one account, workspace or share). A complete inventory speaks only for its own scope.                                                      |
| `scope_seq`                  | The producer's sequence for this scope. Recorded as a monotone high-water mark; ordering of individual objects comes from revisions, not from this.                                        |
| `access_epoch`               | A counter the producer increments whenever the scope's access changes.                                                                                                                     |
| `scope_access`               | The scope's ceiling on the four-rung ladder: `public`, `internal`, `private`, `sensitive`.                                                                                                 |
| `inventory`                  | `complete` (the export claims every object in the scope) or `delta` (only the objects it names).                                                                                           |
| `membership_attested`        | Required `true` for `complete`: the producer attests that membership is exhaustive and access was unchanged during capture. A single exported conversation never implies a full inventory. |
| `observed_at`                | When the producer captured it. Informational; never used to order revisions.                                                                                                               |
| `mapper`, `producer_schema`  | Which mapper and which provider schema produced it, for attestation (#6045).                                                                                                               |

**Object**:

| Field                       | Meaning                                                                                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kind`                      | One of `conversation`, `message`, `memory`, `artifact`. Any other kind is unknown identity.                                                                               |
| `key`                       | The provider's stable identity for the object, unique in the namespace. Required. Fuzzy text matching never stands in for it.                                             |
| `revision`                  | A positive integer, strictly increasing per key in source order. A provider that offers only opaque tokens cannot satisfy this; its mapper must refuse, not invent order. |
| `op`                        | `upsert` or `delete`.                                                                                                                                                     |
| `parent`                    | Optional. The containing object (a message's conversation, an artifact's conversation). Must be in the same scope.                                                        |
| `access`                    | Optional source access designation on the four-rung ladder. Missing or unrecognized falls closed to `sensitive`.                                                          |
| `content`, `content_sha256` | Required for `upsert`: the payload and its SHA-256, hex.                                                                                                                  |
| `edges`                     | Optional typed links: `attaches`, `reply_to`, `branch_from`, `derived_from`. An edge to an object the caller cannot read reveals nothing about that object.               |

## 3. Inventories: complete, partial and delta

- A **delta** changes only the objects it names. An object it omits is untouched. A delta
  never deletes by absence.
- A **multi-part export** stays staged until every part from 1 to `parts` has arrived with
  identical assembly fields (namespace, export, parts, scope, sequence, access epoch,
  scope access, inventory, attestation). Until then nothing from it is visible and nothing
  is retired. A part that disagrees with its siblings is a `conflict`.
- A **complete** export retires by absence only when all of these hold: every part
  arrived, `membership_attested` is `true`, and its access epoch is current. Then each live
  object of that scope that the export does not name is retired with a tombstone at its
  current revision, marked `by: inventory`. Objects in other scopes are never touched.
- A `complete` export without attestation is **incomplete scope** and is refused whole.

## 4. Edits, duplicates, ordering and tombstones

Per key, Atlas keeps the last applied upsert revision and, if retired, the tombstone's
revision. Their maximum is the key's floor.

- An upsert above the floor replaces the object, its content and its edges. Edges are
  re-declared with each revision, so an edit that keeps an attachment keeps the edge.
- An upsert at the live revision with the same digest is a **no-op**: no re-chunking, no
  new vectors. The same revision with a different digest is a `conflict`.
- A delete above the floor retires the key at that revision, `by: delete`. A delete for a
  key Atlas has never seen still records the tombstone, so an older upsert that arrives
  later cannot create it.
- An upsert or delete at or below the floor is **stale**: the delivery is accepted, the
  object is not, and the stale key is reported. A stale event never resurrects a retired
  object or rolls a live one back.
- Only a later revision from the source restores a retired key.
- Re-delivering `(export_id, part)` with identical bytes is a **duplicate**: nothing
  changes. The same `(export_id, part)` with different bytes is a `conflict`.

## 5. Fail-closed refusals

A refused delivery publishes nothing, including its valid objects, does not advance any
checkpoint and does not count as delivered (a corrected re-send is a new delivery).

| Reason             | When                                                                                                                                                                                                                               |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `malformed`        | Wrong schema, missing manifest field, impossible part numbers, unknown inventory or scope access value.                                                                                                                            |
| `incomplete-scope` | `complete` without `membership_attested: true`.                                                                                                                                                                                    |
| `stale-access`     | `access_epoch` lower than the scope's current epoch. A delivery captured under old access cannot widen or restore reads.                                                                                                           |
| `unknown-identity` | Any object with an unknown kind, a missing or empty key, a missing or non-positive revision, an unknown op, or a non-string parent.                                                                                                |
| `corrupt-member`   | Any upsert whose content does not match its declared SHA-256 (and, for file members, declared length).                                                                                                                             |
| `conflict`         | A reused `(export_id, part)` with different bytes; parts that disagree; a key repeated in one delivery; a key moving scope or kind; the same revision with different content; the same access epoch with a different scope access. |

## 6. Access scope and reads

An object's effective sensitivity is the most restrictive of: its scope's `scope_access`,
its own `access` (missing or unknown is `sensitive`), and the effective sensitivity of
every ancestor up its parent chain. The staging classifier's verdict intersects on top of
this, as for every source; the fixture leaves it out because it is not Muse-specific.

A caller holds a ceiling on the same ladder and a set of granted scopes. A read succeeds
only when the object is live, its scope is granted, every ancestor is live and in the same
scope, and its effective sensitivity is within the ceiling. Every other case, including a
key that never existed, answers the same `not-found`, so a refusal is never an existence
oracle. `related` answers `not-found` for an unreadable object and otherwise lists only
neighbours the caller can read, in either edge direction: a private side branch that
attaches a shared artifact does not show up as that artifact's neighbour for a narrower
caller, and deduplicating the artifact does not lend the branch its visibility.

A narrower access epoch takes effect on delivery, with no object changes needed: reads
that were allowed a moment earlier are revoked at once.

## 7. Checkpoint

Per scope, Atlas records `high_seq` (the highest accepted `scope_seq`), `access_epoch`,
`scope_access` and `last_complete` (the last complete export applied), alongside the set of
accepted `(export_id, part)` digests. Staged items commit idempotently before any
checkpoint field moves, and only an applied delivery moves them. A refused or still-staged
delivery leaves the checkpoint where it was.

## 8. The fixture

`fixture.json` has two callers, `agent` (ceiling `internal`, scope S1) and `owner`
(ceiling `private`, scopes S1 and S2), and 17 deliveries. Each step's `expect` holds the
outcome, the refusal reason if any, the stale keys, the object state changes (`effects`),
checkpoint changes and read checks. Effects accumulate: the expected state after a step is
every earlier step's effects plus its own.

| Step | Input                                                                                                              | Expected effect                                                                                                                       |
| ---- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| 01   | Complete export E1, part 1 of 2                                                                                    | Staged; nothing visible                                                                                                               |
| 02   | E1 part 2 of 2: private side branch, private memory, artifact attached from both branches, artifact with no access | All eight objects published; the unlabelled artifact is readable by neither caller; `related` hides the private branch from the agent |
| 03   | E1 part 2 again, identical                                                                                         | Duplicate; no change                                                                                                                  |
| 04   | Delta edits m1 to revision 2                                                                                       | m1 at revision 2; its attachment edge intact                                                                                          |
| 05   | Delete m2 at revision 3, delivered first                                                                           | m2 retired at 3                                                                                                                       |
| 06   | The older m2 edit at revision 2, delivered late                                                                    | Accepted delivery, stale object; m2 stays retired                                                                                     |
| 07   | m2 at revision 4                                                                                                   | Restored by a newer source revision                                                                                                   |
| 08   | Delta names only m3                                                                                                | m3 edited; omitted siblings untouched                                                                                                 |
| 09   | Complete export of scope S2                                                                                        | Published; the agent's grant does not reach it                                                                                        |
| 10   | Complete E2 omits the memory, only part 1 of 2 arrives                                                             | Staged; memory still readable                                                                                                         |
| 11   | `complete` without attestation                                                                                     | Refused `incomplete-scope`                                                                                                            |
| 12   | Complete, attested E4 omits the memory                                                                             | Memory retired by inventory; unchanged objects are no-ops; S2 untouched                                                               |
| 13   | Valid edit plus an object of unknown kind with no key                                                              | Refused `unknown-identity`; the valid edit does not land                                                                              |
| 14   | Digest mismatch                                                                                                    | Refused `corrupt-member`                                                                                                              |
| 15   | Export D1 part 1 re-sent with different bytes                                                                      | Refused `conflict`                                                                                                                    |
| 16   | Scope S1 narrows to `private` under access epoch 2                                                                 | The agent's reads of S1 revoked at once; the owner's unchanged                                                                        |
| 17   | A delta from access epoch 1 tries to edit m1                                                                       | Refused `stale-access`                                                                                                                |

Provider-specific validation still needs a user-authorized export or API sample whose
format and completeness can be documented, and a mapper onto this shape, before any live
ingestion.
