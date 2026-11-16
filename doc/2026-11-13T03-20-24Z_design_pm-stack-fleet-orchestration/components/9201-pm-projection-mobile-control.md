# PM projection and mobile control surface

**Component:** #9201 · **Track:** A · **Status:** implementation-ready design, not implementation authorization · **Evidence baseline:** pinned commit `a038f084dbeb525494d7bb928ad91f597db2ac51`; bounded issue/PR evidence dated 2026-11-13. V1 means only code/docs observed at that commit; deployed status is unverified. This design consumes the current PM Stack and Fleet boundaries. It does not authorize a new PM database, a second Project writer, queue mutation, or dispatch.

## 1. Requirement and decision summary

Give an authorized user a mobile-usable, explainable projection of work and a safe way to request actions from the existing shared Fleet operation layer. The UI is not an authority for queue state, priority policy, Project values, or session state. It submits an action intent; a policy-owning Fleet consumer re-reads current authoritative state, re-authorizes the actor and resource, checks the expected revision, applies or rejects through its existing owner boundary, and returns an auditable result.

Source coverage: handoff H-SURVIVE-A, H-DISTRIBUTED, H-SCENARIOS; charter C-3 through C-8, C-16/C-17; original O-PART1 §§5, 8, 11–14 and 18, mobile §§12–13; supplemental S-STATE/S-READY/S-MOBILE. Source 02 keeps intrinsic urgency, importance, effective scheduling pressure, workflow/ownership, and dispatch directive separate. `do-now` is not a score. Priority/label normalization remains #9200's authority; lifecycle and dispatch policy remain their respective component contracts.

### Settled constraints for this design

1. **Project #4 stays single-writer.** `pm-stack/README.md:94-112` says Project `Kind=Epic` is canonical, `epic` is its maintained mirror/gate, sub-issue parentage is hierarchy, `blockedBy`/`blocking` are dependencies, and the orchestrator alone writes Project fields. Mobile and agents submit intents; the current orchestrator or its formally designated Fleet operation applies Project changes. This component does not move authority to the dashboard.
2. **Consume the existing PM Stack.** Use #9062's collector and #9070's genealogy read model where available, and #9073–#9076 for Metadata/session/ingestion foundations. Hosted parity consumes #9079 PM service and #9081 Fleet TypeScript equivalent. None is treated as complete just because its epic is closed; relevant components remain open.
3. **No new policy in the browser.** The same validated Fleet service/CLI contract must process browser, comment, and agent requests. Client-side projections may sort/filter a fetched bounded result for presentation, but may not authorize, rank claim candidates, calculate canonical urgency, resolve conflicts, or perform a Project/GitHub/session mutation.
4. **Intent and effect are distinct.** User-facing receipts render the shared Result outcomes (`accepted`, `running`, `pending_sync`, `partial`, `conflict`, `uncertain`, `rejected`, `verified_complete`) with a reason and next action. A policy-blocked request is `rejected` with a stable eligibility reason code, not a new wire outcome. These are not interchangeable with item lifecycle states such as ready, claimed, parked, or completed.
5. **Offline never means stale authority.** A stale/offline board is read-only. A user may keep a local draft intent; reconnect must fetch a fresh revision and show the new preview, then require explicit reconfirmation. Never silently replay priority/state changes, dispatches, merges, or other consequential actions against stale state.

## 2. V1 — observed at the pinned code, not inferred deployment

