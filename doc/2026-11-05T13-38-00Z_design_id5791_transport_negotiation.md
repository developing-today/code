# Design: transport negotiation & fallback ordering for ID

- Date: 2026-11-05
- Status: **decision record** — no implementation is included in this change.
- Issue: [#5791](https://github.com/developing-today/code/issues/5791),
  "id: design: transport negotiation & fallback ordering (from #5671)".
- Parent design: [#5671](https://github.com/developing-today/code/issues/5671),
  "id: design: ID transport fallback for UDP-denied networks v1", whose
  decisions this record **does not re-decide** (see §1). Programme epic:
  [#2056](https://github.com/developing-today/code/issues/2056).
- Evidence baseline: `origin/main` at `f86e532b77`, worktree
  `~/git/fleet/10554-5791-transport-negotiation`, 2026-11-05. Verified at
  that baseline (triage task 10553): `doc/` and `docs/` contain no
  transport-negotiation or fallback-ordering design for #5671 or #5791;
  the probing, memory, override, and failure-classification questions
  below were undesigned before this record.

## 1. What the parent already fixed (not re-decided here)

#5671 fixes, and this record takes as given:

1. **Attempt order:** direct UDP → relay-assisted UDP (hole-punch /
   relay) → TCP → HTTP/HTTPS proxy (CONNECT, and where necessary plain
   HTTP / WebSocket-style framing).
2. **Broker shape and trust model:** the broker/relay is a dumb pipe
   forwarding opaque, end-to-end-encrypted traffic; it holds no session
   or content keys, never terminates ID payloads or SSH, and is
   authenticated by default (no open relay).
3. **E2E boundary:** ID data/sync payloads, queries and results, and
   tunneled SSH stay end-to-end encrypted between the true endpoints;
   anything not E2E must be named in the design that introduces it.
4. **Rendezvous reachability:** rendezvous must itself be reachable
   over plain HTTPS, or the fallback is circular; records are signed,
   TTL-bound, and fall back to last-known-good.

This record decides only _how a client chooses, remembers, overrides,
and learns from failures within that order_. Where an answer below
would change a parent decision it is flagged explicitly in §8 — none
does.

## 2. Decision summary (chosen shape)

Negotiation is a **per-peer, per-network state machine** driven by a
small, explicit **failure-classification** result from every attempt:

1. **Candidate list, in parent order.** For a peer, the client builds
   the ordered candidate list from (a) the peer's signed rendezvous
   advertisement (which transports/endpoints the peer offers), (b) any
   operator override (§5), and (c) remembered outcomes (§4). The parent
   order in §1 is the sort key; memory and overrides reorder only
   _within_ the rule in §3.2, never invent a new global order.
2. **Sequential probing with racing only at the top.** Attempts run
   sequentially in candidate order, each bounded by a per-attempt
   deadline (§3.3). Sequential — not parallel racing — is the default
   because probing is itself observable noise on restricted networks
   and parallel attempts multiply proxy/broker load and muddy failure
   attribution. The one exception: when memory holds a known-good
   transport for this (peer, network) pair, that transport is tried
   first, alone, with a short deadline; only on its failure does the
   full sequential walk run.
3. **Every attempt returns a failure class, not a boolean** (§6). The
   class decides the next action — fall through, retry with backoff,
   stop and surface, or quarantine a transport for this network — so
   fallback is a decision, not a thrash.
4. **Success is remembered per (peer, network), with decay** (§4), and
   a live connection **upgrades in place** when a better transport
   becomes usable, where the underlying protocol supports migration;
   otherwise upgrade happens at the next natural reconnect, never by
   dropping a healthy session to chase a better transport.
5. **Overrides always win over memory and probing** (§5), are scoped
   (global / per-peer / per-network / per-invocation), and are visible:
   the active override and the reason a transport was chosen are
   reportable (`id`-CLI status / diagnostics), never silent.

## 3. Probing

### 3.1 What a probe is

A probe is a real, minimal connection attempt on one candidate
transport to the peer's advertised endpoint for that transport,
carrying only the normal ID handshake (which is already authenticated
and E2E per the parent). There is no separate probe protocol and no
unauthenticated "ping" surface: a probe that cannot complete the
handshake has not proven the transport usable, and a dedicated ping
would add an unauthenticated endpoint for no extra signal.

### 3.2 Candidate construction

For peer _P_ on network _N_ (network identity per §4.2):

1. Start from _P_'s rendezvous advertisement, filtered to transports
   the client implements and the advertisement has not expired.
2. Apply forbids (override or a quarantined transport for _N_, §6.3):
   remove those candidates entirely.
3. Apply a pin (override): if a transport is pinned, it is the _only_
   candidate; probing and fallback are disabled and a failure is
   surfaced with its class — a pin is a diagnostic/operator decision,
   and silently falling back from it would defeat its purpose.
4. Otherwise sort by parent order, then stable-promote the remembered
   known-good transport for (_P_, _N_), if any, to the front as the
   single first attempt (§2.2). Promotion never crosses a forbid and
   never promotes a transport the current advertisement no longer
   offers.

### 3.3 Deadlines and budgets

- Each attempt has a per-transport deadline set by the latency-budget
  design (parent decomposition item 7); this record fixes only the
  shape: a deadline exists, is per transport (a UDP attempt and an
  HTTP-CONNECT attempt do not share one number), and expiry is the
  failure class `timeout` (§6), not a generic error.
- The whole negotiation has one overall budget. Expiry of the overall
  budget stops the walk and surfaces the last failure class per
  candidate tried; the client never loops the candidate list within a
  single negotiation.
- Re-probing a quarantined or previously failed transport happens only
  on (a) network change, (b) memory decay expiry (§4.3), or (c) an
  explicit operator request — never on a timer tight enough to be
  observable as scanning on a restricted network.

### 3.4 No probing theatre

If the advertisement offers exactly one usable transport, there is no
walk: attempt it, classify the result, done. Probing machinery must
not add round trips to the common direct-UDP case beyond the normal
handshake.

## 4. Per-peer memory

### 4.1 What is remembered

One record per (peer, network): `{ peer, network_id, transport,
endpoint, outcome_class, first_seen, last_success, last_failure,
consecutive_failures }`. Only the _last known-good_ transport and the
_last failure class per transport_ are retained — not a history log.
Memory is a local client cache: it is never synced, never sent to the
peer or a broker, and never part of a rendezvous record.

### 4.2 Network identity

`network_id` is a local, opaque fingerprint of the egress environment
sufficient to distinguish "home" from "sandbox" from "corporate Wi-Fi"
— e.g. a hash of the default gateway / DNS resolver set / proxy
configuration the client would use. It is deliberately coarse: its
only job is to stop a transport learned on one network being promoted
on another. It is computed locally, stored only as the hash, and
never transmitted.

### 4.3 Decay and invalidation

- A known-good record is trusted for promotion for a bounded TTL
  (default order of days, set by implementation with the latency
  design); after the TTL the record still orders the walk but is no
  longer given the exclusive first attempt.
- Any failure of the promoted transport immediately demotes it and
  increments `consecutive_failures`; two consecutive failures of the
  remembered transport clear its known-good status for that network.
- A peer's new rendezvous advertisement that stops offering the
  remembered transport invalidates the record at once.
- Memory never overrides a failure class of `policy_denied` or
  `auth_required` (§6): those are properties of the network/operator,
  not of luck, and retrying from memory is how clients get networks
  flagged.

## 5. Overrides

| Surface                    | Scope              | Effect                                               |
| -------------------------- | ------------------ | ---------------------------------------------------- |
| Config file                | global or per-peer | pin / forbid one or more transports; force proxy use |
| CLI flag                   | per-invocation     | same, wins over config for that invocation           |
| Environment (test harness) | per-invocation     | same, wins over config, loses to explicit CLI flag   |

Rules:

1. **Precedence:** per-invocation CLI > per-invocation env > per-peer
   config > global config > memory > parent order. Every layer is
   optional; an unset layer changes nothing.
2. **Pin** = only candidate (§3.2.3). **Forbid** = candidate removed.
   **Force-proxy** = the HTTP/HTTPS-proxy tier is promoted to first
   (it does not forbid direct attempts _after_ a proxy failure unless
   combined with a forbid — force-proxy expresses "try the sanctioned
   egress first", not "leak direct attempts never happened").
3. Overrides are **reported, not silent**: diagnostics show the active
   overrides, which candidate was chosen, and which rule chose it
   (pin / memory / parent order). A support exchange must be able to
   answer "why did it use the proxy?" from local state alone.
4. Proxy credentials referenced by an override live in the platform
   credential store, never in the config value itself, in logs, or in
   diagnostics output (parent open question; this record decides the
   _negotiation_ side: the override names a credential _reference_).

## 6. Failure classification

### 6.1 Classes

Every failed attempt returns exactly one primary class:

| Class                 | Meaning                                              | Examples                                                                                                               |
| --------------------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `udp_blocked`         | UDP egress appears denied                            | UDP attempt times out while a TCP/HTTPS attempt to the same peer/network succeeds, or ICMP administratively-prohibited |
| `peer_offline`        | No transport reaches the peer                        | All offered transports fail with timeout/refused and rendezvous is fresh                                               |
| `proxy_auth_required` | Egress proxy demands authentication                  | HTTP 407 from CONNECT                                                                                                  |
| `proxy_denied`        | Proxy refuses this destination                       | HTTP 403 from CONNECT                                                                                                  |
| `policy_denied`       | Local or network policy forbids the transport        | Pinned/forbidden by operator, OS sandbox denial                                                                        |
| `tls_failure`         | TLS to endpoint/proxy failed validation              | Certificate error — never a fallback trigger by itself                                                                 |
| `auth_failure`        | ID handshake authentication failed                   | Bad/expired peer credential                                                                                            |
| `timeout`             | Attempt exceeded its deadline, no other signal       | Slow network, black-holed TCP                                                                                          |
| `transport_error`     | Everything else, with the OS/protocol error attached | Catch-all; must carry the underlying error                                                                             |

### 6.2 What a class decides

- **Fall through to the next candidate:** `udp_blocked`, `timeout`,
  `transport_error`, and `proxy_denied` (proxy refusing one
  destination says nothing about direct transports, and vice versa).
- **Stop and surface, do not fall through:** `tls_failure` and
  `auth_failure`. Falling through on a TLS or identity failure would
  mask a possible attack or a broken credential as a "network
  problem" and could downgrade the session's assurance silently.
- **Stop and ask the operator (once):** `proxy_auth_required` —
  negotiation pauses for the credential reference (§5.4); without one
  it surfaces, it does not guess or brute-force.
- **Record as peer state, not network state:** `peer_offline` is
  remembered against the peer (all networks), so the client backs off
  the peer rather than re-walking every transport on every retry.

### 6.3 Quarantine

A transport that returns `udp_blocked` or `policy_denied` on network
_N_ is quarantined for _N_ for the memory TTL (§4.3): it is removed
from candidate lists on _N_ without needing an operator forbid, and
re-probed only under §3.3's conditions. Quarantine is per network —
UDP blocked in the sandbox must not remove direct UDP at home.

### 6.4 User-visible errors name the class

The error a user sees is the class plus the candidate walk in one
line each ("direct UDP — udp_blocked; TCP — timeout; HTTPS proxy —
proxy_auth_required"), never a bare "connection failed". Correct
retries follow from the class table above, not from heuristics in
callers: callers consume the class, they do not re-classify.

## 7. Invariants implementation must hold

1. **Parent order is the only global order.** Memory promotion and
   force-proxy change which candidate is tried _first_; neither
   changes the relative order of the remaining candidates.
2. **A pin means pin.** No fallback, retry-on-other-transport, or
   "helpful" direct attempt may happen under a pin.
3. **No silent downgrade.** A `tls_failure` or `auth_failure` never
   results in a session on a weaker-assurance path without an explicit
   operator action.
4. **Memory stays local.** Per-peer memory and `network_id` are never
   transmitted, synced, or logged in raw form.
5. **Classification before retry.** No retry or fallback happens
   without a recorded failure class for the attempt that prompted it;
   unclassified failures default to `transport_error` with the
   underlying error attached, never to a silent loop.
6. **Bounded negotiation.** One negotiation = at most one pass over
   the candidate list, inside one overall budget (§3.3).
7. **E2E is untouched.** Negotiation chooses a transport for the
   parent's E2E traffic; it never terminates, re-encrypts, or inspects
   payloads, and a broker on the chosen path learns nothing new from
   negotiation (probes are normal handshakes, §3.1).
8. **Upgrade never drops a healthy session** solely to chase a better
   transport (§2.4).

## 8. Relation to the parent — explicit check

No decision above changes a #5671 decision: the attempt order (§1.1),
broker shape/trust (§1.2), E2E boundary (§1.3), and rendezvous
requirements (§1.4) are used, not amended. Two parent _open questions_
are touched, and recorded here rather than silently forked:

- "Proxy-auth credentials: where stored…" — §5.4/§6.2 decide only the
  negotiation-side rule (credential _reference_ in config; pause-and-
  surface on `proxy_auth_required`). Storage itself stays with the
  HTTP-CONNECT traversal design (parent decomposition item 6).
- Connection migration (parent feature-set item 1) — §2.4 decides the
  negotiation-side rule (upgrade in place where the protocol allows,
  else at next reconnect; never drop a healthy session). Whether the
  wire protocol allows migration for each transport is item 2's
  (broker/framing) design, not this record's.

## 9. Alternatives considered and rejected

1. **Parallel racing (happy-eyeballs style) for all candidates.**
   Rejected as the default: on the restricted networks that motivate
   #5671, simultaneous UDP+TCP+CONNECT attempts are noisy, can trip
   egress monitoring, multiply broker load, and make failure
   attribution ambiguous (which attempt's error is _the_ error?).
   Racing the single remembered transport first (§2.2) captures most
   of the latency win with none of that. Racing may be revisited by
   the latency design (parent item 7) with measurements, for the
   direct-connect case only.
2. **No memory — probe from parent order every time.** Rejected:
   every reconnect on a UDP-denied network would pay a full failed-UDP
   timeout before reaching the transport that worked yesterday. The
   motivating sandbox case would be slow forever, deterministically.
3. **Memory keyed by peer only (not per network).** Rejected: a
   transport learned in the sandbox (proxy-only) promoted at home, or
   direct UDP learned at home promoted in the sandbox, is wrong in
   both directions and produces exactly the thrash this design exists
   to prevent. Per-(peer, network) keying is the minimum correct
   scope.
4. **Boolean success/failure with caller-side heuristics.** Rejected:
   it cannot distinguish "UDP blocked" from "peer offline" from
   "proxy wants auth", so retries mask outages, downgrade TLS/auth
   failures, and re-ask questions §6 answers once. Classification at
   the attempt site is the whole point of the ticket.
5. **A separate probe/ping protocol.** Rejected (§3.1): extra
   unauthenticated surface, and it proves reachability of a ping
   responder, not usability of the real handshake.
6. **Server-directed negotiation (peer dictates the client's order).**
   Rejected: the peer's advertisement states what it _offers_; egress
   reality is the client's local knowledge (memory, overrides,
   quarantine). A peer dictating order would override operator pins
   and could be spoofed into downgrading clients — advertisements are
   signed (§1.4), but even a genuine peer does not know the client's
   network.

## 10. Splits and follow-ups

This design does not split further. Implementation, when scheduled (a
separate decision per #5791), breaks naturally along the sections
above — candidate construction + walk (§§2–3), memory store (§4),
override plumbing (§5), classification plumbing (§6) — as
implementation tickets filed at that time, not as further design
tickets. The latency numbers this record deliberately leaves open
(per-transport deadlines, memory TTL value) belong to the parent's
latency-budget design (decomposition item 7), which should consume
§3.3's shape rather than re-deciding it.
