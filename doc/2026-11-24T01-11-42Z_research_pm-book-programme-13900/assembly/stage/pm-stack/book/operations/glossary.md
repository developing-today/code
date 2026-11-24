# Glossary

The PM vocabulary, defined as the surveys and the code use the terms. Where a term has two live uses, the entry gives both and names the surface for each.

**Action (dashboard action).** A structured instruction posted to `POST /api/actions` on the aggregator and consumed by a polling consumer. Kinds: `triage`, `status-request`, `session`, `preference`. The consumer reports the outcome through `POST /api/actions/result` with outcome `applied` or `rejected`. The console's counterpart is its own audited action table behind `POST /console/api/act`.

**Aggregator.** The dashboard service in `pm-stack/aggregator/`: the one screen that joins the stack's sources into pages and API twins. Not a hierarchy level and not a work-item type (D10).

**Alert throat.** The aggregator's incident intake: `POST /api/alerts`, guarded by a bearer intake token resolved by secret name, backed by the alerts store in `aggregator/src/alerts.ts`. External producers and the health hooks post incidents through it. Extensions on the intake carry decision sets and stack-health reports.

**Answer-back.** The flow that carries an owner's answer from a phone or page back into the store that raised the question: a resolution in the decisions store, a comment or label change on a GitHub item, an action result, or a roadmap annotation. The surfaces are enumerated as question sets F1–F8 in the survey behind this book.

**Area.** A single-select field on Project #4 items, read verbatim by the aggregator. Programs surface in live data as Area values and as registry projects.

**Blocked-age.** The notification class that reminds on a needs-Drew item left unacted: first at 24 hours, a breakthrough at 48 hours, then weekly in digest form.

**Breakthrough.** A notification property: the delivery bypasses quiet hours and the digest queue. `incident` is a breakthrough class; a failed section in `section-risk` is a breakthrough; the 48-hour point in `blocked-age` is a breakthrough.

**Candidate (notification candidate).** A proposed notification built by `buildNotificationCandidates` in `aggregator/src/notifications.ts` from attention, quota, and health snapshots, carrying a class, an object ref, and a state fingerprint. The alert engine filters candidates per subscription before any send.

**Census (needs-Drew census).** The aggregator's open-only count of items waiting on the owner: open issues and pull requests labelled `user/review` or `user/escalated` across the fleet repos, deduped, with `hands:v1` markers read. The census is open-only by design; a closed item, such as #1007 in the design record, never appears in it (D13).

**Claimant.** The OpenCode execution lane in `pm-stack/opencode-claimant/`. Its fire sequence supervises stuck bound sessions, gates candidates across four queues, applies the concurrency cap, then launches, binds, and prompts a session. A claimant root is one slot under that cap; the cap stands at 36 roots from 2026-11-05.

**Closure gate (epic closure gate).** The check in `aggregator/src/epics.ts` that holds an epic open while its complete subtree has open children or unmet conditions, enumerating blockers by condition.

**Console.** The owner app in `pm-stack/console/`: a Bun server on port 4872, a static single-page app under `/console`, a Cloudflare edge Worker, and an Android TWA. Its views are hash routes; its writes go through the audited action table.

**Decision set.** A group of questions mirrored from the Muse-side decisions ledger, answered on its own page at `/decisions/&lt;setId&gt;` or in the console's questions view. A set has items; an item has questions; a question is answered, deferred, or returned with a request for information. First resolution wins per question.

**Digest (notification digest).** The queued delivery form for non-breakthrough candidates: candidates wait in a digest queue and deliver as one push. A state that disappears, or a class the subscription mutes, cancels the pending entry.

**Dispatch.** The intake service in `pm-stack/dispatch/` on port 4879: authenticated launch-or-enqueue of a work session, creating the tracking issue, the queue task, and the OpenCode session in one call.

**Epic.** A Project #4 item with Kind `Epic`, whose children join through native GitHub sub-issue parents. The `epic` label is the maintained mirror of the field; `meta/epic` is retired with no consumer.

**Hands item.** A census item carrying a `hands:v1` marker: an action only the owner performs, of kind `physical`, `interactive-login`, `payment`, or `signature`. The console's hands segment holds these items, and the hands-done action closes one out.

**Health hook.** A failure-triggered, read-only troubleshoot-and-report script in `pm-stack/health-hooks/`, fired once on a 3-consecutive-failure transition and once on recovery, under one shared 60-second deadline. A hook ships no remediation.

**Horizon.** A time-bucket field with three live vocabularies (D11): `Now | Next | Later | Ideas | Ongoing` on roadmap rows; `today | tomorrow | next-3-days | longer | incoming` on aggregator tier items; and Project #4's own single-select, read verbatim.

**Invariant sweeper.** `pm-stack/invariant-sweeper.py`: the slow-cadence sweep for closed-with-open-children issues, open issues carrying `status:done`, and Kaneo↔GitHub mismatches. Report-only by default; `--act` repairs the first two classes.

