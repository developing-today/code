# C001: F12.1 declarative world manifest, apply, diff and dry-run delta — D1-019

[Main reference](../main.md#c001)

## Relevant transcript excerpts

<!-- workflow:excerpt C001 1 18 -->

[Transcript lines 1-18](../transcript.txt#L1-L18)

```text
Source: https://github.com/developing-today/code/issues/534
Title: id: Phase 12 designs to run through the research workflow
Retrieved: 2026-11-03T00:04:00Z

--- ISSUE BODY (verbatim) ---
## What and why

These Phase 12 tasks (`.opencode/plans/phase-12-platform-flexibility.md`) need a design, or for D1-026 a research package, before anyone implements them. Run each through the research workflow, commit the design, then open an implementation issue and tick the box. F12.1 (manifest) and F12.4 (provider registry) are the plan's two highest-return tasks; F12.7 (ABI spec) is part of its recommended first slice.

## Items

Each box is done when the design (or research package) is committed and an implementation issue is linked here.

- [ ] **D1-019 — F12.1 Declarative world manifest (world.toml) with apply / --dry-run / diff** (L, design)
  A world.toml naming module (hash or source), seed, grants, ceilings, resource profile, roles/invites and schedules. `id world apply` reconciles a live world idempotently through existing admin operations, and `id world diff` detects drift. The plan calls it one of the two highest-return tasks.
  - Status: not started. No `world.toml` parser and no Apply/Diff command: `grep -rn 'world.toml' pkgs/id/src` finds nothing; `WorldCommand` (cli.rs:1388) lacks Apply/Diff.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:31; .opencode/plans/phase-12-platform-flexibility.md:21; .opencode/plans/phase-12-platform-flexibility.md:80
  - Prerequisites: #543 — D1-011 F11.11 One client projection (seed secret, chat by scope) + enforcement test, #227 — D1-012 F11.12 Journal format version header, check and backup commands
```

<!-- workflow:excerpt C001 113 128 -->

[Transcript lines 113-128](../transcript.txt#L113-L128)

```text
[Comment by @dezren39 at 2026-10-24T19:29:10Z; https://github.com/developing-today/code/issues/534#issuecomment-6388559285]
Progress from Phase 12 (Session C, PR #801): F12.16 (`id world doctor`) is implemented with tests; F12.1's declarative world.toml model (parse/plan/apply/diff) is in, with CLI `apply`/`diff` wiring in progress.

[Comment by @dezren39 at 2026-10-24T23:04:41Z; https://github.com/developing-today/code/issues/534#issuecomment-6390665385]
Progress (Session C, PR #801): F12.16 (doctor) done; F12.1 declarative world.toml + diff/apply landed (module/schedule apply → #807); F12.18 consolidated DoS suite landed; F12.3 builder/moderator scope bits + audit journal landed (remaining: wire builder/moderator capability acceptance into the install/revoke frames). Filed #805 (F12.11), #806 (F12.13), #807 (F12.1 apply wiring).

[Comment by @dezren39 at 2026-10-26T17:29:02Z; https://github.com/developing-today/code/issues/534#issuecomment-6422881000]
## Intake review

**Partially progressed — verified on main `14dd66b6` (2026-10-26, fleet monitor).** This is the Phase 12 design tracker. Its lead item, D1-019 (F12.1 declarative world manifest), appears to have outgrown the design stage: `WorldCommand` in `pkgs/id/src/cli.rs` now has `Diff` (:1731, "Show the drift between a world.toml and a live world") and `Apply` (:1756, "Reconcile a live world to a world.toml (idempotent; journaled)", with `--dry-run`) variants. That is the F12.1 surface the plan asked for. Not ticking the box or closing anything from the monitor chair: whether a committed design document exists and whether the implementation counts as complete (the Apply doc itself notes module install is still routed via install/compile) is a reviewer call — flagging it here so the tracker does not claim not-started for work that has landed.

The remaining items (F12.4 provider registry, F12.7 ABI spec, and the rest in the body) show no equivalent landed surface in this pass. No acceptance-criteria section existed, so one has been appended (original body preserved verbatim above it).

**Related:** prerequisite items name #543 and #227; downstream trackers #538 and #536 depend on this one's D1-022/D1-025 designs.

**Risks / suggested approach:** a reviewer pass over the Items list against current main, ticking what has genuinely landed (starting with D1-019), would sharpen this tracker considerably — several Phase 12 items have been shipping incrementally and tracker drift is the main risk.
```

## Normalized purpose

Intake and reconciliation for the F12.1 world.toml manifest, idempotent apply/dry-run and diff, including the roles/invites gap and partial-apply/rerun semantics. Phase-0 reconciliation against implementation issue #807 (CLOSED 2026-11-02: the body status snapshot is stale).

## Initial thoughts

- Reconcile the 2026-10-24 Session C progress and the 2026-10-26 fleet-monitor intake review against current origin/main before any new claims are written.
- Carry the merged design phase-0 acceptance: inventory vs code and the link to #807, without implementing any remaining apply wiring.
- Capture the source/module compile-contract gap from the merged design so the later design records it instead of re-litigating it.

## Inferred suggestions

- Inference: verify whether #807 closure changes the design delta (docs/rerun semantics) or only closes the wiring remainder.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
