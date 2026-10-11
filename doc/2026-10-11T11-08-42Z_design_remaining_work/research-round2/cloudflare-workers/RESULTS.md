# Research round 2: Cloudflare Workers, Tunnel, and relay options

Folder: `/home/user/work/research-round2/cloudflare-workers/`
Date of reads: 2026-10-11 (Cloudflare docs fetched 16:07Z; crates fetched the same session; iroh source from the local cargo registry).
Scope: research only. No account created, nothing deployed, no write to any service. Public read-only fetches only.

Labels used below: **[verified]** = seen in a source or command output in this session; **[inferred]** = my deduction from verified facts; **[not checked]** = not tested or not read.

---

## Questions (verbatim)

Owner's context, paraphrased: there is no Tunnel account now. For now we use the n0 relays, and all users see the server's public IP. Later we will run a server locally behind a Cloudflare Tunnel, and set up several VPS for testing. The owner already has a Cloudflare Workers paid plan. Task 08 lists a "Worker pkarr relay" option. This research is for a later phase, so do not design it deeply.

1. Public documentation for the Workers paid plan, as of today: limits that matter to a relay or DNS-style service (CPU time per request, memory, request duration, WebSockets, Durable Objects, KV, D1, outbound fetch limits, and the pricing unit names). Cite the URLs and the date you read them.
2. Whether a pkarr relay or iroh DNS discovery server can run on Workers. Look at existing open-source pkarr relay implementations and iroh's DNS server crate (the iroh-dns-server source in the cargo registry, if present). Say whether they need UDP, long-lived TCP, or local disk.
3. What a Cloudflare Tunnel can carry for an iroh relay: HTTPS and WebSocket traffic, but not UDP. Confirm the QUIC address discovery limitation in the iroh-relay 1.3.0 source. State what that means for a Tunnel-only relay: no QAD, so no direct paths from that relay alone.
4. The options for the later phase, with pros and cons: n0 only; a Tunnel-fronted relay; a VPS with QAD; a Worker pkarr relay. Recommend an order for when the owner is ready.

---

## Short answers

**1. Workers Paid limits.** [verified] The Paid plan is $5/month minimum and uses the "Standard" usage model. Per HTTP request, CPU time is 5 min maximum with a 30 s default, and it is set in `limits.cpu_ms`; Free is 10 ms. Memory is 128 MB per isolate. HTTP request duration has no hard limit while the client stays connected; `waitUntil()` extends work up to 30 s after the response. Outbound subrequests are 10,000 per request on Paid (Free 50), and each invocation may have 6 connections waiting for response headers. That 6-connection cap includes `fetch`, `connect()` TCP sockets, and outbound WebSockets. Inbound WebSocket messages to a Worker are capped at 32 MiB. Durable Objects (SQLite-backed) have 10 GB of storage per object on Paid, a 2 MB key+value limit, a soft limit of 1,000 requests/s per object, and an `overloaded` error when the queue fills. KV allows 1 write per second per key, eventual consistency of up to 60 s or more, and 1,000 operations per invocation. D1 has a 10 GB database limit, 1,000 queries per invocation on Paid, and a 30 s SQL duration cap. Pricing units are: Worker "Requests" (10 M included, then $0.30/M), "CPU milliseconds" (30 M included, then $0.02/M), DO "Requests" (1 M included, then $0.15/M) and "GB-s" duration (400,000 included, then $12.50/M), DO and D1 "rows read/written" and "GB-month" storage, and KV "keys read/written/deleted", "list requests" and "GB-month". There is no egress or bandwidth charge.

**2. Can a pkarr relay or iroh-dns-server run on Workers?** Not as-is, and no open-source pkarr relay in the list runs on Workers unmodified. [verified] pkarr-relay 2.1.0 needs a TCP HTTP listener (on Workers the `fetch` handler replaces it), a local LMDB directory (Workers has none), and an always-started Mainline DHT node over UDP (`lib.rs:187-199`), which Workers cannot run. iroh-dns-server 1.3.0 needs UDP and TCP on DNS port 5300 (`dns.rs:47-50, 102-118`), TCP on HTTP 8080 and HTTPS 8443 (`config.rs:260, 264`), and a redb file on disk (`config.rs:222-227`). Its optional Mainline fallback also uses UDP. Workers has no UDP: `node:dgram` is a non-functional stub [verified, Node compat page], and outbound TCP is only `connect()`. Workers has no persistent local disk either: `/tmp` is per request and not persistent [verified, fs page]. So the DNS-over-UDP listener cannot run on Workers. The HTTP parts can: pkarr PUT/GET `/<z32>` is plain HTTP (`iroh-dns-server http/pkarr.rs`), and DNS-over-HTTPS (`/dns-query`) is a request/response HTTP endpoint that iroh's client can use (`n0-dns-resolver transport.rs:286-292`). A Worker can implement those with Durable Object storage and WebCrypto Ed25519, which Workers supports [verified]. Neither the pkarr HTTP routes nor DoH need long-lived TCP. A Worker relay would be a store, not a DHT node, because it cannot send UDP. [inferred] No long-lived TCP and no local disk are needed for the pkarr or DoH paths; UDP is needed for the DNS listener and the DHT, and neither can run on Workers.

