# Kaneo ↔ GitHub sync rules (fleet record)

This file records the GitHub-integration behaviour this fleet depends
on, so a Kaneo re-connect, re-import, or database rebuild does not
silently change it. The settings themselves live in Kaneo's database
(`integration.config.statusTransitions` per project) — nothing in this
repository sets them — which is exactly how the defect below happened
unnoticed.

## The rule (set 2026-11-14, defect #7610, Drew's order)

For **every** GitHub integration (projects: code, opencode,
openchamber, nix):

| Kaneo event                                          | Target status              | Why                                       |
| ---------------------------------------------------- | -------------------------- | ----------------------------------------- |
| `onBranchPush`                                       | `in-progress`              | unchanged                                 |
| `onPROpen`                                           | `in-review`                | unchanged                                 |
| `onPRMerge`                                          | `in-review`                | **was `done` — changed 2026-11-14**       |
| issue closed on GitHub (e.g. native closing keyword) | `done` (built-in fallback) | legitimate closes still complete the task |

### Why `pr_merged` must not be `done`

Kaneo (v2.47.0, `apps/api/src/plugins/github/`) links a PR to a task
at PR-open time using loose references: a `#n` anywhere in the PR
**title**, a closing-keyword reference in the body, a branch-pattern
match, or a Kaneo task key. The merge handler
(`webhooks/pull-request-closed.ts`) never checks for a closing
keyword — any merged PR with a link moved its task to the
`pr_merged` target. With the target set to the final column, the
generic status-change sync labelled the linked GitHub issue
`status:done` and closed it seconds after the merge.

That closed issues that a PR merely referenced (Refs-only, no
closing keyword), including program epics whose first design PR
merged: #5866, #5767, openchamber #1831, #2381, #5772, #9060,
#9078, #9106, #9568. Epic #9078's elaboration task was even consumed
as `already_done` because the epic looked done. Full evidence is on
#7610.

With `pr_merged → in-review`, a Refs-only merge leaves the issue and
its labels untouched (the task is normally already in-review from
PR-open, so nothing changes at all). A PR with a real closing keyword
(`Closes`/`Fixes`/`Resolves #n`) still closes the issue — via
GitHub's native closing on merge to the default branch — and Kaneo's
`issue_closed` handler then moves the task to Done and applies
`status:done`. Verified live 2026-11-14 with fixtures #10106
(Refs-only: stayed open, no label change) and #10107 (Closes:
closed, task completed).

### Re-applying after a re-connect / re-import

The integration connect flow recreates `statusTransitions` with the
upstream default `onPRMerge: "done"`. After any re-connect, set it
back, either in Kaneo (Project Settings → Workflow → Automations:
`github / pr_merged` → **In Review**) or in the database on the
pm-stack host:

```sql
UPDATE integration
SET config = jsonb_set(config::jsonb,
                       '{statusTransitions,onPRMerge}',
                       '"in-review"')::text,
    updated_at = now()
WHERE type = 'github';
```

