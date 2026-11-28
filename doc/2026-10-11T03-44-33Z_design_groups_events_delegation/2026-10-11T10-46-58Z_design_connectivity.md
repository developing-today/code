# Connectivity: n0 by default, our relay, tickets, and alternates

- Date: 2026-10-11 (UTC 10:46)
- Status: owner choice recorded. The choice is n0 by default, plus our own relay, plus invite tickets, plus alternate discovery such as the mainline DHT. Deploying our relay waits on the owner's account and host decisions (§8). No product code changed.
- Supersedes: the Minimal recommendation in the [design review](./2026-10-11T08-08-56Z_design_review.md), §3.6 and §6 items 2 and 3.
- Evidence: connectivity spike [C1](./spikes/shared/c1-connect/RESULTS.md) (tests t1–t8, run in this sandbox), and the relay kit in [connectivity/relay/](./connectivity/relay/).

## 1. Summary

- **Zero-setup default: n0.** The N0 preset resolves a bare node ID through n0's lookup service and relays through n0's relays. C1 t4 dialed by ID with no ticket, in 277 ms.
- **Easiest relay of our own: a Cloudflare Tunnel in front of `iroh-relay` on a machine you control.** About an hour, $0 on Cloudflare, and no open inbound port. We checked the server config here: it starts and serves `/ping` (200), `/generate_204` (204), and `/healthz` (200). A client on that relay, with QUIC discovery off, connects in 17–25 ms (t7).
- **A relay on Cloudflare itself is not easy.** Workers cannot run the `iroh-relay` binary. A Worker would have to reimplement the wire protocol, about 400 lines plus a BLAKE3 library, in one to two days. A Cloudflare Container runs the stock binary, but it needs Workers Paid (a card) and costs about $7–12 a month.
- **Alternates:** the mainline DHT is opt-in, and t3 showed its stored record contains no IP addresses. mDNS is off, because it binds every interface and publishes every IP to the LAN. A Worker pkarr relay is a later opt-in.
- **Tickets** carry the home relay and one alternate relay, with no IP addresses by default. A ticket is a snapshot, so it can go stale (§7).

## 2. The model

Four layers, each with its own switch.

| Layer        | What it does                                                    | Default                                                                | Switch                        | What it reveals                                                                     |
| ------------ | --------------------------------------------------------------- | ---------------------------------------------------------------------- | ----------------------------- | ----------------------------------------------------------------------------------- |
| Relays       | Carries packets when there is no direct path                    | n0's four production relays, plus ours                                 | The relay list in the profile | Relay operators see IPs, timing, and byte counts. Payloads are encrypted end to end |
| Lookups      | Turns a node ID into addresses                                  | Tickets (always). n0 pkarr publish and resolve, and n0 DNS (N0 preset) | Lookup list in the profile    | n0 sees your ID, your relay, your IP, and each ID you dial                          |
| Tickets      | Carries a snapshot of addresses for invites and handoffs        | Home relay and one alternate relay. No IPs                             | Ticket builder policy         | The holder learns the relays and anything else included                             |
| Direct paths | Holepunching, using addresses from QUIC address discovery (QAD) | On, through relays that run QAD                                        | Per-relay QAD flag            | The relay operator sees your public IP                                              |

Two things the table does not show:

- **QAD is per relay.** A client learns its public address from relays that run QUIC address discovery. So keep n0's relays in every client's map, even when our relay is the home relay. Our Tunnel relay has no UDP, so it gives no public address. This is an inference from the net-report design. It has not been tested, because it needs two peers on separate networks.
- **Profiles** bundle the switches:
  - `default`: n0 relays plus ours; tickets; n0 pkarr and DNS; relay-only policy.
  - `private`: our relay (and optionally n0 as a fallback); tickets only; no bare-ID lookup; no DHT.
  - `dht`: as `default`, plus the mainline DHT, publishing relay URLs only.
  - `lan`: as `default`, plus mDNS. Warn the user that every LAN host learns the node ID and all IPs.

## 3. Measured here (C1, localhost and the public n0 network; not Cloudflare)

