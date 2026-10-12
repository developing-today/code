# Phase 7: connectivity for later servers

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 12.
- Start commit: 68d6aebc (origin/main, after Phase 6 #197).
- Base branch `phase-7`. Tracks run in parallel:
  - `p7-relay`: P7.1 relay kit for a server behind a Cloudflare Tunnel, and for VPS testing (updated for the S11 findings).
  - `p7-dht`: P7.3 DHT opt-in per world (decision 15).
- Not started: P7.2 Cloudflare Workers pkarr option (design only; needs owner's go). P7.4 access control (decision 13: none now). P7.5 deployments and real-network tests (owner-blocked: accounts and networks).
- Phase 8 items each become their own phase document when they start (P8.1 to P8.6); P8.7 BeeKEM only if its conditions are met (they are not); P8.8 lints: do nothing now.

## Steps

| Step | Track | Status                | Commits | Tests | Notes                 |
| ---- | ----- | --------------------- | ------- | ----- | --------------------- |
| P7.1 | relay | todo                  |         |       |                       |
| P7.2 | -     | blocked on owner's go |         |       | design only           |
| P7.3 | dht   | todo                  |         |       |                       |
| P7.4 | -     | none now              |         |       | decision 13           |
| P7.5 | -     | owner-blocked         |         |       | accounts and networks |
