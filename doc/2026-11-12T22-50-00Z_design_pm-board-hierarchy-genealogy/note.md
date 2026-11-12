# Design: PM stack multi-board hierarchy + full task genealogy

- Date: 2026-11-12 · Program epic: #9060 · Parent epic: #1184 (PM stack coherence)
- Inputs: Drew's directive 2026-11-12 ~16:27 CST (quoted in #9060);
  `pm-stack/lineage-manifest-2026-11-12.md` (PR #8980); design register #8805 /
  successor #8986; visualization design #1122 (+ adoption #1262–#1266);
  board-family work of 2026-11-12 (Projects #4/#5, forks #1, Lab Cluster #1).
- Status: design for implementation. Slices are filed as sub-issues of #9060;
  the collector multi-board + identity slice is #9062 (already filed — this
  design is its specification, not a duplicate).

## 0. What exists (V1, measured 2026-11-12)

- Boards: developing-today Project #4 (1,229 items; fields Status ×9 options,
  Area ×12, Horizon, Kind(Task|Epic); 22 views incl. #22 "ID Program",
  Roadmap — \<repo\> views 11–15, Program views 16–21, Design Register view 4,
  Designs — \<area\> views 5–10); Project #5 "Idea Register" (ideas only,
  draft items, stays ideas-only); developing-today-forks Project #1 (72 items);
  Clustered-Computing Project #1 "Lab Cluster" (37 items). **The satellite
  boards have stock schemas only** (Status Todo/In Progress/Done; no
  Area/Horizon/Kind).
- Collector: `pm-stack/aggregator/src/projects.ts` — ONE hardcoded board
  (`PROJECT_ID PVT_kwDOAZNvF84Bouay`), `PROJECT_QUERY` already fetches fields,
  views, per-issue `parent`, `blockedBy(first:50)`, `blocking(first:50)`,
  `labels(first:20)`; `resolveEpicRefs()` walks parent chains to the nearest
  Kind=Epic ancestor. Pagination to exhaustion landed today (PR #9059).
  `/api/projects.json` returns one flat pool + a hardcoded
  `project: "developing-today#4"`. Identity is the Projects-only PAT
  (`github.project-token`, developing-today-scoped, shared bucket, no repo
  access) — it **cannot read the forks or Clustered-Computing boards**.
- Substrate: native sub-issue chains + two-sided epic membership
  (`epic/<name>` label(s), many allowed + at most one native parent);
  `design/<name>` labels for top-level designs (3+ issues) with per-design
  tracking issues (register: 41 top-level + 69 named sub-designs, on paper);
  effort/mode labels; effective-Area inheritance designed in #1122 (#1262
  still open).
