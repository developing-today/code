# AUDIT-7 Lane 1 follow-up — rebase-and-merge queue for blocked PRs

Filed by AUDIT-7 Lane 1 (PR-SWEEP). Parent tracking: https://github.com/developing-today/code/issues/13872 — Umbrella: https://github.com/developing-today/code/issues/13871
Full per-PR ledger: doc record `doc/2026-11-24T00-40-00Z_audit_audit7-lane1-pr-sweep/` (Lane 1 report).

Premise: the sweep merged 103 of 199 unique open PRs. The 90 remaining blocked PRs are batched below by unblocker. Singular ownership: this issue is the only follow-up tracker for these batches; claim a batch by commenting here before working it.

## Batch A — rebase-only (conflicts at census, no other blocker)
code: #13850 #13842 #13772 #13738 #13701 #13700 #13639 #13600 #13589 #13567 #13445 #13406 #13397 #13372 #13368 #13313 #13210 #13167 #13155 #13133 #13132 #13127 #13113 #13111 #13093 #13089 #13042 #13037 #12840 #12814 #12756 #12732 #12627 #12605 #12548 #12539, plus #13184 (conflict named on the PR: event_store/ingest.rs decide_in vs main's COMPACTED residue check; re-run id-events suites).
opencode: #2629 #2541 #2533 #2532 (note #2525 prerequisite). openchamber: #2535.
Method per PR: worktree off the existing clone, rebase on main, re-run the PR's own stated verification, merge. Stop on any platform flag; vary the mechanic per standing doctrine.

## Batch B — became conflicting during the sweep (sibling merged first)
code: #13829 #13587 #13311 #13301 #13594. Same method as Batch A.

## Batch C — dependabot lockfile clusters (dependabot rebase does the work; verify + merge after)
code grok-cli: #13813 #13812 #13809. code supergateway: #13436 #13435 #13434, and group PR #13438 (express 5 / zod 4 majors — verify majors or close in favour of the individual PRs). opencode: #2536 #2535 #2534.

## Batch D — stacked (land parent first, retarget via API, merge)
code: #13047 (DIRTY; parent) → #13060 (also has an unresolved Codex P1 per warden note); #12539 (Batch A) → #12557; component A slice 2 PR (pending) → #13102 (slice 1 #13054 already merged by the sweep).

## Batch E — drafts (author action; no lane work until ready)
code: #13425 #13407 #13371 #12980 #12963 #12741 #12726 #12743 #12576 #12569 #12568 #12567 #12566 #12563 #12561 #12554 #12553 #12547 #12510 #12348 #12312 #12301 #12038 #9245 (coordinator disposition: not landed as-is), #12496 (negative result — keep as evidence, do not merge). opencode: #2499 (Turso flip = Drew decision).

## Batch F — decisions
code: #13265 (quota proposal; Drew decision + #1342 A/B gate), #13858 (dependabot cargo group with redb 5 / iroh 1.5 majors; needs build verification dispatch). dezren39/fleet#1 (Renovate activation = Drew/fleet-owner decision).

## Batch G — failing check
openchamber#2584 — "build and test (macos-15)" failure; macOS native fix belongs to the #2573 owner work.

## Batch H — sequencing
code#12740 — after #12605 and #12732 (Batch A) land.

## Superseded — close when the owner confirms
code#13300 (→ #13625 merged), #13699 (→ #13716 merged). (#13814 → #13820 already closed by its owner.)

Note on queueing: the pilot-queue enqueue verb exists on the laptop (`fleet queue enqueue <packet> --queue repo-health-code-pilot`, kind `feature_slice` + `payload.slice`), but its packet schema could not be verified from repo sources during the sweep session, so these batches were NOT enqueued blind. Whoever picks this up: confirm the packet shape from a known-good packet, then enqueue Batches A–D as separate `feature_slice` tasks citing this issue.
