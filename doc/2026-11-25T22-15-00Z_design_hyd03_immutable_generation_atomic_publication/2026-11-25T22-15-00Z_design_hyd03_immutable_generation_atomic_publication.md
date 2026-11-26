# HYD-03 — Immutable hydration generation with atomic publication

Issue: [#9460](https://github.com/developing-today/code/issues/9460)
(slice `HYD-03` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the HYD-03 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row's goal is "stage artifacts/manifest and atomically
update current pointer only after validation". Its first oracle is
`pmo_hyd_publish_crash_never_points_to_partial_generation` ("injects
failure before/after manifest and pointer swap plus competing CAS").
Its acceptance is "old generation stays current on failure; no
partial current pointer; immutable generations; orphan behavior
deterministic", and its negatives are "pointer before manifest
durability, concurrent publishers". This record _decides_:

- the publication protocol and its durability order;
- the pointer record and its compare-and-swap;
- which generations may become current;
- how a generation is numbered for the journal;
- what an orphan is and when it may ever be collected;
- the storage contract a backend must meet.

It authorizes no code, schema, migration, deployment or live
acceptance (§10).

Governing decisions consumed, not re-opened:

- **Component #9207 §2.1–§2.3 and §5**
  (`components/9207-context-hydration-incremental-activity.md`):
  - build in staging and write the manifest last;
  - publish by changing `current.json`, or by "a versioned pointer
    with CAS/conditional update" where atomic swap is unavailable;
  - never mutate a prior immutable generation;
  - a failed refresh leaves the prior current bundle intact;
  - bind PR artifacts to one head SHA.
- **FDR-PMO-004**: generations are immutable once published, and
  partial or truncated state is never presented as complete.
- **HYD-01 record** (#9458, open PR #15184,
  `doc/2026-11-25T18-52-28Z_design_hyd01_versioned_manifest_result_envelope/`):
  - the manifest is the bundle root's `manifest.json`, written last,
    and the only file whose presence makes a generation publishable;
  - generation `status` is derived, never asserted: `complete`,
    `partial`, or `failed`, where `failed` means "no generation
    could be published at all; the prior generation, if any, remains
    current and untouched";
  - §5's result envelope names the generation it describes.

  This record reads HYD-01 as proposed in that PR. If the freeze
  changes before it merges, this record follows it.

- **HYD-04 record** (#9461,
  `doc/2026-11-25T21-40-00Z_design_hyd04_bounded_hydration_adapter_summary/`):
  - HYD-04 writes only into HYD-03's staging area and hands it a
    complete staged set;
  - it never touches `current`;
  - `complete` and `partial` generations are published results, and
    `failed` names the still-current prior generation.
- **HYD-02** (#9459, open, no record yet): owns source keys and
  artifact paths. HYD-03 names the directories it needs (§3) and
  defers every file name and key inside them to HYD-02.
- **JRN-03 checkpoint model** (`work/journal.rs:192-200`, landed): a
  checkpoint carries `HydrationRef { generation: <canonical decimal
string>, manifest_digest: Option }`. HYD-03 decides which number
  that is (§5).
- **CEN-04** (#9434, record merged): §5 admits no persisted field
  without a matrix row, and §7 makes rollback disable-only.

## Verified state at `origin/main` `7f2931b5`

- `pkgs/fleet/crates/fleet-queue/src/work/context.rs` does not exist.
- No code writes a manifest, a generation directory or a `current`
  pointer.
- `contracts.rs:53` registers the digest domain
  `fleet.context-manifest/v1`.
- `journal.rs` already references a hydration generation as a
  canonical decimal string, while the 9207 manifest shape and HYD-01
  use `generation_id: "g-<stable unique id>"`. Nothing reconciles the
  two yet; §5 does.
- The REL-04 record (#9514, merged in #15266) found that the broker
  store's multi-statement `BEGIN … COMMIT` pipelines are not proven
  atomic. HYD-03's pointer update is designed as a **single
  conditional statement or a single atomic rename**, so it does not
  wait on that proof (§4.3).
- No artifact store is named on main. The 9207 component proposes
  `.fleet/work/<work-id>/context/…` "subject to the existing artifact
  root/path policy", and the HYD-03 row names "#9198 artifact-store/
  transaction proof" as a hard blocker. That is still open, and §6
  states what any backend must provide.

## 1. Intent

A reader of a work item's context always sees one whole generation:

- the one the pointer named when it looked;
- its manifest, which validates against its digest;
- every artifact the manifest lists, present with the recorded hash.

A crash, a failed fetch, a concurrent publisher or a slow disk can
delay a new generation. It can never expose a half-written one,
replace a published one, or point at one that was not durable when
the pointer moved.

## 2. Ownership

| Concern                                                                                                                                                        | Owner                                           |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Staging, manifest-last write, durability order, pointer record and CAS, which generations may become current, publication sequence, orphan rules (this record) | **HYD-03**                                      |
| Manifest schema, derived status, result envelope                                                                                                               | HYD-01 (#9458)                                  |
| File names and keys inside a generation, path safety                                                                                                           | HYD-02 (#9459)                                  |
| What is fetched and staged, the summary, head binding                                                                                                          | HYD-04 (#9461)                                  |
| Cursor commits for pages a generation summarizes                                                                                                               | HYD-05 (#9507)                                  |
| Commands (`hydrate`, `refresh`, `show`)                                                                                                                        | HYD-08 (#9510) and CLI-13                       |
| The artifact store and its root                                                                                                                                | #9198 storage owner, admitted through CEN-04 §5 |
| The checkpoint that references a generation                                                                                                                    | JRN-03 (landed model)                           |

HYD-03 adds no database, queue, service, scheduler or daemon. Its
code is a publisher in `work/context.rs` and, only if the storage
owner keeps the pointer in the broker, one conditional statement on
the existing store.

## 3. Layout

Per work item, under the artifact root the storage owner names:

```text
<root>/<work-key>/context/
  staging/<generation_id>/        being built; never read by readers
  generations/<generation_id>/    immutable once present
    manifest.json                 HYD-01; written last
    …                             HYD-02 names, HYD-04 content
  current.json                    the pointer record (§4)
  publish.lock                    filesystem backend only (§6)
```

`<work-key>` is HYD-02's repo-scoped, traversal-safe key for the
`(authority_id, work_id)` pair. `generation_id` is
`g-<collision-resistant id>` (HYD-01), minted when staging starts. It
is unique without coordination, so concurrent builders never share a
directory.

## 4. Publication protocol

### 4.1 Steps, in order

1. **Stage.** HYD-04 streams artifacts into
   `staging/<generation_id>/` through HYD-02's paths. The publisher
   rejects any path outside that directory and any symlink.
2. **Seal.** When HYD-04 hands over the staged set, the publisher:
   - recomputes every artifact's sha256 and byte count from the bytes
     on disk;
   - builds the manifest from those values, never from in-memory
     fetch state;
   - derives `status` per HYD-01 §4 and computes the manifest digest
     under `fleet.context-manifest/v1`;
   - writes `manifest.json` last.
3. **Make durable.** Every artifact and the manifest are flushed to
   stable storage before anything refers to them. On a filesystem:
   - `fsync` each file;
   - then `fsync` the staging directory;
   - then rename `staging/<id>` to `generations/<id>`;
   - then `fsync` both parent directories.

   On an object store: each object put has returned success, and a
   read-back of the manifest matches its digest.

4. **Validate.** The publisher re-reads the generation from
   `generations/<id>/`, not from staging. It validates the manifest
   against its digest and HYD-01's schema, and checks that every
   listed artifact exists with the recorded hash and size. A
   generation that fails validation is never published. It stays in
   `generations/` with a sibling `rejected.json` naming the check
   that failed, and `rejected.json` is itself written once.
5. **Publish.** It compare-and-swaps the pointer (§4.2) from the
   pointer value read **before step 2** to the new value.
6. **Report.** It returns HYD-01's envelope for the published
   generation, or for `failed` the generation that is still current.

The pointer is never written before step 4 completes. That is the
"pointer before manifest durability" negative, ruled out by order,
not by timing.

### 4.2 The pointer record

`current.json` (or the equivalent row, §6) holds:

```json
{
  "schema_version": "fleet.context-pointer/v1",
  "work": { "authority_id": "local", "work_id": "842" },
  "publication_seq": "7",
  "current": {
    "generation_id": "g-…",
    "manifest_digest": "sha256:…",
    "status": "complete | partial",
    "publication_seq": "7"
  },
  "last_complete": {
    "generation_id": "g-…",
    "manifest_digest": "sha256:…",
    "publication_seq": "6"
  }
}
```

- `publication_seq` is a per-work decimal string. It starts at `"1"`
  for the first publication and increases by exactly one on every
  successful CAS. An absent pointer is sequence `"0"`.
- The CAS condition is "the stored `publication_seq` still equals the
  one this publisher read before sealing". A publisher that loses the
  CAS does not retry the swap blindly. It re-reads the pointer and
  publishes only if its own generation is still newer by
  `created_at` and its source pins than the one now current.
  Otherwise it leaves its generation unpublished and reports that a
  newer generation won.
- `last_complete` moves only when the new generation's status is
  `complete`. Otherwise it is carried forward unchanged.

### 4.3 Which generations may become current

- `complete` and `partial` generations publish: both are honest
  about what they hold, and HYD-04 treats both as published results.
  Readers that need completeness read `last_complete`, and readers
  that need freshness read `current` and its status. A partial
  refresh therefore never hides the last complete bundle.
- `failed` never publishes. HYD-01 defines it as "no generation could
  be published", so a failed build has no pointer entry and the prior
  pointer is untouched. HYD-01 §6's phrase "failed/partial-outside-
  current" is read as: failed is outside current, and partial
  publishes with its status stated.
- A rejected generation (§4.1 step 4) never publishes, whatever its
  manifest claims.

### 4.4 Why the swap is atomic without the REL-04 proof

The swap is one atomic operation on the chosen backend:

- one `rename(2)` of a fully written, fsynced temporary pointer file
  over `current.json`, under the publish lock;
- or one conditional `UPDATE … WHERE work = ? AND publication_seq = ?`
  on the broker store, whose affected-row count is the CAS result.

Neither needs a multi-statement transaction. Steps 1–4 write only
immutable new objects, which no reader can reach until the pointer
names them. So the REL-04 transaction gap (§ Verified state) does not
apply here, and HYD-03 must not add a second statement to the swap.

## 5. Generation numbers and the journal

A generation has two names:

- `generation_id` (`g-…`) names the directory and is in the
  manifest. It exists from the moment staging starts.
- `publication_seq` exists only for published generations, and is
  assigned by the CAS that published it.

`journal.rs`'s `HydrationRef.generation` is the `publication_seq` of
the published generation the checkpoint was hydrated from, and
`manifest_digest` is that generation's digest. A checkpoint therefore
always refers to something a reader could have seen as `current`,
never to a staging, failed or rejected build. To resolve a
`HydrationRef`, the publisher keeps an append-only publication log
(§6) mapping each `publication_seq` to its `generation_id` and
digest.

## 6. Storage contract

A backend conforms when it provides:

1. **Immutable object write**, with a durability point (a returned
   flush or put), and readable-after-durable.
2. **An atomic pointer swap with compare-and-swap** on
   `publication_seq`, as one operation.
3. **An append-only publication log**, written before or in the same
   atomic operation as the swap. If it is written before the swap, a
   log entry whose sequence the pointer never reached is ignored.

Two backends are candidates. The #9198 storage finding picks which
lands first; HYD-03 does not.

- **Local filesystem.** The swap is a `rename(2)` under an exclusive
  `flock` on `publish.lock`, held from re-reading the pointer to the
  rename. The log is one line appended and fsynced under the same
  lock before the rename. Same-filesystem only, and staging lives on
  the same filesystem as `generations/`. This serves one machine.
  Hosted or multi-machine use needs the next one.
- **Broker store pointer row.** The swap is one conditional `UPDATE`
  (the first publication is an `INSERT … ON CONFLICT DO NOTHING`
  whose affected rows is the CAS). Objects live in the storage owner's
  artifact store. The log is the row's history, written by the same
  single statement only if the store can do that in one statement;
  otherwise by a separate insert before the swap, under rule 3. The
  pointer row and the log need a CEN-04 §5 row each. `turso_queue.rs`
  is touched only for this, per the HYD-03 row's ownership.

Weaker backends (an object store with no conditional put) are not
conforming. The component's "document weaker semantics" fallback is
refused for the publisher: a backend that cannot CAS does not
publish.

## 7. Orphans and retention

Deterministic classification, from the store's contents alone:

| State              | Rule                                                                                                                    |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| **Staging orphan** | a directory under `staging/`. Never readable.                                                                           |
| **Unpublished**    | under `generations/`, valid, and never named in the publication log (a CAS loser, or a crash between step 3 and step 5) |
| **Rejected**       | under `generations/` with `rejected.json`                                                                               |
| **Published**      | named in the publication log                                                                                            |
| **Pinned**         | published and named by `current`, `last_complete`, or any checkpoint's `HydrationRef`                                   |

Retention:

- Nothing is deleted by HYD-03.
- Collection is a separate, approved operation (component rollback
  rule: "retain objects until approved GC").
- When it is approved, only staging orphans, unpublished and rejected
  generations past a retention age may be collected, and never a
  pinned or published one. Published generations are history.
- A crash leaves at most one staging orphan or one unpublished
  generation per interrupted run, and a rerun mints a new
  `generation_id`, never reusing the old directory.

## 8. Recovery after a crash

| Crash point                                                | State afterwards                                                                        | What a rerun does                                                                                                 |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| During stage or seal                                       | staging orphan; pointer unchanged                                                       | builds a new generation                                                                                           |
| After the manifest is written, before durability completes | staging orphan, possibly missing files; pointer unchanged                               | builds a new generation; the orphan is never validated                                                            |
| After the rename to `generations/`, before validation      | unpublished; pointer unchanged                                                          | builds a new generation; may later validate and publish the old one only if it is still newer than current (§4.2) |
| After validation, before the CAS                           | unpublished; pointer unchanged                                                          | same as above                                                                                                     |
| After the CAS, before the report                           | published; pointer moved                                                                | the next read sees it; the caller's retry gets the current pointer's generation                                   |
| Mid pointer write, filesystem                              | the temporary file is orphaned; `current.json` is the old one, because rename is atomic | nothing to repair                                                                                                 |

There is no repair step that edits `current.json` by hand. A pointer
that fails its own validation (unknown schema, digest mismatch with
the generation it names) is a typed read error per HYD-01 §5, and the
reader falls back to `last_complete`. Repair is a fresh publication.

## 9. Acceptance oracle

Names follow the row's convention. Every test must be listed by exact
name before it counts, per the row's done command.

1. `pmo_hyd_publish_crash_never_points_to_partial_generation` — the
   row's first oracle. Inject a crash at every step boundary of §4.1
   and inside the pointer write. After each, the pointer names
   either the old generation or a fully durable, valid new one.
2. `pmo_hyd_pointer_never_written_before_manifest_durable` — a
   storage fake records the order of durability points; the swap is
   never before the last artifact's and the manifest's.
3. `pmo_hyd_concurrent_publishers_one_wins` — two publishers seal
   from the same pointer; exactly one CAS succeeds, `publication_seq`
   advances by one, and the loser does not overwrite a newer winner.
4. `pmo_hyd_failed_build_leaves_pointer_untouched` — a `failed` build
   changes neither `current`, `last_complete` nor the log.
5. `pmo_hyd_partial_publishes_but_keeps_last_complete`.
6. `pmo_hyd_rejected_generation_never_published` — a tampered
   artifact (hash mismatch) is rejected at validation.
7. `pmo_hyd_published_generation_is_immutable` — every write path
   refuses to modify a directory under `generations/`.
8. `pmo_hyd_hydration_ref_resolves_to_published_generation` — a
   checkpoint's `generation` resolves through the log to the
   generation and digest it was hydrated from.
9. `pmo_hyd_orphans_classify_deterministically` — the §7 table,
   from store contents alone, on a crash corpus.
10. `pmo_hyd_publisher_rejects_paths_outside_staging` — traversal and
    symlink inputs, alongside HYD-02's own key tests.

## 10. Gates and sequencing

- **Design inputs.**
  - HYD-01: record proposed in PR #15184, not yet merged. This record
    follows it.
  - HYD-02 #9459: no record yet. This record names only directories,
    and every name inside them is HYD-02's.
  - HYD-04: record merged.
- **Implementation gates.**
  - HYD-01 and HYD-02 implemented.
  - The #9198 artifact-store finding names the root and the backend
    (§6).
  - CEN-04 §5 rows for the pointer row and the publication log if
    the broker backend is chosen.
  - The row's shared-file edges: after DEP-03 on `turso_queue.rs`
    and after HYD-02 on `work/context.rs`.
- **Downstream.**
  - HYD-04's `publisher` argument is this record's publisher.
  - HYD-08's `show` reads `current` and `last_complete`.
  - REL-09 completion freshness reads `current` and its status.
  - JRN checkpoints carry §5's `publication_seq`.
- **Rollback.**
  - Disable the publisher; the pointer stays where it is.
  - With no pointer, readers treat the work item as never hydrated
    (the row's "legacy/none pointer default").
  - Rolling back the pointer itself is a CAS to a previously
    published generation named in the log, never an edit of a
    generation.
  - Nothing is deleted.

## 11. Requirement trace (design-level)

| IDs    | Theme                                    | Where addressed      |
| ------ | ---------------------------------------- | -------------------- |
| C-R117 | versioned manifest, generation lifecycle | §4, §5               |
| H-R001 | immutable, provenance-bearing context    | §4.1, §7             |
| P-R008 | no partially published snapshot          | §4.1 order, §4.3, §8 |
| P-R036 | recovery after interruption              | §7, §8               |

Full per-occurrence text is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`.

## 12. Non-goals

- No manifest field, status or envelope change (HYD-01).
- No file name, key or path rule inside a generation (HYD-02).
- No fetching, summary or head binding (HYD-04), and no cursor
  (HYD-05).
- No garbage collection and no retention period; only the rule for
  what a future approved collector may touch.
- No command, flag or registration (HYD-08, CLI-13).
- No new database, queue, service or daemon, and no live GitHub read.
