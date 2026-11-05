# Roadmap homes: synchronization design

Status: proposed design for [#1738](https://github.com/developing-today/code/issues/1738). This is a design record, not an implementation. It follows Drew's direction correction on [#1738](https://github.com/developing-today/code/issues/1738#issuecomment-6475682718): keep every roadmap home current, and settle the synchronization model before changing the dashboard.

## Summary and recommendation

Use **option (b): one canonical roadmap store, with the other homes generated as native-format views**. The current Grist `Program roadmap` table is the lowest-migration candidate for the canonical store: it is already the live source used by the aggregator and is the source named by the interim `roadmap-sync-grist-md` job. Keep the dashboard reading Grist in the first implementation; this design does not require a dashboard read-side change.

Generate only explicitly managed roadmap sections in each destination. Preserve prose and sections outside those boundaries. Give each roadmap item a stable ID; never identify or merge records by title similarity. Treat Grist as authoritative for managed fields, report edits or stale projections in generated sections as drift, and never silently import a conflicting generated-section edit back into the canonical store.

This is a recommendation, not an assumption that Drew has approved Grist as the long-term authority. The open questions below ask Drew to confirm the canonical editing surface, generated-file write policy, and full inventory of roadmap homes. If Grist is not an acceptable authority, keep option (b) but choose a versioned canonical manifest before implementation; do not fall back to three-way writes without a conflict contract.

## Current state and evidence

Revalidated on code `main` at `0885d6046` on 2026-11-03:

- The aggregator's live roadmap reader calls the Grist `Roadmap` table API and maps those records into `RoadmapRow`s (`pm-stack/aggregator/src/sources/live.ts:235-255`). The live source selects that reader (`src/sources/live.ts:876`); its README identifies Grist REST as the live roadmap source (`pm-stack/aggregator/README.md`, source table around line 463). There is no repo-file roadmap reader in that path.
- Code has a root portfolio `ROADMAP.md` and per-project files in `planning/roadmaps/`. `planning/README.md` says the root file is the overall view and the per-project files are maintained in place. `planning/DOCUMENTATION_ARCHITECTURE.md` classifies both as maintained documents, rather than generated outputs.
- The current `dezren39/nix` `main` tree has a root `ROADMAP.md`. The #1738 scope also calls out other roadmap homes; they need an explicit inventory rather than an assumption that these two repositories are exhaustive.
- Drew's correction on #1738 says all roadmap homes must stay synchronized and records the three options below. It describes a 4-hourly `roadmap-sync-grist-md` interim sync, first run on 2026-10-28.
- The interim sync's present operational state could not be confirmed from the source repositories or this laptop: no matching implementation was found in code `main` or the inspected `dezren39/nix` tree, and the laptop's user systemd has no matching timer. It may be deployed elsewhere. Therefore, do not claim it is currently running or use it as evidence that destinations are fresh; locate and instrument it as part of the first implementation step.
- No open PR for #1738 was present during revalidation. The issue remains open and the design-first correction is still the recorded direction.

The prior 4-hour sync is a temporary bridge only. Until its host, code, last-success time, and failure path are observable, each destination's freshness is unknown. A schedule firing is not proof that all homes converged.

## Goals and boundaries

The eventual system must:

1. Keep every registered roadmap home synchronized automatically, including the Grist table, code `ROADMAP.md`, code `planning/roadmaps/*.md`, Nix `ROADMAP.md`, and additional homes added to the registry.
2. Preserve stable identity, source links, and enough provenance to explain where a generated entry came from.
3. Make partial failure and staleness visible. Never label a home current merely because a job started or because another home succeeded.
4. Preserve human-authored context outside generated regions and avoid destroying a file on format or ownership ambiguity.
5. Keep each consumer free to use its native view; no aggregator build or read-through is part of this design task.

Out of scope: implementing adapters, changing the aggregator, migrating every existing paragraph into structured fields, deleting or rewriting roadmap files, and deciding priority policy beyond a deterministic projection of canonical fields.

## Options considered

### (a) Synchronize every native store; consumers read their local store

An engine would propagate writes among Grist and every Markdown home, while each consumer continued reading its preferred native representation.

**Advantages:** preserves current editing surfaces; minimal immediate change to readers; a store can remain useful during another store's outage.

**Costs and risks:** this is multi-master synchronization. Concurrent edits, offline destinations, retries, renamed files, and partial writes need field-level conflict semantics and durable per-home cursors. Without a single authority, "latest" is not a safe conflict rule (clock skew and independent edits can discard intent). Pairwise adapters grow with the number of homes. It also risks echo loops and makes it unclear which copy is authoritative during a disagreement.

**Assessment:** viable only if preserving independent native editing is a hard requirement and the design first specifies a conflict-free operation model. Not recommended as the first implementation.

### (b) One canonical store; other homes are generated views

Writers update one canonical record set. A projector renders the registered items into each home's native format; consumers can read that home without implementing cross-store merge logic.

**Advantages:** one conflict authority; idempotent retries; deterministic output; a new home needs one renderer, not pairwise sync; current Grist-backed dashboard can remain unchanged. Stable IDs make duplicate detection exact.

**Costs and risks:** changes the editing contract for generated fields; all writers must use the canonical surface. Generated Markdown must coexist with valuable prose, and cross-repository writes require scoped credentials, review/merge behavior, and visible failures. Grist's edit history, export, availability, and recovery need to meet the accepted canonical-store contract.

**Assessment:** recommended, with Grist as the migration-minimizing canonical candidate, subject to Drew confirming the authoring surface. Generated views must be bounded to explicit managed regions.

### (c) Read through the aggregator

At request time, the aggregator would read Grist and repository files, reconcile them, and display a merged roadmap without synchronizing the underlying homes.

**Advantages:** fast prototype; avoids writing to other repositories; each store remains independently editable.

**Costs and risks:** does not satisfy the requirement that roadmap homes themselves stay current; consumers disagree; every read becomes a distributed merge with source-specific failures and deduplication ambiguity. The dashboard would become the only place where reconciliation occurs.

**Assessment:** reject as the system design. A read-only observability view may be useful later, but it is not synchronization and must not be presented as such.

## Recommended data and identity contract

The canonical store should expose a versioned, normalized item contract before any renderer is built. At minimum, each record needs:

- immutable `roadmapId` (assigned once; never derived from title or file position);
- title and concise description;
- owning project/repository and optional area;
- status and horizon/date at a documented grain;
- priority/order where the source has one;
- canonical issue/PR references and source links;
- source/owner metadata and last canonical revision.

Keep source-specific fields when the canonical store cannot represent them; do not silently discard them in a projection. A home registry maps each destination to its repository, path/table, renderer version, managed-region marker, and last successfully applied canonical revision/checksum. Unknown homes or an unrecognized file shape fail closed and raise a drift/error record instead of receiving a guessed write.

Titles are labels, not keys. Two similarly titled items remain distinct until an explicit stable-ID alias or merge decision is recorded. A same-ID conflict is resolved by the canonical record; a duplicate-looking different-ID item is reported for human reconciliation and is never automatically collapsed.

## Projection, conflicts, and safe writes

For Markdown, place generated rows between stable markers, for example `<!-- roadmap-sync:begin <home-id> -->` and `<!-- roadmap-sync:end <home-id> -->`. Renderers replace only that block. Keep surrounding explanation, links, and local notes untouched. Where a whole table is managed, mark the table boundary explicitly. Do not rewrite an unmarked file or infer ownership from headings.

For Grist, canonical rows are the source records, not a second projection. If Drew selects another canonical store, Grist becomes a generated view and its editable/generated fields must be visibly distinguished.

Conflict rules:

1. Canonical fields win for a managed `roadmapId`; generated destinations are never silently promoted to authority.
2. A destination with an edited managed block, unknown marker/version, duplicate ID, or unrelated local structure is not overwritten. Record drift, keep its last content, and require repair or an explicit migration.
3. An edit outside a managed block is preserved. A human wishing to change a generated field edits the canonical record, not the view.
4. Retries are idempotent on `(homeId, roadmapId, canonicalRevision)` and write only when the rendered checksum differs.
5. Every write records the canonical revision, renderer version, target revision, outcome, and timestamp. Partial success is reported per home; no global “synced” state is emitted until all required homes acknowledge the same canonical revision.

For Git homes, use a narrowly scoped automation identity and a generated-only branch/PR or an explicitly approved generated-region commit policy. Never grant a synchronizer broad write access to arbitrary repository files. If a generated-only change cannot be proven, leave it for review. The accepted operational policy must ensure updates are automatic without silently overwriting a concurrent human edit.

## Currency and failure model

Use change-triggered projection from the canonical store, with a periodic full reconciliation as repair. The existing 4-hour schedule, if confirmed and still needed, may remain a temporary fallback; it is not the correctness mechanism.

Track freshness per home, not just per job: canonical revision, last attempted revision/time, last successful revision/time, rendered checksum, and failure reason. Surface lag, rejected writes, and unknown destination state on an operational view or alert. A destination is current only when its acknowledged revision/checksum matches the canonical projection. Retry transient failures with bounded backoff; a persistent permission, schema, or conflict error remains visible and does not block unrelated homes from updating.

Acceptance SLOs (initial proposal, to confirm): event-driven propagation target under 15 minutes for healthy homes; full reconciliation at least every four hours; alert/visible stale state after one failed retry window; no destructive automatic resolution of a conflict. The schedule is a recovery sweep, not an assertion that all hosts are online or current.

## Rollout outline — design only

1. Drew confirms the canonical editing surface and generated-write policy; inventory all roadmap homes and owners, including any home beyond code, Nix, and Grist.
2. Locate the deployed `roadmap-sync-grist-md` implementation, host, credentials scope, schedule, and last-success signal. Decide whether to retire, adapt, or retain it as a reconciler; do not assume the laptop timer is its host.
3. Define the versioned canonical schema, stable-ID migration, home registry, and managed-region rules. Dry-run against every registered destination and report conflicts without writing.
4. Implement one projection adapter at a time, starting with code `planning/roadmaps/` and root `ROADMAP.md`; then Nix `ROADMAP.md`; then remaining registered homes. Require deterministic/idempotent output and source-to-view reconciliation tests for every renderer.
5. Add change-triggered execution, periodic reconciliation, per-home freshness/error reporting, access controls, and recovery tests. Keep the aggregator on Grist during this phase.
6. Only after synchronized homes are operational should a separate design/implementation decide whether the aggregator should consume a different native store or add a cross-home read view.

## Questions for Drew

1. **Canonical authority:** Is Grist the accepted canonical editing surface for structured roadmap fields, with code/Nix Markdown as generated views? If not, should the canonical store be a versioned Git manifest, and in which repository?
2. **Markdown ownership:** May automation own explicitly marked blocks and publish/merge generated-only changes, while all surrounding prose remains human-maintained? What review/auto-merge policy is acceptable across repositories?
3. **Home inventory:** Which other roadmap stores/files are in scope, who owns each, and which are source records versus views? The design cannot claim “all homes” until this registry is complete.
4. **Interim job:** Where is `roadmap-sync-grist-md` deployed, is it still running, and what are its last success/failure and rollback signals? The source repositories and this laptop did not establish its current operational state.
5. **Freshness:** Are the proposed 15-minute healthy propagation target and four-hour reconciliation interval acceptable? What alert path should own persistent drift?
6. **Identity and migration:** Who approves stable IDs and duplicate aliases when titles match but records may differ? Should an initial migration report all ambiguous rows before any projection is enabled?

## Future implementation task (not started)

After the canonical authority and write policy are decided, create a separate implementation task for the registry/schema, dry-run reconciliation, and generated views. It should accept: every registered home has a deterministic view for every canonical ID; edits outside managed regions survive; managed-region drift fails closed and is reported; repeated runs are idempotent; per-home revision/freshness is observable; and failure/recovery tests cover partial destinations. A later, separately scoped task may change dashboard reads; this design does not authorize that build.

## Evidence references

- [#1738 and Drew's direction correction](https://github.com/developing-today/code/issues/1738#issuecomment-6475682718) — automatic sync across all homes, three options, and description of the interim 4-hour sync.
- Code `main` `0885d6046`: `pm-stack/aggregator/src/sources/live.ts:235-255, 876`; `pm-stack/aggregator/README.md` roadmap source table; `planning/README.md`; `planning/DOCUMENTATION_ARCHITECTURE.md`.
- `dezren39/nix` `main` tree: root `ROADMAP.md` exists; no `roadmap-sync-grist-md` path was found in that tree.
- Fresh local check on 2026-11-03: no matching user systemd timer on this laptop. This is scoped evidence only; it does not rule out a timer or cron on another host.