**3. What a Tunnel can carry, and QAD.** [verified] A Tunnel public hostname carries HTTP, HTTPS, UNIX sockets, and the TCP/SSH/RDP/SMB services, which use WebSockets (client-side `cloudflared` needed for non-HTTP). The docs list no UDP service type [routing page]. Tunnel hostnames are proxied through Cloudflare, and WebSockets are supported on every plan. The docs state an idle timeout but do not give a period. In iroh-relay 1.3.0, QAD is a separate QUIC service on UDP 7842 with ALPN `/iroh-qad/0` (`quic.rs:9-10`, `defaults.rs:7`). It is disabled by default (`main.rs:132-134`) and requires TLS when enabled (`main.rs:585-587`, `server.rs:842-851`). On the client, QAD is attempted only for relay entries whose config has `quic: Some` (`relay_map.rs:236-240`), and a `None` entry fails with `MissingPort` (`reportgen.rs:648-657, 683-689`). The relay websocket protocol carries no observed-address frames (grep of `protos/` found none; see Evidence C). So a Tunnel-fronted relay gives no QAD and no public candidate. For a peer homed on that relay alone, that means no direct path. The C2 spike measured this: with no QAD relay in the map, no direct path formed even behind a restricted NAT (`spikes-next/c2-holepunch/RESULTS.md` verdict 5 and §3.4). [inferred] A peer that also has n0's QAD-enabled entries in its map can still get a direct path. This was not tested against real n0 servers.

**4. Options and order.** Recommended order: (a) keep n0 as the default for now, since it is the only QAD source the owner has today; (b) add the Tunnel-fronted relay as the home relay with n0 kept in every map, then test it on real networks; (c) add one VPS with QAD when n0's QAD proves unreliable or independence is needed; (d) build the Worker pkarr relay last and only if the owner wants a lookup service independent of n0. The Worker option does not affect NAT traversal. Details and the table are in "Options" below.

---

## Evidence

### Commands run (all in this folder; nothing written outside it except one temporary probe file, see "Not verified")

```
# Cloudflare docs: markdown variant of each page (index.md), saved to sources/
for p in workers/platform/limits workers/platform/pricing workers/runtime-apis/websockets \
  workers/runtime-apis/web-crypto durable-objects/platform/limits durable-objects/platform/pricing \
  durable-objects/best-practices/websockets durable-objects/api/storage-api kv/platform/limits \
  kv/platform/pricing kv/concepts/how-kv-works d1/platform/limits d1/platform/pricing tunnel \
  tunnel/routing tunnel/setup; do curl -sSL -A "research-round2-agent/1.0 (public docs read)" \
  "https://developers.cloudflare.com/$p/index.md"; done
# Extra pages: tunnel/concepts/routing, tunnel/get-started, tunnel/reference/origin-parameters,
#   network/websockets, cloudflare-one/.../private-net, workers/runtime-apis/tcp-sockets,
#   workers/runtime-apis/bindings/rate-limit, workers/runtime-apis/nodejs (+ net, fs),
#   containers/concepts/architecture, spectrum
# crates.io metadata (API) and downloads, checksum verified against the API
https://crates.io/api/v1/crates/{iroh-dns-server,pkarr-relay,pkarr,iroh-relay}
https://static.crates.io/crates/iroh-dns-server/iroh-dns-server-1.3.0.crate   sha256 ok
https://static.crates.io/crates/pkarr-relay/pkarr-relay-2.1.0.crate           sha256 ok
# Extracted into their own empty dirs; read only, never built or run:
src-untrusted/iroh-dns-server-1.3.0/extracted/   (34 files)
src-untrusted/pkarr-relay-2.1.0/extracted/       (24 files)
# GitHub raw: pkarr design/relays.md
https://raw.githubusercontent.com/pubky/pkarr/main/design/relays.md  (200, 3552 B)
# Public product pages (webfetch, 2026-10-11)
https://docs.iroh.computer/concepts/relays
https://www.iroh.computer/pricing
# Local source (cargo registry, iroh 1.3.0 set): grep/sed with line numbers, cited below
python3 -I scripts/pkarr_cost.py     # back-of-envelope cost, see Evidence §E
```

Read time: `2026-10-11T16:07:53Z` (from `sources/READ_AT.txt`). Each Cloudflare page's "Last updated" date is cited with its URL.

### A. Cloudflare Workers, plans and limits