- `pm-stack/aggregator/src/model.ts:56-116,178-204` has Project-pool item/ref types, Project `status/area/horizon/kind`, native `parentRef`, `blockedBy`/`blocks`, and board card/source state. Project/issue references are repo-qualified. This is a read model, not a queue work-item authority.
- `pm-stack/aggregator/src/queue-view.ts:1-4,10-34,99-220,325-402` explicitly builds a cross-project projection from board, Project pool, issue refs, and orchestration. Its `inProgress/blocked/doneRecently/nextUp` values are not the broker's live eligible queue or claim order. Do not use this view as a Fleet queue truth source.
- `pm-stack/projector/src/queue-view.ts:1-20,104-140` consumes externally supplied broker task JSON plus projector hot-feed events/checkpoint and emits a claim-order **snapshot**; it does not fetch the live broker. Treat the hot feed as partial remote activity, not a complete work journal or canonical lifecycle view.
- `pm-stack/aggregator/src/actions.ts:1-44,76-139,211-239` already defines an append-only-ish action request pattern and verbs for triage, status-request, session, and preferences. Existing action state is `new/applied/rejected/superseded`; it lacks the full user-facing receipt lifecycle designed here. The action store is a local JSON-backed process store, not a new canonical PM database.
- `pm-stack/aggregator/src/server.ts:1818-1858,1992-2010,2136-2150` gates dashboard writes on Access identity, validates requests, stores intent, exposes action results and an action read endpoint. Comments in `actions.ts:9-44` explicitly say the dashboard does not write GitHub, Project, Kaneo, or session servers; consumers apply and report back.
- `pm-stack/aggregator/src/actions.ts:25-33` makes the orchestrator the Project single writer and enables session controls only for live-control sources. `pm-stack/aggregator/src/status-taxonomy.ts` and the #910 convention own existing Project Status vocabulary. Do not invent a second Status field.
- **Additional V1 integration input: PM projector (`a038f084...`).** `pm-stack/projector/README.md:3-8,23-30,113-133,145-179` says it is disposable projection glue that never writes back to GitHub/Project/Kaneo/Pad/Grist; `/github/webhook` handles issue/comment/PR/review/push events and exposes reconcile cursors plus a `pmp_hot_events` ledger. `src/webhook.ts:1-58` already provides bounded raw-body HMAC-SHA256 verification with constant-time compare; `src/server.ts:146-201` applies projection, publishes freshness and records projection delivery `applied/ignored`; `src/store.ts:499-538` only queries projector delivery state. The hot feed has server-pinned high-water and monotone consumer ACK (`src/store.ts:602-739`), with tests `test/webhook-delivery-id.test.ts` and `test/hot.test.ts`. These are partial ingress/activity/read seams; they do **not** accept Fleet commands, authorize actors per verb, persist canonical operation truth, or constitute a generic journal/outbox. Projector deployment/fanout liveness is unverified.
- The repo copy of the Project #4 contract is current-state documentation, but this census did not read live Project schema/field values or verify a production mutation. Source freshness, missing facets, and stale feed must remain visible, never coerced to zero or fabricated as current.
- No acceptance evidence establishes all 15 original mobile actions, queue-true claim state, all desired board views, offline drafts/revisions, or mobile WCAG/performance behavior. Those remain design/implementation work, not V1 claims.
- Existing expedited items have a ratified `expedite` mapping/readback at queue priority **15** (#7030 evidence); this is neither the charter's one-shot `do-now` directive nor a basis to fold dispatch into priority. Preserve both semantics and use #9200's mapping.

## 3. V2 — contracts and policy

### 3.1 Read model: one projection, provenance per fact

Expose a versioned read operation over the existing fleet-feed/PM Stack sources; do not add a PM database. The shape below adopts the shared `(authority_id, work_id)` identity and `snake_case` wire convention in `architecture.md`; every identifier/revision that might exceed JavaScript's safe integer range is a canonical decimal string. Each fact carries source/ref, observed-at, revision, and freshness or an explicit absent/invalid/error state:

```json
{
  "schema_version": "pm_work_view/v1",
  "generated_at": "<ISO-8601>",
  "work": { "authority_id": "fleet-authority", "work_id": "842" },
  "source_refs": [{ "kind": "github_issue", "repository_id": "…", "resource_id": "…" }],
  "revisions": { "fleet_work_version": "…", "github_revision": "…", "project_revision": "…", "activity_snapshot_ref": "…" },
  "facts": {
    "intrinsic_urgency": { "value": "20", "source": "fleet-policy", "revision": "…" },
    "importance": { "state": "absent" },
    "effective_pressure": { "state": "not_yet_defined_by_9200" },
    "dispatch_directive": { "value": "normal", "source": "fleet" },
    "workflow_state": { "value": "open", "source": "fleet-lifecycle" },
    "eligibility": { "state": "eligible", "hard_blocker_refs": [] },
    "claim": { "state": "unclaimed" },
    "session": { "state": "none" },
    "project": { "status": "Todo", "source": "github_project_4" },
    "relations": { "parent_refs": [], "hard_blocker_refs": [], "advisory_refs": [] }
  },
  "projection": { "column": "Ready", "reason_codes": ["…"], "conflicts": [] },
  "available_actions": [{ "verb": "request_triage", "enabled": true, "reason_code": null }],
  "freshness": { "state": "live", "facets": {} }
}
```

Rules:

- A GitHub `owner/repo#number` is a display/input locator only; retain provider/repository stable ids and kind so rename/equal-number refs cannot merge identities. Resolve `fleet://work/842` only with unambiguous authenticated authority context. An absent dimension is absent, not zero, not guessed from a label, and not synthesized from another score. Canonical urgency/label interpretation comes from #9200. Until then show source label/score and `unmapped` where needed.
- If the view explains candidate eligibility, it must preserve V1 conjunction: both the task's declared requirements and the claimer's explicit filter must pass. Do not use OR, and do not use claimer identity as a filter. The client never reimplements candidate selection.
- Board column is a **projection with explanation**, not an authoritative writable lifecycle enum. Return evidence contributing to the column and conflicts. Separate underlying GitHub issue/PR state, queue state, claim/lease, session state, Project status, and dispatch request state.
- A claimed task is `In Progress` only if an authoritative fresh claim read says it is claimed; display worker/lane plus a bound session only when readback confirms the task/session binding. A `work/assigned` label, dispatch intent, queue row status, or session shell alone is not proof of active execution.
- Product workflow state, eligibility/readiness, and board projection are separate facets: e.g. workflow `open` + eligibility `eligible` can project to board `Ready`, while terminal `completed` is a distinct lifecycle state. Parking is typed `work.park` with reason/checkpoint preconditions, never arbitrary Project `set_status`; `project.request_status` separately names a Project #4 field and routes through its sole writer.
- A blocked or parked task keeps that state and its recorded reason/checkpoint/dependencies; urgency and owner assignment cannot promote it to Ready. `manual_only` is a hard eligibility property, not a directive; it remains visible but is excluded from automatic claim. `do_now` is a distinct one-shot directive and cannot bypass `manual_only` or hard gates. `ready/todo` is a derived eligible projection: eligible/unblocked + executable High-and-above under #9200, or explicitly prepared, or explicitly owner-assigned by authorized policy. All other actionable work projects to Backlog. Explicit Board/Project statuses may override only through a documented source authority/precedence rule, not a client heuristic.
- Do not show Done from a closed queue task, merged PR, closed issue, or `status:done` label in isolation. Visible `Done` requires the designated completion policy's verified terminal receipt and any required issue/PR/queue reconciliation; otherwise show conflict/pending. Project #4's one Status value cannot encode every independent state machine; preserve facets and explain lossy mapping.
- Needed views (filters over the same read model, never separate rules): All; Inbox/Triage; Ready/Todo; High/critical path; Blocking others; My work; Active agents; Awaiting review; Parked/Blocked; Needs triage; Recently changed; Cold backlog; by repo/project/epic/owner/lane. Each view discloses its filter and source freshness.
- Session list/detail gives provider adapter name, supported capabilities, task binding state, observed activity and timestamps; do not display unsupported controls as live. A Slack post is a `delivered-to-channel` receipt, not session acknowledgment or progress.

### 3.2 Action request / result contract

Use the single shared `Command`/`Result` contract in `architecture.md`; do not introduce a competing action schema. `Command` carries `schema_version`, `operation_id`, `idempotency_key`, `request_digest`, work identity `(authority_id,work_id)`, `kind`, typed `arguments`, `actor_ref`, source/event/session refs, expected work version, lease generation, observed snapshot ref, reason and dry-run. `Result` carries common outcome, effects, warnings/errors, next action and observed-at. Authentication supplies principal/capabilities: `actor_ref` and client `authorized` fields are audit context only, never proof. The browser may preview but cannot mint actor authority. Wire field names remain snake_case and unsafe integers become decimal strings.

```json
{
  "schema_version": "fleet.command/v1",
  "operation_id": "op-…",
  "idempotency_key": "…",
  "request_digest": "…",
  "work": { "authority_id": "fleet-authority", "work_id": "842" },
  "kind": "work.park",
  "arguments": { "reason": "…", "checkpoint_ref": "…" },
  "actor_ref": "access-principal-reference",
  "source_ref": "mobile_action/…",
  "expected_work_version": "…",
  "observed_snapshot_ref": "…",
  "reason": { "code": "user_requested_park", "evidence_refs": [] },
  "dry_run": false
}
```

The read/preview endpoint returns exact intended effects, current value/revision, affected authority, policy explanation, reauthorization/confirmation requirements and an expiry through the shared `Result` shape. The mutation endpoint revalidates identity, permissions, current item and policy on apply; checks `expected_work_version`; returns `conflict` with fresh observation on stale inputs; and records the shared operation receipt. A blocked effect uses `rejected` plus reason code/next action. For Project writes, forward to the designated orchestrator single writer. For queue mutations/session operations, invoke the existing validated Fleet command contract. For GitHub edits use the existing Fleet binary `gh.rs`/#9016 owner; this design does not claim the creation outbox is a general lifecycle outbox. No browser directly writes a Project field, database, label, comment, queue row, or session API.

Use common result outcomes `rejected`, `accepted`, `running`, `pending_sync`, `partial`, `conflict`, `uncertain`, and `verified_complete`. Individual effects separately carry requested, observed and verified states; a verified read is not work-item completion. Keep raw action-store state separately; do not silently relabel `new/applied/rejected/superseded` as these outcomes. A timeout returns a resumable receipt, not fictional cancellation; dry-run creates no work-state, journal, outbox or remote mutation.

Every receipt contains operation/idempotency key, work identity, source/delivery reference, verb, authenticated principal/capabilities, created/accepted/started/finished timestamps, expected and observed revisions, owning consumer/run, outcome, source-before/source-after, exact effect diff, policy/reason code, and downstream numeric Project/GitHub/task ids as canonical decimal strings. Opaque provider refs, including OpenCode session ids, remain opaque strings without numeric coercion. Include verification source/readback and retry/recovery action. Exclude secrets and untrusted payload from executable context. V1 worker-ID leases are not evidence of a secret lease token or monotone fence. A future Fleet fence cannot stop an external GitHub request already in flight; expired-owner receipt recording is restricted to reporting the outcome of an already-authorized operation and cannot start a new mutation or overwrite newer ownership. Projected final state identifies source and observation time.

### 3.3 Controls inventory (all 15 original actions)

1. Search work item. 2. Inspect queue metadata. 3. Read issue and PR summaries. 4. Add a private note. 5. Add/reply to a public comment. 6. Change urgency. 7. Set/clear `do-now` (one-shot, separate from score; preserve ratified `expedite=15`/`next-up=25`). 8. Request permitted Ready/Parked/Blocked state. 9. Request triage. 10. Request an agent (normal enqueue or explicit direct-dispatch semantics are shown distinctly). 11. Inspect current session status. 12. Send a live-session instruction. 13. Request a status report. 14. Inspect hard/advisory dependencies. 15. Inspect recent activity and verified completion outcomes.

Defaults: private note unless public mirror is explicitly selected and previewed; status changes are rejected if they violate lifecycle/lease/dependency rules; reprioritization uses #9200's canonical model and immutable before/after reason; `do-now` requests a directive and never modifies urgency; dispatch while parked requires authorization, fresh blocker/lease check, and explicit “start now” confirmation; merge/close/stop are higher-risk operations and are not a generic one-tap action.

### 3.4 Mobile and accessibility envelope

These are **acceptance targets**, not measured V1 performance or an assertion that the current app complies:

- Support a 360 CSS-pixel-wide viewport and 200% zoom without horizontal scrolling for primary queue/task/action flows; responsive compact list first, details on demand.
- WCAG 2.2 AA review target. Pointer targets at least 24×24 CSS px or applicable spacing exception; use 44×44 px preferred sizing for primary mobile actions. Keyboard-only navigation, visible focus, semantic headings/labels, accessible status announcements, contrast, reduced motion, screen-reader usable action results, and no gesture-only path.
- No hidden hover-only actions; no accidental double-submit; destructive/consequential actions have accessible confirmation naming target/effect. Preserve keyboard parity on desktop.
- Proposed budgets to validate with representative fixtures: initial useful queue/read content p75 ≤2.5s and action submission p95 ≤1s to an accepted receipt over the supported normal path; these are targets requiring measurement and may be revised from current environment baselines. A slow source is shown as stale/partial, not silently omitted or fabricated.

### 3.5 Conflicts, stale state, and recovery

| Condition                                     | UI and service behavior                                                                                        | Durable evidence / recovery                                                              |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| One source stale/unavailable                  | Render available facts with facet-level stale/error label; disable mutations depending on that facet           | Read freshness/source timestamp; retry read, never report absent as zero                 |
| Revision changed after preview                | Return `conflict`; include current state/diff                                                                  | User reviews and reconfirms; do not auto-rebase consequential intent                     |
| Project/GitHub write pending                  | Show `pending_sync`, target field, owning writer, next retry/recovery                                          | Existing outbox/result source; UI never claims verified completion until readback        |
| Claimed item lacks verified session           | Show `claimed / binding unknown`, not active session or “working”                                              | Request reconciliation; don't dispatch another worker blindly                            |
| Parked/blocked item has active binding        | Preserve parked/blocked plus binding discrepancy; disable new start until #9179/#9178 recovery policy resolves | Existing owner-run reconciliation; never hide as Backlog or duplicate-dispatch           |
| Offline or stale client                       | Read-only; keep draft as unsubmitted local intent                                                              | Re-fetch, preview changed target/revision, explicit re-confirm; discard remains possible |
| Unauthorized actor/expired role               | Reject before effect; explain required role without leaking private data                                       | Audit denied operation; refresh Access identity                                          |
| Duplicate submit / retry                      | Same operation key returns original status/result                                                              | Idempotency lookup; no second external effect                                            |
| Project column cannot encode lifecycle facets | Show Project Status and Fleet/queue/PR/session facets separately, plus mapping explanation                     | Projection conflict/desired-vs-observed revision; no silent collapse                     |

### 3.6 Authority and source-compatibility matrix

| Fact/action                                                   | Authority / owner                                                                                            | UI role                                                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| Project #4 fields, especially Status/Kind/Area/Horizon        | Existing orchestrator single writer; Project #4 actual fields                                                | Preview request, show desired vs observed; no direct write                                         |
| Issue/PR canonical state, description/comments/reviews/checks | GitHub, mutated through the existing Fleet `gh.rs` binary/API owner and its established operation discipline | Read; explicit public action request with conflict check                                           |
| Queue item, eligibility, claim/lease, checkpoint              | Existing fleet-queue broker/validated fleet command layer                                                    | Show broker read with revision; request allowed transition only                                    |
| Intrinsic urgency and label normalization                     | #9200 policy; avoid guessing while unresolved                                                                | Show score+label provenance; submit policy-mediated request                                        |
| Importance/effective pressure/dependencies                    | Future typed Fleet metadata + #9199/#9200 contract                                                           | Display absent/unavailable if not sourced; no fabricated fallback                                  |
| Session id, state, adapter capabilities, acknowledgment       | Provider adapter / bound task readback                                                                       | Send only advertised supported operations; distinguish posted, delivered, acknowledged, progressed |
| Private note                                                  | Fleet-owned private note/journal path, if available; explicit authorization                                  | Private by default; explicit preview/consent for public mirror                                     |
| Dispatch directive / launch receipt                           | Existing validated dispatch service and queue/session binding protocol                                       | Request, display reservation/launch/bind/ack/verification stages distinctly                        |

Current adjacent work does not establish the full owner model. #3424/#8579 and #8581 may materially affect durable queue facts and operation-intent design; #8580 mirror remains open. QTI identity recovery is now tracked as research #9241 under #9198, but its answer is not a prerequisite for V2 read-only projection design, mobile UX, or compatibility adapters. This proposal introduces no new data store and gates schema-coupled persistence only on the #9241 crosswalk plus the audited contracts.

## 4. V3 — implementation slices (ready-to-file specs; not filed here)

Files listed are future ownership boundaries; new files are explicitly proposed, not V1. Commands below are exact future commands against the named existing package/target. Each issue is one deliverable with local requirement IDs and a decidable oracle; a command is not acceptance until its named test exists and ran (zero-test success is not a pass).

### MOB-00 — Deterministic browser acceptance harness for mobile views

- **Parent:** #9201. **Requirements:** C-7, O §13. **Goal:** add browser-level evidence for responsive/mobile and accessible interactions without changing production semantics.
- **Existing substrate:** `pm-stack/aggregator` has Bun unit tests and `.github/workflows/pm-aggregator-tests.yml`; package currently has no Playwright config/dependency/browser-test CI target. This slice extends those, not a new service/workflow/binary.
- **Owned files:** `pm-stack/aggregator/package.json`, `bun.lock`, new `playwright.config.ts`, new `test/browser/mobile-work-view.spec.ts`, and the existing `.github/workflows/pm-aggregator-tests.yml` test job. Pin the exact `@playwright/test` package version in package manifest and lockfile, pin/browser-cache matching browser revision in CI, and make browser install deterministic. No dependency installation or browser run is part of this design pass.
- **Failing test to start:** `pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract` in `test/browser/mobile-work-view.spec.ts`; it begins red on the current view by asserting primary 360px queue/detail flow, keyboard reachability, and accessible action/status announcement.
- **Done/oracle:** isolated deterministic fixture data only (network blocked except local test server; API fixtures or existing fixture-source mode), no GitHub/Project/session credentials. Exact command: `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. `bun test` contract/read-model fixtures remain separate and fast. CI runs this job in the existing aggregator workflow with pinned dependency/browser cache; report trace/video on failure, not screenshot-only pass. Verify the named test is listed and actually executes; zero-test success is not acceptance.
- **Edges:** MOB-03/MOB-04 browser acceptance hard-blocked on MOB-00; their pure model/render unit tests and design may proceed. No new project-wide browser service or binary.

### MOB-01 — Versioned canonical work read model and provenance

- **Parent:** #9201. **Requirements:** H-SURVIVE-A, C-3/C-6/C-7, O-PART1 §18; V2 §3.1. **Goal:** project per-fact authority/revision/freshness and queue/Project/GitHub/session state without a new DB.
- **Owned files:** `pm-stack/aggregator/src/model.ts`, new `pm-stack/aggregator/src/work-projection.ts`, corresponding `pm-stack/aggregator/test/work-projection.test.ts`; read integration only where existing source APIs already expose the fields.
- **Enables:** shared cards/details/views and receipt previews; no mutation.
- **Failing test to start:** add a model/golden fixture with same issue open, PR merged, queue blocked, stale Project feed and bound session; assert every facet survives and status projection reports conflict rather than `Done`/`Ready`.
- **Named red test:** `work_projection_preserves_source_state_under_conflicting_facets` in `pm-stack/aggregator/test/work-projection.test.ts`.
- **Done/oracle:** deterministic model-based matrix over independent GitHub/Project/queue/session states; output equals golden `pm-work-view/v1` fixtures, each fact has a source/revision/freshness; unknown remains absent; stale/errors are not zero. Command: `cd pm-stack/aggregator && bun test test/work-projection.test.ts` (`package.json` declares `test: bun test`).
- **Edges:** soft dependency on #9070/#9062 and read contract; consume #9073–#9076. No schema migration; hard boundary that Project stays single-writer.

### MOB-02 — Lifecycle projection, column precedence, and truthful claim binding

- **Parent:** #9201. **Requirements:** C-6, C-7, H-SURVIVE-A/H-SCENARIOS, O §12.2–12.4. **Goal:** map lifecycle facets to board columns with precedence blocked/parked > eligible Ready > Backlog and require verified task/session evidence for In Progress.
- **Owned files:** `pm-stack/aggregator/src/work-projection.ts`, new `src/work-lifecycle-policy.ts`, new `test/work-lifecycle-policy.test.ts`, relevant docs in component design.
- **Failing test:** falsify that `work/assigned`, priority label, Project Todo, or closed task alone produces In Progress/Done; verify active verified binding creates In Progress and preserves source facets.
- **Named red test:** `work_lifecycle_never_infers_done_from_one_facet` in `pm-stack/aggregator/test/work-lifecycle-policy.test.ts`.
- **Done/oracle:** table-driven model test covers every pairwise conflict: hard-blocked+High; parked+owner; Ready+stale; claimed+no session; session shell+unbound task; PR merged+issue open; task done+issue follow-up open. Exact expected column and explanation in golden oracle; no terminal state fabricated. Command: `cd pm-stack/aggregator && bun test test/work-lifecycle-policy.test.ts`.
- **Edges:** depends MOB-01 (hard); contract dependency on #9178/#9179 and #9200; consumes #9070.

### MOB-03 — Responsive mobile queue, item detail, and saved views

- **Parent:** #9201. **Requirements:** H-SURVIVE-A, C-7, O §§12–13, all 15 mobile requirements read-only subset. **Goal:** expose all/triage/ready/high/critical-path/blocking/my-work/active-agent/review/blocked-parked/needs-triage/recent/cold backlog/by repo/project/owner views from one read model.
- **Owned files:** `pm-stack/aggregator/src/render.ts`, `src/ui.css`, proposed `src/work-views.ts`, `test/work-mobile-views.test.ts`, browser fixture and view documentation; avoid action-store semantics.
- **Failing test:** page at 360px and 200% zoom must have no horizontal overflow for queue and detail flows; keyboard can reach filter, item, back, and capability panel.
- **Named red test:** `work_mobile_views_match_reference_filter` in `pm-stack/aggregator/test/work-mobile-views.test.ts`.
- **Done/oracle:** Bun reference-filter unit test `cd pm-stack/aggregator && bun test test/work-mobile-views.test.ts` plus browser assertions in `test/browser/mobile-work-view.spec.ts`, exact command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04`. Browser oracle checks rendered refs vs pure view filter at 360/768/1280 CSS px and 200% zoom; no screenshot-only pass.
- **Edges:** hard MOB-00 for browser acceptance, MOB-01/02 for projection behavior; soft #9070 genealogy and existing views store #1276; no new view database.

### MOB-04 — Accessible compact responsive interaction system

- **Parent:** #9201. **Requirements:** C-7 / O §13.1. **Goal:** shared accessible card, filter, dialog, status and result components.
- **Owned files:** `pm-stack/aggregator/src/ui.css`, `src/render.ts`, new accessibility tests and `pm-stack/aggregator/docs/mobile-accessibility.md`.
- **Failing test:** keyboard-only and viewport fixtures expose a non-labeled button/insufficient target or overflow; establish checks for focus order, live result announcement, contrast tokens, 24px minimum target/spacing exceptions, 44px preferred primary controls, and no gesture-only affordance.
- **Named red test:** `mobile_primary_controls_are_keyboard_and_target_sized` in `pm-stack/aggregator/test/mobile-accessibility.test.ts`.
- **Done/oracle:** accessibility DOM assertions and measured CSS geometry at 360/768/1280 CSS px and 200% zoom, keyboard flow, plus manual review checklist; no critical/serious automated violations and documented exceptions accounted for. Target is WCAG 2.2 AA, not a blanket compliance claim. Commands: `cd pm-stack/aggregator && bun test test/mobile-accessibility.test.ts`; browser command `cd pm-stack/aggregator && bunx playwright test --config playwright.config.ts --grep pmo_mob_03/04` using MOB-00's pinned harness.
- **Edges:** MOB-00 hard prerequisite for browser acceptance; can proceed with MOB-03 prototype/unit tests, then gates production controls. No policy dependency.

### MOB-05 — Preview/apply action envelope through shared Fleet service

- **Parent:** #9201. **Requirements:** C-6/C-7, H-SURVIVE-A, O §§12–13; V2 §3.2. **Goal:** turn existing dashboard action request pattern into operation-key + expected-revision preview/apply/result contract, with server-side authorization and no client-side mutations.
- **Owned files:** existing `pm-stack/aggregator/src/actions.ts`, `src/server.ts`, new `pm-stack/aggregator/src/work-action-contract.ts` and `test/work-action-contract.test.ts`; common Fleet `pkgs/fleet/crates/fleet-queue/src/work/contracts.rs` and `operations.rs` seam under #8581 ownership. Do not add an independent router or `fleet-gh` crate.
- **Failing test / oracle:** `work_action_stale_version_conflicts_without_effect` in `pm-stack/aggregator/test/work-action-contract.test.ts`; stale work version or actor-role revocation between preview/apply must fail before any consumer effect, while same operation key returns the original receipt.
- **Done/oracle:** model-based state-transition test: draft→previewed→accepted→running→pending_sync/partial/conflict/uncertain/rejected→verified_complete; expected-work-version oracle compares against authoritative fixture; fake downstream consumes only validated operations and count asserts one effect per idempotency key. Existing action store remains a request ledger, not authority for the applied fact. Command: `cd pm-stack/aggregator && bun test test/actions.test.ts test/work-action-contract.test.ts`.
- **Edges:** hard MOB-01 and authority contract from #9180/#9200; #8581 Fleet verb contract and `pkgs/fleet/crates/fleet/src/gh.rs` (existing binary module; no `fleet-gh` crate) are soft caller seams. #8575/#8579 authority audit must be reconciled before assigning persistent fields. `fleet-queue/src/work/contracts.rs`, `operations.rs`, `authority.rs` and adjacent modules in `architecture.md` are proposed internal modules, not V1. Any `fleet/src/main.rs`, `fleet-registry` or shared route registration is one later integration slice after domain contracts settle, not parallel edits by each MOB slice.

### MOB-06 — Safe status, note, comment, and priority controls

- **Parent:** #9201. **Requirements:** H-SURVIVE-A, C-5/C-6/C-7, O §13.2, S-STATE/S-LABELS. **Goal:** enable items 4–8 of the mobile inventory through MOB-05, with private note default, explicit public mirror, #9200 score mapping, and blocked/parked protection.
- **Owned files:** `pm-stack/aggregator/src/actions.ts`, `src/server.ts`, `src/work-action-contract.ts`, status/priority policy adapter, UI action cards and operator docs; Fleet equivalents in `pkgs/fleet/crates/fleet-queue/src/work/authority.rs`, `operations.rs` and `priority.rs` only under their respective contracts. Do not edit the Project writer or duplicate #9200.
- **Failing test:** conflicting revision, unsupported Project Status, noncanonical/contradictory priority labels, blocked item `do-now`, private note accidentally sent public, and status request trying to bypass hard blocker.
- **Named red test:** `mobile_private_note_is_not_public_by_default` in `pm-stack/aggregator/test/mobile-work-actions.test.ts`.
- **Done/oracle:** every verb yields exactly one policy result and expected source-of-truth change in fake orchestrator/Fleet owners; cross-surface golden corpus verifies no sync loop and priority always re-runs #9200; public side-effect test asserts no comment on private default. A schema-dependent durable note is a follow-on gate, not a reason to add parallel storage here.
- **Edges:** hard MOB-05, #9200 classification/mapping, #9178/#9179; only nonpersistent preview/read UX proceeds before those contracts settle.

### MOB-07 — Session capability panel, live directions, status report and agent request

- **Parent:** #9201. **Requirements:** H-SURVIVE-A/C-7/C-8, O §§13.2–14.5. **Goal:** item operations 9–13, with adapter capability discovery and proof separated from request state.
- **Owned files:** `pm-stack/aggregator/src/render.ts`, proposed `src/session-control-view.ts` and `src/work-action-contract.ts` consumer view, `test/session-capabilities.test.ts`, `test/session-action-receipts.test.ts`, adapter capability docs. No direct session network calls from the browser.
- **Failing test:** unsupported provider/action is shown disabled with reason; Slack delivery never maps to acknowledged; session without matching task binding is not targetable as current owner; two clicks produce one op key.
- **Named red test:** `session_action_requires_verified_task_binding` in `pm-stack/aggregator/test/session-action-receipts.test.ts`.
- **Done/oracle:** per-adapter capability matrix has contract tests; state-machine test distinguishes accepted by Fleet, queued to adapter, delivered, acknowledged, agent meaningful progress and verified task bind; deterministic fake adapter receipts are the oracle. Reuse #6533 as the separate live acceptance fixture; do not create a duplicate live-proof issue. Command: `cd pm-stack/aggregator && bun test test/session-capabilities.test.ts test/session-action-receipts.test.ts`.
- **Edges:** hard MOB-05; consume #9075 session identity and #9203 dispatch route; no duplicate provider integration.

### MOB-08 — Dependencies, activity, outcomes and explicit view receipt

- **Parent:** #9201. **Requirements:** C-6/C-7, H-SURVIVE-A/H-SCENARIOS, O §12.5–12.6. **Goal:** operations 14–15, blocker graph and recently changed/outcome views with source cursors.
- **Owned files:** `pm-stack/aggregator/src/work-projection.ts`, proposed `src/work-activity-view.ts`, `test/work-dependencies-activity.test.ts`, and activity/view docs. No schema writes.
- **Failing test:** parent edge rendered as blocker, or stale/out-of-order event moves a cursor past unprocessed activity, or resolved task marked as parent issue done.
- **Named red test:** `work_relations_keep_parent_separate_from_hard_blocker` in `pm-stack/aggregator/test/work-dependencies-activity.test.ts`.
- **Done/oracle:** reference graph oracle preserves typed native parent vs blockedBy edges and hard/advisory relation; per-source cursor simulation with duplicate/late events yields deterministic visible activity; test verifies task-keyed completion vs issue closure distinction from #7016/#8579. Command: `cd pm-stack/aggregator && bun test test/work-dependencies-activity.test.ts`.
- **Edges:** hard MOB-01, consumes #9070 and #9208/#9207; persistent new cursor/history semantics gate on #8581 and QTI reconciliation.

### MOB-09 — Desired/observed reconciliation, freshness and rollback

- **Parent:** #9201. **Requirements:** H-DISTRIBUTED/H-SCENARIOS, C-6/C-16/C-17, O §18–20. **Goal:** readback/drift receipts, stale/partial read behavior, operational rollback controls.
- **Owned files:** `pm-stack/aggregator/src/work-projection.ts`, proposed `src/work-operation-reconciliation.ts`, `test/work-operation-reconciliation.test.ts`, freshness display in `src/render.ts`, and rollback/runbook docs. No new durable store.
- **Failing test:** fake Project writer accepts request but remote readback remains old; UI presently must not call it verified. Recovery test disables mutations yet leaves pending intent inspectable/replayable.
- **Named red test:** `work_verified_effect_requires_source_readback` in `pm-stack/aggregator/test/work-operation-reconciliation.test.ts`.
- **Done/oracle:** fault-injected desired→pending→conflict/verified states; second independent read confirms every “verified” fixture. Shadow mode compares live projection to legacy board with no writes; rollback config disables new mutation adapter and retains receipts/action intents for later review.
- **Edges:** hard MOB-05/06; Project writer and #8580/#8581 integration; live Project proof is separately recorded, mocks never equal deployment.

### MOB-10 — Hosted PM/Fleet projection parity

- **Parent:** #9201, after V2 contract accepted. **Trace:** program-level V4 hosted parity and adjacent hosted-edition work #9079/#9081. **Goal:** implement the same read/action schemas in those hosted editions with no semantic drift. This is an additive compatibility requirement; do not attribute it as a literal recovered-original clause unless the statement inventory finds that source evidence.
- **Owned files:** only the existing hosted service/TS tool modules assigned under #9079/#9081; contract corpus/version fixtures under existing shared test path.
- **Failing test / oracle:** `hosted_mobile_contract_matches_shared_golden` in the common conformance fixture suite returns different column, permissions, or operation state on laptop and hosted implementations.
- **Done/oracle:** same cross-target golden corpus and conformance suite; explicit unsupported capability reports; migration/rollback notes per edition. Existing aggregator corpus command: `cd pm-stack/aggregator && bun run contract:check`; the analogous hosted-suite command must be recorded from #9079/#9081's owning package before implementation. Do not claim deployment until hosted smoke + readback evidence exists.
- **Edges:** hard settled V2, MOB-01/05, #9079/#9081; no duplicate hosted service.

## 5. Rollout, migration and reversal

1. Read-only projection first: compare the new versioned work projection in shadow mode to existing board/queue views and record gaps; no mutations.
2. Enable action preview but not apply; exercise authorization, expected-revision conflicts, stale/outage behavior and user-visible receipt states.
3. Pilot low-risk notes/status-request action family via existing validated consumers. Enable separately gated status/priority actions only after #9200 and writer readback tests; dispatch/session controls only after #9203 and #6533's proof plan.
4. Turn on controls with per-verb kill switches and an operator-owned flag; rollback disables new apply calls, leaves action receipts/drafts visible and returns board to read-only legacy projection. Never delete pending requests or revert already-applied Project/GitHub effects; reconcile them from their owners.
5. No migration adds Project or PM database fields. If an existing source lacks a revision, use its strongest available ETag/update timestamp plus stable object identity and mark revision quality; no false atomicity claim. Any schema change goes to #9199/#9200/#9207/#9208 or its explicitly owned issue after #3424/#8579/QTI review.

Revisit if Project's authority changes, #8581 adopts a different Fleet operation boundary, #3424/#8579 changes the durable home of action intent/notes, a live feed is not available at the required freshness, or provider APIs cannot support receipts/capability discovery. Keep all final design decisions as **proposed FDR candidates** reconciled through #9180; unprefixed FDR-0001…0010 are reserved there. Suggested namespace `FDR-PMO-001-*` is not adopted without the shared index decision.
