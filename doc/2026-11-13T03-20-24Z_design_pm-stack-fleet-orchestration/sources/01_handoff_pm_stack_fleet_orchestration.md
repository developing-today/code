# 02 — Muse Handoff: PM Stack & Fleet Orchestration

> Source: Google Doc `1zPCTo_a4p5qiVts2HEFL0WXKeh9_KiS7oQ3mTprKYEQ` — https://docs.google.com/document/d/1zPCTo_a4p5qiVts2HEFL0WXKeh9_KiS7oQ3mTprKYEQ/edit
> Drive dates: created 2026-11-13T01:24:39Z, modified 2026-11-13T01:28:41Z. Exported (plain text via Google Docs API) 2026-11-13T03:16Z for the PM Stack & Fleet orchestration design-mill intake.
> Role: GOVERNING INTAKE DOCUMENT for this design mill (Muse handoff, 2026-11-12).

---

Muse handoff: Fleet and PM Stack orchestration design buildout

Preservation comes first. Treat recent repo work as integration evidence, not a replacement for the recovered requirements. Run this as a design mill: preserve and map every requirement, break it into reviewed component designs, and turn each design into linked, testable implementation issues. Verify source access through the authorized identity or approved staging. Do not make private material public.

Muse, prepare and directly launch an OpenCode Summit pool assignment for the Fleet/PM Stack work below, then supervise it through verified completion. Treat the earlier attempted submission as cancelled and this design intake as pending, but do not assume overlapping repository work is unimplemented. Reuse existing issues, sessions, designs and commits wherever they already satisfy requirements. Do not launch a duplicate live session.

Read these sources completely

1. Governing integrated charter, Fleet & PM Stack — Priority, Work-Item Orchestration, and Direct Dispatch, v1.1 (6 November): https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit
2. Full supporting work-item orchestration and priority draft: https://docs.google.com/document/d/15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA/edit
3. Original conversation, Naming Fleet Agents: https://chatgpt.com/c/6aedd6ba-93dc-83ea-a1cd-78fe1b234f0a
4. Repository: https://github.com/developing-today/code

Supplemental raw note (review as design candidates, not a new overriding directive): https://docs.google.com/document/d/1MSdUt97SnWIzgqyglMJuSzTMq4RKtoThQq1a4JVVZvU/edit. The v1.1 charter already incorporates its main urgency/importance, bidirectional aging, clump smoothing, boundary resistance, dependency pressure, immediate dispatch, mobile control and three-item weighted exploration ideas. Read its full text for nuances and reconcile against current ratified policy; do not create a third independent implementation stream merely for this overlapping note.

This workstream is distinct from Fleet v0.3 Secrets/Cryptography/PKI. That workstream's coordinator handoff is https://docs.google.com/document/d/1AKDACXObRcUwmJIeCfLaGv6bxcWC00e3rYjOztk4qiY/edit and its secrets addendum is https://docs.google.com/document/d/1QkQMPYGEHbjdyMBdrcSdo4i2GKxlJYK2S3PDrvCRfVw/edit. Share interfaces and dependency links, but do not duplicate its provider, vault, credential-generation or CA deliverables here.

Intended result and immediate mission

Preserve the two coordinated design tracks within this charter: (A) urgency, triage, PM visibility, direct dispatch and mobile control; (B) work-item lifecycle, context hydration, typed dependencies, journal, outbox and direct verified completion. Build an evidence-backed design-of-designs and granular implementation backlog, then progress authorized executable stages through the established workflow. Do not confuse a completed design pass with a completed implementation epic. The first Summit task must review and reconcile the complete requirements, make architectural decisions, and produce implementation-ready component designs and issues, not attempt a monolithic production rewrite.

Reconcile current work before adding anything

Pin the inspected commit, read AGENTS.md and current Fleet/PM Stack/queue authority documents, and inspect code, tests, open PRs, issue comments and actual deployment evidence. In particular verify these entry points again at execution time:

