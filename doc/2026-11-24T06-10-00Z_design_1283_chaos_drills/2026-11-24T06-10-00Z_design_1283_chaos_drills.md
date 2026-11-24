# Chaos and rotation drills for the PM stack — scheduled game-days

- Source issue: developing-today/code#1283 (part of epic #1184, PM stack coherence)
- Tracking: #13873 (AUDIT-7 Lane 6, DESIGN-GAPS); umbrella #13871
- Queue task: 21671 (slice 1 of #1283, owned paths `doc/`)
- Status: design of record, design-only. No implementation in this record.
- Date: 2026-11-24 (UTC 06:10)

## 1. Problem and context

The PM stack fails silently. Each of these has presented as "everything is fine" at least once:

- **Dead collector.** The fleet-feed collector stops and the pages keep rendering the last good data without a visible staleness signal.
- **Aged feed.** The feed is published but old. Freshness is only visible if someone reads a timestamp.
- **Rejected orchestration section.** The aggregator drops a section the orchestrator publishes. Evidence: #1110 (closed). `isOrchestrationSection` required `provenance` to be a string while the publisher emitted an object. `/api/orchestration.json` reported "absent", `/today` fell back to derived mode, and `/attention` showed "Needs Drew (0)" while the published file carried 11 NEEDS DREW items. Every footer claimed the orchestrator had published nothing.
- **Down service presented as fine.** Epic #1184 records that every service was up at review time, so the failure there was coherence rather than uptime. A service that is actually down has not been shown to produce a visible degraded state on every page that depends on it.

Health pages, staleness banners and watchdogs exist or are being built (#1184 children). They have only been exercised by accident. #1283 asks for a deliberate, scheduled test: inject one known fault, check that the expected signals fire, restore, and record the result.

**Evidence bounds.** This record is grounded in the bodies of #1283, #1110 and #1184 as read on 2026-11-24, and in the #13873 scope. It did not inspect the aggregator, orchestrator or health-page source code in this pass. Where a claim depends on current code (for example, which banner a page shows), it is stated as a requirement for the implementation slices to verify, not as a fact about today's code. The #1110 failure is a documented past instance; whether the current code has the same shape is not established here.

## 2. Goals and non-goals

Goals:

- G1. A catalogue of drills, each injecting exactly one named fault and naming the signals it expects: at minimum collector-kill, feed-aging and service-drop.
- G2. Each drill verifies each expected signal (degraded banner, watchdog entry in NEEDS ORCHESTRATOR, health page state) and records pass or fail per signal.
- G3. Drills run on demand and on a cadence, in a declared maintenance window.
- G4. A passed drill posts its result as a comment on the tracking issue. A failed signal files a new issue under epic #1184 with the drill evidence attached.
- G5. Every drill restores all touched state. The restore is verified by a post-drill health read, not by assumption.
- G6. Drills are interlocked: never two faults at once, no overlap with another drill, and no run while a real-incident flag is set.

Non-goals:

- NG1. No random or unbounded chaos. Every fault is named in the catalogue and bounded in time and scope.
- NG2. No fault injection on production data. Drills touch only the feed path and service processes they name, and never delete or rewrite stored content.
- NG3. No automatic remediation beyond restoring the drill's own change. Fixing a failed signal is human or agent work through the filed issue.
- NG4. No new monitoring stack. Drills read the signals the stack already exposes.
- NG5. Not a load or performance test.

## 3. Alternatives

- **A0. Do nothing.** Rely on accidental discovery, as #1110 was found during a `/program` build. The cost is that each silent-failure class recurs until someone looks. This is the baseline all other options are judged against.
- **A1. Manual runbook.** A written checklist that a person runs each month. It is cheap to write, but depends on memory and gives no machine-readable pass or fail, and it cannot be interlocked.
- **A2. Scheduled drill runner with a catalogue (chosen).** A small runner reads a drill catalogue, takes the interlock, injects one fault, reads the expected signals, restores, verifies the restore, and reports. It is the same runner for on-demand and scheduled runs.
- **A3. Continuous chaos (random faults in production).** Highest signal, but breaks G6 and NG2 and is not acceptable for a single-owner stack in a maintenance window.
- **A4. Unit and integration tests only, with mocked collector and feed.** Cheap and safe, but does not exercise the deployed banners, watchdogs or health states, which is the thing being tested. Kept as a complement in the verification plan, not as the drill mechanism.

Tradeoff summary: A2 costs one runner and a catalogue, and needs a maintenance window and a restore check. A1 is cheaper but fails G2, G4 and G6. A3 fails the safety goals.

## 4. Chosen approach

### 4.1 Drill catalogue

Each drill is a declarative entry. Fields:

- `id`, `title`, `fault` (exactly one), `inject` (the action), `expect` (list of signals), `window` (max duration), `restore` (inverse action), `owner` (named decider from §8).

Minimum set:

| Drill | Fault | Expected signals | Restore |
|---|---|---|---|
| D1 collector-kill | Stop the fleet-feed collector process for one cycle | Staleness or degraded banner on `/today`, `/attention`, `/program`; health page shows collector down or stale; watchdog entry in NEEDS ORCHESTRATOR | Restart the collector; confirm next publish |
| D2 feed-aging | Serve or pin a feed copy whose timestamp is older than the freshness threshold | Aged banner on the affected pages; health page shows feed age over threshold; watchdog entry | Release the pin; confirm feed timestamp advances |
| D3 service-drop | Stop one component service (for example the aggregator or one dependency container) | Down state on the health page for that service; degraded banner on every page that reads it; watchdog entry | Start the service; confirm health endpoint green |
| D4 section-reject (optional, second wave) | Corrupt one orchestration section field so the aggregator rejects it, using a copy, not the publisher | "Section rejected" state on the orchestration freshness card and `/attention`; watchdog entry | Remove the corrupt copy; confirm section accepted |

D4 directly targets the #1110 failure shape and is in scope for the catalogue, but is scheduled after D1–D3 are proven, because it needs a copy path that does not alter the publisher.

### 4.2 Fault injection

- Each drill runs one `inject` action against one named target. Targets are listed in the catalogue, never discovered at run time.
- Injection is done through the normal service control surface (process stop, pinned copy, or config switch). It must not edit stored feed content. D2 and D4 use a copy under the drill's own path.
- Every injection records a start timestamp and the restore action that undoes it, before the injection runs. A crash mid-drill therefore leaves a known restore path.

### 4.3 Signal verification

- For each expected signal: poll the named surface (banner text or element on the page, watchdog entry in NEEDS ORCHESTRATOR, health endpoint state) within a bounded deadline (default 2 polling intervals past the expected cadence, then a fail).
- Record one row per signal: `drill_id`, `signal`, `expected`, `observed`, `pass|fail`, `at`.
- A drill passes only if every expected signal passes. Partial results are recorded per signal, not collapsed.

### 4.4 Scheduling and on-demand runs

- Same runner for both. A scheduled entry names the drill, the cadence and the maintenance window.
- Cadence default: D1 weekly, D2 weekly, D3 monthly, offset so they never fall in the same hour (see interlock).
- The schedule is a versioned file under `doc/` or the repo's schedule registry, and any change to it updates the registry in the same PR (per the fleet schedule rule in AGENTS.md).

### 4.5 Interlocks

- **Single fault.** The runner holds one lock for the whole run. A second drill start fails fast with "drill in progress".
- **Real incident.** Before injecting, the runner reads the real-incident flag. If set, it refuses to start and records "skipped: incident". The flag's source is named in §8 Q2.
- **Overlap with schedule.** The lock covers scheduled and on-demand runs alike.
- **Abort.** If a restore fails or an unexpected alert fires, the runner stops injection, runs restore for everything already injected, and marks the run aborted.

### 4.6 Restore

- Each injection pairs with its restore. Restore runs in reverse order of injection.
- After restore, the runner does a post-drill health read of every surface it touched and requires the healthy state. A restore is verified, not assumed.
- A restore that does not verify marks the drill `restore-failed`, files an issue at high priority under #1184, and keeps the lock held until a human clears it. This is the only path that blocks subsequent drills.

### 4.7 Reporting

- Pass: one comment on #13873 with the drill id, timestamp, per-signal table and the post-drill health read.
- Fail on any signal: a new issue under epic #1184 titled `drill <id>: signal <name> did not fire`, with the per-signal table, the drill log and the health snapshots attached.
- Aborted or skipped: a comment on #13873 with the reason. No new issue for a skip caused by a real incident.

## 5. Dependency-ordered implementation slices

Each slice is sized for one lane or session and has its own acceptance criteria. Later slices depend on earlier ones as listed.

- **S1. Drill catalogue schema and validator.** Depends on nothing. Define the catalogue format from §4.1 and a validator that rejects drills with more than one fault, no restore, or unnamed signals. Acceptance: validator rejects a two-fault entry and an entry with no restore; accepts D1–D3 as written.
- **S2. Interlock and lock.** Depends on S1. Single-run lock, incident-flag read, and fail-fast on overlap. Acceptance: two concurrent starts yield one run and one "drill in progress"; incident flag set yields "skipped: incident".
- **S3. Signal reader.** Depends on S1. Read banner, watchdog entry and health state with bounded polling and per-signal result rows. Acceptance: against a fixture with the expected banner present, the reader returns pass; with it absent, fail, at the deadline.
- **S4. Injection and restore for D1 collector-kill.** Depends on S2, S3. First end-to-end drill. Acceptance: on a staging target, D1 runs, all signals pass, restore verifies by post-drill health read.
- **S5. D2 feed-aging and D3 service-drop.** Depends on S4. Acceptance: each runs once on staging with all signals passing and restore verified; abort path exercised by forcing a restore failure.
- **S6. Reporting: comment and auto-filed issue.** Depends on S4. Acceptance: a passing run posts one comment on #13873; a forced signal failure files one issue under #1184 with evidence attached and no duplicate on a re-run with the same drill id and signal.
- **S7. Schedule and registry entry.** Depends on S5, S6. Cadence set, registry updated in the same PR. Acceptance: the schedule appears in the registry; on-demand and scheduled runs share a code path.
- **S8. D4 section-reject.** Depends on S5 and on the #1110 fix path being settled. Acceptance: with a corrupt copy, the section-rejected state appears on the named surfaces; removing the copy restores acceptance.

Parallel-safe: S2 and S3 may run in parallel after S1. S6 may run in parallel with S5 once S4 is merged.

## 6. Rollout and rollback

Rollout:

1. Land S1 and S2 with no schedule. Only on-demand, dry-run mode, which validates and prints the plan without injecting.
2. Run S4 on staging. Repeat until three consecutive clean runs.
3. Enable D2 and D3 on staging.
4. Enable the schedule in a maintenance window on the deployed stack, first for D1 only, then D2 and D3 after two clean weeks.
5. Enable D4 only after S8 has run clean on staging.

Rollback:

- Disable the schedule entry (one-line change in the registry, same PR process). No data is written by the runner, so disabling it leaves the stack as it was.
- If a drill leaves the stack unhealthy, the restore path in §4.6 runs. If restore fails, the lock stays held and a human clears it. The runner has no automatic override.
- Revert the runner PR if the runner itself misbehaves. The catalogue and the health surfaces are not changed by the runner, so a revert does not require data repair.

## 7. Verification plan

Oracles, each named:

- **Catalogue validity.** Oracle: validator output on a fixed set of valid and invalid entries (S1).
- **Interlock.** Oracle: two-start race test yields exactly one running lock and one fail-fast (S2).
- **Signal reader.** Oracle: fixture pages with and without the expected banner, and a fixture health endpoint with each state, give the expected pass or fail (S3).
- **Injection and restore.** Oracle: post-drill health read matches the pre-drill snapshot for every touched surface (S4, S5). A mismatch is a failure, not a warning.
- **Reporting.** Oracle: comment or issue exists with the expected title, per-signal table and evidence links, and re-running the same drill id does not duplicate the issue (S6).
- **Live proof.** Oracle: a dated run record on staging for D1–D3 showing all signals pass and restore verified, linked from #13873.
- **Silent-failure catch.** Oracle: a deliberately broken signal (banner removed on staging) makes the drill fail and file an issue. This proves the drill can fail, not only pass.

Tests are example-based for the runner and property-based where the invariant is clear (for example, "no two faults in one run", and "restore order is the reverse of injection order"), per the testing doctrine.

## 8. Open questions

Each has a named decider and the default the design uses until the decider answers. Per FDR-0013 the design does not wait on the decider.

- **Q1. Maintenance window: which hours and which days?** Decider: Drew (owner). Default: a 30-minute window, Sunday 06:00–06:30 CST, for scheduled runs; on-demand runs anytime outside an incident.
- **Q2. Real-incident flag: what is the source and who sets it?** Decider: the epic #1184 owner for the PM stack. Default: a file named `incident.active` in the drill state location, set by hand or by the existing incident path; absent means no incident.
- **Q3. Freshness threshold for D2 and staleness banners.** Decider: the aggregator owner for the feed contract. Default: the threshold already in the aggregator for the staleness banner; if none is declared, the drill uses 2× the collector cadence.
- **Q4. Which service is D3's target?** Decider: the epic #1184 owner. Default: the aggregator, because every page depends on it, which maximises the signal.
- **Q5. Should D4 wait for the #1110 fix path to be final?** Decider: the owner of the orchestrator contract. Default: D4 is scheduled after S5; it runs against a copy and does not depend on which side of the #1110 fix is chosen.
- **Q6. Where does the schedule live: the fleet schedule registry or a drill-only file?** Decider: the repo maintainer. Default: the fleet schedule registry (`doc/fleet-schedule-registry.md`), since AGENTS.md requires every schedule to be listed there.
- **Q7. Who clears a `restore-failed` lock?** Decider: Drew. Default: a human with write access to the drill state, after reading the restore evidence; the runner never clears it.

## 9. Record, tracking and acceptance checklist

Record:

- [x] Dated record under `doc/` with the 9 sufficient-design sections (this file).
- [x] Linked from `doc/README.md` under Dated records.
- [ ] Linked from developing-today/code#1283 (comment on merge).
- [x] Tracking: #13873 (AUDIT-7 Lane 6) and epic #1184 for filed failures.
- [x] Queue item 21671 (slice 1 of #1283).

Acceptance (from #1283), mapped to this record:

- [x] A documented drill schedule with collector-kill, feed-aging and service-drop: §4.1 and §4.4, implemented by S4, S5, S7.
- [x] Each drill verifies named signals with per-signal pass or fail: §4.3, implemented by S3.
- [x] Failed signal files an issue under #1184; passed drill posts to the tracking issue: §4.7, implemented by S6.
- [x] Restore verified by post-drill health read; interlock against overlap and incident flag: §4.5 and §4.6, implemented by S2 and S4.

Evidence references:

- developing-today/code#1283 (source issue, read 2026-11-24)
- developing-today/code#1110 (closed; orchestration section rejected for provenance type mismatch)
- developing-today/code#1184 (epic, PM stack coherence)
- developing-today/code#13873 (AUDIT-7 Lane 6 tracking)
- developing-today/code#13871 (AUDIT-7 umbrella)
- AGENTS.md, FDR-0013 (no owner-decision gates), fleet schedule registry rule

Design-only. This record does not implement the runner, the catalogue or the schedule.
