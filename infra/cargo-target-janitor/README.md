# Cargo target janitor

Keep fleet-worktree Cargo targets below the **150 GiB** cap. The primary
`~/git/code/pkgs/id/target` is out of scope. Only explicit directories under
`~/git/fleet/*/pkgs/id/target` and numeric `~/git/fleet/target-<task-id>`
ad-hoc targets are considered.

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
- Queue data is read through `~/hatch/tools/repo-queue.py`'s transient
  Bitwarden credential path; credentials are never printed or persisted.
  Open PRs are read with `gh`. If either source is unavailable, cleanup stops.
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
  `~/.local/state/cargo-target-janitor/runs/`.
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
