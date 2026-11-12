# Turso + Cloudflare idea engine

Living document · Status: shipped (2026-10-20) · Register entry T3 · Tracking epic: developing-today/code#8805

## Purpose

- Produce a steady supply of concrete, buildable ideas that use
  the Turso $5 plan and/or Cloudflare $5-tier features.
- Keep each idea inside Drew's budget rules.
- Advance a standing ledger by one small rotating step per run.
- This document covers the engine only, not its individual ideas.

## What landed (features, and how each works)

- A standing cron named `turso-cloudflare-idea-engine`.
- Each run produces ONE concrete buildable idea.
- Each run also advances a standing ledger by one rotating step:
  - (a) check one service's usage / limits / spend, or
  - (b) find one new free/cheap compute/storage/egress offer, or
  - (c) note hits/geo/latency for a property.
- Budget rules recorded for the engine:
  - free tiers first
  - $5 plans in play
  - $25–30 "maybe later"
  - $75 out
- First asked: 2026-10-20, recorded in the cluster-goal setup.
- The first idea was delivered 15:01 CDT the same day.
- Interval change: Drew instructed the cron be doubled the same
  day, from every 8 hours to every 4 hours.
- Output volume at window end: 131 dated idea entries
  (2026-10-20 → 2026-11-11), plus 131 rotating ledger-step entries.
- All 131 idea titles are preserved in the design register's
  Appendix A; they are not documented individually here.

## Rough architecture

- Vehicle: the `turso-cloudflare-idea-engine` cron.
- Home: under the lab-cluster goal; no GitHub issue.
- Artifacts named in the register:
  - the cron task body
  - `~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/idea-register-2026-10-20.md`
  - `hidden_files/cloud-ideas-log.md` in that goal
- Triage in the register: (b) DONE-WITHOUT-ISSUE for the engine
  itself — it exists and runs.

## Status

- design_status in the register: built-into-running-system.
- implementation_status in the register: shipped.
- Still running at window end; the 131st idea is dated 2026-11-11.
- Finding, stated plainly: the logs record no pickups.
  - The register contains zero `issues/` links for the ideas.
  - The ideas log contains no pickup/implemented markers.
  - Every idea is therefore pickup-unverified in the record, and
    parked, unless Drew approves a build.

## Next steps — immediate / near-term / long-term

- Immediate: none recorded; ideas wait for Drew's approval.
- Near-term: not recorded.
- Long-term: not recorded.

## Open follow-ups

- Every idea's recorded "first step" remains unstarted unless
  Drew approves a build — by the engine's design, ideas are
  parked unless Drew approves.
- No pickup tracking is recorded in either log.

## Future ideas not yet specced

- The engine's own output is the future-ideas stream; individual
  ideas are registered in Appendix A and not specced here.

## Links (register entry, related issues/PRs named in the register entry)

- Register entry: T3, in the design register (REGISTER.md).
- Register Appendix A: all 131 idea titles, grouped by theme.
- Cron: `turso-cloudflare-idea-engine` (under the lab-cluster goal).
- Tracking epic: developing-today/code#8805.
- Tracking in the register: NEVER-ISSUED (no GitHub issue).
