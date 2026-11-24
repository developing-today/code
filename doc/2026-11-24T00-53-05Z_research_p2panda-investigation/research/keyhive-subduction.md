# Keyhive & Subduction: local-first access control and encrypted sync (Ink & Switch)

Research lane KEYHIVE, for the auth + transport investigation (tracking issue
developing-today/code#13779). Written 2026-11-23 from primary sources — the
Keyhive dev notebook, the in-repo design documents, the two GitHub repos,
crates.io / docs.rs / lib.rs, and the BeeKEM paper. Every factual claim is
linked. Secondary summaries are flagged where used.

**One-sentence verdict:** Keyhive is the only candidate in this programme
that treats *authorization itself* as a replicated, offline-first data
structure — a state-based CRDT of signed delegations and revocations
("Keyline") whose evaluation gives coordination-free revocation, plus a
decentralized group-key protocol (BeeKEM) that turns that membership graph
into end-to-end encryption keys with post-compromise security. Nothing else
in the field does this combination. It is also, today, **pre-alpha,
unaudited, and carrying known authorization gaps its own threat model
discloses** — so it is a design to learn from and a composition candidate
for later, not a foundation to build the estate on this quarter.

---

## 1. What each piece IS, and the naming history

The family is one stack from one lab (Ink & Switch — the Automerge lab),
with three separable artifacts:

- **Keyhive** — the access-control system: signing, encryption, and
  delegation for local-first data. Rust workspace, Apache-2.0
  ([repo](https://github.com/inkandswitch/keyhive)). It has three layers:
  convergent capabilities (the authority graph), a group-management CRDT,
  and E2EE with causal keys
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).
- **BeeKEM** — the continuous group key agreement (CGKA) protocol inside
  Keyhive: a concurrent variant of TreeKEM (the protocol under MLS) that
  needs no central ordering server. Now also a published research paper
  with security proofs (§7).
- **Subduction** — the sync protocol: peer-to-peer synchronization of
  encrypted, hash-linked data (CRDT commits) over pluggable transports,
  with Keyhive as its authorization layer. Separate Rust workspace,
  MIT OR Apache-2.0 ([repo](https://github.com/inkandswitch/subduction)).

Naming, because the literature uses all of these: the project was
originally called **Beehive** and renamed Keyhive before open-sourcing
([notebook 03](https://www.inkandswitch.com/keyhive/notebook/03/)).
**Beelay** was the first Keyhive-enabled sync/relay design (an RPC protocol
with RIBLT set reconciliation, described in
[notebook 05](https://www.inkandswitch.com/keyhive/notebook/05/)); the
Keyhive README states plainly that Subduction is "the successor to Beelay"
([repo README](https://github.com/inkandswitch/keyhive)). "Beehive" as a
*system* name is dead; BeeKEM and the bee puns survive.

The design intent is explicit about scope: Keyhive does access control for
replicated data and deliberately does **not** do identity, naming, key
recovery, or sync itself — those are layered above/beside it, and the
design overview draws identity, recovery, and sync as external dashed
boxes ([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).

---

## 2. Identity & key management

- **Keys are the only principals.** Every node in the authority graph is an
  Ed25519 verifying key; at the Keyline level there is *no distinction*
  between individuals, groups, and documents — higher layers assign meaning
  to particular keys
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **An individual is one immutable Ed25519 keypair.** A *person* is
  modelled as a group whose members are that person's per-device
  individuals; key rotation for a person = add a new device individual to
  the group, remove the old one
  ([notebook 05](https://www.inkandswitch.com/keyhive/notebook/05/)).
- **Documents/groups bootstrap via a root edge.** At creation the subject
  key signs exactly one delegation to a freshly minted "root role," and the
  subject's signing key is then destroyed (Keyhive ships an
  `EphemeralSigner` for this). The subject's identity is its verifying key,
  permanently; its authority immediately lives in the role graph
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **Human identity is explicitly out of scope.** "Keyhive deliberately
  excludes user identity (i.e. the binding of a human identity to an
  application's identifier like a public key)"; name registration,
  discovery, and verification are left to a future layer above it
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)). The
  design overview names the intended neighbours: PKI, DIDs, Keyoxide, KERI,
  OpenPGP, petnames, ATProto
  ([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).
  This composes cleanly with this estate's existing fleet CA / IdP work —
  Keyhive would consume those identities, not replace them.
- **Key custody is the host's problem, by design.** Keyhive does not own
  key storage; its `AsyncSigner` abstraction lets hosts keep signing keys
  in WebCrypto (non-extractable), hardware, or a KMS
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
  In the GAIOS integration, messages are signed with "the local
  non-extractable signing key"
  ([Dispatch 014](https://www.inkandswitch.com/newsletter/dispatch-014/)).
- **Share (encryption) keys**: each group member also holds an X25519
  keypair used by BeeKEM, published as signed "prekey" operations; prekeys
  must be canonical large-order X25519 keys, verified against the issuer's
  signature, so nobody can publish keys for an identity they don't hold
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).

## 3. Capability model — "convergent capabilities" / Keyline

This is the family's centre of gravity, and the part with no real rival in
the programme.

### 3.1 The model

Keyhive's capability layer, **Keyline**, is "the authority graph: a
state-based CRDT of delegations and revocations"
([repo README](https://github.com/inkandswitch/keyhive)). It sits
deliberately between the two classical capability families: object
capabilities are fail-stop under partition (wrong for local-first);
certificate capabilities (SPKI/SDSI, zcap-ld, UCAN) are partition-tolerant
but stateless, which "somewhat limits their flexibility." Convergent
capabilities ("concap") carry CRDT state to get both
([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).

There are exactly two certificate kinds, both signed Ed25519 statements,
content-addressed by the digest of issuer + payload, stored in an
append-only set whose merge is set union
([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)):

- **Delegation**: `{issuer, audience, subject, power, citation?}` — reads
  "issuer asserts that audience may exercise `power` over `subject`."
  `subject` is a *scope*, not an endpoint: a delegation whose subject is a
  role is a *membership* and conveys everything that role reaches, now or
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
`Edit` = write, `Admin` = act on the graph itself (revoke on routes
through the node; manage a role's roster)
([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).

### 3.2 No proof chains — authority is derived, not presented

A delegation has **no proof field**. It does not name the chain that
justifies it; justification is computed at verification time from the whole
certificate set: an audience holds standing over a subject iff *some*
unbroken route exists from the subject to the audience through live
delegations. The design doc's summary of its own lineage is exact: "Keyline
is RT₀ with SDSI chain discovery, with revocation semantics closest to
ARBAC97's administrative relations, evaluated as stratified Datalog"
([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
Consequences the authors call out, all of which matter for an offline
estate:

- **Late binding**: a delegation issued before its issuer had authority
  becomes effective the moment the issuer gains it, and dies the moment
  the issuer loses it. "Edges are facts; authority is derived."
- **Redundant routes**: access via two chains survives one breaking; there
  is no single brittle proof chain to invalidate (contrast UCAN, where the
  chain is the artifact).
- **Order independence**: any two replicas holding the same set compute
  the same authority graph, in any delivery order — the CRDT property.
- Evaluation is graph-global, not certificate-local: "no certificate can
  be verified in isolation, only against a set." The evaluation is a
  stratified-Datalog least fixed point in two passes — a positive pass
  ignoring all revocations, then a live pass filtering routes through the
  computed revocation coverage
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).

### 3.3 Attenuation

Effective power is the **maximum over routes of the minimum along each
route** (widest-path / bottleneck): delegating `Admin` when you hold only
`Edit` conveys `Edit`. Indirection cannot raise power, and `keyhive_core`
refuses to even sign a delegation above the issuer's effective power
([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md);
[threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).

### 3.4 Revocation — the hard part, done unusually well

Revocation in eventually-consistent systems is where capability designs
usually hand-wave. Keyline's semantics are specified to an unusual depth:

- **Coordination-free and permanent.** A revocation, once applied by any
  replica, can never be un-applied by a merge; access can only be restored
  by a fresh signature from live authority, never by delivery order. The
  design invariant: "Once a replica has applied a revocation, no merge may
  un-apply it"
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **Who can revoke what.** The issuer or the audience of a delegation can
  kill it on *every* route (total). A third party's revocation breaks the
  target only on routes passing through the revoker's **admin reach** —
  the set of nodes that key *ever* held Admin over, computed with all
  revocations ignored, so it only ever grows. Anyone can revoke a
  delegation on routes through their own node (refusing to let their
  standing carry someone else's delegation). A revocation outside those
  scopes is *inert* — valid, admissible, zero effect. Seniority needs no
  rule: an edge above your admin reach never routes through it, so
  revoking it does nothing
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **Implicit cascade.** Revoking Alice's membership enumerates nothing;
  every delegation she issued simply fails the liveness check at next
  evaluation, transitively. Each certificate's liveness is evaluated on
  its own, so revoking one of two redundant routes leaves the other
  untouched.
- **Revival, honestly treated as both feature and hazard.** If a removed
  key regains standing, its old delegations spring back to life (repair by
  re-add costs one certificate, with provenance intact). The hazard — an
  unintended re-add reviving forgotten grants — is blunted by documented
  practice: explicit revocations on removal (permanent, survives re-add)
  and fresh-key discipline after compromise. A `citation` field on
  re-issued delegations names the revocation being healed past, so a heal
  gets a fresh hash instead of colliding with the revoked original
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **The ex-admin sharp edge, disclosed rather than hidden.** An ex-admin
  retains revocation power over their frozen admin reach *forever* —
  deny-only, so they can grief but never grant. The durable escape is
  **rotation**: mint a fresh role key, re-supply it, re-roster the
  survivors; the fresh node post-dates the ex-admin on every graph and can
  never enter their reach. The design doc is blunt that "removing an admin
  needs a rotation," and mirrors the same shape in BeeKEM (§5)
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **Root-role caveat.** Removal from the root role is *never durable*
  while the subject key is destroyed (every route transits the root role,
  and every root admin's reach covers it); the root role is append-only
  trust, "treat it like a root CA: set it up once and use it rarely," with
  all churn in second-layer roles where rotation works
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).

### 3.5 What the capability model pointedly does NOT have

- **No expiry.** There is no timestamp, TTL, or not-after field on either
  certificate kind (confirmed by full-text search of the Keyline design).
  This is a deliberate consequence of the no-clocks stance: the threat
  model lists "trusted time" as a non-goal — "ordering is causal only"
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
  Short-lived authority must be modelled as delegation + prompt
  revocation, or layered on top. For an estate used to UCAN-style expiring
  tokens, this is the single biggest semantic difference.
- **No ambient policy language.** Roles, powerboxes, caretakers, pinning,
  device groups are *patterns* (arrangements of nodes) over the two
  primitives, not built-in constructs — "some guarantees become
  conventions rather than semantics"
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- **Back-dating is only partly answerable.** Keyhive cannot distinguish a
  genuinely old operation arriving late from a fabricated back-dated one;
  revocations record content-head watermarks (`retain`) so the content
  layer can reject post-revocation edits, but "applications that need
  this must add a timestamping or anchoring service"
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).

## 4. Group management

Groups are not a separate subsystem; they are the same graph read
differently:

- A **role** is just a key that others hold membership in. Membership =
  delegation with the role as `subject`; a **supply** = delegation *to* a
  role about another subject, connecting the role to that subject. Roles
  nest arbitrarily (a role can be a member of another role), documents are
  themselves groups, and documents-as-members gives folder-style
  hierarchies
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md);
  [notebook 05](https://www.inkandswitch.com/keyhive/notebook/05/)).
- Because membership is late-bound, supplying a role with a new document
  grants it to all current members with **no re-issuance** — "nobody
  re-issues the memberships, no certificate changes."
- The **group-management CRDT** framing from the overview: "self-certifying,
  concurrent group management complete with coordination-free revocation"
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).
  Concurrent adds/removes converge deterministically (in BeeKEM, concurrent
  adds to the same leaf are resolved by sorting and blanking paths, §5).
- A documented pattern, **constitutional flatness** (a role's Admin
  memberships name only individuals, never another role), makes rosters
  self-certifying and untouchable by outsiders, and makes rotation cheap;
  the patterns document also covers caretakers, pinning, sealing, and
  stewardship conventions
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).
- The group graph is also what drives encryption and sync: following
  delegations tells a writer who holds Read (who must be seated in the
  CGKA tree), and tells a trust-minimized sync engine which documents a
  peer may pull
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).

## 5. Encryption — the cryptographic design, precisely

### 5.1 Primitives

Fixed ciphersuite, no crypto agility (an explicit non-goal in
[notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)):
**Ed25519** signing, **X25519** Diffie-Hellman, **XChaCha20-Poly1305**
AEAD, **BLAKE3** hashing and key derivation
([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md);
[threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
The one novel construction is the AEAD nonce: a synthetic nonce computed
as a keyed hash, under a subkey of the encryption key, of the associated
data and plaintext — deterministic, key-committing, and explicitly
**"assumed secure pending review"**; the repo README warns that "the
nonce / key-commitment construction in `design/ciphersuite.md` has not
been independently reviewed"
([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md);
[repo README](https://github.com/inkandswitch/keyhive)).

### 5.2 BeeKEM — per-document group keys

- Every document (subject) with readers gets a **BeeKEM tree**: a binary
  tree whose leaves are the members (each leaf = member ID + current
  X25519 public key), whose root holds the group's current **root secret**
  encrypted under the tree, and whose inner nodes hold DH public keys plus
  secrets encrypted via DH exchange with the sibling subtree. A member
  decrypts the root secret by walking its leaf-to-root path doing O(log n)
  DH decryptions
  ([notebook 02](https://www.inkandswitch.com/keyhive/notebook/02/)).
- **Application secrets** are derived from the root secret with BLAKE3
  `derive_key` per purpose, so compromising one application secret reveals
  neither the root secret nor other application secrets (crates.io
  `beekem` README, via
  [crates.io search record](https://crates.io/crates/beekem/reverse_dependencies)).
- **Why not MLS/TreeKEM**: TreeKEM requires a central server to totally
  order operations and pick winners among concurrent ones. BeeKEM keeps
  TreeKEM's shape but requires only *causal* order, merging concurrent
  updates in any order — when two leaves concurrently update a node, both
  DH keys are kept as "conflict keys," the node is treated as blank for
  encryption purposes, and updates encrypt to the node's *resolution*
  (highest non-blank, non-conflict descendants) so that a compromised old
  leaf key cannot decrypt across the fork ("cross-fork security" in the
  paper). A causally later update collapses the conflicts
  ([notebook 02](https://www.inkandswitch.com/keyhive/notebook/02/)).
- **Membership change = key change.** *Remove* blanks the member's leaf
  and its entire path to the root; remove paths are blanked *after* all
  concurrent operations merge, so a removed member's concurrent update
  cannot survive. *Add* places the member at the next blank leaf and
  blanks its path. The next member *Update* (key rotation) re-keys the
  blanked path and restores a root secret the removed member cannot
  derive — that is the post-compromise-security mechanism
  ([notebook 02](https://www.inkandswitch.com/keyhive/notebook/02/)).
- **Costs**: O(log n) update/decrypt in the common case, O(n) worst case
  (maximally blanked or conflicted tree); worst-case space O(n log n)
  while conflicts are unresolved. Target group size is "on the order of
  thousands of members" — well past DCGKA's ~100, short of MLS's 50k
  design point
  ([notebook 02](https://www.inkandswitch.com/keyhive/notebook/02/)).
- Only members with `Read` or higher are seated as BeeKEM leaves; `Relay`
  holders are excluded from the CGKA group and never receive a path secret
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
  Keyline drives the CGKA: the capability graph determines whose X25519
  keys are in the tree — "something not possible with standard certificate
  capabilities alone"
  ([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).

### 5.3 Content encryption — causal keys, and the forward-secrecy trade

- Content is encrypted **at rest, in chunks**: ranges of CRDT changes are
  compressed (Automerge binary format) and then encrypted, rather than
  encrypting every operation. Chunk boundaries shift as history stabilizes
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).
- **Causal encryption**: each chunk's key envelope includes the keys of
  all its *causal predecessor* chunks (Cryptree-like). Holding a chunk's
  key lets you iteratively discover the keys for its entire causal
  history — but not its parents' other children or concurrent chunks
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/)).
- **Forward secrecy is deliberately sacrificed for content, and the
  design says why**: an op-based CRDT cannot materialize a view with gaps
  in history, so every reader needs every historical key anyway; passing
  them in the envelope adds simplicity and flexibility. What is retained
  is **post-compromise security for the future**: revoke + next BeeKEM
  path update cuts off all future chunks. The threat model lists forward
  secrecy as an explicit **non-goal**: "a current reader can decrypt all
  history"
  ([notebook 01](https://www.inkandswitch.com/keyhive/notebook/01/);
  [threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
- **Padding**: block bodies are padded inside the AEAD to PADMÉ lengths
  (256-byte floor), leaking only a coarse size class; operation counts,
  CGKA operation sizes, timing, document IDs, and the membership graph
  remain visible to relays — accepted, since "relays must see membership
  to evaluate capabilities"
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
- **In transit**: Keyhive itself prescribes no channel security; the
  design expects sync to run over TLS/mTLS or an equivalently confidential
  transport, with all payloads E2EE regardless
  ([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).
  Subduction adds its own authenticated handshake (§6).

### 5.4 Known gaps — from the project's own threat model

The current *code* falls short of the design in ways the authors disclose
as tracked gaps G1–G4. Two are authorization-relevant and must be quoted
in any honest assessment
([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)):

- **G1**: CGKA authorization reads the *positive* graph (ignoring
  revocations), so **a revoked member can delegate to a fresh key of its
  own and get that key seated in the tree** by every replica.
- **G2**: a CGKA add's share key is not checked against the added member's
  published prekeys — an adder can seat a key it holds itself.
- **G3**: eviction DoS — any principal with positive standing (even
  `Relay`) can manufacture a cite-able revocation and remove a reader from
  the tree (denial of service, not disclosure; removing the creator
  empties the tree).
- **G4**: replicas reconcile their BeeKEM trees only after their *own*
  membership writes, so a peer's revocation removes a reader from the tree
  only via the revoker's replica.

Plus two open items: no content layer yet reads the `retain` watermark
(T5 untested), and the synthetic-nonce construction awaits independent
review. And the repo-level warning stands over everything: "DO NOT use
this release in production applications… The code has not had a security
audit" ([repo README](https://github.com/inkandswitch/keyhive)).

## 6. Subduction — the sync protocol

### 6.1 What it syncs

Subduction replicates **Sedimentrees**. A Sedimentree is a set-based,
content-addressed store for one hash-linked commit DAG: *loose commits*
(individual signed commits) plus *fragments* (compressed ranges of
commits). It is essentially an LSM tree adapted to CRDT graph data, with
one twist — compaction must be deterministic across replicas that have
never communicated, so fragment depth is assigned by the number of leading
zero bytes in each item's BLAKE3 hash: any peer holding the same data
partitions it identically
([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).
The structure is CRDT-like (merge is set union; `diff` and `minimize` are
the core operations)
([DeepWiki, secondary](https://deepwiki.com/inkandswitch/subduction/4.1-sedimentree-data-structure)).

The protocol is **CRDT-agnostic**: it syncs hash-linked data; Automerge is
the original target and the only shipped data integration
(`automerge_subduction_ingest`, `automerge_subduction_wasm`), but "the
core sync layer doesn't assume anything about the CRDT it's carrying"
([author's notes](https://notes.brooklynzelenka.com/notes/subduction/);
[repo README](https://github.com/inkandswitch/subduction)).

### 6.2 How reconciliation works

- **Fingerprint set reconciliation.** Instead of exchanging 32-byte
  digests, peers exchange 8-byte SipHash-2-4 fingerprints computed with a
  random per-request seed; both sides fingerprint their items with the
  same seed and diff on u64 values. Batch sync reconciles a whole tree in
  **1.5 round trips** (request with fingerprint summary + subscribe flag;
  response with the data you're missing and the fingerprints needed back;
  a fire-and-forget third leg — storage is idempotent, and an empty diff
  next round confirms success). The authors claim the initial diff is
  ~75% smaller than sending full digests
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).
- **Subscriptions.** After batch sync, peers opt into real-time
  incremental forwarding: new commits/fragments are forwarded to
  subscribers, filtered per-subscriber by authorization. Subscriptions are
  per-peer, not per-connection
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).
- **Encryption-friendly by construction.** Reconciliation runs on content
  hashes and fingerprints only; sync never touches plaintext, which is
  the whole point of pairing it with Keyhive — a relay can reconcile and
  forward data it cannot read
  ([repo README](https://github.com/inkandswitch/subduction)).

### 6.3 Peer authentication and authorization

- **Mutual authentication handshake**: two messages, symmetric (either
  side may initiate). The initiator sends a signed challenge
  `{audience, timestamp, nonce}`; the responder verifies the signature,
  validates audience and timestamp, checks the nonce against a replay
  cache (4 buckets × 3 min = 12-minute window, lazy GC), and returns a
  signed response over the challenge digest. Both sides learn each
  other's peer ID from the verifying key — Ed25519 throughout
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).
- **Identity ≠ permission, kept separate in the API.** The handshake
  establishes only *who* is connecting. Whether they may connect is a
  `ConnectionPolicy`; whether they may fetch/store a given tree is a
  `StoragePolicy` — distinct traits in `subduction_core`
  ([docs.rs](https://docs.rs/subduction_core/0.5.0/subduction_core/)).
- **Keyhive integration is a policy crate**: `subduction_keyhive` (types)
  and `subduction_keyhive_policy` (Keyhive-based authorization policy for
  connections) sit in the workspace's Integrations layer
  ([repo README](https://github.com/inkandswitch/subduction)). With it, a
  relay serves only peers that can prove `Relay` or better for the
  document in question
  ([Keyhive threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
- **Revocation interplay**: there is no revocation message in Subduction.
  Access changes arrive via Keyhive; when the auth check fails,
  Subduction simply stops forwarding to that peer
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).

### 6.4 Relationship to Keyhive, precisely

Subduction does not depend on Keyhive, and Keyhive does not depend on
Subduction. Keyhive's design overview draws sync as an external box and
says identity/recovery/sync are "not in scope for this repository (sync is
Subduction)"
([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).
The coupling is a policy interface plus a shared worldview: both treat
signed, hash-linked, encrypted data as the unit of replication, and the
Keyhive graph tells Subduction which trees a peer may pull. In the one
shipped end-to-end stack (Automerge Repo + GAIOS), Keyhive is plugged in
as the auth provider at the repo layer — messages signed with the local
key, per-document authorization checked against the local Keyhive
instance — while Keyhive's own ops initially synced over the same network
stack with "a naive diffing mechanism" that Subduction is meant to
replace
([Dispatch 014](https://www.inkandswitch.com/newsletter/dispatch-014/)).

## 7. BeeKEM the paper — the family's strongest maturity signal

*BeeKEM: Decentralized, Secure and Efficient Group Key Agreement* — Yen,
Fábrega, Da, Kleppmann, Mumm, Park, Zelenka — is on the Cryptology ePrint
Archive as [Paper 2026/1434](https://eprint.iacr.asia/2026/1434) (July
2026; listed as in submission by the authors). Its claims, in its own
abstract: "the first decentralized group key agreement protocol with
logarithmic update cost in the common case (degrading to linear in the
worst case) **and proofs of security**," proven under the existing DCGKA
security definitions (Weidner et al.), with a new security notion
(cross-fork security) for attacks unique to the decentralized setting, an
open-source implementation, and benchmarks "competitive with OpenMLS."
That is a materially stronger evidence base than any other novel crypto in
this programme. The honest counterweight, from the ActivityPub E2EE task
force's public minutes (2026-07-28): participants recommended BeeKEM as a
model to consider while noting it "would need another cryptographer to
review it"
([swicg meeting notes](https://github.com/swicg/meetings/blob/HEAD/2026-07-28-e2ee/README.md)) —
i.e. preprint + proofs, not yet peer-reviewed or independently audited.

## 8. Transport assumptions & NAT traversal

- **Keyhive prescribes no transport at all.** Its operations are signed
  data; the design overview notes the only requirement is that "patches
  eventually get from one machine to another, so P2P, sync servers,
  sneakernets, and carrier pigeon are all feasible in this model"
  ([design README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).
- **Subduction is transport-agnostic with three shipped transports**:
  WebSocket (`subduction_websocket`, plus `subduction_hyper` for
  hyper/axum/tower servers), HTTP long-poll (`subduction_http_longpoll`,
  "for restricted network environments"), and **Iroh/QUIC**
  (`subduction_iroh`, "for direct peer-to-peer connections")
  ([repo README](https://github.com/inkandswitch/subduction)).
- **NAT traversal is inherited, not built**: the iroh transport gets
  iroh's hole-punching and relay fallback; the WebSocket/HTTP transports
  assume a reachable server/relay (the CLI runs server/relay modes). The
  protocol core is `no_std`, so it also runs in Wasm/browser and embedded
  contexts where the transport is whatever the host provides
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/)).
- There is **no discovery system**: peers connect to known peer IDs /
  URLs (the Wasm example distinguishes discovery-by-first-contact from
  `connect(url, expectedPeerIdHex)` when identity is known in advance)
  ([repo README](https://github.com/inkandswitch/subduction)). Discovery
  and naming are, as with identity, someone else's layer.

## 9. Sync/replication semantics (summary across the family)

- **Data layer**: set-union replication of content-addressed, signed
  items (commits, fragments, certificates). Idempotent — "receiving the
  same data twice is always safe." No total order anywhere; causal order
  only where the CRDT requires it, and Keyhive buffers operations whose
  predecessors are missing rather than diverging
  ([author's notes](https://notes.brooklynzelenka.com/notes/subduction/);
  [threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
- **Auth layer**: state-based CRDT; convergence is by construction (same
  set ⇒ same derived authority graph, §3.2).
- **Offline behaviour**: this is the family's home turf — every operation
  (delegate, revoke, edit, rotate keys) is a local act on local state
  that merges later. Nothing in either protocol requires a live round
  trip to a server or a quorum.
- **Conflict story**: data conflicts are the carried CRDT's problem;
  auth conflicts are resolved by the evaluation rules (concurrent
  revocations both stand; concurrent BeeKEM updates fork-and-heal via
  conflict keys). The one content-layer conflict the family explicitly
  leaves open is *whiteout* — whether a revoked member's past writes stay
  materialized — punted to the application via the `retain` watermark,
  with the semantics of combining watermarks still an open question
  ([Keyline design](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/README.md)).

## 10. Storage fit (SQLite / Turso)

- **Keyline's stored state is two append-only tables.** Delegations and
  revocations are flat, fixed-shape, content-addressed rows — as
  SQLite-shaped as data gets. The subtlety is *evaluation*: the
  implementers' notes work through exactly this question. The pipeline
  (positive pass → coverage → live pass) is "expressible in SQL as
  chained CTEs, and the negation is a legal anti-join," **except** the
  standing derivation itself, which is a *non-linear* fixpoint (a
  derivation step can consume two recursively derived facts — the role
  rule) and therefore exceeds `WITH RECURSIVE` in both SQLite and
  PostgreSQL. A SQL backend needs "one ordinary statement per fixpoint
  round, plus a driver loop that repeats until a round adds no rows. The
  loop is the only non-declarative ingredient in the design." Evaluation
  cost itself is polynomial with small constants; the reference evaluator
  answers `members()` in ~1 ms for a 180-member document
  ([evaluation notes](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/evaluation-notes.md)).
- **What ships today is an in-memory store**: `keyline_memory` is "the
  in-memory reference `Keyline` store," behind a `Keyline` store trait —
  so a SQLite/Turso backend is a designed-for, documented exercise, not
  existing code
  ([repo README](https://github.com/inkandswitch/keyhive)). `keyhive_core`
  additionally has an `archive` module for serializing a whole Keyhive
  state
  ([docs.rs](https://docs.rs/keyhive_core)).
- **Subduction storage is a trait with several backends, none of them
  SQLite**: filesystem storage with journaling/fsync
  (`sedimentree_fs_storage`), an in-repo `subduction_redb_storage`
  (redb — a pure-Rust embedded key-value store), in-memory storage, and a
  published `subduction_object_storage` crate implementing the `Storage`
  trait over the `object_store` crate (S3 and other object stores)
  ([repo README](https://github.com/inkandswitch/subduction);
  [DeepWiki, secondary](https://deepwiki.com/inkandswitch/subduction/2.1-core-components);
  [crates.io](https://fastly-app.crates.io/crates/subduction_object_storage)).
  For this estate: Turso/libsql would be a new backend behind existing
  traits on both sides — plausible, bounded work; not a download.

## 11. Rust maturity & project health

- **Keyhive**: repo created 2024-08-13; 328 commits, 280 stars, 32
  releases at time of writing; Apache-2.0. Crates published:
  `keyhive_core` 0.5.0 (2026-06-26; six releases since 0.1.0 in June
  2025; ~13.5k all-time downloads, ~1k/month), `beekem` 0.5.x,
  `keyhive_crypto`, `keyhive_content`, `keyhive_codec`, `keyline`,
  `keyline_memory`, `keyhive_wasm` (Wasm + TypeScript bindings)
  ([repo](https://github.com/inkandswitch/keyhive);
  [lib.rs](https://lib.rs/crates/keyhive_core);
  [crates.io](https://crates.io/crates/beekem/reverse_dependencies)).
  CI checks include `no_std` and MSRV. Self-described status, repeated in
  the README, the notebook, and the crate keywords ("pre-alpha",
  "preview"): **pre-alpha, unstable APIs, no security audit, do not use
  in production**.
- **Subduction**: repo created 2025-07-18; 280 commits, 139 stars, 110
  releases; crates at 0.19.x (`subduction_core` 0.19.0, `sedimentree_core`
  0.15.0) with releases still landing ~fortnightly at time of writing;
  dual MIT/Apache-2.0. Self-described as "an early release preview… very
  unstable API… DO NOT use for production use cases at this time"
  ([repo](https://github.com/inkandswitch/subduction);
  [inspect.software, secondary](https://inspect.software/software/inkandswitch/subduction)).
- **Signs of life beyond the lab**: Ink & Switch is dogfooding both in
  GAIOS (its ARIA Safeguarded AI Programme platform) — Keyhive as the auth
  provider for Automerge Repo, Subduction being integrated to replace the
  reference sync server, which "has shown stress points" at scale
  ([Dispatch 014](https://www.inkandswitch.com/newsletter/dispatch-014/)).
  The Automerge project's July 2026 notes describe Keyhive as the
  work-in-progress access-control layer and Zelenka as "hard at work
  making our new sync system — Subduction — both fast and boring to
  operate in production"
  ([This Month in Automerge](https://automerge.org/blog/2026-july/)).
  A third-party adopter (Muni Town's Roomy) is publicly planning on
  Keyhive for access control (secondary:
  [meri.garden](https://meri.garden/posts/a-deep-dive-explainer-on-beekem-protocol/)).
- **Funding/backing**: Ink & Switch is a small independent research lab
  (1–10 people) funded by grants, sponsorships, and commissioned work;
  Keyhive work was accelerated under the ARIA programme for GAIOS
  ([Dispatch 014](https://www.inkandswitch.com/newsletter/dispatch-014/)).
  Bus factor is a real consideration: the named authors across both repos
  and the paper are a handful of people (Zelenka, Good, Mumm, plus
  academic collaborators).
- **Spec/interop status**: no standards-track specification and no second
  implementation of either protocol. The closest things to specs are the
  in-repo design documents (extensive, and unusually candid) and the
  BeeKEM paper. One genuine interop hook exists: the `keyline` crate
  ships a **conformance suite** (scenarios and laws) that the design docs
  treat as the contract — "where this document and the conformance suite
  disagree, the suite wins"
  ([evaluation notes](https://github.com/inkandswitch/keyhive/blob/main/design/keyline/evaluation-notes.md)).

## 12. Cloudflare fit

- **Nothing Cloudflare-specific exists.** No Workers bindings, Durable
  Objects integration, or KV usage in either repo. What exists is Wasm:
  `keyhive_wasm` (keyhive_core + TS bindings) and Subduction's Wasm
  crates (`subduction_wasm`, `sedimentree_wasm`,
  `automerge_subduction_wasm`) targeting browser and Node.js
  ([repo README](https://github.com/inkandswitch/keyhive);
  [repo README](https://github.com/inkandswitch/subduction)). Running
  Keyline evaluation inside a Worker via `keyhive_wasm` is plausible but
  unpublished and unexercised; the estate would be the first.
- **R2 is reachable in principle**: `subduction_object_storage` puts a
  Sedimentree store on any `object_store` backend, and that crate family
  speaks S3 — R2's API is S3-compatible. The same crate has a `keyhive`
  feature storing Keyhive events content-addressed in the same bucket.
  This is a storage backend for a Subduction *server*, not a Worker
  deployment shape
  ([crates.io](https://fastly-app.crates.io/crates/subduction_object_storage)).
- **Servers are native**: Subduction's server story is the CLI (with
  NixOS and Home Manager modules in the flake — a genuine fit for this
  estate's NixOS hosts) or hyper/axum embedding
  ([repo README](https://github.com/inkandswitch/subduction)). In the
  estate's current shape, a Subduction relay would sit *behind* a
  Cloudflare door (like the commander/mcpx bridges), not on the edge.

## 13. iroh fit

The best of any family in the programme, on paper: Subduction **already
ships an iroh transport** (`subduction_iroh`, published at 0.11.x) — it
doesn't merely compose with iroh, it has done the composition
([repo README](https://github.com/inkandswitch/subduction);
[inspect.software, secondary](https://inspect.software/software/inkandswitch/subduction)).
The identity models also align: Subduction peer IDs are Ed25519 verifying
keys learned from the handshake, and iroh NodeIds are Ed25519 public keys
— the estate's #13752 design already treats NodeId-as-identity as the
iroh story, and a Subduction-over-iroh binding inherits it directly.
Keyhive itself is transport-blind (§8), so it neither helps nor hinders.
Caveat: the iroh transport's maturity inside Subduction is unproven —
published and integration-tested alongside the other transports, but the
project's own dogfooding (GAIOS/Automerge Repo) runs over WebSocket-style
network stacks, per Dispatch 014.

## 14. What this family pointedly does NOT do

Collected from the projects' own non-goals, so the comparison matrix can
score them as absences rather than oversights:

- **Identity, naming, discovery, key recovery** — all layered elsewhere,
  by design (§2, §8).
- **Forward secrecy for content history** — sacrificed deliberately;
  PCS for future content is the guarantee (§5.3).
- **Expiry / trusted time** — no clocks in the capability layer at all
  (§3.5).
- **Availability** — "Keyhive guarantees that what is delivered is
  correct, not that it is delivered"
  ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)).
- **Traffic-analysis resistance** beyond coarse size padding; relays see
  the membership graph (§5.3).
- **A data model** — Keyhive/Subduction carry *other people's* CRDTs.
  There is no Keyhive answer to "what is a document's schema" or "how do
  concurrent text edits merge"; that is Automerge's (or p2panda's) job.
- **Production readiness** — by the authors' own repeated statement (§11).

## 15. Where this family sits in a composed stack for this estate

**As the auth layer over someone else's sync — that is exactly the shape
it was designed for, and the strongest composition story in this
programme.** Concretely, over p2panda:

- p2panda operations are signed, hash-linked, and Ed25519-keyed — the
  same shape of artifact Keyhive's layers consume. The Automerge team's
  own framing is that Keyhive "will work with any hash graph, including
  CRDTs," not just Automerge
  ([This Month in Automerge](https://automerge.org/blog/2026-july/)).
  In a composed stack: p2panda topics/documents become Keyline subjects;
  fleet seats and user devices become individuals grouped under person/
  seat roles; Keyline evaluation answers "may this key pull/edit this
  topic" at every replica, offline, with coordination-free revocation;
  BeeKEM seats the Read-holders of each encrypted topic and provides the
  group keys p2panda itself does not manage.
- The **`Relay` power level is a direct hit on this estate's
  architecture**: the Cloudflare doors and bridge relays that today hold
  ambient trust would hold `Relay` — able to store/forward ciphertext,
  cryptographically unable to read, and *inside* the same authorization
  graph as everyone else rather than trusted by construction.
- **Subduction is a credible alternative sync for hash-linked data** if
  the estate ever wants a second sync engine: CRDT-agnostic, iroh-native,
  fingerprint reconciliation, and an object-storage backend that fits R2.
  But adopting it *instead of* p2panda-sync means adopting its data
  structure (Sedimentree) too, and it is younger and less proven than
  Keyhive's design documents are.

**The blockers are maturity, not fit:**

1. Pre-alpha + unaudited + **G1 is a live revocation bypass in the CGKA
   seating path** as of the current threat model. Until G1/G2 close and
   the ciphersuite gets its independent review, Keyhive cannot be the
   enforcement layer for anything the estate actually needs to keep
   private — it can be the *model* (the graph semantics are implementable
   from the design docs + conformance suite) before it can be the *code*.
2. No expiry anywhere means the estate's short-lived-session patterns
   (Actions runners, leased exec) need a layer Keyhive doesn't provide —
   revocation-as-expiry works only when replicas sync promptly.
3. The root-role append-only caveat and the ex-admin rotation discipline
   are operational commitments: adopting Keyline means adopting its
   operational folklore (constitutional flatness, fresh-key discipline,
   rotate-after-admin-removal) as runbook, not just its API.
4. Bus factor: one small lab. The design docs are good enough to reimplement
   from (and the conformance suite makes that verifiable), which is the
   real insurance policy — but it should be priced into the decision.

**Net**: if the programme's question is "p2panda for everything, or
p2panda + a composed auth layer," Keyhive is the only candidate whose auth
layer was *built* to be composed this way, and its revocation semantics
are the best-thought-through in the field. The honest sequencing is:
adopt the *model* now as the reference for whatever auth the estate
builds or borrows; revisit the *code* when the audit/gap list moves.