**Caution (2026-11-14, #10140):** the `jsonb_set` write above leaves
`integration.config` in jsonb text form (spaces after `:` and `,`),
which silently disables Kaneo's outbound state/title/description sync
— see "Outbound sync depends on the config text format" below. After
using it, rewrite the config in compact `JSON.stringify` form (the
sync guard now does this automatically when it heals drift).

Do **not** simply delete the `pr_merged` workflow rule in the UI:
in v2.47.0 deleting the rule falls back to
`statusTransitions.onPRMerge || "done"`, which restores the bug.

### Known upstream defects (usekaneo/kaneo v2.47.0)

- `utils/issue-references.ts`: title references need no closing
  keyword while body references do — the asymmetry that links
  Refs-only PRs to issues.
- `webhooks/pull-request-closed.ts`: merge handling never inspects
  title/body for closing keywords at all.
- Automations UI: the "don't move" option deletes the workflow rule,
  which silently re-enables the `done` fallback (above).

### Safety net

The closure watchdog (#10105, script in
`~/workspace/closure-watchdog/` on the Muse sandbox, cron every
15 minutes) polls issue-close events in the app-covered repos and
auto-reopens any automation close where the issue still has open
sub-issues or the closing PR carried no closing keyword. It stays
useful even with the rule fixed: it catches regressions of this rule
and task moves to Done made inside Kaneo itself.

## Enforcement — the sync guard (2026-11-14, #10137)

The rule above is asserted continuously, not just documented.
`pm-stack/kaneo-sync-guard.py` reads every GitHub integration's
`statusTransitions` from Kaneo's Postgres and checks: no transition
targets `done`, `onPRMerge` = `in-review`, `onPROpen` = `in-review`,
`onBranchPush` = `in-progress`. On drift it re-applies the safe map,
prints the before/after, and exits 1 (healed, but loud).

- Runs at deploy time: `deploy.sh` invokes it after the health check
  and fails the deploy if it reports drift or error.
- Runs hourly on the laptop: `pm-kaneo-sync-guard.timer` /
  `pm-kaneo-sync-guard.service` (unit files in `pm-stack/systemd/`).
- Verified 2026-11-14: clean run conforms on all 4 integrations; an
  induced `onPRMerge="done"` drift on the nix integration was detected
  and healed in one run.

Design rationale: `doc/2026-11-14T17-00-00Z_design_pm-stack-close-authority/note.md`
(close-authority program #10136). The closure watchdog (#10105, v2)
remains the outer safety net for any close path the guard cannot see.

## Fixture battery (2026-11-14, #10140 — re-run after any Kaneo upgrade or re-connect)

All on developing-today/code, verified from issue timelines + Kaneo DB:

| #   | Case                                                                                                          | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --- | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Refs-only PR (body `Refs #n`) on a plain issue (#10197 ← PR #10216, merged)                                   | Issue stayed OPEN, no `status:done`, no close events. Note: a body-only `Refs` created no Kaneo link at all (no `external_link` row) in this configuration.                                                                                                                                                                                                                                                                                                            |
| 2   | Refs-only PR on an epic with an open child (#10199 ← PR #10218, merged; child #10205 open)                    | Stayed OPEN, no `status:done`, no close events.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| 3   | Title-ref-only PR (#10201 ← PR #10220, title `(#10201)`)                                                      | Link formed — bot applied `status:in-review` at PR open (17:01:50Z) — merged 17:02:35Z; issue stayed OPEN, no `status:done`. Loose linking is live but contained by the `onPRMerge → in-review` rule.                                                                                                                                                                                                                                                                  |
| 4   | `Closes #n` PR (#10137 ← PR #10190, merged 17:04:11Z)                                                         | Closed natively by the merger 17:04:13Z; bot applied `status:done` 17:04:17Z (the `issue_closed → done` fallback). Legit path intact.                                                                                                                                                                                                                                                                                                                                  |
| 5a  | Human close on GitHub (#10197, closed 17:06:26Z)                                                              | Linked Kaneo task completed (`task.status = done` in DB read-back).                                                                                                                                                                                                                                                                                                                                                                                                    |
| 5b  | Human Done move inside Kaneo → issue closes                                                                   | **PASS 2026-11-14 (second run)**: fixture #10308 ← task CODE-3715 moved to Done via the Kaneo API (dedicated automation account + API key, below). First run failed — bot applied `status:done` but never closed; root cause was the config text format (next section) plus the closure watchdog stripping the orphaned label. After the config fix: bot `status:done` 17:51:20Z, issue **closed by developing-today[bot] 17:51:21Z**, stable across a watchdog sweep. |
| —   | Branch-named PRs (`fixture/refs-epic-10199` → PR #10258, `fixture/refs-plain-10197` → PR #10259, both merged) | No link formed (the `{slug}-{number}` branch pattern keys on Kaneo task slugs, not GitHub issue numbers); targets stayed OPEN.                                                                                                                                                                                                                                                                                                                                         |

Fixtures #10106/#10107 from the original fix (2026-11-14 earlier) cover the
title-ref-to-main and closes-to-main paths as well.

## Outbound sync depends on the config text format (2026-11-14, #10140)

Kaneo v2.47.0's outbound field sync (`syncLatestTaskValue`, used for
issue **state**, title and description) string-compares the stored
`integration.config` text against `JSON.stringify(JSON.parse(config))`
and silently aborts on any byte difference. Postgres `jsonb` text
output inserts spaces after `:` and `,`, so **any** write of the
config through `jsonb_set(...)::text` — including the re-apply SQL
above and the sync guard's original heal — disables Kaneo→GitHub
closing while leaving label sync (which compares parsed JSON) working.
Symptom: a task moved to Done applies `status:done` to the linked
issue but the issue never closes (and the closure watchdog then strips
the orphaned label as contradictory state).

Fix applied 2026-11-14: all four GitHub integrations' configs were
rewritten byte-identical in meaning, in compact `JSON.stringify`
form. The sync guard now also treats non-compact config text as drift
and rewrites it compactly after healing. After any manual config
write, verify: `JSON.stringify(JSON.parse(config)) === config`.

## Kaneo automation account (2026-11-14, #10140)

A dedicated account exists for API automation so fixtures and sync
work never need a human's session:

- User: **Muse Automation** (`claude-automation@developing-today.local`),
  workspace role `owner` in Developing Today.
- API key: `muse-pm-automation` (created via the better-auth api-key
  endpoint; verified with authenticated reads and the 5b fixture).
- Credentials live only in fleet secrets, by name:
  `kaneo.muse-automation-password` and `kaneo.api-key-muse-automation`.
