# Cloudflare options for an iroh relay: research notes (2026-10-11)

Read-only research. No accounts, no deploys, nothing under /home/user/code or the docs repo.

Downloaded and extracted (untrusted; read only, never executed):
- crate-dl/iroh-relay-1.3.0.crate (HTTP 200, 147,215 bytes, sha256 ee116a8233f84980574bb8d11e4517503080cd2c3605bafca04ee9c35d708bd9) -> crate-src/iroh-relay-1.3.0/
- dl-iroh-base/ -> src-iroh-base/iroh-base-1.3.0/ (PublicKey serde, sign/verify)
- dl-iroh/ -> src-iroh/iroh-1.3.0/ (QAD use, relay actor pings)

Markers: [read] = primary source read; [snippet] = search summary only; [inferred] = my reasoning, not tested.

## (a) Relay protocol, iroh-relay 1.3.0

Transport
- Path `/relay` (RELAY_PATH), HTTP GET with `Upgrade: websocket`, `Sec-WebSocket-Version: 13`, `Sec-WebSocket-Key`, and `Sec-WebSocket-Protocol`. Client offers `iroh-relay-v2, iroh-relay-v1`. Server must answer 101 with `Sec-WebSocket-Protocol` set to the chosen version, or the client fails with BadVersionHeader. [read: src/http.rs 7-68; src/server/http_server.rs 586-670; src/client.rs 255-355]
- Client maps https->wss and sets path /relay. [read: src/client.rs 267-281]
- Binary WS messages. Max WS payload 1 MiB (MAX_FRAME_SIZE). [read: src/protos/relay.rs 29; src/client.rs 314]
- Optional fast-path auth header `x-iroh-relay-client-auth-v1` built from TLS exporter keying material. If it does not verify, the server falls back to the challenge. Behind a TLS-terminating proxy it simply fails over. [read: src/protos/handshake.rs 425-450; src/client.rs 328-335]
- `--dev` = plain HTTP on port 3340. Binary needs cargo feature `server`. [read: src/main.rs 34, 588-596; Cargo.toml [[bin]] required-features]
- No STUN code anywhere in the crate. [read: grep stun/STUN in src: zero matches]

Handshake (challenge path)
- S->C ServerChallenge: tag 0 + 16 random bytes. [read: handshake.rs 77-82, 452-453; common.rs 20]
- C->S ClientAuth: tag 1 + PublicKey (32 raw bytes; non-human-readable serde = bytes) + signature (64 bytes via serde_bytes, so postcard length-prefixed 0x40). [read: handshake.rs 88-97; key.rs 78-88] [inferred on postcard length-prefix details]
- Signed message = blake3::derive_key("iroh-relay handshake v1 challenge signature", challenge), 32 bytes. Signed with Ed25519 using the endpoint secret key (EndpointId = public key). [read: handshake.rs 51-52, 209-228; key.rs 323-326]
- Server verifies with `verify_strict`. [read: key.rs 134-137]
- S->C ServerConfirmsAuth (tag 2, empty body) or ServerDeniesAuth (tag 3 + string). [read: handshake.rs 99-107, 458-474, 528-552]

Frames: tag = QUIC varint (1 byte below 64). [read: protos/common.rs 18-65; protos/relay.rs 330-559]
- 0 ServerChallenge, 1 ClientAuth, 2 ServerConfirmsAuth, 3 ServerDeniesAuth
- 4 ClientToRelayDatagram = dst(32) + ecn(1) + payload; 5 = same + seg_size u16 BE (batch)
- 6 RelayToClientDatagram = src(32) + ecn(1) + payload; 7 = batch variant
- 8 EndpointGone = endpoint(32); 9 Ping = 8 bytes; 10 Pong = 8 bytes
- 11 Health (v1 only); 12 Restarting = two u32 BE ms; 13 Status (v2) = 1 byte (0 healthy, 1 same-id connected, 2 rate-limited)
- Frame content <= MAX_PACKET_SIZE 64 KiB. [read: relay.rs 23; client/conn.rs 150; server/streams.rs 133]

Routing [read: src/server/clients.rs 109-234; relay.rs 40]
- Map EndpointId -> active connection. Packet to unconnected dst: silently dropped, no error frame.
- Otherwise queued (per-client depth 512; full = drop) and sent_to[src] gains dst.
- On src disconnect, EndpointGone(src) goes to each peer in sent_to[src].

