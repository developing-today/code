# Verification — #2174 monitoring gaps 2 and 4, for #15344 (pool-quota-steward)

Recorded 2026-11-26 by xaltha (fleet lane, hourly work-loop run
2026-11-26 01:41 CST). This record satisfies acceptance criterion 1 of
#15344: the current state of #2174 monitoring gaps 2 and 4 is verified
and recorded before any migration work starts. Gaps 1 and 3 were
already merged per the #14222 inventory (PR #8437 and PR #11158) and
are not re-verified here.

**Verdict: the #2174 gate is NOT passed. Both gaps remain open in the
gating sense, so the `pool-quota-steward` migration must not start.**
Per the intake note on #15344, the migration stays blocked on the
gaps rather than the port starting. The remaining criteria of #15344
(replacement design, parallel run, cron retirement) are downstream of
this gate and are not attempted here.

## Gap 2 — token accounting (#2072 exporter work): exporter built, scheduled-task accounting NOT closed

What #2174 requires: an OpenCode-side exporter twin of
runtime-db-export, so OpenCode token usage is accounted — and, for the
pilot, token accounting demonstrated on an OpenChamber scheduled task.

Evidence (all read 2026-11-26):

- The exporter exists on main: `pm-stack/opencode-usage-export/`
  (`opencode-usage-export.mjs`, `opencode-usage-export.test.mjs`,
  systemd `.service`/`.timer` pair, `install-laptop.sh`, README). Its
  README names it "#2072 slice 1": a read-only pull exporter over
  OpenCode's own history database, verified 2026-11-02 against the
  running managed instance on the laptop (live store
  `~/.local/share/opencode/opencode.db`, v2 tables; the legacy tables
  frozen 2026-10-06 are explicitly never read). It is maintained on
  main as recently as 2026-11-25 (commit `9394e0d86fa7`, PR #14750).
- #2072 itself is still OPEN (`status:in-review` at last check
  2026-11-24). Its slices 1–4 are recorded as delivered (slice 4's
  outcome attribution landed via PRs #12709, #13022, #13065), and the
  tracker stays open for its remaining slices.
- The decisive point for #2174: the exporter is **not yet joined to
  scheduled-task run history**. The Gap 1 store
  (developing-today-forks/openchamber PR #2082) records `tokens: null`
  until that join exists, and on 2026-11-22 Felix verified live that
  **5,000 of 5,000 run-history rows carry `tokens: null`** and
  recorded that fact on #2072 as **slice 5 input** — every row carries
  its OpenCode `sessionId`, so the join key to the usage export
  exists, but the join itself has not landed. No merged PR found on
  2026-11-26 closes that join.

So gap 2 is partially closed (the exporter #2174 named is built and
live) but **not closed in the gating sense**: a pilot scheduled task
still cannot be token-accounted from its run-history record.

Next step that closes it: #2072 slice 5 — join run-history rows to
the usage export on `sessionId` so scheduled-task runs carry real
token totals, then demonstrate on one run.

## Gap 4 — server residency: structural evidence only, live confirmation still missing

What #2174 requires: confirm the laptop OpenChamber deployment is
always-on — the scheduler dies with the server process, so it must
not be socket-activated or idle-stopped.

Evidence (all read 2026-11-26):

- On current main the laptop OpenChamber runs as a **systemd user
  unit** owned by Home Manager: `doc/fleet/services.md` (note dated
  2026-11-25, #14187) records `openchamber.service` exec'ing the
  stable path `%h/.local/state/openchamber/start`, and
  `nixos/home/stable-entrypoints.nix` sets that unit's `ExecStart` to
  the stable link with `Unit."X-RestartIfChanged" = false` — a NixOS
  switch does not bounce the service; redeploys go through the link
  plus an explicit restart. The start wrapper is
  `bin/openchamber-start.sh`.
- A code search of this repo on 2026-11-26 found **no
  socket-activation unit** for OpenChamber (no `.socket` unit naming
  it among the hits for the service).
- What is **not** recorded anywhere found: that the user unit is
  enabled and currently active on the laptop, that lingering is on
  for the laptop user (a user unit stops at logout without it), or
  the unit's restart policy. Those are live-laptop facts, not repo
  facts.

Limit: the laptop was unreachable from this seat at verification
time — the fleet queue/shell legs were down (broker tunnel failure,
the #15131 flapping incident), the same limit the #14222 inventory
recorded. No live `systemctl --user` / `loginctl` read was possible,
and none is claimed.

So gap 4 is **unverified, not closed**: the deployment shape is
consistent with always-on, but #2174 asks for confirmation, and
confirmation needs one live laptop check.

Next step that closes it: from a seat with laptop access, record on
#15344 or #2174 the output (or a faithful summary) of
`systemctl --user is-enabled openchamber.service`,
`systemctl --user is-active openchamber.service`,
`loginctl show-user <laptop user> -p Linger`, and the unit's
`Restart=` policy.

## Consequence for #15344

- Criterion 1 (verify gaps 2/4 before migration starts): **done** —
  this record.
- Criteria 2–4 (replacement design, parallel run, retirement):
  **blocked on the gate**, in order — first #2072 slice 5 (gap 2
  join), the live residency check (gap 4), and then #2174's 48h pilot
  demonstrating gaps 1–4 together. The steward's change-ledger and
  impact-follow-up duties still need their explicit home, but
  designing that home before the gate passes would put the
  replacement ahead of the monitoring #2174 requires to watch it.

## Sources

- Issue #15344 body and intake comment (2026-11-26T04:23Z).
- Issue #2174 body (gaps list and pilot acceptance) and its
  2026-11-17 per-gap status comments.
- Issue #2072 body, direction comment (2026-10-29), and progress
  comments through 2026-11-24, including the 2026-11-22
  run-history `tokens: null` verification.
- `pm-stack/opencode-usage-export/README.md` and commit history on
  main (read via the GitHub API, 2026-11-26).
- `doc/fleet/services.md`, `nixos/home/stable-entrypoints.nix`,
  `bin/openchamber-start.sh` on main (read via the GitHub API,
  2026-11-26).
- PR #15351 (merged 2026-11-26T03:01Z): the #14222 inventory record
  this verification follows on from.
