# Health, quota and usage surfaces

## Scope of this chapter

This chapter describes surfaces: the pages, stores, and jobs that report health, quota, and usage. It quotes no quota figures. Figures — pool percentages, dollar amounts, reset times as live values — belong to the LLM-plans chapters, which own them as dated reads. A threshold that defines a notification class is a rule, not a figure, and appears in [Push notifications: detection to phone to answer](push-notifications.md), not here.

## The health page and its twin

`/health` and `/api/health` are one assembly: a single health context feeds both (`aggregator/src/server.ts`). The page reports live probes for every stack service, feed section ages, last job runs, this dashboard's own render history, and the services-and-ports topology.

The sources named in code are `aggregator/src/health.ts`, `stack-health.ts`, `host-health.ts`, `build-health.ts`, `watchdog*.ts`, and `slo*.ts`, plus one component outside the aggregator package: the host-health collector (`pm-stack/host-health-collector.py`), a laptop user service on a 5-minute timer (`pm-host-health` service and timer). The collector writes the JSON snapshot the aggregator derives watchdog tier items from (#10141). It exists because the packaged aggregator cannot collect host state itself; the snapshot carries the claimant mode file, the backup last-run record, and user-unit and timer states for pm-git-sync, pm-projector-reconcile, pm-kaneo-sync-guard, and pm-dashboard-backup.

`/api/health` also returns the render registry: one entry per manifest page with its route, last rendered time, and ok flag (#1147). One page is outside that registry by a manifest gap: `/decisions/\<setId\>` is missing from `SUBROUTE_PARENTS`, so decision-page renders are not counted there (D8; see [Question sets and the answer-back flow](questions-and-answers.md)).

## Health hooks: failure-triggered diagnosis

The health hooks (`pm-stack/health-hooks/`) are a Python runner (`run.py`) with two wrappers: `pm-stack-reachable.sh` and `opencode-working.sh` (#9571). Their contract is narrow:

- **Trigger.** The aggregator's stack sentinel fires a hook once on a transition to 3 consecutive failures, and once on recovery. Two secondary callers fire the same hooks fire-and-forget: the claimant cap step and `launch_one.py` in lane-launch, and the aggregator live source (`aggregator-tick`).
- **Action.** A hook run is read-only diagnosis and report. No remediation ships in the hooks.
- **Record.** Incidents append as JSONL to `~/.local/state/pm-aggregator/health-hook-incidents.jsonl`, and the runner makes a best-effort post to the alert throat with the `stackHealth` extension. One shared 60-second deadline bounds the run.

A hook incident enters the same alert pipeline as every other incident; its path from throat to phone is traced in [Push notifications: detection to phone to answer](push-notifications.md).

## Report-only monitors

fleet-monitor (`pm-stack/fleet-monitor/`) holds two Python monitors. Both are report-only: a finding is a report, never an automatic repair and never a failure state.

- `epic_close_check.py` is the weekly adapter for the aggregator's epic closure gate. A blocked epic is a finding, not a failure. The gate itself is described in [The roadmap hierarchy: initiatives, projects, epics, aggregators](roadmap-hierarchy.md).
- `review_loop_checks.py` is the second monitor; the survey names it and its report-only character, and this chapter claims no more than that.

Both monitors carry tests. Their last commit in the surveyed clone is 2026-11-22.

## The quota and usage pages

Two dashboard pages carry quota and usage:

- **`/quota`** (Usage & billing), twin `/api/quota.json`, with a per-service detail page at `/quota/service/\<id\>` and refresh posts at `POST /quota/refresh` and `POST /api/quota/refresh`. The page reports token quotas per pool, pay-as-you-go dollars, and every SaaS service's usage against its limits. The machinery behind it spans the `quota-*.ts` module family (current state, forecast, anomaly, reweight, admission, weekly review), the charges family (`charges-catalog`, `charges-collect`, `charges-match`), spend attribution and rollups, and the gather and forecast scripts. Reweight output is a proposal, recorded as a record or a pull request; it changes no order by itself (see [Prioritisation: how order is decided and recorded](prioritisation.md)).
- **`/aa`** (Cost Explorer), twin `/api/aa.json`, holds two tabs: the AA Explorer (Artificial Analysis measurements, frontier charts, and plan, API, and per-task efficiency) and the Model Explorer (every model in the fleet's catalogues, with pool seats, plan, and measured value).

The console carries its own smaller equivalents: the `#more/platform`, `#more/releases`, `#more/machines`, and `#more/services` views, drawn from the platform registry and the ops layer.

## The jobs behind the quota surfaces

Four exporter and checker components feed the pages above. None of them renders a page; each writes a store or a snapshot that a page reads.

- **quota-check** (`pm-stack/quota-check/`) is one helper: `turso-mirror-read.py` (#6828). It reads the fleet-platform and fleet-mirror Turso databases per database over the libsql HTTP interface, with per-database tokens fetched by secret name and a single fallback. It explicitly does not use the Turso CLI's org token.
- **provider-usage** (`pm-stack/provider-usage/`) is `check-usage.py`: a provider-usage and OpenCode fleet snapshot written for the "Jules keeper job", which consults the snapshot when choosing a failover model. `reconcile.py` and tests accompany it. Its snapshot paths point into a goal workspace, which marks it as a Muse-side job's tooling kept in the repository.
- **Browser-dependent exporters** (`pm-stack/exporters/`) are the watchdog core for quota sources reachable only through a logged-in website (#6032). The decision core is built and tested: failure classes and a repair ladder in `watchdog-state.ts`, and the top-rung hand-off in `watchdog-escalation.ts` (incident issue first, then a queue task, then a NEEDS DREW surfacing line). The only adapter present is a synthetic fixture; Mistral is named as the first real adopter, with the Claude console and Open Design named as later adopters. Live per-provider production use is UNVERIFIED: only the fixture adapter ships.
- **Dedicated exporters.** `pm-stack/cloudflare-export/` (`cloudflare-quota-export.mjs`, service and timer) exports Cloudflare quota and usage into the quota system. `pm-stack/meta-exporter/` (`meta-quota-export.mjs`, service and timer) exports Meta quota and usage; its README records a flow limitation — an absent tab yields a null pay-as-you-go balance field. opencode-usage-export reads the OpenCode session store into the same surfaces and is treated in [Scheduled jobs and the history they keep](scheduled-jobs.md).

## Reading the surfaces together

The three families answer three different questions, and this book keeps them apart. Health surfaces answer "is it up, and when did it last render and run". Quota surfaces answer "what is consumed, against which limit, under which proposal". Usage surfaces answer "what ran, and what did the session store record". A red health probe is an incident input to the push pipeline. A quota state is an input to prioritisation proposals and to two notification classes. Neither family writes the other's record.

Sources. Survey `apps.md` §§12, 15, 17, 21–23, 27–28, 30; survey `website-pages.md` §§C–D; survey `disagreements.md` D8; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
