# Seam record — multiplexed persistent streams over one tunnel (#5756)

Status: design record (no wire code changed by this record). Issue:
[developing-today/code#5756](https://github.com/developing-today/code/issues/5756);
parent epic #5412; targets from #5701. Recorded 2026-11-16 by queue task
16816 (lane any-b), executing the re-packet split that triage task 15259
confirmed on 2026-11-14.

## 1. The problem, measurably

On the sandbox↔laptop path, transport is ≈99% of a call's cost (measured
2026-11-04, #5756): warm trivial call client-trip p50 369 ms against 1–11 ms
of daemon work; cold CONNECT ≈1.2–1.8 s and CONNECT setup serializes under
concurrency (128 tunnels = 14.6 s wall, zero failures); failure wobble
amplifies into 502 clusters at ~16.3 s p50 and no-status at ~26 s p50. The
shape causing this is HTTP/1.1-request-per-call over a pooled CONNECT tunnel
(pool of 50 in `fleet proxy serve`): every operation pays a warm RTT floor,
and concurrency pays serialized CONNECT setup.

## 2. Assumptions — landed work this design builds on (not redo items)

- **Fail-fast transport + single-flight recovery** — PR #6490
  (#5758): ≤2 s transport failure bounds and single-flight recovery are on
  main. The mux inherits these bounds; it does not re-decide them.
- **TCP_NODELAY** — set on tunnel sockets in
  `pkgs/fleet/crates/fleet-proxy/src/tunnel.rs` (and `connect_udp.rs`).
- **The wire-contract discipline of `fleet-proxy-proto`** — framing is the
  proven 8-byte big-endian length prefix + one JSON object per frame; the
  first frame is the hello exchange whose services map _is_ the capability
  negotiation; versioning accepts protocol majors N and N−1, fields additive
  within a major, unknown fields ignored. This record extends that contract;
  nothing here redefines framing elsewhere (the crate's own rule).
- **Current transport shape** — `fleet-proxy` client/serve with pooled
  per-port CONNECT and one sequential HTTP/1.1 exchange per connection is
  the fallback path (§5), not a defect to delete.

## 3. Endpoint owners

| End             | Owner (code)                                                                                                                    | Owns                                                                                                                   | Must change                                                                                                                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Client          | fleet client stack on the sandbox side — `fleet-proxy/src/client.rs` and the laptop-agent client path that speaks to the broker | Stream open/close per call, per-stream timeout ceiling, trip-id minting, fallback selection                            | Open streams instead of pooled CONNECTs when the peer advertises the mux capability (§5); keep the pooled path intact as fallback                           |
| Proxy           | `fleet proxy serve` — `fleet-proxy/src/serve.rs`, `tunnel.rs`                                                                   | The 1–2 persistent tunnels, stream↔tunnel assignment, tunnel keepalive, single-flight reconnect (inherited from #6490) | Terminate and originate streams; interleave frames by stream id; never let one stream's stall block the tunnel (frame-size bound, §4)                       |
| Broker / daemon | the laptop-side broker speaking `fleet-proxy-proto` (Rust broker; the Python broker remains a legacy major-0 peer)              | Stream dispatch against the hello services map (port → service), response interleaving, trip-id echo                   | Dispatch each stream to the advertised service for its port; an unadvertised port fails the stream fast (the proxy-level capability stub), never the tunnel |
| Wire contract   | `fleet-proxy-proto`                                                                                                             | The versioned frame types and negotiation semantics                                                                    | Add the stream frame kinds and the `proxy-mux` capability (§4); no other crate redefines them                                                               |

## 4. Decision D1 — wire protocol: extend the existing framing with a stream layer

**Decided:** multiplexing is a new frame family inside the existing
`fleet-proxy-proto` contract — same 8-byte length prefix, same JSON frame
objects, discriminated by a `kind` field — negotiated by a new hello
capability, `proxy-mux`. Stream frames:

- `open` — client→broker: stream id (client-chosen, odd), target port,
  method/path or raw-connect intent, trip id.
- `data` — either direction: stream id + payload chunk. Payload per frame
  is bounded (64 KiB); larger bodies are multiple frames, so no stream can
  head-of-line block the tunnel behind one frame for long.
- `window` — either direction: stream id + byte credit. Per-stream receive
  window starts at 256 KiB; a sender must not exceed granted credit. This
  is the backpressure bound: a slow consumer stalls its own stream, never
  the tunnel and never other streams.
- `rst` — either direction: stream id + reason. Cancellation and timeout
  are `rst`: the per-stream timeout ceiling is a client-side timer
  (default 10 s, hard cap below the historical 16–26 s failure cluster);
  on expiry the client sends `rst`, frees the stream, and may retry once
  on a _fresh stream_ if the tunnel is alive (per #5756 §4).
- `ping`/`pong` — application-level keepalive on the tunnel, carrying the
  #5756 starting constants (KEEPALIVE_S=180; the broker must not idle-exit
  while a tunnel with healthy streams is up — exit only when the tunnel is
  dead AND no calls for 3600 s).

**Tunnel failure:** a dead tunnel fails every in-flight stream immediately
with a tunnel-lost error that preserves the stream's trip id (the ≤2 s
bound from PR #6490 applies); reconnect is single-flight — one CONNECT at
a time, waiting streams queue on it — so a proxy wobble cannot stampede
CONNECTs the way the pool of 50 did.

**Rejected alternatives:**

- **HTTP/2 over the tunnel (h2 crate):** replaces the proven framing with a
  second, binary wire contract (HPACK, HTTP/2 flow-control windows) inside
  the tunnel. The broker's other consumers (fleet-queue's session-proxy
  transport, fleet-secrets' broker source) speak the JSON framing; a second
  dialect forks the contract the proto crate exists to keep singular, and
  its stream semantics would still need mapping onto the services-map
  dispatch. Cost without a compensating gain at this scale.
- **yamux (crate) framing:** yamux's own header format is likewise a second
  framing dialect layered over the tunnel, and its session/window defaults
  would need re-tuning to the bounds above regardless. The _shape_ of this
  decision is yamux-style (stream ids, window updates, rst) — implemented
  as frame kinds in the existing contract rather than as a foreign framing.
- **QUIC:** out on the issue's own evidence — the runtime proxy path is
  CONNECT/TCP only; no UDP. Revisit only if that path changes.

## 5. Rollout compatibility — negotiation, fallback, no one-sided shipment

- The mux activates **only when both hello peers advertise `proxy-mux`**.
  Absence on either side leaves today's pooled HTTP-per-call path byte-for-
  byte in charge — that path is the permanent fallback, not a deprecated
  mode.
- Mixed versions are therefore safe in both directions: old client + new
  proxy, and new client + old (or legacy major-0 Python) broker, both
  negotiate down to the current behavior. The proto versioning window
  (majors N and N−1) is unchanged; stream frames are additive within
  major 1, and a peer that never advertises the capability never receives
  one.
- **No-one-sided-shipment rule (#5756):** neither side ships mux _behavior_
  alone. A side may land its code behind the capability at any time —
  dormant until negotiated — but no release may make the mux the only path
  or remove the pooled fallback until the acceptance harness (§6) passes
  with both sides advertising.

## 6. Acceptance harness (what the implementation must pass, measurably)

Restated from #5756/#5701 and the task packet; the implementation is not
accepted on any weaker evidence:

1. **Concurrency:** 128 concurrent requests over the single tunnel —
   **zero failures**, wall **≤ 2.92 s** (≥5× better than the 14.6 s
   128-tunnel ramp).
2. **Warm latency:** warm trivial-call client-trip **p50 ≤ 50 ms** at
   comparable load (from 369 ms), daemon duration unchanged.
3. **Failure injection:** kill the runtime proxy briefly — calls fail or
   recover in **≤ 2 s**; no 16–26 s cluster appears in trips.jsonl.
4. **Correlation:** trip logs on both sides keep the **same trip id** per
   call, so the client/daemon split stays measurable before and after.

## 7. Implementation follow-on split (enqueue only after this record lands)

Per crate, in dependency order:

1. **fleet-proxy-proto** — stream frame types (`open`/`data`/`window`/
   `rst`/`ping`/`pong`), the `proxy-mux` capability, window/credit
   semantics as types + contract tests (frame bounds, unknown-field
   tolerance, N/N−1 negotiation).
2. **fleet-proxy (proxy)** — tunnel mux in serve/tunnel: stream table,
   frame interleaving, single-flight reconnect integration, keepalive.
3. **fleet-proxy (client)** — stream-open call path, per-stream timeout +
   one-retry-on-fresh-stream, fallback selection on capability absence.
4. **Broker/daemon dispatch** — stream→service dispatch against the hello
   services map, trip-id echo, unadvertised-port fast stream failure.
5. **Acceptance harness** — the §6 measurements as a runnable packet
   (128-concurrent run, warm-p50 sampling, proxy-kill injection, trips.jsonl
   correlation check).

No wire code is changed by this record; each follow-on is enqueued as its
own packet referencing this record and §6 as its acceptance.
