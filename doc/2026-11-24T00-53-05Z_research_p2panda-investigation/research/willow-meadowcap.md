# Willow / Meadowcap — research note

Family lane for the auth + transport investigation (tracking issue developing-today/code#13779).
Researched 2026-11-23 from primary sources: the specifications at
[willowprotocol.org](https://willowprotocol.org/), the canonical Rust repo
([Codeberg: worm-blossom/willow_rs](https://codeberg.org/worm-blossom/willow_rs/)),
the TypeScript implementations, the n0 iroh experiment, and crates/docs.rs listings.
Every load-bearing claim is linked inline.

## 0. What this family is

Willow is a **family of specifications** (not one protocol) for synchronisable,
multi-writer, peer-to-peer data stores, designed by Aljoscha Meyer and Sam Gwilym
(worm-blossom), generalising and formalising the Earthstar data model
([About Us](https://willowprotocol.org/more/about-us/);
[LWN: Offline applications with Earthstar](https://lwn.net/Articles/1005639/)).
Earthstar (TypeScript, since 2020) is being rebuilt on Willow as Earthstar 11;
Willow is the reusable protocol layer underneath it
([Earthstar/Willow spring update](https://opencollective.com/worm-blossom/updates/willow-and-earthstars-sping)).

The core idea is **naming, not content-addressing, at the protocol level**:
you bind arbitrary payloads to meaningful hierarchical names, and later re-bind
those names to new payloads or to nothing. The authors claim Willow has "the
strongest story for mutability" among comparable protocols precisely because it
"completely eschew[s] content-addressing on the protocol level"
([Comparison to Other Protocols](https://willowprotocol.org/more/willow_compared/)).

### The data model (spec status: Final)

Source: [Willow Data Model](https://willowprotocol.org/specs/data-model/index.html).

- An **Entry** is metadata only: `namespace_id`, `subspace_id`, `path`,
  `timestamp` (u64), `payload_length`, `payload_digest`. The payload itself is a
  separate bytestring, tracked only via its digest — entries can be reconciled
  before (or without ever) moving payloads.
- A **namespace** is a completely independent data universe. A **subspace** is a
  partition inside it — intuitively one author's universe. A **path** is a
  sequence of bytestring components, filesystem-like (`blog/idea/2`).
- Conflict semantics are filesystem semantics, defined by the **join** of two
  stores (same namespace): union, then drop every entry that is *prefix-pruned*
  (an entry whose path is a prefix of another's, in the same subspace, with a
  newer timestamp) or *superseded* (same subspace + path, older timestamp;
  ties broken by greater payload digest, then greater payload length).
  Stores form a **join semi-lattice — a state-based CRDT** — so joins are
  order-independent and convergence is guaranteed. A store holds **only the
  surviving current entries: no history, no log, no tombstones**.
- Timestamps are *claimed*, not proven. The recommended interpretation is
  microseconds of TAI since J2000, but logical counters are explicitly allowed.
  This is a deliberate, much-discussed design choice
  ([Timestamps, Really?](https://willowprotocol.org/more/timestamps_really/)).
- The model is **generic ("higher-kinded")**: namespace/subspace id types, path
  limits, hash function, and the authorisation mechanism are parameters.
  **Willow'25** ([spec, status: Proposal](https://willowprotocol.org/specs/willow25/index.html))
  is the recommended concrete instantiation: Ed25519 public keys as *both*
  NamespaceId and SubspaceId, WILLIAM3 (from the Bab hash family) as payload
  hash, path limits of 4096/4096/4096, Meadowcap as the authorisation system.

### Grouping: 3dRanges, Areas, Areas of Interest, AreaSlices

Source: [Grouping Entries](https://willowprotocol.org/specs/grouping-entries/).

Entries are points in a 3D space (subspace × path × timestamp). Groupings:

- **3dRange**: independent ranges on all three axes (most expressive; breaks
  under path encryption because ciphertext order ≠ plaintext order).
- **Area**: the restricted, encryption-friendly grouping — subspace is one
  exact id or `any`, path is a *prefix*, time is a range. Areas are closed
  under intersection. **Areas are the unit of authorisation in Meadowcap and of
  interest in sync.**
- **AreaOfInterest**: an Area plus `max_count` / `max_size` budgets (only the
  newest N entries / M payload bytes) — partial-sync resource control.
- **AreaSlice**: an Area cut by a range over the *reconciliation order*
  (timestamp, then path, then subspace) so any set can be split into roughly
  equal halves — the unit of set reconciliation.

### Spec family inventory and status

From the spec pages themselves and the
[site changelog](https://willowprotocol.org/more/changes/):

| Spec | Status | Notes |
|---|---|---|
| Data Model | **Final** | Core semantics above |
| Meadowcap | **Final** (21 Nov 2025) | Capability system, §2 below |
| Grouping Entries | Final-grade core doc | Areas/AreaSlices vocabulary |
| Willow'25 | Proposal (May 2025) | Recommended parameter set |
| Confidential Sync (ex-WGPS) | **Proposal — design paused** | Being redrafted after the Willow Transport Protocol work; spec page says so explicitly ([Confidential Sync](https://willowprotocol.org/specs/confidential-sync/index.html)) |
| Read Access & Confidentiality (PIO) | Sub-spec of Confidential Sync | Private interest overlap, §5 |
| Region-Based Set Reconciliation | Core technique doc | [RBSR](https://willowprotocol.org/specs/rbsr/) |
| Willow Transfer Protocol (WTP) | Proposal (Oct 2026) | Simpler request/stream protocol, current focus ([WTP](https://willowprotocol.org/specs/wtp/)) |
| Drop Format (ex-Sideloading) | Candidate per changelog; page header reads Proposal | Single-bytestring bundles for sneakernet/email/USB ([Drop Format](https://willowprotocol.org/specs/drop-format/)) |
| Encrypted Willow | Explanatory document, **not a protocol** | What can/cannot be encrypted, §5 |
| Willow URIs | Proposal | `willow://namespace.subspace/path` |
| Encodings | Proposal (Meadowcap encodings: Candidate) | Canonical + relative/private encodings |

There is no standards-body involvement (no IETF/W3C track); interoperability
means "same parameter instantiation", in practice Willow'25.

---

## 1. Identity & key management

- **Identity is a bare signature keypair.** In Willow'25, a user *is* an
  Ed25519 public key, and that key is simultaneously their SubspaceId
  ([Willow'25](https://willowprotocol.org/specs/willow25/index.html)). There is
  no account, handle, DID, certificate, or key-directory layer in the family.
- **What a "subspace key" is**: in a *communal* namespace, authority over a
  subspace stems from holding the secret key whose public key *is* the
  subspace id — you prove authorship by signing entries with it. In an *owned*
  namespace, the **NamespaceId is itself a public key** and the namespace
  secret key is the root of all authority
  ([Meadowcap overview](https://willowprotocol.org/specs/meadowcap/index.html)).
- Meadowcap is **generic over signature schemes** — namespace and user schemes
  are separate parameters, and any scheme with the usual
  generate/sign/verify shape can be instantiated. Willow'25 pins Ed25519 for
  both, but nothing in Meadowcap's security definitions requires Ed25519.
  (Relevant to the estate: a P-256/P-384 instantiation is spec-legal, though no
  such instantiation or implementation exists today.)
- **No key rotation, no recovery, no multi-device linking, no expiry of
  identities** anywhere in the specs. Lose a communal subspace key and that
  subspace is permanently read-only history; lose an owned namespace key and
  all authority over the namespace is gone. Key custody is entirely the
  application's problem.
- Communal vs owned is decided by a parameter function `is_communal(namespace_pk)`.
  Willow'25 defines it as "least significant bit of the public key is 0" — so
  roughly half of randomly generated namespace keys are communal and half are
  owned, and you grind keys until you get the type you want. Elegant,
  stateless, and a good example of the family's taste: the type of a namespace
  is a mathematical property of its id, needing no registry.

## 2. Capability model — Meadowcap in detail

Source: [Meadowcap specification](https://willowprotocol.org/specs/meadowcap/index.html)
(status: **Final** as of 21 Nov 2025). This is one of the most completely
specified capability designs in the local-first field: validity is defined
recursively and precisely enough to implement from the spec alone (two
independent implementations exist, §8).

### 2.1 What a capability is

An unforgeable, receiver-bound token answering four questions: *to whom*
(a user public key — the **receiver**), *which access mode* (`read` or
`write` — the only two), *for which entries* (an **Area**: subspace,
path prefix, time range), and *is it valid* (a chain of signatures rooted in
the namespace or subspace owner). A capability is **useless if leaked**:
exercising it requires signing with the receiver's secret key.

### 2.2 The two roots of authority

**Communal namespaces** (horizontal model). `CommunalCapability` =
`{ access_mode, namespace_key, user_key, delegations[] }`. With zero
delegations it is a *source capability*: the `user_key` (== subspace id)
owner's authority over their own subspace area. Anyone can create one by
generating a keypair — communal namespaces are permissionlessly joinable,
and *nobody*, not even the namespace key's holder, has authority over other
people's subspaces.

**Owned namespaces** (top-down model). `OwnedCapability` =
`{ access_mode, namespace_key, user_key, initial_authorisation, delegations[] }`.
The `initial_authorisation` is a signature by the **namespace secret key**
over the first receiver and access mode. With zero delegations the granted
area is the *full area* — the owner can mint a capability for any subspace,
any path, any time. This is the model that maps to managed systems (a fleet,
a company share): one root key, all authority flows from it.

`McCapability` wraps either variant, and validity additionally checks
`is_communal(namespace_key)` so the two models can never be confused for the
same namespace — presenting an owned capability in a communal namespace (or
vice versa) is invalid by construction.

### 2.3 Delegation and attenuation — one mechanism

There is no separate "restriction token" in the final spec (the 2023
announcement's source/delegation/restriction/merge taxonomy
([Open Collective](https://opencollective.com/worm-blossom/updates/meadowcap))
was folded down). Instead:

- A capability carries a list of **delegations**, each a triple
  `(Area, new_receiver_key, signature)`.
- The capability is valid iff, recursively: the prefix capability is valid;
  the **previous granted area includes the new Area** (area inclusion is
  checked on subspace, path-prefix, and time-range simultaneously); and the
  signature is a valid **handover signature by the previous receiver** over
  bytes binding the new area, the new receiver, and the previous signature.
- So *every* delegation step may narrow the area — by subspace, by path
  prefix, and/or by time range — and can never widen it. Attenuation is not a
  policy convention; widening is cryptographically unverifiable.
  Delegating to *yourself* with a narrower area produces a restricted
  capability. The final delegation's area is the capability's granted area;
  the final receiver is its holder.
- **Delegation is always transitive**: there is no "may not re-delegate"
  flag or depth limit in the spec. Any receiver can delegate further, within
  their area. Depth is bounded only by chain size (each step adds an area, a
  key, and a signature that travel with the token).
- Verification cost is linear in chain length, and chains are verified from
  the root every time (no caching semantics specified).

### 2.4 How capabilities are used

**Writes are self-certifying.** Willow's data model takes an
`AuthorisationToken` parameter; Meadowcap instantiates it as
`MeadowcapAuthorisationToken { capability, signature }`, where the signature
is the capability receiver's signature over the encoded entry.
`meadowcap_is_authorised_write` checks: capability valid, mode is `write`,
granted area includes the entry, signature valid. The token **travels with
the entry forever** — any peer, offline, at any later time, can verify that
this entry was authorised, with no reference to any server or registry.
This is the strongest part of the design for offline-first systems.

**Reads are cooperative, not self-certifying.** The data model has no read
concept; read enforcement lives in the replication layer: "all peers must
cooperate in sending Entries only to peers who have presented a valid read
capability". A read capability is simply an `McCapability` with mode `read`.
Once a peer has legitimately received data, nothing in Meadowcap controls
what they do with it. The confidential presentation of read capabilities
(hiding even *which areas* you hold rights to) is the PIO protocol, §5.

**Enumeration capabilities.** A companion type, `McEnumerationCapability`,
certifies read access to *arbitrary subspace ids* within a namespace (at an
unspecified path). It exists to resolve "awkward pairs" in private interest
overlap, where two peers' interests overlap only across different subspaces.
Notably, all-subspace read capabilities can only exist in **owned**
namespaces
([Read Access and Confidentiality](https://willowprotocol.org/specs/pio/index.html)).

### 2.5 Revocation and expiry — the honest weak point

**Meadowcap has no revocation mechanism.** No revocation lists, no epoch
keys, no online check — by design, because capabilities must verify offline.
What the design offers instead, in the authors' own reasoning
([Timestamps, Really?](https://willowprotocol.org/more/timestamps_really/)):

1. **Time-bounded grants + non-renewal.** A capability's area includes a
   time range, so you grant write access *for entries timestamped within a
   window*. "Repeatedly handing out capabilities that grant access for a
   small time span imitates the concept of revoking rights…: you simply do
   not renew someone's capability." The essay is candid about the tension:
   "Low latency in access revocation requires short-lived capabilities, but
   those need to be continuously renewed by contacting the authority… If I
   can only connect to the Internet once every three months, I need
   long-lived capabilities."
2. **Supersession as effect-revocation.** Because stores keep only the
   newest entry per (subspace, path), a grantor who retains (or is granted)
   write access with *later* timestamps can overwrite / prefix-prune
   everything a revoked party wrote. The authors' recommended hygiene for
   expiring credentials: when a credential expires, write entries timestamped
   just after expiry so backdated writes with the expired credential are
   discarded in favour of the newer entries
   ([LWN summary](https://lwn.net/Articles/1005639/)). Note the subtlety this
   exposes: expiry constrains *claimed timestamps*, not wall-clock creation
   time — an expired capability can still produce entries, but only
   backdated ones, which lose every join against newer entries.
3. **Read revocation barely exists.** Peers can stop *serving* a revoked
   reader, but data already replicated is already read. Nothing can undo it.

Third parties are patching this from outside the spec: the Leaf protocol
(built on Willow) proposes **FROST threshold co-signing** — a capability
whose use requires a co-signature from an online server, which can simply
stop co-signing to revoke — explicitly trading offline usability for
revocability
([Leaf: Capabilities & Identity](https://blog.muni.town/capabilities-and-identity-with-leaf/)).

**Estate translation**: Meadowcap revocation ≈ "short validity windows +
a renewal service + overwrite hygiene". In an infrastructure setting where
seats check in with a broker regularly, the renewal service is natural and
the offline tension mostly evaporates — but it is a system *you* build
around Meadowcap, not something Meadowcap gives you.

### 2.6 What Meadowcap is not

- Not an object-capability system: resources (areas) and identities (keys)
  are separate concepts; capabilities are bearer-of-proof tokens bound to a
  receiver key, not unforgeable object references (a distinction the Leaf
  authors also note). There is no live object graph, no membranes, no
  promise pipelining — it authorises *data access*, not *computation*.
- No caveats beyond the area: no rate limits, no operation sub-types
  (append-only vs overwrite are the same `write`), no conditional logic.
- No delegation control (no depth caps, no non-transferable grants).

## 3. Group management

There is **no group abstraction** in Meadowcap or Willow: no membership
lists, no roles, no group keys, no join/leave protocol. What exists:

- A **communal namespace** behaves like an open group: membership = having a
  keypair and writing in your own subspace; there is no admission and no
  removal.
- An **owned namespace** behaves like an owner-administered group: membership
  = holding a delegation chain from the namespace key. "Removing a member"
  is the revocation problem of §2.5.
- A "group" sharing a *subspace* means sharing a secret key (all authority,
  no accountability separation) — the thing Meadowcap was created to replace
  in Earthstar, where knowing a share's secret key gave irrevocable write
  access to anyone who learned it
  ([Meadowcap announcement](https://opencollective.com/worm-blossom/updates/meadowcap)).

Any real group semantics (rosters, roles, admins, removal) must be modelled
by the application as data inside the namespace plus delegation discipline.

## 4. Transport & NAT traversal

**Willow provides no transport.** This is deliberate and total:

- Confidential Sync is specified over "arbitrary reliable, ordered,
  byte-oriented channels, whether tcp, quic, or unix pipe"
  ([Confidential Sync](https://willowprotocol.org/specs/confidential-sync/index.html)).
- There is **no peer discovery, no NAT traversal, no relay, no addressing**
  anywhere in the spec family. The Drop Format exists precisely because the
  authors consider connection establishment somebody else's problem — drops
  move over "USB keys, SD cards, email attachments, instant messages,
  torrents, or whatever means users have at hand"
  ([Drop Format](https://willowprotocol.org/specs/drop-format/)).
- WTP (the current sync focus) likewise runs over any reliable ordered
  bidirectional channel and adds a handshake, but no discovery/NAT layer
  ([WTP](https://willowprotocol.org/specs/wtp/)).

For the estate this means Willow *composes with* the existing transport
story (iroh, tsnet, Cloudflare doors) rather than competing with it — and
also that "Willow for transport" is a category error: there is nothing
there to adopt.

## 5. Encryption

### 5.1 What Willow does NOT encrypt

The [Encrypted Willow](https://willowprotocol.org/specs/e2e/) document opens:
"Willow has no built-in mechanisms for encrypting data." It is an analysis of
what *can* be layered on, not a protocol:

- **Payloads**: encrypt freely at the application layer (symmetric; add a
  nonce, pad to hide length). Relays can store/replicate ciphertext and still
  compute joins. Authenticity comes from authorisation tokens, not the
  encryption.
- **Timestamps: cannot be encrypted.** Joins require numeric comparison, so
  timestamps are plaintext by necessity — timing metadata leaks to any
  replicating peer. Mitigations are social/operational: coarsen resolution,
  or use logical counters per subspace.
- **Namespace/Subspace ids: not worth encrypting** (equality must be
  preserved for joins; per-namespace encryption changes no observer
  inferences). Hash meaningful identifiers before using them as ids instead.
- **Paths: can be encrypted prefix-preservingly.** Encrypt each component
  with a key derived from the previous component's key via a KDF (HKDF is
  suggested). Equal plaintext prefixes ⇒ equal ciphertext prefixes, so
  pruning/join semantics survive; holding the key for a prefix decrypts
  exactly its extensions — *hierarchical decryption capabilities that mirror
  the path structure*. The payload can be encrypted under the final
  component's key, tying payload access to path access. This is a genuinely
  elegant construction — and it is a *documented technique*, not an
  implemented, versioned protocol with test vectors.
- **Authorisation tokens: cannot be encrypted** — verifiers who lack
  decryption keys must still verify Meadowcap signatures.
- **At rest**: nothing specified; store encryption is the implementer's
  problem.
- **Payload key distribution / group encryption**: out of scope entirely.
  There is no ratchet, no group key agreement, no forward secrecy or
  post-compromise security for stored payloads. Whoever holds a path key
  reads past and future payloads under it until keys are manually rotated
  (and rotation invalidates ciphertext addressing).

### 5.2 Sync-session confidentiality (Confidential Sync + PIO)

This is where the family's real cryptographic ambition lives — in a
**Proposal-status, currently-paused** spec:

- **Private interest overlap (PIO)**: before reconciling, two peers must
  discover which (namespace, area) interests they *share* without revealing
  non-shared interests. They exchange *salted hashes* of "private interests"
  (read capability + area of interest pairs), prove read access, and learn
  only the overlap — designed to hold "even in realistic adversarial
  settings", including against the counterparty
  ([Read Access and Confidentiality](https://willowprotocol.org/specs/pio/index.html)).
- **Handshake-bound presentation**: the sync handshake is an authenticated
  Diffie–Hellman exchange (Noise XX pattern fulfils the stated requirements)
  in which each peer proves knowledge of the secret key corresponding to
  the *receiver* of the read capabilities it will present; session traffic is
  encrypted to that receiver, and capabilities are transmitted in special
  **relative encodings that omit confidential fields** (namespace, paths)
  already implied by the overlap. Willow'25 instantiates this as
  `Nose_XX_ED25519_ChaChaPoly_WILLIAM3` (sic — the spec's protocol name
  literally spells "Nose"), AEAD ChaCha20-Poly1305, 12-byte nonces, Ed25519
  scalar multiplication as DH, WILLIAM3 as handshake hash
  ([Willow'25 §7](https://willowprotocol.org/specs/willow25/index.html)).
  Being Noise-XX-shaped, the session gets forward secrecy from ephemeral DH
  — for the *session*, not for stored data.
- **Man-in-the-middle resistance** was a major work item in the Confidential
  Sync rewrite, via binding read access into the handshake
  ([site changelog](https://willowprotocol.org/more/changes/)).

Verdict: the *design* for confidential sync is the most privacy-serious in
the local-first field; the *artefact* is an unfinished spec whose own page
says design is paused and much of it outdated.

## 6. Sync / replication semantics

- **Model**: state-based CRDT (join semi-lattice, §0). No operation log, no
  causal history, no vector clocks. Convergence is unconditional on order of
  joins; deletion is *traceless by design* — the authors "deliberately avoid
  any cryptographic proofs of completeness, allowing for traceless data
  removal" ([Comparison](https://willowprotocol.org/more/willow_compared/)).
  If the estate ever needs audit history, Willow's store semantics actively
  work against it (history would live in a separate, application-level log).
- **Conflicts**: deterministic last-writer-wins per (subspace, path) with
  digest/length tie-breaks, plus prefix pruning. Concurrent writes to the
  same path in the same subspace silently lose — filesystem semantics, not
  merge semantics. Different subspaces never conflict. There is no
  text-merge/CRDT-payload story; payloads are opaque bytes.
- **Trust caveat**: within a subspace, writers are trusted with timestamps.
  A writer can future-date entries to win all future joins and prune others'
  work — simultaneously a moderation feature and an attack; Meadowcap
  time-bounded capabilities are the mitigation.
- **Reconciliation efficiency**: region-based set reconciliation
  ([RBSR](https://willowprotocol.org/specs/rbsr/)). Peers fingerprint sets of
  *LengthyAuthorisedEntries* (entry + how many payload-prefix bytes are held)
  with a set hash whose combine operation is associative and commutative
  (Willow'25: sum of per-entry WILLIAM3 hashes mod 2²⁵⁶), then recursively
  split mismatched AreaSlices into balanced halves and exchange only the
  differing leaves. The spec's claim: peers "drill down to the differences…
  in a logarithmic number of communication rounds, spending only little
  bandwidth on those regions… where they hold the same items." Cost scales
  with the *difference* between stores, not their size — the same RBSR family
  (Meyer 2023) that iroh-docs also uses. Caveats: (a) these are asymptotic
  design claims; the Willow team publishes no benchmark suite; (b) storage
  layout determines whether fingerprints are cheap — an October 2026
  analysis by Amparore shows *no* lexicographic ordering of
  (subspace, path, timestamp) makes every Area contiguous in an ordered
  index, that current implementations pay for this with multi-index or
  scan-and-filter strategies, and proposes a denormalised index
  (AreaScopeIndex, 2(d+1) index entries per entry at depth d) to make every
  Area one contiguous range
  ([Making Every Willow Area Contiguous](https://worm-blossom.org/assets/papers/amparore2026making.pdf)).
  Efficient Willow storage is an open research problem, not a solved one.
- **Partial sync is first-class**: intersections of AreasOfInterest, count/
  size budgets, eager-vs-lazy payload thresholds (expressive enough to
  implement Plumtree-style dissemination), unsolicited forwarding within an
  open session, and resumable payload transfer via Bab *verifiable streams* —
  a partially received payload is verifiably a prefix of the expected
  payload, because the payload digest is a Merkle-style Bab hash.
- **Offline behaviour**: total. Stores are local; any two stores join
  whenever they meet, by any channel including Drop Format bundles with no
  connection at all. Stale data in drops is harmless (joins discard it).

## 7. Storage fit (SQLite / Turso)

- The data model defines an abstract **Store trait**; each store holds
  exactly one namespace. Implementations are responsible for 3D range
  queries, area queries, fingerprint computation over ranges, and payload
  prefix storage.
- **Existing Rust stores are embedded-KV, not SQL**: the canonical
  `willow-store-simple-sled` (sled; separate trees for entries, payloads,
  misc; keys encode subspace‖path‖timestamp for prefix scans); n0's
  `willow-store` on **redb**; and a new persistent store in the willow25
  0.6 alphas built on **fjall** (per the Riot audit, the umbrella crate pulls
  storage + fjall + async-filesystem dependencies even if you only want the
  data model).
- **No SQLite/libsql/Turso store exists** in either lineage. Writing one is
  feasible — entries are rows, (subspace, path, timestamp) is an index — but
  the two hard parts are bespoke: (1) fingerprints must be maintainable
  incrementally over arbitrary AreaSlices (a commutative set hash helps:
  it's a SUM over the slice, which SQL can compute but not cache for free);
  (2) prefix pruning on ingest is a range delete keyed by path prefix.
  Nothing about Turso (distributed SQLite) is used or usable off-the-shelf:
  Willow already *is* the replication layer, so a Turso-backed store would
  double-replicate.
- Payloads are stored separately from entries everywhere; large payloads are
  a blob-store concern (iroh-willow used iroh-blobs), not a Willow concern.

## 8. Rust maturity

Two distinct Rust lineages — do not confuse them:

**Canonical (worm-blossom)** — moved from GitHub (`earthstar-project/willow-rs`,
now archived) to [Codeberg](https://codeberg.org/worm-blossom/willow_rs/) in
Oct 2025. Crates: `willow-data-model`, `meadowcap`, `willow25`, `bab_rs`,
plus the `ufotofu` async ecosystem the team builds alongside.

- `willow25` "currently implements the Data Model and Meadowcap"; persistent
  storage and Drop Format are landing in the 0.6 alpha line
  ([Willow — Rust](https://willowprotocol.org/rust/)). Version state per an
  independent July 2026 audit: willow25 0.5.0 / 0.6.0-alpha.3; the audit
  *requires* pinning `bab_rs = 0.8.1` because **every bab_rs version before
  0.7 computed incorrect WILLIAM3 digests** and 0.8 "truly fixes" them —
  digest-correctness churn in the hash layer is a real adoption hazard
  ([Riot Willow implementation audit](https://github.com/rabble/riot/blob/HEAD/docs/research/2026-07-10-willow-implementation-audit.md)).
- **No Rust implementation of Confidential Sync / WGPS exists** in the
  canonical repo (still listed as future work); the TypeScript `willow-js`
  implements the data model, WGPS, and sideloading.
- The canonical repo has **no hosted CI** (open issue, noted in the same
  audit). The codebase is small-team academic Rust: heavy generics
  (const-generic path limits, signature-scheme type parameters), its own
  async philosophy ("frugal async" + ufotofu producers/consumers rather than
  Tokio idioms). Embedding it in a Tokio fleet binary is possible — the
  data-model and meadowcap crates are computational — but it is not a
  drop-in Tokio-native library.
- License: Apache-2.0 (repo listing). Downloads are tiny (lib.rs shows
  `willow-data-model` at ~410 downloads/month, 3 reverse dependencies) —
  this is a research-grade ecosystem, not a depended-upon one.

**n0 / iroh lineage** — see §10.

**TypeScript**: `willow-js` + `meadowcap-js` (published as
[@earthstar/meadowcap on JSR](https://jsr.io/@earthstar/meadowcap)) are the
most complete implementations (data model + Meadowcap + WGPS + sideloading),
and Earthstar 11 (beta) is built on them. A Go implementation (`willow-go`)
exists as an independent interoperability cross-check.

## 9. Cloudflare fit

- **No Cloudflare-specific support exists anywhere in the family**: no
  Workers examples, no KV/R2/D1 bindings, no Durable Objects store.
- What could work, honestly assessed:
  - *Meadowcap verification as a pure function* is portable: the Rust
    `meadowcap` crate is generic computation and the TS `meadowcap-js` runs
    anywhere JS runs, including Workers. Capability *checking* at the edge
    is plausible today.
  - *A Willow store on Workers* is not: the Rust stores need a filesystem
    (sled/fjall/redb); WASM storage modularity was still an open issue in
    willow_rs (issue #56 per the Riot audit). A Durable Object hosting a
    store + sync endpoint would be a from-scratch build on D1/SQLite with
    the §7 caveats, syncing via the TS WGPS or a hand-rolled WTP.
  - *Drops via R2/KV* is the one naturally Cloudflare-shaped pattern: a drop
    is a single verifiable bytestring; R2 is an ideal drop mailbox. But
    tooling for that pattern does not exist; you'd build it.
- Verdict: components are portable to the edge; the system is not available
  there. Nothing about Willow *needs* Cloudflare, and nothing about
  Cloudflare helps Willow except as dumb transport/storage for drops.

## 10. iroh fit

This is the family's most interesting composition story — and a cautionary
one.

- **Conceptual fit is excellent.** Willow'25 identities *are* Ed25519 public
  keys — the same key type as an iroh NodeId; a subspace key can literally
  be an endpoint key. Confidential Sync needs only reliable ordered streams,
  which is exactly what iroh provides over QUIC, plus the NAT
  traversal/discovery Willow lacks. Payloads map naturally to iroh-blobs.
- **An implementation exists**: [n0-computer/iroh-willow](https://github.com/n0-computer/iroh-willow)
  — a Rust implementation of Willow + Meadowcap + (its era's) Confidential
  Sync over iroh, using `willow-store` (redb) for entries and `iroh-blobs`
  (bao-tree verified streaming) for payloads, sessions over QUIC via
  quic-rpc ([DeepWiki architecture summary](https://deepwiki.com/n0-computer/iroh-willow)).
  n0 built it while exploring Willow as the successor direction to iroh-docs.
- **But it is not a maintained product.** docs.rs carries the crate
  (v0.28.0) with the note "This work is not released yet"
  ([docs.rs](https://docs.rs/iroh-willow/latest/iroh_willow/)); it is built
  against iroh 0.34-era APIs with `[patch.crates-io]` overrides to n0 forks
  of the willow crates and git-branch dependencies — several iroh generations
  behind current iroh. Downstream projects vendoring it report broken builds.
  Adopting willow-over-iroh today means *forking and porting* iroh-willow,
  not depending on it.
- **Hash-world collision**: Willow'25 payload digests are WILLIAM3/Bab;
  iroh-blobs is BLAKE3/bao. iroh-willow resolved this by instantiating
  Willow with BLAKE3 digests — i.e. it is *a* Willow, but not Willow'25, and
  would not interoperate with willow25 peers. Any estate adoption must pick
  which hash world its payloads live in.

## 11. Spec / interop status

Covered in §0's table. Summary: the *settled* parts are the data model and
Meadowcap (both Final, with two independent implementations each in Rust and
TS). The *unsettled* parts are everything about moving data between peers
over a network: Confidential Sync paused mid-rewrite, WTP an early proposal,
Willow'25 itself still Proposal. Interop risk concentrates exactly where a
fleet would need stability — sync.

## 12. Project health

- **Team**: two people (Meyer, Gwilym) as worm-blossom, plus occasional
  contributors (glyph worked on the Rust implementation). Bus factor is the
  obvious risk.
- **Funding**: grant-based throughout — NLnet NGI Assure (2022), then a
  €50K NGI0 Core grant, then a dedicated NLnet grant for Bab
  ([Open Collective](https://opencollective.com/worm-blossom/updates/willow-and-earthstars-sping);
  [Codeberg repo funding note](https://codeberg.org/worm-blossom/willow_rs/)).
  Donation income is ~$3K/year. There is no company, no hosted service, no
  commercial support.
- **Activity**: alive and publishing — weekly dev diary at worm-blossom.org,
  spec updates through Nov 2025 (Meadowcap → Final), 2026 commits on
  Codeberg (persistent-store work, April 2026), and external research uptake
  (Amparore's storage paper, Riot's audit, Leaf building on top). The pace
  is deliberate and specification-first: the team optimises for the spec
  being *right*, not for shipping product surface.
- **Posture**: public-good infrastructure research. If the estate adopts
  Willow, it is adopting a spec and a two-person upstream, and should expect
  to carry implementation weight itself.

## 13. Operational complexity

- **Instantiation burden**: raw Willow is a blueprint; Willow'25 removes most
  choices, but you still own key custody, renewal services, and store
  operations.
- **Timestamp discipline becomes security-critical**: claimed timestamps
  drive both conflict resolution and capability windows. Clock policy,
  future-dating bounds, and expiry-hygiene writes are operational duties
  most systems don't have.
- **Capability chains are per-entry overhead**: every authorised entry
  carries its full delegation chain + signature; deep delegation graphs cost
  bytes and verification time on every sync, forever.
- **Store operations are immature**: no admin tooling, no metrics, backup =
  copy the embedded-KV directory or export drops; the efficient-storage
  question is still being researched (§6).
- **Sync operations don't exist yet** in Rust (§8): any production sync is
  your code, your bugs.

## 14. What Willow / Meadowcap pointedly does NOT do

1. **No revocation** (§2.5) — only non-renewal of time-bounded grants and
   supersession of writes.
2. **No groups, roles, or membership management** (§3).
3. **No transport, discovery, NAT traversal, or relays** (§4).
4. **No built-in end-to-end encryption** — no payload key distribution, no
   group encryption, no forward secrecy for stored data (§5.1).
5. **No history or auditability** — stores keep only current state, by
   design (traceless deletion is a feature).
6. **No Byzantine tolerance inside a subspace** — authorised writers are
   trusted, including with timestamps; there is no completeness proof a
   malicious peer is withholding data (deliberately).
7. **No consensus / global ordering / cross-namespace transactions** —
   namespaces are independent universes; a store is single-namespace.
8. **No SQL store, no Cloudflare story, no production Rust sync** (§§7–9).
9. **No identity recovery/rotation story** (§1).
10. **No rich authorisation logic** — read/write over areas is all there is;
    no append-only mode, no quotas, no conditional caveats (§2.6).

---

## 15. Judgment

### (a) As a full stack

Willow is two finished, excellent layers (data model + Meadowcap) wearing
an unfinished third layer (network sync). The data model's bet — mutable,
named, deletable data with filesystem semantics, replicated as a state-based
CRDT with first-class partial sync — fits infrastructure state (configs,
registries, seat state) arguably better than append-only log models, and
its reconciliation design is principled and difference-proportional.

But adopting "Willow for everything" in Rust today means: alpha crates with
a recent hash-correctness incident in their history, a sled/fjall/redb store
instead of the estate's SQLite/Turso, *no* Rust sync implementation, a sync
spec paused mid-rewrite, your own transport integration, your own
revocation/renewal service, and a two-person grant-funded upstream. The
estate would become the de-facto third implementation team. That is a
research adoption, not a platform adoption.

### (b) As a borrowed capability layer

Meadowcap is the family's export-grade asset, and it was *designed* to be
borrowed: the data model treats authorisation as a parameter, and Meadowcap
itself is generic over signature schemes and needs only an area-like
grouping (subspace × path-prefix × time-range) over whatever it protects.

Borrowed into another stack (p2panda logs, a fleet broker, a sync protocol
of our own), it buys:

- receiver-bound, offline-verifiable, self-certifying authorisation tokens
  that travel with the data;
- attenuation that is cryptographically monotone (area inclusion enforced by
  the validity definition, not by policy code);
- the cleanest available answer to "who may write/read *this slice* of the
  data space", including the owned/communal duality that maps directly onto
  "managed fleet" vs "open federation";
- a Final spec with two independent implementations to check against.

And its costs travel with it: no revocation (budget for a renewal service
issuing short-window capabilities — cheap for the fleet, whose seats already
check in with a broker; expensive for genuinely offline peers), always-on
transitive delegation, read enforcement that requires cooperating servers,
and per-entry chain overhead. A borrower should also decide early whether
to take the `meadowcap` crate (generic but const-generic-heavy, ufotofu
ecosystem) or re-implement the (compact) validity rules against the spec —
the spec is genuinely implementable, which is itself the strongest thing
that can be said about a capability design.

**Bottom line**: Willow/Meadowcap is the field's best-specified answer to
*area-scoped capability authorisation for replicated data*, and its data
model is a serious contender wherever mutable named state matters. As a
turnkey stack for this estate it is not ready — the sync layer is a draft,
the Rust storage story conflicts with the estate's SQLite/Turso standard,
and revocation/group semantics would be estate-built regardless. The
composed play — another sync/transport core, Meadowcap (spec or crate) as
the authorisation layer, short-window renewal run by the fleet broker —
captures most of the value at a fraction of the adoption risk.
