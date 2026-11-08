# Design: security binding review across adapters & relays

- Date: 2026-11-09
- Status: **decision record** — design only; no implementation is included in this change.
- Issue: [#5815](https://github.com/developing-today/code/issues/5815),
  "id: design: security binding review across adapters & relays (from #5674)".
- Parent design: [#5674](https://github.com/developing-today/code/issues/5674),
  "id: design: Wire format & transport adapters — postcard schema, non-UDP
  carriers, relays/proxies". Programme epic:
  [#2056](https://github.com/developing-today/code/issues/2056).
- Sibling records this review is written *against*:
  [#5811](https://github.com/developing-today/code/issues/5811) (WSS/HTTPS
  adapter, record in PR #7613), [#5813](https://github.com/developing-today/code/issues/5813)
  (relay & proxy protocol, record in PR #7653),
  [#5809](https://github.com/developing-today/code/issues/5809)
  (versioning/negotiation), [#5810](https://github.com/developing-today/code/issues/5810)
  (framing & fragmentation), [#5814](https://github.com/developing-today/code/issues/5814)
  (conformance coverage).
- Already-settled neighbours this record does **not** re-decide:
  [#2036](https://github.com/developing-today/code/issues/2036) messaging
  semantics (`doc/2026-11-01_design_id2036.md`),
  [#5791](https://github.com/developing-today/code/issues/5791) transport
  negotiation & fallback *ordering*
  (`doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`),
  [#5795](https://github.com/developing-today/code/issues/5795) broker
  authorization, allowlists and metadata minimization.
- Evidence baseline: `origin/main` at `62e12a0175`, worktree
  `~/git/fleet/13769-5815-security-binding`, 2026-11-09. Verified at that
  baseline: issue #5815 is OPEN with no implementing PR or decision record;
  the sibling records for #5811 and #5813 are in open PRs #7613 and #7653
  and both explicitly defer the cross-adapter security binding to this
  review (#5813 invariant 11, §"Review order").

## 1. The question

From #5815: *replay and downgrade protection reviewed across all adapters
and relays — the security binding that stops a transport downgrade
becoming a security downgrade.*

The adapter family (#5674 §4, ordered by #5791) deliberately spans
carriers with wildly different transport security: native UDP, relayed
UDP, TCP/TLS, WSS, plain HTTPS request/response, WebRTC, local IPC, and
store-and-forward carriers. A client *will* walk down that ladder on
hostile networks, and relays and middleboxes sit on several of the
rungs. If any security property of an ID session is supplied by the
carrier — TLS on one rung, nothing on another — then forcing a client
down the ladder (blocking UDP, breaking WebSocket upgrades, stripping
capabilities) is a security downgrade attack, and replaying frames
captured on one carrier into a session on another is a replay attack.
This record settles the binding rules that make both impossible by
construction rather than by per-adapter vigilance.

## 2. What is already fixed (not re-decided here)

### 2.1 From #5674 (parent)

- Adapters **MUST NOT change ID semantics; they only carry frames.**
- **TLS is transport encryption only, never a substitute for ID
  end-to-end identity.** Every adapter spec must state how #2036
  identity binds to the carrier.
- Version negotiation settles on the highest mutually supported schema
  version; unknown discriminators and oversized payloads have defined
  error behaviour — no silent coercion.

### 2.2 From #2036 (messaging semantics)

- Two message classes: **durable/chained** (origin-signed graph events,
  eventual, range-sync recoverable) and **ephemeral/live**
  (session-scoped MAC-authenticated frames with signed segment
  checkpoints; disconnected peers get **no backlog**).
- Invariant 9: **transport is not authority** — direct connections,
  relays and bridges all lead to the same ID validation.
- The shared frame cap is 1 MiB; no new chunking wire protocol.

### 2.3 From #5791 and #5813

- #5791 fixes the fallback attempt order (direct UDP → relay-assisted
  UDP → TCP → HTTP/HTTPS proxy), the dumb-pipe broker trust model, and
  the per-peer/per-network negotiation state machine with failure
  classification. **Ordering and failure classification stay with
  #5791; this record states which failures are security failures.**
- #5813 fixes the relay roles: relays forward opaque end-to-end
  payloads, never retain or replay durable facts, keep only in-memory
  lease/session bindings, and its invariant 9 already states "no silent
  downgrade" as a relay-side rule. This record generalizes that rule to
  every adapter and gives it mechanism.
- #5795 owns broker authorization, allowlists/capabilities, metadata
  minimization and the compromised-broker ceiling. This record consumes
  that contract; it does not restate authorization policy.

## 3. Decision summary (chosen shape)

**All session security binds to the ID layer — the origin signature, the
session handshake transcript, and a policy floor — and never to the
carrier.** Concretely:

1. **Carrier-independent authenticity.** A durable frame is valid
   because of its origin signature; a live frame is valid because of
   the session MAC keyed at handshake. Carrier TLS, where present, is
   additive defence in depth and is never load-bearing for validity,
   on any adapter, including the ones that always have it.
2. **Transcript binding.** The session handshake computes a transcript
   hash over everything negotiated: schema version range and settled
   version, adapter/carrier class, relay path (role identities, not
   payload keys), capability set, and security parameters. The
   transcript hash is an input to session key derivation, and both
   peers exchange a MAC/signature confirmation over it before any
   application frame flows. Any tampering with negotiation — stripped
   capabilities, a forced lower version, a substituted relay — yields
   different transcripts, failed confirmation, and an aborted session.
3. **A security floor negotiation cannot cross.** Each peer applies a
   local policy floor: minimum schema major, origin signatures required
   for durable traffic, MAC + signed checkpoints required for live
   traffic, no plaintext or unauthenticated mode on any adapter. The
   floor is never transmitted as a negotiable offer and cannot be
   lowered by a peer, a relay, or a carrier failure. An adapter or
   carrier class that cannot meet the floor is simply not offered.
4. **Replay protection per class, bound to the session.**
   - *Durable:* replay is idempotent by construction — origin-signed
     events carry their event identity and are deduplicated by the
     #2036 range-sync/event-graph contract. A relay or attacker
     re-injecting a captured durable frame creates no new fact.
   - *Live:* per-session monotonic sequence numbers under the session
     MAC, with signed segment checkpoints per #2036. Session keys are
     derived from the transcript (§3.2), so a frame captured in one
     session is cryptographically invalid in any other session, on any
     carrier. Keys are discarded at session close; there is no
     cross-session replay window to manage.
   - *Handshake and control:* handshake nonces are single-use and the
     confirmation (§3.2) binds them into the transcript, so a replayed
     handshake cannot resurrect a session. Relay registration and
     route advertisements are signed and expiring per #5813; an
     expired or replayed advertisement grants nothing.
5. **Downgrade is a classified security failure, never a silent
   transition.** Walking down the #5791 ladder changes the carrier
   only. Each new session re-runs the full handshake under §§3.2–3.3;
   if the settled context would fall below the floor, or the transcript
   confirmation fails, the attempt aborts and is reported upward with a
   security-distinct failure class (`security_downgrade_refused` /
   `transcript_mismatch`, recorded here as an interface obligation on
   #5791's classification, §6) — it is never retried at a weaker
   context inside the same attempt. Capability advertisements arrive in
   signed rendezvous records, so a network attacker cannot strip them
   undetectably: stripping changes the transcript the peer confirms.
6. **Relays cannot re-originate.** A relay forwards opaque frames; it
   cannot terminate, re-key, or re-sign an ID session, and hop metadata
   it adds is unauthenticated advisory data that never feeds ID
   validation. Member-relay validation-before-forward (#2036) lets a
   relay *check* an envelope signature; it never lets a relay *create*
   one. All signatures and MACs use purpose-separated domain labels
   (handshake confirmation, live-frame MAC, checkpoint signing,
   registration) — no key or label is shared across purposes, adapters,
   or the co-hosted relay roles distinguished in #5813, so a frame or
   token from one context is meaningless in another.

## 4. Alternatives considered and rejected

| Alternative | Verdict | Why |
|---|---|---|
| Rely on carrier TLS for session security | Reject | TLS terminates at relays, corporate middleboxes and TLS-terminating proxies — exactly the rungs where downgrade pressure exists. #5674 already forbids treating TLS as ID identity; making it load-bearing on some adapters recreates the downgrade seam between adapters. |
| Per-adapter bespoke security rules | Reject | The attacks this review exists to stop live *between* adapters (forced fallback, cross-carrier replay). N bespoke rule sets guarantee N+1 seams. One binding contract, stated once, is the design. |
| Negotiated downgrade with a warning / best-effort security | Reject | A downgrade an attacker can induce is not a negotiation outcome, it is the attack succeeding politely. The floor + abort rule (§§3.3, 3.5) is deterministic, testable, and matches #5813 invariant 9's "no silent downgrade". |
| One global replay window / sequence space across sessions | Reject | Breaks store-and-forward carriers and contradicts #2036's no-backlog live semantics; forces durable state for ephemeral traffic. Per-session binding (§3.4) achieves the same resistance with session-scoped state only. |
| Let relays terminate and re-originate sessions (hop-by-hop security) | Reject | Makes every relay a trusted security principal and contradicts the #5791 dumb-pipe model and #5813's opaque-payload rule. A compromised relay must be a metadata/availability problem (#5795's ceiling), never a content-integrity problem. |

## 5. Invariants implementation must hold

1. **No carrier-supplied validity.** No code path accepts a frame as
   valid because of the carrier it arrived on (TLS state, source
   address, relay identity). Validity is origin signature (durable) or
   session MAC (live) only.
2. **No session without transcript confirmation.** Application frames
   never flow before both peers have confirmed the same handshake
   transcript; confirmation failure aborts the session.
3. **Floor is local and non-negotiable.** The security floor is local
   policy, never a wire value a peer or relay can lower; no adapter
   offers a mode below it (in particular, no plaintext/unauthenticated
   mode exists to fall back to).
4. **Cross-session, cross-carrier replay is invalid.** Live frames are
   valid only under the keys of the session whose transcript derived
   them; durable re-injection is idempotent and creates no new fact.
5. **Downgrade attempts surface.** Every refused downgrade and
   transcript mismatch produces the security failure class of §3.5 —
   never a silent retry at a weaker context.
6. **Relays add no authority.** Relay receipts, hop metadata, lookup
   results and transport identities never change message validity
   (#2036 invariant 9, #5813 invariant 1).
7. **Domain separation everywhere.** Signatures and MACs are computed
   under purpose-specific labels; keys for handshake, live traffic,
   checkpoints and registration are independent.
8. **Unknown critical extensions fail closed** (per #5809); an adapter
   that cannot determine whether an extension is critical does not
   forward the frame.

## 6. Interface obligations on siblings (not forks of their decisions)

- **#5809 (versioning/negotiation)** must carry, in the negotiation it
  owns: the transcript fields of §3.2 (settled version, carrier class,
  relay role identities, capability set, security parameters) and the
  confirmation exchange. If #5809's settled design cannot carry a
  transcript hash, that is a change to a parent-level decision and must
  be recorded on #5674 — this record flags the dependency explicitly
  rather than assuming it away.
- **#5791 (negotiation & fallback)** failure classification gains the
  two security classes named in §3.5. Ordering does not change.
- **#5810 (framing)** carriers inherit §3 unchanged: framing and
  fragmentation never interact with the binding (a fragmented frame is
  validated whole, after reassembly, under the same keys).
- **#5814 (conformance)** should add golden adversarial vectors:
  capability-stripping, forced-fallback below floor, cross-session and
  cross-carrier replay, relay re-origination attempts — each expected
  to fail per §5. Filed as an obligation on the existing #5814, not a
  new split.
- **#5795 (broker authorization)** remains the owner of what a relay is
  *allowed* to do; this record owns what a relay *cannot* do to session
  security even if authorization fails or a broker is compromised,
  which is the binding half of #5795's compromised-broker ceiling.

## 7. Parent-decision statement

This record changes **no** #5674 decision: the wire-format family, the
adapter model, and the "TLS is not identity" rule all stand and are
consumed as-is. The one place it touches a neighbour's surface — the
negotiation transcript and the two failure classes — is recorded in §6
as an interface obligation on #5809/#5791, to be settled in their
records; if either cannot honour it, the escalation path is #5674, not
a silent fork here.

## 8. Splits

No new splits are filed by this record. The conformance work is an
obligation on the existing sibling #5814 (§6), and the negotiation
surface is an obligation on the existing #5809. If implementation of
either obligation proves to be its own design problem, that sibling
files the split under the parent programme's rule.