| Test   | What it checks                                                 | Result                                                                                                                                                   |
| ------ | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| t1     | Our relay only. Ticket with one relay and no direct address    | Pass. Handshake 13 ms                                                                                                                                    |
| t2     | Fallback. Local relay killed; ticket lists local and n0 relays | Pass. n0 handshake 0.5–1.3 s. The peer re-homes after 20–22 s, which is the real failover limit. Not re-run after the QUIC-flag change; it passed before |
| t3     | Mainline DHT on a local testnet                                | Pass. The stored record has the relay URL and no IP literal, though the publisher had direct addresses. Resolve and dial 80 ms                           |
| t4     | n0 by ID, no ticket, real internet                             | Pass. Handshake 277 ms over `use1-1`. One run. It published two throwaway test IDs                                                                       |
| t5     | Precedence between lookups                                     | The correct relay always wins. A failing address listed first costs about 1 s. A failing address alone holds the dial for 30 s                           |
| t6     | Negatives                                                      | No lookup: fails in under 0.4 ms. Unknown ID on the testnet: fails in about 3 ms                                                                         |
| t7     | Tunnel-style relay, QAD off, `http://127.0.0.1:3340`           | Pass. Echo over the relay, 17–25 ms, zero direct addresses                                                                                               |
| t8     | Wrong flag: QAD on against a relay with no UDP                 | The dial still works, in 16–34 ms. Both endpoints take 3.0 s to come online instead of 0.2 s. Nothing at warn level                                      |
| Server | `tunnel.toml` loaded by `iroh-relay` 1.3.0                     | `/ping` 200, `/generate_204` 204, `/healthz` 200, metrics 200 on loopback                                                                                |

Not run: mDNS, because it binds `0.0.0.0:5353` on every interface and has no interface option. Not tested here: Cloudflare, a VPS, a Container, a Worker, and NAT holepunching across real networks.

## 4. Our relay: setups, ranked by ease

Effort and cost figures come from the research passes. Check current prices before buying.

| Option                                                                                | Effort                                          | Cost                                                                                                     | What the owner provides                                                                        | Direct paths  | Caveats                                                                                                                                     |
| ------------------------------------------------------------------------------------- | ----------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Cloudflare Tunnel in front of `iroh-relay` on a machine you control                | About 1 h                                       | $0 on Cloudflare (Tunnel is on all plans). The machine is extra                                          | Cloudflare account, a domain on Cloudflare DNS, an always-on machine with outbound connections | None (no QAD) | Cloudflare sees client IPs. Free, Pro, and Business plans restrict large-file traffic under the CDN clause, so bulk relaying is a grey area |
| B. VPS with a public IPv4 address, `iroh-relay` with built-in Let's Encrypt           | 30–45 min                                       | About €5.49 a month excl. VAT for Hetzner CX23, per the listing read at research time, plus the IPv4 fee | Provider account, payment, a domain, open ports 80 and 443 TCP and 7842 UDP                    | Yes (QAD)     | Let's Encrypt allows 5 certificates per name set per week. Persist `cert_dir`                                                               |
| C. Cloudflare Containers running the stock binary                                     | 2–4 h                                           | About $7–12 a month, including the $5 base                                                               | Workers Paid (a card), a machine that can build Docker images                                  | None (no UDP) | Sleeps after 10 minutes idle unless a socket stays open. Restarts happen                                                                    |
| D. Worker reimplementing the relay protocol (Durable Objects, hibernating WebSockets) | 1–2 days, about 400 lines plus a BLAKE3 library | $0 at light use on the Free plan. Paid is $5 a month for heavier use                                     | Cloudflare account and `wrangler` login                                                        | None          | We own and maintain the wire protocol. Unverified: the WebSocket subprotocol echo. Deploys drop sockets                                     |
| E. Home server with port forwarding                                                   | 1–2 h                                           | About $0 plus power                                                                                      | Static IP or DDNS, and no CGNAT                                                                | Yes (QAD)     | Upload speed caps relayed traffic                                                                                                           |

**Recommendation.** Start with A. It is the fastest route to a working relay, it costs nothing extra on Cloudflare, and it needs no open port. Move to B when we want direct paths through our own QAD, or when the box is not always on. Choose D only if we refuse to run any machine.

Fly.io (a container with a public IP) is a further option. Its cost is $2.19–3.69 a month for the machine, plus about $2 for an IPv4 address and $0.02 a GB out. QAD on Fly is unverified.

**Traffic cost.** A 100 MB transfer in packets of 1.2–1.4 KB is 71,000–87,000 inbound WebSocket messages. Cloudflare bills inbound WebSocket messages at a 20:1 ratio, so the transfer is $0 on either plan. Sustained rates above about 500 messages a second are risky, because the Durable Object soft limit is 1,000 requests a second.

### 4.1 Steps for option A

