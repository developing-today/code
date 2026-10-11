# Task 08: Connectivity extensions

- Status: n0 relays are done (feature doc). Everything else here is designed, and the relay kit is ready. Our relay needs the owner's accounts and a host, and a deploy needs explicit go-ahead.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: owner decisions (below).
- Design: [connectivity design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T10-46-58Z_design_connectivity.md) (all sections). Kit: [connectivity/relay](../2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay/). Evidence: C1 in the [spikes](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).

## Goal

n0 is the default for relays and lookup. Our own relay is added as a fallback and as a route independent of n0. Alternate lookups (DHT, a Worker pkarr relay) are opt-in. Tickets carry addresses for invites and handoffs.

## Scope

1. **Our relay, option A (recommended).** Cloudflare Tunnel in front of `iroh-relay` on a machine the owner controls. About an hour. Steps are in connectivity §4.1. Verify with `curl https://relay.<domain>/ping` (200) and `/generate_204` (204), then run C1 t7 against the public URL.
2. **Our relay, option B.** A VPS with a public IP, using the kit's `vps.toml` and the systemd unit. Steps are in connectivity §4.2. This adds direct paths through QUIC address discovery.
3. **Per-relay QUIC flag in the product.** The C1 module already has it (`connect.rs`, `RelayEntry { url, quic }`, `RelayConfig::new(url, None)` for relays with no UDP). Port it into `pkgs/id/src/connectivity.rs`. Keep n0's relays in every client's map, even when our relay is the home relay. This is an inference, to be tested under item 9.
4. **Mainline DHT, opt-in.** `iroh-mainline-address-lookup` 0.6.0 (which depends on `n0-mainline`, not `mainline`). Use `.no_publish()` for lookup only. Publish relay URLs only (verified in C1 t3). The record is public, and anyone who knows your ID can read it. Off by default.
5. **Worker pkarr relay, opt-in, later.** A Worker that serves PUT and GET on `/<z32>`, with Durable Objects for storage and WebCrypto Ed25519 for verification. About 150–200 lines of JavaScript, plus 150–250 for DNS over HTTPS. Use the Paid plan (CPU limits).
6. **mDNS.** Off by default in the connectivity design, but `serve` enables it today. It binds `0.0.0.0:5353` on every interface, and iroh-mdns has no interface option. Decide the default. If it stays on, document the LAN exposure.
7. **Tickets.** Carry the home relay and one alternate relay. No IPs by default. A ticket has no expiry, so the invite event carries it. Refresh tickets when home servers re-home.
8. **Access control and limits.** For our relay: `access.shared_token` (verify the key names against the `iroh-relay` source before enabling), an endpoint allowlist, and the rate limits under `[limits.client.rx]` (verify). The kit comments these out until verified.
9. **Tests still to run.** C2: holepunching between two peers on separate networks, both homed on our relay, with n0's relays in their maps. Re-run C1 t2 and t4 on current infrastructure. A test for MemoryLookup pruning.
10. **Metrics and certificates.** Metrics stay on loopback (the kit does this). Let's Encrypt: test with staging (`prod_tls = false`) before production, and persist `cert_dir`.

## Decisions

- Which relay first: A (recommended) or B. Accounts needed for each (connectivity §8).
- Access control: shared token (recommended) or open.
- DHT default: off (recommended).
- mDNS default: off (design) or on (current `serve`). See item 6.
- Ticket policy: relay list only, no IPs (recommended).

## Acceptance

- C1 t1–t8 pass against the configured relays (already true on localhost).
- The public relay answers `/ping` and `/generate_204`, and a client test passes against it with QUIC discovery off.
- A dial on a peer whose lookup stalls fails within the deadline.
- Holepunching test (C2) recorded for each network pair.

## Gotchas

- `RelayMode::custom(urls)` turns QUIC discovery on for every relay. A relay with no UDP then costs 3 s at startup (C1 t8). Use the per-relay flag.
- A relay that is not the home relay closes after 60 s idle, so a ticket that names it goes stale.
- n0's relays are for development and hobby use. They have no SLA, and a Pro shared relay is $19 a month (per the iroh blog, as read). Production should lean on our relay.
- Cloudflare sees client IPs when it fronts a relay. It cannot read payloads.
- The iroh repository's docs are out of date (ports and config examples). Use the code's values.

## Files

`pkgs/id/src/connectivity.rs` (extend), `pkgs/id/src/commands/serve.rs` (mDNS flag). Kit files stay in the doc folder until a deploy.

## Dependencies

