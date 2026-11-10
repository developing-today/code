# iroh-relay 1.3.0: self-hosted relay (research notes)

Read-only research. Nothing deployed, no accounts, no DNS. Sources: crate tarballs for iroh-relay 1.3.0, iroh 1.3.0, iroh-base 1.3.0, iroh-metrics 1.0.2, tracing-subscriber 0.3.23, iroh-tickets 1.0.0 (static.crates.io), docs.iroh.computer, Docker Hub and GitHub APIs, Fly, Cloudflare, Let's Encrypt and Hetzner docs. Extracted source: /tmp/research-relay/src/.

**Answer.** Run one relay on a small VPS with a public IPv4 address, using the relay's built-in Let's Encrypt. It is the simplest option with the full stack: TCP relay on 443, QUIC address discovery (QAD) on UDP 7842, and hole punching. Clients should use `RelayMode::Custom` with n0's four production relays plus ours. There is no STUN server; QAD does that job [read]. Caveats: n0's public relays are for dev and hobby use, with no SLA; a peer advertises one home relay, so "reachable through our relay alone" needs that peer on our relay only; and the server logs almost nothing unless `RUST_LOG` is set [read].

## Deployment options (effort is my estimate)

| Option | Effort | Monthly cost | Owner provides | What works |
|---|---|---|---|---|
| A. VPS, public IPv4 (Hetzner CX23) | 30–45 min | €5.49 excl. VAT, IPv4 extra [read]; 20 TB EU traffic [read] | payment method, domain, ports 80/443 TCP and 7842 UDP | Relay, QAD, hole punching |
| B. Home server, port forwarding | 1–2 h | ~$0 + power; upload caps relay traffic [inferred] | domain, static IP or DDNS, no CGNAT | Full, if forwarding works |
| C. Home server, Cloudflare Tunnel | 30–60 min | $0 on Free [snippet] | domain on Cloudflare DNS; no card on Free [snippet] | Relay only; no QAD [read] |
| D. Fly.io container | 45–90 min | $2.19–3.69 machine, +$2 IPv4 for UDP, +$0.02/GB out [read] | card, no free tier [read]; domain | Relay; QAD unverified |

## (a) Server

- **Services** [read: server.rs:12-16, 736-740]: HTTP :80 (captive-portal `/generate_204` only when TLS is on, full relay when off). HTTPS :443 serves `/relay` (WebSocket), `/ping`, `/generate_204`, `/healthz`. QAD on UDP :7842, opt-in, needs TLS. Metrics on TCP :9090, on by default.
- **Config**: TOML only, `iroh-relay -c file.toml`. The only flags are `--dev` (plain HTTP on :3340) and `--config-path` [read: main.rs:47-60].
- **TLS**: ACME is simplest; validation is TLS-ALPN-01 on 443 [read: http_server.rs:1095]. Let's Encrypt allows 5 certs per identical name set and 50 per registered domain per 7 days [read], so persist `cert_dir` and use `prod_tls = false` while testing. `cert_mode = "Reloading"` serves your own certs [read: main.rs:673].

```toml
http_bind_addr = "[::]:80"
enable_quic_addr_discovery = true
metrics_bind_addr = "127.0.0.1:9090"   # default [::]:9090 is public [read]

[tls]
https_bind_addr = "[::]:443"
hostname = "relay.example.org"         # owner: domain
cert_mode = "LetsEncrypt"
cert_dir = "/var/lib/iroh-relay/certs"
contact = "ops@example.org"            # owner: email
```

- **Image**: Docker Hub `n0computer/iroh-relay`, tags through v1.3.0 [read]; GitHub v1.3.0 tarballs [read]. Build recipe unchecked (no root Dockerfile in the iroh repo, 404).
- **Resources**: no official sizing. Default key cache is 1,048,576 entries, about 56 MB [read: defaults.rs:18-23]. Bandwidth dominates [inferred].
- **Logging**: stderr, filtered by `RUST_LOG`; unset means ERROR only [read: main.rs:570; tracing-subscriber 0.3.23].
- **Access**: `everyone` (default), endpoint allowlist or denylist, `shared_token` (Bearer header or `?token=`), or HTTP callout [read: main.rs:160-197].
- **Rate limits**: per-connection receive bucket in `[limits.client.rx]`, off by default; `accept_conn_*` is not enforced [read: server.rs:486-499].

## (b) Without UDP

- Clients still connect: the relay path is WebSocket over TLS on TCP [read: client/conn.rs].
- Home relay selection uses HTTPS `/ping` latency, so it needs no UDP [read: report.rs:70-72; reportgen.rs:821].
- No QAD means no public (reflexive) addresses, so hole punching has no NAT candidates. Same-LAN direct paths still work [read: socket.rs:1834; inferred].
- Logs: QAD failures and timeouts are `debug!`; net_report.rs has no info or warn for lost UDP [read]. Try `RUST_LOG=iroh::net_report=debug` [inferred].

