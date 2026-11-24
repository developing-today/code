# Willow / Meadowcap — named mutable state, and the field's best-specified capability system

Full lane notes: research lane for
[#13779](https://github.com/developing-today/code/issues/13779), researched
2026-11-23 from the specifications at
[willowprotocol.org](https://willowprotocol.org/), the canonical Rust repo
(Codeberg: worm-blossom/willow_rs), the TypeScript implementations, the n0
iroh experiment, and crates/docs.rs listings. The collective behind it is
catalogued separately in the [Worm Blossom catalogue digest](worm-blossom-catalogue.md).

## What this family is

Willow is a **family of specifications** (not one protocol) for
synchronisable, multi-writer, peer-to-peer data stores, designed by
Aljoscha Meyer and Sam Gwilym (worm-blossom), generalising and formalising
the Earthstar data model. Earthstar (TypeScript, since 2020) is being
rebuilt on Willow as Earthstar 11; Willow is the reusable protocol layer
underneath it.

The core idea is **naming, not content-addressing, at the protocol level**:
you bind arbitrary payloads to meaningful hierarchical names, and later
re-bind those names to new payloads or to nothing. The authors claim Willow
has "the strongest story for mutability" among comparable protocols
precisely because it completely eschews content-addressing at the protocol
level ([Comparison to Other Protocols](https://willowprotocol.org/more/willow_compared/)).

### The data model (spec status: Final)

- An **Entry** is metadata only: `namespace_id`, `subspace_id`, `path`,
  `timestamp` (u64), `payload_length`, `payload_digest`. The payload is a
  separate bytestring, tracked only via its digest — entries can be
  reconciled before (or without ever) moving payloads.
- A **namespace** is a completely independent data universe. A **subspace**
  is a partition inside it — intuitively one author's universe. A **path**
  is a sequence of bytestring components, filesystem-like (`blog/idea/2`).
- Conflict semantics are filesystem semantics, defined by the **join** of
  two stores: union, then drop every entry that is _prefix-pruned_ (an
  entry whose path is a prefix of another's, in the same subspace, with a
  newer timestamp) or _superseded_ (same subspace + path, older timestamp;
  ties broken by greater payload digest, then greater payload length).
  Stores form a **join semi-lattice — a state-based CRDT** — so joins are
  order-independent and convergence is guaranteed. A store holds **only
  the surviving current entries: no history, no log, no tombstones**.
- Timestamps are _claimed_, not proven. The recommended interpretation is
  microseconds of TAI since J2000, but logical counters are explicitly
  allowed — a deliberate, much-discussed design choice
  ([Timestamps, Really?](https://willowprotocol.org/more/timestamps_really/)).
- The model is **generic ("higher-kinded")**: id types, path limits, hash
  function, and the authorisation mechanism are parameters. **Willow'25**
  (status: Proposal) is the recommended concrete instantiation: Ed25519
  public keys as _both_ NamespaceId and SubspaceId, WILLIAM3 (from the Bab
  hash family) as payload hash, Meadowcap as the authorisation system.

### Grouping: Areas

Entries are points in a 3D space (subspace × path × timestamp). An
**Area** — subspace is one exact id or `any`, path is a _prefix_, time is
a range — is closed under intersection, and is simultaneously **the unit
of authorisation in Meadowcap and of interest in sync**. One algebra
serving authorisation, interest, and reconciliation is the family's
signature design move. AreaOfInterest adds count/size budgets for partial
sync; AreaSlice cuts an Area by the reconciliation order so any set can be
split into roughly equal halves for set reconciliation.

### Spec family inventory and status

| Spec                                | Status                                   | Notes                                                   |
| ----------------------------------- | ---------------------------------------- | ------------------------------------------------------- |
| Data Model                          | **Final**                                | Core semantics above                                    |
| Meadowcap                           | **Final** (21 Nov 2025)                  | Capability system, below                                |
| Grouping Entries                    | Final-grade core doc                     | Areas/AreaSlices vocabulary                             |
| Willow'25                           | Proposal (May 2025)                      | Recommended parameter set                               |
| Confidential Sync (ex-WGPS)         | **Proposal — design paused**             | Being redrafted after the Willow Transfer Protocol work |
| Read Access & Confidentiality (PIO) | Sub-spec of Confidential Sync            | Private interest overlap                                |
| Region-Based Set Reconciliation     | Core technique doc                       | RBSR                                                    |
| Willow Transfer Protocol (WTP)      | Proposal (Oct 2026)                      | Simpler request/stream protocol, current focus          |
| Drop Format (ex-Sideloading)        | Candidate/Proposal                       | Single-bytestring bundles for sneakernet/email/USB      |
| Encrypted Willow                    | Explanatory document, **not a protocol** | What can/cannot be encrypted                            |
| Willow URIs                         | Proposal                                 | `willow://namespace.subspace/path`                      |

There is no standards-body involvement (no IETF/W3C track);
interoperability means "same parameter instantiation", in practice
Willow'25.

## Identity & key management

- **Identity is a bare signature keypair.** In Willow'25, a user _is_ an
  Ed25519 public key, and that key is simultaneously their SubspaceId.
  There is no account, handle, DID, certificate, or key-directory layer.
- In a _communal_ namespace, authority over a subspace stems from holding
  the secret key whose public key _is_ the subspace id. In an _owned_
  namespace, the **NamespaceId is itself a public key** and the namespace
  secret key is the root of all authority.
- Meadowcap is **generic over signature schemes** — nothing in its
  security definitions requires Ed25519. (Relevant to the estate: a
  P-256/P-384 instantiation is spec-legal, though no such instantiation or
  implementation exists today.)
- **No key rotation, no recovery, no multi-device linking, no expiry of
  identities** anywhere in the specs. Lose a communal subspace key and
  that subspace is permanently read-only history; lose an owned namespace
  key and all authority over the namespace is gone. Key custody is
  entirely the application's problem.
- Communal vs owned is decided by a parameter function
  `is_communal(namespace_pk)`; Willow'25 defines it as "least significant
  bit of the public key is 0" — so the type of a namespace is a
  mathematical property of its id, needing no registry.

## Meadowcap in detail

Source: the [Meadowcap specification](https://willowprotocol.org/specs/meadowcap/index.html)
(status: **Final** as of 21 Nov 2025). Validity is defined recursively and
precisely enough to implement from the spec alone; two independent
implementations exist.

### What a capability is

An unforgeable, receiver-bound token answering four questions: _to whom_
(a user public key — the **receiver**), _which access mode_ (`read` or
`write` — the only two), _for which entries_ (an **Area**: subspace, path
prefix, time range), and _is it valid_ (a chain of signatures rooted in
the namespace or subspace owner). A capability is **useless if leaked**:
exercising it requires signing with the receiver's secret key.

### The two roots of authority

**Communal namespaces** (horizontal model). With zero delegations a
capability is a _source capability_: the `user_key` (== subspace id)
owner's authority over their own subspace area. Anyone can create one by
generating a keypair — communal namespaces are permissionlessly joinable,
and _nobody_ has authority over other people's subspaces.

**Owned namespaces** (top-down model). An `initial_authorisation` — a
signature by the **namespace secret key** over the first receiver and
access mode — roots the chain. With zero delegations the granted area is
the _full area_: the owner can mint a capability for any subspace, any
path, any time. This is the model that maps to managed systems (a fleet,
a company share): one root key, all authority flows from it. A wrapper
type checks `is_communal(namespace_key)` so the two models can never be
confused for the same namespace.

### Delegation and attenuation — one mechanism

- A capability carries a list of **delegations**, each a triple
  `(Area, new_receiver_key, signature)`.
- The capability is valid iff, recursively: the prefix capability is
  valid; the **previous granted area includes the new Area** (checked on
  subspace, path-prefix, and time-range simultaneously); and the
  signature is a valid **handover signature by the previous receiver**
  over bytes binding the new area, the new receiver, and the previous
  signature.
- So _every_ delegation step may narrow the area and can never widen it.
  Attenuation is not a policy convention; widening is cryptographically
  unverifiable. Delegating to _yourself_ with a narrower area produces a
  restricted capability.
- **Delegation is always transitive**: there is no "may not re-delegate"
  flag or depth limit in the spec. Depth is bounded only by chain size.
- Verification cost is linear in chain length, verified from the root
  every time.

### How capabilities are used

**Writes are self-certifying.** Willow's data model takes an
`AuthorisationToken` parameter; Meadowcap instantiates it as the
capability plus the receiver's signature over the encoded entry. The
check: capability valid, mode is `write`, granted area includes the
entry, signature valid. The token **travels with the entry forever** —
any peer, offline, at any later time, can verify that this entry was
authorised, with no reference to any server or registry. This is the
strongest part of the design for offline-first systems.

**Reads are cooperative, not self-certifying.** The data model has no
read concept; read enforcement lives in the replication layer: peers must
cooperate in sending entries only to peers who have presented a valid
read capability. Once a peer has legitimately received data, nothing in
Meadowcap controls what they do with it. The confidential presentation of
read capabilities (hiding even _which areas_ you hold rights to) is the
PIO protocol, below.

A companion type, `McEnumerationCapability`, certifies read access to
_arbitrary subspace ids_ within a namespace; all-subspace read
capabilities can only exist in **owned** namespaces.

### Revocation and expiry — the honest weak point

**Meadowcap has no revocation mechanism.** No revocation lists, no epoch
keys, no online check — by design, because capabilities must verify
offline. What the design offers instead:

1. **Time-bounded grants + non-renewal.** A capability's area includes a
   time range, so you grant write access _for entries timestamped within
   a window_. Repeatedly handing out short-span capabilities imitates
   revocation: you simply do not renew. The authors are candid about the
   tension: low revocation latency requires short-lived capabilities,
   but those need continuous renewal by contacting the authority — a poor
   fit for a peer who connects once every three months.
2. **Supersession as effect-revocation.** Because stores keep only the
   newest entry per (subspace, path), a grantor who retains write access
   with _later_ timestamps can overwrite / prefix-prune everything a
   revoked party wrote. Note the subtlety: expiry constrains _claimed
   timestamps_, not wall-clock creation time — an expired capability can
   still produce entries, but only backdated ones, which lose every join
   against newer entries.
3. **Read revocation barely exists.** Peers can stop _serving_ a revoked
   reader, but data already replicated is already read.

Third parties are patching this from outside the spec: the Leaf protocol
(built on Willow) proposes **FROST threshold co-signing** — a capability
whose use requires a co-signature from an online server, which can simply
stop co-signing to revoke — explicitly trading offline usability for
revocability ([Leaf: Capabilities & Identity](https://blog.muni.town/capabilities-and-identity-with-leaf/)).

**Estate translation**: Meadowcap revocation ≈ "short validity windows +
a renewal service + overwrite hygiene". In an infrastructure setting
where seats check in with a broker regularly, the renewal service is
natural and the offline tension mostly evaporates — but it is a system
_you_ build around Meadowcap, not something Meadowcap gives you.

### What Meadowcap is not

- Not an object-capability system: resources (areas) and identities
  (keys) are separate concepts; capabilities are proof tokens bound to a
  receiver key, not unforgeable object references. No live object graph,
  no membranes, no promise pipelining — it authorises _data access_, not
  _computation_.
- No caveats beyond the area: no rate limits, no operation sub-types
  (append-only vs overwrite are the same `write`), no conditional logic.
- No delegation control (no depth caps, no non-transferable grants).

## Group management

There is **no group abstraction** in Meadowcap or Willow: no membership
lists, no roles, no group keys, no join/leave protocol. A communal
namespace behaves like an open group (membership = having a keypair and
writing in your own subspace; no admission, no removal). An owned
namespace behaves like an owner-administered group (membership = holding
a delegation chain from the namespace key; "removing a member" is the
revocation problem above). A "group" sharing a _subspace_ means sharing a
secret key — the thing Meadowcap was created to replace in Earthstar.
Any real group semantics must be modelled by the application as data
inside the namespace plus delegation discipline.

## Transport & NAT traversal

**Willow provides no transport.** This is deliberate and total:
Confidential Sync is specified over "arbitrary reliable, ordered,
byte-oriented channels, whether tcp, quic, or unix pipe"; there is **no
peer discovery, no NAT traversal, no relay, no addressing** anywhere in
the spec family. The Drop Format exists precisely because the authors
consider connection establishment somebody else's problem — drops move
over "USB keys, SD cards, email attachments, instant messages, torrents,
or whatever means users have at hand". WTP likewise runs over any
reliable ordered bidirectional channel. For the estate, Willow _composes
with_ the existing transport story (iroh, tsnet, Cloudflare doors)
rather than competing with it — and "Willow for transport" is a category
error: there is nothing there to adopt.

## Encryption

### What Willow does NOT encrypt

The [Encrypted Willow](https://willowprotocol.org/specs/e2e/) document
opens: "Willow has no built-in mechanisms for encrypting data." It is an
analysis of what _can_ be layered on, not a protocol:

- **Payloads**: encrypt freely at the application layer. Relays can store
  and replicate ciphertext and still compute joins. Authenticity comes
  from authorisation tokens, not the encryption.
- **Timestamps: cannot be encrypted.** Joins require numeric comparison,
  so timestamps are plaintext by necessity — timing metadata leaks to any
  replicating peer.
- **Paths: can be encrypted prefix-preservingly.** Encrypt each component
  with a key derived from the previous component's key via a KDF. Equal
  plaintext prefixes ⇒ equal ciphertext prefixes, so pruning/join
  semantics survive; holding the key for a prefix decrypts exactly its
  extensions — hierarchical decryption capabilities that mirror the path
  structure. This is a genuinely elegant construction — and it is a
  _documented technique_, not an implemented, versioned protocol with
  test vectors.
- **Authorisation tokens: cannot be encrypted** — verifiers who lack
  decryption keys must still verify Meadowcap signatures.
- **Payload key distribution / group encryption**: out of scope entirely.
  No ratchet, no group key agreement, no forward secrecy or
  post-compromise security for stored payloads.

### Sync-session confidentiality (Confidential Sync + PIO)

This is where the family's real cryptographic ambition lives — in a
**Proposal-status, currently-paused** spec:

- **Private interest overlap (PIO)**: before reconciling, two peers
  discover which (namespace, area) interests they _share_ without
  revealing non-shared interests, exchanging salted hashes of "private
  interests" and learning only the overlap — designed to hold even
  against the counterparty.
- **Handshake-bound presentation**: an authenticated Diffie–Hellman
  exchange (Noise XX pattern) in which each peer proves knowledge of the
  secret key corresponding to the _receiver_ of the read capabilities it
  will present; session traffic is encrypted to that receiver, and
  capabilities are transmitted in special **relative encodings that omit
  confidential fields** already implied by the overlap. Being
  Noise-XX-shaped, the session gets forward secrecy from ephemeral DH —
  for the _session_, not for stored data.

Verdict: the _design_ for confidential sync is the most privacy-serious
in the local-first field; the _artefact_ is an unfinished spec whose own
page says design is paused and much of it outdated.

## Sync / replication semantics

- **Model**: state-based CRDT (join semi-lattice). No operation log, no
  causal history, no vector clocks. Convergence is unconditional on order
  of joins; deletion is _traceless by design_ — the authors deliberately
  avoid cryptographic proofs of completeness, allowing traceless data
  removal. If the estate ever needs audit history, Willow's store
  semantics actively work against it (history would live in a separate,
  application-level log).
- **Conflicts**: deterministic last-writer-wins per (subspace, path) with
  digest/length tie-breaks, plus prefix pruning. Concurrent writes to the
  same path in the same subspace silently lose — filesystem semantics,
  not merge semantics. Different subspaces never conflict. Payloads are
  opaque bytes; there is no text-merge story.
- **Trust caveat**: within a subspace, writers are trusted with
  timestamps. A writer can future-date entries to win all future joins
  and prune others' work — simultaneously a moderation feature and an
  attack; Meadowcap time-bounded capabilities are the mitigation.
- **Reconciliation efficiency**: region-based set reconciliation (RBSR).
  Peers fingerprint sets of entries with a commutative set hash, then
  recursively split mismatched AreaSlices and exchange only the differing
  leaves. Cost scales with the _difference_ between stores, not their
  size — the same RBSR family that iroh-docs also uses. Caveats: these
  are asymptotic design claims with no published benchmark suite; and
  storage layout determines whether fingerprints are cheap — an October
  2026 analysis (Amparore) shows no lexicographic ordering of (subspace,
  path, timestamp) makes every Area contiguous in an ordered index, and
  proposes a denormalised index to make every Area one contiguous range.
  Efficient Willow storage is an open research problem, not a solved one.
- **Partial sync is first-class**: intersections of AreasOfInterest,
  count/size budgets, eager-vs-lazy payload thresholds, unsolicited
  forwarding, and resumable payload transfer via Bab _verifiable streams_
  — a partially received payload is verifiably a prefix of the expected
  payload.
- **Offline behaviour**: total. Stores are local; any two stores join
  whenever they meet, by any channel including Drop Format bundles with
  no connection at all.

## Storage fit

The data model defines an abstract Store trait; each store holds exactly
one namespace. **Existing Rust stores are embedded-KV, not SQL**: the
canonical sled-based store, n0's redb-based `willow-store`, and a new
persistent store in the willow25 0.6 alphas built on fjall. **No
SQLite/libsql/Turso store exists** in either lineage. Writing one is
feasible — entries are rows, (subspace, path, timestamp) is an index —
but the hard parts are bespoke: fingerprints must be maintainable
incrementally over arbitrary AreaSlices, and prefix pruning on ingest is
a range delete keyed by path prefix. Nothing about Turso is usable
off-the-shelf: Willow already _is_ the replication layer, so a
Turso-backed store would double-replicate. Payloads are stored separately
from entries everywhere; large payloads are a blob-store concern
(iroh-willow used iroh-blobs).

## Rust maturity

Two distinct Rust lineages — do not confuse them:

**Canonical (worm-blossom)** — moved from GitHub to
[Codeberg](https://codeberg.org/worm-blossom/willow_rs/) in Oct 2025.
Crates: `willow-data-model`, `meadowcap`, `willow25`, `bab_rs`, plus the
`ufotofu` async ecosystem the team builds alongside.

- `willow25` implements the Data Model and Meadowcap; persistent storage
  and Drop Format are landing in the 0.6 alpha line (and by November 2026
  the crate line had advanced further — see the catalogue digest).
  Version state per an independent July 2026 audit: willow25 0.5.0 /
  0.6.0-alpha.3; the audit _requires_ pinning `bab_rs = 0.8.1` because
  **every bab_rs version before 0.7 computed incorrect WILLIAM3 digests**
  and 0.8 "truly fixes" them — digest-correctness churn in the hash layer
  is a real adoption hazard
  ([Riot Willow implementation audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md)).
- **No Rust implementation of Confidential Sync exists** in the canonical
  repo; the TypeScript `willow-js` implements the data model, WGPS, and
  sideloading.
- The canonical repo has **no hosted CI** (open issue, noted in the same
  audit). The codebase is small-team academic Rust: heavy generics, its
  own async philosophy ("frugal async" + ufotofu rather than Tokio
  idioms). Embedding it in a Tokio fleet binary is possible — the
  data-model and meadowcap crates are computational — but it is not a
  drop-in Tokio-native library.
- License: Apache-2.0. Downloads are tiny (lib.rs shows
  `willow-data-model` at ~410 downloads/month) — a research-grade
  ecosystem, not a depended-upon one.

**TypeScript**: `willow-js` + `meadowcap-js` are the most complete
implementations (data model + Meadowcap + WGPS + sideloading), and
Earthstar 11 (beta) is built on them. A Go implementation (`willow-go`)
exists as an independent interoperability cross-check.

## Cloudflare fit

No Cloudflare-specific support exists anywhere in the family: no Workers
examples, no KV/R2/D1 bindings, no Durable Objects store. What could
work, honestly assessed: _Meadowcap verification as a pure function_ is
portable (the Rust crate is generic computation; the TS implementation
runs anywhere JS runs, including Workers) — capability _checking_ at the
edge is plausible today. _A Willow store on Workers_ is not: the Rust
stores need a filesystem, and a Durable Object hosting a store + sync
endpoint would be a from-scratch build. _Drops via R2/KV_ is the one
naturally Cloudflare-shaped pattern (a drop is a single verifiable
bytestring; R2 is an ideal drop mailbox), but tooling for that pattern
does not exist. Verdict: components are portable to the edge; the system
is not available there.

## iroh fit

- **Conceptual fit is excellent.** Willow'25 identities _are_ Ed25519
  public keys — the same key type as an iroh NodeId; a subspace key can
  literally be an endpoint key. Confidential Sync needs only reliable
  ordered streams, which is exactly what iroh provides over QUIC, plus
  the NAT traversal/discovery Willow lacks. Payloads map naturally to
  iroh-blobs.
- **An implementation exists**: n0-computer/iroh-willow — a Rust
  implementation of Willow + Meadowcap + (its era's) Confidential Sync
  over iroh, using a redb store for entries and iroh-blobs for payloads.
  n0 built it while exploring Willow as the successor direction to
  iroh-docs.
- **But it is not a maintained product.** docs.rs carries the crate
  (v0.28.0) with the note "This work is not released yet"; it is built
  against iroh 0.34-era APIs with patch overrides to n0 forks — several
  iroh generations behind current iroh. Adopting willow-over-iroh today
  means _forking and porting_ iroh-willow, not depending on it.
- **Hash-world collision**: Willow'25 payload digests are WILLIAM3/Bab;
  iroh-blobs is BLAKE3/bao. iroh-willow resolved this by instantiating
  Willow with BLAKE3 digests — i.e. it is _a_ Willow, but not Willow'25,
  and would not interoperate with willow25 peers. Any estate adoption
  must pick which hash world its payloads live in.

## Project health

Team: two people (Meyer, Gwilym) as worm-blossom, plus occasional
contributors (a third member joined in 2026 — see the catalogue digest).
Bus factor is the obvious risk. Funding is grant-based throughout — NLnet
NGI Assure (2022), then a €50K NGI0 Core grant, then a dedicated NLnet
grant for Bab. Donation income is ~$3K/year. There is no company, no
hosted service, no commercial support. Activity: alive and publishing —
a weekly dev diary, spec updates through Nov 2025 (Meadowcap → Final),
2026 commits on Codeberg, and external research uptake. The pace is
deliberate and specification-first: the team optimises for the spec being
_right_, not for shipping product surface. If the estate adopts Willow,
it is adopting a spec and a two-to-three-person upstream, and should
expect to carry implementation weight itself.

## Operational complexity

- **Instantiation burden**: raw Willow is a blueprint; Willow'25 removes
  most choices, but you still own key custody, renewal services, and
  store operations.
- **Timestamp discipline becomes security-critical**: claimed timestamps
  drive both conflict resolution and capability windows.
- **Capability chains are per-entry overhead**: every authorised entry
  carries its full delegation chain + signature, forever.
- **Store operations are immature**: no admin tooling, no metrics; the
  efficient-storage question is still being researched.
- **Sync operations don't exist yet** in Rust: any production sync is
  your code, your bugs.

## What Willow / Meadowcap pointedly does NOT do

1. **No revocation** — only non-renewal of time-bounded grants and
   supersession of writes.
2. **No groups, roles, or membership management.**
3. **No transport, discovery, NAT traversal, or relays.**
4. **No built-in end-to-end encryption** — no payload key distribution,
   no group encryption, no forward secrecy for stored data.
5. **No history or auditability** — stores keep only current state, by
   design (traceless deletion is a feature).
6. **No Byzantine tolerance inside a subspace** — authorised writers are
   trusted, including with timestamps.
7. **No consensus / global ordering / cross-namespace transactions.**
8. **No SQL store, no Cloudflare story, no production Rust sync.**
9. **No identity recovery/rotation story.**
10. **No rich authorisation logic** — read/write over areas is all there
    is; no append-only mode, no quotas, no conditional caveats.

## Judgment

**(a) As a full stack: not ready.** Willow is two finished, excellent
layers (data model + Meadowcap) wearing an unfinished third layer
(network sync). The data model's bet — mutable, named, deletable data
with filesystem semantics, replicated as a state-based CRDT with
first-class partial sync — fits infrastructure state (configs,
registries, seat state) arguably better than append-only log models, and
its reconciliation design is principled and difference-proportional. But
adopting "Willow for everything" in Rust today means alpha crates with a
recent hash-correctness incident in their history, an embedded-KV store
instead of the estate's SQLite/Turso, _no_ Rust sync implementation, a
sync spec paused mid-rewrite, your own transport integration, your own
revocation/renewal service, and a two-person grant-funded upstream. The
estate would become the de-facto third implementation team. That is a
research adoption, not a platform adoption.

**(b) As a borrowed capability layer: the export-grade asset.** Meadowcap
was _designed_ to be borrowed: the data model treats authorisation as a
parameter, and Meadowcap itself is generic over signature schemes and
needs only an area-like grouping (subspace × path-prefix × time-range)
over whatever it protects. Borrowed into another stack (p2panda logs, a
fleet broker, a sync protocol of our own), it buys receiver-bound,
offline-verifiable, self-certifying authorisation tokens that travel with
the data; attenuation that is cryptographically monotone; the cleanest
available answer to "who may write/read _this slice_ of the data space",
including the owned/communal duality that maps directly onto "managed
fleet" vs "open federation"; and a Final spec with two independent
implementations to check against. Its costs travel with it: no revocation
(budget for a renewal service issuing short-window capabilities — cheap
for the fleet, whose seats already check in with a broker), always-on
transitive delegation, read enforcement that requires cooperating
servers, and per-entry chain overhead. A borrower should also decide
early whether to take the `meadowcap` crate or re-implement the (compact)
validity rules against the spec — the spec is genuinely implementable,
which is itself the strongest thing that can be said about a capability
design.

**Bottom line**: Willow/Meadowcap is the field's best-specified answer to
_area-scoped capability authorisation for replicated data_, and its data
model is a serious contender wherever mutable named state matters. As a
turnkey stack for this estate it is not ready. The composed play —
another sync/transport core, Meadowcap (spec or crate) as the
authorisation layer, short-window renewal run by the fleet broker —
captures most of the value at a fraction of the adoption risk.
