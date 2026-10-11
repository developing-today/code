# n0 relays by default: one endpoint policy and dial timeouts

- Date: 2026-10-11 (UTC 11:08)
- Scope: `pkgs/id`, the endpoint builders and the dial sites.
- Source: owner request, 2026-10-11: "just do n0 relay to start, document the rest and we will address them all later". The rest is in the [remaining-work index](../2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md). The design is in the [connectivity design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md).

## Intent

- n0's relays and n0's lookup services (the N0 preset) are the default for every endpoint that talks to the wider network.
- `--no-relay` turns relays off, as it does now.
- The policy lives in one module, so later changes (our relay, alternate lookups) are one edit, not twenty.
- A dial by node ID cannot hang indefinitely. The pkarr HTTP lookup has no timeout (connectivity design §7.1).

## Current state

Checked by grep on `main` at `8d0d59e7`:

- Ten non-test sites build `Endpoint::builder(presets::N0)` by hand. Three of them honour `--no-relay` (`meta_client.rs`, `commands/world.rs`, `commands/serve.rs`). The rest have no flag.
- About 25 production sites call `endpoint.connect(...)` with no deadline.
- `serve` enables mDNS unless `--no-mdns` is given. The connectivity design says mDNS is off by default. This is a discrepancy. It is not changed here, because the owner scoped this work to n0 relays. It is listed in the remaining-work index as a decision.
- Local and test endpoints use `presets::Minimal` with relays disabled. They stay as they are.

## Plan

1. New module `pkgs/id/src/connectivity.rs`:
   - `RelayPolicy::{N0, Disabled}`, with `N0` as the default. `from_no_relay(bool)` maps the flag.
   - `public_builder(RelayPolicy)`: `Endpoint::builder(presets::N0)`. Disabled sets `RelayMode::Disabled`. N0 keeps the preset's own relay mode.
   - `DIAL_TIMEOUT` = 45 s. The C1 spike measured a dead address alone holding a dial for 30 s, so 45 s leaves room for lookup and handshake.
   - `within(limit, what, fut)` and `connect(endpoint, target, alpn)`, which wrap the lookup and the dial in the deadline.
2. Replace the ten N0 builders with `public_builder`, passing the `no_relay` flag where one exists. Remove the duplicated `if no_relay { relay_mode(Disabled) }` blocks.
3. Replace the production dial sites with `connectivity::connect`. Leave the protocol modules `envelope_net.rs` (which has its own 30 s timeout) and `artifact.rs` (pull) for a follow-up, unless the change is trivial there.
4. Tests:
   - Unit tests for the mapping, `within` (success, inner error, timeout), and a Disabled endpoint with no relay address.
   - One `#[ignore]` test that dials between two N0 endpoints by ID, for manual runs.
5. No new dependencies. Report any that become necessary.

## Acceptance

- `cargo test --lib --features world`: only the three known `world_compile` failures (they need `host.wasm`).
- `cargo test --features world --test cli_integration`, skipping the known hang (`test_world_module_shared_over_iroh_and_web_presentations`) and the known Roc test: all pass.
- The ignored N0 test passes when run by hand.
- `treefmt` is clean on the changed files. No `unwrap` or `expect` in non-test code.

## Out of scope

Our own relay, the mainline DHT, the Worker pkarr relay, mDNS defaults, ticket policy, and holepunching tests. Each is a task in the remaining-work index.

## Risks

- About 30 call sites change. Each change is mechanical, and the compiler and the test suite check them.
- A 45 s deadline changes behaviour only for dials that would otherwise hang.

---

## Update: results

Pending. This section is filled in after the change is built and tested.