Source: `https://developers.cloudflare.com/workers/platform/limits/` (Last updated Oct 8, 2026; local copy `sources/workers_platform_limits.md`). [verified]
- Plan table lines 19-33: Free 100,000 requests/day, Paid no limit; CPU 10 ms vs 5 min; memory 128 MB both; subrequests 50/request vs 10,000/request; simultaneous connections 6 both; Worker size 64 MiB; startup 1 s.
- CPU: line 71, "CPU time per HTTP request: Free 10 ms, Paid 5 min (default 30 s)". Set with `limits.cpu_ms` (lines 84-105).
- Memory: line 121, "Memory per isolate: 128 MB". Isolate can be replaced when over the limit (line 125).
- Duration: line 154-159, HTTP has no limit while the client is connected; `waitUntil()` extends up to 30 s (line 159). Cron, Queue and DO alarm: 15 min (lines 152-157). DO RPC/HTTP: unlimited while active (line 426).
- Subrequests: line 188, "Subrequests per invocation: 50 (Free), 10,000 (Paid, up to 10M)"; line 189 internal services. Redirects count (line 191).
- Simultaneous connections: line 205, six connections waiting for headers; line 212 includes `connect()` TCP; line 214 includes outbound WebSockets.
- Request body: line 50-57: Free and Pro 100 MB, Business 200 MB; the limit depends on the Cloudflare plan, not the Workers plan.
- Daily requests: line 171, Free 100,000/day, Error 1027 on overflow (fail open or closed per route, lines 173-176).
- Logs: line 360, 256 KB per request.
- Rate limiting binding (`sources/workers_runtime-apis_bindings_rate-limit.md`, Last updated Apr 23, 2026): `limit` and `period` in seconds, period must be 10 or 60 (line 47). This is the per-IP limit a Worker relay would need.

WebSockets (`https://developers.cloudflare.com/workers/runtime-apis/websockets/`, Last updated Apr 23, 2026): received message size 32 MiB, then close code 1009 (line 121). `new Response(null, {status: 101, webSocket})` is the server pattern (DO websockets page, line 394). The page does not mention `Sec-WebSocket-Protocol` or subprotocol echo. [not checked] whether Workers passes a custom subprotocol header on 101; the iroh relay requires it (see C).

Pricing (`https://developers.cloudflare.com/workers/platform/pricing/`, Last updated Oct 2, 2026; `sources/workers_platform_pricing.md`). [verified]
- Line 19: Paid minimum $5/month per account; no egress or bandwidth charge.
- Line 34: Standard: Requests 10 million included, +$0.30 per additional million; Duration: "no charge or limit"; CPU 30 million CPU milliseconds included, +$0.02 per additional million CPU ms; per-invocation max 5 min (default 30 s).
- Lines 36-38: inbound requests are billed; subrequests are not; WebSocket connections are billed as one request (the upgrade); WebSocket messages routed through a Worker are not counted as requests.
- Line 166-178: KV table (Free / Paid): keys read 100,000/day or 10M/month +$0.50/M; keys written 1,000/day or 1M/month +$5.00/M; keys deleted same as written; list requests 1,000/day or 1M/month +$5.00/M; stored data 1 GB, +$0.50/GB-month.
- Lines 265-273: D1 table: rows read 5M/day or first 25B/month then +$0.001/M; rows written 100,000/day or first 50M/month then +$1.00/M; storage 5 GB then +$0.75/GB-month.
- Lines 296-316: DO on Paid: Requests 1 million/month +$0.15/M (includes HTTP, RPC sessions, WebSocket messages, alarms); Duration 400,000 GB-s/month +$12.50/M GB-s (durable-objects pricing page line 42 agrees).
- Lines 354-363: DO SQLite storage: rows read first 25B/month then +$0.001/M; rows written first 50M/month then +$1.00/M; SQL stored data 5 GB-month then +$0.20/GB-month. The page says SQLite storage billing had a January 7, 2026 target date; whether it applies to this account is not checked.
- Lines 386-398: KV-backed DO (legacy, unavailable to new accounts): read request units, write request units, delete requests, stored data GB-month.

Durable Objects limits (`https://developers.cloudflare.com/durable-objects/platform/limits/`, Last updated Jun 1, 2026): [verified]
- Lines 21-32 (SQLite-backed): storage per DO 10 GB (line 27, Paid); key+value combined max 2 MB (lines 28-29); WebSocket message size 32 MiB (line 30); CPU per request 30 s default, configurable to 5 min (line 31); 6 simultaneous outgoing connections (line 32).
- Lines 38-46 (SQL): 100 columns; string/BLOB/row 2 MB; statement 100 KB; 100 bound parameters.
- Lines 59-70 (KV-backed, legacy): key 2 KiB, value 128 KiB, storage per account 50 GB.
- Line 82: soft limit of 1,000 requests per second per object; "overloaded" error when the queue is full (line 86).
- Lines 118-126: when full, writes fail with `SQLITE_FULL`; reads and deletes still work.

Durable Objects WebSockets (`https://developers.cloudflare.com/durable-objects/best-practices/websockets/`, Last updated Sep 30, 2026): [verified] Hibernation API lets an idle DO sleep without disconnecting clients (lines 36-56); alarms, incoming requests, and `setTimeout`/`setInterval` prevent hibernation (line 342); outgoing WebSockets do not hibernate (line 338); constructor re-runs after hibernation (line 68).

KV (`https://developers.cloudflare.com/kv/platform/limits/`, Oct 8, 2026): [verified] writes to the same key 1 per second (line 23); operations per invocation 1,000 (footnote 1); value 25 MiB; key 512 bytes; namespace storage 1 GB free. Consistency (`kv/concepts/how-kv-works`, Apr 21, 2026, lines 47-61): changes may take up to 60 s or more to be visible in other locations; KV is not for atomic operations, and Cloudflare suggests Durable Objects for stronger guarantees.