## (c) Client, iroh 1.3.0

- `RelayMode::Custom(RelayMap)`. A RelayMap holds many `RelayConfig`s: URL, QUIC port (default 7842; `None` disables QAD), optional auth token [read: relay_map.rs:230-310].
- **Home relay**: lowest best latency over 5 minutes; switch only if the new best is at most two-thirds of the current home's latency. Periodic re-report every 20–26 s, and on relay-map change [read: net_report.rs:749-822; socket.rs:1760, 2000-2015].
- **EndpointAddr** = `{ id, addrs }`, with `TransportAddr::Relay(url)` and `Ip(addr)` [read: iroh-base endpoint_addr.rs:42-57]. `endpoint.addr()` gives the home relay plus IPs; `online()` waits for a connected home [read: endpoint.rs:1215, 1374].
- **Tickets** are not in iroh 1.3.0; `iroh-tickets` 1.0.0 wraps `EndpointAddr` as `endpoint::EndpointTicket` [read].
- **Caveat**: non-home relay connections close after 60 s idle [read: actor.rs:68, 524], so a ticket naming a relay you no longer home on goes stale.

```rust
// not compiled
use iroh_tickets::endpoint::EndpointTicket;
let ours: RelayUrl = "https://relay.example.org".parse()?;
let relays = RelayMode::Default.relay_map();   // n0's four
relays.insert(ours.clone(), Arc::new(RelayConfig::from(ours)));
let ep = Endpoint::builder(presets::Minimal)   // no n0 DNS lookup
    .relay_mode(RelayMode::Custom(relays))
    .bind().await?;
ep.online().await;
let ticket = EndpointTicket::from(ep.addr()).to_string();
```

## (d) n0 relays

- **URLs** [read: defaults.rs:26-33]: `https://use1-1.relay.n0.iroh.link.`, `usw1-1`, `euc1-1`, `aps1-1`. Staging: `use1-1.staging-relay…`, `euc1-1.staging-relay…`.
- `RelayMode::Default` uses the four. The `N0` preset also publishes to and resolves through n0's pkarr/DNS at `dns.iroh.link` [read: presets.rs:117-137; pkarr.rs:127].
- **Policy** [read: docs public-relays, rate-limiting]: dev and hobby use; no SLA; no version lock; per-connection limits, unpublished; n0 may block IPs; relays see IPs, timing and byte counts. Pro shared relay: $19/mo, 100 GB included, 5 MB/s throttle, authenticated [read: iroh blog].
- **SLA**: Enterprise only [read]. **Status page**: none in the docs; guessed hostnames don't resolve [read].
- **Our own discovery with n0 relays**: yes. Use `presets::Minimal` and set the relay mode yourself; tickets carry addresses [read: presets.rs:61-79; dial-by-ticket inferred].

## (e) Steps

- **A**: create the VM; A record `relay.<domain>`; open 80/443 TCP and 7842 UDP; install the binary or image; apply the config; run under systemd with `CAP_NET_BIND_SERVICE` [inferred]; check `/healthz`.
- **B**: as A, plus router forwards and DDNS; confirm no CGNAT.
- **C**: no `[tls]`, no QAD, `http_bind_addr = "127.0.0.1:3340"`; `cloudflared` public hostname to that address; clients use `RelayConfig::new(url, None)`. Cloudflare says Free, Pro and Business plans must use a paid service for large files [read], which a bulk-transfer relay may hit.
- **D**: `fly.toml` with TCP 80/443 and no handlers, so the relay does its own ACME [read: Fly services]; `fly ips allocate-v4` for UDP. Fly binds UDP to `fly-global-services`, but iroh's `quic_bind_addr` is a `SocketAddr` [read], so QAD is unverified.

## (f) Recommendation

Deploy **A** first, with (a)'s config and (c)'s client code. Owner decisions:
1. Domain, subdomain, provider, payment method.
2. Certs: ACME (recommended) or `Reloading`.
3. Access: open, `shared_token`, or allowlist (tokens via `RelayConfig::with_auth_token`).
4. `[limits.client.rx]` rate, to cap egress.
5. Whether n0's free relays are acceptable in production. n0 says dev and hobby; Pro shared is the supported path.
6. Whether ticket issuers should run on our relay only.

**Doc discrepancies** [read]: `docs/local_relays.md` shows an old config (`secret_key`, `addr`); `docs/relays.md` says 3 relays (code has 4); the README gives QAD as 7824, code says 7842. I used `main.rs` and `defaults.rs`.

**Not verified**: nothing built or run; the Rust snippet is uncompiled; Hetzner's IPv4 price, Cloudflare's card rule (snippet only), and QAD on Fly are unconfirmed.
