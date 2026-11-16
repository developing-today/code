# FDR-PMO-004 — Immutable hydration, per-source cursors, and attribution

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. Persistence and rollout keep their separate gates.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** durable context generations, incremental source ingestion, cursor advancement, event revisions, session-origin attribution
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-003, -005, -006, -007

## Context and problem

Agents need a durable navigable context bundle and reliable “what changed?” view across queue data, linked GitHub issues/PRs, reviews, checks, comments, commits/diffs, dependencies, notes, and checkpoints. Large histories and endpoint limits make one unbounded response unsafe. A timestamp or comment creation ID is not a complete update cursor: old comments can be edited and pages can be interrupted. Shared bot identity does not establish which session caused an event.

Sources: [architecture, “Shared command and result interface” and “Lifecycle and cross-system reliability”](../architecture.md); [source 02 §§10–12, 15–16](../sources/02_charter_v1_1.md); [#9207, “Versioned manifest,” “Retrieval and performance contract,” “Incremental activity and cursor correctness,” and “Testing and oracles”](../components/9207-context-hydration-incremental-activity.md); [#9206, “Domain state”](../components/9206-work-item-lifecycle-fleet-subcommands.md); [prior art B, “Hydration, cursors, and attribution” and “Failure matrix”](../research/track-b-state-of-art.md); [Track B census, “Finding” and “Pinned existing test evidence and gaps”](../census/track-b.md).

## Options considered

1. **Immutable versioned generations, per-source/per-endpoint cursors, durable page ingest and first-party attribution (recommended).** Provides bounded reads, recoverability, and honest completeness.
2. **One mutable context file and one `last_checked_at`.** Cheap but crashes can expose partial state, and edits/reordered pages can disappear behind cursor.
3. **Re-fetch full history for every request.** Simplifies cursor correctness but causes repeated O(history) API, memory, latency, and rate-limit cost.
4. **Treat username/actor and comment creation ID as session attribution and cursor.** Easy but false with shared actors, edits, and non-monotonic source updates.
5. **Do nothing.** Agents retain incomplete or stale context without a reliable distinction between empty and unavailable.

**Tradeoff comparison:** Immutable generations cost artifact metadata and publication coordination but give stronger recovery and extensible source adapters with memory bounded by page size. Full rereads simplify cursor state but scale poorly in API volume, latency, and rate-limit use. A mutable file/one timestamp is cheapest and fastest only on the happy path; crash consistency and edit coverage degrade as sources/history grow. Username attribution is easy to implement but unreliable under shared actors; operation mapping costs durable correlation metadata for materially better attribution.

## Proposed operational decision

Publish immutable context generations with a manifest that reports status/completeness per source and endpoint. Stage bounded page/item artifacts, verify hashes and safe paths, write manifest last, then atomically switch a current-generation pointer when storage supports it. If the existing store cannot provide atomic publication, use a versioned conditional pointer and explicitly report weaker semantics; preserve the previous complete generation on refresh failure. Never present partial/truncated/forbidden as complete or silently treat missing as empty. For new self-describing manifests, compute `integrity.manifest_digest` with RFC 8785 JCS + SHA-256 and a manifest-purpose/version separator while omitting that digest field itself; artifact hashes cover exact stored bytes. Shared HYD-01 owns golden vectors and negative cases; no self-referential whole-manifest hash.

Use a cursor key scoped by canonical resource, endpoint, query shape, API version, auth-scope class, and generation. Fetch, validate, durably persist page events/artifact refs and cursor continuation as one transaction where supported; cursor advances only after all page data is durable. If artifact store and DB cannot transact together, stage immutable objects, persist an ingest intent, verify hashes, then commit event+cursor; garbage collect only unreferenced staging data under retention policy.

Track source-native event/resource ID separately from resource revision/updated marker, delivery ID, occurrence time, observed time, and source update time. Edits revise the same logical item and append an observation. Deletions/minimizations are recorded when authoritative; otherwise periodic full reconciliation is a bounded detection strategy with an explicit visibility gap. Attribute Fleet-caused activity through operation/session mapping and returned remote IDs; shared GitHub actor alone is insufficient. Unknown attribution stays uncertain and is included by default in external-only results.

## Rationale and evidence boundary

**Facts:** charter requests hydration manifests, per-source cursors, overlap/dedup, attribution and completeness. Research documents API pagination/edit limitations and no general snapshot across endpoints. The component design proposes bounded memory and immutable generation publication. These API facts are cited as prior-art findings; live API behavior and deployed artifact store were not tested here.

**Inference:** immutable generations and per-source cursor state trade metadata/storage for predictable crash recovery and avoid presenting mixed heads as one snapshot. No numeric volume/latency measurements are claimed.

## Consequences, compatibility, and migration

- Keep queue rows small; artifacts remain under existing storage/path authority and are repository/resource-scoped with safe names.
- Unknown legacy cursors require conservative full initial fetch; do not fabricate a high-water mark from old timestamps.
- Existing artifact root, atomic rename/CAS support, retention, and access controls must be established before persistence implementation. Schema/storage changes gated by #3424/#8575/#8579 and QTI/#9241.
- Hydration failure returns prior generation plus partial/new status; no silent fallback to “complete”. Request/time/byte/page caps are explicit capability limits, not truncation hidden from the caller.
- This record does not authorize polling scheduler, new database, API access, or retention change.

## Negative cases and test oracles

- **Crash oracle:** fail before/after item persistence, manifest, and pointer switch; readers see old complete generation or new complete generation, never partial current pointer.
- **Cursor MBT:** duplicate/reordered pages and crash on page N cannot skip a durably unprocessed item; repeat ingestion is idempotent.
- **Revision oracle:** edit old comment after creation cursor; one logical comment with new revision and appended observation, not a duplicate creation or omission.
- **Completeness oracle:** forbidden, unavailable, truncated/capped, unknown source or changed PR head cannot yield complete bundle.
- **Attribution oracle:** shared bot username across sessions without operation mapping remains uncertain and included in external-only.
- **Digest oracle:** manifest digest changes on any covered version/artifact/completeness change, excludes only its own digest field, and uses the independent RFC 8785 golden bytes; artifact digest uses raw stored bytes.
- **Resource oracle:** streaming memory remains within configured page/item bound; path identity prevents same issue number in different repos colliding.

## Revisit triggers

Reopen if current adapter exposes a stronger opaque cursor or snapshot guarantee, storage owner provides a different durable artifact primitive, measured payload scale requires alternate indexing, or provider endpoints change edit/deletion visibility. Any such change must preserve durable-page-before-cursor semantics or document an equivalent oracle.

## Implementation, research, and source links

- Existing issues: [#9207](https://github.com/developing-today/code/issues/9207), [#9206](https://github.com/developing-today/code/issues/9206), [#9209](https://github.com/developing-today/code/issues/9209), [#9198](https://github.com/developing-today/code/issues/9198), [#9241](https://github.com/developing-today/code/issues/9241).
- Existing granular component keys: #9207 `HYD-01`–`HYD-09`; #9206 `CLI-03/09`; #9209 `REL-03/07/09`. Research: `research/track-b-state-of-art.md`, “Hydration, cursors, and attribution” and `research/track-b-sources.json`; repository facts at pinned commit in `census/track-b.md`.