`iroh-relay` 1.3.0 with `server` (for hosting only, not for clients). `iroh-mainline-address-lookup` 0.6.0 (opt-in, behind a feature if possible). Report both per AGENTS.md.

## 2026-10-11T12-20-38Z Update: connectivity answers and re-runs

- n0 stays the default. Tickets: yes, carrying relays, not direct addresses only. Tunnel first for our own relay.
- Relay access control (#13): no shared token now, since the relay is public. A non-secret token embedded in the binary is an option to revisit at implementation.
- Bare node-ID lookup stays on. The owner decided this over my privacy recommendation.
- t2 and t4 re-run, both pass. t2: re-home in 22.5 s, earlier runs 20.5 s and 21.6 s, so the range is 20–23 s. Fresh handshake over n0: 547 ms. t4: dial by ID through n0, handshake 216 ms, two throwaway IDs published. Raw output is in `spikes/c1-rerun`.
- Pending: S11 (relay operation runbook) and C2 (holepunching across namespaces). Open: mDNS default (#14), DHT default (#15).

## 2026-10-11T12-49-09Z Update: relay operation (S11)

- Restart: the QUIC connection survived every SIGTERM, SIGKILL and SIGINT run with a 1 s outage (3 of 3 each). With 3 s down, the 3 s SIGKILL runs also survived (3 of 3). No in-flight frame was lost in a surviving run.
- Home relays were back 1.2 to 1.7 s after the kill, and echoes resumed at 1.7 to 1.9 s. With 3 s down, echoes took 5.6 s.
- A 35 s outage (one run) timed the connection out at 35 s. The peers returned at +45 s and +61 s, and 1,058 of 1,062 in-flight frames were lost.
- SIGTERM behaves like SIGKILL, because the relay does not handle SIGTERM. The kit's systemd unit uses `KillSignal=SIGTERM`, so systemd hard-kills it. SIGINT exits cleanly with code 0. Recommend `KillSignal=SIGINT`; not run under systemd.
- Relay key: iroh-relay 1.3.0 has no relay key. Trust is TLS against the CA roots you configure, matched to the URL host. Nothing is pinned. A new key under the same CA is invisible to clients (back online in 268 ms). A CA change breaks CA-pinned clients, and the endpoint shows no error; only the relay logs `UnknownCA`.
- Certificates: manual mode needs a restart (verified). Reload mode uses a fixed 24 h timer, not measured; it ignored a file change for 5 s.
- Idle: a non-home connection closed exactly 60.0 s after its last datagram, and stayed open while QUIC keep-alives ran. The home relay never idled out over 3 min 40 s.
- Operations hazards: a missing `--config-path` file silently starts the relay on `[::]:80`, with metrics on `[::]:9090`. Unknown keys, including the old `secret_key`, are ignored without a warning. Metrics on port 9090 answer on any path, so keep it off the public interface. `/ping`, `/healthz` and `/generate_204` work. With `RUST_LOG` unset the relay is silent.
- Peers' reconnect backoff reached 27 s after a long outage, and the application cannot reset it.
- Not tested: real networks, systemd, the 3 to 35 s outage boundary, the 24 h reload, the home relay's ping cadence, real tickets.

## 2026-10-11T12-52-41Z Update: holepunching across namespaces (C2)

- Full cone and port-restricted NATs: holepunch succeeds, 3 of 3 runs each. The direct path forms 40 to 321 ms after dial, and holds for the whole 60 s window.
- Symmetric against full cone works in both orders. Symmetric against port-restricted falls back in both orders. Symmetric against symmetric falls back (3 of 3). Fallback carried every echo, with none failed.
- QAD provides the public candidate. With no QAD relay in the map, no direct path forms, even behind a restricted NAT. A QAD-only stand-in on the same host gives a direct path in 39 ms. n0 could not be reached from the lab, so the n0 part is untested.
- Relay-only tickets hold no IP addresses, but candidates travel inside the QUIC connection. A connected peer therefore learns our LAN and public addresses. `addr_filter` applies only to address-lookup publishing (N30).
- A symmetric NAT gives a QAD address that is valid only toward the relay, so it is useless for holepunching.
- A dead relay entry costs about 3 s at startup, from iroh's net-report probe timeout. A lost first holepunch round costs up to 5 s.
- Not tested: n0's own relays (no uplink from the lab), real NAT devices, double NAT, WAN latency, NAT timeouts beyond 60 s, TLS on the public relay, real Cloudflare Tunnel.
- Next: a real-network test with two separate networks, with n0's QAD in the map (N29).
