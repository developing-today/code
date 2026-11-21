---
name: feature-slice
description: "Deliver one vertical slice of a large feature issue as a single PR with a failing-first test and recorded evidence, instead of triaging or parking the issue. Use when a claimed task points at an effort/l or effort/m feature (an id F-item, a phase-14 step, a #9106 component), when a packet says small_fix or triage but the issue is large, when an issue has been triaged more than once with no PR, or when a performance target is missed and the next step is a measurement."
---

# Feature slice

The queue knows five packet kinds: `pr_shepherd`, `triage`, `small_fix`, `ci_investigation` and `second_pass` (`KINDS` in [`pkgs/fleet/crates/fleet-queue/src/validate.rs`](../../../pkgs/fleet/crates/fleet-queue/src/validate.rs)). None of them says "build part of a feature", so a large feature issue reaches a lane as a triage or small-fix packet, the lane correctly sees it is neither, and the issue is triaged again on the next cycle. This skill is what to do instead: cut the next slice and land it.

A slice is one PR that makes one acceptance criterion pass, or moves one measured number toward its target, and that could be reverted without breaking what was there before.

## Steps

1. **Read what "done" means.** Open the issue and its plan or design. List the acceptance or exit criteria and mark each one checked, unchecked or unmeasured against the code on `origin/main`, with `file:line` or a merged PR for every "checked". Do not trust the issue's own checkboxes.
   Done when every criterion has a state and evidence.

2. **Pick the slice.** Take the unchecked criterion with the fewest prerequisites. If it is too large for one PR, take the first step toward it that a test can observe: the interface with a permissive body, the storage without the wire, the measurement harness without the fix. For a missed performance target the first slice is always the profile: where the time goes, as a committed number and the command that produced it.
   Done when you can state the slice as "after this PR, `<command>` shows `<result>`".

3. **Boot a worktree.** Follow [`doc/agents/parallel-writers-and-subsessions.md`](../../../doc/agents/parallel-writers-and-subsessions.md): own worktree, empty branch pushed first, one writer per file. Run `scripts/worker-preflight.sh`.

4. **Write the failing test first, and name its oracle.** The test fails on `origin/main` and passes after the change. Say what decides correctness: a reference implementation, a model, an invariant or a golden corpus ([testing doctrine](../../../AGENTS.md#testing-doctrine)). Where the code has invariants, state them as a property test.
   Done when the test fails for the reason the slice fixes, not for a compile error or a missing fixture.

5. **Register the test.** A new test file inside `pkgs/id` or `pkgs/mcpx` needs a row in [`.github/test-suite-registry.json`](../../../.github/test-suite-registry.json), or no CI venue is on record as running it. Check with:

   ```bash
   bash .github/scripts/check-suite-registry.sh | grep -F "<your test path>"
   ```

   Done when that prints nothing for your file.

6. **Implement, then verify with the narrowest build that proves it.** On the laptop, build through `~/tools/fleet-build-lock`; `cargo test -p <crate> <filter>`, not the whole workspace, and without `--features web` unless the slice touches web code ([owner preferences](../../../doc/agents/owner-preferences.md)). A sandboxed worker does not build: it says so and names the check a laptop lane or the nightly must run.
   Done when the test passes and the output is saved.

7. **Open the PR with evidence.** Use the house shape: Summary, Evidence (before and after, with the command), Merge Danger. State the evidence level in plain words: "ran on the laptop at `<sha>`", "not built here; relies on nightly `<workflow>`", or "measured: `<number>` on `<machine>`".

8. **Update the issue, once.** Tick the criterion in the issue body with the PR link, and post one progress note. Close the issue only when every criterion is checked with evidence ([FDR-0010](../../../doc/decisions/FDR-0010-completion-requires-evidence.md)). If criteria remain, the note ends with the next slice in the same "after this PR" form, so the next worker starts at step 3, not step 1.

## When something stands in the way

- **A product or architecture choice.** Make it, record it, and build on it ([`decide-and-proceed`](../decide-and-proceed/SKILL.md)). Nothing waits on the owner.
- **The packet's type or timebox does not fit.** Re-scope to the slice, record it in the queue checkpoint, and continue. Do not return `packet_defect` for size alone.
- **The slice needs another issue's work first.** Check that claim against the code. If it holds, land the part that does not depend on it, and name the dependency as `Requires: #N` with the file or symbol that proves it.
- **Out of time.** Push the branch, record the resume point in the checkpoint, and park with the slice named. Never park a large issue with no code and no measurement ([park rules](../../../doc/agents/one-queue-claiming-work.md#who-holds-an-issue-assignee-is-not-a-hold)).

## Should not load

Settling a design across several sub-features is `design-orchestrate`. Turning a settled design into a numbered plan with one issue per step is `feature-plan`. Running many builders at once is `implementation-orchestrate`. This skill is for one worker, one issue, one PR.
