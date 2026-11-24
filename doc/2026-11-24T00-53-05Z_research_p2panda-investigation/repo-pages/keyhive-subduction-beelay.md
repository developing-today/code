# Keyhive, Subduction, and Beelay — convergent capabilities and encrypted sync (Ink & Switch)

Full lane notes: research lane KEYHIVE for
[#13779](https://github.com/developing-today/code/issues/13779), written
2026-11-23 from the Keyhive dev notebook, the in-repo design documents,
both GitHub repos, crates.io / docs.rs, and the BeeKEM paper.

## Verdict in one sentence

Keyhive is the only candidate in this programme that treats
_authorization itself_ as a replicated, offline-first data structure — a
state-based CRDT of signed delegations and revocations ("Keyline") whose
evaluation gives coordination-free revocation, plus a decentralised
group-key protocol (BeeKEM) that turns that membership graph into
end-to-end encryption keys with post-compromise security. Nothing else in
the field does this combination. It is also, today, **pre-alpha,
unaudited, and carrying known authorisation gaps its own threat model
discloses** — a design to learn from and a composition candidate for later,
not a foundation to build the estate on this quarter.

## The three pieces, and the naming history

One stack from one lab (Ink & Switch — the Automerge lab), three separable
artifacts:

- **Keyhive** — the access-control system: signing, encryption, and
  delegation for local-first data. Rust workspace, Apache-2.0
  ([inkandswitch/keyhive](https://github.com/inkandswitch/keyhive)).
  Three layers: convergent capabilities (the authority graph), a
  group-management CRDT, and E2EE with causal keys.
- **BeeKEM** — the continuous group key agreement (CGKA) protocol inside
  Keyhive: a concurrent variant of TreeKEM (the protocol under MLS) that
  needs no central ordering server. Now also a published research paper
  with security proofs (ePrint 2026/1434, July 2026; listed as in
  submission).
- **Subduction** — the sync protocol: peer-to-peer synchronisation of
  encrypted, hash-linked data (CRDT commits) over pluggable transports,
  with Keyhive as its authorisation layer. Separate Rust workspace, MIT OR
  Apache-2.0 ([inkandswitch/subduction](https://github.com/inkandswitch/subduction)).

Naming: the project was originally **Beehive**, renamed Keyhive before
open-sourcing. **Beelay** was the first Keyhive-enabled sync/relay design
(an RPC protocol with RIBLT set reconciliation); the Keyhive README states
plainly that Subduction is "the successor to Beelay".

The design intent is explicit about scope: Keyhive does access control for
replicated data and deliberately does **not** do identity, naming, key
recovery, or sync — those are layered above/beside it, and the design
overview draws identity, recovery, and sync as external dashed boxes.

## Identity & key management

- **Keys are the only principals.** Every node in the authority graph is
  an Ed25519 verifying key; at the Keyline level there is _no distinction_
  between individuals, groups, and documents — higher layers assign
  meaning to particular keys.
- **An individual is one immutable Ed25519 keypair.** A _person_ is
  modelled as a group whose members are that person's per-device
  individuals; key rotation for a person = add a new device individual,
  remove the old one.
- **Documents/groups bootstrap via a root edge.** At creation the subject
  key signs exactly one delegation to a freshly minted "root role," and
  the subject's signing key is then destroyed (Keyhive ships an
  `EphemeralSigner` for this). The subject's identity is its verifying
  key, permanently; its authority immediately lives in the role graph.
- **Human identity is explicitly out of scope.** Keyhive excludes the
  binding of a human identity to an application's identifier; name
  registration, discovery, and verification are left to a future layer.
  The design overview names the intended neighbours: PKI, DIDs, Keyoxide,
  KERI, OpenPGP, petnames, ATProto. This composes cleanly with this
  estate's existing fleet CA / IdP work — Keyhive would _consume_ those
  identities, not replace them.
- **Key custody is the host's problem, by design.** Keyhive does not own
  key storage; its `AsyncSigner` abstraction lets hosts keep signing keys
  in WebCrypto (non-extractable), hardware, or a KMS.
- **Share (encryption) keys**: each group member also holds an X25519
  keypair used by BeeKEM, published as signed "prekey" operations that
  must be canonical large-order keys verified against the issuer's
  signature, so nobody can publish keys for an identity they don't hold.

## Capability model — Keyline, "convergent capabilities"

This is the family's centre of gravity, and the part with no real rival in
the programme.

### The model

Keyline is "the authority graph: a state-based CRDT of delegations and
revocations". It sits deliberately between the two classical capability
families: object capabilities are fail-stop under partition (wrong for
local-first); certificate capabilities (SPKI/SDSI, zcap-ld, UCAN) are
partition-tolerant but stateless. Convergent capabilities carry CRDT state
to get both.

There are exactly two certificate kinds, both signed Ed25519 statements,
content-addressed by the digest of issuer + payload, stored in an
append-only set whose merge is set union:

- **Delegation**: `{issuer, audience, subject, power, citation?}` —
  "issuer asserts that audience may exercise `power` over `subject`."
  `subject` is a _scope_, not an endpoint: a delegation whose subject is a
  role is a _membership_ and conveys everything that role reaches, now or
  in the future (late binding).
- **Revocation**: `{issuer, revoke: Digest<Delegation>, retain}` — names
  one delegation by hash; `retain` carries per-subject content-retention
  watermarks for the content layer and is never read by evaluation.

Power is a totally ordered ladder, and the bottom rung is this estate's
friend:

```
Relay < Read < Edit < Admin
```

`Relay` = may sync and relay ciphertext but **cannot decrypt** — it makes
untrusted relays first-class citizens of the graph. `Read` = decrypt,
`Edit` = write, `Admin` = act on the graph itself.

### No proof chains — authority is derived, not presented

A delegation has **no proof field**; justification is computed at
verification time from the whole certificate set: an audience holds
standing over a subject iff _some_ unbroken route exists from the subject
to the audience through live delegations. In the design doc's own
summary: "Keyline is RT₀ with SDSI chain discovery, with revocation
semantics closest to ARBAC97's administrative relations, evaluated as
stratified Datalog."

- **Late binding**: a delegation issued before its issuer had authority
  becomes effective the moment the issuer gains it, and dies the moment
  the issuer loses it. "Edges are facts; authority is derived."
- **Redundant routes**: access via two chains survives one breaking;
  there is no single brittle proof chain to invalidate (contrast UCAN,
  where the chain is the artifact).
- **Order independence**: any two replicas holding the same set compute
  the same authority graph, in any delivery order — the CRDT property.
- Evaluation is graph-global: no certificate can be verified in
  isolation, only against a set — a stratified-Datalog least fixed point
  in two passes (a positive pass ignoring all revocations, then a live
  pass filtering routes through the computed revocation coverage).

### Attenuation

Effective power is the **maximum over routes of the minimum along each
route** (widest-path / bottleneck): delegating `Admin` when you hold only
`Edit` conveys `Edit`. Indirection cannot raise power, and `keyhive_core`
refuses to even sign a delegation above the issuer's effective power.

### Revocation — the hard part, done unusually well

Revocation in eventually-consistent systems is where capability designs
usually hand-wave. Keyline's semantics are specified to an unusual depth:

- **Coordination-free and permanent.** A revocation, once applied by any
  replica, can never be un-applied by a merge; access can only be restored
  by a fresh signature from live authority, never by delivery order.
- **Who can revoke what.** The issuer or the audience of a delegation can
  kill it on _every_ route (total). A third party's revocation breaks the
  target only on routes passing through the revoker's **admin reach** —
  the set of nodes that key _ever_ held Admin over, computed with all
  revocations ignored, so it only ever grows. Anyone can revoke a
  delegation on routes through their own node. A revocation outside those
  scopes is _inert_ — valid, admissible, zero effect.
- **Implicit cascade.** Revoking Alice's membership enumerates nothing;
  every delegation she issued simply fails the liveness check at next
  evaluation, transitively.
- **Revival, honestly treated as both feature and hazard.** If a removed
  key regains standing, its old delegations spring back to life (repair by
  re-add costs one certificate, with provenance intact). The hazard — an
  unintended re-add reviving forgotten grants — is blunted by documented
  practice: explicit revocations on removal (permanent, survives re-add)
  and fresh-key discipline after compromise.
- **The ex-admin sharp edge, disclosed rather than hidden.** An ex-admin
  retains revocation power over their frozen admin reach _forever_ —
  deny-only, so they can grief but never grant. The durable escape is
  **rotation**: mint a fresh role key, re-supply it, re-roster the
  survivors. The design doc is blunt that "removing an admin needs a
  rotation."
- **Root-role caveat.** Removal from the root role is _never durable_
  while the subject key is destroyed; the root role is append-only trust —
  "treat it like a root CA: set it up once and use it rarely," with all
  churn in second-layer roles where rotation works.

### What the capability model pointedly does NOT have

- **No expiry.** No timestamp, TTL, or not-after field on either
  certificate kind, deliberately: the threat model lists "trusted time" as
  a non-goal — ordering is causal only. Short-lived authority must be
  modelled as delegation + prompt revocation, or layered on top. For an
  estate used to UCAN-style expiring tokens, this is the single biggest
  semantic difference.
- **No ambient policy language.** Roles, powerboxes, caretakers, pinning,
  device groups are _patterns_ over the two primitives, not built-in
  constructs.
- **Back-dating is only partly answerable.** Keyhive cannot distinguish a
  genuinely old operation arriving late from a fabricated back-dated one;
  revocations record content-head watermarks so the content layer can
  reject post-revocation edits, but applications that need this must add a
  timestamping or anchoring service.

## Group management

Groups are not a separate subsystem; they are the same graph read
differently. A **role** is just a key that others hold membership in;
membership = delegation with the role as `subject`; a **supply** =
delegation _to_ a role about another subject. Roles nest arbitrarily,
documents are themselves groups, and documents-as-members gives
folder-style hierarchies. Because membership is late-bound, supplying a
role with a new document grants it to all current members with **no
re-issuance**. A documented pattern, **constitutional flatness** (a role's
Admin memberships name only individuals, never another role), makes
rosters self-certifying and untouchable by outsiders, and makes rotation
cheap. The group graph is also what drives encryption and sync: following
delegations tells a writer who holds Read (who must be seated in the CGKA
tree), and tells a trust-minimised sync engine which documents a peer may
pull.

## Encryption

### Primitives

Fixed ciphersuite, no crypto agility (an explicit non-goal): **Ed25519**
signing, **X25519** Diffie-Hellman, **XChaCha20-Poly1305** AEAD, **BLAKE3**
hashing and key derivation. The one novel construction is the AEAD nonce:
a synthetic nonce computed as a keyed hash, under a subkey of the
encryption key, of the associated data and plaintext — deterministic,
key-committing, and explicitly "assumed secure pending review"; the repo
README warns that the nonce / key-commitment construction has not been
independently reviewed.

### BeeKEM — per-document group keys

Every document with readers gets a **BeeKEM tree**: a binary tree whose
leaves are the members (each leaf = member ID + current X25519 public
key), whose root holds the group's current **root secret** encrypted under
the tree. A member decrypts the root secret by walking its leaf-to-root
path doing O(log n) DH decryptions.

- **Why not MLS/TreeKEM**: TreeKEM requires a central server to totally
  order operations and pick winners among concurrent ones. BeeKEM keeps
  TreeKEM's shape but requires only _causal_ order, merging concurrent
  updates in any order — when two leaves concurrently update a node, both
  DH keys are kept as "conflict keys," the node is treated as blank for
  encryption purposes, and updates encrypt to the node's _resolution_, so
  a compromised old leaf key cannot decrypt across the fork ("cross-fork
  security"). A causally later update collapses the conflicts.
- **Membership change = key change.** _Remove_ blanks the member's leaf
  and its entire path to the root; _add_ places the member at the next
  blank leaf and blanks its path. The next member _Update_ (key rotation)
  re-keys the blanked path and restores a root secret the removed member
  cannot derive — the post-compromise-security mechanism.
- **Costs**: O(log n) update/decrypt in the common case, O(n) worst case
  (maximally blanked or conflicted tree); worst-case space O(n log n)
  while conflicts are unresolved. Target group size is "on the order of
  thousands of members."
- Only members with `Read` or higher are seated as BeeKEM leaves; `Relay`
  holders are excluded from the CGKA group. Keyline drives the CGKA: the
  capability graph determines whose X25519 keys are in the tree.

### Content encryption — causal keys, and the forward-secrecy trade

Content is encrypted **at rest, in chunks**: ranges of CRDT changes are
compressed (Automerge binary format) and then encrypted. **Causal
encryption**: each chunk's key envelope includes the keys of all its
_causal predecessor_ chunks (Cryptree-like). Holding a chunk's key lets
you iteratively discover the keys for its entire causal history.
**Forward secrecy is deliberately sacrificed for content**, and the design
says why: an op-based CRDT cannot materialise a view with gaps in history,
so every reader needs every historical key anyway. What is retained is
**post-compromise security for the future**: revoke + next BeeKEM path
update cuts off all future chunks. The threat model lists forward secrecy
as an explicit non-goal.

- **Padding**: block bodies are padded to PADMÉ lengths (256-byte floor);
  operation counts, CGKA operation sizes, timing, document IDs, and the
  membership graph remain visible to relays — accepted, since relays must
  see membership to evaluate capabilities.
- **In transit**: Keyhive itself prescribes no channel security; the
  design expects sync to run over TLS/mTLS or an equivalently confidential
  transport, with all payloads E2EE regardless. Subduction adds its own
  authenticated handshake.

### Known gaps — from the project's own threat model

The current _code_ falls short of the design in ways the authors disclose
as tracked gaps G1–G4. Two are authorisation-relevant:

- **G1**: CGKA authorisation reads the _positive_ graph (ignoring
  revocations), so **a revoked member can delegate to a fresh key of its
  own and get that key seated in the tree** by every replica.
- **G2**: a CGKA add's share key is not checked against the added
  member's published prekeys — an adder can seat a key it holds itself.
- **G3**: eviction DoS — any principal with positive standing (even
  `Relay`) can manufacture a cite-able revocation and remove a reader
  from the tree (denial of service, not disclosure).
- **G4**: replicas reconcile their BeeKEM trees only after their _own_
  membership writes, so a peer's revocation removes a reader from the
  tree only via the revoker's replica.

Plus: no content layer yet reads the `retain` watermark, and the
synthetic-nonce construction awaits independent review. And the
repo-level warning stands over everything: "DO NOT use this release in
production applications… The code has not had a security audit."

## Subduction — the sync protocol

### What it syncs

Subduction replicates **Sedimentrees**: a set-based, content-addressed
store for one hash-linked commit DAG — _loose commits_ (individual signed
commits) plus _fragments_ (compressed ranges of commits). It is
essentially an LSM tree adapted to CRDT graph data, with one twist —
compaction must be deterministic across replicas that have never
communicated, so fragment depth is assigned by the number of leading zero
bytes in each item's BLAKE3 hash. The protocol is **CRDT-agnostic**: it
syncs hash-linked data; Automerge is the original target and the only
shipped data integration, but the core sync layer doesn't assume anything
about the CRDT it's carrying.

### Reconciliation

- **Fingerprint set reconciliation.** Peers exchange 8-byte SipHash-2-4
  fingerprints computed with a random per-request seed; batch sync
  reconciles a whole tree in **1.5 round trips** (request with fingerprint
  summary + subscribe flag; response with the data you're missing and the
  fingerprints needed back; a fire-and-forget third leg — storage is
  idempotent, and an empty diff next round confirms success). The authors
  claim the initial diff is ~75% smaller than sending full digests.
- **Subscriptions.** After batch sync, peers opt into real-time
  incremental forwarding, filtered per-subscriber by authorisation.
  Subscriptions are per-peer, not per-connection.
- **Encryption-friendly by construction.** Reconciliation runs on content
  hashes and fingerprints only; sync never touches plaintext — a relay can
  reconcile and forward data it cannot read.

### Peer authentication and authorisation

- **Mutual authentication handshake**: two messages, symmetric. The
  initiator sends a signed challenge `{audience, timestamp, nonce}`; the
  responder verifies the signature, validates audience and timestamp,
  checks the nonce against a replay cache (4 buckets × 3 min = 12-minute
  window), and returns a signed response over the challenge digest. Both
  sides learn each other's peer ID from the verifying key — Ed25519
  throughout.
- **Identity ≠ permission, kept separate in the API.** The handshake
  establishes only _who_ is connecting. Whether they may connect is a
  `ConnectionPolicy`; whether they may fetch/store a given tree is a
  `StoragePolicy` — distinct traits.
- **Keyhive integration is a policy crate**: with it, a relay serves only
  peers that can prove `Relay` or better for the document in question.
- **Revocation interplay**: there is no revocation message in Subduction.
  Access changes arrive via Keyhive; when the auth check fails,
  Subduction simply stops forwarding to that peer.

### Relationship to Keyhive, precisely

Subduction does not depend on Keyhive, and Keyhive does not depend on
Subduction. The coupling is a policy interface plus a shared worldview:
both treat signed, hash-linked, encrypted data as the unit of replication,
and the Keyhive graph tells Subduction which trees a peer may pull. In
the one shipped end-to-end stack (Automerge Repo + GAIOS), Keyhive is
plugged in as the auth provider at the repo layer, while Keyhive's own
operations initially synced over the same network stack with a naive
diffing mechanism that Subduction is meant to replace.

## BeeKEM the paper — the family's strongest maturity signal

_BeeKEM: Decentralized, Secure and Efficient Group Key Agreement_ — Yen,
Fábrega, Da, Kleppmann, Mumm, Park, Zelenka — is on the Cryptology ePrint
Archive as [Paper 2026/1434](https://eprint.iacr.asia/2026/1434) (July
2026; listed as in submission). Its claims: the first decentralised group
key agreement protocol with logarithmic update cost in the common case
(degrading to linear in the worst case) **and proofs of security**, proven
under the existing DCGKA security definitions, with a new security notion
(cross-fork security) for attacks unique to the decentralised setting, an
open-source implementation, and benchmarks competitive with OpenMLS. That
is a materially stronger evidence base than any other novel crypto in this
programme. The honest counterweight, from the ActivityPub E2EE task
force's public minutes (2026-07-28): participants recommended BeeKEM as a
model to consider while noting it "would need another cryptographer to
review it" — preprint + proofs, not yet peer-reviewed or independently
audited.

## Transport assumptions & NAT traversal

- **Keyhive prescribes no transport at all.** Its operations are signed
  data; the design overview notes the only requirement is that "patches
  eventually get from one machine to another, so P2P, sync servers,
  sneakernets, and carrier pigeon are all feasible in this model."
- **Subduction is transport-agnostic with three shipped transports**:
  WebSocket, HTTP long-poll ("for restricted network environments"), and
  **Iroh/QUIC** (`subduction_iroh`, "for direct peer-to-peer
  connections").
- **NAT traversal is inherited, not built**: the iroh transport gets
  iroh's hole-punching and relay fallback; the WebSocket/HTTP transports
  assume a reachable server/relay. The protocol core is `no_std`, so it
  also runs in Wasm/browser and embedded contexts.
- There is **no discovery system**: peers connect to known peer IDs /
  URLs. Discovery and naming are, as with identity, someone else's layer.

## Sync / replication semantics

- **Data layer**: set-union replication of content-addressed, signed
  items. Idempotent — receiving the same data twice is always safe. No
  total order anywhere; causal order only where the CRDT requires it.
- **Auth layer**: state-based CRDT; convergence is by construction (same
  set ⇒ same derived authority graph).
- **Offline behaviour**: the family's home turf — every operation
  (delegate, revoke, edit, rotate keys) is a local act on local state
  that merges later. Nothing requires a live round trip to a server or
  quorum.
- **Conflict story**: data conflicts are the carried CRDT's problem;
  auth conflicts are resolved by the evaluation rules. The one
  content-layer conflict the family explicitly leaves open is _whiteout_
  — whether a revoked member's past writes stay materialised — punted to
  the application via the `retain` watermark, with the semantics of
  combining watermarks still an open question.

## Storage fit

- **Keyline's stored state is two append-only tables.** Delegations and
  revocations are flat, fixed-shape, content-addressed rows — as
  SQLite-shaped as data gets. The subtlety is _evaluation_: the
  implementers' notes work through exactly this question. The pipeline
  (positive pass → coverage → live pass) is expressible in SQL as chained
  CTEs, **except** the standing derivation itself, which is a _non-linear_
  fixpoint (a derivation step can consume two recursively derived facts)
  and therefore exceeds `WITH RECURSIVE` in both SQLite and PostgreSQL. A
  SQL backend needs one ordinary statement per fixpoint round, plus a
  driver loop that repeats until a round adds no rows. The loop is the
  only non-declarative ingredient in the design. Evaluation cost itself is
  polynomial with small constants; the reference evaluator answers
  `members()` in ~1 ms for a 180-member document.
- **What ships today is an in-memory store**: `keyline_memory` is the
  in-memory reference store behind a `Keyline` store trait — a
  SQLite/Turso backend is a designed-for, documented exercise, not
  existing code. `keyhive_core` additionally has an `archive` module for
  serialising a whole Keyhive state.
- **Subduction storage is a trait with several backends, none of them
  SQLite**: filesystem storage with journaling/fsync, an in-repo redb
  storage, in-memory storage, and a published object-storage crate
  (`subduction_object_storage`, S3 and other object stores — R2's API is
  S3-compatible). For this estate: Turso/libsql would be a new backend
  behind existing traits on both sides — plausible, bounded work; not a
  download.

## Rust maturity & project health

- **Keyhive**: repo created 2024-08-13; 328 commits, 280 stars, 32
  releases; Apache-2.0. Crates published: `keyhive_core` 0.5.0
  (2026-06-26; six releases since 0.1.0 in June 2025; ~13.5k all-time
  downloads, ~1k/month), `beekem` 0.5.x, `keyhive_crypto`,
  `keyhive_content`, `keyhive_codec`, `keyline`, `keyline_memory`,
  `keyhive_wasm` (Wasm + TypeScript bindings). CI checks include `no_std`
  and MSRV. Self-described status, repeated in the README, the notebook,
  and the crate keywords ("pre-alpha", "preview"): **pre-alpha, unstable
  APIs, no security audit, do not use in production**.
- **Subduction**: repo created 2025-07-18; 280 commits, 139 stars, 110
  releases; crates at 0.19.x with releases still landing ~fortnightly;
  dual MIT/Apache-2.0. Self-described as "an early release preview… very
  unstable API… DO NOT use for production use cases at this time".
- **Signs of life beyond the lab**: Ink & Switch is dogfooding both in
  GAIOS (its ARIA Safeguarded AI Programme platform) — Keyhive as the auth
  provider for Automerge Repo, Subduction being integrated to replace the
  reference sync server, which "has shown stress points" at scale. A
  third-party adopter (Muni Town's Roomy) is publicly planning on Keyhive
  for access control.
- **Funding/backing**: Ink & Switch is a small independent research lab
  (1–10 people) funded by grants, sponsorships, and commissioned work;
  Keyhive work was accelerated under the ARIA programme for GAIOS. Bus
  factor is a real consideration: the named authors across both repos and
  the paper are a handful of people.
- **Spec/interop status**: no standards-track specification and no second
  implementation of either protocol. The closest things to specs are the
  in-repo design documents (extensive, and unusually candid) and the
  BeeKEM paper. One genuine interop hook exists: the `keyline` crate
  ships a **conformance suite** (scenarios and laws) that the design docs
  treat as the contract — "where this document and the conformance suite
  disagree, the suite wins."

## Cloudflare fit

Nothing Cloudflare-specific exists. No Workers bindings, Durable Objects
integration, or KV usage in either repo. What exists is Wasm:
`keyhive_wasm` and Subduction's Wasm crates targeting browser and Node.js.
Running Keyline evaluation inside a Worker via `keyhive_wasm` is plausible
but unpublished and unexercised; the estate would be the first. **R2 is
reachable in principle**: `subduction_object_storage` puts a Sedimentree
store on any `object_store` backend, and that crate family speaks S3. This
is a storage backend for a Subduction _server_, not a Worker deployment
shape. Subduction's server story is the CLI (with NixOS and Home Manager
modules in the flake — a genuine fit for this estate's NixOS hosts) or
hyper/axum embedding. In the estate's current shape, a Subduction relay
would sit _behind_ a Cloudflare door, not on the edge.

## iroh fit

The best of any family in the programme, on paper: Subduction **already
ships an iroh transport** (`subduction_iroh`, published at 0.11.x) — it
doesn't merely compose with iroh, it has done the composition. The identity
models also align: Subduction peer IDs are Ed25519 verifying keys learned
from the handshake, and iroh NodeIds are Ed25519 public keys — the estate's
[#13752](https://github.com/developing-today/code/issues/13752) design
already treats NodeId-as-identity as the iroh story, and a
Subduction-over-iroh binding inherits it directly. Keyhive itself is
transport-blind, so it neither helps nor hinders. Caveat: the iroh
transport's maturity inside Subduction is unproven — published and
integration-tested alongside the other transports, but the project's own
dogfooding (GAIOS/Automerge Repo) runs over WebSocket-style network stacks.

## What this family pointedly does NOT do

- **Identity, naming, discovery, key recovery** — all layered elsewhere,
  by design.
- **Forward secrecy for content history** — sacrificed deliberately; PCS
  for future content is the guarantee.
- **Expiry / trusted time** — no clocks in the capability layer at all.
- **Availability** — "Keyhive guarantees that what is delivered is
  correct, not that it is delivered."
- **Traffic-analysis resistance** beyond coarse size padding; relays see
  the membership graph.
- **A data model** — Keyhive/Subduction carry _other people's_ CRDTs.
  There is no Keyhive answer to "what is a document's schema" or "how do
  concurrent text edits merge."
- **Production readiness** — by the authors' own repeated statement.

## Where this family sits in a composed stack for this estate

**As the auth layer over someone else's sync — that is exactly the shape
it was designed for, and the strongest composition story in this
programme.** Concretely, over p2panda: p2panda operations are signed,
hash-linked, and Ed25519-keyed — the same shape of artifact Keyhive's
layers consume. In a composed stack: p2panda topics/documents become
Keyline subjects; fleet seats and user devices become individuals grouped
under person/seat roles; Keyline evaluation answers "may this key
pull/edit this topic" at every replica, offline, with coordination-free
revocation; BeeKEM seats the Read-holders of each encrypted topic and
provides the group keys p2panda itself does not manage.

The **`Relay` power level is a direct hit on this estate's architecture**:
the Cloudflare doors and bridge relays that today hold ambient trust
would hold `Relay` — able to store/forward ciphertext, cryptographically
unable to read, and _inside_ the same authorisation graph as everyone
else rather than trusted by construction.

**Subduction is a credible alternative sync for hash-linked data** if the
estate ever wants a second sync engine: CRDT-agnostic, iroh-native,
fingerprint reconciliation, and an object-storage backend that fits R2.
But adopting it _instead of_ p2panda-sync means adopting its data
structure (Sedimentree) too, and it is younger and less proven than
Keyhive's design documents are.

**The blockers are maturity, not fit:**

1. Pre-alpha + unaudited + **G1 is a live revocation bypass in the CGKA
   seating path** as of the current threat model. Until G1/G2 close and
   the ciphersuite gets its independent review, Keyhive cannot be the
   enforcement layer for anything the estate actually needs to keep
   private — it can be the _model_ (the graph semantics are implementable
   from the design docs + conformance suite) before it can be the _code_.
2. No expiry anywhere means the estate's short-lived-session patterns
   (Actions runners, leased exec) need a layer Keyhive doesn't provide —
   revocation-as-expiry works only when replicas sync promptly.
3. The root-role append-only caveat and the ex-admin rotation discipline
   are operational commitments: adopting Keyline means adopting its
   operational folklore (constitutional flatness, fresh-key discipline,
   rotate-after-admin-removal) as runbook, not just its API.
4. Bus factor: one small lab. The design docs are good enough to
   reimplement from (and the conformance suite makes that verifiable),
   which is the real insurance policy — but it should be priced into the
   decision.

**Net**: if the programme's question is "p2panda for everything, or
p2panda + a composed auth layer," Keyhive is the only candidate whose auth
layer was _built_ to be composed this way, and its revocation semantics
are the best-thought-through in the field. The honest sequencing is:
adopt the _model_ now as the reference for whatever auth the estate
builds or borrows; revisit the _code_ when the audit/gap list moves.
