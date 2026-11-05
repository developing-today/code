# Bufferbloat: a technical reference — mechanism, sources, and what it does and doesn't explain about the fleet path

Written 2026-11-04 at Drew's request ("review the literature and articles describing the
technical side of buffer bloat; it seems related [to our connection problem]; would be good
to document for future caching discussions"). Companion to the #5701 incident record.
Designs referenced: #5756 (multiplexed persistent streams), #5757 (request coalescing),
#5758 (fail-fast recovery), #5702 (trip-log v2, four timestamps).

Scope note: this is a literature reference, not a diagnosis. Section 5 argues both sides of
whether bufferbloat explains our measured numbers and ends in measurements that would
settle it. Nothing here changes any filed design; it gives those designs a shared
vocabulary and a citation base.

---

## 1. The mechanism, in one page

**A queue is the only place latency is made.** A packet (or request) spends its life either
being serviced or waiting in a queue in front of a service point. Service time is set by
the work itself; waiting time is set by how much work is queued ahead of it. Little's law
ties the three quantities together: mean number in system = arrival rate × mean time in
system (L = λW). Any latency that is not service time is, by definition, queueing — the
only questions are *which* queue, and *why is it occupied*.

**Bufferbloat is a specific way a queue stays occupied.** Classic TCP congestion control
(CUBIC and its ancestors) detects congestion by packet loss. Faced with a buffer, it
therefore keeps sending until the buffer overflows — i.e., it *fills any buffer it is
given, by design*. If the buffer is oversized, the result is a **standing queue**: the
buffer sits near-full for as long as the flow runs, and every packet pays delay ≈ buffer
size ÷ drain rate. Throughput is unaffected (that is what makes it insidious — speed tests
pass), while latency inflates by exactly the standing queue's drain time. Gettys' home
trace is the canonical exhibit: a path whose real RTT was 10 ms, where buffering misled
TCP's own RTT estimator by more than a factor of 100, with seconds of data parked in a
modem buffer (via LWN's report of his December 2010 posts).

**Why the buffers got that big.** Three compounding causes, per Gettys & Nichols:

1. A sizing rule from a low-flow-count era — B = RTT × C (Villamizar & Song, 1994, from
   experiments with ≤8 flows) — generalized into "provision ~250 ms of line rate
   everywhere," regardless of how many flows share the link.
2. Cheap memory plus a loss-avoidance instinct: drops looked like failures to vendors, so
   buffers grew until loss nearly vanished — removing the only signal loss-based TCP had.
3. Stacking: operating systems autotune socket buffers in response to observed delay, so
   bloat at one layer "cascades" into larger buffers higher in the stack. Buffers exist at
   every layer — driver, OS, NIC, switch, modem, middlebox — most of them unmanaged and
   unmeasured: "dark buffers."

**Delay then poisons the control loops.** Inflated RTT estimates inflate retransmission
timeouts (RTOs); congestion reacts more slowly the deeper the buffer (Gettys & Nichols
report the reaction slowdown grows quadratically — a 10× deeper buffer means ~10× the
latency but ~100× slower congestion response). The system doesn't just get slower; it gets
slower to *notice and recover*.

**The good queue / bad queue distinction (CoDel).** Not all queueing is bloat. A burst
that drains within about one RTT is a *good* queue — buffering doing its one legitimate
job, absorbing transients. A queue that persists across several RTTs is a *bad* queue: it
is a standing queue, and its delay is pure cost. CoDel's design follows from this: measure
each packet's **sojourn time** (time actually spent in the queue), not queue length or
averages, and drop only when the *minimum* sojourn over an interval (~100 ms) exceeds a
target (~5 ms). No knobs, because the measured quantity is the one users feel.

**The four families of fix.**

- *Manage the queue* (AQM): CoDel (RFC 8289), PIE (RFC 8033, ~15 ms target), fq_codel
  (RFC 8290 — per-flow queues + deficit round-robin scheduling + CoDel per queue, so one
  flow's standing queue is not everyone's latency), CAKE (shaping + AQM + per-flow and
  per-host fairness + link-overhead compensation in one qdisc). The FQ-PIE draft
  (draft-ietf-tsvwg-fq-pie-03, Oct 2026) shows the taxonomy still hybridizing.
- *Size the buffer right*: the BDP rule is correct for one/few flows; Appenzeller,
  Keslassy & McKeown (SIGCOMM 2004) showed that with n desynchronized flows the need falls
  to BDP/√n; Spang, Arslan & McKeown (2021) re-examined the theory and found √n holds
  under broader assumptions than the original, including newer congestion controls. Tiny
  buffers (tens of packets) suffice when flows are many and bursts smooth out.
- *Don't build the queue at the endpoint*: BBR models the path as BtlBw × RTprop = BDP
  (Kleinrock's optimal operating point) and paces at the estimated bottleneck bandwidth
  with inflight capped near the BDP. Where loss-based control parks at BDP + buffer size
  (a standing queue by construction), BBR aims to run with the pipe full and the queue
  empty; Google reported median RTT −53% vs CUBIC globally. L4S (RFCs 9330–9332) takes
  the network-side version of the same idea: mark ECN at ~1 ms of queue so "scalable"
  congestion controls can hold queues near-empty, with a DualQ separating L4S traffic from
  classic traffic. Deployment is genuinely contested (see §2).
- *Remove the queue from the path*: fewer trips, fewer serial queues, queues relocated to
  where they are cheap and observable. This is where caching, coalescing, and connection
  architecture live — §3 and §4.

**Two adjacent pathologies that are not, strictly, bufferbloat** — but belong in this
reference because they produce the same user-visible signature (latency ≫ service time,
worse under load, worst during recovery):

- *Head-of-line (HoL) blocking*: many logical streams over one ordered byte stream. One
  lost or slow segment stalls every stream behind it, however small the buffers are. QUIC
  (RFC 9000) / HTTP/3 (RFC 9114) exist largely to delete this: streams are independent at
  the transport, so a loss delays only the stream whose data it carried.
- *TCP-over-TCP meltdown*: run a reliable transport inside a reliable transport and the
  two retransmission/backoff loops couple destructively. The outer layer masks path health
  from the inner; the inner layer's RTO (calibrated in calm conditions) fires first and
  injects duplicate retransmissions that queue behind the delayed originals in the outer
  buffers; both layers then back off exponentially. Titz wrote the canonical warning in
  2001; RFC 9329 uses the name "TCP meltdown" for TCP-in-TCP tunnels. The buffers matter
  here too — they are where the duplicates accumulate — but the root cause is stacked
  timers, not buffer size.

**One-sentence version.** Bufferbloat = queues that stay full because nothing in the
system is rewarded for keeping them empty; the fix is always some combination of
measuring sojourn, isolating flows, pacing senders, capping waits, and deleting trips.

---

## 2. Annotated sources

Each entry: what it establishes, and why it matters to us. URLs verified 2026-11-04.

### 2.1 The canonical line

**Gettys, "Whose house is of glasse, must not throw stones at another" and the December
2010 bufferbloat posts** — reported at https://lwn.net/Articles/418918/
The discovery narrative. Establishes the empirical signature: latency 4–10× (in his modem
trace, >100× the RTT estimator) what the path should deliver, caused by buffers holding
*seconds* of data. Matters to us: it defines the shape we should look for — delay that
scales with offered load while throughput and service time stay innocent.

**Gettys & Nichols, "Bufferbloat: Dark Buffers in the Internet," ACM Queue Jan 2012 /
CACM Jan 2012** — https://queue.acm.org/doi/10.1145/2063166.2071893 · PDF:
https://courses.cs.duke.edu/common/compsci092/papers/bufferbloat.pdf · NANOG 52 slides:
https://archive.nanog.org/meetings/nanog52/presentations/Tuesday/Gettys-NANOG11a.pdf
The field's founding survey. Proves the problem is systemic: buffers at every layer, the
RED manifesto's advice ignored for a decade, OS autotuning cascading bloat up the stack.
Matters to us: our path (client → tunnel pool → runtime proxy → DERP relay → broker) is
likewise a *stack of independently-buffered layers*; no single layer needs to be "the bug"
for the sum to be pathological.

**Nichols & Jacobson, "Controlling Queue Delay," ACM Queue May 2012 (CoDel)** —
doi 10.1145/2209249.2209264; summary:
https://blog.acolyer.org/2015/11/19/fail-at-scale-controlling-queue-delay/ · standardized
as RFC 8289 (doi 10.17487/RFC8289).
Proves you can control delay without measuring link rate or tuning knobs, by judging the
queue on packet sojourn time (local minimum over an interval; 5 ms target / 100 ms
interval defaults). Matters to us: sojourn, not occupancy, is the metric to copy — it is
exactly what #5702's four trip timestamps let us compute per path segment. Also the source
of the good-queue/bad-queue test we apply in §5.

**Täht, Høiland-Jørgensen et al., RFC 8290 (fq_codel) and RFC 8033 (PIE)** — both cited
and compared in draft-ietf-tsvwg-fq-pie-03:
https://www.ietf.org/archive/id/draft-ietf-tsvwg-fq-pie-03.txt
fq_codel proves flow isolation is as important as queue control: hash flows into separate
queues, schedule between them (DRR), run CoDel in each. PIE is the control-theoretic
alternative (proportional-integral error on delay, 15 ms default target) from the
DOCSIS/cable world. Matters to us: isolation is the transferable idea — one herd (e.g.
post-recycle reconnects) must not be able to queue ahead of unrelated interactive calls in
the broker or pool. Our analogue of "flows" is lanes/callers.

**Høiland-Jørgensen, Täht, Morton, "Piece of CAKE," arXiv:1804.07617** —
https://arxiv.org/pdf/1804.07617 · LWN: https://lwn.net/Articles/758353/
CAKE integrates rate shaping with link-layer overhead compensation, the COBALT AQM
(CoDel + BLUE), 8-way set-associative flow hashing with per-flow *and* per-host fairness,
DiffServ tiers, and ACK filtering, in one Linux qdisc derived from fq_codel. Proves the
integrated approach beats bolting AQM onto an unshaped link. Matters to us as the design
pattern: shaping + AQM + fairness as one mechanism, and the per-host fairness idea maps to
per-caller fairness at shared fleet chokepoints.

### 2.2 Buffer sizing

**Villamizar & Song (1994), the B = RTT × C rule** — as popularized (and critiqued) in
Gettys & Nichols and the sizing literature below.
Correct for its measured regime (≤8 flows, 40 Mb/s backbone links); catastrophic as a
universal default, because it sizes for the worst case of a single flow needing a full
BDP of slack. Matters to us: every fixed-size pool/buffer default in our stack deserves
the same question — *sized for which regime, measured when?*

**Appenzeller, Keslassy & McKeown, "Sizing Router Buffers," SIGCOMM 2004** —
http://yuba.stanford.edu/~appenz/pubs/bufsiz.pdf
With n long-lived, desynchronized flows, aggregate burstiness smooths and BDP/√n suffices
— orders of magnitude less at backbone flow counts. Matters to us: the √n intuition is the
pool-sizing intuition too. Fifty mostly-idle tunnels do not need fifty tunnels' worth of
queued work behind them; concurrency smoothing is real, and oversizing "for safety" is
precisely how standing queues are born.

**Spang, Arslan & McKeown, "Updating the Theory of Buffer Sizing" (2021)** —
https://arxiv.org/pdf/2109.11693
Re-derives the sizing theory and finds the √n rule survives under broader assumptions,
including modern congestion controls. Matters to us: the 2004 result is not a period
piece; cite this when a "buffers must be deep" claim needs an answer.

### 2.3 The modern line

**Cardwell, Cheng, Gunn, Hassas Yeganeh & Jacobson, "BBR: Congestion-Based Congestion
Control," ACM Queue Sep–Oct 2016 / CACM Feb 2017** — doi 10.1145/3009824 · summary:
https://blog.acolyer.org/2017/03/31/bbr-congestion-based-congestion-control/ · full text:
https://iqua.ece.toronto.edu/baochun/ece1771f/papers/bbr-cacm.pdf
The model-based counterpoint to the whole AQM line: instead of managing the queue the
sender creates, model the path (windowed-max delivery rate over ~10 RTTs, windowed-min
RTT over ~10 s), pace at BtlBw, cap inflight ≈ BDP, and cycle ProbeBW/ProbeRTT. Proves
the standing queue is a *sender* behavior — loss-based control's equilibrium *is* a full
buffer. Matters to us in two places: (a) any bulk mover we run (atlas restores, backups,
file RPC) shapes the latency of everything sharing its path, and its congestion control
decides whether it leaves a standing queue behind it; (b) BBR's probe cycle is a reminder
that an unloaded-path measurement (RTprop) must be actively re-taken — idle-path numbers
go stale, which is one reason low-load benchmarks mislead (§4).

**The L4S family: RFCs 9330 (architecture), 9331 (ECN protocol / Prague requirements),
9332 (DualQ Coupled AQM), Jan 2023, Experimental** — overview and debate:
https://en.wikipedia.org/wiki/L4S · partial-deployment measurements, ANRW 2024:
https://irtf.org/anrw/2024/slides-120-anrw-08.pdf
L4S tries to make ~1 ms queueing the *normal* operating point via immediate ECN marking
and scalable congestion controls (TCP Prague, adapted from DCTCP). The contention is also
documented and unresolved: the pacing/burstiness Prague must exhibit (RFC 9331 expects
per-flow burst queue ≲250 µs at minimum RTT; the Prague draft states "a Prague CCA MUST
pace"), and independent partial-deployment tests showing Prague throughput degraded vs
CUBIC in some shared-queue scenarios, with fairness claims contested. Comcast is an early
production adopter (Jan 2025); Linux TCP Prague remains experimental. Matters to us
mostly as a caution: even a standards-track, well-resourced attempt to fix queueing
end-to-end is bottlenecked on deployment and fairness edge cases. Anything we build
should get its wins from our own segment, not from hoping the path upgrades.

**Iyengar & Thomson, RFC 9000 (QUIC); Bishop, RFC 9114 (HTTP/3)** — explainers:
https://http.dev/3 · https://safeguard.sh/resources/blog/quic-and-http-3-explained-rfc-9000-and-rfc-9114
QUIC moves multiplexing into the transport over UDP: streams are independent, so loss on
one stream never stalls another; handshake is 1 RTT (0 on resumption). Proves HoL blocking
is an *architecture* property, removable by stream independence — not a tuning problem.
Matters to us directly: it is the existence proof and the design spec for #5756. A
multiplexed tunnel that serializes streams behind one ordered byte stream re-creates
HTTP/2-over-TCP's failure mode inside our own stack.

### 2.4 Tunnels, relays, pools, retries — the overlay angle

**Titz, "Why TCP Over TCP Is A Bad Idea," April 2001** —
http://sites.inka.de/sites/bigred/devel/tcp-tcp.html (as cited by the WUSTL OpenVPN
analysis, https://www.cs.wustl.edu/~jain/cse567-08/ftp/ovpn/index.html) · mechanism
summary and further citations: https://en.wikipedia.org/wiki/Tunneling_protocol ·
measurement study: Honda et al., "Understanding TCP over TCP," 2005,
doi 10.1117/12.630496 · the name standardized in RFC 9329 ("TCP meltdown").
The meltdown mechanics, step by step: the outer TCP holds and retransmits lost data
without telling the inner TCP anything is wrong; the inner TCP's RTO — calibrated in
low-latency conditions — expires first, so it retransmits into the outer send buffer,
where duplicates queue behind the delayed originals; ACKs stop flowing; the inner RTO
doubles repeatedly while the outer congestion window collapses. Both layers back off
exponentially, so recovery is slow even after the path clears. Matters to us: our request
path stacks reliable layers (RPC over HTTP over CONNECT tunnels over TCP, with a
DERP-over-HTTPS leg). We are not running literal PPP-over-TCP, and our per-call payloads
are small — but any design where an inner retry/timeout can fire while an outer layer is
still patiently retransmitting the original inherits a share of this failure geometry.
The discriminators in §5 include looking for its fingerprint (retransmit storms +
inflated outer RTOs coincident with inner timeouts).

**Tailscale, DERP vs direct connections** — third-party summary of Tailscale's own
position and measurements: https://www.ssdnodes.com/learn/tailscale-slow-direct-vs-relayed
DERP is the fallback when direct WireGuard (UDP) connectivity fails; a relayed path is
two Internet paths joined at a machine you don't control, and shows both higher latency
and higher variance than direct. Tailscale's own rule: "Direct connections nearly always
result in lower latency and higher throughput." Matters to us: our path deliberately
hairpins through a relay leg (WS-C, #5701). A relay's queues are the archetypal dark
buffers — real, load-bearing, and instrumented by nobody. The DERP-vs-direct comparison
is therefore a first-class discriminating measurement (§5), not an optimization idea.

**Connection-pool sizing, the HikariCP tradition (Little's law applied)** — interactive
model: https://www.giaicuuthegioi.com/visualization/PoolSizing.html · production tuning
notes: https://www.umurinan.com/pages/tutorials/hikaricp-production-tuning.html
Two results worth importing wholesale. (1) A pool does not abolish queueing; it *chooses
where the queue lives*. Too small a pool and requests queue in front of it (cheap,
visible, sheddable); too large and the queue moves inside the server, where each admitted
request makes all the others slower — the interactive model shows a just-right pool
(cores × 2 rule of thumb) beating both a starved pool and a 4× oversized one on
throughput *and* latency. (2) The exhaustion signature is measurable separately from
service: threads-awaiting-connection count and acquire-time percentiles vs in-service
time. Matters to us: this is the exact anatomy of our TunnelPool question. A warm pool
*creates* a standing queue when callers are admitted faster than the far end drains, and
*hides* one when pool-wait is not measured separately from call time. Our warm p50 of
369 ms with 1–11 ms of daemon work is, until decomposed, precisely an un-split
acquire+service number (§5).

**Brooker, "Timeouts, Retries and Backoff with Jitter," Amazon Builders' Library** —
https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/
(companion to his 2015 AWS Architecture Blog post, "Exponential Backoff and Jitter").
Retries are selfish: each layer's retry multiplies the load seen below it (the standard
worked example: 3 retries at each of 5 layers = up to 243×). Backoff alone does not
desynchronize a fleet that failed together — clients retry in lockstep waves; *full
jitter* (sleep = random(0, min(cap, base × 2^attempt))) is what breaks the wave, and a
*retry budget* (retries capped as a fraction of request volume) is what bounds the total
harm. Matters to us: the post-recycle herd is our fleet failing together by construction
(everything restarts at once). Whatever jitter/budget discipline we adopt belongs in the
same design conversation as queue depth, because herds are how deep queues get filled.

**Dean & Barroso, "The Tail at Scale," CACM 56(2), Feb 2013, pp. 74–80** —
doi 10.1145/2408776.2408794 · full text:
https://www.rose-hulman.edu/class/cs/csse576/papers/The%20Tail%20at%20Scale%20_%20February%202013%20_%20Communications%20of%20the%20ACM.htm
Fan-out amplifies tails: if each of 100 leaves is slow 1% of the time, ~63% of composite
requests hit a slow leaf; in their table, one-request p99 of 10 ms becomes all-requests
p99 of 140 ms. Their key queueing observation: most response-time variability is *queue
wait before service starts*, not service — hence "tied requests" (enqueue on two
replicas; whichever *starts service* cancels its twin) and hedged requests (a 10 ms hedge
cut a BigTable p99.9 from 1,800 ms to 74 ms at ~2% extra requests). Matters to us: (a)
any fleet operation that fans out over the tunnel path inherits the path's tail, squared;
(b) hedging is only safe with cancellation and idempotence — otherwise it is a
self-inflicted retry herd; (c) percentiles or it didn't happen: means hide exactly the
part of the distribution this document is about.

### 2.5 Caching sources

**Nottingham, RFC 5861, "HTTP Cache-Control Extensions for Stale Content," May 2010** —
https://www.rfc-editor.org/info/rfc5861/
Defines `stale-while-revalidate` — serve the stale response immediately and revalidate in
the background, in the RFC's own words "thereby hiding latency (both in the network and
on the server) from clients" — and `stale-if-error`, where "an error is any situation
which would result in a 500, 502, 503 or 504 HTTP response status code." Matters to us
almost verbatim: our failure tax is denominated in exactly those statuses (502 trips at
p50 16.3 s). SWR/stale-if-error are the standards-track form of "let the caller proceed
with the last good answer while the path recovers" — the caching face of fail-fast.

**Request collapsing at Fastly (Perf Planet, 2019)** —
https://calendar.perfplanet.com/2019/request-collapsing-the-hidden-magic-that-invisibly-speeds-up-the-web/
A cache that receives concurrent misses for the same object forwards *one* fetch upstream
and attaches the other requesters to it; done in layers (per edge server, per data
center, between data centers via shielding) so exactly one request reaches the origin.
Also documents the sharp edges: a `private` response forces queued waiters to be dequeued
and processed individually, and batched dequeueing makes origin traffic bursty even when
client arrivals are steady. Matters to us: this is the reference implementation story for
#5757. Coalescing is simultaneously a cache mechanism (one fill serves everyone) and an
anti-bloat mechanism (offered load at the bottleneck divided by the collapse factor) —
and its failure modes (waiter pile-ups behind one slow fill) are queueing failure modes.

**Collapsed forwarding / origin shield, standard CDN practice** — course summary:
https://www.compress-pdf.pro/courses/secure-video-streaming/pages/08-cdn-signed-urls.html
The shield pattern: a designated mid-tier is the only layer allowed to fetch from origin,
so synchronized misses across many edges converge into one fetch. Names the failure it
prevents — the "thundering herd" where caches amplify load instead of absorbing it.
Matters to us: placement *is* queue relocation. A shield/cache hit deletes arrivals from
every downstream queue; a miss converges at one instrumented point instead of stampeding
the origin from fifty directions. Our post-recycle herd is a thundering herd wearing a
different hat (cold everything, all at once), and the shield/collapse pattern is the
canonical answer to it.

---

## 3. Mitigation taxonomy, mapped to our designs

| # | Mitigation family | Literature anchor | Our mapping |
|---|---|---|---|
| 1 | Measure sojourn, not occupancy | CoDel (sojourn time, local min); Tail at Scale (percentiles) | #5702's four trip timestamps = per-segment sojourn. Prerequisite for every claim in §5. |
| 2 | Cap the wait, not the retries | CoDel's 5 ms target as a *time* budget; #5758 | Fail-fast ≤2 s is a queue-length cap expressed in time — CoDel's move, applied at the RPC layer. A request that cannot be served within budget should fail while the queue is still short enough to recover. |
| 3 | Isolate flows so one queue isn't everyone's | fq_codel (per-flow queues + DRR); CAKE (per-flow + per-host fairness, DiffServ tiers) | Broker/pool fairness by lane or caller class; bulk movers (restores, backups) in a bulk tier that cannot queue ahead of interactive calls. |
| 4 | Pace and cap inflight at the sender | BBR (pace at BtlBw, inflight ≈ BDP) | Bulk transfers over the fleet path should be rate-shaped to the path, not to the disk; an unpaced fill creates the standing queue everyone else pays for. |
| 5 | Delete trips (reduce λ at the bottleneck) | Little's law; request collapsing (Fastly); origin shield | #5757 coalescing (server /claim-flow, /batch; the daemon already batches 10 cmds/trip at 483 ms); caching (§4). Every deleted trip shortens every queue behind it. |
| 6 | Remove serialization inside the pipe | QUIC/HTTP/3 stream independence; HoL analysis | #5756 multiplexed persistent streams — with the QUIC caveat: streams must be independently flow-controlled, or multiplexing re-creates HoL blocking on one ordered stream. Also amortizes cold CONNECT setup (1.2–1.8 s) across the pool's life. |
| 7 | Desynchronize recovery | Brooker (full jitter, retry budgets); collapsing literature (stampede) | Post-recycle reconnects and client retries: jittered backoff + a fleet-level retry budget, so the herd arrives as a slope, not a step. |
| 8 | Keep queues where they are cheap and observable | HikariCP pool doctrine (queue in front of the pool, measured as acquire time) | Prefer waiting at the client/broker (instrumented, sheddable, deadline-able) over waiting inside tunnels/relays (dark). Split pool-acquire time from in-call time in metrics. |
| 9 | Don't stack blind reliability layers | Titz; TCP meltdown (RFC 9329) | Audit timeout/retry layering on the path: an inner timeout shorter than the outer layer's delivery patience manufactures duplicate work. Deadlines should be set end-to-end and subdivided, not independently per layer. |
| 10 | Serve stale under duress | RFC 5861 (SWR, stale-if-error) | §4. For control-plane reads that tolerate staleness, a cached answer beats a 16 s 502. |

What the taxonomy deliberately omits: "add bandwidth" and "enlarge the pool/buffer."
Both raise the ceiling without touching the standing queue; the sizing literature
(Appenzeller; Spang et al.) says the queue, not the capacity, is usually the adjustable
term.

---

## 4. The caching connection (the reason this doc exists)

Queueing delay changes what a cache is *for*. At zero load a cache saves work; under
bloat it saves *queueing*, and queueing is the expensive part. Five interactions worth
carrying into future caching discussions:

1. **A miss costs latency-under-load, not idle latency.** Cache evaluations quote miss
   cost from benchmarks on quiet paths. But misses cluster exactly when the path is
   busiest (launches, expiries, restarts), and under bloat the marginal cost of one more
   trip through the bottleneck queue is nonlinear — Little's law with a queue already
   near saturation. The same hit ratio is worth far more at peak than at idle, and a
   cache sized/valued by idle measurements will be undersized for the moments that
   matter.

2. **Prefetching into a bloated path is self-defeating — unless it is paced and
   idle-only.** A prefetch is speculative offered load. If it runs while queues are
   occupied, it lengthens the queue for real demand and its own fill arrives stale-dated
   by the sojourn it suffered. BBR's probe discipline is the model: refresh your picture
   of the path during quiet, and treat "the path is busy" as a reason to *defer*
   speculative work, not to push harder. Cache warming belongs in the same scheduler
   class as bulk transfer: bulk tier, paced, preemptible (taxonomy #3/#4).

3. **Stale-while-revalidate is the latency-under-bloat strategy.** SWR converts queueing
   delay from user-visible to background: the caller takes the cached answer now; the
   refresh pays the sojourn once, off the critical path, and — collapsed — once per
   object per cache rather than once per caller. stale-if-error extends the same logic
   to failure: during the exact window where our path charges a 16–26 s failure tax, a
   cache holding the last good answer can serve it instead. The trade is explicit and
   bounded (the directive's seconds), unlike the unbounded trade a deep queue makes on
   your behalf.

4. **Coalescing is one mechanism wearing both hats.** Request collapsing is a cache
   mechanism (one fill, many waiters, result stored) and an anti-bloat mechanism
   (arrivals at the bottleneck divided by the collapse factor; the queue of waiters is
   explicit and countable, unlike a network buffer). Its caveat is also queueing-shaped:
   waiters' latency equals the fill's latency, so collapsing without a waiter deadline
   trades a stampede for a pile-up — pair it with fail-fast (#5758) and with the
   Fastly-documented care around uncacheable responses (dequeue in batches, mark
   hit-for-pass) so a non-cacheable fill doesn't serialize everyone behind it.

5. **Hit ratios measured at low load mislead twice.** First, as in (1): the *value* of a
   hit is load-dependent. Second, the *ratio itself* is load-dependent: synchronized
   expiry and cold starts produce correlated misses at peak (the thundering herd), so
   the hit ratio observed in steady state overstates the hit ratio during the only
   intervals where the path is in trouble. Report hit ratio conditioned on load (or on
   incident windows), or it will keep certifying the cache right up to the moment it
   matters. The symmetric trap exists for the tunnel path itself: a warm-pool p50
   measured at idle certifies nothing about the post-recycle herd.

6. **Edge placement is queue relocation.** Every hit at an edge deletes that request
   from every downstream queue — λ falls at the bottleneck, so by Little's law the queue
   shortens for the misses too. Conversely, a cache placed *behind* the bottleneck
   protects the origin but does nothing for path latency. When we discuss where a fleet
   cache or shield should sit, the question to ask is: *which queue does a hit here
   delete?*

---

## 5. Critical mapping: does bufferbloat explain OUR numbers?

The measurements on the table (WS-C, #5701): the sandbox↔laptop path runs pooled HTTP
CONNECT tunnels through the runtime proxy (:3130) plus a DERP hairpin; the pool is 50
tunnels. Warm per-call p50 is 369 ms, of which actual daemon work is 1–11 ms. A 502 trip
costs p50 16.3 s (n=119); a no-status trip 26.0 s (n=73). Connection setup serializes:
latency in the 50→128 tunnel ramp grows roughly linearly with concurrency. Post-recycle,
clients arrive as a herd.

### 5.1 What matches

- **Delay ≫ service time.** ~97% of the warm p50 is not work. In queueing terms the call
  is almost all wait. That is the *shape* of every queueing pathology in this literature
  (and of no other common failure mode) — something on the path is making requests wait
  in line. Gettys' signature is the same shape at packet scale.
- **The failure tax is denominated in tens of seconds.** 16.3 s / 26.0 s stalls are the
  timescale of timer interactions and deep queues draining — RTO ladders, exponential
  backoffs, buffers holding seconds of data (cf. Gettys' modem; the meltdown spiral).
  CPU-bound work does not naturally produce clean multi-second plateaus; queues and
  timers do.
- **Herds after recycles.** Synchronized re-arrival is the stampede pattern from the
  caching literature and the retry-storm pattern from Brooker. Deep, unmanaged queues
  anywhere on the path convert a herd into long sojourns instead of fast, cheap
  failures — and our current design has no fail-fast (#5758 is still a design), so
  nothing sheds the load early.
- **There is at least one certified dark buffer on the path.** The DERP relay leg is, by
  construction, a queue on a machine we do not instrument, carrying traffic in a pattern
  (hairpin) that doubles its exposure to the path. The relay literature's warning —
  relayed paths show higher latency *and variance* — matches a path whose p50 is 369 ms
  against 1–11 ms of work on what should be a short hop.

### 5.2 What doesn't match

- **Classic bufferbloat is load-dependent by definition.** A standing queue exists only
  while a sender is trying to overfill the path. If the warm 369 ms p50 persists at
  near-zero concurrency — one call at a time, nothing else moving — it is *not* bloat; it
  is fixed path cost: hairpin propagation, per-hop handling, proxy overheads. Bloat
  predicts a latency-vs-load curve that climbs; a flat 369 ms floor is architecture,
  not queueing. (WS-C's number does not record the load at measurement time; that gap is
  itself one of the findings.)
- **Setup serialization is not bloat.** Roughly linear latency growth across the
  50→128 ramp indicates a serialized resource — handshakes processed one at a time,
  proxy CPU, TLS/session setup. That is a *service-rate* limit at a setup server.
  Queueing is involved (requests wait for the serializer), but enlarging or shrinking
  buffers changes nothing; the fix is parallelism/pre-warming/multiplexing (#5756), i.e.
  deleting the setup from the critical path — a different lever than AQM.
- **The incident's prime mover was not a queue.** The 2026-11-04 wedge itself was CPU
  saturation on the sandbox (load 30 on 4 CPUs, a .gh-wrapped tree at 1645% CPU) plus a
  silent sampler. Bloat may describe how the *recovery* felt from the laptop side; it
  does not explain the wedge. Conflating them would send the fix effort to the wrong
  machine.
- **Small-payload RPC is not the classic bloat workload.** Textbook bloat needs a flow
  trying to move bulk data through the bottleneck. Our warm calls are tiny. For bloat to
  inflate *their* latency, someone else's bulk (or an aggregate of many small flows)
  must be keeping a shared queue full — possible (restores, backups, herd moments), but
  it is a claim that needs a witness, not an assumption.

### 5.3 The honest middle

The closest literature relative of our stack is probably not the home-router bloat of
2011 but its two cousins from §1: **HoL blocking** (many independent calls serialized
through shared ordered tunnels — one stall taxes every call behind it) and
**stacked-timer coupling** (RPC timeouts/retries layered over CONNECT/TCP/DERP delivery
guarantees, TCP-over-TCP geometry at one remove). Both are queueing pathologies; both
are in this document's taxonomy; neither is fixed by AQM on a router. This is also why
#5756 (independent streams), #5757 (fewer trips), and #5758 (time-capped waits) are the
right shapes of fix regardless of how §5.4 comes out: they attack serialization,
arrival rate, and wait caps — the three terms of Little's law — rather than betting on
one named disease.

### 5.4 Discriminating measurements (testable claims)

Each measurement is stated with the prediction that would confirm queueing/bloat-family
behavior, and the observation that would kill it. All are buildable on #5702's four trip
timestamps plus host tooling; none requires new infrastructure.

1. **Latency-vs-load curve on the tunnel path (RRUL-style).** Warm-call latency at
   concurrency 1 / 5 / 10 / 25 / 50, idle path, then repeated with one bulk transfer in
   flight. *Bloat/queueing predicts:* latency climbs steeply with load (claim: p50 at
   concurrency 50 > 3× p50 at concurrency 1) while throughput stays flat, and the bulk
   flow taxes interactive calls disproportionately. *Kills it:* latency flat vs load →
   the 369 ms is fixed path cost; spend effort on the hairpin, not on queues.
2. **Per-segment sojourn decomposition.** Split the 369 ms by #5702's timestamps:
   client→proxy, proxy→DERP→laptop, broker queue, daemon service. *Queueing predicts:*
   the waiting share concentrates in one segment and grows with load. *Kills it:* shares
   constant across load → fixed per-hop costs.
3. **Pool acquire vs in-call split.** Instrument TunnelPool with the HikariCP pair:
   waiters-pending gauge + acquire-time histogram, separate from call duration.
   *Pool-queueing predicts:* sustained pending > 0 and acquire p99 tracking the warm
   latency inflation. *Kills it:* acquire ≈ 0 while call time stays 369 ms → the wait
   is downstream of the pool.
4. **Retransmit/RTO fingerprint during failure windows.** `ss -ti` on tunnel sockets
   while 502/no-status trips occur. *Meltdown-family predicts:* retransmit bursts and
   inflated RTOs on outer connections coincident with inner timeouts. *Kills it:*
   clean sockets during failures → the tax is application-layer timeouts, not transport
   coupling.
5. **DERP vs direct A/B.** Same call, same hour, over the hairpin vs a direct tailnet
   path (tailscale status/ping to establish which is which). *Relay-queueing predicts:*
   direct collapses latency toward daemon time (tens of ms) and slashes variance.
   *Kills it:* direct ≈ hairpin → the relay leg is innocent; look at proxy/broker.
6. **Failure-tax decomposition.** For individual 502/no-status trips, log where the
   16–26 s went: client timeout budget, per-layer retries, RTO ladder, proxy timeout.
   *Timer-coupling predicts:* durations cluster at sums of doubling timers / stacked
   timeout values. *Kills it:* a single constant dominates → one mis-set timeout is the
   whole tax, and #5758 is a one-line fix.
7. **Herd replay.** Restart a client cohort against a warm path with and without
   jittered reconnect backoff (Brooker). *Stampede dynamics predict:* unjittered cohort
   reproduces the failure tax; jittered cohort drains cleanly at the same total work.
   *Kills it:* identical outcomes → herd shape is not the amplifier; the path fails at
   steady load too.

**Reading of the likely outcome, stated in advance so it can be wrong:** measurement 1
will show a mostly-flat warm curve with a real knee under herd/bulk conditions;
measurement 5 will assign the largest single share of the warm 369 ms to the hairpin's
fixed cost; measurement 6 will find the failure tax is mostly stacked timeouts/retries,
i.e. timer coupling rather than standing queues. If all three land that way, bufferbloat
proper explains the *tail* of our problem (herds, bulk interference) and not its median —
and the caching/fail-fast agenda in §3–§4 remains the right spend, because the tail is
where the incident lived.

---

## 6. Pointers

- Incident record and measurements: #5701. Trip instrumentation: #5702.
- Designs this reference supports: #5756 (multiplexed streams), #5757 (coalescing),
  #5758 (fail-fast). Broker redesign adjacent: #5703.
- When a future caching discussion needs a term from here: *standing queue*, *sojourn
  time*, *collapsed forwarding*, *stale-while-revalidate / stale-if-error (RFC 5861)*,
  *full jitter + retry budget*, *pool acquire vs service split*, *good queue vs bad
  queue*. Each is defined above with its source.
