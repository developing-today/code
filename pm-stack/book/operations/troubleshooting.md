# Troubleshooting

Symptom → cause → fix. Each entry names the check that confirms the cause before you apply the fix. Every entry in this chapter comes from the surveys and the numbered disagreements; no entry is a generic shape. For the health surfaces referenced throughout, see [Health checks and the heal stack](health-and-healing.md); for how the services are run, see [Running the stack as services](services.md).

## Kaneo cards move to done on merge, or issues close seconds after a merge

- **Symptom:** A pull request merge closes issues it merely referenced, or Kaneo cards jump straight to done on merge.
- **Cause:** The Kaneo GitHub integration's status transitions have drifted from the recorded settings. The upstream default sends merge → `done`, and that default closed merely-referenced issues seconds after merge. The recorded settings send merge → `in-review` (changed 2026-11-14, defect #7610). The settings live only in Kaneo's Postgres, in `integration.config.statusTransitions` per project; no repository file sets them, so drift has no diff to read.
- **Confirm:** Run the sync guard, `pm-stack/kaneo-sync-guard.py`. It asserts every integration's transitions: no target `done`; merge → in-review; open → in-review; push → in-progress.
- **Fix:** Let the guard heal the drift — it rewrites the transitions in compact-JSON form and exits 1 loudly so the trip is on record. The `pm-kaneo-sync-guard` timer runs it hourly, and `deploy.sh` runs it at deploy time. Then check the wrongly closed issues against the closure watchdog's record (next entry) before reopening anything by hand.

## An automation close is reopened

