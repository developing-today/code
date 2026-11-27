# Census D — Fleet queue ledger: 24h terminal-event census

**Window:** 2026-11-26T04:10:00Z → 2026-11-27T04:10:00Z (24h ending Thu 2026-11-26 22:10 CST).
**Sources (read live for this census):** the fleet-queue **D1 store**, read through the
d1-gateway libSQL endpoint the queue broker itself uses
(`https://d1-gateway.developing-today.workers.dev/db/fleet-queue`, token by name only) —
tables `tasks`, `events`, `repo_task_lane_ledger`. Cross-check: `fleet queue health`
run on the laptop at window end.
**Source caveat:** the older Turso copy of fleet-queue is a frozen mirror — its newest
task completion is 2026-11-26T13:18:38Z — so nothing below rests on it. The lane
ledger (`repo_task_lane_ledger`) has effectively no coverage of this window in either
store (D1: 1 executed row in window; Turso: executions frozen at 2026-11-23T00:36:36Z),
so worker attribution comes from the `events` table, not the ledger. (This also means
the lane-share producer's recent inputs are stale; its identical daily line is an
artifact of that, flagged here as a finding.)

## 1. Terminal events by queue

From `events` (kind ∈ completed / failed / cancelled), joined to `tasks` for the queue:

| Queue                         | Completed | Failed | Cancelled | Terminal events |
| ----------------------------- | --------- | ------ | --------- | --------------- |
| repo-health-code-pilot        | 1,533     | 13     | 133       | 1,679           |
| repo-health-opencode-pilot    | 76        | 5      | 7         | 88              |
| repo-health-openchamber-pilot | 46        | 2      | 8         | 56              |
| repo-health-nix-pilot         | 50        | 4      | 8         | 62              |
| repo-health-zettel-pilot      | 1         | 0      | 0         | 1               |
| scratch-recovery-2026-11-26   | 0         | 0      | 1         | 1               |
| **Fleet total**               | **1,706** | **24** | **157**   | **1,887**       |

No other queue recorded a terminal event in the window.

**Reconciliation with the tasks table:** 1,850 task rows carry `status='done'` with
`completed_at` in the window — 144 more than the 1,706 `completed` events. The gap is
exactly accounted for: 143 of those 144 tasks carry a `staleness_autocomplete` event
in the window instead of a `completed` event (the #14268-era autocomplete path stamps
the task done and records its own event kind). Per queue the two views reconcile to
the task: code-pilot 1,533 + 101 = 1,634 done rows; opencode 76 + 17 = 93;
openchamber 46 + 14 = 60; nix 50 + 12 = 62; zettel 1. In the D1 store, `completed_at`
is stamped only on done tasks, so failed/cancelled timing can only be read from
events — the events view above is the census of record.