- https://github.com/developing-today/code/issues/9060 and https://github.com/developing-today/code/pull/9064: design merged, component implementation remains open.
- https://github.com/developing-today/code/issues/9078 and https://github.com/developing-today/code/pull/9088: hosted prototype and component issues #9079–#9087. Epic closure must not cause unfinished components to be skipped as already_done.
- https://github.com/developing-today/code/issues/9091, https://github.com/developing-today/code/pull/9102 and https://github.com/developing-today/code/issues/9106: design-mill starter, parked wrong-worktree work with potentially salvageable drafts, and open iteration. Preserve useful drafts and inspect ownership before replacing or restarting.
- https://github.com/developing-today/code/issues/8187: inline triage and immediate bound handoff changes.
- https://github.com/developing-today/code/issues/6533: live launch-and-bind acceptance proof remains a separate requirement; a merged fix is not that proof.
- https://github.com/developing-today/code/issues/7030: priority brief was ratified. Do not ask for the same ratification again or trust stale issue-body status over dated evidence.

These are discovery anchors, not permission to overwrite recent decisions. Read the ongoing ID/Atlas and design-mill patterns and integrate with their existing component boundaries. Preserve current ascending urgency ordering. The original charter's 450/High and 1000/default score scheme is a proposal; map the intent into the current ratified taxonomy rather than silently renumbering the live queue. Keep intrinsic urgency, importance, effective scheduling pressure, state and immediate-dispatch directive distinct.

Produce a requirement crosswalk classifying each item as verified existing, partially implemented, open implementation, design-only, research, superseded with rationale, blocked, or unverified. Link real code/tests/issues. Identify requirements whose current ownership differs from the historical charter. Do not create another queue, scheduler, database, Fleet binary or secret resolver simply to implement this intake.

Muse preparation and launch

Export and preserve all source documents with links, dates and hashes; commit reviewed public-safe design text and source inventory into the repository using its current conventions. Do not commit private credentials or unrelated private chat material. Create/reuse a central design epic and a small initial domain issue tree. Each design should decompose into testable component designs and independently actionable implementation, validation, migration and documentation issues, with stable requirement IDs, native parent/blocker relationships and measurable acceptance.

Use the current sanctioned GitHub App/outbox and queue writer. Verify actual created issue IDs and queue records rather than counting pending intents. Discover the live Summit route and host; do not invent provider/model IDs or silently choose another pool. Create/bind the canonical queue task and immediately launch the authorized Summit OpenCode session with this prompt and staged sources. Confirm prompt delivery, session identity, ownership/lease and meaningful start. A label, queue position, session shell without a prompt or pending outbox record is not a launch receipt. Reuse a compatible active session if one exists; do not double-claim.

Requirements that must survive refinement

- Numeric urgency and separate importance, dependency pressure, deadline/opportunity boosts, bounded aging and de-escalation, stable ties, explainable triage, band hysteresis and manual overrides. Avoid priority inflation or cold-backlog disappearance.
- Authenticated labels, comments and slash commands, single-writer Project projection, distinct hierarchy vs hard dependencies, live-session instruction routing, direct dispatch for authorized parked work, durable receipt and idempotent webhook processing.
- Mobile-first PM board controls for status, search, notes, reprioritization, triage, dispatch and live-session direction, using shared validated Fleet APIs rather than independent frontend policy.
- Safe atomic claiming and fencing, work-item hydration into durable files, complete GitHub context/diffs, per-source activity cursors, session-origin attribution, private notes vs public comments, managed body sections, checkpoints and resumable parking.
- Transactional outbox with verified remote effects and safe retry; direct completion that verifies authorized merge/issue closure then closes the canonical queue item without waiting for a webhook, while retaining recovery for partial effects.
- Bounded exploration only when higher-urgency eligible work does not require the lane; configurable density-corrected sampling, suggested preference 1.25 per 200 urgency points and density exponent 0.5, statistical simulation and explanation. Preserve outcomes rather than mechanically copying arbitrary numeric choices.
- End-to-end scenarios: fresh work, blocked/parked resume, late external comment, near-timeout checkpoint, crash after remote mutation, duplicate webhook, stale lease, conflict and live launch/bind proof.

Supervision and completion

Reuse established durable supervision at a practical 5–15 minute cadence while active. Check logs, bound task/session, actual progress, questions, outbox receipts and terminal state. Answer questions from verified sources and user-approved authority; preserve genuine production/security approval gates. Recover or redirect stalls without racing a live owner or creating duplicate sessions. A “continue” message is not recovery evidence until the session acknowledges or resumes meaningful work. Record checkpoints and monitor ownership; do not promise monitoring that has not been installed and verified.

