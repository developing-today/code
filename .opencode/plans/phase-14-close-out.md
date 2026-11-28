# Phase 14: close-out — finish, measure, verify, and open the gates

See the 2026-10-24 project status audit (summarised under [Evidence](#evidence-this-plan-starts-from)), the [platform world-state report](../../doc/2026-10-18T18-40-00Z_report_platform_world_state/2026-10-18T18-40-00Z_report_platform_world_state.md) (§8, §10, §12) and the [architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md). This phase follows [Phase 11](phase-11-foundations.md), [Phase 12](phase-12-platform-flexibility.md) and [Phase 13](phase-13-client-shells.md); the dependencies are named per task.

## Goal

Phases 9 to 13 designed far more than they verified. Phase 11 was meant to make the platform trustworthy and measurable, but its measuring tasks (F11.7, F11.8, F11.9) never started, and Phase 12 then landed half-slices (manifest, fork, roles) whose last step is unwired, plus two new journal entry types under an unchanged journal version. Phase 13 is now designed, and its implementation is gated on work nobody has scheduled: F12.4 and F12.6 gate F13.7, and the F11.9 baseline gates F13.4 and F13.5.

Phase 14 makes "done" mean done. It finishes the half-landed slices, records the numbers the later phases are stated against, adds the proving tests the earlier phases promised, and builds the two extension-model foundations (provider registry, audit log) that Phase 13 waits on. Nothing here adds a user-visible feature for its own sake.

## Why this theme

The theme is chosen from the record, not invented. Four facts decide it:

1. **The gates are unscheduled.** The Phase 13 plan says F13.7 "cannot start until F12.4 and F12.6 land" and F13.4/F13.5 run "after the F11.9 batching baseline exists". F12.4 and F12.6 are design checkboxes in epic #534 with no implementation issue; F11.9 (#542) is an S-sized task that has not started. No phase owns them.
2. **The record is not safe to build on.** The Phase 12 plan's own risk section says "F11.12 must land before the first Phase 12 entry type". Phase 12 then added `JournalEntry::BuilderInstalled` and `JournalEntry::Forked` (`pkgs/id/src/world/mod.rs:519`, `:530`) while `JOURNAL_VERSION` is still `1` (`world/mod.rs:644`) and #227 is open. Binaries built before #244 silently truncate a final unreadable line. Every later journal change makes this worse.
3. **Half-slices are live on main.** `id world apply` reports steps it did not apply and plans against a fabricated `seed: 0` state (#818, `commands/world.rs:707-731`); an omitted `ceiling` in a manifest widens the world to the whole catalog (#817); fork exists only at the store level (#556); builder and moderator authority is reachable only through `install_program_by`/`revoke_by` with `by: None` from every caller (`world/handle.rs:489`, `:648`), so no frame carries it.
4. **Verification debt is now itemised.** The audit and #796 record that Phases 9 and 10 "were delivered faster than they were verified". The phases 0-10 review split that debt into concrete gaps (#811, #812, #813) and a child inventory of missing proving tests on #796. The Phase 11 status tracker (`2026-10-20_phase-11-status.md`) is stale in both directions: it lists F11.5, F11.10, F11.13 and F11.17 as planned (all merged in #808) and F11.4 as done (its exit is not met, #551).

A feature phase stacked on this would inherit all four problems. A close-out phase removes them and leaves Phase 12's remaining work and Phase 13's implementation unblocked.

## Scope and non-goals

**In scope:** the remaining Phase 11 measurement, toolchain, determinism and CI tasks; finishing the Phase 12 slices already partly on main (F12.1 apply, F12.2 fork, F12.3 roles, F12.18 suite); journal format management (F11.12); the Phase 9/10 verification debt; the Phase 10 directory-feed remainder; the F12.4 provider registry and F12.6 audit log as the Phase 13 gates; and the one open platform-completion gap, signed document revision replication (#792).

**Phase 14 is NOT:**

- **Not new guest capabilities.** `kv`, `blob`, `http.fetch` and `notify` stay F12.5. The registry (F14.20) is proven with a test provider, not a new real one.
- **Not the guest ABI or SDKs.** F12.7, F12.8 and F12.9 stay Phase 12. F14.14 builds the determinism check F12.7's conformance suite will use; it does not write the spec.
- **Not scale or cross-node worlds.** F12.10 (fair scheduling), F12.11 delta frames (#805), F12.12 many worlds (#557), F12.13 journal replication (#806) and F12.14 portals stay Phase 12. Phase 14 provides their prerequisites (F11.7, F11.8, F11.9, F11.12, F12.6) and stops there.
- **Not authoring loops.** F12.15 `id world dev` (#558) stays Phase 12; it waits on F12.9.
- **Not Phase 13.** No shell contract, transport, presentation tier, native shell, client provider or topology work (#783-#790). In particular, F13.7's design is Phase 13's (#789) and is not redone here; Phase 14 only builds the host-side F12.4 and F12.6 it depends on. Redraw batching (#561) and the diff renderer (#562) stay with the F13.4 chain; Phase 14 only records the baseline they are measured against.
- **Not the Phase 2-8 follow-ups** (#161-#195, #793-#803), which belong to the membership, sync, groups, documents and connectivity subsystems and their own plan, nor the owner-blocked real-network work (P7.2, P7.5, P3.7).

## Evidence this plan starts from

| Observation | Source |
| --- | --- |
| `pkgs/id` main CI was red on a stale p2panda hash (run 40094508527); the id build was then fixed in the first wave. | status audit (outside the repo), PR history |
| F11.7, F11.8, F11.9 not started; no `benches/`, no Prometheus, no frame-size table. | #539, #542, #553 |
| Fast lane takes 46-58 minutes per PR; the status tracker calls F11.4 done. | #551 |
| `JOURNAL_VERSION = 1` while Phase 12 added two entry types. | `world/mod.rs:519`, `:530`, `:644`; #227 |
| Manifest apply skips module and schedule steps but reports them applied; omitted fields are treated as authoritative, and an omitted ceiling widens to the catalog. | #807, #817, #818 |
| Fork is store-level only; no CLI or frame. | #556 comment; `tests/world_fork.rs` |
| Builder/moderator authority has no caller that passes an actor. | `world/handle.rs:489`, `:494`, `:648`, `:654` |
| A joiner may resize to 500x200 although the screen DTO caps at 120x50. Since F11.17 (#808) an oversized `layout` no longer poisons the world (`world/actor.rs:1127-1150`), but it gives that session an error screen instead of falling back to `view` plus cropping. | #811; `world_session/mod.rs:1888`; `world_presentation.rs:15` |
| Roc `expect` blocks (29 in `RocJson`, 15 in `Screen`, 13 in `Key`) never run in any gate. | #813 |
| Checked-in example modules are 526-686 KB and built by whatever `roc` is on PATH. | #541, #555 |
| No double-replay determinism check; host events are not retained, so fixtures are incomplete for interactive programs. | #544, #242 |
| `id world list --json` is admin-only, has no presence counts, and `open` means loaded in memory. | #812; `commands/world.rs:635`, `:652` |
| Per-slice P9/P10 acceptance criteria lack proving tests (web/terminal parity, DTO caps, chat/records bounds, resize/paste wiring, CLI verbs, schedule visibility). | #796 child inventory |
| The capability catalog is a const list and dispatch is a string `match`. | `world_caps.rs:71`; `world/actor.rs:539-559` |
| Signed ProseMirror revisions publish only to the node-local `LocalEvents` store. | #792; `local_events.rs:10`; `web/routes.rs:1131` |

## What "done" means here

Exit tests for the whole phase, each measurable.

| Criterion | Target |
| --- | --- |
| The journal is safe to extend | A journal with an entry newer than the binary is refused up front with a clear message, by version header, and `backup` restores byte for byte |
| Every Phase 12 slice on main is complete | `apply`, `fork`, builder install and moderator revoke each work end to end over the wire, with a test per path |
| The numbers later phases are stated against exist | Frame-size table, benchmark baselines and host metrics are committed and rerunnable |
| What shipped is proven | Every #796 child item and #811/#812/#813 has a proving test in CI; the Roc `expect` blocks run in a gate |
| Determinism is checked | `id world verify` fails a deliberately nondeterministic guest and passes every example |
| The fast lane is fast | Median PR fast-lane feedback under 15 minutes over ten runs |
| The Phase 13 gates are open | A test provider is added without editing `world_caps.rs` or the dispatch `match`, and every admin action and provider use leaves an audit line that replays to the final grants |
| No signed record is node-local by accident | A second node discovers a signed document revision published on the first (#792), or the design records why not and what replaces it |

## Architecture

Phase 14 touches four planes of the existing system. It adds no new plane.

### 1. The durable record (journal)

The world journal is the one durable authority (`world/core.rs`). F14.1 gives it a version contract: the header version rises whenever an entry type is added, a binary refuses a journal whose version it does not know before reading any entry, and `id world journal check` and `backup` turn the rollback rule into tooling. The two Phase 12 entry types (`BuilderInstalled`, `Forked`) are assigned to the first new version, and the record lists every entry type with the version that introduced it. F14.4, F14.5, F14.6 and every later phase's new entry builds on this, which is why F14.1 is first.

### 2. The measurement plane

Three committed artifacts, each rerunnable by a `just` recipe with a matching `nix run .#<cmd>` app (`pkgs/id/AGENTS.md` keeps the two in sync):

- the **frame-size table** (F14.7) — bytes per full frame for each example at 80x24 and 200x60, the number F12.11, #561, #562, F13.4 and F13.5 must beat;
- the **benchmark baselines** (F14.8) — guest replay, journal append and fsync, checkpoint compaction, session fan-out;
- the **host metrics** (F14.9) — counters and histograms behind `id world stats` and an admin-token Prometheus endpoint, with a `tracing` span per actor command.

F14.9 is the substrate F14.21 (audit) reuses for export.

### 3. The verification plane

Tests and CI gates only, no new runtime behaviour unless a test exposes a bug: Roc `expect` execution (F14.10), the pinned Roc toolchain check (F14.11), double replay (F14.14), per-slice proving tests (F14.16), the probe-driven DoS harness (F14.17), and the fast/full CI split (F14.18). A test that finds a real bug files an issue and is fixed in its own PR, not folded into the test PR.

### 4. The extension plane

F14.20 replaces the `CATALOG` const (`world_caps.rs:71`) and the dispatch `match` (`world/actor.rs:539-559`) with a registry of providers, each with a manifest: name, version, argument schema, bounds, required scope and default ceiling. The ledger, the compile gate, the ceiling check and `id world caps` all read the registry. Existing providers (`time.now`, `chat.history`, `records.read`, `schedule.at`, …) are migrated without behaviour change; a test provider proves the seam.

F14.21 adds a structured audit log: one typed line per admin frame and per provider use, carrying actor, world, action, arguments digest and outcome. It generalises the journal-only audit that F12.3 introduced (`BuilderInstalled`, `Revoked { by }`) rather than adding a second mechanism, exports through the F14.9 plumbing, and replays to the final grants. The audit line shape is the contract F13.7 (#789) consumes for client providers; Phase 14 fixes the host side only and does not decide client-side granting.

### Signed records beyond the worlds platform

F14.22 is the one item outside the worlds code. ProseMirror saves sign a `document.put` revision and publish it to `LocalEvents`, which "stands in for sync" (`local_events.rs:10`). The design must decide whether those revisions join the existing signed-event sync path (the p2panda-backed store and LogSync) or whether `LocalEvents` itself replicates, and must keep the Phase 3 sync bounds (#178, #180) and blob gating (#173) intact.

## Security and trust

- **No new authority.** The registry migrates providers; it does not widen any. F14.20's acceptance includes a test that the registry exposes exactly the providers the old `CATALOG` did, under the same ceilings.
- **Manifest fail-closed.** F14.3 makes an omitted field mean "leave alone" and never "widen". An omitted ceiling must never become the whole catalog.
- **Roles stay narrow.** F14.6 wires builder install and moderator revoke through frames; builder still never receives network, file or process authority, and every use leaves an audit entry.
- **Fork respects the seed policy.** F14.5 inherits the install seed only as the F11.11 policy allows.
- **Metrics and audit are admin-only.** The Prometheus endpoint and the audit log refuse requests without the admin token (tested). Audit lines carry an arguments digest, not raw guest arguments.
- **No new Wasm import.** Nothing in Phase 14 adds a guest import (`sandbox.rs:1-8`).

## Workstreams

Tasks are numbered F14.n. Each names its tracking issue, a size in the Phase 11-13 issue scale, and an exit criterion a test or CI job can check. Where an existing issue already covers the task, that issue is the tracking issue and its acceptance list is authoritative; the exit below summarises it.

### A. Make the record safe

- **F14.1 Journal format version and the forward-only debt (F11.12; #227; L).** A version header that rises with each new entry type, refusal of an unknown version before reading entries, `id world journal check` and `id world journal backup`, and an entry-type-to-version table in the record. `BuilderInstalled` and `Forked` move to the first new version, with an upgrade path for existing journals that cannot leave a partly rewritten file. Exit: a test opens a journal with a future version and gets a clear refusal; `backup` restores byte for byte; a version-1 journal still opens and upgrades atomically on the first new entry.
- **F14.2 Clamp joiner resize and fall back per session (#811; S).** Clamp `Resize` to `MAX_SCREEN_WIDTH`/`MAX_SCREEN_HEIGHT` before `view_at`, and treat an oversized `layout` result as a per-session fallback to `view` plus cropping instead of an error screen. Re-verify the world-wide poisoning that #811 reported is gone since F11.17. Exit: #811's proving test — session A resizes to 500x200 against a guest whose `layout` exceeds `presentation_bytes`; session B still receives views and input is accepted.

### B. Finish the half-landed Phase 12 slices

- **F14.3 Manifest semantics and honest apply/diff (F12.1; #817, #818; M).** Represent "unspecified" distinctly from "empty" for grants, ceiling, schedules and profile; expose the install seed digest and pending schedules over the admin wire so `diff` and `plan` read real state; report applied and skipped steps separately with a nonzero exit when any planned step was skipped. Exit: a minimal manifest against a configured world plans zero steps; an omitted ceiling never widens; `apply` never claims a step it skipped.
- **F14.4 Manifest apply wiring: module and schedules (F12.1; #807; M).** `id world apply` installs or compiles the manifest module and sets and cancels schedules, journaling each step. Depends on F14.1 and F14.3. Exit: a manifest naming only a module and schedules applies idempotently — the second run changes nothing.
- **F14.5 `id world fork` over the wire (F12.2; #556; M).** CLI and admin frame for the store-level fork, the template flag, and the F11.11 seed-inheritance hook. Depends on F14.1. Exit: a fork replays to the parent's records at the fork point, and the seed is inherited only as the policy allows.
- **F14.6 Builder and moderator authority through session frames (F12.3; #835; M).** The install and revoke frames accept builder and moderator participants and pass the actor to `install_program_by`/`revoke_by`, so the `BuilderInstalled` and `Revoked { by }` audit entries are produced in practice. Depends on F14.1. Exit: over a real session, a builder installs a module, a moderator revokes a participant, a guest can do neither, and each use is journaled with its actor.

### C. Measure

- **F14.7 Bytes-per-frame baseline (F11.9; #542; S).** Full-frame sizes for each example at 80x24 and 200x60 for SSH, web and CLI presentations, as a rerunnable test or script. Exit: the table is committed with the commit it was measured at. Unblocks #786, #787, #561, #562 and #805.
- **F14.8 Benchmark harness (F11.8; #539; L).** Guest replay of N events, journal append with fsync, checkpoint compaction and fan-out to K clients; committed baselines; a non-blocking CI job that reports regressions over 20 percent. Exit: baselines committed and a regression is reported on a PR. Unblocks #551, #557 and F12.10.
- **F14.9 Host metrics and tracing (F11.7; #553; L).** Actor queue depth, command latency, journal append and fsync latency, checkpoint duration, session count and frame bytes, in `id world stats` and admin-token Prometheus text, with a `tracing` span per actor command. Exit: a test reads each metric after a scripted session; the endpoint refuses a request without the token; overhead under 2 percent in the F14.8 harness.

### D. Verify what shipped

- **F14.10 Run the Roc `expect` tests in CI (#813; S).** A `roc-test` Nix check and matching `just roc-test` recipe running `roc test` on every platform module with the pinned compiler, added to the `id` checks so the skip path cannot pass silently. Exit: a deliberately false `expect` fails the check.
- **F14.11 Pinned Roc compiler for checked-in modules (F11.18; #541; M).** Rebuild every example with `nix/roc.nix`, make each `build.sh` refuse another compiler, and add a CI job that rebuilds examples and checks behaviour (sandbox suite and fuel budgets), not hashes. Exit: the job fails when an example is built by another compiler.
- **F14.12 Smaller example modules (F11.15; #555; S).** `wasm-opt -Os` and strip, no `--debug`. Depends on F14.11. Exit: every checked-in module at or under 400 KB, instantiate time recorded before and after.
- **F14.13 Finish the example migration (#565; S).** tic-tac-toe onto `RocJson`, roc-counter onto `Screen`, and one example imports `RocText`. Depends on F14.11. Exit: no example hand-builds JSON or renders plain text, and example tests pass.
- **F14.14 Replay self-check (F11.16; #544; M).** `id world verify` replays a journal twice, compares records and reports the first divergence; runs against the #239 fixtures. Exit: a deliberately nondeterministic guest fails, every example passes. This is the determinism guard F12.7's conformance suite and F12.13 replication build on.
- **F14.15 Host-event retention and visibility (F11.11 remainder; #242; M).** Decide whether host events (presence, capability results, ticks, schedule fires) are recorded from the journal through an admin path or retained in the stream with a visibility rule, then implement the decision through the single client projection. Exit: a fixture of `territory` replays to matching records, and a test proves no join-only participant receives a capability result or schedule fire it may not see.
- **F14.16 Phase 9 and 10 proving tests (#796; M).** The #796 child inventory: a shared web/terminal screen-cell corpus asserted by both Rust and vitest (9-2, which also settles #724/#761), full DTO rejection caps and serialization determinism (9-3), `chat.history` and `records.read` bounds and refusals (9-5), SSH window-change and browser paste/ResizeObserver wiring (9-6), source-only requiring a compiler (9-7), CLI verbs beyond parse tests including compiling the `world new` scaffold (9-8), and schedule-fire visibility and ceiling (10-2). Exit: each child item has a named test in CI; any bug a test finds has its own issue.
- **F14.17 DoS probe harness (F12.18; #560; M).** Fold the bounds `tests/world_dos.rs` maps to other files (fuel and memory, journal and frame size, session and open-world counts, schedule count) into the one probe-driven harness, each refusal asserting its F11.13 code while a probe client stays responsive. Exit: the suite covers every bound in the F12.18 list and runs in CI.
- **F14.18 Fast lane and an honest status tracker (F11.4; #551; M).** A fast job (fmt, clippy, unit tests) separate from the full lane, and correct `2026-10-20_phase-11-status.md` for F11.4, F11.5, F11.10, F11.13 and F11.17. Depends on F14.8 for timing. Exit: median fast-lane feedback under 15 minutes over ten consecutive runs, with the runs listed.
- **F14.19 Directory feed remainder (Phase 10; #812; M).** One shared `directory_json` serializer for CLI and web, `open` renamed to `loaded` where it means in memory, a shape test; then a short design for a public, non-admin feed with presence counts that does not open idle worlds, implemented if the design needs no new authority. Exit: one serializer with a shape test; the public-feed decision is recorded and either built with an e2e check or filed.

### E. Open the Phase 13 gates

- **F14.20 Provider registry (F12.4; #534 D1-022; L).** Design, then implement: providers as registered components with a manifest, read by the ledger, compile gate, ceiling and `caps`; existing providers migrated with no behaviour change. Depends on F11.10 and F11.13 (both merged in #808). Exit: a test provider is added without editing `world_caps.rs` or the dispatch `match` and appears in `caps`, the ceiling and the compile gate; the migrated registry exposes exactly the old catalog. Unblocks F12.5, F12.14 and F13.7 (#789).
- **F14.21 Structured audit log (F12.6; #534 D1-024; L).** Design, then implement: one typed audit line per admin frame and provider use, generalising the F12.3 journal-only audit, exported through F14.9 (OpenTelemetry optional). Depends on F14.9 and F14.20. Exit: every admin frame produces an audit line, and a test replays the log to the final grants. Unblocks F12.5, F12.13, F12.14 and F13.7 (#789).

Per #534's rule, F14.20 and F14.21 tick their #534 boxes when their designs are committed, and their implementation issues are opened from those designs; this plan does not pre-file them.

### F. Replicate what is signed

- **F14.22 Replicate signed document revisions (#792; XL).** Design first: choose between joining the signed-event sync path and replicating `LocalEvents`, keeping the Phase 3 bounds; then implement the chosen path. Exit: a two-node test in which node B discovers a `document.put` revision node A signed on save, and the acceptance brief's open workstream item is closed. If the design shows this needs a sync-layer change larger than one phase task, the design is the deliverable and the implementation moves to its own plan.

## Sequencing

```text
F14.2                                (smallest, anytime; first)
F14.1 -> F14.4, F14.5, F14.6         (version contract before any new journal entry)
F14.3 -> F14.4                       (fix semantics before applying more steps)
F14.7                                (S; unblocks #786/#787, #561/#562, #805)
F14.8 -> F14.18; F14.8 -> F14.9      (harness, then fast-lane timing and metric overhead)
F14.10, F14.11 -> F14.12, F14.13     (run the Roc tests, pin the compiler, then rebuild)
F14.15 -> F14.14                     (host-event rule decides what verify compares)
F14.16, F14.17, F14.19               (anytime)
F14.20 -> F14.21; F14.9 -> F14.21    (registry, then audit over it)
F14.21 -> #789 (F13.7 implementation)
F14.22                               (independent; design first)
```

**First slice:** F14.1, F14.2, F14.3, F14.7, F14.10 and the F14.20 design. Each is small or unblocks several others, and together they stop the record and the manifest from getting worse while measurement starts. Decide the rest after their exit tests, as Phases 12 and 13 did.

F14.1 is the highest-return task: every later journal entry, and Phase 12's F12.13 replication, inherits its contract. F14.20 is the other: it turns a core-code change into data, and Phase 13 waits on it.

## Acceptance criteria

Each F14.x carries an exit criterion above. The consolidated checkpoints are:

- [ ] `.opencode/plans/phase-14-close-out.md` committed (this file) — PR #834
- [ ] Every F14.x names one tracking issue; existing issues are referenced, not duplicated
- [ ] The Phase 13 gates (F12.4, F12.6, F11.9) each have a scheduled F14.x item
- [ ] The Phase 11 status tracker is corrected (F14.18)
- [ ] Items left in Phase 12 or Phase 13 are named in the non-goals with their issues

## Risks

- **Collision with Phase 13 implementation.** Session-level Phase 13 work (F13.1, F13.2) edits `world_session/` and `world_ssh.rs`, which F14.2, F14.5, F14.6 and F14.15 also touch. One writer per file: sequence those Phase 14 tasks after the in-flight Phase 13 PR merges, or coordinate the file owner explicitly.
- **F14.1 upgrade correctness.** Upgrading an existing version-1 journal is the riskiest change in the phase: a failed upgrade must leave the old file intact. Backup before upgrade, write to a new file and rename, and test a crash between the two.
- **Registry migration drift.** Moving every provider into the registry can change a refusal code or bound silently. F14.17's DoS suite and the F11.13 code tests run before and after, and the registry PR must not change their output.
- **Scope.** Twenty-two tasks, most S or M but four L and one XL. The first slice is fixed above; F14.22 is allowed to end at its design.
- **Verification finds bugs.** F14.16 and F14.17 may expose real defects. Each becomes its own issue and PR; the test PR does not grow into a fix PR.
- **Stale trackers recur.** The Phase 11 tracker went stale within days. Each F14 PR updates this plan's status in its own section, not by rewriting a task line.

## Open questions

1. **Host-event visibility (F14.15).** Record host events from the journal through an admin path, or retain them in the stream with a visibility rule? #242 lists both; the owner decided the seed and chat rules for F11.11 and should decide this one. Recommend the journal path: it changes nothing joiners receive.
2. **Public directory feed (F14.19).** Is a non-admin feed wanted at all, and who may read it — anyone who can reach the node, or holders of any world invite? The #812 design must answer this before presence counting is built.
3. **Audit export (F14.21).** Is OpenTelemetry export required in the first slice, or is a structured local log plus the Prometheus plumbing enough? Recommend local log first, OpenTelemetry behind a feature flag.
4. **Revision replication path (F14.22).** Join the p2panda-backed signed-event sync, or replicate `LocalEvents`? The first reuses audited sync and bounds; the second is smaller but duplicates them. Recommend the first.
5. **Journal upgrade policy (F14.1).** Automatic upgrade on the first new entry, or an explicit `id world journal upgrade` step? #227 lists both. Recommend explicit upgrade with `doctor` (F12.16) reporting a pending one.

## References

- [Phase 13 plan](phase-13-client-shells.md) (F13.4/F13.5 gated on F11.9; F13.7 gated on F12.4 and F12.6)
- [Phase 12 plan](phase-12-platform-flexibility.md) (F12.1-F12.4, F12.6, F12.18; the F11.12-first risk)
- [Phase 11 plan](phase-11-foundations.md) and [status tracker](2026-10-20_phase-11-status.md)
- [Phase 9 and 10 plan](phase-9-portable-platform.md) (P9.1 acceptance, the Phase 10 directory sketch)
- [Platform world-state report](../../doc/2026-10-18T18-40-00Z_report_platform_world_state/2026-10-18T18-40-00Z_report_platform_world_state.md) (§8 done/not done, §10 verification state, §12 where next)
- [Architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [Platform completion acceptance brief](../../doc/2026-10-14T_platform_completion/acceptance-brief.md) (the revision-replication gap, #792)
