# Creator tooling & insight — owner basics v1 design (#6308)

- **Issue:** [#6308](https://github.com/developing-today/code/issues/6308), core topic [#6282](https://github.com/developing-today/code/issues/6282), Phase 14 plan of record [#6312](https://github.com/developing-today/code/issues/6312).
- **Date:** 2026-11-18.
- **Status:** first-iteration design for review; implementation is explicitly out of scope.
- **Entry condition:** met before design work. Task 18475 recorded on #6308 that #6282 has first-pass findings and the Phase 14 design queue is moving. The plan-of-record roadmap lists this v1 design and its v2-v4 ladder as #6308-#6311.
- **Principles:** #1240 — local-first; hostable by anyone at any scale; P2P behavior identical to hosted; our hosted instance is a public backstop, never a requirement.

## Decision in brief

Build an owner-facing, read-only world-inspection projection from the world's own journal and already-produced runtime metrics. The interface is a local `id world owner` report (with a stable machine-readable form) that can be rendered by any shell. It reports bounded, explicitly timestamped counts and storage estimates; it does not introduce a hosted analytics service, a second source of truth, or a continuous event-upload path. Moderation stays a separate, authenticated command path: each action is authorized by the existing world role model and durably records the actor, target, action, and outcome. The report may summarize moderation activity only from those durable records; seeing a report never grants authority.

V1 answers four owner questions for one locally available world: who joined and when, how many members are active under a stated window, what program runs have occurred and their outcomes, and how much on-disk space the world consumes. It gives no cross-world/global analytics, public directory ranking, user profiling, remote control plane, or new moderation policy. The world journal remains authoritative for world facts; local filesystem inspection is authoritative for bytes-on-disk; ephemeral runtime metrics are labeled as a snapshot, not silently persisted as journal facts.

## 1. Scope and non-goals

**In scope**

- A local, owner-authorized summary for one world, usable offline and on a self-hosted machine.
- Join history and active-member count, with explicit time window and source/limitations.
- Program-run count and recent outcomes when the existing journal/runtime has enough evidence; missing historical attribution is reported as unavailable, never inferred.
- Storage footprint for the world's directory, split into journal, content-addressed modules, and other recognized files; identify whether the value is exact or a best-effort filesystem walk.
- Audited moderation actions using the existing authority checks, with an append-only, replayable record and a read-only audit view.
- Text and JSON output with stable schema/versioning suitable for later shell presentation.

**Out of scope**

- Hosted multi-tenant dashboards, remote telemetry, analytics upload, or dependence on our public instance.
- Cross-world search, discovery, leaderboards, social graphs, user behavior scoring, or individual engagement profiles.
- New roles, self-appointment, delegated moderation, configurable moderation rules, or new moderation powers. Those need their own governance design; this v1 honors the existing roles.
- Retention, export, billing/quota enforcement, storage compaction, backup, and journal mutation.
- Live subscriptions/streaming dashboards. V1 is an on-demand snapshot.
- Claiming that the data is complete when old journal entries or old versions did not capture the needed fact.

## 2. User-visible contract

The owner report is a snapshot with a common `generated_at`, a `world_id`, `schema_version`, and per-section provenance/status. Example conceptual shape (field names illustrative until implementation issue):

```json
{
  "schema_version": 1,
  "world_id": "…",
  "generated_at": "…",
  "window": { "from": "…", "to": "…" },
  "joins": { "count": 0, "available_since": "…", "complete": true },
  "members": { "active_count": 0, "definition": "connected within window", "as_of": "…" },
  "program_runs": { "count": null, "reason": "not recorded by this journal version" },
  "storage": { "total_bytes": 0, "exact": false, "as_of": "…" }
}
```

`null` plus a reason means unavailable, not zero. Every count names its time bounds and source. Human-readable output uses the same values and labels. JSON consumers must tolerate added optional fields but reject an unsupported major schema version. A query that cannot be answered from available records returns an explicit unavailable status rather than scanning unrelated worlds or contacting a server.

The initial CLI surface should follow existing `id world` command conventions: `id world owner <world> [--since <duration>] [--json]`, with a default recent window documented in help. The command requires the same local world access/owner authorization as other administrative inspection, does not start or wake an asleep world solely to gather volatile metrics, and never mutates the journal. If the world is not loaded, it may replay its local journal for durable facts and report live-only facts unavailable.

## 3. Fact sources and semantics

| Question       | Authority/source                                                                                                      | Meaning and limits                                                                                                                                                                                                                                                                                                 |
| -------------- | --------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Joins          | Durable join/member journal entries, replayed through the existing world model                                        | A join is a recorded successful membership event, not an attempted connection. Deduplicate only according to the journal's stable participant identity and documented rejoin semantics. `available_since` is the earliest point from which the current format can answer completely.                               |
| Active members | Current in-memory participant/session state when available; otherwise unavailable                                     | “Active” means connected at the report snapshot, not “recently active.” If product chooses a time-window proxy, it must be a separately named metric, not this field. No last-seen profile is invented from unrelated events.                                                                                      |
| Program runs   | Existing durable run/start/finish/outcome records if present; otherwise runtime counters for the current process only | The design does not add a journal entry just to make historical charts. A process-local count is marked `complete: false`, scoped to process start, and disappears on restart. Durable history requires a separate, versioned journal decision and migration/compatibility plan.                                   |
| Storage        | Walk of the selected world's data directory, including journal and content-addressed program modules                  | Bytes-on-disk is a point-in-time local filesystem observation, may race with writes, and may differ due to sparse/compressed files or filesystem accounting. Report logical file bytes first; optionally allocated bytes only where portable and clearly labeled. Do not traverse other worlds or symlink targets. |

The report is deliberately conservative. It shows observability gaps directly, making them actionable without fabricating certainty. A future design may add durable run provenance if the product case justifies its journal and privacy cost.

## 4. Module and seam

Add one read-only owner-projection module behind a small interface: accept a world identifier, a bounded time window, a durable journal reader/replay view, an optional live snapshot provider, and a filesystem metadata reader; return a typed report with per-field availability/provenance. The command is an adapter that performs authorization, supplies those inputs, and formats text/JSON. It must not own aggregation logic. Tests call the projection interface with in-memory journal, live-state, and filesystem adapters, so the oracle is the supplied event/state model and byte fixtures—not a second implementation of the same query.

Keep the seam inside `id`'s world inspection path, adjacent to existing world listing/inspection commands. Do not add a web server, database, generic analytics bus, or provider capability. A later shell can consume the stable report without changing its semantics. Filesystem walking and journal replay are bounded to one world and stream records rather than loading an unbounded history into memory.

## 5. Moderation authority and audit

The report has no moderation affordances that imply permission. A moderator action continues through the existing world command/handle authorization path. The actor must be resolved from authenticated identity and checked against the role/scope for that exact action; a client-supplied actor name is never authority. A denied action is not silently treated as success.

For every successful moderation mutation supported by the existing model (currently participant revocation), persist the acting participant, target participant, action, timestamp/order, and result in the existing durable journal entry or an explicitly versioned successor. Replay must reconstruct the same state and audit record. The actor is the actual authorized principal; absent actor identity is a hard error, not `None` or “system.” Failed authorization attempts may be emitted to local diagnostics but are not durable world events unless a separate privacy/retention decision requires that. The owner report may present the durable successful-action history; it must not expose secret material or broaden who may read it.

V1 does not add hide/ban/kick variants, a world moderator roster, self-service appointments, or a separate audit database. If the current journal already records the actor for revocation, reuse that record and prove the existing path end-to-end; if it does not, implementation must first make the smallest compatible journal change under the journal-version rules. Directory/server moderation and world participant moderation remain distinct authorities.

## 6. Local-first and deployment parity

All calculations happen on the machine holding the world's journal and files. The same binary and report semantics apply to laptop, home server, and hosted instance. P2P clients do not receive owner data unless an existing, separately authorized owner interface explicitly returns it; transport topology does not change access. The public hosted instance is optional: no account, external analytics backend, or network connection is needed to run the report. The report is recomputable from local state and can be omitted without changing world behavior.

No generated report is written into the journal. If an operator chooses to save output, that is an ordinary local file outside authoritative world state. Time values are UTC; storage and live counts say when measured. A host may impose operational limits on report time/memory but cannot silently return partial results as complete.

## 7. Privacy, safety, and failure behavior

- Treat participant identifiers and moderation records as sensitive world data. Enforce the existing owner/admin read path; avoid names or free-text data where stable IDs suffice.
- Never emit tokens, capability secrets, private keys, or raw journal payloads in the report or errors.
- Malformed, unknown-version, or truncated journal input follows the existing journal refusal/recovery contract; do not make a partial owner report look complete.
- Permission denial, missing world, unsupported journal version, unavailable live state, incomplete history, and filesystem race are distinct typed outcomes.
- Apply configurable output limits to recent event samples; aggregate counts may cover the full requested bounded window. No unbounded all-history response by default.

## 8. Acceptance and verification oracle

The implementation is done when:

1. A local owner can request text and JSON reports without network access; a non-owner cannot obtain the report through the same command.
2. A fixture journal yields exact join counts and time bounds; replays and repeated report calls produce identical durable sections (model oracle).
3. Active-member counts match a supplied live-state fixture at one snapshot and are explicitly unavailable without it (fixture oracle).
4. Program-run output distinguishes durable history, process-local counters, and unavailable; no missing record becomes zero (invariant oracle).
5. Storage totals match fixture files, stay within the selected world root, and report best-effort/race semantics (known-byte fixture oracle).
6. Successful moderation replay preserves actor/target/action; unauthorized requests cannot append an event; no actor is accepted from untrusted request data (model + invariant oracle).
7. No request creates network traffic or writes a report/journal event. Two hosts with identical world data produce identical durable sections; host-local storage/live fields are explicitly allowed to differ.
8. Tests cover unknown journal versions, corrupt/truncated records, empty world, rejoin semantics, window edges, symlinks, disappearing files, and oversized histories. Property-based coverage is appropriate for time-window aggregation and report serialization invariants.

## 9. Implementation breakdown (proposed follow-up issues)

These are ordered by dependency. File ownership is deliberately narrow; IDs are assigned when filed. Implementation should begin only after this design is reviewed and accepted.

1. **Owner-report domain types and read-only projection.** Define report/schema types, provenance and unavailable states; implement bounded joins aggregation from journal replay behind injected readers. Own `pkgs/id/src/world/owner_report.rs` (or the established inspection module), report tests, and minimal exports. Depends on existing journal facts; no CLI.
2. **Live membership and program-run sources.** Add a snapshot adapter for connected participants and expose any already-existing run counters with explicit process scope. Do not add durable journal formats in this step. If durable run history is required, stop and propose a separate journal change. Own the world actor/runtime integration and fixture tests.
3. **World storage inventory.** Add a one-world safe directory walk and byte accounting with no symlink following, clear exactness/provenance, and failure states. Own the storage inspection module and filesystem fixture tests.
4. **Owner CLI adapter and JSON contract.** Add `id world owner` with window selection, authorization, text/JSON rendering, help and schema compatibility tests. Connect steps 1–3; no web UI or network endpoint.
5. **Moderation audit path verification.** Trace existing role checks and durable actor attribution for revocation; close any missing-actor path with the smallest compatible journal change, replay tests, and migration/version notes. This may be implemented ahead of step 4 because the audit is an invariant, but must not invent new powers. Own authorization/handle/journal files and focused integration tests.
6. **End-to-end parity and privacy acceptance.** Add offline/local-host acceptance tests for denied access, no network/write side effects, deterministic durable report sections, incomplete-data signaling, and bounded output. Document operator-facing semantics and limits.

All follow-ups link back to #6308 and core topic #6282, and are children of #6308 where GitHub supports native sub-issues. Steps 1 and 5 can be parallel only if their file ownership remains disjoint; step 4 depends on 1–3; step 6 depends on all preceding work. No step implements cross-world analytics or a hosted dashboard.

## 10. Open decisions for review

- Whether “active members” should remain strictly “connected now” (recommended for semantic honesty) or whether a separate “seen in last N” metric is wanted. Recommendation: keep only connected-now in v1; add history only with an explicit event/retention policy.
- Whether program-run history is important enough to justify durable journal events. Recommendation: v1 exposes only facts already durable and labels process-local counters; defer journal growth until evidence from real use.
- Whether the owner report is admin-only or also available to a world owner role distinct from admin. Recommendation: use the existing strongest local owner/admin authorization for v1 and do not create a new role in this issue.
- Whether storage reports logical bytes only or allocated bytes where available. Recommendation: logical bytes as portable baseline; allocated bytes may be optional metadata.

## 11. References and implementation grounding

- #6308 defines the owner-basics scope and entry condition; #6282 owns the topic-level findings; #6312 is the Phase 14 plan of record; #1240 establishes the four invariants above.
- Existing role and moderation vocabulary is documented in `doc/2026-11-08T13-45-00Z_design_id_world_governance/2026-11-08T13-45-00Z_design_id_world_governance.md` (§§1–3): world participant scopes and directory moderation are distinct; the world moderator bit is admin-issued and tied to revocation; governance is not a grantable provider.
- Hosted/local parity and the local journal as world authority are specified in `doc/2026-11-16_design_id6283_hosted_worlds_v1.md` (§§1, 3, 5, 7). This design reuses that model rather than introducing a hosted-only analytics plane.
- The current Phase 14 map in `planning/roadmaps/id.md` lists #6282 and its #6308–#6311 ladder. The implementation breakdown below is limited to #6308 v1; later ladder items remain outside scope.

These references establish constraints, not a claim that every proposed data source already exists. The follow-up issues explicitly require implementation owners to inspect the current journal/runtime and preserve “unavailable” where evidence is absent.
