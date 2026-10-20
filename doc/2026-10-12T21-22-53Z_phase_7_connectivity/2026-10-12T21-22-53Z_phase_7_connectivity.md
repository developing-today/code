# Phase 7: connectivity for later servers

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 12.
- Start commit: 68d6aebc (origin/main, after Phase 6 #197).
- Base branch `phase-7`. Tracks run in parallel:
  - `p7-relay`: P7.1 relay kit for a server behind a Cloudflare Tunnel, and for VPS testing (updated for the S11 findings).
  - `p7-dht`: P7.3 DHT opt-in per world (decision 15).
- P7.2 Cloudflare Workers pkarr option: design complete; implementation deferred pending explicit owner approval. See [design note](../2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md). P7.4 access control (decision 13: none now). P7.5 deployments and real-network tests (owner-blocked: accounts and networks).
- Phase 8 items each become their own phase document when they start (P8.1 to P8.6); P8.7 BeeKEM only if its conditions are met (they are not); P8.8 lints: do nothing now.

## Steps

| Step | Track | Status                                   | Commits                      | Tests                                                   | Notes                                                                                                  |
| ---- | ----- | ---------------------------------------- | ---------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| P7.1 | relay | done                                     | 80feb48b, ff844396           | relay_ops: 21 (14 red before the mode)                  | tunnel and vps modes in relay.kit.toml                                                                 |
| P7.2 | -     | design complete; implementation deferred |                              | none (design-only)                                      | [design note](../2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md); no deployment/account access |
| P7.3 | dht   | done (server-wide)                       | 1c91f76a, 5249c60a, 61f7e00c | dht_opt_in 3; dht unit 5; dht_testnet 2 (`dht` feature) | server-wide --dht on serve; publish enabled                                                            |
| P7.4 | -     | none now                                 |                              |                                                         | decision 13                                                                                            |
| P7.5 | -     | owner-blocked                            |                              |                                                         | accounts and networks                                                                                  |

## 2026-10-12T21:40:00Z Note: P7.1 relay kit modes

- The kit has two modes in `connectivity/relay/relay.kit.toml`: `tunnel` (plain HTTP on 127.0.0.1:3340, no TLS table, QAD off, no unit drop-in) and `vps` (HTTP on [::]:80, TLS on [::]:443, QAD on, drop-in `iroh-relay.service.d/vps.conf`). Metrics stay on 127.0.0.1:9090 in both.
- The shared `iroh-relay.service` no longer grants `CAP_NET_BIND_SERVICE`. A unit grants it only if its mode binds a port below 1024, so the tunnel unit grants nothing.
- Red first: 14 of 21 `relay_ops` tests failed on the missing kit (commit 80feb48b). Green after the mode (ff844396): 21 of 21.
- Lib suite on the head: `cargo test --lib`, 1251 passed, 0 failed, 4 ignored.
- The S11 findings applied: `KillSignal=SIGINT` in both modes, `ConditionPathExists` for the config, metrics on loopback, no `secret_key`. Not applied: a systemd test (S11 did not run under systemd).
- Design connectivity §4.2 step 4 now names the vps drop-in, so a VPS install gets the port capability.
- Owner-blocked (P7.5): running either mode on a real host, with a Cloudflare Tunnel or a VPS, and checking `/ping`, `/generate_204`, and the systemd KillSignal behaviour. Not deployed here.

## 2026-10-12T22:05:00Z Note: P7.3 DHT server-wide config and publishing decisions

- The owner decided: "dht can be server wide config. sure, publish. yes skip beekem for now."
- Scoping: `DhtPolicy::Server` (`from_server`) adds server-wide DHT enablement. `--dht` flag wired into `id serve` (mirrors `--no-mdns`).
- Publishing: `iroh-mainline-address-lookup` builder keeps publishing enabled (default).
- BeeKEM: P8.7 remains skipped as decided.