The design pass must deliver committed source inventory, architecture and component designs, ADRs, traceability, granular issue index, dependency-aware staged plan, acceptance and migration/rollback tests, current-state evidence, and explicit unresolved gaps. Keep implementation children open until their own tests and operational evidence pass. Report separately what was designed, implemented, merged, deployed, live-verified, blocked and still pending. Provide launch and final receipts with issue URLs, queue/task/session IDs, route, commit/PR, monitor record and first executable stage. Continue the agreed process through component buildout using those records; do not close the overall program merely because the first planning session completed.

Preservation and distributed priority

Retain the earlier full design text and handoff, including the issue/queue schema, priority policy, mobile interactions, command grammar, concrete subcommands, error/receipt semantics, and worked examples; do not reduce the implementation backlog to the shorter Muse brief. Capture each command and option in a compatibility/acceptance inventory. The original combined PM handoff is linked in this packet's discussion index.

Treat distribution of priority as its own design question, not just a local sorting formula. Trace every priority producer and consumer: human rulings and labels, comments and direct dispatch, work-item/issue metadata, typed dependencies, queue broker and claimant, PM projections and hosted implementations. Specify the authoritative policy and precedence, stable IDs and versioned payloads, provenance and reason codes, replay and reconciliation when views diverge, and how to validate the same ordering across targets. Keep urgent dispatch separate from stored priority and hard eligibility gates. Verify exact historical coverage in the source notes and record missing requirements as additive rather than silently rewriting the existing charter.

Recent repo work is contextual evidence. It may satisfy or constrain specific requirements, but is not the scope of either design and does not replace either original. For the design mill retain unimplemented subcommands and features as traced issues, even if they do not overlap current ID/Atlas work.

Original combined PM design, preserved in Drive: https://docs.google.com/document/d/1h8zF-uWTFuXQZ2KFeAPg-aNXR_S3BY9ebKtr3pc6Tg8/edit

PM/Fleet preservation checklist

Preservation-first rule: the original design and discussion are source requirements, not a list to shrink around today's code. Keep immutable copies and map every requirement to implementation/design/test/research/deferred-with-reason. Recently landed work is integration context and evidence, not permission to discard desired features. Preserve the original proposed mechanisms alongside any better replacement and explain the mapping.

Source anchors

- Original combined assistant design: https://chatgpt.com/c/6aedd6ba-93dc-83ea-a1cd-78fe1b234f0a?messageId=c038e0cb-6b4f-4094-993c-1dc5a1bc74c5 (full 82,793-character source recovered through search; saved locally as pm_original_combined_chat.md).
- User prioritization specification: https://chatgpt.com/c/6aedd6ba-93dc-83ea-a1cd-78fe1b234f0a?messageId=bbb21890-7fb9-46af-9d34-4f5e4d8ed8b4.
- Charter: https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit.
- Full supporting draft: https://docs.google.com/document/d/15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA/edit.

Priority distribution and scheduling detail that must not disappear

Original combined design Part1 §§2–17; charter §§3–9:

