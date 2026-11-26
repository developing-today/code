---
name: queue-reprioritization-audit
description: Full audit and reprioritization of the fleet work queue — census, coverage reconciliation against every open issue, band reorder under the priority law, stale-state and question cleanup, and a closing band audit. Use when the queue's priorities have drifted, after bulk intakes, or on a scheduled reprioritization pass.
---

# Queue reprioritization audit (November programme method, first run 2026-11-24)

Reprioritize the whole fleet queue against the whole open-issue estate,
under the priority law, without ever touching held work. The programme
runs in three passes — census, reorder + cleanup + coverage, band
audit — with a written record at every pass boundary on a tracking
issue. First run: developing-today/code#14158 (2026-11-24); this skill
distils that run, including its failure patterns as named checks.

## Custom rules (first-class input)

A run may carry a **per-run instructions block** besides this standard
programme: a different focus order, extra rules, extra exclusions,
changed band targets, a narrower estate (one org/repo), a different
tracking issue. Custom rules are read first, recorded verbatim on the
tracking issue, and win over every default in this skill **except the
hard rails** — custom rules never override the hard rails, the
priority law's scale, or the read-back verification requirement. With
no custom rules, the standard programme below is the whole brief.

## The priority law

Priorities are set at enqueue, only from the named scale:

| value | name | value | name |
|---|---|---|---|
| 5 | immediate | 50 | high |
| 10 | now | 60 | medium |
| 20 | critical | 70 | low |
| 30 | urgent | 80 | very low |
| 40 | very high | 90 | minimal |

**Higher number = LOWER priority.** Only scale values are allowed.