D1 (`https://developers.cloudflare.com/d1/platform/limits/`, Apr 21, 2026): [verified] databases per account 50,000 Paid; max database size 10 GB Paid (line 20); queries per invocation 1,000 Paid (line 24); max SQL duration 30 s (line 33); single DO backs each database (lines 73-), queue then "overloaded" error (line 71).

Outbound TCP and Node compatibility: [verified]
- `connect()` is the TCP API (`workers/runtime-apis/tcp-sockets/`, Jun 19, 2026, line 17).
- Node compat (`workers/runtime-apis/nodejs/`, Aug 12, 2026): `node:net` "supported" (line 69) and is TCP only (`nodejs/net`, Apr 23, 2026, line 21). `node:dgram` appears in the **Non-functional stub modules** table (line 106; the list begins at line 93: "A stub can be imported or required, but does not provide a working implementation"). So there is no UDP socket.
- `node:fs` is a memory-based virtual file system (`nodejs/fs`, Apr 23, 2026, line 33). `/tmp` is writable but "not persistent and unique to each request" (line 66).

Containers (`https://developers.cloudflare.com/containers/concepts/architecture/`, Sep 30, 2026, line 65): "end-users cannot make non-HTTP TCP or UDP requests to a Container instance." [verified] I did not evaluate Containers further.

Spectrum (`https://developers.cloudflare.com/spectrum/`, Apr 23, 2026, line 25): "Custom TCP/UDP applications require an Enterprise plan with Spectrum as a paid add-on." [verified]

Web crypto (`https://developers.cloudflare.com/workers/runtime-apis/web-crypto/`, Sep 30, 2026, line 325): Ed25519 has sign/verify, generateKey, exportKey and importKey marked as supported. [verified] Footnote 1 says Secure Curves API. I did not run any Ed25519 code on Workers, so verification speed is [not checked].

### B. Tunnel (cloudflared)

`https://developers.cloudflare.com/tunnel/concepts/routing/` (Last updated Sep 11, 2026; `sources/tunnel_concepts_routing.md`). [verified]
- Line 34: "Public hostname routes proxy traffic through Cloudflare." The service-specific terms restrict serving video or large files through the CDN on Free, Pro and Business plans (same line). Relay traffic is not a file, but whether these terms apply to relay traffic is [not checked].
- Lines 38-51, supported protocols: HTTP, HTTPS, UNIX, UNIX+TLS, TCP, SSH, RDP, SMB, HTTP_STATUS, BASTION, HELLO_WORLD. No UDP service type.
- `https://developers.cloudflare.com/tunnel/get-started/` (Sep 30, 2026, line 27): cloudflared must reach Cloudflare on port 7844. This is cloudflared's own outbound connection, not something the origin can use for UDP.
- A grep of every cached Cloudflare page for "UDP" returned no tunnel-public-hostname statement. The private-network page (`cloudflare-one/.../private-net`, Sep 16, 2026) says private-network routes need the end user to enroll a device in Zero Trust. That is not a public relay path [verified].
- WebSockets (`https://developers.cloudflare.com/network/websockets/`, Aug 14, 2026): "supported on all Cloudflare plans" (line 47); proxied WebSockets need no configuration (line 17); Cloudflare closes a WebSocket with no data in either direction "for a period of time" (line 79), no number given. Enterprise can change the timeout. The idle period is [not checked]. The iroh relay sends pings every 15 s (C2 and S11 notes), so keepalive traffic is likely to prevent idle closes, but this is inferred.

### C. iroh-relay 1.3.0 source (cargo registry: `iroh-relay-1.3.0/src`)

QAD facts: [verified]
- `defaults.rs:4-7`: `DEFAULT_RELAY_QUIC_PORT = 7842` ("QUIC" on a phone keypad).
- `quic.rs:9-10`: ALPN `/iroh-qad/0`. `quic.rs:145`: the server reports `incoming.remote_address()`, i.e., the source address it observes. `quic.rs:111, 495`: observed-address reports enabled. `quic.rs:354-363`: client `create_conn(server_addr, host)`, so the QUIC connection uses the relay hostname as SNI. The certificate must match the relay host [inferred from `create_conn` and rustls verification].
- `server.rs:105-118`: `ServerConfig.quic: Option<QuicConfig>`, "disabled if None". `server.rs:430-446`: `QuicConfig { bind_addr, server_config }`, default port 7842 documented. `server.rs:842-851`: the QUIC server spawns only with a TLS config (`TlsNotConfigured` otherwise).
- `main.rs:132-134`: `enable_quic_addr_discovery` defaults to `false`. `main.rs:585-587`: enabling it without TLS is an error.
- `relay_map.rs:232-240`: "When `None`, we will not attempt to do QUIC address discovery with this relay server." `relay_map.rs:250`: `RelayConfig::new(url, quic)`. `relay_map.rs:282-283`: `From<RelayUrl>` default is `Some(RelayQuicConfig::default())`, port 7842. `relay_map.rs:292-308`: `RelayQuicConfig { port }`.
- `protos/` (`common.rs`, `handshake.rs`, `relay.rs`, `streams.rs`): no observed-address frame in the WebSocket relay protocol. `grep -rn -i -E 'observed|external|qad' .../iroh-relay-1.3.0/src/protos/` returned nothing. Across the whole crate, `grep -rln -E 'observed_addr|ObservedAddr|observed_external|qad'` matched only `server.rs`, `quic.rs`, `server/metrics.rs` and `lib.rs` [verified].

