# P2P composition options compared: baseline, A, C, A+C, A+B, A+B+C, and the B trajectory

Lane P2P-COMPARE, 2026-11-23 — an addendum to the capability-auth +
p2p-transport investigation (tracking issue
[developing-today/code#13779](https://github.com/developing-today/code/issues/13779)).
It compares the synthesis's compositions **against each other and against
the naive baseline stack Drew named** ("just Willow + Meadowcap + p2panda

- iroh"), on the six dimensions Drew specified. It does not re-litigate
  the recommendation; §8 notes the one place the comparison softens the
  recommendation's footing.

**Sources — no new research.** Everything below is drawn from the
on-disk programme: the synthesis
(`doc/2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison/`), the
family notes in the research bundle
(`doc/2026-11-23T22-02-29Z_research_p2p-auth-transport/`), and the
maturity census of 2026-11-23. Where the programme never examined a
question, the text says so and labels any judgement as judgement (§9).

---

## 1. The options, defined

**0 — Baseline: "just Willow + Meadowcap + p2panda + iroh."** The stack
someone names before the investigation: Willow's data model and its
native authorisation (Meadowcap) for replicated state, p2panda as the
sync machinery, iroh as transport. No broker renewal service, no
session-token layer, no Keyline semantics. This option contains a fork
the name hides, and the fork matters more than the option:

- **0-M, Willow as data model.** Willow's _semantics_ — namespaces,
  subspaces, paths, mutable entries, traceless deletion — adopted as the
  estate's data model for the domains they fit, with entries carried on
  p2panda operations so LogSync and iroh do the moving, and Meadowcap
  areas mapped onto the model. Coherent, but it quietly contains an
  undesigned implementation: a Willow-semantics layer (entry model, join
  semantics, prefix pruning, range fingerprinting) over p2panda logs.
  The research programme never designed or priced this composition; its
  cost below is judgement from component facts, flagged as such.
- **0-P, Willow as protocol.** The willow25 crates _are_ the data plane:
  state CRDT, embedded-KV store, Meadowcap native, sync by Willow's own
  protocols. In this reading p2panda has no real job — Willow's sync is
  a separate (missing) layer — so 0-P is really "Willow for everything,"
  priced in the Willow note: no Rust sync implementation exists,
  Confidential Sync is paused mid-rewrite, WTP is an early proposal, the
  only willow-over-iroh implementation (n0's iroh-willow) is stale at
  iroh 0.34-era APIs, and the store is sled/redb/fjall, not SQLite.

Unless stated otherwise, "baseline" below means **0-M**, the strongest
honest reading; where 0-P differs decisively it is called out.

**A — p2panda data plane + Meadowcap-as-spec + broker renewal.**
p2panda core/sync/net/store unchanged as the data plane (its log model
stays the data model). Meadowcap's validity rules adopted as a Final
specification — the `meadowcap` crate or a house re-implementation
checked against the two existing implementations — bound to p2panda as
a header-extension token (area = topic × path-prefix × time window).
p2panda-auth demoted to replicated membership state. The fleet broker
runs the renewal service Meadowcap's model requires: short-window
grants at check-in; revocation is non-renewal plus supersession
hygiene.

**C alone — Biscuit + capnp-over-iroh for live sessions.** An iroh
stream (NodeId identity, QUIC/TLS 1.3) carrying Cap'n Proto RPC Level 1
with facet attenuation and interposition revocation; Biscuit tokens —
expiring, attenuable, offline-checkable, bound to the holder's NodeId,
revoked via a per-block deny-list — authorise _opening_ the session.
**What C alone leaves undecided, honestly:** everything about
replicated data. C has no data plane, no data authorisation, no group
story. Adopted alone, the estate's replicated state stays exactly where
it is today (broker + Turso + tsnet-era patterns) and C is an
_addition to the present_, not a stack.

**A + C — the recorded recommended default.** A for replicated data,
C for live sessions. The two layers answer different questions and
share only iroh and the broker.

**A + B — A plus a house Keyline engine.** Everything in A, plus
Keyline's semantics — delegation/revocation certificate sets evaluated
as derived authority — implemented in-house from Ink & Switch's design
documents and verified against the `keyline` conformance suite, owning
_membership, revocation-of-record, and relay power_: the Cloudflare
doors hold `Relay` (store and forward ciphertext, cryptographically
unable to read) as a power inside the authorisation graph. Meadowcap
still authorises each entry write. Two authorisation systems run at
once, with an explicit coherence boundary between them.

**A + B + C — the full stack.** A+B's data and membership layers plus
C's session layer.

**B as trajectory — not a stack, a switch.** The future state in which
Keyhive's code matures (gaps G1/G2 closed, the ciphersuite construction
independently reviewed, BeeKEM's paper through peer review) and
Keyline-as-code displaces Meadowcap-as-spec as the authorisation layer
for replicated data. §7 prices the switch and what has to be true.

---

## 2. The matrix

Compressed to phrases; §3 carries the texture. Build sizes are relative
judgements (small / moderate / large) from component shape — the
programme prices nothing in person-time (§9).

| Dimension               | 0 Baseline (0-M)                                                                                                                                                                     | A                                                                                                                                                                                  | C alone                                                                                                                                                      | A+C                                                                                         | A+B                                                                                                                                | A+B+C                                                                     | B trajectory                                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Develops into**       | Willow-semantics dialect of the estate; exit cost grows with data modelled in it. 0-P: estate becomes Willow's third implementation team                                             | Scoped grants + edge verification now; every later move (B, UCAN) still open — auth layer is the only replaceable part                                                             | Session surfaces get production authority; data plane untouched, decision deferred, not avoided                                                              | The full five-surface cover with the least invented machinery; B stays a switch, not a fork | Permanent revocation + Relay now; converges with B-trajectory from strength if Keyhive matures                                     | Everything in A+B, plus sessions; the most capable end state on the table | A's present, B's future; waiting costs renewal-service operation, nothing structural                                              |
| **Live sub-choices**    | Hash world (WILLIAM3 vs BLAKE3); window discipline without a service; encryption story (none native)                                                                                 | Meadowcap crate vs house re-implementation; root curve (Ed25519 vs P-384); window lengths per surface                                                                              | Biscuit now vs wait for UCAN 1.0; facets vs bare streams; Cap'n Web at the JS edge or not                                                                    | A's and C's, independently tunable                                                          | Engine's remit (membership only vs all revocation-of-record); who wins a Meadowcap/Keyline disagreement                            | Sequencing: engine before, with, or after the session layer               | The trigger set; adopt upstream code vs keep a house engine if one exists                                                         |
| **Looks like**          | Willow-shaped entries on p2panda logs; Meadowcap chains verified offline; live sessions = raw iroh streams, key possession is the whole credential                                   | Logs + travelling area tokens; broker window service at the centre; membership in p2panda-auth                                                                                     | Broker-minted Biscuits open capnp sessions over iroh; deny-list in SQLite; data plane = today's estate                                                       | Both sketches, on their own layers, sharing iroh + broker                                   | A, plus certificate tables + evaluator in the broker/seat path; doors hold Relay                                                   | A+B's sketch with C's session story at every live surface                 | A's sketch until the trigger; then grant state re-issued as certificates, renewal service retires, data never moves               |
| **Gains over baseline** | — (it is the baseline). Over doing nothing: a real data model for mutable state + offline-verifiable write auth                                                                      | Sustainable short windows (revocation latency = minutes, not grant lifetimes); one-shot scoped grants as routine; no second data plane; SQLite intact                              | Expiring session credentials; immediate revocation by interposition; token verification inside Workers                                                       | Both gain sets; the only row that covers all five estate surfaces                           | Coordination-free permanent revocation; Relay as a cryptographic power; late binding; one membership primitive                     | All gains in the table                                                    | vs A+B now: upstream's fixes instead of G1/G2-era semantics imported into house code                                              |
| **How much is built**   | Specs Final; Rust data-model crates research-grade alphas; **no Rust sync**; TS implementation most complete; one production deployment (Ourcana, Dart, ~9–10k users)                | p2panda 0.8.1 pre-1.0 (lower layers ~2 yrs; spaces integration landed 2026-11-09); Meadowcap spec Final 2025-11-21, two implementations; renewal service does not exist anywhere   | Biscuit: reference implementation, production use, ~13M crate downloads. capnp-rust: Level 1 only, single-maintainer lineage; C++ proven at Cloudflare scale | The union; the least mature shared piece is p2panda's young auth/encryption integration     | Keyline: pre-alpha, in-memory store only, conformance suite as contract; BeeKEM pre-alpha, unaudited, G1/G2 open, paper unreviewed | The union; adds Keyhive's pre-alpha risk to A+C's profile                 | Trigger conditions are upstream events: G1/G2 closure, independent review, BeeKEM peer review — none has a date                   |
| **We build**            | IMPLEMENT (moderate–large, undesigned): Willow-semantics layer over logs. 0-P: IMPLEMENT (large) + OWN-FOREVER: Rust sync against a paused spec. E2EE for Willow entries: unexamined | IMPLEMENT (small–moderate): Meadowcap validity core or crate adaptation + header binding. IMPLEMENT (small, policy-heavy): renewal service. GLUE: membership/grant coherence rules | IMPLEMENT (small–moderate): session-opening protocol, facet/membrane library. GLUE: NodeId binding, deny-list service, schemas                               | A's + C's lists; nothing extra emerges from the combination                                 | IMPLEMENT + OWN-FOREVER (moderate code, high assurance): Keyline engine, SQLite store, conformance runner, subject binding         | A+B's + C's lists                                                         | At switch time: IMPLEMENT (moderate): subject re-binding + grant re-issuance (a re-issue, not a conversion); data migration: none |

---

## 3. Per-option sections

### 3.0 Baseline — Willow + Meadowcap + p2panda + iroh, nothing else

**What develops out further.** In the good upstream future —
Confidential Sync unpauses, a maintained Rust sync lands, Earthstar 11
matures — 0-M's semantics layer becomes a thin adapter over a healthy
ecosystem and the estate was early on the right data model. In the
stall future, worm-blossom remains two grant-funded people
(donation income ~$3K/year per the Willow note), the semantics layer is
ours alone, and every dataset modelled in Willow semantics raises the
cost of leaving. 0-P's stall future is harsher: the estate _is_ the
Rust sync implementation team, against a spec its authors paused
mid-rewrite. Either way, the baseline forecloses nothing about
sessions — it simply never addresses them, so the foreclosure happens
by default, in whatever ad-hoc shape the exec surfaces grow.

**What the options are.** The 0-M/0-P fork above; inside 0-M: the hash
world (Willow'25 payloads are WILLIAM3/Bab; iroh-blobs is BLAKE3 — n0's
iroh-willow chose BLAKE3 and thereby non-interoperability with willow25
peers, so this choice is load-bearing and early); window discipline
without a service (issuers pick windows and re-issue by hand — short
windows are possible in principle, a standing operational burden in
practice); and the encryption story, which is the baseline's softest
spot: Willow has no built-in E2EE, and p2panda-encryption's spaces bind
encryption contexts to p2panda-auth groups over p2panda operations —
whether that carries Willow-semantics entries is a question the
programme never examined (flagged, §9). As specified, the baseline's
honest encryption answer is channel-only.

**What it would look like.** A seat holds a Meadowcap chain granted
days or weeks ago (long, because nothing renews it). It writes an entry
— say, a registry update — into its subspace; the entry carries its
full delegation chain and signature; any peer, offline, verifies the
chain and joins the entry into current state, where it may supersede an
older entry at the same path. A compromised seat's writes stay valid
until its window ends, and are then countered only entry-by-entry, by
supersession hygiene writes. A live session — a runner exec, a
coordinator handoff — is a raw iroh stream: possession of the seat key
is the entire credential, there is no expiry to wait out and no
interposition to flip; killing it means rotating keys out-of-band.

**What we would gain.** Against doing nothing: a genuinely better data
model for mutable infrastructure state (registries, configs, seat
state) than append-only logs — named state, traceless deletion,
difference-proportional reconciliation — plus Meadowcap's
offline-verifiable, receiver-bound write authorisation. Against A, the
baseline gains nothing in authorisation (same Meadowcap, worse
operated) and loses the log model's auditability: Willow stores keep
current state only, _by design_ — no history is a feature for the data
and a loss for an estate that audits.

**How much is built.** The two finished layers are finished: data-model
and Meadowcap specs are Final, with two independent implementations
each. Everything around them is not: canonical Rust crates are 0.6–0.7
alphas (census: willow25 0.7.11 stable, 0.7.12-alpha.1 newest,
2026-11-18) with a recent hash-correctness incident in their history
(every bab_rs before 0.7 computed incorrect WILLIAM3 digests); the TS
implementation is the most complete; the one production deployment
(Ourcana, ~9–10k users) is Dart. Sync in Rust: does not exist. Team:
two people.

**What we would need to build ourselves.** 0-M: IMPLEMENT,
moderate–large and currently _undesigned_ — the Willow-semantics layer
over p2panda logs (entry model, join, prefix pruning, incremental range
fingerprints), plus the Meadowcap binding A also needs. 0-P:
IMPLEMENT, large, and OWN-FOREVER — a Rust sync implementation against
a paused spec, a store on sled/redb/fjall or a house SQLite store (the
Willow note prices both hard parts: incremental fingerprints over
arbitrary area slices, prefix-prune range deletes), and the transport
integration iroh-willow attempted and abandoned at iroh 0.34. Feeds
decision points **5** (this _is_ DP5, taken estate-wide instead of
per-domain), **6** (a non-SQL store beside Turso, or a house store),
**3** (E2EE unexamined), **7** (deepest dependence on the smallest
upstream in the set).

### 3.1 A — Meadowcap-as-spec with broker renewal

**What develops out further.** A is the option whose future is least
hostage: the capability layer is a _Final_ spec, so upstream stall is
survivable (a frozen spec does not rot); p2panda stall is survivable
(~60k-line codebase, permissively licensed, forkable). If upstreams go
well, A absorbs the improvements without re-architecture: p2panda 1.0
strengthens the plane, a conformant UCAN pressures the token format,
and B remains reachable _because the data plane never changes_ — A is
the only authorisation choice in the set that keeps every other choice
open. What A forecloses: permanent revocation, permanently. Non-renewal
is a posture, not a mechanism, and no amount of A's own development
turns it into one.

**What the options are.** The `meadowcap` crate vs a house
re-implementation of the validity core (the crate exists and is
spec-grade; its const-generic/ufotofu flavour is the reason
re-implementation is on the table at all — the validity rules
themselves are pure computation: area inclusion, chain verification).
The root-key instantiation: Ed25519 aligned with NodeIds, or P-384
aligned with the interim IdP CA — Meadowcap is generic over signature
schemes, so both are spec-legal; this is decision point 4 in miniature,
and A is the only option where P-384 survives at the data layer.
Window lengths per surface (minutes for seats, lease-shaped for
runners, longer for rarely-online peers — with the tension named:
Meadowcap's own essays state it). Renewal at check-in vs pushed;
supersession hygiene automated in the broker vs manual.

**What it would look like.** A seat checks in to the broker and
receives a fifteen-minute area grant over its topics. Its writes carry
the chain; peers verify offline and apply them. At window end the
grant dies unless renewed — revocation is the broker declining to
renew, plus hygiene writes that supersede the seat's entries. A runner
gets the pattern in its purest form: an area-and-window grant for
exactly one artifact topic, worthless before and after. Membership —
who is a seat at all — lives in p2panda-auth as replicated state, and
a written coherence rule keeps membership and grants from disagreeing.

**What we would gain** (over baseline). Short windows become a _system_
instead of a discipline: revocation latency drops from grant lifetimes
to window lengths. Scoped one-shot grants become routine rather than
hand-built. The data plane stays one plane, on SQLite, with its
history intact. Verification is pure computation — the TS Meadowcap
implementation runs in Workers, so the Cloudflare doors can check the
same tokens the seats carry. And alignment with p2panda's own
documented composition story: this is the integration its docs point
at.

**How much is built.** p2panda 0.8.1 (2026-11-13; 1,591 commits, 19
contributors, 9 releases in 12 months) — pre-1.0, breaking changes per
minor, but the lower layers carry ~2 years of publication history. The
young parts are exactly the adjacent ones: p2panda-auth first published
2025-07; spaces — the auth/encryption integration — landed in the
high-level API in 0.8.0 (2026-11-09). DCGKA encryption is unaudited,
with group control messages currently unencrypted (membership metadata
exposed) — A's data plane inherits that status verbatim. Meadowcap:
spec Final 2025-11-21, two independent implementations.

**What we would need to build ourselves.** IMPLEMENT, small–moderate:
the Meadowcap validity core (or crate adaptation) + the
header-extension binding to p2panda. IMPLEMENT, small in code and
heavy in policy: the renewal service — window policy, clock and
future-dating discipline (capability windows bind _claimed_
timestamps, so timestamp policy becomes security-critical operations),
supersession hygiene automation. GLUE: the membership↔grant coherence
rules. Nothing here is OWN-FOREVER at the scary end: the validity core
is bounded, spec-pinned, and checkable against two reference
implementations. Feeds decision points **1** (A _is_ DP1's option (a)),
**4** (the curve instantiation), **3** (DCGKA accepted as-is, or not),
**6** (only if the estate also funds the libsql store backend — common
to every p2panda option, not A's alone).

### 3.2 C alone — the session layer, and only the session layer

**What develops out further.** C's pieces are the most mature in the
set and the least likely to strand: Biscuit's trajectory is production
hardening, capnp's is stasis at Level 1 (proven, thin). The live
evolution risk is upward pressure from UCAN: if a conformant UCAN 1.0
Rust implementation stabilises, C's token format wants migrating
(invocation separation, `did:key` identity are strictly nicer) — a
format migration inside one layer, not a re-architecture. What C alone
develops _in the estate_ is narrower: the Actions-exec and coordinator
surfaces get real authority while they are being built this month —
and the data-plane decision gets deferred into a future where more
data has accumulated in today's patterns. Deferral is the cost; C
alone buys time at interest.

**What the options are.** Biscuit now vs holding the token slot for
UCAN (Biscuit's P-384 exclusion and UCAN's suite exclusion of P-384
make this partly a curve decision — DP4 again). Facets over capnp vs
Biscuit over bare iroh streams (the facet layer is where interposition
lives; skipping it keeps expiry but loses immediate, granular
revocation). Cap'n Web at the JS edge (first-party in Workers) vs
keeping the edge to token verification only. Deny-list placement:
broker SQLite (simple, central) vs replicated (available offline,
slower to bite).

**What it would look like.** The coordinator hands a runner a Biscuit
minted minutes ago, bound by a Datalog check to the runner's NodeId,
expiring with the lease. The runner opens an iroh stream and presents
it; inside the session, capnp facets expose exactly the surfaces the
lease implies — a dispatch facet, an artifact facet — each a
forwarder that can be switched off. Ending authority is physical:
sever the connection, flip the caretaker, or land the token's block ID
on the deny-list and every verifier, including a Worker door, refuses
it thereafter. Meanwhile the estate's replicated data sits wherever it
sits today, authorised by whatever authorises it today. That sentence
is C-alone's whole limitation.

**What we would gain** (over baseline). Everything the baseline
structurally cannot say about live authority: expiring credentials,
attenuation inside a session, revocation that is immediate rather than
eventual or windowed, and a token that verifies identically in the
fleet binary and inside a Worker (Biscuit's official WASM build).
Over A alone, the same list — A says nothing about sessions either.

**How much is built.** Biscuit: the Rust reference implementation
(`biscuit-auth`, 6.0 stable line; census records 7.0.0-alpha.4 as
newest, 2026-11-17), Eclipse-project governance, production use,
~13M crates.io downloads against 254 stars — the census's cleanest
stars≠adoption case. capnp-rust: capnp 0.28.2 (2026-11-22), Level 1
only, single-maintainer lineage; the C++ original is proven at
Cloudflare scale inside workerd, which is evidence about the design,
not about the Rust crates. UCAN, for contrast: spec 1.0 untagged,
rs-ucan frozen since 2023-06-28.

**What we would need to build ourselves.** IMPLEMENT, small–moderate:
the session-opening protocol (Biscuit → capnp bootstrap capability
over an iroh stream — including the handoff shape capnp lacks: no
sturdy refs, no library handoffs at Level 1, so the coordinator's
introduction protocol is house work), and the facet/membrane library
for broker surfaces. GLUE: the NodeId-binding check (a thin house
layer on a bearer token), the deny-list service (a SQLite table and a
distribution rule), capnp schema governance, Biscuit Datalog policy
authorship. The OWN-FOREVER component is discipline, not code:
membrane discipline is a coding standard — facets must actually be
written attenuated, every time. Feeds decision points **1** (the
live-session half of the revocation posture), **2** (buys boring
maturity — the cheapest ownership in the set), **4** (Biscuit excludes
P-384; C's token layer is Ed25519/P-256 by construction).

### 3.3 A + C — the recorded default, as a comparison row

**What develops out further.** A+C is A and C's futures running in
parallel, plus one interaction: the estate's two fastest-moving
surfaces (live sessions being built now; replicated fleet/atlas state)
each get their layer while the layers are young enough to shape.
Nothing in A+C forecloses B (same data plane), UCAN (one layer's
format), or per-domain Willow (a data-model addition, §3.0's 0-M,
could still be adopted _for a domain_ later — see §8). If both
upstreams stall, A+C degrades to: a forkable data plane, a frozen
Final spec, a production token library, and a Level-1 RPC — the
gentlest stall profile in the table.

**What the options are.** A's and C's sub-choices, independently
tunable — the composition's virtue is that its decisions don't couple:
window lengths don't touch session facets; the token format doesn't
touch the data plane. The one joint choice is operational: the broker
is the renewal service, the token minter, and the deny-list keeper —
three small services sharing one centre of gravity the estate already
has.

**What it would look like.** §3.1's story for data, §3.2's story for
sessions, simultaneously: a seat's writes travel with Meadowcap chains
and die by non-renewal; a runner's session opens on a Biscuit and dies
by interposition. A door verifies a Biscuit in-Worker and a Meadowcap
chain in TS. The broker's live decisions (claims, leases — the
atomic-conditional-update primitive the estate already chose in
#13765) stay in the broker, un-replicated, exactly where their
contention semantics belong.

**What we would gain.** Over baseline: both gain sets, and the only
complete surface coverage in the comparison — fleet seats/broker,
Actions runners, Cloudflare doors, mcpx (moderate: cross-check
implementations and bridge integration are ours), atlas/id data.
Over A+B (below): the same coverage with no pre-alpha component and
no security-critical engine owned — the difference is paid for in
revocation strength, which is §6's first contrast.

**How much is built.** The union of §3.1's and §3.2's maturity —
nothing in the composition is pre-alpha, and the least mature piece
(p2panda's spaces integration, landed 2026-11-09) is adjacent to the
composition rather than load-bearing inside it: A uses p2panda's
mature lower layers and treats its young auth crate as membership
state, not as the capability system.

**What we would need to build ourselves.** The union of A's and C's
lists. The honest total: two IMPLEMENT items of small–moderate size,
one small policy-heavy service, a session protocol and facet library,
and glue. No OWN-FOREVER security engine; the own-forever surface is
operational (renewal policy, deny-list hygiene, membrane discipline).
Feeds decision points **1, 2, 3, 4** in their mildest forms, and
leaves **5** deliberately unanswered per-domain (§8).

### 3.4 A + B — Keyline built, on top of A

**What develops out further.** A+B is the only option that changes
what revocation _means_ in the estate: from a posture (windows,
hygiene) to a fact (a certificate that no merge can un-apply). If
Keyhive matures, A+B converges with the B trajectory from a position
of strength — the semantics are already lived-with, the subjects
already bound — and the remaining question is whether upstream's code
replaces the house engine. If Keyhive pivots or stalls, the estate
owns an orphan design; the mitigation is real but partial: the design
documents are extensive and candid, and the conformance suite pins the
semantics — owning Keyline deliberately is feasible in a way that
owning, say, BeeKEM is not. What A+B forecloses: the _simplicity_ of
one authorisation system. Every future auth question has two places to
live, forever, until one layer is retired.

**What the options are.** The engine's remit: membership only;
membership + relay power; or membership + relay + all
revocation-of-record (with Meadowcap reduced to a per-entry token
format whose issuance the engine governs). The coherence rule for
disagreements (Meadowcap says the write is authorised; Keyline says
the writer is revoked — the only safe rule is _deny wins_, stated now
or discovered later). Whether BeeKEM's _shape_ informs group keys now
(synthesis: design reference only; DCGKA carries group E2EE until
BeeKEM is adoptable). And the folklore, adopted as operating policy or
not at all: constitutional flatness, fresh-key discipline after
compromise, rotate-after-admin-removal (an ex-admin's deny-only reach
is permanent), the root role treated like a root CA.

**What it would look like.** A's sketch, with a second authorisation
substrate under it: two append-only SQLite tables of delegation and
revocation certificates; an evaluator (SQL CTEs for the declarative
passes, a driver loop for the non-linear fixpoint — the implementers'
own recipe) answering derived-authority questions in ~1 ms at
180-member scale in the reference. When an admin revokes a seat, the
revocation certificate replicates with everything else and _survives
every merge_ — there is no replica state in which the seat is
un-revoked. The Cloudflare door holds `Relay`: it can prove, inside
the graph, that it may store and forward ciphertext and may not read.
A runner's short-lived authority is the awkward fit, stated plainly:
Keyline has no expiry _by design_ (no clocks), so short-lived
authority must be modelled as prompt revocation — which bites only
when replicas sync. Frequently-checking-in runners mitigate;
genuinely offline peers don't. That is why C, not B, owns sessions in
every stack that has both.

**What we would gain** (over A). The best revocation semantics in the
field — coordination-free and permanent, where A's is bounded-latency
and broker-dependent and p2panda-auth's is eventual and retroactive.
Late binding (supply a role with a new subject and every member has it,
no re-issuance). Redundant routes (no brittle proof chain whose
invalidation strands a delegate). Groups, roles, device collectives,
and documents as one primitive. And Relay — the single best fit in the
entire investigation for this estate's door architecture, converting
ambient trust into an auditable power.

**How much is built.** Keyhive is **pre-alpha** by its authors'
repeated statement: 328 commits, 8 contributors (census, 2026-11-23);
`keyline` ships with an in-memory store only; the conformance suite
exists and is treated by the design docs as the contract — which is
what makes a house implementation _verifiable_ rather than vibes.
BeeKEM: pre-alpha, unaudited; its paper (ePrint 2026/1434) is not yet
peer-reviewed; and the code carries disclosed gaps **G1** (the CGKA
authorisation path reads the positive graph, so a revoked member can
seat a fresh own key — a live revocation bypass in the key layer) and
**G2** (share keys unchecked against prekeys). A+B as defined here
adopts Keyline's _semantics_ only; BeeKEM's code stays out — but a
house Keyline engine is built in the shadow of a design whose only
implementation is pre-alpha, and pre-alpha designs move.

**What we would need to build ourselves.** IMPLEMENT + OWN-FOREVER,
moderate in code and high in assurance: the Keyline engine, the SQLite
store behind the designed-for store trait, the conformance runner, the
binding from Keyline subjects to p2panda topics and broker decisions,
and timestamping/anchoring where back-dating matters (the threat model
is explicit that without anchoring, a late old operation and a
fabricated back-dated one are indistinguishable). This is the only
option whose build list contains a security-critical authorisation
engine the estate maintains forever. Feeds decision points **1**
(DP1's option (b), taken), **2** (the maximal-ownership end of the
scale), **7** (a design dependency on a handful-of-people lab, taken
with eyes open).

### 3.5 A + B + C — the full stack

Everything in §3.4 plus §3.2's session layer. The comparison-relevant
facts are the interactions, and there are three. **First, the layers
stop overlapping:** with C owning live sessions, B's no-expiry weakness
loses its sharpest edge (runners were its worst fit), and B can be
scoped to what it is best at — replicated-state membership and
revocation-of-record. **Second, the revocation story becomes
complete in kind:** permanent for replicated state (Keyline),
windowed for data grants (Meadowcap), immediate for live authority
(interposition), deny-listed for session tokens (Biscuit) — every
revocation physics in the field, each on the layer it suits.
**Third, the ownership bill is the whole bill:** A's list + B's engine

- C's protocol and discipline, and an operations surface with four
  authorisation artefacts (chains, certificates, tokens, facets) whose
  coherence rules must be written down once, correctly. Sequencing is
  its live sub-choice: engine-first delays the session layer the
  in-flight builds want this month; sessions-first (the A+C order, B
  added) is the same end state reached through the recorded default —
  which is another way of saying A+B+C is less an alternative to A+C
  than its possible future, differing from the B trajectory mainly in
  _who writes the engine_ (us, now, from the design docs) versus
  _who writes it later_ (upstream, adopted when its gaps close).

### 3.6 B as trajectory — the switch, priced

**What has to be true.** The synthesis's triggers, restated as switch
conditions: G1 and G2 closed in Keyhive's code; the ciphersuite
construction independently reviewed (or a full audit landed); BeeKEM's
paper through peer review; and — the condition the code conditions
imply — APIs stable enough that adopting the code is not adopting a
moving target. None of these is an estate action; all are observable
upstream events (decision point 7 is where the estate decides whether
to _fund or hasten_ them rather than merely watch).

**What the switch costs, then.** Because A and B share the data plane
exactly, the switch touches only the authorisation layer: subjects
re-bound from topics to Keyline's model; grant state **re-issued** as
certificates (Meadowcap chains and Keyline certificates are different
artefacts — there is no conversion, only re-issuance under the new
engine, which the broker's check-in cadence makes a rolling operation
rather than a flag day); per-entry verification changes from chain
checking to derived-authority evaluation; the renewal service retires
(its reason to exist — Meadowcap's missing revocation — is the thing
being replaced). Data migration: none. Session layer: untouched (C
composes with B exactly as with A).

**What committing now costs, by contrast** — i.e., A+B today: the
engine is built against a pre-alpha design whose reference
implementation may still move; G1/G2-era semantics are imported as
house folklore even if the buggy code is not; and if upstream's design
_does_ move, the estate maintains a dialect. The asymmetry is the
point: **waiting is cheap precisely because A exists** — every month
on A is a month of scoped grants and working renewal, not a month of
exposure — and the switch's price does not grow with time spent on A,
because nothing in A needs undoing. The one cost that _does_ grow with
waiting: if the house engine is eventually wanted regardless (because
upstream stalls indefinitely), every year of delay is a year the
estate's revocation stayed windowed. That is a DP1 judgement about how
long windowed revocation is acceptable, not a scheduling fact.

---

## 4. Where the options actually differ decisively

Four contrasts carry nearly all of the decision weight. Everything
else in the matrix is engineering preference by comparison.

1. **Revocation physics.** Three genuinely different answers to "how
   does a compromised seat's authority die": non-renewal (baseline
   without a service: at grant expiry, whenever that is; A: at window
   end, minutes away, _if the broker is reachable in the loop_);
   coordination-free permanence (B in any form: at the speed of
   replication, forever, with no centre); and interposition (C: now,
   for live authority only). No option has all three; A+C has the
   first and third; only B-containing options have the second.
   → **Decision point 1**, with point 3 setting how much the
   replicated-data half matters.
2. **Where the build burden sits.** The baseline puts it in the data
   plane itself — sync, store, semantics — the layer hardest to get
   right, hardest to audit, and hardest to exit once data lives in it.
   A puts it in a bounded, spec-pinned implementation plus a small
   policy-heavy service. B puts it in a security-critical engine owned
   forever. C puts it in protocol glue and coding discipline. The
   options are, more than anything, four different answers to "what do
   we volunteer to be responsible for." → **Decision point 2**, with
   points 5 and 6 as the baseline's specific forms of it.
3. **Live sessions exist, or they don't.** The estate's fastest-growing
   surfaces (Actions-runner exec, coordinator handoffs) need expiring
   credentials and immediate revocation _this quarter_. Only options
   containing C provide them; every other option — including the
   strongest data-plane option, A+B — leaves those surfaces on raw
   transport identity. This contrast is independent of the A-vs-B fork,
   which is why the synthesis treats C as needed under either.
   → **Decision points 1 and 2** (the live half of both).
4. **The curve quietly decides the token layer.** Meadowcap is the
   only capability layer in the set that admits the interim IdP CA's
   P-384; Biscuit supports P-256 but not P-384, and UCAN's suite
   excludes P-384. So adopting C (or a future UCAN) settles the
   session/token layer to Ed25519/P-256 whatever DP4 decides for the
   data layer — and adopting A is the only path that keeps P-384
   available where the CA lives. → **Decision point 4**.

The B-specific contrast — adopt semantics now vs code later — is
decision point **7** wearing technical clothes: every trigger for the
trajectory is an upstream event, so the real choice is whether the
estate merely watches Ink & Switch (and worm-blossom, and p2panda) or
funds/contributes/forks. No comparison table settles that; it is
resourcing posture.

---

## 5. Flagged: where this comparison touches the recommendation

One softening, stated once. The recommendation (A+C, B as north star)
treats Willow as _not adopted_ — decision point 5 left open
per-domain. The comparison shows the baseline's strongest reading
(0-M) is not dominated by A on every axis: for mutable infrastructure
state, Willow's data model is arguably the better _model_, and
Meadowcap-over-Willow-semantics is its native shape rather than a
binding. A+C therefore doesn't refute the baseline's data-model claim;
it _defers_ it, domain by domain. If a domain earns Willow semantics,
0-M is the shape it would take — as an addition alongside A, priced
with the undesigned-implementation warning of §3.0 — not as the
estate-wide default the baseline's name suggests. Nothing else in the
comparison moves the recommendation's footing: in particular, C's
necessity and the A/B data-plane identity (which makes B a trajectory
rather than a fork) survive the head-to-head intact.

---

## 6. Gaps and honesty notes

- **The 0-M composition was never designed in the research
  programme.** Willow-semantics-over-p2panda-logs appears in no note as
  an evaluated design; §3.0's treatment composes component facts
  (Willow's store/sync state, p2panda's log model, Meadowcap's
  parameterised authorisation) and its build size is judgement,
  labelled as such. The same applies to whether p2panda-encryption
  could carry Willow-semantics entries: unexamined; the baseline is
  therefore specified with channel-only encryption.
- **No build is priced in person-time anywhere in the programme.**
  Small/moderate/large are relative judgements from component shape
  (spec-pinned pure computation < protocol + discipline < engine <
  sync-against-a-paused-spec), not estimates.
- **Census vs notes, two version wrinkles.** Where the 2026-11-23
  census and the family notes differ on versions (keyhive_core's
  per-crate tags; willow25's 0.7.x line vs the notes' July-2026 audit
  figures; biscuit-auth's 7.0 alpha vs the 6.0 stable line), this
  record cites the census for versions and dates and the notes for
  design facts.
- **The baseline's session story is a definition, not a finding.**
  "Raw iroh streams, key possession as the whole credential" is what
  the baseline _contains_ by Drew's definition of it (no session
  layer); the notes evaluate the estate's current surfaces, not a
  hypothetical baseline deployment.

_Sources: synthesis record and research bundle for #13779 (paths in
the header); maturity figures from the 2026-11-23 census snapshot.
Decision-point numbering is the synthesis's §5, used unchanged._
