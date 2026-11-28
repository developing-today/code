# Project Management Space — Roadmap mechanics

> Pack file 3 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources: `survey/roadmap.md` (primary), `survey/website-pages.md` §F,
> `survey/apps.md` §1–§2/§25, `survey/disagreements.md` D4, D10–D13.
> Ground truth: laptop clone `~/git/code` at `4d937b3` (2026-11-23).
> Code wins over design docs. This file is explainable structured prose:
> a Space page or diagram must be buildable from it alone.

---

## 1. The hierarchy — the real ladder

There is **no single "hierarchy" type enum in code**. Four levels are carried
by four different systems, joined by refs.

| Level                                          | What it actually is in code                                                                                                                                                                                                                                                                                                                                                                                                                                               | Source of record                                                                                                                                                                             |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Programs** (the brief's "major initiatives") | **Programs P1–P9** — a _document_ scheme, not a schema: `pm-stack/design-reprioritization-2026-11-12.md` defines nine programs (P1 ID, P2 Atlas (rolling), P3 Fleet, P4 SaaS for ID/Atlas/fleet, P5 PM stack, P6 Turso+Cloudflare, P7 GitHub+dynamic compute, P8 Storage, P9 Lab), each with a queue band. In live data a program surfaces as a **project** in the aggregator's project registry / program page and as an **Area** value in GitHub Project #4.            | Design doc + `aggregator/src/program.ts`, `project-config.ts`, `projects.ts`                                                                                                                 |
| **Projects** (portfolio)                       | **One GitHub Project**: org Project "Developing Today" (#4), the portfolio map. Its items carry single-select fields **Status / Area / Horizon / Kind**, read verbatim by the aggregator into `ProjectPoolItem{status, area, horizon, kind}`. "Project" in the aggregator's own registry (`/program`, `/program/<id>`) is a second, repo-side notion: a tracked programme with its own status/roadmap derived from the registry + tracked issues + the Project pool.      | `aggregator/src/projects.ts`; `pm-stack/README.md` ("GitHub Project #4 conventions")                                                                                                         |
| **Epics**                                      | **Project items with `Kind = "Epic"` — canonical.** The aggregator reads this field and resolves `epicRef` through native sub-issue parents; it does **not** infer epic membership from labels. The `epic` _label_ is the maintained gate/mirror (every pool item with Kind=Epic carries it, no other pool item does); `meta/epic` is retired with no consumer. In code, `ProjectPoolItem.epicRef` is absent for epics themselves and names the parent epic for children. | `pm-stack/README.md`; `aggregator/src/projects.ts` (~line 311), `aggregator/src/model.ts`                                                                                                    |
| **Tasks / items** (below the epic)             | Project pool items with `Kind = "Task"`, GitHub issues/PRs (`GhItem`, kind `issue                                                                                                                                                                                                                                                                                                                                                                                         | pr`), Kaneo cards, and Grist/repo roadmap rows — all joined as refs (`owner/repo#N`) and registry subjects (`gh:…`, `kaneo:<slug>/<id>`, `grist:<doc>/Roadmap/<id>`, `session:…`, `orch:…`). | `aggregator/src/model.ts` and source adapters |

**Edges are kept distinct** (`pm-stack/README.md`): **hierarchy** = native
GitHub sub-issue parents (epic membership); **dependencies** = native
`blockedBy`/`blocking` relations (`aggregator/src/orchestration-dependency.ts`,
`epic-graph.ts`, `graph.ts`). Prose is a substitute for neither. The edge list
as data is in `priorities-and-maps.md`.

### D10 — the code-reality note (must appear in any Space rendering)

> **D10. "Aggregators" as a hierarchy level does not exist in code.**
> Programme framing lists "major initiatives, projects, epics, aggregators"
> as hierarchy types. In code, Kind on Project #4 is exactly
> **`Task | Epic`** (`aggregator/src/model.ts`), programs are a document
> scheme, and "aggregator" names a _service_ — the dashboard
> (`aggregator/src/program.ts`: "This dashboard is the aggregator of the PM
> stack") — not a node type. The other "aggregator" in the estate is Atlas's
> staging aggregator (embed-at-promotion) — unrelated to the PM hierarchy.
> The word "initiative" is UNVERIFIED as a code-level type; no `Initiative`
> type exists in `aggregator/src/model.ts`.
>
> **The real ladder is: program (doc/Area) → project (registry + Project #4)
> → epic (Kind=Epic + sub-issue parent) → task/item.** Do not present a
> four-level typed hierarchy with "aggregator" as the fourth level.

Writer rule from the reconciled skeleton: the roadmap chapter (and any Space
page derived from it) frames Drew's four types but states this code reality.

### Two horizon vocabularies + one Project field (D11)

No single "horizon" definition is correct:

| Surface                        | Vocabulary                                                                                                                  |
| ------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| Repo-file / Grist roadmap rows | `Now \| Next \| Later \| Ideas \| Ongoing` (`sources/repo-roadmap.ts` `HORIZON_BUCKETS`, validated by `roadmap-horizon.ts`) |
| Aggregator tier items          | `Horizon = today \| tomorrow \| next-3-days \| longer \| incoming` (`aggregator/src/model.ts`)                              |
| GitHub Project #4              | Its own free-standing `Horizon` single-select; values are whatever the Project defines, read verbatim in `projects.ts`      |

### Two roadmap implementations (D12)

- **Aggregator roadmap** (`/roadmap`, `/api/roadmap.json`): Grist + repo
  files + Project pool (merge rule below).
- **Console roadmap** (`console/src/plan.ts`): an independent implementation
  — phases `doing|now|next|later|built` derived from labels, `CAPABILITIES`
  list, `parseRoadmap` parsing a plan doc's markdown table. It is **not** a
  client of `/api/roadmap.json`. Docs that speak of "the roadmap" in the
  singular understate this.

---

## 2. How items roll up

### Epic rollup

- `aggregator/src/epics.ts`: `uniqueEpicItemsByRef` takes every pool item
  classified Epic; `buildEpicRollups` groups children by `epicRef` and counts
  done/total — a child is done per `isEpicChildDone` (state/status).
- Rollups feed `/projects` (now the pool section of `/program`) and the
  `/roadmap` drill-down.

### Complexity weighting — progress is weight, not counts

- `aggregator/src/weights.ts` (#1108): `effort/xs=1, s=4, m=12, l=40` (`l` is a
  floor, flagged open-ended).
- Unsized items contribute **no weight** and are counted visibly as unsized,
  never zero-weighted.
- Summaries also split open weight by `mode/*` (physical/hybrid = hands
  weight, online, unmoded).

### Epic closure gate

- `epics.ts` `evaluateEpicClosureGate` + `scripts/epic-closure-gate.ts`,
  monitored report-only by `fleet-monitor/epic_close_check.py`: an epic may
  not close while its complete subtree has open children / unmet conditions;
  blockers are enumerated by condition.
- The same invariant is enforced from the other side by the Kaneo safety
  nets: `invariant-sweeper.py` invariant (a) and `closure-watchdog.py`
  (auto-reopens automation closes with open sub-issues or no closing keyword).

### Reparenting

- `aggregator/src/epic-reparent.ts` + `scripts/epic-reparent.ts` move
  sub-issue parents; conformance is checked by
  `scripts/check-epic-conformance.ts`.

### Roadmap roll-up (`/roadmap`)

Rows from three sources merge:

1. Grist `Roadmap` table (`sources/live.ts` `gristRoadmap()`).
2. Repo files `planning/roadmaps/*.md` in developing-today/code + `ROADMAP.md`
   in dezren39/darwin-nix (`sources/repo-roadmap.ts`, #1738).
3. Project-pool / program data.

**Merge rule (#1738):** a title in both repo-file and Grist collapses to the
**repo-file row**; the Grist duplicate is noted in provenance and its links
unioned in. Rows carry Title/Status/Horizon/Area/Intent-or-Notes/Links.
View filtering is shared between the page and its twin
(`filterRoadmapForView`, #8723). Annotations attach via
`roadmap-annotations.ts` (see §4).

### Program roll-up (`/program`)

Per project: status distributions over Status/Area/Horizon/Kind
(`projects.ts` `distribution(...)`), epic rollups, tracked-issue state,
orchestration freshness. Stale/conflicting Project Status vs derived state is
**surfaced, not hidden** (canonical-status comparison, #1198/#1199 family);
`/conflicts` lists every live source disagreement.

### Console roll-up

`console/src/plan.ts`: `PHASES = doing|now|next|later|built` derived from
labels (`phaseOf`), `KINDS = note|bug|change`, `CAPABILITIES` with phases;
`parseRoadmap` parses the plan doc. A second, simpler ladder for the phone —
see D12 above.

---

## 3. How prioritisation is computed / recorded

Priority is recorded in **four places that do not automatically agree** (the
`/conflicts` page exists partly because of this), plus one adjacent quota
surface:

1. **Fleet queue priority (execution order)** — numeric bands
   `10/20/30/40/50/70/90` (`design-reprioritization-2026-11-12.md` band map;
   queue tasks carry `priority`, claimed in priority-then-age order —
   `console/src/work.ts` `board()`: `ORDER BY (priority IS NULL), priority
ASC, created_at ASC`). Programs map to bands in the 2026-11-12 scheme
   (P1–P3 band 20, P4–P6 band 30, P7–P9 band 40); the doc's "Queue re-banding
   — APPLIED (2026-11-12, queue lane)" section records the re-banding as
   done. Owner prompts from the console take the top band, **10**
   (`promptPacket`: "Owner prompt from the console: top band (10)").
2. **Console task priority scale** — `PRIORITY_SCALE = [10,20,30,40,50,70,90]`
   (`console/src/ops.ts`); `Ops.setTaskPriority` writes it on queue tasks.
3. **Issue rank labels (console ranked-issues view)** — `RANKS = [expedite,
priority/critical, now, priority/high, next]` with `RANK_PRIORITY` mapping
   to numbers (`console/src/ops.ts`); `Ops.priorities()` searches open issues
   carrying any rank label and sorts by rank then issue number;
   `Ops.setRank()` adds the new label _first_, then removes the others, so a
   failure never leaves an issue rankless. Intake routes assign ranks by kind
   (`console/src/plan.ts` `ROUTES`: per-kind labels + rank).
4. **Project fields (portfolio intent)** — Horizon + Status + Area on
   Project #4, and effort weights (§2) for progress. **The Project's single
   writer is the orchestrator** (`pm-stack/README.md`: "Agents and the
   dashboard do not edit Project fields directly; the aggregator reads the
   pool through the fleet feed") — a dashboard status change is a
   `status-request` action consumed by the orchestration pass, which performs
   the write itself (`aggregator/src/actions.ts` header).
5. **Quota-side prioritisation (adjacent)** — pool/route guidance, forecast
   and reweight proposals in `aggregator/src/quota-{guidance,forecast,reweight}*.ts`
   and `scripts/quota-reweight-proposals.ts` / `quota-weekly-review.ts` —
   proposals, recorded as records/PRs, not silent reprioritisation. Detail
   beyond this pointer: UNVERIFIED in the survey. (LLM-pool ordering rules —
   burn-by-expiry, free-first, OpenAI last — are in `llm-plans.md` and
   `priorities-and-maps.md`.)

**Design-doc frame:** `design-register-2026-11-11.md` (register of PM
designs), `design-tracking-manifest-2026-11-11.md`,
`lineage-manifest-2026-11-12.md` (design/issue lineage per piece — including,
at line 329, the open question "#1518 Define code/config embeddings…"),
`decisions-ledger-system.md` (2026-11-11: the ledger decision sets mirror),
`needs-user-actions-design-2026-10-26.md` (needs-user/attention design; live
vocabulary is the `needs-drew.ts` census + attention tiers — D13).

**D13 — vocabulary warning:** the 2026-10-26 needs-user design uses its own
terms; shipped code splits waiting items by `user/review` + `user/escalated`
labels and `hands:v1` markers into hands / review / agent-decides, and the
console renames the buckets again for the phone (Questions / Only you can do /
Reviews / Escalated). Use the census vocabulary; treat the 2026-10-26 doc as
history.

---

## 4. How the answer-back flows work (roadmap/prioritisation angle)

Full catalogue: `survey/website-pages.md` §F (question sets F1–F8). The flows
that change roadmap/priority state:

- **Decision sets** (`/decisions/<setId>`, console `#needs/questions`): an
  answer / defer / request-info records a resolution in the decisions store;
  the dropped-thread catcher applies it to the Muse-side decisions ledger
  per question. A `status-request`-class consequence (e.g. an epic/project
  status change) is _not_ written by the dashboard — it becomes an action
  (next bullet).
- **Dashboard actions** (`POST /api/actions`, `POST /api/actions/batch`;
  consumers poll `GET /api/actions.json?state=new`; outcomes return via
  `POST /api/actions/result` `{id, outcome: applied|rejected, byRun, note}`):
  kinds are `triage` (acknowledge / snooze / escalate / de-escalate /
  **answer** / delegate / dismiss), `status-request`, `session`
  (poke/**answer**/stop), `preference` (set-delegation-default). Triage and
  status-request actions are consumed by the orchestration pass — the
  Project's single writer — which applies them and reports back; the item's
  next published tier entry carries the ack. Escalate/de-escalate are tier
  overrides with the reason recorded; snooze suppresses until a named time
  and renders visibly as "snoozed by you". The triage `answer` verb is an
  answer-back: "the pass delivers payload text/option/answers like its own
  classifications" (`actions.ts` header). Outage-queue variant:
  `GET /api/actions/outage-queue.json`, `POST …/confirm`, `…/reject`.
- **Console direct writes (the exception path):** rank changes (`setRank`),
  issue comments, queue actions (`queueAct` via
  `POST /console/api/queue/<task>/<action>`), and hands-done label removal are
  performed by the console itself through `gh` / the queue broker,
  audit-logged before execution (`console/src/{ops,work,server}.ts`).
  **D4:** the aggregator service never writes GitHub labels; the stack does,
  through the console — the orchestrator is not the only label writer. (The
  single-writer rule is about Project _fields_; the console writes _labels_ —
  the seam must not be blurred.)
- **Roadmap annotations** (`POST /api/roadmap/annotations`, `/alias`):
  Drew-side notes/aliases attach to roadmap rows without editing Grist or the
  repo files — the annotation store (`roadmap-annotations.ts`) is the
  aggregator's own, reconciled at render (`roadmap-reconcile.ts`).
- **Intake** (console `#more/intake`, `plan.ts`): a note/bug/change becomes a
  GitHub issue with the route's labels + rank _and_ a queue packet
  (`triage.enqueue`, or `prompt.enqueue` once an issue exists) — new work
  enters both the portfolio record and the execution queue in one action,
  with the rank from the route table, not chosen ad hoc. Intake label:
  `owner/intake`.
- **Needs-Drew census answers** (console `#needs`): **Reply**
  (`act("need.comment")` → GitHub issue comment with
  `<!-- owner-reply:v1 -->`) and, for hands items, **I did it**
  (`act("hands.done")` → GitHub comment "Done by the owner…" + deletes the
  `user/escalated` label). The aggregator census itself never writes labels;
  the console does (D4).
- **Reports:** `POST /api/reports/resolve` (resolve a prose conflict choosing
  a stored version), `POST /api/reports/late-update` — smaller answer-backs
  on the `/reports` surface.

Question-set mechanics in full (decision sets F1, single questions F2,
census sets F3, prompt/chat F4, dashboard comments F5, dashboard actions F6,
console generic actions F7, reports/roadmap/alerts F8) are enumerated in
`survey/website-pages.md` §F and the book's `question-sets.md` reference;
push-side answering is in `push-flow.md` §Answer-back.
