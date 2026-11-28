# Prioritisation: how order is decided and recorded

## The rule

Priority is recorded in four places. The four records do not agree automatically. The `/conflicts` page exists in part because of that fact: it lists every live source disagreement with both values and the disagreement's age. No single field in this stack is "the priority". Each record below governs the surface that reads it.

## 1. Fleet queue priority: execution order

Queue tasks carry a numeric `priority`. The claimant and the console board take tasks in priority-then-age order: `ORDER BY (priority IS NULL), priority ASC, created_at ASC` (`console/src/work.ts` `board()`).

The band scale is `10 / 20 / 30 / 40 / 50 / 70 / 90`. The 2026-11-12 reprioritisation scheme (`pm-stack/design-reprioritization-2026-11-12.md`) maps programs to bands: P1–P3 at band 20, P4–P6 at band 30, P7–P9 at band 40. The scheme document records the re-banding as applied on 2026-11-12 by the queue lane. An owner prompt submitted from the console takes the top band, 10 (`promptPacket` in `console/src/work.ts`).

The console writes this scale directly: `PRIORITY_SCALE = [10, 20, 30, 40, 50, 70, 90]` in `console/src/ops.ts`, and `Ops.setTaskPriority` writes a value from that scale onto a queue task.

## 2. Issue rank labels: the ranked-issues view

The console's ranked-issues view orders open issues by rank label. The ranks are `RANKS = [expedite, priority/critical, now, priority/high, next]`, with `RANK_PRIORITY` mapping each rank to a number (`console/src/ops.ts`). `Ops.priorities()` searches open issues carrying any rank label and sorts by rank, then by issue number.

`Ops.setRank()` writes in a fixed order: it adds the new rank label first, then removes the other rank labels. A failure mid-write never leaves an issue rankless.

Intake assigns ranks by kind. The route table in `console/src/plan.ts` (`ROUTES`) gives each intake kind its labels and its rank. A note, bug, or change submitted through console intake files a GitHub issue with the route's labels and rank, and enqueues a queue packet in the same action: `triage.enqueue` for a new item, `prompt.enqueue` once an issue exists. New work enters the portfolio record and the execution queue together, with the rank from the route table.

## 3. Project fields: portfolio intent

Project #4 carries Horizon, Status, and Area per item, and effort weights carry progress (see [The roadmap hierarchy: initiatives, projects, epics, aggregators](roadmap-hierarchy.md)). These fields record intent. They do not set execution order; the queue bands in §1 do.

The Project has a single writer: the orchestrator. The standing rule in `pm-stack/README.md` states it exactly: "Agents and the dashboard do not edit Project fields directly; the aggregator reads the pool through the fleet feed." A dashboard status change is therefore a request, not a write. The dashboard records a `status-request` action; the orchestration pass consumes it, performs the write, and reports back through `POST /api/actions/result`. The item's next published tier entry carries the acknowledgement.

The console is the exception path, and the exception is narrow (D4). The console writes labels and comments through `gh` and queue actions through the queue broker: rank changes (`setRank`), issue comments, queue verbs, and the hands-done label removal. Every console action is audit-logged before execution. The aggregator service itself never writes labels back; its header in `aggregator/src/needs-drew.ts` states that the aggregator never writes labels and names the orchestrator as label writer. Both statements are true at different scopes: the aggregator service writes no labels; the PM stack does, through the console (D4). Project fields and issue labels are different records, and this book keeps them separate.

## 4. Dashboard actions: recorded overrides

`POST /api/actions` carries two priority-relevant action kinds:

- `triage` answers: acknowledge, snooze, escalate, de-escalate, answer, delegate, dismiss. Escalate and de-escalate are tier overrides, and the reason is recorded with the override. Snooze suppresses an item until a named time and renders on the item as "snoozed by you".
- `status-request`: the request path from §3.

Consumers poll `GET /api/actions.json?state=new` and return outcomes through `POST /api/actions/result` with outcome `applied` or `rejected`, the applying run, and a note.

## 5. The decision ledger

Decisions are recorded in a ledger, separate from all four records above. `pm-stack/decisions-ledger-system.md` (2026-11-11) defines the ledger that decision sets mirror. A decision set groups ledger items; an answer, defer, or request-info on a set records a resolution in the decisions store, and the dropped-thread catcher applies resolutions to the Muse-side decisions ledger per question. A consequence that changes Project state does not write through the dashboard; it becomes a `status-request` action under §4. The full answer flow is in [Question sets and the answer-back flow](questions-and-answers.md).

## 6. Quota-side proposals

The `/quota` surface carries a fifth, adjacent mechanism: pool and route guidance, forecasts, and reweight proposals in `aggregator/src/quota-guidance*.ts`, `quota-forecast*.ts`, and `quota-reweight*.ts`, with `scripts/quota-reweight-proposals.ts` and `scripts/quota-weekly-review.ts`. These are proposals recorded as records and pull requests. They do not reprioritise silently. Detail beyond this pointer is UNVERIFIED in the survey this chapter rests on; the surfaces themselves are described in [Health, quota and usage surfaces](health-surfaces.md), and quota figures live in the LLM-plans chapters, not here.

## Design record

Four design documents frame this chapter: `design-register-2026-11-11.md` (the register of PM designs), `design-tracking-manifest-2026-11-11.md`, `lineage-manifest-2026-11-12.md` (the lineage each piece descends from), and `design-reprioritization-2026-11-12.md` (the band scheme in §1). Where a design document and the code disagree, this book follows the code and cites the disagreement by number.

Sources. Survey `roadmap.md` §§3–4; survey `disagreements.md` D4; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