1. Put the domain on Cloudflare DNS, or move its nameservers there.
2. Install `iroh-relay` 1.3.0 with the server feature. Use `cargo install iroh-relay --version 1.3.0 --features server --locked`, or the `n0computer/iroh-relay` image on Docker Hub.
3. Copy [`connectivity/relay/tunnel.toml`](./connectivity/relay/tunnel.toml) to `/etc/iroh-relay/relay.toml`. Run the binary under systemd with [`iroh-relay.service`](./connectivity/relay/iroh-relay.service).
4. In the Cloudflare dashboard, create a tunnel. Install it with `sudo cloudflared service install <TOKEN>`. Add a public hostname `relay.<domain>` that points at `http://localhost:3340`.
5. Check it: `curl -s https://relay.<domain>/ping` should return 200, and `/generate_204` should return 204.
6. In clients, build the relay with `RelayConfig::new("https://relay.<domain>", None)`, so QAD is skipped.

### 4.2 Steps for option B

1. Create the VPS and point an A record for `relay.<domain>` at its address.
2. Open 80 and 443 TCP, and 7842 UDP.
3. Copy [`connectivity/relay/vps.toml`](./connectivity/relay/vps.toml). Set the hostname and contact. Keep `prod_tls = false` until the staging certificate issues, then set it to `true`.
4. Install the binary, the unit file [`iroh-relay.service`](./connectivity/relay/iroh-relay.service), and its drop-in [`iroh-relay.service.d/vps.conf`](./connectivity/relay/iroh-relay.service.d/vps.conf) (to `/etc/systemd/system/iroh-relay.service.d/`). The drop-in grants the port capability for 80 and 443. Start the service.
5. In clients, use the default relay configuration, which keeps QAD on.

## 5. Discovery alternates

| Service                                                                             | Default        | Privacy cost                                                                                                                | Switch                                                                              | Notes                                                                                                                                        |
| ----------------------------------------------------------------------------------- | -------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| n0 pkarr and DNS                                                                    | On (N0 preset) | n0 learns your ID, relay, IP, and each ID you dial                                                                          | Use the Minimal preset, or drop the lookup                                          | Publishes relay URLs only. Republishes every 5 minutes                                                                                       |
| Mainline DHT (`iroh-mainline-address-lookup` 0.6.0, which depends on `n0-mainline`) | Off, opt-in    | A public record per key, refreshed hourly. Anyone who knows your ID can read it, and DHT storage nodes see your UDP address | `.address_lookup(DhtAddressLookup::builder())`. Add `.no_publish()` for lookup only | Publishes relay-only by default (verified in t3). Bootstrap nodes can be replaced. The dependency is `n0-mainline`, not the `mainline` crate |
| Worker pkarr relay (ours, later)                                                    | Off, opt-in    | Our host sees what n0 sees                                                                                                  | Relay URL                                                                           | About 150–200 lines of JavaScript, plus 150–250 for DNS over HTTPS. Use the Paid plan, because the Free CPU limit is 10 ms                   |
| mDNS                                                                                | Off            | Every LAN host learns the ID and all IPs. It binds all interfaces                                                           | Leave it out, or `.advertise(false)`                                                | Not run in this sandbox                                                                                                                      |
| In-memory lookup (tickets)                                                          | On             | None beyond what the ticket holds                                                                                           | Always on                                                                           | Entries never expire. Prune stale ones (§7)                                                                                                  |

## 6. What the code must do (not done yet)

The C1 module (`spikes/shared/c1-connect/src/connect.rs`) already has these pieces, except where marked:

- A per-relay QUIC flag: `RelayEntry { url, quic }` at `connect.rs:60`, built with `RelayConfig::new(url, None)` at `:70`, and the relay map assembled in `bind` at `:182`. `RelayMode::custom` cannot express this, because it turns QAD on for every relay.
- Lookups as switches, with mainline and mDNS off by default.
- A ticket policy: relay-only, so no IPs.
- Still to add in the product: a timeout on every dial, because pkarr HTTP has no timeout. Pruning for the in-memory lookup. A profile read from the user's config.

## 7. Gotchas

1. **An ID-only connect can hang.** Pkarr HTTP has no timeout. Wrap every dial in `tokio::time::timeout`.
2. **A failing address costs time.** Listed first, it costs about 1 s. Alone, it holds the dial for 30 s. Race or time out each path.
3. **Peers re-home slowly.** After their home relay dies, peers take 20–22 s to re-home (t2). Expect that delay.
4. **Tickets go stale.** A non-home relay connection closes after 60 s idle. A ticket that names a relay its owner no longer homes on is out of date. Tickets have no expiry, so put the expiry in the signed invite event, and refresh tickets when home servers re-home.
5. **In-memory lookup entries never expire.** Stale IPs stay until pruning forces them out. Set a pruning policy.
6. **Metrics default to a public address.** The `iroh-relay` metrics port defaults to `[::]:9090`. Keep it on loopback, as the kit does.
7. **QAD on for a relay with no UDP.** It costs 3 s at startup and logs nothing at warn level (t8). The per-relay flag exists to prevent this.
8. **Let's Encrypt limits.** Five certificates per name set per week. Persist `cert_dir`, and test with staging first.
9. **An open relay is free bandwidth for anyone.** Use `access.shared_token`, an endpoint allowlist, or the rate limits. Verify the key names against the `iroh-relay` source before enabling them. The kit's `[access]` block is commented out until then.
10. **n0's relays are for development and hobby use.** They have no SLA and may block IPs. A Pro shared relay costs $19 a month, with 100 GB and a 5 MB/s throttle, according to the iroh blog. Production should rely on our relay, with n0 as the fallback.
11. **Cloudflare sees client IPs** when it fronts our relay. It cannot read payloads, because iroh encrypts them end to end.
12. **The DHT record is public.** Relay-only publishing keeps IPs out of it, but the existence of the node and its relay is still public.