- Separate urgency, importance, effective scheduling pressure, workflow, dispatch directive and ownership. Preserve original provisional score bands/defaults and explain migration to any current numeric convention, rather than silently substituting it.
- Distribute work across eligible lanes by capability/readiness/dependency/ownership; ranking and sampling are independent. Atomic claim revalidates eligibility after selection. Direct dispatch bypasses waiting, not hard blockers or live leases.
- Stable total order with equal scores permitted, deterministic secondary rank, oldest-first and stable-ID ties. Explicit exact-score/band/before/after/relative edits remain available.
- Progressive clump refinement, gentle spacing, avoiding uncontrolled drift; lower-urgency-to-higher-urgency sweep order with safeguards against repeatedly moving the same item. Preserve user example50→25/10/5/singles as an intended shape, not a mandated algorithm.
- Bidirectional aging informed by actual value, pivots, opportunity windows and last meaningful triage; no automatic climb to Critical, no permanent cold-backlog purgatory. Temporary boosts expire to retriage, not silent deletion.
- Hard-dependency urgency inheritance, critical-path pressure, cycle safety, effort/alternatives and explanation. Do not conflate inherited urgency with intrinsic value or rewrite it blindly.
- Configurable low-confidence resistance/hysteresis at band boundaries, cooldown and high-confidence/manual bypass. Preserve reasons, confidence, before/after and evidence.
- Authorized GitHub priority-label edits normalize into canonical score; priority-label removal triggers triage without losing score. Separate needs-triage from executable work. Threshold/coalesced managed updates avoid comment floods while meaningful transitions remain visible.
- Exploration only when eligible Emergency/Supercritical/Critical/Do Now work does not require the lane. Suggested4% chance, three distinct candidates, all dispositions persisted before action. No three exclusive claims solely because sampled. Select/triage/defer/release each; interrupted intentions survive as bounded hints, not automatic unsupported score changes.
- Weighted selection: w_i=r^(-(u_i-u_ref)/d)×count(bucket_i)^(-alpha); defaults r1.25,d200,alpha0.5,bucket200, optional uniform-mixture epsilon0.05. Normalize over eligible items, sample without replacement, seeded simulations and --explain. Future urgency/importance axis mixture remains separate and configurable; never fabricate importance from urgency.
- Project/epic lifecycle and strategic urgency can change effective order; preserve manual intervention, limited opportunities and high-value escalation to the owner, not just arithmetic sorting.

Command/features inventory must remain explicit

Preserve every original namespace/operation in the source crosswalk, even if implementation names change:

- Discovery: help, capabilities, agent bootstrap queue-worker/assigned-item/resume/triage.
- Queue: list/search/inspect/rank/claim/release/park/resume/update/close; filters for urgency bounds, labels, assignee, readiness/blockers, created/updated time, type/repo/capability/effort; exploration/sampling and explanation.
- Work: next/open/inspect/hydrate/refresh/changes/checkpoint/note/block/unblock/park/resume/release/handoff/complete/status.
- GitHub: issues and PR metadata, comments/timeline/events, commit/diff/reviews/checks, create/update/merge/close; managed comment/body regions and explicit mirroring.
- Metadata: get/patch/priority/labels/title/summary; independent queue vs GitHub titles/bodies.
- Dependencies: add/remove/list/graph/ready/blockers; hard vs advisory, parent vs blocker, resolved history.
- Activity/sync: list/since/watch/reconcile/status/run/retry/verify/outbox; per-source cursors, overlap/dedup and reliable session exclusion.
- Agent control: dispatch/session status/send/resume; authenticated comments/slash commands, receipt/acknowledgment, near-timeout recovery.
- Shared flags/context: work/repo/issue/PR/agent/session/lease, since/start/checkpoint/external-only, output/format/json,dry-run,explain,expected-version,idempotency-key,verify,wait,timeout. Safe inferred defaults never ambiguous cross-repo mutations.

Keep complete hydration metadata/diffs/manifests, structured errors, journals/outbox, direct verified completion, all startup→claim→work→block→resume→external-change→merge→close walkthroughs, negative tests and failure cases. The new Muse launch prompt is a wrapper around these full preserved documents, not a replacement specification.

Recovered historical short prompt
Source: https://chatgpt.com/s/t_6af66737e9348191849d403726fcd631
Use the current Muse instructions above for dispatch. Retain “Queue Table Improvements” as an explicit discovery anchor: locate the original work by name and crosswalk it against current queue work before assuming equivalence.

Verbatim prompt from the original final reply:
Read and execute the complete Fleet & PM Stack Execution Charter: https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit

Inspect the current developing-today/code repository and existing Queue Table Improvements work. Carry out the entire coordinating workflow, create and commit the design artifacts, establish the high-priority GitHub issue and dependencies, and directly dispatch the designated design agent rather than waiting for ordinary queue selection. Follow the existing Fleet, GitHub Project, PM Stack, and outbox conventions. Exercise architectural judgment, resolve implementation details against current code, and delegate substantial work appropriately. Verify the actual agent session, task ownership, issue state, and durable handoff. Continue the specified downstream workflow through its completion mechanisms, preserving checkpoints and reporting verified results, blockers, and remaining work. Do not mistake a queued issue or dispatched request for completed execution.