Client side, iroh 1.3.0 (`iroh-1.3.0/src`): [verified]
- `net_report.rs:443-455`: `spawn_qad_probes` returns early unless a QUIC client exists (set only when the endpoint has IP transports, `socket.rs:1035-1049`).
- `net_report.rs:497-500`: `const MAX_RELAYS: usize = 5`; the loop is `relay_map.relays().take(MAX_RELAYS)`. Relays come from a `BTreeMap<RelayUrl, _>` (`relay_map.rs:32`), so the first five by URL string are probed. An entry with `quic: None` still takes a slot and fails at once with `MissingPort`.
- `net_report.rs:826-844`: `run_probe_v4` resolves the relay address and calls `create_conn`. `reportgen.rs:648-657` (`get_quic_port`) returns `None` when `quic` is `None`. `reportgen.rs:683-689`: `get_relay_addr_ipv4` maps that to `MissingPort`.
- `net_report/probes.rs:98-115`: HTTPS latency probes go to every entry (`ProbePlan::initial` adds only `Probe::Https`). QAD is separate, as above.
- `defaults.rs:27-33` (`prod` module, hostnames): `use1-1.relay.n0.iroh.link.`, `usw1-1.relay.n0.iroh.link.`, `euc1-1.relay.n0.iroh.link.`, `aps1-1.relay.n0.iroh.link.`. `defaults.rs:36-44` (`default_relay_map`): these four, each built with `RelayConfig::from(url)`, so QAD is configured client-side for each. Whether n0's servers answer QAD was not tested (no uplink from this environment).
- Connection path `/relay` (`http.rs:13`); subprotocol `iroh-relay-v2` (`http.rs:58`), negotiated from `iroh-relay-v2,iroh-relay-v1` (`server/http_server.rs:1361-1382`). The client fails with `BadVersionHeader` if the server does not echo the protocol (`client.rs:352, 424`). The Tunnel passes the origin's handshake through, but this is [not checked] on Cloudflare's side.

Project kit (`integrated/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay/`): `tunnel.toml` sets `enable_quic_addr_discovery = false` and says "QUIC address discovery needs UDP, which Tunnel does not carry" (lines 6-9). `vps.toml` sets it to `true` with 80/tcp, 443/tcp and 7842/udp open (lines 3-11). Both agree with the source.

### D. Open-source pkarr relays and the iroh DNS server

Crate metadata (`crates-meta/*.json`, crates.io API, read 2026-10-11): [verified]
- `iroh-dns-server` 1.3.0, created 2024-04-15, updated 2026-09-28, repo `n0-computer/iroh`, description "A pkarr relay and DNS server".
- `pkarr-relay` 2.1.0, created 2025-02-11, updated 2026-10-02, repo `pubky/pkarr`, "Pkarr relay (https://github.com/pubky/pkarr/blob/main/design/relays.md)".
- `pkarr` 8.1.0 (used by pkarr-relay), updated 2026-10-02.