## 8. Owner decisions and inputs

1. **Which relay first.** Option A (Tunnel) is recommended. Option B is the alternative.
2. **What to provide.** For A: a Cloudflare account, a domain on Cloudflare DNS, and an always-on machine. For B: a VPS account, payment, and a domain.
3. **Access control.** Open, a shared token, or an allowlist. Recommended: a shared token shipped in our clients. It stops casual abuse, and it can be rotated if it leaks.
4. **DHT.** Off by default, opt-in (recommended).
5. **Ticket policy.** Relay-only, with no IPs (recommended). Each ticket carries the home relay and one alternate.
6. **Whether to port the C1 module into `pkgs/id` now.** Recommended after the relay exists, so the product tests use a real relay.

I will not create accounts, deploy, or change DNS without an explicit go-ahead, because those are outward-facing and cost money.

## 9. Next steps

1. Owner chooses option A or B and provides the inputs in §8.
2. Deploy the chosen option, with the owner's confirmation.
3. Smoke-test the public URL with `/ping` and `/generate_204`. Then run C1 t7 against it.
4. Test holepunching with two peers on separate networks, both homed on our relay, with n0's relays in their maps (§2). This is not testable in the sandbox.
5. Port the C1 module into `pkgs/id` with the §6 items, and add profile tests.

## 10. References

- C1 spike, with RESULTS and the API: [spikes/shared/c1-connect](./spikes/shared/c1-connect/).
- Relay kit: [connectivity/relay/](./connectivity/relay/): `tunnel.toml`, `vps.toml`, `iroh-relay.service`.
- `iroh-relay` 1.3.0 configuration (`src/main.rs`, `src/defaults.rs`): `http_bind_addr`, `enable_quic_addr_discovery`, `metrics_bind_addr`, `[tls]`, `[access] shared_token`. The iroh repository's own docs are out of date (§11).
- n0 relay URLs and defaults: iroh 1.3.0 `defaults.rs`. The N0 preset: `presets.rs`.
- Lookup crates: `iroh-mainline-address-lookup` 0.6.0, `iroh-mdns-address-lookup` 0.6.0, `n0-mainline` 0.7.
- BEP 44 (mutable items): https://www.bittorrent.org/beps/bep_0044.html. pkarr: https://pkarr.org.
- Cloudflare Tunnel, Containers, and Workers pricing: Cloudflare documentation, as read by the research passes. Check current pricing before relying on it.

## 11. Documentation discrepancies in the iroh repository

- `docs/local_relays.md` shows an old configuration with `secret_key` and `addr`.
- `docs/relays.md` says there are three n0 relays. The code has four.
- The README gives the QAD port as 7824. The code uses 7842.

This document uses the code's values.

## 2026-10-11T12-20-38Z Update: re-run of t2 and t4

- t2 re-home measured 22.5 s on this run (earlier runs 20.5 s and 21.6 s). The "20–22 s" in §3 and gotcha 3 should read 20–23 s.
- t2 and t4 both pass on the current build. Raw output is in `spikes/c1-rerun`.
- Not tested: real NAT. Both endpoints share one host.

## 2026-10-11T19-55-00Z Correction: mDNS on for serve

- `serve` enables mDNS by default, as the code did before this design. `--no-mdns` turns it off. This corrects "mDNS is off" in §1 and §5 for `serve`. It is decision 14 in the [remaining-work index](../2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md).
- The library default is still off. `connectivity::public_builder` adds no mDNS. A library caller opts in by adding `MdnsAddressLookup`, as `serve` does.
- The cost is the same as the table in §5 describes: every host on the local network learns the node ID and all IPs. The `serve --help` text says so.
- Tests: `test_serve_enables_mdns_by_default_and_no_mdns_turns_it_off` and `test_serve_help_documents_no_mdns` in `pkgs/id/tests/cli_integration.rs`.