**Cross-check vs `fleet queue health`:** health's `terminal_events_24h` equals
completed + failed events (it does not count cancellations): opencode 81 = 81 ✓,
openchamber 48 = 48 ✓, nix 54 = 54 ✓, code-pilot 1,544 vs 1,546 counted here (Δ2;
three code-pilot completions landed in the final ten minutes of the window, after
the health snapshot's anchor). Failed: opencode 5 ✓, openchamber 2 ✓, nix 3 vs 4
counted here, code-pilot 12 vs 13 counted here — the health rollup undercounts
failures by one on two queues; the events table is authoritative and every one of
the 24 is itemized in §3.

## 2. Completions by worker

Actor on the `completed` event (1,706 total; claim volume from `claimed` events in
the same window shown for context):

| Worker                  | Completed | Share | Claims (events) |
| ----------------------- | --------- | ----- | --------------- |
| repo-worker-code        | 1,241     | 72.7% | 2,275           |
| archer                  | 148       | 8.7%  | 439             |
| xaltha                  | 63        | 3.7%  | 165             |
| repo-worker-openchamber | 42        | 2.5%  | 114             |
| agent-next-24-local     | 29        | 1.7%  | 57              |
| agent-next-23-local     | 26        | 1.5%  | 52              |
| agent-next-21-local     | 24        | 1.4%  | 36              |
| agent-next-25-local     | 22        | 1.3%  | 37              |
| agent-next-19-local     | 20        | 1.2%  | 40              |
| agent-next-26-local     | 16        | 0.9%  | 41              |
| agent-next-22-local     | 14        | 0.8%  | 37              |
| lane11-claude           | 9         | 0.5%  | —               |
| claude-lane4            | 9         | 0.5%  | 17              |
| lane-8-claude           | 8         | 0.5%  | 19              |
| lane5-cowork            | 6         | 0.4%  | 26              |
| lane-2-cloud            | 4         | 0.2%  | —               |
| claude-lane-9           | 4         | 0.2%  | 24              |
| agent-next-27-local     | 4         | 0.2%  | 12              |
| repo-worker-nix         | 3         | 0.2%  | —               |
| oc-code-02              | 3         | 0.2%  | —               |
| agent-next-20-local     | 3         | 0.2%  | 17              |
| oc-code-04              | 2         | 0.1%  | —               |
| oc-code-03              | 2         | 0.1%  | 20              |
| oc-code-01              | 1         | 0.1%  | —               |
| felix                   | 1         | 0.1%  | 22              |
| conductor-cycle         | 1         | 0.1%  | —               |
| any-o                   | 1         | 0.1%  | —               |

Reading the identities: `repo-worker-code` is the shared claim identity of the Muse
work-loop / repo-worker lane pool (its individual lanes appear as `lane_id` inside
event payloads, e.g. oc-code-_, any-_, code-*); `archer` is the main agent completing
directly; the nine `agent-next-*-local` identities are the laptop Codex lanes (158
completions combined, 9.3%); `xaltha` is Drew's Claude Code project-thread seat;
`repo-worker-openchamber` / `repo-worker-nix` are the other repo-worker pools.

## 3. Every failed task in the window (24)

All 24 `failed` events, grouped by cause. Errors are quoted from the task record's
`last_error` / the failed event's payload. "Status now" is the task's status at
census time — several "failures" were later revived and finished, which matters for
how the count should be read: a `failed` event here is a lane's recorded give-up on
that attempt, not always a terminal death.

### Group A — Out-of-scope cross-repo claims (9)

The broker served a task from another repo's pilot queue to a lane scoped to
developing-today/code; the lane failed it untouched and said so. Eight of nine name
lane `any-n`. This is a routing defect, not lane error: the work was never attempted.

**Task 25589** · nix-pilot · failed 2026-11-27T01:27:20Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: dezren39/darwin-nix#2038 — https://github.com/dezren39/darwin-nix/issues/2038
- Title: Triage dezren39/darwin-nix#2038: agents dispatch: task prompts are fetched unpinned from developing-today/code main and sent to an agent session unverified
- Why (quoted): "Out-of-scope claim: this scheduled lane is scoped to developing-today/code, but fleet agent next --repo developing-today/code returned dezren39/darwin-nix#2038 from repo-health-nix-pilot. Existing triage on #2038 was refreshed 2026-11-27 by prior lane; no action taken in this repository."

**Task 25559** · openchamber-pilot · failed 2026-11-27T01:53:26Z · worker `repo-worker-code` · lane `any-n` · attempts 2/1000 · status now: failed

- Item: developing-today-forks/openchamber#2671 — https://github.com/developing-today-forks/openchamber/issues/2671
- Title: Triage developing-today-forks/openchamber#2671: fix(web): sessions-unread baseline writes through a predictable non-exclusive tmp path, following a planted symlink
- Why (quoted): "Out-of-scope claim: this scheduled any-n run is scoped to developing-today/code (agent next --repo developing-today/code), but the broker returned developing-today-forks/openchamber#2671 from repo-health-openchamber-pilot. No action taken; an openchamber-scoped lane should pick this up."

**Task 26012** · opencode-pilot · failed 2026-11-27T01:54:18Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: developing-today-forks/opencode#2875 — https://github.com/developing-today-forks/opencode/issues/2875
- Title: Triage developing-today-forks/opencode#2875: Merge-forward upstream dev into fork main (carry strategy from #2290 slice 2)
- Why (quoted): "Out-of-scope claim: this scheduled any-n run is scoped to developing-today/code (agent next --repo developing-today/code), but the broker returned developing-today-forks/opencode#2875 from repo-health-opencode-pilot. No action taken; an opencode-fork-scoped lane should pick this up."

**Task 24433** · nix-pilot · failed 2026-11-27T01:59:37Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: dezren39/darwin-nix#2240 — https://github.com/dezren39/darwin-nix/issues/2240
- Title: ci-fix: flake checks build failing on fix/2236-lsof-diagnostic-exit1
- Why (quoted): "Out-of-scope claim: this scheduled any-n run is scoped to developing-today/code (agent next --repo developing-today/code), but the broker returned dezren39/darwin-nix ci-fix task 24433 from repo-health-nix-pilot. No action taken; a nix-scoped lane should pick this up."

**Task 25584** · opencode-pilot · failed 2026-11-27T02:16:26Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: developing-today-forks/opencode#2795 — https://github.com/developing-today-forks/opencode/issues/2795
- Title: Add cursor pagination to the route-attempt read API (#2795 slice 4)
- Why (quoted): "Out-of-scope claim: this scheduled any-n run is scoped to developing-today/code, but the broker returned developing-today-forks/opencode task 25584. No action taken; an opencode-fork-scoped lane should pick this up."

**Task 24405** · opencode-pilot · failed 2026-11-27T02:43:42Z · worker `repo-worker-code` · lane `any-n` · attempts 2/4 · status now: failed

- Item: developing-today-forks/opencode#2844 — https://github.com/developing-today-forks/opencode/pull/2844
- Title: Shepherd ready PR #2844: feat: propagate Fleet session context
- Why (quoted): "Out-of-scope claim: this scheduled any-n run is scoped to developing-today/code, but the broker returned developing-today-forks/opencode PR shepherd task 24405. No action taken; an opencode-fork-scoped lane should pick this up."

**Task 21608** · code-pilot · failed 2026-11-27T02:53:59Z · worker `repo-worker-code` · lane `any-n` · attempts 12/1000 · status now: failed

- Item: developing-today-forks/opencode#13879 — https://github.com/developing-today-forks/opencode/issues/13879
- Title: AUDIT-7 Lane 4 merged-PR audit slice: developing-today-forks/opencode batch 3/5 (250 PRs)
- Why (quoted): "Out of scope for developing-today/code any-n lane: task audits developing-today-forks/opencode batch 3/5. Returned without acting on the other repository."

**Task 24480** · opencode-pilot · failed 2026-11-27T03:01:09Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: developing-today-forks/opencode#2870 — https://github.com/developing-today-forks/opencode/issues/2870
- Title: Triage developing-today-forks/opencode#2870: fix(core): out-of-range session timestamps still brick reads via fromRow
- Why (quoted): "Out of scope for developing-today/code any-n lane: task triages developing-today-forks/opencode#2870. Existing progress also records that lane 10 already owns the issue; returned without acting on the other repository."

**Task 25472** · opencode-pilot · failed 2026-11-27T03:39:46Z · worker `repo-worker-code` · lane `any-n` · attempts 1/1000 · status now: failed

- Item: developing-today-forks/opencode#2677 — https://github.com/developing-today-forks/opencode/issues/2677
- Title: Implement remaining upstream-sync follow-ups on opencode#2677
- Why (quoted): "Claim targets developing-today-forks/opencode, outside the scheduled lane scope of developing-today/code; no repository work performed."

### Group B — Disposition-gate and packet-contract rejections (8)

The lane did the work, or triaged correctly, but no truthful completion was
available: the broker's disposition gate rejected the outcome as
`invalid_no_work`, or the task packet itself was defective at import
(kind=small_fix declared over triage-only acceptance, a misclassified
reconciliation import). The failure lands on the task; the defect sits in the
packet or the gate. Five of the eight say so in nearly the same words — the
gate cannot accept "verified: no work exists" as a completion.

**Task 24203** · code-pilot · failed 2026-11-26T19:18:14Z · worker `repo-worker-code` · lane `oc-code-02` · attempts 2/1000 · status now: failed

- Item: developing-today/code#12460 — https://github.com/developing-today/code/issues/12460
- Title: Triage developing-today/code#12460: epic(fleet): maintenance
- Why (quoted): "Deterministic no-op repeat triage: issue #12460 already has the current unchanged verdict; issue remains an intentionally open canonical maintenance epic with no linked PR or live same-issue follow-up. verify-disposition returns invalid_no_work, so completion would be rejected. No repeated issue comment posted."

**Task 25050** · code-pilot · failed 2026-11-26T23:55:58Z · worker `repo-worker-code` · lane `oc-code-01` · attempts 1/1000 · status now: failed

- Item: developing-today/code#13793 — https://github.com/developing-today/code/issues/13793
- Title: Triage developing-today/code#13793: SAML S1 — core: samael module, signed metadata + Web Browser SSO on the minimal IdP
- Why (quoted): "Packet defect: task payload declares kind=small_fix/fix dedupe key, but title, scope, acceptance and existing same-day triage require verification only for the effort/l SAML S1 feature. No small fix was made; issue #13793 comment https://github.com/developing-today/code/issues/13793#issuecomment-7205990985 records the verdict (PR #14640 merged, #14625 open/draft, SSO integration remainder stays on #13793). Completion rejected because fix-class requires merged PR/commit/verified-closure evidence. Correct disposition is a triage packet or valid feature-slice packet, not falsely claiming a fix."

**Task 25096** · code-pilot · failed 2026-11-27T00:02:13Z · worker `repo-worker-code` · lane `oc-code-01` · attempts 1/1000 · status now: failed

- Item: developing-today/code#15002 — https://github.com/developing-today/code/issues/15002
- Title: Triage developing-today/code#15002: C6 child — automatic cloud-reset rule at 100% quota (blocked: Drew FDR ruling)
- Why (quoted): "Reconciliation import packet is misclassified: existing triage task 24516 already records the decision to reject unattended cloud-reset automation and retain the human-confirmable reset under #9574/#15018, with no sourced owner ruling. The import incorrectly declares kind=small_fix/fix although this is a design decision/rejected candidate, and duplicate triage generated another follow-on. Issue #15002 remains OPEN despite its existing decision note saying to close as rejected/superseded; the Fleet close command refused closure because its required reason/successor syntax is not clear. Do not claim a fix or closure. Existing decision evidence: https://github.com/developing-today/code/issues/15002#issuecomment-7205990985; merged confirmation path PR #15097. Follow up with a correctly classified triage/closure operation and prevent duplicate follow-on."

**Task 25074** · code-pilot · failed 2026-11-27T00:02:37Z · worker `repo-worker-code` · lane `oc-code-02` · attempts 1/1000 · status now: failed

- Item: developing-today/code#12921 — https://github.com/developing-today/code/issues/12921
- Title: Triage developing-today/code#12921: id: implement capability-change notices — client baseline store, connect-stage comparison, notice surface (#5959 design)
- Why (quoted): "Packet defect: coverage import labels this as kind=small_fix but its acceptance is triage-only. Correct disposition is retain #12921 open and route to existing #14050/task 23776. Completion rejected by the broker because no merged PR or verified-closure evidence exists; no implementation or tests were claimed. See issue comment 7206020633 and follow-up linkage #15004 on #14050."

**Task 25142** · code-pilot · failed 2026-11-27T00:06:31Z · worker `repo-worker-code` · lane `oc-code-01` · attempts 1/1000 · status now: failed

- Item: developing-today/code#10974 — https://github.com/developing-today/code/issues/10974
- Title: Triage developing-today/code#10974: fleet gh step 2.3: decide the fleet gh library surface (crate) after steps 2.1/2.2
- Why (quoted): "Could not complete due to result contract mismatch despite running required fleet queue verify-disposition 10974 --repo developing-today/code --json. Verified readback returned issue OPEN, pulls [], verdict invalid_no_work, follow_up null; completion requires disposition fields, but the record says follow_up must be an object, while the actual next step is a condition: revisit only after #10972 and #10973 close. Existing triage verdict is current and no new packet is warranted. Added fresh issue comment https://github.com/developing-today/code/issues/10974#issuecomment-7206146490. Requires queue disposition schema to support open triage with prerequisite follow-up, rather than falsely closing the issue."

**Task 25901** · code-pilot · failed 2026-11-27T01:53:15Z · worker `repo-worker-code` · lane `code-g` · attempts 1/1000 · status now: failed

- Item: developing-today/code#10395 — https://github.com/developing-today/code/issues/10395
- Title: Triage developing-today/code#10395: Design mill pass 6: fork upkeep — upstream merges + feature-gap analysis (OpenCode, OpenChamber)
- Why (quoted): "Disposition verified: existing triage is current and no new work is warranted. Broker rejects completion for this imported small_fix as invalid_no_work; open parent #10389 is not recognized as explicitly linked, and adding a duplicate packet/comment would violate the triage verdict. Record task as deterministic import/classification mismatch rather than manufacture work."

**Task 25929** · code-pilot · failed 2026-11-27T01:56:43Z · worker `repo-worker-code` · lane `oc-code-04` · attempts 1/1000 · status now: failed

- Item: developing-today/code#8886 — https://github.com/developing-today/code/issues/8886
- Title: Triage developing-today/code#8886: Epic: fleet GitHub Apps — permission parity, org coverage, registry
- Why (quoted): "Disposition gate rejects open issue #8886 with no linked work as invalid_no_work; triage is already recorded and correct, but the packet requires triage-only completion without an actionable follow-up or merged fix. No real follow-up work exists to enqueue."

**Task 25153** · code-pilot · failed 2026-11-27T02:21:39Z · worker `repo-worker-code` · lane `oc-code-05` · attempts 1/4 · status now: failed

- Item: developing-today/code#15665 — https://github.com/developing-today/code/pull/15665
- Title: Shepherd ready PR #15665: chore(deps): bump picomatch, tailwindcss, @parcel/watcher and sass in /pkgs/identity/web
- Why (quoted): "PR #15665 was closed with a recorded salvage reason, but the completion broker rejects this issue-less pr_shepherd task: fix-class requires an issue-based verified disposition even though the packet names only PR #15665. No truthful issue readback can be supplied. Salvage note: https://github.com/developing-today/code/pull/15665#issuecomment-7207828163"

### Group C — `fleet gh` issue-close defect (2)

Triage verdicts were recorded and the work was done, but the close verb rejected
valid `--reason` values ("Please give a reason" despite one being supplied; the
closure endpoint likewise rejects `--reason cancelled`). Both lanes refused to
bypass Fleet with a direct GitHub write and failed the task instead. This is the
known close-verb defect class tracked on code#9221, still live.

**Task 25147** · code-pilot · failed 2026-11-27T00:27:51Z · worker `repo-worker-code` · lane `oc-code-03` · attempts 1/1000 · status now: failed

- Item: developing-today/code#11166 — https://github.com/developing-today/code/issues/11166
- Title: Triage developing-today/code#11166: Scheduled Palette — developing-today/code
- Why (quoted): "Triage verdict is recorded and stale claim labels removed, but the Fleet closure endpoint rejects `--reason cancelled` with its policy error. Did not bypass Fleet with direct GitHub write; #11166 remains open."

**Task 25026** · nix-pilot · failed 2026-11-27T00:33:42Z · worker `repo-worker-code` · lane `any-o` · attempts 1/1000 · status now: failed

- Item: dezren39/darwin-nix#1890 — https://github.com/dezren39/darwin-nix/issues/1890
- Title: Triage dezren39/darwin-nix#1890: Scheduled Palette — dezren39/darwin-nix
- Why (quoted): "Triage verdict/comment recorded (https://github.com/dezren39/darwin-nix/issues/1890#issuecomment-7206340141), but cannot close: every fleet gh issue close invocation supplied a valid --reason and still fails locally with 'Please give a reason'; source parser accepts these flags, indicating installed binary/runtime mismatch. Issue remains open; do not bypass fleet gh or use raw gh."

### Group D — Scope guard: Drew-authored PR hard stop (2)

Shepherd tasks whose PR was authored by Drew himself (dezren39). Lane protocol
treats Drew-authored PRs as a hard stop, so both lanes failed the task as
transient-for-requeue rather than touch the PR. Task 24408 has since been
cancelled; task 24413 has since been parked.

**Task 24408** · openchamber-pilot · failed 2026-11-26T21:49:48Z · worker `repo-worker-openchamber` · lane `not-code-d` · attempts 2/4 · status now: cancelled

- Item: developing-today-forks/openchamber#2660 — https://github.com/developing-today-forks/openchamber/pull/2660
- Title: Shepherd ready PR #2660: fix(web): concurrent project-actions writes lose one write — atomic mutate inside project lock (#2605)
- Why (quoted): "halt: task references OpenChamber PR authored by Drew (dezren39), but OpenChamber work items are out of scope under Drew direct ownership"

**Task 24413** · nix-pilot · failed 2026-11-26T21:51:42Z · worker `repo-worker-openchamber` · lane `not-code-d` · attempts 2/4 · status now: parked

- Item: dezren39/darwin-nix#2227 — https://github.com/dezren39/darwin-nix/pull/2227
- Title: Shepherd ready PR #2227: 🎨 Palette: improve brew-repair failure guidance
- Why (quoted): "PR #2227 is Drew-authored (dezren39); lane protocol treats Drew-authored PRs as hard stop. No review findings exist; mergeability is CLEAN/MERGEABLE against main (19ac9cae). Parking per lane rule — task failed transient for requeue."

### Group E — Real work failures (3)

The only three failures where the work itself failed or was genuinely blocked:
an undeliverable implementation slice, a PR that failed real Darwin acceptance
tests, and an implementation blocked by a dirty shared checkout plus an
unresolved execution-risk hold. Two of the three tasks (24187, 25657) were later
revived and are `done` now; 23951's PR remains unmerged on its evidence.

**Task 24187** · code-pilot · failed 2026-11-26T15:25:45Z · worker `repo-worker-code` · lane `code-i` · attempts 2/1000 · status now: done

- Item: developing-today/code#14400 — https://github.com/developing-today/code/issues/14400
- Title: Triage developing-today/code#14400: broker-tls BT-2: trust-bundle consumption for broker clients (versioned, two-phase rollover, ack in health)
- Why (quoted): "Unable to deliver a verified implementation slice in this run. Issue #14400 remains open; PR #15137 covers only the trust store. Handshake and stale-client health reporting remain, and #14403 task 23905 is parked on #14400, creating a dependency cycle. Details: https://github.com/developing-today/code/issues/14400#issuecomment-7195110312. Retry as feature slice; do not mark solved."

**Task 23951** · code-pilot · failed 2026-11-26T21:11:38Z · worker `repo-worker-code` · lane `oc-code-02` · attempts 3/1000 · status now: failed

- Item: developing-today/code#15550 — https://github.com/developing-today/code/pull/15550
- Title: Merge-watch developing-today/code#15550: recheck fix PR for issue #15407
- Why (quoted): "PR15550 head a6d9f96 failed real Darwin acceptance in dezren39/fleet#45895786817 (go1.27.4 darwin/arm64): takeover_test.go:282 changed old child still running; takeover_test.go:373 scoped child PID10054->0. Evidence posted PR comment 7203473442 and checkpoint darwin-acceptance-failed. Not merged; issue15407 remains open. Continue correction on existing PR branch, not a fresh implementation."

**Task 25657** · code-pilot · failed 2026-11-27T02:22:52Z · worker `repo-worker-code` · lane `code-j` · attempts 2/1000 · status now: done

- Item: developing-today/code#9178 — https://github.com/developing-today/code/issues/9178
- Title: Implement #9178 §2 slice 2: audited manual claim at max_attempts
- Why (quoted): "Could not safely enter the implementation phase: task checkout is a shared clone with extensive unrelated local changes, while the only candidate worktree for this exact slice is covered by the issue's unresolved execution-risk HOLD. The clarification prompt timed out; the current local origin/main is stale versus GitHub main. No source files were edited. Needs a fresh trusted isolated checkout before retry."

## 4. Cancellations (157 events)

All 157 carry `via: cancel_repo_task_turso` and a free-text or coded `reason` in
the event payload. By queue: code-pilot 133, nix-pilot 8, openchamber-pilot 8,
opencode-pilot 7, scratch-recovery-2026-11-26 1. By cancelling actor:
repo-worker-code 53, fleet-monitor 51, the nine agent-next-*-local Codex lanes 34
combined, lane11-claude 6, xaltha 6, others 7.

Cause classes (verbatim reasons bucketed; the single most common verbatim reason
is `issue-closed`, 40×):

| Cause                                                                     | Count | Share |
| ------------------------------------------------------------------------- | ----- | ----- |
| Duplicate / redundant follow-on packet                                    | 75    | 48%   |
| `issue-closed` (coded reason)                                             | 40    | 25%   |
| Work already done / issue closed (free-text)                              | 10    | 6%    |
| Superseded by another task or PR                                          | 7     | 4%    |
| False-premise / default-spawned follow-on (from the unbucketed remainder) | ~13   | ~8%   |
| Misclassified packet / packet defect                                      | 5     | 3%    |
| User-owned / escalated to Drew                                            | 3     | 2%    |
| Dispatch launch failure (OpenCode session prompt unreachable)             | 2     | 1%    |
| Transport-recovery probe cleanup                                          | 1     | 1%    |
| Other free-text                                                           | ~1    | —     |

The story of the cancellation column is the triage-completion machinery itself:
a completed triage task auto-creates a follow-on packet, and in roughly **85% of
cancellations** that packet was then cancelled as a duplicate, a false premise,
already-done work, or a superseded duplicate of a live task. Examples, verbatim:
task 24181 — "Duplicate triage follow-on: task 24178 is the verified live
feature-slice coverage for #15434"; task 25378 — "Spawned by mistake from triage
task 24925 (default follow_on outcome)"; task 25916 — "Erroneous automatic
follow-on spawned by completed triage task 25628." Cancellations are queue
hygiene, not lost work — but each one is a lane-cycle spent creating and then
retiring a packet nobody needed.

## 5. Claim-evaporations (400 events, 309 tasks)

A claim-evaporation = a `lease_expired` event: a task was claimed, the lease
lapsed with no complete/park/block, and the task fell back to queued. (Plus 2
`lease_expired_at_cap` events.) Repeat distribution: 232 tasks expired once, 65
twice, 10 three times, 2 four times.

**By worker (events / distinct tasks):** repo-worker-code 256 / 222, archer
42 / 42, xaltha 32 / 26, felix 7 / 6, dispatch-opencode 7 / 7,
agent-next-19-local 5 / 5, repo-worker-openchamber 4 / 4, claude-lane-9 4 / 4,
agent-next-25-local 4 / 3, agent-next-20-local 4 / 4, and a long tail of 1–3.
Evaporation is overwhelmingly a repo-worker-code (Muse lane pool) phenomenon —
64% of events — which matches where the claim volume is (2,275 of 3,540 claims).

**By hour (UTC), and correlation with the day's known outages:**

| Hour (UTC)  | Events   | Note                                              |
| ----------- | -------- | ------------------------------------------------- |
| 10:00       | **103**  | Broker recovery sweep — see below                 |
| 11:00–16:00 | 4–6 / hr | Quiet baseline                                    |
| 17:00       | 33       | US-day work-loop ramp                             |
| 18:00       | 30       |                                                   |
| 19:00       | 35       |                                                   |
| 20:00       | 39       | Contains the ~20:19–20:24Z broker outage          |
| 21:00       | 13       |                                                   |
| 22:00       | 26       |                                                   |
| 23:00       | 24       |                                                   |
| 00:00       | 26       | Contains the 00:55–01:15Z heal-failure window     |
| 01:00       | 16       |                                                   |
| 02:00       | 10       |                                                   |
| 03:00       | 14       | Contains bravo 2052's ~03:10Z broker-down run end |
| 04:00       | 1        | (to 04:10)                                        |

- **The 10:00Z spike (103 events, 26% of the day's evaporations) is the laptop
  outage ending.** amd-6 was off the tailnet from ~2026-11-25T14:04Z and the
  queue broker recovered at 2026-11-26T10:45:24Z (issue #15131 closed then).
  Every lease that had lapsed during the ~20h outage was swept in one pass when
  the broker came back. These are outage casualties, not lane misbehavior.
- **The 17:00–00:00Z plateau (~25–39/hr against a 4–6/hr baseline)** is the
  work-loop's peak claiming day running over a flaky broker tunnel: lanes ended
  runs without completing or parking (the recurring silent-lane pattern), and
  the tunnel drops made some completions undeliverable. The three named
  in-window outage moments (20:19–20:24Z, 00:55–01:15Z, ~03:10Z) sit inside
  hours that are elevated but show **no distinct spike** at hourly grain — the
  plateau, not the individual drops, is the measurable effect.
- **Evaporation was churn, not loss.** Of the 309 distinct evaporated tasks,
  their status at census time: 186 done (60%), 62 parked, 29 claimed again,
  19 blocked, 7 failed, 6 cancelled. Most evaporated work was re-claimed and
  finished by someone; the cost is duplicated effort and claim/label repair,
  not dropped items.

## 6. Findings

1. **Failure is mostly routing and paperwork, not work.** Of 24 failed events:
   9 out-of-scope cross-repo claims (broker served another repo's task to a
   code-scoped lane — the `fleet agent next --repo` path does not hold its repo
   scope), 8 disposition-gate/packet-contract rejections, 2 the known `fleet gh`
   close defect, 2 Drew-authored-PR hard stops, and only 3 real work failures —
   2 of which later completed anyway. True unrecovered work failure in 24h: 1
   task (23951, PR #15550's Darwin acceptance failure, evidence on the PR). **The completion contract cannot say "verified: nothing to do."** Eight
   failures share one root: a lane verifies that no work exists (or that the
   packet was mis-built) and the broker rejects the completion as
   `invalid_no_work`, forcing a `failed` record. This manufactures failure
   statistics and burns the task's terminal slot. **Auto-follow-on churn dominates cancellations** (~85%): triage completions
   spawn follow-on packets that are then cancelled as duplicates or false
   premises — the same defect family the delta 1922 run flagged (reconciliation
   re-serving already-triaged homes; task 23230 re-served 3× the same day). **Claim-evaporations cluster on infrastructure, not lanes:** one recovery
   sweep after the laptop outage (103) plus a day-long plateau under a flaky
   broker tunnel, concentrated in the highest-volume claim identity. **The lane ledger is dark.** `repo_task_lane_ledger` recorded 1 execution in
   this window (D1) and its Turso copy's executions stop at 2026-11-23. Any
   lane-share or worker-attribution reporting that reads the ledger — including
   the stewardship cycle's daily lane-share line — is currently running on a
   frozen source; this census used the events table instead. **Health counters are near-true but not exact:** `fleet queue health`'s
   terminal count silently excludes cancellations, and its failed count
   undercounts by one on two of four pilot queues against the events table.

_Method note:_ all counts were taken from the live D1 store in one pass after
window close (2026-11-27T04:11Z+); task "status now" fields are as of that read
and will keep moving as parked/failed tasks are revived.
