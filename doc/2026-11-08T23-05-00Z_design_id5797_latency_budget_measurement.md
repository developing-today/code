# Design: per-fallback-tier latency budget & measurement plan for ID

- Date: 2026-11-08
- Status: **decision record** — budgets and a measurement/acceptance plan.
  No implementation, benchmark run, or performance claim is included. The
  numbers below are *acceptance targets* for future implementation, sized
  from the parent's topology and the settled sibling decisions; the first
  implementation run produces the baseline that confirms or revises them
  through a dated record (see [§9](#9-what-would-revise-this-record)).
- Issue: [#5797](https://github.com/developing-today/code/issues/5797),
  "id: design: latency budget & measurement plan per fallback tier (from
  #5671)".
- Parent design: [#5671](https://github.com/developing-today/code/issues/5671),
  "id: design: ID transport fallback for UDP-denied networks v1"
  (decomposition item 7). This record **does not re-decide** any parent
  decision: the attempt order, broker/trust model, E2E boundary and
  rendezvous requirements are inherited as-is ([§2](#2-fixed-inputs)).
- Settled inputs used, not re-decided:
  - [#5791](https://github.com/developing-today/code/issues/5791) transport
    negotiation & fallback ordering, merged as
    [`doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`](2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md)
    (PR #6234). §3.3 and §10 there explicitly assign per-transport
    deadlines, the overall negotiation budget and the memory-TTL value to
    this design; the shape (per-transport deadline, one overall budget,
    one pass over the candidate list, `timeout` as its own failure class)
    is taken as given.
  - [#5794](https://github.com/developing-today/code/issues/5794)
    Cloudflare Workers rendezvous/relay, merged as
    [`doc/2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md`](2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md)
    (PR #6260). Its boundary is binding here: the Workers relay is a
    **usability fallback for interactive and ordinary sync traffic, not a
    bulk pipe** (§ "When Workers is the right shape").
  - [#5811](https://github.com/developing-today/code/issues/5811)
    WebSocket/WSS + HTTPS fallback adapter for the browser surface. Its
    latency cost is recorded there; this record covers the non-browser ID
    client tiers and only names the reconciliation point
     ([§7](#7-provisional-assumptions-and-reconciliation-points)).
- Programme: epic [#2056](https://github.com/developing-today/code/issues/2056).
- Evidence baseline: `origin/main` at `5ad9688450`, worktree
  `~/git/fleet/13213-5797-latency-budget`, 2026-11-08. At that baseline,
  `doc/` and `pkgs/id` contain no per-tier latency budget or measurement
  plan for #5671; #5791's record says the numbers are this ticket's to set.

## 1. The problem this settles

#5671 says fallback traffic must remain usable, but "usable" is not
measurable as written: there is no per-tier target, no traffic-class split
between interactive use and bulk sync, no definition of which metric and
percentile counts, and no reproducible procedure that would let a future
implementation claim or refute a tier. Without that, every fallback
discussion becomes an argument about anecdotes, and a slow proxy tier can
be declared "working" because something eventually arrived.

This record fixes:

1. **Numeric budgets for every #5671 fallback tier, per traffic class**
   (interactive, ordinary sync, bulk), plus connection-setup and whole
   negotiation budgets ([§4](#4-budgets), [§5](#5-negotiation-budgets-and-per-attempt-deadlines)).
2. **A component allocation** for each interactive budget, so a failed
   cell points at a layer instead of a mystery
   ([§4.3](#43-component-allocation-at-the-reference-profile)).
3. **A reproducible measurement plan** — fixtures, network profiles,
   workloads, instrumentation, percentiles, repetitions and pass/fail
   gates ([§6](#6-measurement-plan)) — covering proxy/TLS, broker/framing,
   queueing/HOL/multiplexing, keepalive/idle and reconnect/resumption.
4. **Implementation invariants** that turn the plan into acceptance tests
   ([§10](#10-invariants-implementation-must-hold)).
5. **Provisional assumptions** for the still-open siblings and the exact
   points to reconcile ([§7](#7-provisional-assumptions-and-reconciliation-points)).

## 2. Fixed inputs

Inherited from #5671 exactly, and used as the axis of every budget:
**direct UDP → relay-assisted UDP → TCP → HTTP/HTTPS proxy (CONNECT, and
where necessary plain HTTP / WebSocket-style framing)**. This record adds
no tier, removes none, and does not change their relative order. The
"HTTP/HTTPS proxy" tier is measured in three shape groups because #5671
names multiple carriers with different costs:

- **T3a — CONNECT tunnel** to a broker/endpoint (tunnel established, then
  opaque bytes flow);
- **T3b-W — Workers WSS relay** (#5794 shape), and **T3b-S — self-hosted
  WSS relay** with the same pairing contract;
- **T3c — HTTPS request/response polling** as a provisional last-ditch
  shape, exactly as #5794 leaves it ("a parent-level addition, not this
  component's choice"). It is budgeted only so measurements have a place
  to land; selecting it is a parent decision this record does not take.

Inherited from #5791 and used verbatim: sequential walk within the parent
order, a single first attempt for a remembered known-good transport,
per-transport deadlines whose expiry is the failure class `timeout`, one
overall budget per negotiation, at most one pass over the candidate list,
and the override/classification rules. Nothing in this record changes
what a failure class *means* or when the client falls through.

Inherited from #5794 and used as a hard boundary: **T3b-W** is budgeted
for interactive and ordinary sync only. It has **no bulk SLO**; bulk on
T3b-W is best-effort with a progress watchdog, and #5794's "not a bulk
pipe" stands. T3b-S is self-hosted and is not covered by the Workers
limitation; it uses the brokered bulk budget in §4.4.

E2E boundary: every metric below measures transport behaviour only. No
budget, fixture or gate may inspect payload plaintext, terminate ID
payload encryption, or require the broker to see content (#5671 §6,
#5791 invariant 7, #5794 "blind pipe"). The measurement harness measures
ciphertext sizes and delivery timing, nothing more.

## 3. Budget model

### 3.1 Metrics (each budget names exactly one)

| Metric | Definition | Used for |
|---|---|---|
| **IRTT** | Interactive round trip: application writes a ≤ 4 KiB request/keystroke; the matching application-level response/echo is readable. Timed at the client application boundary, excluding client UI rendering. | interactive tiers |
| **TTFB** | Time from interactive request write to first response byte, on an established session. | queries, first keystroke after idle |
| **SETUP** | Time from "start connecting on this candidate" to "session ready for application data" (covers DNS/rendezvous where that is part of the attempt, proxy CONNECT, TCP/TLS/QUIC handshake, broker pairing and ID authentication). | connection-setup budgets |
| **NEG** | Time from negotiation start to the chosen transport being session-ready, including every failed attempt and its backoff inside the walk. | overall negotiation budget |
| **SYNC** | Time to transfer a defined ordinary-sync fixture end-to-end with acknowledgement, and its goodput. | ordinary sync |
| **GOODPUT** | Sustained application bytes/second for a bulk fixture, measured after 2 s warm-up, plus completion time. | bulk |
| **KEEPALIVE** | Spurious disconnects per session-hour while idle, and time to detect a genuinely dead peer. | keepalive/idle |
| **RESUME** | Time from transport loss detection to application data flowing again, and bytes re-sent after the last acknowledged offset. | reconnect/resumption |

Percentiles are **p50, p95, p99 and max**, computed over samples, not over
runs. A budget stated as p95 is a tail requirement; p50 is reported for
diagnosis but never replaces it. Means are not used for gates.

### 3.2 Network profiles (the RTT axis)

Absolute budgets are stated at **P2 (reference)**. The same tier must also
satisfy the relative criteria at P1/P3/P4, because a fallback that is slow
only because the network is slow is not a fallback defect.

| Profile | One-way delay (each direction) | RTT p50 | Jitter | Loss | Rate (down/up) | Notes |
|---|---:|---:|---:|---:|---:|---|
| **P0** | loopback / same host | ≤ 0.5 ms | 0 | 0 | unshaped | component isolation only; never an acceptance profile |
| **P1** | 1 ms | 2 ms | 0.2 ms | 0 | 100/100 Mbps | same LAN |
| **P2** | **15 ms** | **30 ms** | 2 ms | 0 | 100/20 Mbps | **reference profile for absolute budgets** |
| **P3** | 60 ms | 120 ms | 10 ms | 0.1% | 50/10 Mbps | tail-latency check |
| **P4** | 100 ms | 200 ms | 25 ms | 1% | 10/2 Mbps | relative criteria only; no absolute usability claim |

Profiles are produced with `tc netem` delay/jitter/loss and `tbf` rate
limits (and MTU variation where a fixture needs it), not by choosing real
locations for every run. For a proxy or relay path with two network legs,
split the profile's one-way delay across the legs so their sum equals the
table value; do not apply the full profile to both legs. Record the per-leg
RTT and loss. One
real-network cross-check at P3 per release is required ([§6.7](#67-gates)).

### 3.3 Traffic classes

| Class | Definition | Target |
|---|---|---|
| **Interactive** | Human-in-the-loop request/response or keystroke echo; messages ≤ 4 KiB; idle gaps of 100 ms–30 s between exchanges. SSH keystrokes and single ID queries are the canonical cases. | p95 IRTT / TTFB budgets (§4.1) |
| **Ordinary sync** | Batched control/sync traffic with a completion deadline but no per-message human deadline; a single transaction ≤ 16 MiB. The class #5794 says Workers must remain usable for. | SYNC budgets (§4.2) |
| **Bulk** | A transfer > 16 MiB, or a stream/transaction whose rolling 10 s goodput exceeds 1 MiB/s. For a stream/transaction with both properties, bulk takes precedence. | GOODPUT floors and mixed-load bound (§4.4) |

Classification is by logical operation/source, never by ciphertext frame
size alone (a bulk stream split into 4 KiB frames is still bulk). Interactive
user-control messages remain interactive even when multiplexed beside a
bulk stream; bulk is classified before ordinary sync for the same stream;
ordinary sync is a finite transaction ≤ 16 MiB that does not trip the
sustained-rate rule.
The 16 MiB boundary is a budget definition, not a wire-format decision; if
#5792's framing chooses a different natural transaction size, the fixtures
are re-parameterised, not the budgets re-derived.

## 4. Budgets

All absolute numbers are p95 at **P2 (30 ms RTT)** on an established
session unless marked SETUP/GOODPUT. p99 is given where tail behaviour
decides usability. Budgets are labelled `LB-n` so future tests can cite
them directly.

### 4.1 Interactive (IRTT / TTFB)

| ID | Tier | IRTT p95 | IRTT p99 | TTFB p95 (warm) | Fixed usability ceiling |
|---|---|---|---|---|---|
| LB-1 | T0 direct UDP | **≤ 90 ms** | ≤ 150 ms | ≤ 80 ms | interactive |
| LB-2 | T1 relay-assisted UDP | **≤ 120 ms** | ≤ 180 ms | ≤ 110 ms | interactive |
| LB-3 | T2 TCP direct | **≤ 120 ms** | ≤ 190 ms | ≤ 110 ms | interactive |
| LB-4 | T3a CONNECT tunnel (established) | **≤ 140 ms** | ≤ 220 ms | ≤ 130 ms | interactive |
| LB-5 | T3b-W Workers WSS relay | **≤ 180 ms** | ≤ 300 ms | ≤ 170 ms | interactive |
| LB-5a | T3b-S self-hosted WSS relay | **≤ 160 ms** | ≤ 260 ms | ≤ 150 ms | interactive |
| LB-6 | T3c HTTPS polling (provisional) | **≤ 500 ms** | ≤ 800 ms | ≤ 500 ms | **degraded, not interactive** |

The fixed usability ceiling is **250 ms p95 IRTT at P2**. A tier that
cannot meet it may still ship, but must be labelled *degraded* in
diagnostics and documentation and cannot be described as interactive at
P2; LB-6 is already in that category and exists only for last-ditch
reachability. P3/P4 make **no absolute usability claim**; the profile RTT
itself exceeds the P2 ceiling at P4, so only the relative-overhead tests
below apply there.

Rules that go with the table:

- **Relative criterion:** at P1/P3/P4, each tier labelled interactive's
  p95 IRTT must be ≤ **max(1.5 × the T0 p95 IRTT on the same profile,
  T0 + 40 ms)**. These profiles have no absolute usability ceiling or
  pass claim; report the measured RTT and overhead relative to T0.
- **T3c degraded-polling criterion:** at P1/P3/P4, p95 IRTT must be
  ≤ its P2 p95 budget + 2 × (profile RTT − 30 ms), capped at 1.5 s; report
  it as degraded regardless of pass/fail because its normal polling delay
  is already outside the interactive ceiling.
- **Warm path:** IRTT/TTFB are measured after SETUP. The first exchange
  after SETUP is measured as TTFB and included in the interactive cell.
- **Idle wake-up:** first keystroke after 30 s idle must meet TTFB p95
  within +50 ms of the same tier's warm TTFB; a keepalive that adds more
  than that is a keepalive defect, not a network cost.
- **Relay caveat:** LB-5/LB-5a hold only when the relay is
  not CPU- or frame-capped at the broker. A run that hits the current
  16 KiB DATA payload cap, 256 KiB frame-body cap or broker backpressure
  is reported as *capped*, not as a pass or fail, and re-run with the
  documented logical-chunk sizes.

### 4.2 Ordinary sync (SYNC)

Reference fixture: **8 MiB** of records in 64 KiB logical chunks,
acknowledged end-to-end; the current serve-proxy codec splits each logical
chunk into DATA payloads ≤ 16 KiB and caps a full frame body at 256 KiB
(`pkgs/id/src/serve_proxy.rs`). The 1 MiB Workers application cap in #5794
is not a broker wire-frame size. P2.

| ID | Tier | Goodput p50 | Completion p95 | Notes |
|---|---|---|---|---|
| LB-7 | T0 direct UDP | ≥ 4 MiB/s | ≤ 3 s | direct-path floor |
| LB-8 | T1 relay-assisted UDP | ≥ 2 MiB/s | ≤ 6 s | |
| LB-9 | T2 TCP direct | ≥ 2 MiB/s | ≤ 6 s | |
| LB-10 | T3a CONNECT tunnel | ≥ 2 MiB/s | ≤ 6 s | |
| LB-11 | T3b-W Workers WSS relay | **≥ 512 KiB/s** | **≤ 20 s** | #5794 boundary: ordinary sync, not bulk |
| LB-11a | T3b-S self-hosted WSS relay | ≥ 2 MiB/s | ≤ 6 s | |
| LB-11b | T3c HTTPS polling | ≥ 256 KiB/s | ≤ 60 s | provisional degraded target; not preferred |

Ordinary sync is also bound by the mixed-load rule in §4.4: an 8 MiB sync
must not push a concurrent interactive session over its ceiling.

### 4.3 Component allocation at the reference profile

Each interactive budget is allocated so a failing cell names a layer. The
network floor is 2×RTT = 60 ms at P2 (one request and one response leg).
"Broker hop" is an extra RTT when the path traverses a relay; the
polling row uses the poll interval instead.

| Tier | Network floor | Crypto/framing | Broker hop | Queueing/scheduling | Instrumentation error | Headroom | Budget (LB) |
|---|---|---|---|---|---|---|---|
| T0 | 60 | 5 | 0 | 5 | 2 | 18 | 90 (LB-1) |
| T1 | 60 | 5 | 30 | 8 | 2 | 15 | 120 (LB-2) |
| T2 | 60 | 5 | 0 | 10 | 2 | 43 | 120 (LB-3) |
| T3a | 60 | 5 | 30 | 15 | 2 | 28 | 140 (LB-4) |
| T3b-W | 60 | 5 | 30 | 20 | 2 | 63 | 180 (LB-5) |
| T3b-S | 60 | 5 | 30 | 15 | 2 | 48 | 160 (LB-5a) |
| T3c | 60 + 250 poll | 5 | 30 | 40 | 2 | 113 | 500 (LB-6) |

All values are milliseconds. The allocation is a *diagnostic hypothesis*,
not an excuse: if a run exceeds a component's allocation while total p95
still passes, the record reports the allocation miss; if total p95 fails,
the overrun names the first layer to investigate.

Per-layer measurement budgets used by the plan:

- **Proxy/TLS:** CONNECT setup p95 ≤ 1 RTT + 200 ms; TLS handshake
  (including TLS-in-TLS where the topology has it) p95 ≤ 2 RTT + 150 ms;
  certificate validation adds no network round trip.
- **Broker/framing:** broker forwarding adds ≤ 1 RTT + 10 ms p95 at P2;
  per-frame processing ≤ 0.5 ms p95 per 16 KiB DATA frame on one core;
  frame overhead ≤ 64 bytes per wire frame (measurement ceiling; current
  codec is 17 bytes including the length prefix; see
  `pkgs/id/src/serve_proxy.rs`).
- **Queueing/HOL:** at ≤ 40% carrier utilisation, broker queueing p95
  ≤ 20 ms and p99 ≤ 100 ms; at saturation the mixed-load rule decides.
- **Instrumentation:** clock/timestamp error ≤ 1 ms on LAN fixtures
  (NTP/PTP), ≤ 2 ms end-to-end; end-to-end IRTT is always measured at one
  clock, so cross-host skew affects only hop decomposition.

### 4.4 Bulk and mixed load

Bulk is where #5794's boundary matters. Budget:

| ID | Tier | GOODPUT floor (p50) | Completion p95 | Notes |
|---|---|---|---|---|
| LB-12 | T0 direct UDP | ≥ 5 MiB/s | ≤ 30 s for 128 MiB | absolute baseline floor at P2 only |
| LB-13 | T1 relay-assisted UDP | ≥ 60% of T0 on the same profile | ≤ 2× T0 + 5 s | |
| LB-14 | T2 TCP direct | ≥ 70% of T0 | ≤ 2× T0 + 5 s | |
| LB-15 | T3a CONNECT tunnel | ≥ 50% of T0 | ≤ 2× T0 + 5 s | |
| LB-16 | T3b-W Workers WSS relay | **no SLO** | best effort | #5794: not a bulk pipe |
| LB-16a | T3b-S self-hosted WSS relay | ≥ 50% of T0 | ≤ 2× T0 + 5 s | |
| LB-16b | T3c HTTPS polling | rejects bulk class (size > 16 MiB or sustained > 1 MiB/s over 10 s) | — | provisional; ordinary sync remains separately budgeted |

LB-12's 5 MiB/s and 30 s absolute targets apply at P2 only. At P1/P3/P4,
LB-12 is a measured direct-path baseline, and LB-13…LB-16a compare goodput
and completion to that same-profile T0 result (completion ≤ 2× T0 + 5 s).
This avoids a physically impossible 5 MiB/s floor at P4's 10 Mbps
downlink. At P2, if T0 cannot meet LB-12's floor, the profile is invalid
for bulk comparisons and must be reported separately rather than
substituting a different transport as the denominator.

For LB-16, "best effort" has no throughput pass threshold. The **256
KiB/s** value is only a watchdog trigger: if progress remains below it for
60 s, surface `transport_error` with the measured rate instead of stalling
silently. G5 checks watchdog behaviour and reports throughput; it does
not require Workers to exceed the trigger. The recommendation not to select
T3b-W for bulk is a capability-filtering proposal to reconcile with
#5791/#5671 before implementation (§7); it does not change the fallback
order here.

**Mixed-load rule (the HOL/multiplexing acceptance test):** with a bulk
transfer saturating the tier, a concurrent interactive session's IRTT p95
must be ≤ **2× its idle IRTT p95** and ≤ 250 ms absolute (≤ 400 ms for
T3b-W). p99 must be ≤ 500 ms (≤ 800 ms for T3b-W). If a single-stream
multiplexed carrier cannot meet this, the tier is *interactive-degraded
under bulk load* and #5792 must add streams or the parent must accept the
limitation explicitly. This is the decidable form of #5671's open question
"is multiplexing over one TCP connection sufficient, or is HOL
disqualifying for mixed interactive+bulk traffic?"

### 4.5 Keepalive, idle and reconnection

| ID | Property | Budget |
|---|---|---|
| LB-17 | Spurious disconnect while idle (persistent tiers only) | ≤ 1 per 10 session-hours, and 0 in the 10 × 30 min idle gate (§6.5); T3c polling is N/A |
| LB-18 | Keepalive interval, proxy tiers (T3a/T3b-W/T3b-S) | 15 s when no application traffic; never more than 1 per 10 s |
| LB-19 | Keepalive interval, direct tiers (T0/T1/T2) | 30 s; QUIC PING/ACK is acceptable |
| LB-20 | Dead-peer detection | 2 missed keepalives: ≤ 30 s (proxy tiers), ≤ 60 s (direct tiers) |
| LB-21 | Idle survival through a 300 s proxy idle timeout (persistent proxy tiers) | 0 disconnects during keepalive-enabled 30 min idle; if a session is forcibly dropped after > 10 min idle, detect it before the next application write and apply LB-22/LB-24 rather than leaving a silent half-open; T3c polling is N/A |
| LB-22 | Resume after transport loss (P2) | p95 ≤ 1.5 s, p99 ≤ 3 s to application data; 0 bytes re-sent after the last acknowledged offset except in-flight data |
| LB-23 | Post-session reconnect | Record retry count, backoff intervals and final failure class; reconnect does not extend initial NEG, and retry policy remains with #5792 |
| LB-24 | Sync resumption | a dropped bulk/ordinary-sync session resumes from the last acknowledged offset; restarting a > 1 MiB transfer from zero is a failure |

LB-18's interval is chosen to be ≥ 4× shorter than the shortest common
proxy idle timeout (60 s) so one missed keepalive does not kill the
session. Keepalives are transport-level and must not be counted as
application activity for sync progress accounting.

## 5. Negotiation budgets and per-attempt deadlines

#5791 fixes the shape; this section supplies the numbers it asked for.
Deadlines are fixed wall-clock caps (not per-network multipliers); SETUP
targets are p95, while the deadline is a hard maximum for a failed
candidate. T3a/T3b/T3c are measured shapes *within the one parent
HTTP/HTTPS-proxy tier*, not new global tiers. T3b-W and T3b-S are measured
separately because they have different operator/platform limits. The
existing advertisement/candidate order decides which shapes are attempted;
every attempt consumes the same shared NEG budget, which is never extended
to fit additional candidates.

| Parent tier / carrier shape | LB-31 SETUP p95 at P2 | Per-attempt hard deadline P2 | Per-attempt hard deadline P4 |
|---|---:|---:|---:|
| T0 direct UDP | ≤ 250 ms | **750 ms** | **1.5 s** |
| T1 relay-assisted UDP | ≤ 500 ms | **1.25 s** | **2 s** |
| T2 TCP direct | ≤ 750 ms | **1.5 s** | **2.25 s** |
| T3a CONNECT tunnel | ≤ 1.25 s | **2.5 s** | **3.5 s** |
| T3b-W Workers WSS relay | ≤ 1.5 s | **2.5 s** | **3.5 s** |
| T3b-S self-hosted WSS relay | ≤ 1.25 s | **2.5 s** | **3.5 s** |
| T3c HTTPS polling, provisional | ≤ 2 s | **2.5 s** | **3.5 s** |

| ID | Negotiation budget (NEG) | Value |
|---|---|---|
| LB-25 | Interactive, cold, no memory, P2 | p95 ≤ 7 s to success; hard deadline **8 s** |
| LB-26 | Interactive, cold, P4 | p95 ≤ 9 s to success; hard deadline **12 s** |
| LB-27 | Interactive, remembered known-good first attempt | hard cap **min(tier deadline, 1.5 s at P2 / 3.5 s at P4)** for the promoted attempt; any remaining walk stays inside LB-25/LB-26 |
| LB-28 | Ordinary sync, cold | hard deadline **15 s** |
| LB-29 | Bulk, cold | hard deadline **30 s** |
| LB-30 | Pinned transport | one attempt only, bounded by its §5 deadline; no walk, no overall-budget extension |

Consistency: at P2, T0+T1+T2+one T3 carrier deadline = 6 s. Add ≤ 1 s
for rendezvous and signature validation, 250 ms for selection/reporting,
and 750 ms safety reserve: **8 s total**. At P4, the same four-tier path
uses 9.25 s of attempt caps; the 12 s NEG cap leaves 2.75 s for slower
rendezvous/signature validation and reporting. Thus a cold walk with one
candidate per parent tier completes without the overall budget cutting it
off. If
the signed advertisement offers multiple candidates/shapes for a tier,
they share its remaining time in the same parent order and the overall
budget stops the walk; they do not each get a fresh 8 s or 12 s. No
profile gets a second pass (which #5791 forbids).

Memory timing values #5791 delegates here:

- known-good promotion TTL: **7 days** (after expiry the transport still
  orders the walk but gets no exclusive first attempt, per #5791 §4.3);
- quarantine TTL for `udp_blocked` / `policy_denied`: **7 days**, the
  same TTL as the known-good memory record, per #5791 §4.3;
- minimum re-probe interval for a quarantined transport on the same
  network: **5 minutes**, and never on a background timer — only on
  network change, decay expiry or explicit operator request (#5791 §3.3).

These are values, not new rules; they slot into #5791's existing state
machine. If #5791's implementation finds a shorter TTL necessary for
correctness, that is a #5791 revision and this record must be re-pointed,
not silently overridden.

## 6. Measurement plan

The plan is reproducible from this section alone. A run is identified by
`(commit, fixture version, profile, workload, repetition)`, and no result
is reportable unless all five are recorded with the artifact.

### 6.1 Fixtures and topology

| Fixture | Purpose | Minimum spec |
|---|---|---|
| **F1 direct pair** | T0 baseline, T2 direct TCP | Two `id` endpoints (client + server) on separate hosts/containers, same or adjacent networks; no proxy; no broker |
| **F2 UDP-denied sandbox** | The motivating #5671 case | F1 plus an egress firewall that drops all UDP (including QUIC) while permitting TCP/443; result must be `udp_blocked`, not a hang |
| **F3 HTTP proxy** | T3a CONNECT | `squid` or `tinyproxy` configured CONNECT-only, no URL inspection; proxy on its own host so its RTT is controllable; optional 407 auth flow |
| **F4 self-hosted broker** | T1 relay UDP, T3a and T3b-S | `id` broker/serve-proxy on a third host; use the current codec in `pkgs/id/src/serve_proxy.rs` for frame-size fixtures and add the full #5792 broker path when available |
| **F5 Workers staging relay** | T3b-W (#5794 shape) | A dedicated staging Worker + Durable Object on a paid plan, config frozen for the run; never production, never a shared demo account |
| **F6 polling endpoint** | T3c provisional | A plain HTTPS request/response endpoint on the broker host; long-poll supported; no WebSocket upgrade |

Topologies are recorded as a diagram in the run artifact. Each fixture
host reports OS, kernel, CPU, rustc, TLS library, MTU and clock source.
Containers are pinned (CPU quota, no noisy neighbours); P0 is used only
to isolate a component, never as an acceptance profile.

### 6.2 Workloads

| ID | Workload | Parameters | Metrics |
|---|---|---|---|
| **W1** | SSH keystroke echo through the fallback | 1 keystroke/100 ms for 5 min; 4 KiB max echo | IRTT, TTFB, p99 |
| **W2** | ID query mix | 1 KiB request / ≤ 4 KiB response, 10 req/s for 3 min | IRTT, TTFB, error rate |
| **W3** | Ordinary sync | 8 MiB in 64 KiB logical chunks, split into ≤ 16 KiB wire DATA payloads; acknowledged | SYNC, goodput |
| **W4** | Bulk | 128 MiB sequential in 1 MiB logical chunks, split into ≤ 16 KiB wire DATA payloads | GOODPUT, completion |
| **W5** | Mixed | W4 saturates the test tier (target 90% of its measured maximum goodput) + W1 concurrently on the same tier | interactive IRTT under load, HOL bound |
| **W6** | Idle/keepalive | 30 min idle, then W2 for 30 s; repeated 10× | LB-17…LB-21 |
| **W7** | Reconnect/resume | Kill the carrier at a random point ≥ 30× during W2 and W3/W4 | RESUME, re-sent bytes, retry count/backoff, final failure class, LB-20/22/23/24 |
| **W8** | Bulk-class edge | 12 MiB logical stream paced at 1.2 MiB/s for 10 s (one finite transaction < 16 MiB) | class decision, selected tier/reason; provisional capability-filter check |
| **W9** | Negotiation walk | Reset memory; T0/T1/T2 fail using recorded blackhole/closed-port fixtures; one T3 candidate succeeds; repeat with seeded known-good memory | NEG, candidate order, per-attempt deadline, failure class, LB-25…LB-30 |

W1 and W2 are the interactive cells (LB-1…LB-6, plus LB-5a); W3 is ordinary sync
(LB-7…LB-11b); W4 is bulk (LB-12…LB-16b); W5 decides the mixed-load bound;
W6/W7 are the keepalive/resumption gates; W8 tests the sustained-rate
precedence for a transaction below the size threshold; W9 measures the
cold/remembered one-pass negotiation bounds. W7's kill method is recorded
(SIGKILL, `tc` blackhole, proxy restart) because each produces a different
failure-detection path.

### 6.3 Instrumentation

- Monotonic `CLOCK_MONOTONIC` timestamps at: application write, first
  byte, full response, SETUP start, and each SETUP milestone (proxy
  CONNECT sent/answered, TLS started/finished, handshake sent/acked,
  broker paired, auth complete).
- Per-frame counters (bytes, frames, queue depth, drops) on the client,
  broker and server. Broker timestamps are used for hop decomposition;
  clock skew is reported, and any hop figure with skew > 2 ms is marked
  indicative rather than gating.
- One raw result record per sample (JSON Lines) plus a run summary. The
  required fields are in [§6.8](#68-artifact-format-and-reporting); a
  missing field makes the run non-reportable.
- Percentiles computed with an HDR histogram or exact sort; never from a
  mean. Report pooled p50/p95/p99/max **and** per-repetition p95 so
  run-to-run variance is visible.

### 6.4 Repetitions and statistics

- Per cell: **≥ 5 independent runs**, **≥ 2,000 interactive samples**
  pooled (W1/W2 naturally supply this), ≥ 30 reconnect events for W7,
  ≥ 10 idle cycles for W6, and ≥ 100 W9 negotiations per profile (including
  a seeded-memory subset).
- Rationale: a p95 estimated from 2,000 samples has a manageable
  confidence interval; 5 runs catch environment drift. Report a bootstrap
  95% confidence interval for each p95. A cell is **inconclusive** (not a
  pass) if the CI half-width exceeds 10% of the budget.
- Discard the first 5 s of every run (connection/setup warm-up is measured
  separately as SETUP). Pin CPU frequency where the OS allows; record
  turbo/governor state.
- Reject a whole run (and re-run) if: a cell's T0 baseline is > 20% off
  the recorded baseline for the same commit family, the host reports
  thermal throttling, or clock skew exceeds §4.3's allowance.

### 6.5 Pass/fail gates

| Gate | What must hold |
|---|---|
| **G1 interactive** | Every LB-1…LB-5 and LB-5a cell meets its p95 and p99 at P2 (where the tier is claimed), and the relative criterion at P1/P3/P4; LB-6 meets its P2 degraded budget and §4.1 polling criterion |
| **G2 setup** | LB-31 SETUP p95 per tier at P2 |
| **G3 negotiation** | LB-25…LB-30 NEG bounds, with per-attempt deadlines from §5; no negotiation exceeds one pass |
| **G4 ordinary sync** | LB-7…LB-11b goodput and completion at P2 |
| **G5 bulk** | LB-12…LB-15 and LB-16a floors/completion; LB-16 watchdog trigger behavior only (rate is reported, not gated); LB-16b bulk-class rejection |
| **G6 mixed** | §4.4 mixed-load bound (HOL/multiplexing) for tiers claiming bulk support; T3b-W uses W1+W3 (interactive + ordinary sync) instead of W1+W4 |
| **G7 idle/keepalive** | 0 spurious disconnects in the 10 × 30 min gate; LB-18/LB-19 intervals, LB-20 detection and LB-21 idle survival; T3c persistent keepalive is N/A and request timeout is reported |
| **G8 resume** | LB-22/LB-23/LB-24: resume p95/p99, retry/backoff report, zero re-sent acknowledged bytes, no restart-from-zero |
| **G9 absolute ceilings** | Every tier labelled interactive meets the 250 ms p95 P2 ceiling; otherwise the degraded label is mandatory |
| **G10 class capability** | W8 reports `bulk` for the 12 MiB / 10 s rate-edge case; T3b-W/T3c are not bulk-capable candidates under the #5794 boundary / provisional T3c assumption. This remains a parent-reconciliation test until #5791 accepts the capability-filter rule. |

A tier passes only if every gate for the traffic class it claims passes.
Partial passes are reported per cell, never rounded up to "the tier works".
A run where a fixture is missing (e.g. no Workers staging deployment) can
pass T0–T3a and T3b-S and must report T3b-W as **unmeasured**, not as passing.

### 6.6 Procedure (repeatable, in order)

1. Freeze the commit, fixture versions, profile parameter file and
   workload file; record hashes in the artifact.
2. Bring up F1; run P0 component-isolation smoke (W2, 60 s) to prove the
   harness itself adds < 2 ms p95.
3. For each profile P1…P4: run T0 (W1, W2, W3, W4, W5, W8) as that profile's
    baseline. A profile with a failed or unstable T0 baseline is void.
4. For each fallback tier available in the fixture set: set up its
   topology, verify with diagnostics that the intended transport was
   actually chosen (and only it — a fallback accidentally walking to
   another tier is a harness failure), then run the class-appropriate
   workloads. Run W9 with a controlled one-candidate-per-tier walk and
   the remembered-good variant; capture all attempt spans. W4/W5 are not
   bulk acceptance runs on T3b-W or T3c; run
   only LB-16's forced diagnostic/watchdog case on T3b-W and verify that
   T3c rejects W4 and reports unsupported bulk class for W8.
5. Run W5 only on tiers claiming bulk support; record interactive IRTT
   alongside bulk goodput. For T3b-W, run W1+W3 as G6 specifies.
6. Run W6 (10 cycles) and W7 (≥ 30 kills) last, because they leave sessions
   in a disturbed state.
7. Compute per-cell percentiles and CIs; apply G1–G10; emit the run report
   with every cell marked pass / fail / unmeasured / inconclusive.
8. For any failure, attach the per-layer allocation from §4.3 and the
   trace that identifies the first layer over its allocation.

### 6.7 Gates

- **Design gate (this record):** every tier × class has a numeric budget,
  a metric, a fixture and a gate. Checked by review, not by running code.
- **Implementation PR gate:** the P0/P1 netem subset of G1–G10
  runs in the PR CI where the harness is cheap enough; the full matrix is
   `full (main tier):` per [`.github/ci-policy.md`](../.github/ci-policy.md)
  and is not a merge gate.
- **Pre-release gate:** the full matrix at P1–P4, including one
  real-network P3 cross-check and the F5 Workers staging run, must pass or
  record a dated exception naming the failed cells.
- **Trend gate:** per-commit benchmark results are kept as artifacts; a
  regression > 5% on any T0 metric (same host, same profile, same commit
  family) or > 10% on a fallback tier blocks the release until explained
  or accepted in a dated record. Shorter-lived noise is reported, not
  gated.

### 6.8 Artifact format and reporting

A run artifact is a directory containing:

```
manifest.json      # commit, fixture versions+hashes, profile params, workloads, host specs, clock skew
samples/*.jsonl    # one record per sample: cell, tier, profile, workload, metric, monotonic start/end, bytes, logical/wire frame sizes, frame counts, outcome, error_class, traffic_class, candidate_sequence/attempt_index, known_good_hit, selection_reason, retry_count, backoff_ms, keepalive_interval_ms, keepalive_misses (N/A fields are null)
summary.json       # per cell: n, p50/p95/p99/max, CI, pass/fail/unmeasured/inconclusive, gate ids
report.md          # human-readable matrix: every LB id × profile × cell, plus allocation overruns
```

`summary.json` is the machine-checked acceptance artifact; `report.md` is
generated from it, never hand-edited. Results are linked from the
implementation issue/PR and, once a real baseline exists, summarised in a
new dated `doc/` record that links back here. This record is not rewritten
to match results.

## 7. Provisional assumptions and reconciliation points

These are assumptions, not decisions. If a sibling lands differently, the
budget numbers stay as targets and the fixtures/allocations are updated in
a dated follow-up; a sibling is never silently overridden.

| Sibling | Assumption used here | If it lands differently |
|---|---|---|
| [#5792](https://github.com/developing-today/code/issues/5792) broker/serve-proxy (protocol, framing, multiplexing, keepalive) | One multiplexed carrier with per-stream flow control is a comparison assumption, not a design decision. Current codec: DATA payload ≤ 16 KiB; full frame body ≤ 256 KiB (`pkgs/id/src/serve_proxy.rs`). The #5794 1 MiB application cap is not a broker wire-frame size. Broker adds ≤ 1 RTT + 10 ms; keepalive intervals are test inputs only. | Re-run W5 with the chosen carrier; if it uses multiple connections, HOL budgets may relax. Revalidate LB-18…LB-23 against the chosen keepalive/resumption mechanism; protocol decisions stay with #5792. |
| [#5793](https://github.com/developing-today/code/issues/5793) rendezvous/discovery | One signed-record fetch over HTTPS: ≤ 1 RTT + 250 ms p95, inside SETUP; stale/last-known-good adds no network RTT | Any additional lookup time/candidate consumes the shared NEG budget; no extra budget or candidate-count policy is set here. Reconcile against LB-25/LB-26 once #5793 settles its source order. |
| [#5795](https://github.com/developing-today/code/issues/5795) relay trust/authorization | Capability check reuses the authenticated exchange; no extra round trip on the data path | One extra authenticated round trip adds 1 RTT to SETUP and to NEG for that tier; per-message IRTT unaffected unless the check is per-message, in which case LB-1…LB-6 increment by 1 RTT and this record must be revised |
| [#5796](https://github.com/developing-today/code/issues/5796) CONNECT traversal | CONNECT with one 407 auth challenge per proxy session; 2 RTT once per connection; TLS-in-TLS only where the topology has it | Per-session challenge count/backoff changes SETUP only; per-message budgets unchanged. If #5796 adds a second TLS layer on every message, LB-4 is re-derived |
| [#5798](https://github.com/developing-today/code/issues/5798) fleet surface | No latency effect; the fleet command configures the same client | none |
| [#5811](https://github.com/developing-today/code/issues/5811) WSS/HTTPS browser adapter | Not a tier in this record; browser adapter latency is #5811's to record | Cross-check only: if #5811's measured cost exceeds LB-5's allocation for the same profile, both records are reconciled in a dated note; neither silently adopts the other's number |
| [#5794](https://github.com/developing-today/code/issues/5794) Workers boundary | T3b-W is interactive/ordinary-sync only; LB-16 has a watchdog guardrail but no throughput SLO | Binding: a future Workers change that wants a bulk SLO changes #5794 first, then this record's LB-16 |
| [#5791](https://github.com/developing-today/code/issues/5791) negotiation | Per-attempt deadlines, one overall budget, one pass, `timeout` class, memory TTLs as supplied here | If #5791 changes the walk shape, LB-25…LB-30 are re-derived; the per-tier deadlines and memory TTL values remain valid inputs |

Additional assumptions:

- **Bulk classification / capability filtering.** W8 tests the proposed
  class precedence: >16 MiB or >1 MiB/s over a rolling 10 s window is
  bulk; a ≤16 MiB transaction with no such window is ordinary sync.
  Recommendation: T3b-W and provisional T3c do not advertise bulk
  capability, so they are filtered from bulk candidates without changing
  the parent's relative fallback order. #5791/#5671 must accept this
  capability-filter rule before implementation; until then W8 is a
  reconciliation gate, not a settled runtime behavior.
- **Keepalive inputs.** The 15 s proxy-tier and 30 s direct-tier intervals
  are test values, not protocol decisions; #5792 chooses the implemented
  keepalive cadence. T3c polling has no persistent-session keepalive.
- **Measurement harness is synthetic.** No real user telemetry is used
  (no implementation exists, and metadata minimisation forbids it). All
  fixtures are deterministic and reproducible.
- **No implementation performance is claimed.** These are targets, not a
  statement that T3b-W currently meets 180 ms; nothing has been built or
  measured for #5671's fallback yet.

## 8. Relation to parent and siblings — explicit check

No *adopted* decision above changes a #5671 decision: the attempt order is the
sorting axis (not amended), the broker/trust model is used as-is, the E2E
boundary is an invariant on the harness ([§2](#2-fixed-inputs)), and
rendezvous reachability is charged to SETUP/NEG rather than re-designed.
The bulk capability-filter is a recommendation pending #5791/#5671
reconciliation, not an accepted change to the order.
No #5791 decision is changed: deadlines and the overall budget fill the
shape §3.3 fixed; the memory TTLs fill §4.3's placeholder; `timeout`,
one pass, pin, classification and override precedence are untouched. No
#5794 decision is changed: LB-16 encodes the Workers "not a bulk pipe"
boundary, while LB-16a is explicitly self-hosted.
The sibling gaps are named above as assumptions, not forked.

Where a number here would force a sibling to change, the number yields:
this record supplies a target, the sibling decides the mechanism, and a
dated follow-up reconciles the two.

## 9. What would revise this record

This record is provisional in one specific sense: the numbers are derived
from the topology and sibling decisions, not from a working
implementation. It is revised — by a new dated record linking here, never
by editing — if any of these happens:

1. The first implementation measurement shows a tier cannot meet its
   budget even with a correct implementation. The revision records the
   measured value, the allocation overrun, and either a new budget or a
   degraded label.
2. #5792's carrier cannot meet the mixed-load bound (§4.4) with one
   stream. The revision records the HOL result and the new
   interactive-degraded classification or stream count.
3. #5793/#5795/#5796 add round trips that push NEG past LB-25/LB-26. The
   revision adjusts NEG, not the per-message budgets, unless the extra
   cost is per-message.
4. #5791 alters the walk shape or #5794 reopens the bulk boundary. The
   dependent budgets are re-derived explicitly.
5. Real-network runs show P2's 30 ms is not representative of the fleet's
   dominant restricted-network path. The revision adds a profile; it does
   not silently move the reference.

## 10. Invariants implementation must hold

Each is written so it can become a literal acceptance test.

1. **Every attempt has a deadline from the table.** Expiry is the failure
   class `timeout` (#5791 §6), never a generic error, and is reported with
   the tier and elapsed time.
2. **One negotiation = one pass, one overall budget.** No candidate list
   loop, no second pass after the overall budget expires; the surfaced
   error is the last failure class per candidate tried (#5791 §3.3).
3. **A budget is per tier × traffic class.** An implementation may not
   claim a tier "works" on one class while another class it advertises is
   unmeasured; unmeasured is reported as unmeasured.
4. **p95 and p99, not means.** Every claimed cell reports sample count,
   repetitions, pooled percentiles and a confidence interval; a cell with
   < 2,000 interactive samples or > 10% CI half-width is inconclusive.
5. **Relative criteria hold at P1/P3/P4.** Passing only at P2 does not
   make a tier usable on high-RTT networks.
6. **Interactive label requires the ceiling.** A tier above 250 ms p95
   IRTT at P2 is labelled degraded in diagnostics and documentation.
7. **Mixed load is bounded on bulk-capable tiers.** Bulk must not push a
    concurrent interactive session past 2× idle p95 and its profile
    ceiling; if it does, that tier is degraded under load. T3b-W has no
    bulk SLO; its forced diagnostic is only the LB-16 watchdog check.
8. **Keepalive is transport-level and bounded.** Intervals per §4.5, no
   more than 1 per 10 s, never counted as application/sync progress; a
   spurious disconnect while idle is a defect (LB-17).
9. **Resume, never restart.** Reconnection resumes from the last
    acknowledged offset; retries obey the bounded policy #5792 settles
    and surface the final failure class. A > 1 MiB sync restarting from
    zero is a failure (LB-24).
10. **Bulk capability filtering is a recommendation, not an adopted
    negotiation rule here.** #5794 does not give T3b-W a bulk capability.
    This record recommends filtering it (and provisional T3c) from the
    bulk candidate set while preserving the parent's relative order among
    capable candidates. #5791/#5671 must accept this before
    implementation; until then, report W8 as a reconciliation gate, not a
    settled runtime invariant.
11. **No E2E change to measure faster.** Instrumentation measures
    ciphertext and timing only; it may not terminate payload encryption,
    ask the broker to inspect content, or weaken authentication to reduce
    a latency number.
12. **No result without an artifact.** A performance claim names commit,
    fixture version, profile, workload, repetitions and the artifact path;
    a claim without them is not a result.

## 11. Alternatives rejected

1. **One global "connection must take < X ms" budget.** Rejected: it
   conflates setup with per-message latency, makes the UDP-denied tiers
   fail by definition (they add a broker hop), and says nothing about
   whether SSH keystrokes feel usable once connected.
2. **Absolute-only budgets.** Rejected: a high-RTT but well-implemented
   path would "fail" because the network is far away, punishing exactly
   the users #5671 serves; relative criteria plus a usability ceiling
   separate implementation cost from distance.
3. **Relative-only budgets.** Rejected: on a fast network, a 10× regression
   can still look "fast"; absolute ceilings catch that and keep the
   degraded label honest.
4. **Median/mean targets.** Rejected: tail latency is what breaks
   interactive use and sync completion; p95/p99 are the gates, p50 is
   diagnosis only.
5. **Measure first, set budgets later.** Rejected: without a target there
   is no pass/fail, no gate, and no way to call a tier done; provisional
   targets plus a revision path is the smaller risk.
6. **Treat the Workers relay (T3b-W) as a bulk tier.** Rejected on #5794's
   verified platform limits (per-DO serialization, 128 MB memory, message
   size, cost) — bulk on someone else's switch is both slow and expensive;
   best-effort + watchdog is the honest boundary.
7. **Loopback/LAN-only acceptance.** Rejected: it removes the proxy, TLS,
   broker and RTT costs that are the entire subject of #5671.
8. **Production user telemetry as the oracle.** Rejected: no
   implementation exists; metadata minimisation and the blind-pipe trust
   model both argue against it; synthetic deterministic fixtures are
   reproducible and privacy-preserving.
9. **A single "HOL is/isn't disqualifying" opinion without a test.**
   Rejected: §4.4 states the bound and W5 measures it; the answer becomes
   evidence for #5792/#5671 rather than an assertion.
10. **Reusing #5811's browser-adapter numbers.** Rejected: different
    transport stack and client; the two records cross-check by profile,
    not by adoption (see §7).

## 12. Splits and follow-ups

This design does not split further. The natural implementation work, when
scheduled (a separate decision per #5797 and #5671), is:

- the benchmark harness (fixtures F1–F6, netem profiles, W1–W7) as part of
  the `id` fallback implementation, not as a separate design;
- the first baseline run, reported as a new dated `doc/` record linking
  here;
- per-tier budget enforcement (deadlines, keepalive, resume) in the
  implementation tickets that own each tier.

Nothing here is implementation timing; this record only makes the tiers
measurable.

## Oracle for this decision

Decision-record-only work has no code test oracle. Its checks are:

(a) every #5671 fallback tier and every traffic class has a numeric
budget, a named metric, a fixture/workload and a pass/fail gate — checked
by reading §4–§6 against #5671's decomposition item 7 and #5671 §5's
overhead list;

(b) every number is reproducible from §6 without further design work —
fixtures, profiles, workloads, instrumentation, repetitions and gates are
all stated;

(c) each interactive budget has a component allocation (§4.3) or an
explicit explanation (LB-6's poll interval);

(d) no #5671, #5791 or #5794 decision is re-decided — checked in §8 and by
the explicit assumption table in §7;

(e) the invariants in §10 can each become a literal acceptance test of the
implementation issue when one is filed; "no result without an artifact"
(§10.12) makes the measurement plan itself auditable.

## Change log

- 2026-11-08: Initial decision record for #5797. Budgets and measurement
  plan only; no implementation, no measured performance claim.
