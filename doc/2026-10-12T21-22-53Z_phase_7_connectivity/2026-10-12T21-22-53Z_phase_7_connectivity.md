# Phase 7: connectivity for later servers

- Plan: [phased plan](../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 12.
- Start commit: 68d6aebc (origin/main, after Phase 6 #197).
- Base branch `phase-7`. Tracks run in parallel:
  - `p7-relay`: P7.1 relay kit for a server behind a Cloudflare Tunnel, and for VPS testing (updated for the S11 findings).
  - `p7-dht`: P7.3 DHT opt-in per world (decision 15).
- Not started: P7.2 Cloudflare Workers pkarr option (design only; needs owner's go). P7.4 access control (decision 13: none now). P7.5 deployments and real-network tests (owner-blocked: accounts and networks).
- Phase 8 items each become their own phase document when they start (P8.1 to P8.6); P8.7 BeeKEM only if its conditions are met (they are not); P8.8 lints: do nothing now.

## Steps

| Step | Track | Status                | Commits                      | Tests                                                                            | Notes                         |
| ---- | ----- | --------------------- | ---------------------------- | -------------------------------------------------------------------------------- | ----------------------------- |
| P7.1 | relay | todo                  |                              |                                                                                  |                               |
| P7.2 | -     | blocked on owner's go |                              |                                                                                  | design only                   |
| P7.3 | dht   | done (endpoint-wide)  | 1c91f76a, 5249c60a, 61f7e00c | dht_opt_in 3 (red: E0432 at 1c91f76a); dht unit 4; dht_testnet 2 (`dht` feature) | see note 2026-10-12T21-36-10Z |
| P7.4 | -     | none now              |                              |                                                                                  | decision 13                   |
| P7.5 | -     | owner-blocked         |                              |                                                                                  | accounts and networks         |

## 2026-10-12T21-36-10Z Note: DHT opt-in (P7.3)

- **Scope: endpoint-wide, not per world.** iroh 1.3.0 fixes an endpoint's address lookups when it is bound. `AddressLookup::publish` runs once for the endpoint's own address, and `resolve(endpoint_id)` gets no world. So one opted-in world turns the DHT on for every world the endpoint serves, and the endpoint publishes its record to the DHT. Per-world scoping would need an endpoint per world or a DHT client outside iroh. Neither is built.
- **Off by default.** `DhtPolicy::Off` mounts nothing, so the endpoint neither publishes nor resolves through the DHT. `DhtPolicy::from_worlds` gives `Endpoint` only when at least one world opts in.
- **Opt-out** takes effect on the next session (the next bind). A record already published stays on the DHT until its TTL ends.
- **Publish and resolve**, as P7.3 asks. Task 08 item 4 says `.no_publish()` (lookup only). That conflicts with P7.3 and is left for the owner to decide: publishing is what makes a peer findable by ID.
- **Code:** `src/dht.rs` (`DhtPolicy`, `with_lookup`, and `mainline()` behind the `dht` feature).
- **Tests.** `tests/dht_opt_in.rs` (default build) runs the three cases over a recording `MemoryLookup` standing in for the DHT. `tests/dht_testnet.rs` (`dht` feature) runs the same cases over the real `iroh-mainline-address-lookup` on a local `n0-mainline` `Testnet`. Neither touches the public DHT. iroh 1.3.0 ships no test DHT, so the testnet is the test DHT.
- **Dependencies:** `iroh-mainline-address-lookup` 0.6.0 (optional, feature `dht`, brings `n0-mainline` 0.7.1). `n0-mainline` 0.7 is also a dev-dependency for the testnet. The default build has neither in its graph. `Cargo.lock` also re-resolved `data-encoding-macro-internal` from `syn` 1.0.109 to 3.0.8 (its requirement is `>= 1, < 4`; a proc macro, no runtime effect).
- **Not done:** no product caller. `serve` reads no DHT setting yet. The wiring point is `dht::with_lookup(&policy, builder, dht::mainline())`. `mainline()` uses the public bootstrap, which no test contacts. Public-DHT runs are owner-blocked (P7.5).
- **Verified.** Red at 1c91f76a: `unresolved import id::dht` (E0432). Green: `dht_opt_in` 3/3, `dht_testnet` 2/2 (three repeat runs), `--lib connectivity` 13 passed (1 ignored, needs internet), `--lib dht` 4/4, `relay_ops` 9/9, full `--lib` 1255 passed (4 ignored). Mutant (`with_lookup` always mounts): `dht_opt_in` fails the off-by-default test, and `dht_testnet` fails the opted-out session. Clippy: no new warnings in the new files.
