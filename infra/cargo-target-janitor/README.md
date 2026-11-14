# Cargo target janitor

Keep fleet-worktree Cargo targets below the **150 GiB** cap. The primary
`~/git/code/pkgs/id/target` is out of scope. Considered: directories under
`~/git/fleet/*/pkgs/id/target`, the `~/git/fleet/queue/*` sub-worktrees'
`pkgs/id/target` (one level deeper — ~82 GiB the top-level-only scan never
saw, #6812), repo-root cargo targets `~/git/fleet/*/target` and
`~/git/fleet/queue/*/target` when they hold build products (a `debug`,
`release`, or `.fingerprint` dir — ~114 GiB, #6812; a checkout's root
target belongs to the checkout, a non-checkout probe/scratch dir's root
target is treated like an ad-hoc target except that its owning dir
still exists, so it ages out instead of reading as worktree-gone), and
numeric `~/git/fleet/target-<task-id>` ad-hoc targets.

## Safety policy

- A target is eligible when its ad-hoc task target has no matching live
  worktree, its queue task is terminal (`done`, `failed`, or `cancelled`), or
  its newest file/directory mtime is at least **48 hours** old. If eligible
  targets cannot bring the fleet total under the cap, the janitor reports the
  overage and leaves younger, non-terminal targets alone.
- Never remove a target for a currently claimed `repo-health-code-pilot`
  task, a worktree whose branch has an open PR, an unmapped worktree/ad-hoc
  target, or a worktree/target containing a current-user Cargo, rustc,
  rust-analyzer, or build-script process cwd.
- Queue liveness comes from a claims source, picked per run and named in
  the plan output/record (`claims_source`): the repo-queue helper's
  transient Bitwarden credential path when that helper file still exists
  (broker task ids + states), otherwise the GitHub fallback source —
  `fleet queue fallback scan` live/contested claim markers unioned with
  open issues carrying `queue/claimed`, keyed by issue number because
  fallback claims live on issues (#6619). The fallback source has no
  broker task states, so it never makes a target eligible via the
  terminal-task reason; it only ever adds protection. Open PRs are read
  with `gh`. If no claims source can be read, cleanup stops — and the
  refusal now leaves a run record like every other run (#6812).
  Both liveness reads are paginated to completion (#6618) — claimed
  tasks by offset, open PRs by page — and a read that cannot complete
  within its page bound also stops cleanup: a truncated protection
  set is treated as an unavailable source, never as ground truth.
- Default mode is **dry-run**. It prints each proposed path, size and reason,
  plus every protected target. `--apply` is required to remove anything.
- Ad-hoc targets are exactly `target-<task-id>`; a registered worktree
  whose directory merely starts with `target-` is a checkout and is never
  classified as a disposable target. Sizes count each inode once (Cargo
  hard-links artifacts) at allocated blocks, so the cap tracks real disk
  usage. (#4175 Codex P1/P2 dispositions.)
- Apply re-reads queue claims, open PRs and process cwd state before each
  removal — including any build process whose cwd is anywhere inside the
  owning worktree, not only inside the target directory — checks path
  containment and age again, then writes/fsyncs a JSONL
  write-ahead record before removing the exact enumerated directory. No
  wildcard or force-delete command is used. At most 20 targets are planned
  per invocation; the hourly timer continues later if still over cap.
- Logs and run summaries live under
  `~/.local/state/cargo-target-janitor/runs/`. Every run writes a record
  there, including refusals (`"event": "refused"` with the reason): a
  janitor that cannot run must be visible in its own state, not only in
  the systemd journal (#6812 — the helper-path failure ran hourly,
  unrecorded, while the disk filled).
- If protected/open-PR or younger targets alone exceed the cap, the report and
  systemd journal say `over_cap`; they are deliberately kept rather than
  violating the safety exclusions. A maximum of 20 exact target paths is
  handled per run, so subsequent hourly runs continue eligible cleanup.

## Run and schedule

From the laptop:

```sh
python3 ~/git/code/infra/cargo-target-janitor/bin/cargo_target_janitor.py
# Review every listed candidate and protected path, then:
python3 ~/git/code/infra/cargo-target-janitor/bin/cargo_target_janitor.py --apply
```

The user timer runs hourly (persistent, up to 10 minutes randomized delay)
and uses `--apply`; first run it manually in dry-run mode and inspect the
report before enabling the timer. Install after merging/updating the primary:

```sh
~/git/code/infra/cargo-target-janitor/systemd/install-units.sh
systemctl --user list-timers 'cargo-target-janitor*'
```

Disable with `systemctl --user disable --now cargo-target-janitor.timer`.
The cap and stale threshold can be overridden for a manual run with
`--cap-gib` and `--stale-hours`; the scheduled defaults remain 150 GiB / 48 h.

## Tests

```sh
python3 ~/git/code/infra/cargo-target-janitor/tests/test_cargo_target_janitor.py
```

## Pagination contract (#6622)

Paged reads in this system follow the fleet pagination contract — a
paged read is **complete, bounded, or says so**: (1) the cursor is
validated where read (type first, then progress); (2) the volume is
bounded, and breaching the bound is an error naming the listing;
(3) truncation is never silent — the read completes, fails closed,
or marks the truncation in its output. Closing inventory:
 (#6623).
