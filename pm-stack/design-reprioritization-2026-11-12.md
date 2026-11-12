# Fleet-wide reprioritization — program scheme and record

Ordered by Drew in the main chat, 2026-11-12 ~00:57 CST: reprioritize all issues to focus on major improvements to ID, Atlas and fleet, their related SaaS service, PM stack, SaaS Turso and Cloudflare integration, better use of GitHub and dynamic compute, better use of storage, and a major item for the lab build-out and integrating existing work / fully using the k8s cluster and other machines; lower priority for smaller fixes that aren't blocking. Each focus area gets distinct designs or projects, milestones, labels, deliverables. Existing issues get grouped into appropriate epics and designs with all appropriate labels. Census goes one by one through every issue in developing-today/code first, then the other repos.

Builds on: the design register (`pm-stack/design-register-2026-11-11.md`, 117 entries + 131 ideas), the tracking manifest, the board family in TOOLING-SETUP.md (org Project #4 master board, Project #5 ideas board), tracking epic [code#8805](https://github.com/developing-today/code/issues/8805).

**Naming-scheme assumption (recorded per the coordinator brief):** the sibling lane creating `epic/<name>` and `design/<name>` labels and per-epic milestones had not yet written its section into TOOLING-SETUP.md when this scheme was published (2026-11-12). This program adopts exactly that scheme — `epic/<short-name>`, `design/<short-name>`, the `design` label (0e8a16), and the five existing milestones (nix-cache program · OpenCode 2 fleet migration · Atlas program · Docs migration program · Design register 2026-11) — and creates nothing under different names. If the sibling lane's final names differ, this doc gets a correction note and the lanes reconcile to one scheme.

## Priority order (top → bottom)

1. **Drew's expedite set** — pinned, untouched by this program. Nothing here demotes an expedite item.
2. **Band 10** — fleet-stalling breakage only (existing rule; unchanged).
3. **The nine focus programs**, in Drew's listing order (bands below). Justification: Drew listed the core products first (ID, Atlas, fleet), then their service/platform layer (SaaS service, PM stack, Turso+Cloudflare), then the leverage layer (GitHub/dynamic compute, storage, lab). Bands follow that listing so the queue drains in the order he named.
4. **Blocking small fixes** — a small fix that unblocks program work or fleet operation keeps its teeth: it takes the band of the thing it blocks (recorded per issue), default band 30 when the blocked thing is unclassifiable.
5. **Non-blocking small fixes** — band 70. They still get done; they stop outranking program work.
6. **Not-a-work-items** (records, questions, tracking shells) — priority untouched; the census notes why.

### Band map (fleet queue design scale 10/20/30/40/50/70/90)

| Band | Contents |
|---|---|
| 90 | Drew's expedite set (his alone) |
| 10 | Fleet-stalling breakage only |
| 20 | P1 ID · P2 Atlas · P3 Fleet (core products) |
| 30 | P4 SaaS service · P5 PM stack · P6 Turso + Cloudflare |
| 40 | P7 GitHub + dynamic compute · P8 Storage · P9 Lab build-out |
| 50 | (reserved — composable-claim mid band; unused by this program) |
| 70 | Non-blocking small fixes |
| — | Blocking small fixes ride the band of what they block (default 30) |

Priority mechanics: where an issue has a fleet-queue task, the task is re-banded to the target band. Where it doesn't, grouping + labels + milestone carry the signal, and this doc records the band the group claims at when it enters the queue.

## The nine programs

Each program ends with: design(s) on file · a project/board view · a milestone · labels · explicit deliverables recorded on the epic/milestone.

### P1 — ID (band 20)
Major improvements to the ID binary/program.
- Register: F1 ID platform program (design mill) + F1.x sub-designs.
- Epic home: the ID program's standing issues in developing-today/code (register F1; census confirms the current epic issue).
- Labels: `design` + `epic/id-program` + `design/<name>` for multi-issue F1 designs.
- Milestone: "ID program" in developing-today/code (create if the sibling lane hasn't).
- Board: Project #4 view "Designs — ID & fleet" exists; a P1-specific view is added in phase 5 if the label filter serves better.
- Deliverables: recorded on the epic — the current F1 design-mill fronts (per register §F1) each with their tracking issues.

### P2 — Atlas (band 20) — ROLLING, indefinite
Major improvements to Atlas; follow-on work never ends, so the program structures are standing, not one-shot.
- Register: A1 Atlas program (PARENT) + A2.
- Epic home: dezren39/hatch-personal Atlas issues (#35–#45 family).
- Milestone: "Atlas program" in hatch-personal (exists; rolling — its description states follow-on work lands there indefinitely).
- Labels: `design` + `epic/atlas-program`.
- Board: Project #4 view "Designs — Atlas" exists but stays empty of the hatch-personal items — see the Drew ruling below; Atlas's tracking home is labels + milestones + the register.

### P3 — Fleet (band 20)
Major improvements to the fleet binaries and orchestration.
- Register: F2 mcpx platform · F3 OpenCode platform · F4 fleet orchestration & dispatch.
- Labels: `epic/fleet-orchestration`, `epic/mcpx-platform`, `epic/opencode-platform` + per-design labels.
- Milestones: "OpenCode 2 fleet migration" (opencode, exists); fleet-orchestration/mcpx milestones in code per the sibling-lane scheme (own milestone for big epics, roll up small ones).
- Board: "Designs — ID & fleet" view + per-repo roadmaps.

### P4 — SaaS service for ID/Atlas/fleet (band 30)
The hosted service side: the Supabase muse-dev platform and its successors.
- Register: X8 Supabase agent schema + coordination platform · X7 Supabase DIY backups + $30 ceiling · goal `supabase-muse-dev-platform-backups`.
- Gap: no single design describes the SaaS service as the service counterpart of P1–P3 → **drafted in phase 2** as a living doc, marked as a draft for Drew's ratification.
- Labels: `epic/saas-service`. Milestone: "SaaS service" in the home repo the draft lands in (code, unless the draft's home is hatch-personal — decided in phase 2 and recorded here).

### P5 — PM stack (band 30)
- Register: P1 PM stack program (single URL) · P2 decisions system · plus tonight's board-family work.
- Epic home: code#8805 (register) + the PM program's standing issues; tie-in issue code#8864 carries the board-family page work.
- Labels: `epic/pm-stack` + `app/pm-stack` where it already exists. Milestone: "Design register 2026-11" covers the register slice; a standing "PM stack" milestone is created if the sibling lane hasn't.
- Board: "Designs — PM stack" view exists.

### P6 — Turso (SaaS) + Cloudflare integration (band 30)
- Register: T1–T5 (Turso) · C1–C9 (Cloudflare) · T3 idea engine is the joint vehicle.
- Milestones: "nix-cache program" (nix-cache, exists, Cloudflare side); a Turso-side milestone in hatch-personal ("Turso platform") if the sibling lane hasn't created one.
- Labels: `epic/turso-cloudflare` + existing per-design labels. Board views "Designs — Cloudflare" and "Designs — Turso" exist.
- Gap check in phase 2: whether one integration design ties the Turso↔Cloudflare pattern (ledger-in-Turso, edge-on-Cloudflare) together; draft if missing.

### P7 — GitHub + dynamic compute (band 40)
Better use of GitHub (Actions, runners, merge machinery) and dynamic compute (Codespaces, Blacksmith, ephemeral runners).
- Register: X9 CI program (2026-10-18) is the ancestor; goals `efficient-use-of-codespaces-actions-and-blacksmith`, `github-actions-usage-review-trim-to-what-s-needed`, `github-pro-bots-runners-what-can-we-actually-use`, `github-rulesets-audit-for-all-worked-in-repos` hold the pieces.
- Gap: no single design → **drafted in phase 2**.
- Labels: `epic/github-dynamic-compute`. Milestone: "GitHub + dynamic compute" in code. Board: program view in phase 5.

### P8 — Storage (band 40)
Better use of storage: cold-store, R2 layout, nix store, backups.
- Register: X6 cold-store backup design · X19 backups family · X1 nix persistence · X17 fleet backups provisioning.
- Gap: pieces exist (cold-store living doc committed) but no single storage-program design ties allocation policy across R2 / nix store / Turso / backups → **drafted in phase 2** if the phase-2 check confirms no existing doc does this.
- Labels: `epic/storage`. Milestone: "Storage program" in code. Board: program view in phase 5.

### P9 — Lab build-out + integration (band 40)
THE major item: build out the lab and integrate existing work, fully using the k8s cluster (Talos/Cozystack) and the other machines.
- Register: X5 lab cluster program (PARENT); epic home code#1817 (basement cluster roadmap, code `planning/roadmaps/basement-cluster.md`); goal `stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs`.
- Gap: X5 covers standing the cluster up; the *integration* design — how existing fleet work (ID, Atlas, fleet services, storage, PM stack) lands on the cluster and the other machines, and what "fully using" means in deliverables — is implied by Drew's order but not on file → **drafted in phase 2**.
- Labels: `epic/lab-cluster`. Milestone: "Lab cluster" in code (create if missing). Board: program view in phase 5.

## Classification rubric (census)

Every open issue gets exactly one primary classification:

- `P1`…`P9` — belongs to a focus program. Record the epic/design grouping it lands in.
- `BLOCKING` — small fix that unblocks program work or fleet operation. Record what it blocks (issue/program) in the census row; its band = the blocked thing's band (default 30).
- `SMALL` — non-blocking small fix. Band 70.
- `RECORD` — not a work item (record, question, tracking shell, decision log). Priority untouched; note why.

Tie-breakers: an issue that is itself part of a program's design/epic structure classifies to the program even if small. An issue in Drew's expedite set is never reclassified downward — expedite stays pinned regardless of its census class (note it, touch nothing). darwin-nix: classify + label only, no priority changes, no new work filed (Drew HOLD).

Application per issue (org repos + personal repos alike, within credential limits): add the program's `epic/<name>` and/or `design/<name>` labels (+ keep `design` where it belongs), assign the program milestone where one exists in that repo, re-band the linked queue task if one exists, and place/update the item on Project #4 where the credential route allows. Every application is verified per batch by a second read.

## Census files

- `census-code-a.md` / `census-code-b.md` — developing-today/code, split at issue #5805 (1,034 open issues at program start, 2026-11-12).
- `census-opencode.md`, `census-openchamber.md`, `census-darwin-nix.md`, `census-nix-cache.md`, `census-hatch-personal.md`, `census-hatch.md` — the other repos.
- Row shape: issue · title (short) · current labels/milestone · classification · epic/design assignment · target band · applied (labels/milestone/task/board) · notes.

## Drew ruling — personal-repo board placement is settled (2026-11-12 ~01:22 CST)

**Never auth into Vertex.** The laptop-token project-scope grant is CANCELLED: it requires fresh sign-in through Drew's employer (Vertex) Microsoft SSO, and the fleet must never trigger or require that authentication. Consequences, recorded so future passes do not re-raise this:

- Personal-repo (dezren39/*) issues CANNOT be placed on org Project #4. That is a settled constraint, not a blocker and not a gap awaiting a fix.
- Their tracking home is labels + milestones + the register — which this program has fully applied to them.
- Do not schedule, suggest, or retry any token scope refresh for this purpose.

## Canonical names as they exist on the wire (reconciled 2026-11-12)

The lanes adopted pre-existing sibling names instead of creating duplicates; these are canonical now:

- Milestones in developing-today/code: "ID platform program" (P1; its phase-14 subset rides "ID Phase 14…"), "Atlas program" (P2 items in code), "mcpx platform program" + "Fleet tool - one door, one binary" + "Pool & routing intelligence" (P3 subsets), "SaaS service" (P4), "PM stack coherence" (P5; register slice in "Design register 2026-11", docs migration in "Docs migration program"), "Turso + Cloudflare" (P6), "GitHub + dynamic compute" (P7), "Storage program" (P8), "Lab cluster" (P9). No general fleet-orchestration or opencode-platform milestone exists in code — those epics are carried by labels + board views; noted, not forced.
- Epic labels: code carries the scheme names (`epic/id-program`, `epic/atlas-program`, `epic/mcpx-platform`, `epic/opencode-platform`, `epic/fleet-orchestration`, `epic/pm-stack`, `epic/saas-service`, `epic/turso-cloudflare`, `epic/github-dynamic-compute`, `epic/storage`, `epic/lab-cluster`). Two repos had shorter sibling-created labels first and keep them: `epic/atlas` in dezren39/hatch-personal, `epic/mcpx` in opencode/darwin-nix. Same meaning, same color family; a rename is cosmetic and was not worth churn mid-census.

## Results (2026-11-12)

**Designs drafted (phase 2):** none of the five checked programs had a design covering Drew's 2026-11-12 ask. All five drafted, header-marked as fleet drafts awaiting Drew's ratification, merged in PR [code#8868](https://github.com/developing-today/code/pull/8868) (`220ac1a5`): P4 `doc/living/saas-service-id-atlas-fleet.md` ([#8869](https://github.com/developing-today/code/issues/8869)) · P6 `doc/living/turso-cloudflare-integration.md` ([#8870](https://github.com/developing-today/code/issues/8870)) · P7 `doc/living/github-dynamic-compute.md` ([#8871](https://github.com/developing-today/code/issues/8871)) · P8 `doc/living/storage-program.md` ([#8872](https://github.com/developing-today/code/issues/8872)) · P9 `doc/living/lab-integration.md` ([#8873](https://github.com/developing-today/code/issues/8873)). Each tracking issue is on Project #4 (Registered), labelled, and in its program milestone. Detail: `designs-phase2.md`.

**Census (phases 3–4): 1,227 open issues classified one by one, every one with a census row.**

developing-today/code — 1,039 rows (`census-code-a.md` #171–#5805: 518 · `census-code-b.md` #5806–#8873: 521; repo stood at 1,034 open at start, 1,038 at end after the 5 design filings and one unrelated closure):

| Class | A | B | Total |
|---|---|---|---|
| P1 ID | 161 | 155 | 316 |
| P2 Atlas | 9 | 61 | 70 |
| P3 Fleet | 164 | 116 | 280 |
| P4 SaaS | 4 | 2 | 6 |
| P5 PM stack | 82 | 33 | 115 |
| P6 Turso+CF | 8 | 8 | 16 |
| P7 GitHub/compute | 6 | 21 | 27 |
| P8 Storage | 6 | 5 | 11 |
| P9 Lab | 30 | 11 | 41 |
| BLOCKING | 13 | 24 | 37 |
| SMALL (band 70) | 24 | 63 | 87 |
| RECORD | 11 | 22 | 33 |

Other repos — 188 rows (one census file per repo): opencode 62 (P3 47 · SMALL 14 · BLOCKING 1) · openchamber 61 (P3 23 · SMALL 33 · BLOCKING 3 · RECORD 2) · darwin-nix 32 (P3 7 · P8 4 · P7 3 · P4 1 · P5 1 · BLOCKING 1 · SMALL 13 · RECORD 2 — classify + label only, HOLD respected, zero milestone/band changes) · nix-cache 5 (all P6) · hatch-personal 21 (P2 15 · P4 2 · P6 1 · BLOCKING 1 · RECORD 2) · hatch 7 (P3 3 · P8 2 · RECORD 2).

**Applied (all verified by post-apply re-reads, 0 failures):** program epic labels on 465 (code A) + 405 (code B, 461 label additions) issues in code and 113 issue edits across the other repos; milestone assignments 128 (code A) + 306 (code B) + 10 newly assigned in the small repos. Expedite-set issues (12 found in code: #2898, #3418, #3447, #3789, #3792, #4000, #4409, #4411, #4837, #5047, #8163, #8173) were classified for the record only and verified untouched. No issue was closed, no body edited, no PR touched by the census.

**The one unapplied leg — queue re-banding:** every census row records its target band, but ZERO queue tasks were re-banded. All three census lanes independently found the laptop `repo-queue` CLI exposes list/claim/lane commands with no confirmable band/priority-mutation route, and none would guess at a mutating command. The re-band itself waits on the queue-owning lane confirming the route; the target bands in the census files are the exact input it needs. Issue-side priority (labels, milestones, board) is fully applied; queue-side bands are recorded, not yet moved. This is the program's one open gap.

## Status log

- 2026-11-12 ~01:00 CST — scheme published (this file). Census + designs fan-out begins.
- 2026-11-12 ~01:05–01:18 CST — designs lane complete (PR #8868 merged, issues #8869–#8873). Census lanes complete: code A (518), code B (521), other repos (188). Program views + commit of this record in the wrap-up step.
- 2026-11-12 ~01:22 CST — Drew ruling recorded: never auth into Vertex; laptop-token scope grant cancelled; personal-repo board placement is a settled constraint (see the ruling section above).
- 2026-11-12 ~01:20 CST — wrap-up (phase 5): six program board views created on org Project #4 and verified by read-back (views 16–21, table below; P3 kept on the comma-OR label filter — the AND form matches 0 board items, the OR form returns a sane 66). This record committed to developing-today/code as `pm-stack/design-reprioritization-2026-11-12.md` and the program summary posted on #8805 in the same wrap-up step. The queue re-band gap above remains the program's one open item, owned by the queue lane.
- 2026-11-12 ~02:40 CST — queue re-banding APPLIED by the queue lane (see "Queue re-banding" below): 392 live queue tasks moved to their census bands, each verified by re-read; the program's one open gap is CLOSED. Nothing waits on the coordinated switch deploy.

## Program views (phase 5)

Created 2026-11-12 on org Project #4 via the Projects v2 GraphQL route (`github.project-token`), each verified by a second read-back of name + filter. P2, P5 and P6 needed no new view — the existing Area views already isolate them ("Designs — Atlas", "Designs — PM stack", "Designs — Cloudflare" + "Designs — Turso"). Item counts are board items matching the filter, counted via the API at creation time (763 items on the project then): label filters only match items already on Project #4, so these counts understate the census totals — most censused issues are not board items.

| View | Filter | Items on board (observed) | URL |
|---|---|---|---|
| Program — P1 ID | `label:epic/id-program` | 80 | https://github.com/orgs/developing-today/projects/4/views/16 |
| Program — P4 SaaS service | `label:epic/saas-service` | 5 | https://github.com/orgs/developing-today/projects/4/views/17 |
| Program — P7 GitHub + dynamic compute | `label:epic/github-dynamic-compute` | 6 | https://github.com/orgs/developing-today/projects/4/views/18 |
| Program — P8 Storage | `label:epic/storage` | 3 | https://github.com/orgs/developing-today/projects/4/views/19 |
| Program — P9 Lab | `label:epic/lab-cluster` | 41 | https://github.com/orgs/developing-today/projects/4/views/20 |
| Program — P3 Fleet | `label:epic/fleet-orchestration,epic/mcpx-platform,epic/opencode-platform` | 66 | https://github.com/orgs/developing-today/projects/4/views/21 |

**P3 verdict — view kept, on the OR filter.** The three P3 epic labels are mutually exclusive on board items (`epic/fleet-orchestration` 36 · `epic/mcpx-platform` 11 · `epic/opencode-platform` 19; no board item carries two of them), so the space-separated form — which GitHub ANDs — matches **0** items and would have been deleted under the near-nothing rule. The comma form is GitHub's OR-within-a-qualifier syntax and returns the full union: **66 items**, a sane non-trivial set against the census's ~280 P3 issues in code plus the fork repos. The view therefore stands with the comma-OR filter, and P3 is additionally covered by the "Designs — ID & fleet" view and the per-repo roadmaps, as before.

## Queue re-banding — APPLIED (2026-11-12, queue lane)

The one open leg from the Results section above is closed: every live queue task whose linked issue carries a census target band has been moved to that band.

**The route (found from source + prior art, not guessed).** The broker has no band-mutation verb: the full Turso verb dispatch in `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` (enqueue/revive/claim/lane/bind/heartbeat/checkpoint/park/release/wake/block/unblock/cancel/fail/complete/list/…) and the `fleet queue` CLI surface contain no priority update, and no `UPDATE tasks SET priority` exists anywhere in the repo. The sanctioned route is the one the 2026-11-11 reprioritization (lane 97d4c74b, #8097 record) established: **direct SQL via `~/tools/turso-exec.py` against the `fleet-queue` Turso DB**, writing per task (a) the row `priority`, (b) the packet copy `payload.$.priority` where the stored packet carries one (the claim guard revalidates the full row — on-scale values only, design scale 10/20/30/40/50/70/90 per `validate.rs` `PRIORITY_SCALE`), and (c) one `reprioritized` audit event. Building a `/reband` broker verb was therefore unnecessary; had it been needed it would have waited on the coordinated switch's broker deploy — nothing in this leg did. No broker restart was performed.

**Applied: 392 tasks** (guarded per-row updates — `WHERE id = ? AND priority = <snapshot value>` — so a raced row would have failed loudly; zero raced). By target band: **20 → 282** (P1 137 · P3 120 · P2 18 · BLOCKING-riding-20 7) · **30 → 35** (P5 28 · P4 2 · P6 2 · BLOCKING-default 3) · **40 → 40** (P9 26 · P7 10 · P8 4) · **70 → 34** (all SMALL) · **10 → 1** (task 12770 for code#7098 — the census's own band-10 call: a fleet-stalling incident, "49 sessions dead" OAuth-blanking wave; breakage, so band 10 is correct under the scheme, flagged here explicitly per the standing rule). By status at write time: blocked 372 · parked 12 · queued 7 · claimed 1 (task 15083, code#8853 P5 40→30 — priority-only write, lease untouched; the worker completed it at the new band minutes later). 15 of the moved tasks carried a packet `$.priority`, all synced. Audit: 392 `reprioritized` events under worker `reprioritization-2026-11-12`, detail carrying from/to/tier/issue.

**Skipped, with reasons (all verified unchanged by the after-read):** 69 tasks already at their census band · 56 RECORD-class (priority untouched by scheme) · 10 tasks linked to Drew's expedite issues (never touched) · 5 darwin-nix tasks (repo on Drew HOLD — census was classify-only there) · 129 tasks whose payload issue is not in the census (spot-verified: closed issues — e.g. code#1837/#2259/#3283/#3353 and the closed crate-split step issues #4403–#4408 — plus tasks for repos the census never covered: developing-today/opencode, developing-today/openchamber, dezren39/nix) · 23 tasks with no issue link (Drew's personal ops/finance tasks 1016/1017, PR-shepherd tasks, scheduled dispatch rows). The 14 off-scale priority-15 blocked rows all belong to the expedite crate-split family (#3447 and its step/triage tasks) and were left exactly as found, consistent with their separate todo handling.

**Verification (two independent full snapshots, taken minutes apart, agreeing exactly):** all 392 planned tasks read back at their target band (391 live + task 15083 completed-at-band); packet `$.priority` in sync on all 15 carriers; **zero** non-planned live tasks changed priority; event count for this worker = 392. Live-set band totals moved 20: 27→306 · 30: 358→190 · 40: 135→90 · 50: 137→46 · 70: 12→39 · 90: 5→1 (the one remaining 90 is expedite-held) · 15: 14→14 (expedite family, untouched) · 10: 2→3 (the #7098 move). Sampled per program: P1 tasks 2050/2955/2974 → 20 · P3 tasks 1287/1319/1930 → 20 · P5 tasks 2456/2468/2499 → 30 · SMALL tasks 1640/3049/4812 → 70.

**Artifacts:** `hidden_files/reband-2026-11-12/` in this workspace — plan.json (per-task from/to), run-log.jsonl (per-task applied records), tasks-live-before/after1/after2.json (full snapshots), band-counts-before/after.json.