Keepalive [read: src/server/client.rs 369-376, 424-436; src/ping_tracker.rs 4-5, 87-91; iroh-1.3.0 src/socket/transports/relay/actor.rs 74, 587, 614-619]
- Relay pings each client every 15 s + random 1-5 s. Closes the connection if no Pong within the ping timeout (<= 5 s, or 3x RTT clamped).
- iroh clients ping the relay every 15 s and expect a Pong (actor.rs:614-619).
- Stale doc: http_server.rs:329 mentions a "/derp" alias; there is no such route in code. [read]

UDP
- QAD (quic address discovery) on UDP 7842, opt-in. Default `enable_quic_addr_discovery` = false. If enabled, TLS is required. [read: main.rs 128-134, 585-599; defaults.rs 7]
- Clients only do QAD if the RelayConfig has `quic: Some`. `RelayConfig::new(url, None)` disables it. [read: relay_map.rs 232-240; iroh net_report/reportgen.rs 648-658]
- The relay still carries traffic over WS without UDP. [inferred from the above]

Worker/DO feasibility
- WebSocket hibernation with acceptWebSocket / webSocketMessage: supported. [read: DO websockets best-practices page]
- Subprotocol echo on a Worker 101 response: UNVERIFIED. Workers WebSocket docs say nothing about Sec-WebSocket-Protocol. [read: absence in workers/runtime-apis/websockets page] [snippet: MDN only]
- Ed25519 sign/verify and importKey: supported. [read: workers/runtime-apis/web-crypto table, footnote 1]
- BLAKE3 derive_key: not in WebCrypto (only SHA family listed). Needs a JS/WASM library. [read: web-crypto table] [inferred on the need]
- Attachment (per-socket state) limit: 16,384 bytes. A sent_to set of 32-byte IDs fits ~500 peers per socket. [read: DO websockets page] [inferred on the 500]
- Timers (setTimeout/setInterval/alarms) prevent hibernation and bill duration. Answer client pings; do not schedule server pings. [read: DO websockets page; DO pricing] [inferred for the design]
- Soft limit 1,000 req/s per DO object. [read: durable-objects/platform/limits]
- Deploys disconnect all WebSockets. [read: DO websockets page "WebSocket disconnection on deploy"]
- Size: about 350-500 lines TypeScript (framing, handshake, routing, hibernation hooks) plus BLAKE3 and tests. [inferred]

## (b) Cloudflare Containers

- GA since 2026-04-13 ("Containers and Sandboxes are now generally available"). [read: developers.cloudflare.com/changelog/post/2026-04-13-containers-sandbox-ga/]
- Newer items (durable_object policy, snapshot APIs) in public beta as of 2026-09-30. [snippet: changelog index]
- Requires Workers Paid plan ($5/mo minimum). [read: containers/index.md; workers/platform/pricing]
- Ingress: only through a Worker -> Durable Object. "End-users cannot make non-HTTP TCP or UDP requests to a Container instance." [read: containers/concepts/architecture]
- WebSocket upgrade forwarded through a DO with `getTcpPort(port).fetch(forwarded)`. Open WS keeps the DO active. [read: containers/examples/websocket]
- Instance types: lite 1/16 vCPU 256 MiB 2 GB; basic 1/4 1 GiB 4 GB; standard-1 1/2 4 GiB 8 GB; standard-2 1 6 GiB 12 GB; standard-3 2 8 GiB 16 GB; standard-4 4 12 GiB 20 GB. [read: containers/platform/limits]
- Cold start: 1-3 s. [read: containers/faq, concepts/architecture]
- Sleep: `Container` class sleepAfter default 10 min. No fixed max runtime. Platform may stop instances (SIGTERM, then 15 min grace); no run-time guarantee. Disk ephemeral. [read: containers/faq; concepts/architecture]
- Linux amd64 image. [read: concepts/architecture]
- Pricing (Paid): included 25 GiB-h memory, 375 vCPU-min, 200 GB-h disk per month; then $0.0000025/GiB-s memory, $0.000020/vCPU-s, $0.00000007/GB-s disk. Memory/disk billed on provisioned size while running; CPU billed on active use. Egress NA/EU $0.025/GB after 1 TB/mo. [read: containers/platform/pricing]
- Cost, lite 24/7 idle relay [inferred from the rates above]: memory 0.25 GiB x 2,592,000 s = 648,000 GiB-s - 90,000 included = 558,000 x $0.0000025 = $1.40; disk 2 GB x 2,592,000 = 5,184,000 GB-s - 720,000 = 4,464,000 x $0.00000007 = $0.31; vCPU about $0 at idle (full-time use would add <= $2.79). Total ~$1.7 + $5 base = ~$7/mo, up to ~$10 with full vCPU.
- Cost, basic 24/7 [inferred]: memory 1 GiB -> 2,502,000 GiB-s x $0.0000025 = $6.26; disk 4 GB -> 9,648,000 x $0.00000007 = $0.68; total ~$7 + $5 = ~$12/mo.
- Relay UDP: none possible. Relay WS works. QAD on this path is impossible. [inferred from ingress rule]

