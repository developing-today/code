# Globalping outside-in availability watch

Living document · Status: shipped (2026-10-23) · Register entry X10 · Tracking epic: developing-today/code#8805

## Purpose

- Watch the fleet's public pages from the outside in.
- Detect a public page being down without relying on inside-only
  checks.
- Stay silent unless something is down.
- First asked: 2026-10-23 ~18:32 CDT, when Drew ordered the
  Globalping account and a routine-ping watch.

## What landed (features, and how each works)

- A Globalping account was created.
- Sign-in route recorded: GitHub SSO.
- An API token was filed (location/value not recorded here).
- A routine watch runs hourly probes:
  - HTTP probes of the fleet's public pages
  - ping probes of the fleet's public pages
- Probe network: Globalping.
- Alerting behaviour: silent unless something is down.
- Implemented directly as the `globalping-watch` cron.
- The register notes this is effectively cloud idea #19
  (outside-in watchdog) with Globalping as the probe network;
  see Appendix A, 2026-10-23.

## Rough architecture

- Account: Globalping, via GitHub SSO, with a filed API token.
- Scheduler: the `globalping-watch` cron.
- Cadence: hourly.
- Targets: the fleet's public pages (specific target list not
  recorded in the register entry).
- Output: no output when probes pass; a signal only on a down
  result, per the "silent unless down" rule.
- Tracking in the register: NEVER-ISSUED — no issue cited in
  the record; implemented directly as the cron.

## Status

- design_status in the register: built-into-running-system.
- implementation_status in the register: shipped.
- Account done and hourly watch live 2026-10-23 ~18:41 CDT.
- Still running at window end (2026-11-11).
- Triage in the register: (b) DONE-WITHOUT-ISSUE.

## Next steps — immediate / near-term / long-term

- Immediate: not recorded.
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- None recorded in the register entry.

## Future ideas not yet specced

- None recorded in the register entry.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: X10, in the design register (REGISTER.md).
- Register Appendix A, 2026-10-23: cloud idea #19 (outside-in
  watchdog), which this watch implements with Globalping.
- Cron: `globalping-watch`.
- Tracking epic: developing-today/code#8805.
- Tracking in the register: NEVER-ISSUED (no GitHub issue).
