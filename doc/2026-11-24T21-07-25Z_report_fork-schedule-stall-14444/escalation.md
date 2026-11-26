# Fork schedule stall: developing-today-forks/opencode agents.yml silent since 2026-11-20

Escalation record for developing-today/code#14444 (lane oc-code-05, 2026-11-24).
Slice PR: `.github/scripts/check-fork-schedule-stall.sh` (+ contract test),
which encodes this diagnosis as a repeatable command.

## Verdict

Repo-wide GitHub schedule suppression on the fork — not an agents.yml
defect and not caused by the last runs' failures. Escalated as a
platform limit; the workflow file must not be "fixed".

Owning platform limit: **GitHub Actions schedule policy for fork
repositories** (cron delivery suppressed on forks while every other
event type keeps working).

## Evidence (re-verified live 2026-11-24 ~21:00Z, `gh api`, lane oc-code-05)

- `agents` workflow (id 401762792) state is **active**; last scheduled
  run 44826053161, 2026-11-20T22:06Z, conclusion **failure** — died in
  `Dispatch suggestions` on `pm-dispatch POST /v1/sessions` HTTP 502
  (all four attempts). The 502 is real but unrelated to the stall:
  that run *did* fire.
- Zero schedule-event runs repo-wide after 2026-11-20 (last: repo-sweeper
  44833661765, 2026-11-20T23:01Z, success; auto-format 22:49Z success;
  models-snapshot 19:35Z success). Per-workflow last-schedule list:
  agents 22:06Z failure, repo-sweeper 23:01Z success, auto-format
  22:49Z success, models-snapshot 19:35Z success.
- Non-schedule runs continue the same day (Dependabot `dynamic` runs,
  latest 2026-11-24T18:27Z). Actions is on; only cron triggers are
  silent, repo-wide, across 8 scheduled workflows.
- Machine-readable (committed classifier on the live 100-run window):
  `newest_schedule_event=2026-11-20T23:01:45Z`,
  `days_since_last_schedule=3.9`,
  `nonschedule_runs_since_last_schedule=25`,
  `verdict=repo-wide-cron-silence`.

Reproduce: `gh api "repos/developing-today-forks/opencode/actions/runs?per_page=100" > runs.json`
then `bash .github/scripts/check-fork-schedule-stall.sh --runs-json runs.json`.

## Recommendation (re-enable + observe, then move if silent)

1. An owner/admin re-enables the workflow via the Actions API
   (`PUT /repos/developing-today-forks/opencode/actions/workflows/401762792/enable`)
   and watches the next two cron slots (00:00 / 15:00 UTC). A manual
   `workflow_dispatch` proves the workflow executes but does not prove
   cron is back.
2. If crons stay silent after re-enable, the schedule moves off the
   fork (e.g. onto developing-today/code's pm-scheduled surface, which
   is where the slot table already mirrors from).

## Effect on parked task 20778

It cannot resume/revalidate until a *scheduled* (not manual) agents run
succeeds, so it stays parked on this platform blocker. Separately, the
Nov 18–20 failures show the pm-dispatch 502 (code#6992) must be gone or
the resumed run will fire and fail in `Dispatch suggestions`.

## Issue acceptance state after this slice

- [x] Reason identified with evidence (this record + classifier + live run list).
- [ ] Scheduled runs resume, or blocker escalated with the owning
      platform limit named — escalation half done here (limit named
      above and on the issue); resume awaits re-enable + observe.
- [ ] Finding recorded on the issue including 20778 effect — the PR's
      progress note carries it; the checkbox closes when the note lands.
