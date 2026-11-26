# #15340 criterion 1 — verification of #2174 monitoring gaps 2 and 4

Verification of record for the first acceptance criterion of
developing-today/code#15340 (migrate the new-issue-creator lanes to
OpenChamber scheduled tasks, #2174 wave 1):

> Verify the state of #2174 monitoring gaps 2 and 4 before starting
> (gaps 1 and 3 are recorded merged in the #14222 inventory); do not
> start wave migration until the gating demonstration on a pilot is
> confirmed.

Compiled 2026-11-26 by an xaltha lane (Muse sandbox seat) against
`origin/main` at `4e12377c666ff499581ff813de53bd99aa068993`, the
#14222 inventory record
(`doc/2026-11-26T02-43-19Z_inventory_cron-wrappers-opencode-migration/`),
and the GitHub record for #2174, #2072, and the PRs named below.
Every repo-side claim below is re-checkable with the companion test:

```bash
python3 -m unittest discover -s doc/2026-11-26T07-46-31Z_verification_15340-monitoring-gaps -p 'test_*.py'
```

## Verdict summary

| Gate item                                                                                                                           | Verdict                                                                                                                                                                                                                                                                                  |
| ----------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Gap 2 — token accounting (OpenCode-side exporter, #2072 work)                                                                       | **CLOSED** — exporter built, merged, on a timer, and its source is the one the Gap 1 collector already enriches live rows from                                                                                                                                                           |
| Gap 4 — server residency (OpenChamber server always-on)                                                                             | **DEPLOYMENT SHAPE CONFIRMED, LIVE CONFIRMATION OUTSTANDING** — the server is an enabled systemd user unit with no socket-activation or idle-stop mechanism in this repo, but no live `systemctl` confirmation is on record, and the laptop was unreachable from this seat on 2026-11-26 |
| Pilot demonstration (#2174 acceptance: ONE queue lane as an OpenChamber scheduled task for 48h, reviewed against its Muse baseline) | **NOT CONFIRMED** — no 48h pilot record exists on #2174 or on the pilot's owner, #15341, as of this check                                                                                                                                                                                |
| **Gate consequence for #15340**                                                                                                     | **Do not port any new-issue-creator lane yet.** The next slice is the live Gap 4 check plus the #15341 pilot, not a creator port                                                                                                                                                         |

## Gap 2 — token accounting: CLOSED

#2174's gap 2: every OpenCode assistant message carries cost + tokens,
but nothing exported it; the fix is "an OpenCode-side exporter twin of
runtime-db-export (this is #2072 work)".

Evidence:

- The exporter exists and is merged: PR #4680,
  "feat(pm-stack): OpenCode usage exporter — pull from live history DB
  (#2072 slice 1)", merged 2026-11-03. It lives in
  `pm-stack/opencode-usage-export/` (exporter, README, laptop install
  script, systemd `.service` + `.timer` — a 15-minute tick).
- It reads only the live v2 tables (`session_v2`, `session_message`)
  of `~/.local/share/opencode/opencode.db` and fails loudly
  (`store-missing` / `store-stale` / `schema-drift`, exits 3–6) rather
  than reporting silent zeros — see that directory's README,
  "The live store — read this before touching the path".
- It is not dead code: the Gap 1 per-run-history collector
  (`pm-stack/scheduled-history/collect.py`) enriches each scheduled
  run's row with token/cost from the same `session_v2` store, keyed by
  the run's session id, and that enrichment was live-verified on
  #2174 on 2026-11-11 (first tick inserted 24 rows, all 24
  token-enriched). Scheduled-task runs — the exact population the
  wave-1 comparison needs — therefore already carry token accounting.
- Scope note: the #2072 tracker issue stays open for its slices 2–7
  (webhooks, integrated views, rollups, projections). Those are not
  gap 2. Gap 2 named only the exporter, and the exporter is landed.

## Gap 4 — server residency: shape confirmed, live check outstanding

#2174's gap 4: "the scheduler dies with the OpenChamber server
process; confirm the laptop deployment is always-on (not
socket-activated or idle-stopped)."

Repo-side evidence (all re-checked by the companion test):

- The deployment enables the server as a standing service:
  `home/common/default.nix` sets `services.openchamber = { enable =
true; … }` in the service cascade (2026-11-03), whose module comes
  from the OpenChamber fork's `nix/modules`.
- `doc/fleet/services.md` ("The long-running services: OpenChamber,
  OpenCode, mcpx") describes OpenChamber as one of the three services
  the flake keeps running, and `pm-stack/PORTS.md` records
  `openchamber.service` (user unit, port 3000) as **live**.
- The unit is built to survive deploys without bouncing:
  `nixos/home/stable-entrypoints.nix` (#14187, noted 2026-11-25)
  force-overrides its `ExecStart` to the stable path
  `%h/.local/state/openchamber/start` and sets
  `X-RestartIfChanged = false`.
- No socket activation: there is no `.socket` unit for OpenChamber
  anywhere in this repo, and no idle-stop mechanism for it is
  recorded in `doc/fleet/services.md` or the schedule registry.

What is **not** on record: a live confirmation from the laptop —
`systemctl --user show openchamber.service` (active state, substate,
active-enter timestamp spanning a pilot window) and the absence of a
live `openchamber.socket`. The laptop broker tunnel was down from
this seat on 2026-11-26 (queue/shell legs failing, the #15131 class of
outage), so that check could not be run in this pass. Until it is,
gap 4 is not closed — it is one live command away from closing, and
that command is named in "Next slice" below.

Discrepancy flagged: #15341's body asserts "#2174 gating confirmed:
… gaps 2 and 4 verified". For gap 2 that now has the record above;
for gap 4 no verification record existed before this document, and
this document finds the live half still outstanding. #15341's pilot
should not treat gap 4 as verified until the live check lands.

## Pilot demonstration: NOT CONFIRMED

#2174's acceptance is explicit: gaps 1–4 closed **and demonstrated on
one pilot** — one queue lane running as an OpenChamber scheduled task
for 48h with per-run history, token accounting, and watchdog
coverage, reviewed against its Muse-cron baseline — before a
go/no-go for the rest of wave 1. The pilot belongs to #15341
(repo-queue worker shifts). As of 2026-11-26, neither #2174 nor
#15341 carries a 48h pilot record or a baseline review. The additive
OpenChamber queue lanes recorded in the registry (Surface 2a) prove
scheduled lanes _run_; they are not the measured 48h pilot the gate
names.

## What this means for the remaining #15340 criteria

- Criterion 2 (port one creator lane as a measured A/B port) and
  criterion 3 (port the remaining lanes) stay gated. Starting them
  now would repeat the failure the gate exists to prevent: migrating
  on top of an unconfirmed residency claim and an unmeasured pilot.
- Criterion 4 (disposition for the disabled unpinned
  `new-issue-creator` / `new-issue-creator-b` jobs): the inventory's
  disposition is **retire — if still idle when #15340 starts**, and
  the registry still records both as disabled (last full census
  2026-11-03). "Still idle" is a live-scheduler fact of the same kind
  as gap 4's missing check, so the retirement confirmation rides
  with the same next slice rather than being asserted here.
- Criterion 5 (bookkeeping locality, gap 5) is untouched by this
  verification and remains its own slice.

## Next slice

After this PR, `systemctl --user show openchamber.service -p
ActiveState,SubState,ActiveEnterTimestamp` (plus `systemctl --user
list-unit-files 'openchamber*'`) run on the laptop shows gap 4
**closed** with the timestamps recorded on #15340 — or names the
residency defect to fix. That, plus the #15341 48h pilot, is the
whole remaining gate before the code-a A/B port.

## Limits

- Live laptop state (systemd units, OpenChamber stores, the primary
  seat's scheduler) was not readable from this seat on 2026-11-26;
  nothing above claims a fresh live read of it.
- GitHub facts (PR #4680 merged 2026-11-03; the 2026-11-11 live
  enrichment note on #2174) are quoted from the GitHub record, not
  re-measured here.
- Registry lane states are "as recorded" (last full census
  2026-11-03), the same limit the #14222 inventory states.