## (c) Cloudflare Tunnel (cloudflared)

- Available on all plans. [read: tunnel/index.md]
- Public-hostname service types: HTTP, HTTPS, UNIX(+TLS), TCP/SSH/RDP/SMB (these need client-side cloudflared), HTTP_STATUS, BASTION, HELLO_WORLD. No UDP. [read: tunnel/concepts/routing]
- UDP only via WARP private network, client install needed. [snippet: community / vendor posts]
- WebSockets: "Cloudflare supports proxied WebSocket connections without additional configuration." Idle timeout exists (length not documented), so keep a heartbeat. Cloudflare releases may restart servers and terminate WebSockets. WAF inspects only the upgrade request. [read: network/websockets]
- WebSocket through a tunnel origin: [snippet: community posts report it working] [inferred]
- Setup per tunnel/get-started: domain on Cloudflare (required); dashboard Networking -> Tunnels -> Create Tunnel; `sudo cloudflared service install <TOKEN>`; published application hostname -> Service `http://localhost:3340`; DNS CNAME created; outbound port 7844 required. Cloudflare says "under 5 minutes" for the tunnel itself. [read]
- Terms: on Free, Pro and Business plans, serving video or "a disproportionate percentage of pictures, audio files, or other large files" through the CDN can get the account limited unless a paid service (Developer Platform, Images, Stream) is used. [read: routing page note; cloudflare.com service-specific terms, CDN section] [inferred: a relay carrying large binary volume is a grey area]
- Visibility: Cloudflare terminates TLS on proxied hostnames and sees plaintext WS frames: EndpointIds in every frame, packet sizes and timing, client IPs. QUIC payloads are end-to-end encrypted: "routing encrypted traffic" [read: iroh-relay README, lib.rs]. Payload = QUIC packets [inferred].

## (d) Cost summary

Workers / DO
- Free: Workers 100,000 requests/day (plan-wide). DO has its own Free meter: 100,000 requests/day and 13,000 GB-s/day; SQLite-backed DOs only; over the limit, operations fail. CPU 10 ms per invocation. [read: workers/platform/pricing; durable-objects/platform/pricing; workers/platform/limits]
- Paid: $5/mo minimum; 10M Worker requests and 1M DO requests included, then $0.15/M DO; 400,000 GB-s/mo included, then $12.50/M GB-s; no egress/bandwidth charges; CPU 30M ms included. [read]
- WebSocket messages: incoming billed at 20:1 (1 request per 20 messages); outgoing free; WS protocol pings free. [read: DO pricing footnote 2]

Light use: 10 devices always connected; each sends ~4 pings/min (client) and answers ~3.3 server pings/min. ~105k incoming messages/day -> ~5.3k billable requests/day -> $0 on Free (100k/day cap); ~160k/mo of 1M included on Paid. Duration: hibernated between messages. Assume 5 ms per wake: ~67 GB-s/day of 13,000 Free. [inferred]

100 MB relayed in 1.2-1.4 KB packets: 100 MB = 10^8 bytes gives 71k-83k packets; 2^20 MiB gives 75k-87k. Inbound WS messages ~ packets (~71k-87k), outbound same (free). Billable: ~3.6k-4.4k requests. Duration at 10 min active ~77 GB-s (0.6% of Free daily). Cost $0 on either plan. Rate: 100 MB in 60 s = ~1,200-1,400 msg/s, above the 1,000 req/s soft limit per object [inferred]; keep under ~500 msg/s.

Containers (see b): ~$7/mo (lite) to ~$12/mo (basic). 100 MB well inside 1 TB egress.

Tunnel: $0 on Cloudflare; your box's cost.

Free-tier stoppers for the Worker option: 100k DO requests/day (fail-closed: relay stops for the day); 13,000 GB-s/day; 10 ms CPU per invocation on Free. [read]

## (e) Open items to verify (not done; no account)
1. Worker 101 response with `Sec-WebSocket-Protocol: iroh-relay-v2` accepted by the runtime and by the iroh 1.3 client.
2. BLAKE3 derive_key + WebCrypto Ed25519 verify round trip against a real iroh 1.3 client (also check verify vs verify_strict).
3. Containers: WS survival time and cold start on the `lite` type with iroh-relay 1.3.0 in --dev mode.
4. Tunnel: WS through cloudflared to iroh-relay --dev on 3340 (expected to work).