**Kaneo sync guard.** `pm-stack/kaneo-sync-guard.py`: the script that asserts and heals the Kaneo GitHub integration's status transitions, hourly and at deploy time.

**Kind.** The Project #4 single-select with exactly two values in PM code: `Task` and `Epic` (`aggregator/src/model.ts`).

**Ledger (decisions ledger).** The Muse-side record of raised questions that decision sets mirror. Resolutions recorded in the aggregator are applied back to the ledger per question by the dropped-thread catcher. Not to be confused with the coverage ledger of the book programme, or with the Turso store named `voice-line-ledger`.

**Needs-Drew / needs-you tier.** The attention tier of items waiting on the owner, built by the census in `aggregator/src/needs-drew.ts` and split into hands, review, and agent-decides sets by `needsDrewWait`. The console renames the buckets for the phone: Questions, Only you can do, Reviews, Escalated (D13).

**Notify relay.** The Cloudflare Worker in `pm-stack/notify-relay/` (`pm-notify`): a dead-man's switch that sends one `stack.silent` push after laptop silence, built at step 1 of its plan and not deployed (D5).

**Orchestration pass.** The consumer pass that applies dashboard actions and performs Project field writes as the Project's single writer, reporting outcomes back through the actions result path. Its shared-store writes follow the one-transaction-per-pass shape in [The shared schema as a library](../library/shared-schema-package.md).

**Program.** One of the nine programmes P1–P9 defined in `pm-stack/design-reprioritization-2026-11-12.md`, each with a queue band. A document scheme, not a schema type; in live data a program surfaces as a registry project and as an Area value.

**Projection.** A derived record built from source systems without becoming a source of truth. The projector's links and subject activity are projections; the aggregator's pages render from projections of its sources. See [Projector internals](../library/projector-internals.md).

**Prompt.** In the PM surface, an owner instruction submitted through `/chat` on the aggregator or `#ask` in the console, and the notification class (`prompt`, #10035) that reports when such a submission is answered or fails.

**Question set.** One of the eight surveyed question and answer-back surfaces, F1–F8: decision sets, needs-Drew single questions, the console census sets, prompt and chat questions, dashboard comments, dashboard actions, console actions, and the smaller report, roadmap, and alert answer-backs.

**Queue band.** A numeric priority band on fleet queue tasks. Bands run 10, 20, 30, 40, 50, 70, 90. Owner prompts from the console take band 10. Programs map to bands in the 2026-11-12 reprioritisation scheme.

**Rank label.** An issue label from the console's rank set — `expedite`, `priority/critical`, `now`, `priority/high`, `next` — that orders the console's ranked-issues view. `Ops.setRank()` adds the new label before removing the others, so a failed change never leaves an issue rankless.

**Section risk.** The notification class from #8882: a whole route section (`workhorse`, `speed`, `specialist`, `summit`, `small`) fails or is at severe risk, reported with the pools pending wall, the walled pools, and the soonest related resets. A failed section is a breakthrough; severe risk is not.

**Sentinel (stack sentinel).** `aggregator/scripts/stack-sentinel.ts`: the watcher that fires health hooks on failure and recovery transitions. Its state is authoritative over health-derived notification candidates when both exist.

**Session.** A recorded unit of agent work in the session registry (`/sessions`, twin `/api/registry.json`): who ran what, with token counts and task links. Dispatch and the claimant create sessions; the registry is the record a reader follows an item's work through.

**Shared schema.** The contract layer in `pm-stack/shared-schema/`: migrations, guards, and the orchestration writer for the shared Turso stores, indexed by `pm-stack/SHARED-SCHEMA-MAP.md`.

**Stack token.** The token exchanged through `POST /api/stack-token/exchange` for service-to-service calls against stack targets (`kaneo`, `pad`, `grist`). The break-glass path beside it lives in `aggregator/src/break-glass.ts`.

**Subject.** The join key for an item across systems: `gh:…`, `kaneo:&lt;slug&gt;/&lt;taskId&gt;`, `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;`, `session:…`, `orch:…`. Item and discussion pages address items by subject.

**Tier item.** An aggregator item placed in an attention or work tier, carrying the aggregator's own Horizon vocabulary and the actions a reader takes on it.

**Weight (effort weight).** The progress unit for epic rollups: `effort/xs` 1, `s` 4, `m` 12, `l` 40 as a floor. Unsized items contribute no weight and are counted visibly as unsized.

Sources. survey/website-pages.md §A–§F; survey/apps.md §1, §7–§8, §12, §19, §25, §31–§32; survey/roadmap.md §1–§3; survey/push.md §2–§3; survey/disagreements.md D5, D10, D11, D13. Ground truth: laptop clone at 4d937b3, 2026-11-23.