- **Symptom:** An issue closed by automation is open again, with no human reopen in the timeline you expected.
- **Cause:** The closure watchdog, `pm-stack/closure-watchdog.py`, reopens an automation close when the issue still has open sub-issues, or when the closing pull request carried no closing keyword (#10105). That is the watchdog working, and its usual trigger is the transition drift in the entry above.
- **Confirm:** Read the close event: the closing pull request's keywords, and the issue's sub-issue states at close time. Keyword closes and human closes are untouched by the watchdog; an untouched close rules this cause out.
- **Fix:** Fix the underlying state, then close again in a form the watchdog accepts: complete or detach the open sub-issues, or close through a pull request carrying the closing keyword. Do not disable the watchdog to make a close stick — the close it reopens is the defect it exists to catch. Note its scheduler: the watchdog runs from a Muse sandbox cron job, not from a laptop timer.

## A closed issue carries open children, or an open issue carries `status:done`

- **Symptom:** The invariant sweeper reports a closed issue with non-completed sub-issues, an open issue carrying `status:done`, or a Kaneo↔GitHub state mismatch.
- **Cause:** The GitHub record and the Kaneo record diverged — a transition fired on one side and not the other, or a close landed before its subtree completed. These are the sweeper's three invariant classes (#10139), and the first class is the same invariant the epic closure gate enforces from the aggregator side.
- **Confirm:** `pm-stack/invariant-sweeper.py` in its default, report-only mode names the class and the item for each finding.
- **Fix:** Run the sweeper with `--act` for the first two classes; it repairs them under the closure watchdog's guards. The third class, the state mismatch, stays report-only in every mode: reconcile it through the sync guard and the source records, not through the sweeper.

## An epic will not close

- **Symptom:** An epic stays open after its visible work is done, and the closure gate lists blockers.
- **Cause:** The epic closure gate, `evaluateEpicClosureGate` in `aggregator/src/epics.ts`, holds an epic open while its complete subtree has open children or unmet conditions. The gate enumerates blockers by condition; an unmet condition in the subtree is the cause, not the gate.
- **Confirm:** Run `scripts/epic-closure-gate.ts` and read the enumerated blockers. `pm-stack/fleet-monitor/epic_close_check.py` reports the same state weekly, report-only: a blocked epic there is a finding, not a failure.
- **Fix:** Clear the named conditions — finish, detach, or reparent the open children (`aggregator/src/epic-reparent.ts`, checked by `scripts/check-epic-conformance.ts`). Do not close the epic over the gate; the closure watchdog and the sweeper treat that close as a defect and reopen or report it.

## The claimant stops launching, or a bound session sits stuck

- **Symptom:** Queue work accumulates, no new claimant fires appear, or a session bound to a task makes no progress.
- **Cause:** The claimant's fire sequence in `pm-stack/opencode-claimant/claimant.py` runs supervision and recovery of stuck bound sessions first, then the candidate gate, then the concurrency cap (36 claimant roots). A stuck bound session consumes its root until supervision recovers it; a full cap stops new fires; a candidate that fails the gate in `poll.py` never reaches launch.
- **Confirm:** Read the fire log, `~/.local/state/opencode-claimant/fires.jsonl`, for the last fire and its step. Read the claimant mode file in the host-health snapshot on `/health`. The claimant cap step also fires the health hooks under the caller name `lane-launch`; the hook incident log at `~/.local/state/pm-aggregator/health-hook-incidents.jsonl` records what that step found.
- **Fix:** Match the step that stopped. A stuck bound session: let the supervision and recovery step run its course, or work the task at once through `launch_one.py` (#8187) once the root frees. A cap stop: wait for roots to free; the cap is a fixed limit, not a fault. A gate stop: fix the candidate's gate failure — the task's queue state — rather than the claimant. The surveys do not record a manual unstick procedure for a bound session beyond the supervision step and `launch_one.py`; a procedure past those two is UNVERIFIED.

## A phone answer goes nowhere: the relay routes return 404

- **Symptom:** An answer sent to the notify relay's edge routes — `POST /mirror/event`, `GET /answer/&lt;ledgerId&gt;`, `POST /api/needs-drew/&lt;ledgerId&gt;/answer` on the relay — returns 404, or the relay sends nothing when the laptop is silent.
- **Cause:** Those routes are not routed, and the relay is not deployed (D5). `pm-stack/notify-relay/src/index.ts` states it in its header: the mirror route arrives with step 2 of the relay build and the answer routes with step 3; everything else 404s. `pm-stack/platform/workers.json` records `pm-notify` as "not deployed" with a null address. The relay README's route table lists the unrouted routes beside the live ones, separated only by a "Lands in" column — the table reads as availability and is not.
- **Confirm:** Read the registry entry for `pm-notify` in `workers.json`, and the header of `notify-relay/src/index.ts`. Both say the same thing; the README table is the outlier.
- **Fix:** Route the answer to the laptop aggregator. Phone answers land through the aggregator's own `POST /api/needs-drew/&lt;ledgerId&gt;/answer` and the decision-set pages, as in [Tutorial: answer a waiting question end to end](../tutorials/answer-a-question.md). There is no operator fix that makes the relay live; deploying it is build work (relay steps 2–3 and the deploy step), not a setting.

## The quota view disagrees with the provider

- **Symptom:** `/quota` shows a pool or service in a state the provider's own console contradicts, or a section-risk judgement rests on a pool state that no longer holds.
- **Cause:** The view reads collected snapshots, and a snapshot is dated. The notification code states the adjoining rule for its own reads: pools in `error`, `stale`, or `lapsed` states never count as serving — a stale read distorts both the page and every judgement joined to it. Browser-dependent sources add a second staleness path: the exporters' watchdog core classifies source failures and climbs a repair ladder instead of serving the last good read as current.
- **Confirm:** Refresh through `POST /quota/refresh` (or `POST /api/quota/refresh`) and re-read `/api/quota.json`. Check the collection scripts' last runs on `/health`, which reports last job runs and feed section ages.
- **Fix:** Take the post-refresh read as the view's state. If the refresh itself fails, the fault is in collection — the gather scripts and, for browser-dependent sources, the exporter watchdog's failure class and ladder rung in `pm-stack/exporters/src/watchdog-state.ts`, with the rung-4 hand-off in `watchdog-escalation.ts` (incident issue first, then a queue task, then the NEEDS DREW line). Do not patch the page's numbers by hand; figures carry dates and live with the dated reads. See [Tutorial: read the quota and usage views](../tutorials/read-the-quota-view.md).

## Worker counts disagree: 18, 20, 22

- **Symptom:** Three documents give three Worker counts: the platform README says eighteen, `workers.json` lists 20, and the programme census counts 22.
- **Cause:** Staleness in the README, and two different counting scopes (D6). `workers.json` lists 20 Workers, counted directly, and its own test, `workers.test.ts`, holds the JSON to the repository's wrangler files — the JSON is the live list. The README's "eighteen", "fourteen answered", and "two Actions-deployed" figures are stale as of the file's own checked date, 2026-11-23. The census count of 22 covers the estate scope of its ledger, a wider scope than the platform registry.
- **Confirm:** Count `workers.json` and run its test. Read the census ledger's Table A for its scope.
- **Fix:** Cite the count with its source: 20 in the platform registry, 22 in the census ledger's estate scope. Correct the README figure in the platform docs; do not edit `workers.json` to match the README — the test holds the JSON to the wrangler files, and the JSON is right.

## The board is not the fork the program page describes

- **Symptom:** Work planned against fork-only Kaneo behaviour does not appear on the deployed board, or the board's version does not match the fork.
- **Cause:** Image drift between the program text and the deployment (D1). `aggregator/src/program.ts` describes the board as "our fork (developing-today-forks/kaneo)". `pm-stack/compose.yml` pins the upstream image `ghcr.io/usekaneo/kaneo:2.47.0` by digest. The fork exists as a repo; compose does not run it.
- **Confirm:** Read the image line in `pm-stack/compose.yml`. The deployment file is the authority.
- **Fix:** Plan against upstream v2.47.0 behaviour for the deployed board. Moving the board to the fork is a compose change plus the sync guard's verification pass, not a documentation correction.

## A decision set answers through an API its manifest page denies, or its page is missing from health

- **Symptom:** A reader of `pages.ts` concludes the decision sets have no API, or `/health` shows no render record for a decision-set detail page that plainly renders.
- **Cause:** Two manifest gaps (D7, D8). The `decisions` entry in `pages.ts` declares no `api:` twin — one of only three pages without one — while `server.ts` implements the full decisions API. And `SUBROUTE_PARENTS` omits `/decisions/&lt;setId&gt;`, so `manifestRouteFor` returns null for it and its renders sit outside the #1147 render registry `/api/health` reports.
- **Confirm:** Read the routes in `aggregator/src/server.ts` for the API, and `SUBROUTE_PARENTS` in `pages.ts` for the missing sub-route.
- **Fix:** For the reader: use the server as the authority for the decisions API — submit, defer, request-info, and events per set, plus the decision-sets list and submit routes. For the registry gap: the fix is a manifest change (add the sub-route parent), not a health-check change. Until that change lands, a missing render record for `/decisions/&lt;setId&gt;` is the known gap, not an outage.

## A push class is missing, or the count of classes is wrong in a document

- **Symptom:** A document or issue speaks of six notification classes, or of a seventh class as the final state, and the preferences page shows more.
- **Cause:** Stale premise text (D14). Issue #8882, now closed, describes six classes in `notifications.ts`. The code defines eight in `NOTIFICATION_CLASS_DEFAULTS`: `incident`, `needs-drew`, `quota-wall`, `quota-near-wall`, `section-risk`, `stack-health`, `blocked-age`, `prompt`. `section-risk` is the class #8882 added, and `prompt` (#10035) followed it.
- **Confirm:** Count the keys of `NOTIFICATION_CLASS_DEFAULTS` in `aggregator/src/notifications.ts`, and compare with the toggles on `/alerts/preferences`.
- **Fix:** Cite eight, from the code. A subscription-side check belongs here as well: whether the #8882 subscription flip (section-risk on, quota-wall and quota-near-wall off, on the owner's phone) was applied to the live subscription is UNVERIFIED from the repository — it is subscription state in the Turso alerts store, not code. Read the subscription row before reporting the flip as done.

Sources. survey/apps.md §1, §12, §15, §19, §31; survey/push.md §2–§3; survey/website-pages.md §D; survey/disagreements.md D1, D5, D6, D7, D8, D14. Ground truth: laptop clone at 4d937b3, 2026-11-23.