**Known pattern — validator lag:** the broker's enqueue validator may
still enforce an older value list (on 2026-11-24 it accepted only
[10, 20, 30, 40, 50, 70, 90] while the law's scale was already the
10-name table; the reprioritize verb that updates it was task 21658 /
#14100). Before writing, probe the validator (enqueue a dry-run packet
or read a recent quarantine error) and, while the lists differ, write
only values in the intersection — an off-list value quarantines the
task. Check whether the reprioritize verb (`fleet queue reprioritize`,
#14100) has landed; if it has, prefer the verb over the direct route
and record the switch on the tracking issue.

## Hard rails (never overridden, including by custom rules)

- **Task 18533** is terminal `cancelled` — never change its state.
- **Recorded hard skips stay exactly as-is** (the list lives on the
  tracking issue / in MEMORY: e.g. #1658, #2490 remainder, #9608
  remainders, #2140, #13126 remainders, #13115 ask 3, #13178, #8583
  remainder, #10910, #13506, #13507, program trackers
  #13240/#13241/#13264/#13272). A general cleanup approval never
  rescinds a named hard stop. Note: recording a dependency ON a
  hard-skip issue is a fact, not a touch — allowed.
- **`user/*`-labelled issues are held** (`user/escalated`,
  `user/owned`, `user/review`): their tasks stay exactly as-is,
  priorities included. Sweep for these labels FIRST, before any write.
- **Live claims are never touched** — not even their priorities. A
  claim is released only when verified stale (lease expired AND no
  live worker); on the 2026-11-24 census all 32 claims held live
  leases and the correct stale-claim count was zero.
- A design's self-imposed "not an authorization to implement /
  design-only" gate is **void** wherever it names no hard blocker
  (Drew, 2026-11-24) — the real blockers, if any, are its deps.
- External comms rules unchanged; work lands in the owner's own repos.

## Mechanics

- Queue reads/writes go through the **laptop client**
  (`~/.local/bin/fleet queue …` on amd, via the laptop route forms);
  the sandbox-side client fails through the session proxy. Direct SQL
  against the queue store (the laptop's turso route) is the fallback
  for what the CLI cannot express (appending `blocked_deps`, setting
  `not_before`), queue row first, GitHub sync after, failures to the
  outbox. GitHub reads via `fleet gh api`; writes via `~/tools/gh-post`.
- **Every change is verified by read-back** before it is recorded.
  Tool-reported success is not verification — on the first run, one
  apply pass reported "320 applied, 0 failed" while a fresh read-back
  showed only 197 at target (the run had died mid-apply at a
  transport break). Trust the read-back, never the apply log.
- Records: one comment per pass on the tracking issue with
  before/after band counts and programme totals (moved / unblocked /
  claims released / questions resolved), plus the held list and the
  Drew list (below) — never end silently; if the run must stop, the
  record says where and what remains. If GitHub posting itself is
  blocked (rate limit, channel outage), prepare the record in the
  fleet action outbox instead of dropping it.

## Pass 1 — census

Snapshot the whole queue before touching anything. Since the #14170
queue-schema slices landed, the census runs from the **census pack**:
the canned queries named in the design record
(`doc/2026-11-24T07-23-14Z_design_queue-metadata-history/`, §9), not
client-side assembly over `/list` pages. The pack, per census element:

1. **Counts and bands** (SQL on the queue store): counts by
   queue × status; the actionable set = queued + claimed + handed_off
   + parked + blocked; per-queue, per-kind, per-band tables from the
   same scan. Terminal audit: terminal-status rows with
   `resolution IS NULL` are a finding, not a fact (Q2 columns).
2. **Claims audit:** `fleet queue staleness --lease-cap-seconds 900`
   — the response's `claims_near_lease_cap` names every claim at or
   past its lease cap against the response's own `now`; a claimed task
   absent from that list holds a live lease. (2026-11-24, pre-pack:
   465 actionable of 18,556 tasks; 32 claimed, all live.)
3. **Staleness:** `fleet queue staleness --older-than-seconds <n>`
   — one broker query returns every actionable task whose last
   `task_history` transition is older than the threshold, oldest
   first, with the transition's kind and actor. This replaces the
   ad-hoc age reconstruction the first census did by hand.
4. **Needs-user pile:** `fleet queue needs-user` — blocked/parked
   tasks carrying a `question` metadata entry, oldest first. Its
   blocked-by counts still read the `blocked_deps` cache; they move
   to `task_deps` counts when Q6 lands (see below).
5. **Held check:** all items whose issue carries `user/*` — read the
   Q5 label cache (`fleet queue list --label 'user/*'`), not a
   per-issue GitHub sweep.
6. **Blocker graph — still client-side (the pack's open rows).**
   Open dep edges from `blocked_deps` (issue-keyed), top blockers by
   direct unlock count — these drive the pass-2 push-ups. The canned
   forms (direct + transitive unlock counts from a `task_deps`
   recursive CTE; the blocked-with-no-open-dep cleanup query as a
   `tasks` ⋉ `task_deps` + `metadata.question` join) are specified in
   design §9 and land with Q6 (`queue_links`/`task_deps`). Until
   then this element — and only this element — keeps the client-side
   assembly the 2026-11-24 census used.
4. **Named-error sweep** (check for each, every run):
   - *Off-scale priorities* — any priority not in the named scale
     (2026-11-24: task 21435 at 15, quarantined; fixed to the band
     its own packet recorded).
   - *Malformed deps* — dep entries with null refs (`task:None`);
     scan the full table, not just blocked tasks (2026-11-24: tasks
     16024/20343, found already repaired — verify, don't assume).
   - *Orphaned `handed_off`* — handed_off with no worker: verify the
     handoff target from the task's own record before any action; a
     designated claimant makes it a correct resting state
     (2026-11-24: task 20708, verified legitimate; cancelled later
     only because its issue closed).
   - *The no-dep blocked pile* — blocked tasks with NO open dep
     (computed from the census-pack element 6 assembly until Q6's
     canned cleanup query lands). 2026-11-24: 165 of 343 blocked.
     Expect the composition found
     there: stale environmental blocks, questions already answered
     by standing statements, expired ready conditions, quota-moment
     readings, mis-typed packets, and a minority of genuine holds.
   - *Residue piles* — one band holding a programme-era bulk intake
     (2026-11-24: 276 tasks at urgent/30 from the November intake,
     most of them belonging at 70). A band whose population is mostly
     one era's default is a finding, not a fact.

## Pass 2 — reorder, cleanup, coverage

In this order:

1. **`user/*` sweep first.** Resolve every actionable task's issue
   labels; held tasks are excluded from everything below.
2. **Coverage reconciliation** (before the band audit; the main-focus
   org first and completely, then the rest of the estate under the
   same rules — on the first run: developing-today, then
   developing-today-forks, then the personal account, then the rest):
   - Floor = open issues across all repos the user owns/admins,
     summed per repo, recorded as a table. Scope rules from the
     2026-11-24 run: the member-only org (vertexinc) is excluded
     entirely — never touched; under the personal account only repos
     with real commits from the user count — true mirrors /
     never-committed repos are excluded **by name with evidence**
     (latest commit hash/date/author, user-authored commit count,
     ahead/behind vs upstream) and re-enter the floor when commits
     land there.
   - Cross-reference the floor against actionable items; produce
     three sets on the tracking issue: (a) open issues with NO
     actionable item; (b) actionable items whose issue is CLOSED;
     (c) issues with more than one actionable item.
   - (a) Packetise each uncovered issue into its repo's queue
     (code-pilot is the default home) with a band under the reorder
     rules — EXCEPT recorded by-name exceptions: hard-skip issues,
     `user/*`-labelled issues, pure epics/trackers whose children
     carry the items, issues resolved in substance (close-or-note),
     and repos the broker does not enrol ("payload.repo is not a
     fleet repo" — an enrolment follow-up, recorded, not silently
     skipped; on the first run this was 13 repos / 92 issues,
     including several small developing-today repos and the whole
     twitter-base org). **Coverage rule: every open issue ends with
     AT LEAST one actionable item or a recorded exception; multiple
     items per issue are legitimate distinct slices — dedupe only
     true duplicates of the same work.** Expect the total to land
     ABOVE the floor. Review open pull requests in the same pass:
     they flow through the lane/merge machinery rather than the
     issue queue, and the PRs with queue relevance are those named
     as blockers (dep kind `pr`) — record the PR counts and this
     disposition.
   - *Tracker detection warning:* in this estate `epic/*` labels mark
     programme membership, not parenthood (2026-11-24: 614 open code
     issues carried one; only 92 had sub-issues — the rest were leaf
     work). Classify trackers by sub-issue presence or tracker
     titles, never by the label alone.
   - (b) Cancel stale tasks with the closure as evidence — unless
     live-claimed (the claim rail wins; note them for their workers).
   - (c) Note multi-item issues; cancel only true duplicates, naming
     the survivor.
3. **Question cleanup over the no-dep blocked pile, one by one:**
   answer from the user's standing statements (MEMORY.md / AGENTS.md
   / decisions recorded on the issues); undo manual holds that are
   stale, answered, or unjustified on the record; clear stale blocker
   references (a blocker verified closed/completed is a satisfied
   dep — omit it, and if it was the only blocker, unblock); formalize
   prose-only blockers into `blocked_deps` entries
   (`recorded_by: queue-reprio-pass2`) for blockers verified still
   open; run `fleet queue repair` on suspected packet defects (a
   `no_defects` verdict means the defect is stale — proceed on the
   merits). Genuinely-user questions collect into ONE short list on
   the tracking issue, each with its specific question — never
   manufacture entries the standing statements already answer.
   - *Mis-typed / mis-sized packets:* cancel and re-packetise as
     verify-and-route with the corrected instruction in the packet
     text. **Known pattern — the terminal-key wall:** the canonical
     triage key is held by the terminal (done) bulk-intake task, the
     broker refuses a second holder ("work already queued"), `revive`
     does not apply to `done` tasks, and key suffixes other than the
     designed families fail validation. The working vehicle is the
     **second-pass family**: `repo-health:v1:second-pass:<repo>:<n>:pass-<k>`
     with payload kind `second_pass` and a `payload.review` object
     whose `review.pass` matches the key (validator-enforced). Other
     family keys: `fix` → `repo-health:v1:fix:<repo>:issue:<n>` (the
     segment is `fix`, not `small_fix`), `slice:<repo>:issue:<n>:slice-<k>`,
     `pr:<repo>:pr:<n>`. Standard timeboxes: 20 triage / 45 standard.
   - *Deferred work:* a recorded future date becomes a second-pass or
     triage packet parked with `not_before` (SQL on the queue row —
     `park` and `revive --not-before` do not cover unclaimed/terminal
     tasks).
4. **Reorder** every remaining actionable item: blocked items sit
   strictly BEHIND their blockers (one band worse than the blocker's
   implementing task, iterated to fixpoint); biggest unlockers pushed
   up; focus order for features **id → atlas → fleet → pm stack →
   mcpx**; designs prioritized (they create issues and unlock
   implementations); tweaks / improvements / bugs rise ONLY with a
   recorded reason (important / unblocking / concrete other — the
   reason goes in the packet's `priority_reason` and the pass
   record). Existing `priority/*` GitHub labels and the `expedite`
   label are honoured as recorded reasons (2026-11-24: expedite
   issue #12228's task stays at its packet-recorded band 30 — do not
   let a bulk reorder silently drop an expedited item). Apply
   queue-row-first; then sync GitHub `priority/*` labels for moved
   items whose issues carry one (the repo's label set may be coarser
   than the scale — map to the nearest label and record the mapping).
   - *Held-adjacent caution:* an unlocker whose implementing tasks
     are user-held gets no push-up (2026-11-24: #1080, 6 unlocks,
     both implementers held).
   - *Re-block vignette:* unblocking a stale hold is safe even when
     a real gate remains — the claiming lane's gate check re-blocks
     precisely with formal deps (2026-11-24: task 15982, unblocked
     when #9241 closed; a lane re-blocked it within the hour on
     #9503 + #9178 with evidence).
5. **Pass-2 record** on the tracking issue: per-band before/after,
   totals, cleanup outcomes by group, the coverage table (floor /
   covered / newly packetised / exceptions by kind, per repo), the
   held list, the Drew list, error-fix evidence, transferred
   remainders (work owned outside the queue, named with its owner).

## Pass 3 — band audit

Re-pull the full actionable set fresh (it will have moved — lanes
work during the audit; on 2026-11-24 three tasks unblocked in pass 2
were already done and a fourth claimed before pass 3 started). Check
every band against the same rules, including every pass-2 creation;
fix outliers (queue row first, label sync after). Re-run the coverage
cross-reference on a fresh estate pull: issues whose items completed
during the programme, and issues opened during it, appear as
leftovers — packetise them in this pass (2026-11-24: 110 leftovers).
Record final per-band counts and programme totals on the tracking
issue. The audit is done when a fresh pull shows: no off-scale
priorities, no blocked item ahead of its blocker, no held item moved,
every open issue covered or excepted by name.
