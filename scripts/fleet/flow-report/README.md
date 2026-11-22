# Flow report

Measures where the fleet's output goes: how much of what is opened, commented on and merged turns into finished `id` capability. It is the scoreboard for [`planning/plans/2026-11-21_results-first.md`](../../../planning/plans/2026-11-21_results-first.md). Every figure in that plan's evidence and scoreboard sections came from this tool, so a second person can re-run it and check.

Read-only. It calls `gh api` and `git log` and writes nothing to the repository, the queue or GitHub.

## Run it

```bash
# 1. Issues and PRs (about 120 API pages for developing-today/code)
python3 -I scripts/fleet/flow-report/flow_report.py fetch \
  --repo developing-today/code --out /tmp/issues.jsonl

# 2. History without file contents (about 15 MB instead of the full repo)
git clone --bare --filter=blob:none --single-branch --branch main \
  --shallow-since=2026-10-10 https://github.com/developing-today/code /tmp/hist.git

# 3. The scoreboard
python3 -I scripts/fleet/flow-report/flow_report.py report \
  --repo developing-today/code --issues /tmp/issues.jsonl \
  --git-dir /tmp/hist.git --rev main
```

Add `--format json` for the same numbers as data, `--since <date>` to narrow the git window, and `--now <ISO time>` to stamp the report with the time the data was fetched. Without `--git-dir` the mainline section prints "Not measured" and no figure for it.

Exit codes: `0` completed, `2` usage error, `3` a `gh` or `git` call failed or the input was empty. A failed run prints no scoreboard.

## What each number means

| Number                                 | Definition                                                                                                                                  |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Open issues blocked, parked or waiting | Open issues carrying `status/blocked`, `status/parked` or `status/waiting-dependency`.                                                      |
| Open `user/escalated`                  | Open issues waiting on an owner decision or access.                                                                                         |
| Open `id` issues                       | Open issues with `app/id`, `epic/id-program`, `epic/id-platform`, `design/id-phase-14` or `epic/id-crate-split`. Labels only, never titles. |
| Closure by effort label                | Closed issues over all issues carrying that `effort/*` label.                                                                               |
| main-broken by component               | Issues labelled `main-broken` or titled `main-broken`/`main CI failure`; the component is the first workflow word in the title.             |
| Mainline commits by area               | First-parent commits of `main`; each is assigned to the area holding most of the files it changed. The prefix table is `AREAS` in the tool. |
| `fix` and `feat` share for `id`        | Of commits that touch `pkgs/id`, the share whose PR title starts `fix` or `feat`.                                                           |
| Merged within 10 minutes               | Merged PRs whose merge time is under 10 minutes after the PR was opened.                                                                    |

## Limits

- It counts merged commits, not their size or value. A one-line fix and a new crate are one commit each.
- Area is decided by path prefix. A commit that changes `pkgs/id` and `pm-stack` equally counts for `id` (earlier row wins a tie).
- Labels are read as they are now. An issue relabelled after closing is counted under its current labels.
- It does not read the queue broker, token usage or CI results. Queue completions per merged PR and tokens per merged PR come from the C5 efficiency report (#9573), not from here.

## Tests

```bash
python3 -I -m unittest discover -s scripts/fleet/flow-report -p 'test_flow_report.py' -v
```

Offline. The fixtures are hand-counted, and the mainline test builds a real git history with two merged branches. The suite is registered as `fleet.flow-report` in `.github/test-suite-registry.json` and runs in `scripts-test.yml`.