- Milestones: 15 in this repo, ALL created 2026-11-12, all open, all
  `due_on = null`, all **program containers** — #3/#5 explicitly "ROLLING…
  lands here indefinitely". Zero release containers exist. Milestone does not
  inherit down native chains (only #2056 carries #3; #3195/#3447/steps none).
- Dashboard (:4870, Bun/TS): `/roadmap` renders Grist "Program roadmap" rows
  (28 live; flat — no priority, parent, or release field) with a live
  drill-down join row → tracked epic → epicRef children; `/program/<id>` is
  the per-project page (registry of 6 in `src/project-config.ts`, keyed by
  Area); `/item/<subject>` + `renderItem` is the item-detail surface;
  **milestones/releases appear nowhere in the stack** (grep = 0).

## 1. Board registry model (D7, S1/S2)

Boards are **data, not constants**. New module
`pm-stack/aggregator/src/board-registry.ts`:

```ts
interface BoardDef {
  id: string;              // stable local id, e.g. "devtoday-main"
  org: string;             // "developing-today"
  number: number;          // 4
  nodeId: string;          // PVT_…
  url: string;
  role: "overall" | "project" | "wip" | "register" | "ideas";
  parentBoardId?: string;  // board hierarchy lives here (GitHub has none)
  siblingOrder: number;
  title: string;
}
```

Seed registry = the five boards above: #4 (overall, parent of the family),
#5 (ideas, sibling), forks #1 (project, child of #4), Lab Cluster #1
(project, child of #4). Parent/child and sibling relations are registry
metadata rendered by the dashboard; GitHub Projects have no native board
nesting, and this design does not fake it with items.

Collector: parameterize `fetchProjectSnapshot/fetchProjectPool(token, board)`,
loop the registry **fail-soft per board** (one board failing never empties
the pool), merge into one pool where every item gains `boardId` (items
already carry `projectUrl`; refs are board-agnostic `repo#number`, so
`resolveEpicRefs` runs over the **merged** pool and cross-board ancestry
resolves). Feed + API: `projectAudit` becomes per-board; `/api/projects.json`
returns `projects: BoardSummary[]` instead of the hardcoded singular string
(back-compat: keep `project` for one release, marked deprecated).

**Field schema**: Project #4's schema is canonical. A standardization slice
applies Status (same 9 options), Area, Horizon, Kind to forks #1 and Lab
Cluster #1 so items mean the same thing on every board; until then the
collector treats satellite items as Status-only (fail-soft parse already
does this).

**Identity (with #9062)**: the collector stops fetching the dev-today-only
PAT directly. Per board, it obtains a token through the fleet rotation
(personal PAT with `project` scope → per-org App installation tokens, all
three Apps hold `organization_projects: write` on all admin orgs incl.
developing-today-forks and Clustered-Computing) — i.e. the collector calls
the same tier machinery `fleet gh` merged today (PR #9054), per-org.

**WIP board**: the granular current-implementation board is a *role*, not a
new project: a dedicated view on #4 (Status ∈ {In Progress, In Review},
all Areas) + the dashboard `/board` surface, which already curates WIP
across sources, extended to all registry boards.

## 2. Hierarchy levels and edges (D2, S2/S3)

Levels, top to bottom, and the edge that links each pair:

| Level | Entity | Edge to level below | Edge lives in |
|---|---|---|---|
| Overall roadmap priority | **Roadmap-item issue** with `kind/roadmap-item`, Horizon set, on board #4 | native parent of roadmap items (or label relay, below) | GitHub native sub-issue |
| Project roadmap item | Roadmap-item issue (same kind, per project) | native parent of its epics where the slot is free; else the epic carries `epic/<name>` + `primary-parent/<roadmap-slug>` (§3) | GitHub native / labels |
| Design | Design tracking issue (`design` label + `design/<name>`) | **spawned-by**: the epic's native parent is the design tracking issue where free; else epic carries `design/<name>` and the design's tracking issue lists it (collector derives `spawnedByRef` from the label→tracking-issue map in the design register manifest) | GitHub native / register manifest |
| Epic | Issue, Kind=Epic, `epic` + `epic/<name>` | native parent of its tasks (the standing two-sided membership) | GitHub native + labels |
| Task / bugfix | Issue or PR | — | — |

Cross edges: **blocked-by** = native `blockedBy` (#1122 semantics: an open
blocker blocks; the `status/blocked` label never creates edges).
**Sequencing** ("started after C closed") = a blockedBy edge from the
successor roadmap item to the predecessor roadmap item, plus the
predecessor's release milestone (§4) supplying the date.

Roadmap items as **issues** (not only Grist rows): the Grist "Program
roadmap" table stays as a legacy mirror; its rows already join to tracked
epics (drift check #1144). New roadmap items are created as issues; the
reconcile/drift machinery reports Grist rows with no issue and issues with
no row, and slices migrate the 28 live rows to issues 1:1 via their
`tracks` links. GitHub constraints honored: one native parent per issue,
≤100 sub-issues per parent (measured: #1184 is already at the cap — the
program epic #9060 therefore cross-links #1184 by comment + label instead
of native parenting; grouping epics that approach the cap get an
intermediate grouping epic), ≤8 nesting levels (our chain depth is 5).

## 3. Primary parent (D1, S10)

An issue may belong to many epics (labels) but has exactly **one primary
parent** — the one the genealogy sentence follows.

**Canonical form (GitHub): the native sub-issue parent, when one exists, IS
the primary parent.** When the native slot is unavailable or deliberately
points elsewhere (multi-carrier epics, an item whose working parent is a
design but whose *program* parent is an epic), the primary parent is marked
with a label:

```
primary-parent/<slug>     e.g. primary-parent/id-crate-split
```

`<slug>` = the parent epic's `epic/<slug>` name (longest current slug is
22 chars; `primary-parent/` + 22 = 37 ≤ GitHub's 50-char label limit).
Labels are the only form queryable in GitHub (REST list, Search API,
GraphQL label filter, Projects group-by) — a body-only marker cannot be
the system of record.

**Mirror (human-readable, generated):** `project-reconcile.sh` maintains a
`## PM lineage` block in the issue body —

```
## PM lineage
primary_parent: developing-today/code#3447
spawned_by: developing-today/code#3195
roadmap_item: developing-today/code#<A>
```

— regenerated from the canonical data, never hand-edited (the block is a
mirror; divergence = a reconcile finding, not a second truth).

**Invariants** (collector-checked, surfaced in the audit/coverage output):
at most one `primary-parent/*` label per issue; if a native parent exists
and a `primary-parent/*` label also exists, they must agree or the label
must name an ancestor of the native parent (the program-level parent);
violations are reported, never silently resolved.

## 4. Milestones: two kinds (D3, S4/S5)

The existing 15 milestones are **program milestones**: rolling standing
homes, `due_on = null`, never close. They stay exactly as they are.

New: **release milestones** — bounded containers that close with a release.

- Title: `<Program> — release <version or date>` (e.g. `ID platform — release 2026-12-01`).
- `due_on` = the release date. This is where "closed with the release on
  \<date\>" comes from — the field that does not exist anywhere today.
- Membership: issues/designs/epics assigned to ship in that release. (An
  issue has one GitHub milestone; assignment to a release milestone
  supersedes its program-milestone assignment for the shipping window —
  program membership remains visible via labels, which is why labels are
  additive and never removed.)
- **Close-out accounting**: when a release milestone closes, the collector
  computes and the stack stores a snapshot: counts of issues covered,
  designs covered (items carrying `design/<name>` or whose Kind/role is a
  design tracking issue), epics covered (Kind=Epic), each split closed /
  carried-over, plus the release date. The snapshot is written to the
  milestone description (GitHub-visible) and served by the API (§6) —
  Drew's example shape: "closed with the release covering 123 issues,
  23 designs, 4 epics."
- Retroactive pass: accounting is *computed* for the existing program
  milestones too (they get counts, not closures).

**Carryover (D5, S6) — pinned meaning:** an epic **spans** release
milestones. It stays open; at a release close, its milestone assignment
moves to the next release milestone (or is unset until one exists) and the
closing milestone's snapshot lists it under *carried over*. A carryover
epic is normally a *parent* epic: its child epics may close with the
milestone while it continues — holding fixes from already-merged work
(already-closed children) plus later design phases (open children). No
re-issue, no duplication: one epic, moving assignment, snapshot history.

## 5. Designs spawning designs (D6, S7)

The register already names 69 sub-designs under 41 top-level designs — on
paper only, because `design/<name>` is top-level-only and tracking issues
are unparented. Representation, reusing #1122 machinery verbatim:

- A spawned design's **tracking issue is natively parented** under the
  spawning design's tracking issue. Sub-designs carry the parent's
  `design/<name>` label (no new label family; the label means "this design
  family", the native chain gives the tree).
- **Spawn rollup** is then computable from data the collector already
  fetches: children via the native chain, done/in-progress counts via
  `subIssuesSummary{total, completed}` + Status, and the blocking spawn
  via `blockedBy` (spawn S blocks the final design's tracking issue).
  Drew's example renders as: "spawned 5 designs: 2 done, 2 in progress,
  1 blocking \<final design\>."
- Backfill slice: parent the register's existing sub-design tracking
  issues (where filed) under their parents; file tracking issues for
  named sub-designs that lack one only where the register shows live work.

## 6. Genealogy read model + surfaces (S3/S4, Phase 2/3)

Collector additions to `PROJECT_QUERY` (per board): `milestone { title
dueOn closedAt }` on issues, `subIssuesSummary` (already partially
present), and PR labels/edges parity with issues (today PRs fetch
state/draft only — genealogy for PRs rides their linked issue until this
lands; noted as a known gap, sliced separately if it bites).

Aggregator derivation (pure, over the merged pool + register + roadmap
rows):

```
ancestors(item) =
  walk native parents → chain
  epic        = nearest Kind=Epic ancestor (exists: resolveEpicRefs)
  primaryEpic = primary-parent label target if present, else epic
  design      = nearest ancestor that is a design tracking issue,
                else design/<name> label → register tracking issue
  roadmapItem = nearest kind/roadmap-item ancestor,
                else the Grist row whose tracks link names the epic
  priority    = roadmapItem's own roadmap-item ancestor (or itself)
  release     = priority's (or roadmapItem's) release milestone:
                { title, dueOn } + predecessor via blockedBy →
                "started after <C> closed with the release on <dueOn>"
```

Sentence renderer (server-side string builder, links as refs):

> Working on **X** as part of epic **Y**, spawned by design **Z**. **Y** is
> part of roadmap item **A**, part of overall roadmap priority **CC**,
> started after **C** closed with the release on **\<date\>**.

Chain breaks render honestly ("no epic yet", "no roadmap item yet") —
the codebase's existing idiom — never a fabricated link.

API: `/api/item/<id>.json` gains `ancestors: [{ref, title, kind,
relation}]` (relation ∈ parent | primary-parent | spawned-by |
roadmap | priority) + `release` where known; `/api/projects.json` gains
`projects[]` (§1); `/api/roadmap.json` rows gain `priority`,
`afterRefs`, `releaseRef`; new `/api/releases.json` serves milestone
snapshots (§4).

Dashboard (Phase 3, pages.ts recipe — PageDef + render fn + route + API
twin): (a) **genealogy header** in `renderItem` directly under the title
(cheapest surface; the /item branch already loads pool + roadmap data);
(b) **per-project roadmap**: upgrade the existing "Roadmap — live" section
on `/program/<id>` to the timeline/drill machinery filtered by the
project's Area; (c) **overall roadmap**: a `?view=priorities` mode on
`/roadmap` ordered by priority rank with after/release edges; (d)
**Releases** section on `/roadmap` rendering close-out cards (date,
counts, carried-over epics). Roadmap priority/after/release data lives in
an annotations-style overlay store (precedent: `src/roadmap-annotations.ts`)
until Grist grows the columns.

## 7. Closure gates + re-parenting (D4/D5, S8/S9)

**Gate** — an epic or roadmap item may close only when ALL hold:
1. Every design in its subtree is done (design tracking issues closed, or
   explicitly Drew-gated — the #8805 exception precedent).
2. No open descendant is a qualifying bug/follow-up: `kind/bug` with
   priority high or critical (the live `priority/*` family; the slice
   audits exact label names first), or anything labeled as a follow-up.
3. Every remaining unfinished child has a recorded re-parent destination.

**Re-parenting** (on close): each unfinished child moves to the
**most-related** open epic — relatedness, in order: shared `design/<name>`
> shared primary-parent chain > shared `epic/<name>` > shared board Area —
else to the project's **maintenance epic**. Native parent is re-pointed
(`replace_parent`), `primary-parent/*` label updated, all other labels
kept (additive-retention rule), and the move is commented on both the
child and the closing epic. This overrides the lineage pass's
never-re-parent precedent **for closes only** — ordinary re-parenting
outside a close still does not happen silently.

**Maintenance epics**: none exist per project today (only repo-wide
#8978 daily-maintenance). A slice creates one per project — ID, Atlas,
fleet, pm-stack, lab-cluster, mcpx — titled `epic(<project>): maintenance`,
labeled `epic` + `epic/<project>-maintenance` + the project's `app/*`,
natively parented under the project's program epic where the cap allows,
and boarded on the project's board.

**Enforcement**: a close-check in the weekly monitor family (precedent:
`design-register-weekly-monitor`) + a reconcile report section listing
epics that *look* closable-but-gated and closes that skipped re-parenting.
GitHub cannot hard-block a close; the monitor is the enforcement surface,
which matches how #8805's accidental auto-close was caught.

## 8. Worked example (Phase 2 acceptance)

The crate-split chain, fully native today: steps #4403–#4411 →
**#3447** (epic) → **#3195** → **#2056** (ID epic, milestone #3).
Backfill applies §3–§5 to it: `primary-parent/id-crate-split` mirrors on
the steps (native already agrees), `## PM lineage` blocks generated by
reconcile, #3447's spawned-by recorded to the #3195 design effort
(`design/id-crate-split` already labels both), and the chain renders in
the genealogy header end-to-end. The top of the chain (roadmap item /
priority for ID) is created under §2 if absent — the ID program's
roadmap-item issue — so the sentence completes to the top on real data.

## 9. Slices (filed as sub-issues of #9060)

1. Collector: board registry + multi-board + per-org identity (= #9062,
   cross-referenced; this doc is its design input).
2. Satellite board field-schema standardization (forks #1, Lab #1;
   verify forks schema first — it was not readable during design).
3. Primary-parent convention: label family + collector invariants +
   reconcile body-mirror + crate-split backfill.
4. Release milestones: convention, collector ingestion, close-out
   accounting (+ retroactive counts for the 15 program milestones),
   `/api/releases.json`; includes the lab milestone duplicate pair
   (#9/#15) consolidation recommendation.
5. Design-spawn backfill: native parenting of sub-design tracking
   issues + spawn rollup in the API.
6. Maintenance epics per project + closure-gate/re-parent tooling +
   monitor section.
7. Genealogy read model: ancestors computation, sentence renderer,
   `/api/item` ancestors, roadmap row priority/after/release overlay.
8. Dashboard surfaces: genealogy header, per-project roadmap upgrade,
   overall priorities view, releases section.

## 10. Non-goals / settled constraints (not re-opened)

- Project #5 stays ideas-only; the 14 personal-repo register items stay
  off Project #4 (Drew's standing rulings).
- Labels are only ever added, never removed, by this program's machinery.
- `project-reconcile.sh` remains the boards' single writer.
- Grist remains the roadmap mirror until the row→issue migration slice
  completes; drift is reported, never silently resolved.


## 11. Link semantics (amendment, Drew 2026-11-12 16:45 CST)

An item has **exactly one direct parent at a time**, but it keeps
**historical links**. When work splits (an item sat under epic 123, part of
it moves under epic 345), the item keeps both links: 345 as the direct
parent, 123 as a **historical** link. A link that was simply **wrong**
(bad classification) is a different operation: it is reclassified and the
invalid link is **removed** — not retained as history.

Every link is therefore a record, not a bare pointer, carrying a
`relation` (parent, spawned-by, blocks, relates, board-parent, …), a
`status`, and provenance:

- `status: active` — the current direct parent (exactly one per item), or
  a current non-parent relation.
- `status: historical` — a former direct parent, retained on a split/move,
  with `until` + `reason` (e.g. `split`, `carryover`, `reparent-on-close`).
- `status: removed` — a wrong link, deleted from GitHub (native parent
  re-pointed / label removed) and recorded in Metadata with
  `reason: misclassification`, so the *removal* is auditable without the
  link itself surviving. Historical-retention and wrong-link removal are
  distinct operations and the data must never blur them (§7's
  re-parent-on-close produces `historical`, never `removed`).

Links are **level-agnostic**: items at any level can parent or link items
at the same level (epic↔epic, design↔design) or any other level. Board→
board links follow the same rule — usually a higher-level board links to a
more granular board, but there is **no structural limitation** on link
direction or level; the board registry (§1) stores board links as link
records with the same relation/status shape. The genealogy walker (§6)
follows only the direct parent (+ historical chain when rendering
"formerly part of …"); other relations feed graph views.

## 12. The Metadata section — every issue and PR (amendment)

Every issue and PR body carries a **Metadata section**, delimited by
HTML-comment markers so it is invisible in rendered views but trivially
locatable in the raw body:

```
<!-- pm-metadata:start -->
```json
{
  "pm_meta": 1,
  "direct_parent": {"repo": "developing-today/code", "number": 3447, "kind": "epic"},
  "links": [
    {"target": {"repo": "developing-today/code", "number": 123},
     "relation": "parent", "status": "historical",
     "since": "2026-11-02", "until": "2026-11-12", "reason": "split"}
  ],
  "lineage": {"design": null, "roadmap_item": null, "priority": null},
  "provenance": {"queue_id": null, "session_id": null, "lane": null,
                 "host": null, "worktree": null, "picked_up_at": null,
                 "work_started_at": null, "created_at": null,
                 "model": null, "lane_start_issues": []}
}
```
<!-- pm-metadata:end -->
```

Format decision: **JSON**, not YAML or a bespoke dictionary syntax.
Strict parse with zero indentation ambiguity; both consumers (fleet in
Rust/`serde_json`, the collector in TypeScript) parse it natively; keys
are arbitrary and values nest freely, as required. The block is
human-inspectable in the raw body and diff-friendly in edits.

Relationship to §3: GitHub-native marks (native parent, `primary-parent/*`
label) remain the GitHub-side representation; **the Metadata block is the
canonical record** — the collector and the fleet database (§14) trust it,
and `project-reconcile.sh` keeps the native marks, the label, and the
block in agreement, reporting divergence as a finding instead of letting
two truths drift. §3's `## PM lineage` mirror is superseded by this
block (same fields, machine-canonical form).

## 13. fleet gh as a superset of gh (amendment; extends #9016)

`fleet gh` (rotation shipped today, #8887) grows a Metadata-aware layer —
this is the implementation home for §11/§12, and it extends the step-2
scope filed as #9016 (cross-linked, not duplicated):

- **Create path** (`fleet gh issue create`, PR create): the caller's
  description may already contain a Metadata block. Fleet parses it, then
  **stamps** the fields it knows: queue ID, scheduled start, current time,
  pwd, worktree, host, the time the queue item was picked up, the time
  work started, the set of issues worked on by the same lane start,
  session id (§14), and model. It ensures required fields are present —
  injecting defaults/what it knows when absent — then pushes the issue/PR
  and **inserts the full record, including the full Metadata, into the
  fleet database itself** (§15). A failed DB insert never loses the
  GitHub item: the ingestion layer (§16) heals it.
- **Query path**: fleet offers Metadata queries — list/filter issues and
  PRs **by Metadata fields** (e.g. by direct parent, by session, by lane,
  by provenance host) — served from the database (§15), with GitHub as
  fallback for fields the DB has not yet ingested.

## 14. Session state (amendment)

Each fleet start assigns a **session id**. It is stored (a) in the
environment of everything fleet launches, and (b) in a **pwd-file at the
repo root of every worktree it operates in**, so any later fleet
invocation in that context recovers the provenance without being told.
Resolution order: explicit argument > environment > pwd-file. The session
id is what the create-path stamp (§13) writes into `provenance.session_id`,
which is what makes "which session/lane start produced this item"
answerable from the item itself.

## 15. The metadata database (amendment)

A fleet database tracks **every issue and PR in scope** (all repos the
fleet works in), storing the **full Metadata as JSON** plus extracted key
fields (direct parent, session id, lane, host, queue id, relation/status
pairs) indexed for filtering. The DB is a **first-class query surface
alongside GitHub**: the collector reads genealogy inputs from it, the
dashboard can serve from it, and `fleet` Metadata queries (§13) run
against it. GitHub remains the system of record for the items themselves;
the DB is the system of record for Metadata *queries* — divergence
between a body block and its DB row is a healing target (§16), reported,
never silently overwritten in the wrong direction (body wins on content,
DB wins on nothing — DB rows are rebuilt from bodies + fleet stamps).

## 16. Dual ingestion / self-healing (amendment)

Fleet does not create every item — plain `gh`, the web UI, and other
actors do — and its own DB insert can fail. Two independent watchers keep
the database complete:

1. **Webhook listener**: issue/PR events (opened, edited, closed,
   reopened, labeled) trigger a Metadata check: parse the block + labels/
   title/status; insert or refresh the DB record; enqueue a queue item
   when the event warrants work.
2. **Interval sweeper**: periodically re-scans in-scope repos for
   new/changed items the listener missed (downtime, delivery failures),
   performing the same check.

For items **lacking** a Metadata block, the watcher ascertains what it
can (native parent, labels, milestone, timestamps, actor), generates a
block, stores it in the DB, and writes it back to the body where the
fleet has write access. When the ascertained Metadata is **insufficient**
(no resolvable parent, ambiguous program), the item is **forwarded for
full triage** instead of being left half-described. PRs that merge
quickly may be detailed **retroactively**: the sweeper path explicitly
supports completing Metadata after merge, for record keeping — a merged
PR is a valid ingestion target, not a miss.

## Amendment slice map

New implementation slices under #9060 (in addition to §9):
9. **fleet gh Metadata superset** — create-path parsing/stamping/required
   fields + DB insert (extends #9016).
10. **Metadata database + fleet query surface** — schema, extracted keys,
    list/filter by Metadata fields.
11. **Session state** — session id assignment, env + worktree pwd-file,
    resolution order, stamp integration.
12. **Dual ingestion** — webhook listener + interval sweeper, Metadata
    generation/backfill, triage forwarding, retroactive completion for
    merged PRs.
And §9 slice 3 (primary parent) now also carries §11's historical-link
semantics in the collector (link records with status, historical chain
rendering, removed-link audit).