**iroh-dns-server 1.3.0** (`src-untrusted/iroh-dns-server-1.3.0/extracted/iroh-dns-server-1.3.0/`):
- Dependencies (Cargo.toml): `hickory-server` with `https-ring`, `axum` and `axum-server`, `redb` 4.1, `governor` and `tower_governor`, `tokio-rustls-acme` (Let's Encrypt), `rcgen`, `iroh-dns`, `iroh-base`. [verified]
- DNS (`src/dns.rs:47-50`): "Port to bind the DNS listener to, for both UDP and TCP." Bind at `dns.rs:102-118`: `UdpSocket::bind` and `TcpListener::bind` on the same address. Default port 5300 (`src/config.rs:272`, in the `Default` impl that starts at line 256). Default bind is `::`.
- HTTP (`src/config.rs:260, 264`): HTTP port 8080 and HTTPS 8443 by default. HTTPS uses `SelfSigned` or ACME (`src/http/tls.rs:20-31, 131-149`, `DirCache`).
- Routes (`src/http.rs:275-288`): `/dns-query` GET and POST (DoH), `/pkarr/{key}` GET and PUT, `/healthz`, `/`. Rate limiting is applied only to PUT (`http.rs:277-285`).
- PUT (`src/http/pkarr.rs:14-34`): verifies with `SignedPacket::from_relay_payload`, calls `store.insert`, then returns `StatusCode::NO_CONTENT` whether or not the packet was stored. Stale packets are rejected inside the store: `store/signed_packets.rs:149-175` returns `false` when `existing.more_recent_than(&packet)`.
- GET (`src/http/pkarr.rs:36-55`): returns the stored payload as `application/x-pkarr-signed-packet`.
- Store (`src/store/signed_packets.rs`): redb tables `signed-packets-1` and `update-time-1` (lines 20-23). Persistent file `<data_dir>/signed-packets-1.db` (`signed_packets.rs:295-313`; `config.rs:222-227`). `IROH_DNS_DATA_DIR` or the platform data dir sets `<data_dir>` (`config.rs:208-217`). Eviction after 7 days (`signed_packets.rs:96-100`), checked every 10 s.
- Rate limit (`src/http/rate_limiting.rs:44-64`): governor, `per_second(4)`, `burst_size(2)`, default mode `Simple` = peer IP (`rate_limiting.rs:23-30`).
- Mainline fallback: optional, `config.rs:49-53, 150-178, 241-250`, default `None` (`config.rs:286`). This is the only place the server becomes a UDP DHT participant, and only when enabled.

**pkarr-relay 2.1.0** (`src-untrusted/pkarr-relay-2.1.0/extracted/pkarr-relay-2.1.0/`):
- README (lines 3-4): "an HTTP gateway and cache for publishing and resolving Pkarr packets through the Mainline DHT". [verified]
- `src/lib.rs:187-199`: `DhtClient::build(dht_config)` is called in `run_with_dht` on every run, and the log says "Running as a DHT node on {address}". There is no switch to turn the DHT off. The `testnet` feature only gates the `mainline` crate dependency (`Cargo.toml` features).
- `Cargo.toml` dependency `pkarr` 8.1 with features `dht` and `lmdb-cache`. `src/dht_service.rs:8, 21`: `LmdbCache` on disk. `config.rs:14-18` (CACHE_DIR `pkarr-cache`), `lib.rs:73` (cache path under storage dir). Without `cache.path`, the cache goes to a temporary directory and does not survive restarts (README lines 40-42).
- `config.rs:15, 44-55`: HTTP default port 6881 (TCP). `config.rs:58-80`: DHT `port` and `public_ip` are optional.
- `response.rs:14-30`: PUT returns 204 with `Pkarr-Dht-Stored-Nodes`. `error.rs:91-100`: `NotMostRecent` maps to 409 CONFLICT.
- Record model [inferred from the README and the PUT handler]: the DHT is the store of record, and the LMDB cache is a cache.

**pkarr relay spec** (`design/relays.md`, GitHub raw, 2026-10-11): [verified]
- PUT (lines 9-46): 204 on success; "Perform the DHT Put request as defined in BEP0044" (line 38); 400 for bad key, bad signature or DNS packet; 409 when the timestamp is older than the server or DHT has seen; 413 above 1072 bytes; 429 for rate limiting; 500 when the DHT put fails.
- GET (lines 48-80): `If-Modified-Since` returns 304; `Cache-Control: public, max-age=…`; a DHT mutable GET (line 80).
- Payload (lines 89-100): `RelayPayload = signature timestamp dns-packet`; timestamp is 8-byte big-endian microseconds; DNS packet under 1000 bytes. So the largest body is 64 + 8 + 1000 = 1072 bytes.

**iroh-side pkarr and DNS** (iroh-1.3.0 and iroh-dns-1.3.0): [verified]
- `address_lookup/pkarr.rs:127`: default n0 publisher base `https://dns.iroh.link/pkarr`. `pkarr.rs:290` and `507`: `PkarrPublisher::builder(url)` and `PkarrResolver::builder(url)` accept a custom relay.
- `pkarr.rs:143`: `DEFAULT_PKARR_TTL = 30`. `pkarr.rs:146`: `DEFAULT_REPUBLISH_INTERVAL = 5 min`. `pkarr.rs:388-390`: failed publishes retry after `failed_attempts` seconds.
- `pkarr.rs:615-683`: `resolve` is GET `<base>/<z32>`, needs `is_success()`, then verifies the payload. `publish` is PUT with `to_relay_payload()`, needs `is_success()`. No request timeout is set on the client (`util.rs:10-28` builder has none). The iroh client never reads `Pkarr-Dht-Stored-Nodes` (grep for "stored" in `pkarr.rs` found nothing).
- Wire format (`iroh-dns-1.3.0/src/pkarr.rs:15-32`): pubkey 32 + signature 64 + timestamp 8 + DNS packet at most 1000 bytes. Signable bytes (`pkarr.rs:291-295`): `3:seqi<ts>e1:v<len>:<packet>`.
- DoH client: `n0-dns-resolver transport.rs:286-292` POSTs `application/dns-message` to `https://<name>:<port>/dns-query`. iroh-dns builds `n0-dns-resolver` with `transport-https` for non-wasm targets (`iroh-dns/Cargo.toml:124-130`, non-optional). So the DoH client is compiled into native `pkgs/id` builds [verified].

### E. Cost arithmetic (`scripts/pkarr_cost.py`, run with `python3 -I`)

Assumptions, labelled [inferred] or assumed: only idle republishes at 5 minutes (288 per endpoint per day, from `pkarr.rs:146`); no on-change publishes; no GETs; 2 ms of CPU per request, which is an assumption and not measured; Durable Object duration ignored (at 5 ms per request it stays under the 400,000 GB-s included); $5 base; rates and included amounts from the pricing page above.

```
endpoints=   100  PUT/month=     864,000  worker=$   0.00  do=$   0.00  cpu=$  0.00  total=$    5.00
endpoints=  1000  PUT/month=   8,640,000  worker=$   0.00  do=$   1.15  cpu=$  0.00  total=$    6.15
endpoints= 10000  PUT/month=  86,400,000  worker=$  22.92  do=$  12.81  cpu=$  2.86  total=$   43.59
Free plan: endpoints whose idle republish fits in 100,000/day = 347
```

So at default cadence the Free plan's 100,000 requests/day caps a relay at about 347 endpoints, and Paid is cheap at these volumes. The request counts are the only real driver. [inferred]

### F. Connectivity lab results used here (read-only)

From `/home/user/work/spikes-next/c2-holepunch/RESULTS.md` (2026-10-11): [verified as read]
- Verdict 5: "QAD is what provides the public candidate. With the home relay's QAD off and no other QAD relay, no direct path forms, even with a restricted NAT (30 s)."
- §3.4: the no-QAD control gives "none (LAN only)" and no direct path; the QAD-only stand-in gives a direct path at 39 ms.
- Surprising 1: "A connected peer therefore learns our LAN and public addresses anyway" when relay-only tickets are used. Candidates travel inside the QUIC connection.
- The lab runs `iroh-relay` 1.3.0 locally and uses n0 entries that were not reachable from the sandbox.

From the integrated task-08 update, 2026-10-11T12-49-09Z (`task-08-connectivity-extensions.md` line 71; `spikes-next/s11-relay/RESULTS.md` was not read): a non-home relay connection closed 60.0 s after its last datagram, and the home relay did not idle out in 3 min 40 s.

---

## Options (for the later phase)

| Option | What it carries | QAD / direct paths | Pros | Cons | Cost and ops (owner) |
|---|---|---|---|---|---|
| **A. n0 only** (current) | n0's four relays (NA east, NA west, EU, AP); public, no auth | Yes: n0 relays are QAD-configured on the client (`defaults.rs:36-44`). Not tested against live n0 servers from here. | Nothing to run; QAD available now; already the default (feature doc). | Public relays carry "no uptime or performance guarantees", are for development and testing (docs.iroh.computer/concepts/relays, read 2026-10-11). Rate limited. Shared with every iroh developer. n0's pkarr/DNS would see relay mappings and lookups (per the earlier disc research in `integrated/.../research/disc/report.md`; not re-checked here). | Free ("Community" plan). n0 Pro is $19/month for an authenticated shared relay with 10,000 connections, 5 MB/s and 100 GB egress (iroh.computer/pricing, read 2026-10-11). Pro needs an account the owner would create; not created here. |
| **B. Tunnel-fronted relay** (task 08 option A) | HTTPS and WebSocket to the origin via Cloudflare; no UDP | **No QAD from this relay.** Set `enable_quic_addr_discovery = false` and build the client entry as `RelayConfig::new(url, None)`. A peer with only this relay in its map gets no public candidate and no direct path. With n0's QAD entries in the map, direct paths are possible [inferred], untested. | No open inbound ports on the owner's machine. Clients connect to Cloudflare's edge, so the relay's own address is not exposed (the node's own public address still is, once QAD runs through n0). Cheap ($0 Cloudflare side). The kit already has `tunnel.toml`. | Cloudflare terminates TLS on the public hostname, so it sees client IPs and the relay WebSocket frames (endpoint IDs and timing). The QUIC payloads inside stay end-to-end encrypted [inferred]. WebSocket idle timeout unspecified. WebSocket subprotocol passthrough is [not checked]. Service terms on large files (Free/Pro/Business) may need review. Availability depends on the home machine and uplink. | Cloudflare account and a domain on Cloudflare (owner to create; not created). Machine and uplink are the owner's. |
| **C. VPS with QAD** (task 08 option B) | HTTPS/WSS on 443 plus QUIC on UDP 7842 | **Yes**, direct server QAD. Peers can obtain public candidates from this relay alone. | Full stack in one host: relay + QAD + TLS (kit `vps.toml`). Works without n0 for direct-path discovery. Independent of Cloudflare. | Needs a public IP, open 80/tcp, 443/tcp and 7842/udp, TLS certificates kept on persistent disk, and host maintenance. The relay sees client IPs. QAD exposes each peer's public address to connected peers (Surprising 1). | VPS bill (owner); not priced here. |
| **D. Worker pkarr relay** (task 08 item 5) | HTTP PUT/GET `/<z32>` only (lookup, not relaying) | **No QAD and no direct paths.** Affects lookup only. | Serverless; Paid plan gives enough headroom (table §E); Durable Object storage is strongly consistent; WebCrypto Ed25519 is supported; per-IP limits via the rate-limit binding. Gives the owner a lookup service that is not n0's. | No UDP, so it cannot be a Mainline DHT node. Not DHT-backed as the pkarr spec describes (BEP 44): records live only in the Worker's storage, not in the Mainline DHT. Both publisher and resolver must point at it, so lookups are private to it. DoH is the only DNS option, and it needs clients configured to query the endpoint directly. Ed25519 verification cost is not measured. | Workers Paid $5/month base plus usage (table §E). Build effort from task 08 (about 150-200 lines of JS plus 150-250 for DoH) is an estimate [inferred]. |

**Constraints a Worker relay would have to meet** (from D): PUT body at most 1072 bytes; signable bytes `3:seqi<ts>e1:v<len>:<packet>` exactly as `iroh-dns` builds them; accept only a timestamp newer than the stored one; reply 204 on stale (iroh accepts any 2xx, so 204 is safer for iroh's publisher than the spec's 409 [inferred]); use a SQLite-backed Durable Object per key, because KV has 1 write/s/key and up to 60 s staleness. Retention can follow the reference server's 7 days [inferred from iroh-dns-server].

---

## Recommendation

**Order when the owner is ready:**

1. **Now: n0 only.** Keep n0 as the default. It is the only QAD source the owner has today. Do not change the production posture until a real-network test is run, because n0's QAD has not been reached from this environment.
2. **Next: Tunnel-fronted relay as the home relay, with n0 kept in every map** (option B). This is the cheapest independent relay and is already designed. Before relying on it, check on a real Tunnel: (a) WebSocket upgrade and the `Sec-WebSocket-Protocol` echo through Cloudflare; (b) `/ping`, `/generate_204`; (c) the 60 s idle close on non-home connections. Then run C2 on two separate networks with n0's QAD in both maps.
3. **Then, if n0 QAD is unreliable or independence is wanted: one QAD-capable VPS** (option C). Add it to the maps as a second entry. Note the `MAX_RELAYS = 5` probe window (`net_report.rs:497-500`): with n0's four entries plus our Tunnel entry (no QAD), all four n0 entries and the Tunnel entry are probed, so a sixth entry would push one out by URL order. Keep the QAD-capable entries within the first five in URL order, or drop a relay from the map. The VPS testing hosts the owner plans can each be a QAD entry, but only five slots exist.
4. **Last, and only if wanted: the Worker pkarr relay** (option D). It does not change NAT traversal or direct paths. It adds a lookup service that is not n0's, at the cost of a private lookup namespace unless the owner runs both publisher and resolver against it.

**What would change this order:**
- If n0's QAD answers on real networks and direct paths form in C2 (not yet tested), option C can wait.
- If the Tunnel does not pass the relay's WebSocket handshake (subprotocol echo), option B becomes VPS-only, and option C moves up.
- If the owner needs the server's public address hidden from peers while keeping direct paths, no option in this list does that. QAD is what gives direct paths, and QAD exposes candidates to connected peers (C2 Surprising 1).
- If Workers ever exposes a functional UDP socket (`node:dgram` is a stub today), a Worker could host a DHT node. Nothing in the docs suggests that is planned [not checked].
- If the owner wants relay traffic (not just lookups) moved onto Cloudflare, that means a JavaScript re-implementation of the iroh relay protocol on Durable Objects WebSockets, or Containers. Containers cannot take end-user UDP (containers architecture page, line 65). This was not evaluated further.

---

## Not verified

- **No real Cloudflare Tunnel was tested.** Whether WebSockets and the `Sec-WebSocket-Protocol: iroh-relay-v2` echo survive Cloudflare's proxy is not checked. The C2 and S11 spikes ran on plain HTTP origins in the sandbox.
- **No Worker was built or deployed.** Ed25519 WebCrypto verification speed, CPU per PUT, and the 2 ms assumption in §E are not measured. Durable Object duration cost is assumed negligible.
- **n0's servers were not reached.** QAD on n0 is inferred from client config (`defaults.rs:36-44`). Whether n0 answers QAD on UDP 7842 is not checked.
- **Cloudflare idle timeout for WebSockets** is not stated in the docs. The 60 s non-home idle and 15 s pings are from the iroh side.
- **Cloudflare service-specific terms** on large files: whether relay traffic falls under them is not checked. It needs a read of the terms by the owner.
- **Pricing for the owner's account** may differ (legacy plan, contract, or pricing changes after the page dates Oct 2, 2026 for Workers pricing and Sep 30, 2026 for DO pricing). Figures are from the public pages read on 2026-10-11.
- **iroh-dns-server and pkarr-relay were read as source, not run.** Behaviour at runtime (e.g., rate-limit bursts, eviction timing, redb durability) is not checked.
- **DNS discovery through a Worker** needs clients configured to query the DoH endpoint directly (`NameserverConfig::https`). Whether normal recursive resolution could reach a Worker-hosted zone was not checked, and it would need UDP/TCP port 53 NS delegation, which Workers cannot serve.
- **Spectrum** (UDP for custom apps) needs an Enterprise plan per the docs. I did not check its UDP behaviour or whether QAD would still see the client's address through a proxy. Any L4 proxy would show the proxy's address to the QAD server [inferred from `quic.rs:145`]; not tested.
- **One temporary file:** a `/tmp/cf_probe_$$.out` curl probe was written outside this folder and deleted immediately. Nothing else outside the folder was written.
- **Not checked:** WebSocket messages billed on Workers vs DO for relay traffic; mainline DHT behaviour of iroh-dns-server's optional fallback (not enabled by default).
- **Source artifacts:** `sources/tunnel_routing.md` and `sources/tunnel_setup.md` came back as HTML, not markdown, so I did not rely on them (I used `tunnel_concepts_routing.md`). `sources/workers_runtime-apis_nodejs_dgram.md` is a 404 page. The `node:dgram` claim rests on the stub table in the Node compatibility page (`workers_runtime-apis_nodejs.md`, line 106).
