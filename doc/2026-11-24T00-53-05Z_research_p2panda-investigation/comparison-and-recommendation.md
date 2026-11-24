# Auth + transport investigation: comparison and recommendation

Lane SYNTHESIS, 2026-11-23, for the capability-auth + p2p-transport
investigation (tracking issue
[developing-today/code#13779](https://github.com/developing-today/code/issues/13779)).
Built entirely from the five research notes in
`~/workspace/p2panda-investigation/research/` (`p2panda.md`,
`keyhive-subduction.md`, `willow-meadowcap.md`, `ocap-family.md`,
`capnp-and-survey.md`), each written from primary sources; load-bearing
claims below trace to those notes and their citations.

**Executive summary.** The leaning "p2panda for everything" is right about
the data plane and wrong about the auth layer — and p2panda's own
documentation says so: p2panda-auth is a replicated group-membership state
machine, not a capability system, and the project names Meadowcap and UCAN
as the capability systems you compose in for delegated access. No candidate
in the field — named families or the wider survey — provides identity +
capabilities + sync + storage in one system; every credible stack is a
composition. The recommended default (§6) is therefore a composition:
p2panda's data plane, Meadowcap-as-spec authorisation for replicated data
with broker-run short-window renewal, and Biscuit tokens + capnp-over-iroh
for live sessions — with Keyline (Keyhive's auth model) adopted as the
reference design for revocation semantics and re-evaluated as code when
its disclosed gaps close.

---

## 1. The matrix

One row per family **layer** — several "families" are stacks whose layers
score very differently, and averaging them would hide exactly the facts
this decision turns on. Cells are deliberately terse; the notes carry the
evidence.

### 1.1 Named families, by layer

| Layer                                                | Capability model                                                                                                                                                                                 | Revocation                                                                                                                        | Groups                                                                     | Encryption                                                                                                                                                                                                                                                                                                                 | Sync model                                                                                                                      | Rust maturity                                                                                                | Cloudflare fit                                                        | SQLite/Turso fit                                                               | iroh fit                                                                                    | Production readiness                                                                          |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| **p2panda core + sync + net** (data/transport plane) | None — auth is separate crates                                                                                                                                                                   | n/a                                                                                                                               | Topics only                                                                | Channel only (iroh QUIC/TLS 1.3)                                                                                                                                                                                                                                                                                           | Single-writer logs; LogSync height-vector exact set difference; offline-first                                                   | Native; 0.8.1; lower layers ~2 yrs published                                                                 | None (native only, no WASM)                                           | SQLite yes (SQLx); **Turso no**                                                | **Native — it _is_ an iroh app**                                                            | Pre-1.0; breaking changes per minor; wire format evolving                                     |
| **p2panda-auth**                                     | Group-membership CRDT; 4 levels Pull<Read<Write<Manage + app-evaluated conditions; **no tokens, no delegation chains, no expiry**                                                                | Remove op + StrongRemove resolver: eventual, retroactive over concurrent work; no instant kill                                    | **First-class, nestable; groups as members**                               | None itself (drives spaces)                                                                                                                                                                                                                                                                                                | Rides the op logs (CRDT DAG)                                                                                                    | 0.8.1; first published 2025-07                                                                               | None                                                                  | Via p2panda-store                                                              | Via p2panda-net                                                                             | Pre-1.0; months old                                                                           |
| **p2panda-encryption + spaces**                      | n/a                                                                                                                                                                                              | Key rotation on remove (forward protection only)                                                                                  | Spaces bind auth groups to encryption contexts                             | DCGKA E2EE (X3DH+HPKE, XChaCha20-Poly1305); forward secrecy optional (Data scheme) / strong (Message scheme); **unaudited; group control messages unencrypted → membership metadata exposed; no post-quantum**                                                                                                             | Rides the logs                                                                                                                  | 0.8.1; spaces first published 2026-07, high-level integration landed 0.8.0                                   | None                                                                  | Via store (encryption/auth state persistence still "soon" in docs)             | Via net                                                                                     | Youngest layer; project: "cannot recommend for high-risk use-cases"                           |
| **Keyline** (Keyhive auth layer)                     | **Convergent capabilities**: delegation/revocation certificate CRDT; authority derived (RT₀/SDSI), widest-path attenuation; redundant routes; late binding; **no expiry, by design (no clocks)** | **Coordination-free and permanent — survives merges; best in the field.** Ex-admin keeps deny-only reach; escape is role rotation | Roles are keys; nesting, supply, device groups — same primitive throughout | None itself (drives BeeKEM)                                                                                                                                                                                                                                                                                                | State CRDT, set-union certificates; transport-blind                                                                             | `keyline` crate + conformance suite; **in-memory store only**                                                | None shipped (Wasm crate exists, unexercised in Workers)              | Designed-for (SQL CTEs + driver loop for the non-linear fixpoint); not shipped | Via Subduction                                                                              | **Pre-alpha** — but the design docs + conformance suite are the contract                      |
| **BeeKEM** (Keyhive group keys)                      | n/a                                                                                                                                                                                              | Remove = blank path; next update re-keys (post-compromise security)                                                               | Per-document CGKA trees; ~thousands of members; O(log n) typical update    | TreeKEM-variant CGKA needing only causal order; cross-fork security; content keys causal — **forward secrecy for history deliberately sacrificed**. Proofs: ePrint 2026/1434, **not yet peer-reviewed**. Code gaps: **G1 revoked member can seat a fresh own key (revocation bypass), G2 share keys unchecked vs prekeys** | Causal order only                                                                                                               | `beekem` 0.5.x                                                                                               | None                                                                  | n/a                                                                            | n/a                                                                                         | Pre-alpha, unaudited                                                                          |
| **Subduction** (Keyhive's sync sibling)              | Policy traits; Keyhive policy crate                                                                                                                                                              | None of its own — auth layer decides; sync stops forwarding                                                                       | None                                                                       | Payload-agnostic (carries E2EE); own Ed25519 mutual-auth handshake                                                                                                                                                                                                                                                         | Sedimentree set-union; 8-byte fingerprint reconciliation in 1.5 round trips; CRDT-agnostic                                      | 0.19.x; fortnightly releases                                                                                 | None; object-store backend makes R2 plausible                         | **No SQLite** (fs / redb / object-store)                                       | **Ships an iroh transport**                                                                 | Project's words: "early preview… DO NOT use for production"                                   |
| **Willow data model**                                | None — authorisation is a parameter (Meadowcap)                                                                                                                                                  | n/a                                                                                                                               | Namespaces/subspaces only                                                  | None built in                                                                                                                                                                                                                                                                                                              | State CRDT join; **current state only — no history, traceless deletion by design**; RBSR difference-proportional reconciliation | Canonical crates in 0.6 alphas; TS implementation most complete                                              | None                                                                  | **No** (sled / redb / fjall)                                                   | Conceptual fit; iroh-willow stale (iroh 0.34-era)                                           | **Spec Final**; Rust implementation research-grade                                            |
| **Meadowcap**                                        | **Receiver-bound area tokens** (subspace × path-prefix × time-range); attenuation cryptographically monotone (area inclusion); writes self-certifying, travel with the entry                     | **None.** Short time windows + non-renewal + supersession hygiene; the renewal service is yours to build                          | **None** (owned/communal namespaces are the only shapes)                   | None (prefix-preserving path encryption is a documented technique, not a protocol)                                                                                                                                                                                                                                         | n/a — authorisation parameter                                                                                                   | `meadowcap` crate; **spec Final 2025-11-21**, two independent implementations; const-generic/ufotofu flavour | Verification is pure computation; TS implementation runs in Workers   | n/a (tokens travel with entries)                                               | Keys are Ed25519; a subspace key can literally be a NodeId                                  | Spec-grade; no Rust sync ecosystem around it                                                  |
| **CapTP / OCapN** (protocol layer)                   | Live references; attenuation = facet objects (stateful, Turing-complete policy)                                                                                                                  | **Immediate** — interposition / session severance; no expiry concept                                                              | **None**                                                                   | Netlayer's problem                                                                                                                                                                                                                                                                                                         | **None — live sessions only; offline = broken promises**                                                                        | **Stage-0 single-author learning project**                                                                   | None                                                                  | None                                                                           | None (a netlayer over iroh is conceivable, unbuilt)                                         | **Pre-specification; wire broke compatibility in 2026**                                       |
| **Endo / SES** (JS runtime layer)                    | Ocap in JavaScript; petname daemon; formula retention                                                                                                                                            | Petname deletion → formula GC cancels values, severs connections (live)                                                           | None                                                                       | None                                                                                                                                                                                                                                                                                                                       | None                                                                                                                            | None (JS)                                                                                                    | SES can run in Workers in principle; no CapTP Workers story           | None                                                                           | None                                                                                        | SES production-proven (MetaMask); `@endo/ocapn` 1.1.0 with CI interop vs Goblins              |
| **Goblins** (Scheme runtime layer)                   | Most complete CapTP: handoffs, distributed acyclic GC                                                                                                                                            | Live, by interposition                                                                                                            | None                                                                       | Netlayers: Tor; Prelay v0 forwards **cleartext**; Noise E2EE relay announced, unreleased                                                                                                                                                                                                                                   | None (Aurie/Bloblin persist one node's own graph — not replication)                                                             | None (Guile/Racket)                                                                                          | None (Hoot→Wasm exists)                                               | None (Syrup/Bloblin stores)                                                    | None (libp2p netlayer least exercised)                                                      | "Working Beta", v0.18.0                                                                       |
| **Agoric / SwingSet** (platform layer)               | CapTP into a chain via the comms vat; ERTP rights-as-objects, Zoe offer safety                                                                                                                   | Live, in-vat                                                                                                                      | None                                                                       | The chain's                                                                                                                                                                                                                                                                                                                | Consensus as netlayer — the exception that proves the live-only rule                                                            | None (JS / Cosmos)                                                                                           | None                                                                  | None                                                                           | None                                                                                        | Production chain; value to this estate is patterns, not adoption                              |
| **Cap'n Proto RPC** (Rust)                           | **Level 1 only**: connection-scoped references + promise pipelining; attenuation by facet object                                                                                                 | Membrane pattern (you build it); disconnect revokes incidentally                                                                  | None                                                                       | **Unspecified** — bring TLS/Noise/iroh                                                                                                                                                                                                                                                                                     | None                                                                                                                            | `capnp-rpc` real and used; Level 1; single-maintainer lineage                                                | workerd _is_ Cap'n Proto (C++); the Rust crates have no Workers story | Serialisation is a fine signable blob format; no store                         | **Composable**: runs over any byte stream; iroh supplies the identity + encryption it lacks | Level 1 proven at Cloudflare scale (C++); Rust stable-but-thin                                |
| **Cap'n Web**                                        | Ocap stubs in JS; pipelining; in-band `authenticate()` pattern                                                                                                                                   | Session lifetime; membranes yours                                                                                                 | None                                                                       | HTTPS/WSS (platform)                                                                                                                                                                                                                                                                                                       | None                                                                                                                            | One 0.1.0 sketch crate                                                                                       | **First-party Workers citizen**                                       | None                                                                           | None                                                                                        | Experimental (Sep 2025); JS-only in practice                                                  |
| **UCAN**                                             | **Deepest token model**: mandatory monotone attenuation, policy language, **invocation as a separate signed object**                                                                             | Reserved-namespace revocation exists; **propagation to validators explicitly out of scope**                                       | None (principals/subjects only)                                            | None                                                                                                                                                                                                                                                                                                                       | None                                                                                                                            | **1.0 not ready**: `rs-ucan` is pre-1.0 JWT; 1.0 (DAG-CBOR) only in third-party pre-release crates           | Verification portable; no official WASM story                         | Tokens are CIDs/blobs; a deny-list is trivial                                  | **`did:key` Ed25519 is NodeId-shaped — exact fit**                                          | Spec 1.0.0 untagged on main; production lineage is JS/Go (Storacha)                           |
| **Biscuit**                                          | Bearer tokens; Datalog checks; offline attenuation; third-party blocks                                                                                                                           | Per-block revocation IDs + verifier deny-list (a SQLite table in this estate); expiry idiomatic via time checks                   | None (roles encodable as facts)                                            | None (public-key verification; Ed25519 default, P-256 ok, **P-384 not**)                                                                                                                                                                                                                                                   | None                                                                                                                            | **Reference implementation** (`biscuit-auth` 6.0)                                                            | **Official WASM build** — Workers verification realistic today        | Deny-list is a native fit                                                      | Bind to NodeId via a check (thin house layer)                                               | Eclipse project; production use; two breaking third-party-block security fixes in its history |

### 1.2 Wider field, same axes (survey note, compressed)

| Layer                  | Capability model                                                                                           | Revocation                       | Groups                            | Encryption                                          | Sync model                                                                              | Rust                                | Cloudflare         | SQLite/Turso                    | iroh                                             | Readiness                                                                 |
| ---------------------- | ---------------------------------------------------------------------------------------------------------- | -------------------------------- | --------------------------------- | --------------------------------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------- | ------------------ | ------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------- |
| Macaroons              | Caveat chains — the origin of offline attenuation                                                          | Short lifetimes only             | None                              | **Symmetric-only verification (deal-breaker here)** | None                                                                                    | Crate unmaintained                  | DIY WebCrypto only | n/a                             | n/a                                              | Prior art, not a dependency                                               |
| Veilid                 | DHT record ownership; route blobs as bearer caps                                                           | Weak                             | None                              | Privacy routing (its differentiator)                | DHT store, no merge semantics                                                           | Rust-first, 0.5.x pre-1.0           | None               | Own DHT                         | **Competes with iroh**                           | Beta; no verified large production                                        |
| Hypercore / Holepunch  | Log key = identity; encrypted-core key = read cap                                                          | App-level                        | Autobase multi-writer             | Encrypted cores                                     | **Signed single-writer logs + sparse replication — strongest sync model in the survey** | **JS-only** (`datrs` unofficial)    | None               | Own stores                      | Competes                                         | Production (Keet); funded, single-backer                                  |
| libp2p                 | Key possession only                                                                                        | n/a                              | None                              | Noise / TLS 1.3 channels                            | None (gossipsub is broadcast, not sync)                                                 | Mature (0.57, pre-1.0), heavyweight | Browser transports | n/a                             | Duplicates iroh's job; bridge is 0.1.x WIP       | Production networks; keep as interop escape hatch                         |
| Freenet                | Contract WASM _is_ the validity policy; delegates hold secrets                                             | Contract-level                   | None                              | X25519+AEAD transport                               | Contract state as commutative monoid; delta sync                                        | Rust 0.2.x alpha; **AGPL-3.0**      | None               | Network-hosted state (anti-fit) | Competes                                         | ~440-peer live network                                                    |
| Holochain              | **CapGrant/CapClaim — the only complete capability system in the p2p set**; membrane proofs gate admission | Delete the grant                 | Agent-centric; no group primitive | App-level                                           | Validated DHT; no merge semantics                                                       | Conductor mature-ish, pre-1.0       | None               | DHT                             | **Runs on iroh today (Kitsune2 transport)**      | A decade of churn; framework adoption cost is the price                   |
| AT Protocol (patterns) | Thin (PDS OAuth/JWT)                                                                                       | n/a                              | n/a                               | n/a                                                 | MST repository + CAR export                                                             | Community Rust (Atrium)             | n/a                | n/a                             | n/a                                              | Production at Bluesky scale — borrow the key split (§4), not the protocol |
| Noise / WireGuard      | Identity only                                                                                              | Config edits                     | None                              | **The channel layer itself; forward secrecy**       | None                                                                                    | `snow` / `boringtun` strong         | n/a                | n/a                             | Subsumed by iroh's QUIC/TLS in the target design | Production everywhere                                                     |
| SPIFFE / SPIRE         | SVIDs assert identity only                                                                                 | Short TTL (~1 h) + auto-rotation | Trust domains                     | mTLS via SVIDs                                      | None                                                                                    | Client-only; SPIRE itself is Go     | n/a                | n/a                             | n/a                                              | CNCF graduated — adopt format + discipline (§4), not the control plane    |

**Reading the matrix.** Three structural facts fall out of it:

1. **The axes split.** Nothing scores well on both the _replicated-data_
   columns (sync, groups, offline) and the _live-authority_ columns
   (immediate revocation, stateful attenuation). The log families and the
   ocap families are answering different questions (§2 of the ocap note),
   and each is structurally weak where the other is strong.
2. **Revocation is the discriminating axis.** Exactly three designs have a
   real revocation story for replicated/offline settings: Keyline
   (coordination-free, permanent), p2panda-auth (eventual, retroactive
   over concurrent work), and short-window non-renewal (Meadowcap's
   answer, and every token system's practical answer). Everything else
   either needs a live session (ocap/capnp) or a deny-list (Biscuit).
3. **Maturity is inverse to novelty.** The deployable-today pieces
   (Biscuit, capnp Level 1, p2panda's lower layers, SPIFFE discipline) are
   the least ambitious; the most ambitious auth designs (Keyline/BeeKEM,
   UCAN 1.0) are pre-alpha or pre-implementation in Rust. Any honest
   recommendation is a sequencing, not a coronation.

---

## 2. The verdict on "p2panda for everything"

### 2.1 The honest case for

- **The transport bet is already made, and p2panda is it.** The estate's
  recorded transport design (#13752) chose iroh — NodeId identity,
  self-hosted relays, hole punching. p2panda-net does not merely compose
  with iroh; it _is_ an iroh application (endpoint, gossip, blobs), and
  adds the three things raw iroh lacks: exact log sync, gossip overlays,
  and confidential topic discovery (peers prove shared knowledge of a
  topic without revealing it). Adopting p2panda's data plane adopts no
  new transport risk.
- **The shape fits the fleet's topology.** Seats on laptops and sandboxes
  that meet opportunistically, publish locally, and converge on contact
  is precisely what LogSync + gossip were built for; reconciliation is an
  exact set difference (single-writer logs with contiguous sequence
  numbers), not a heuristic merge.
- **Groups map onto the estate without forcing.** Manage / Write / Read /
  Pull, nested groups, and groups-as-members cover operator keys, seat
  keys, observers, and dumb relays; per-device groups model multi-device
  identity the way the project intends.
- **E2EE for replicated data exists today**, in a published crate — DCGKA
  group encryption wired to group membership via spaces. Neither raw iroh
  nor the current tsnet setup provides encryption for _stored,
  replicated_ content at all.
- **Every hard preference is natively met by the lower stack:** Rust,
  SQLite, MIT/Apache-2.0, self-hostable relays, no server component, and a
  codebase small enough (~60k lines) to fork if the three-person,
  grant-funded team stalls.

### 2.2 The honest case against

- **p2panda-auth is not a capability system, and the project says so.**
  It is a group-membership CRDT with four access levels and
  application-evaluated conditions. There are no delegation chains, no
  bearer credentials, no caveats, no expiry, and no scoped one-shot
  grants ("this runner may read artifact X for ten minutes" has no native
  expression). The project's own high-level docs name Willow's Meadowcap
  and UCAN as external capability systems to integrate via header
  extensions "for permission checks and delegated access control" — i.e.
  delegated capability auth is, by the authors' own framing, something
  you compose in. "p2panda for everything" fails on this axis not by our
  judgment but by the project's documentation.
- **Pre-1.0, and the young layers are the ones we'd lean on.** Every crate
  README warns APIs "are not yet considered stable for production use";
  minors carry breaking changes and the wire format is still evolving.
  The lower layers have ~2 years of history; auth and encryption are
  4–16 months old, and spaces — the integration of the two — landed in
  the high-level API in 0.8.0 (2026-11-09) with a visible tail of
  deadlock/flaky-test fixes in the same changelog.
- **The encryption is unaudited and leaks metadata.** The encryption
  crate's own security section: "This crate has not yet received a
  security audit"; "We currently _cannot_ recommend using this technology
  for high-risk use-cases"; group control messages are currently
  **unencrypted**, so membership metadata (who is in which group) is
  observable to anyone who can see the traffic; and there is no
  post-quantum story (harvest-now-decrypt-later exposure stands).
- **Broker-style coordination fights the single-writer-log grain.** The
  fleet's centre of gravity is a broker: queue claims, placement slots,
  leases — inherently _multi-writer_ contention over shared state.
  p2panda pointedly has no server, no RPC, no request/response semantics;
  modelling claims and leases as replicated state over single-writer logs
  imports exactly the contention semantics the broker exists to avoid.
  The estate's own compile-coordinator design (#13765) already chose
  atomic conditional updates in the broker's store as the house
  primitive; "everything" would have to un-choose that.
- **No edge, no Turso, no Go.** Workers cannot host it (no UDP/QUIC
  listeners), the current generation has no browser/WASM support, the
  only shipped store is SQLite-via-SQLx (a libsql backend is ours to
  write and maintain), and mcpx (Go) could consume it only via
  experimental FFI or a sidecar. The Cloudflare doors would stay
  arm's-length bridges forever — acceptable, but it means "everything"
  never actually covers the estate's edge or its Go service.
- **Identity is raw keys, full stop.** No PKI/CA integration, no naming,
  no rotation or recovery. p2panda identities would live _alongside_ the
  fleet CA / interim IdP CA, not under them; nothing in p2panda consumes
  a certificate. Two identity systems is a permanent operational fact of
  this option, not a migration phase.

### 2.3 Net verdict

**"p2panda for the data plane" is defensible today, with eyes open.
"p2panda for everything" is not** — it fails on delegated auth (by the
project's own framing), on live coordination (by the model's grain), and
on the edge (by platform fact). The interesting question was never
purity; it is which composition covers the estate's five surfaces —
fleet seats/broker, Actions runners, Cloudflare doors, mcpx, atlas/id
data — with the least invented machinery. §3 gives three compositions
that together do.

---

## 3. The composed alternatives

Compositions A and B are alternative answers to the same question —
_authorisation for replicated data_ — and are the real fork. Composition
C answers a different question — _authority inside live sessions_ — and
is needed under either A or B, because the estate is already building
live-session surfaces (coordinator-handed-off Actions-runner exec,
#13774; placement claims, #13765) that neither A nor B addresses.

### 3.A p2panda data plane + Meadowcap-as-spec authorisation + broker short-window renewal

**Shape.** p2panda core/sync/net/store as the data plane, unchanged.
Meadowcap's validity rules are adopted **as a specification** — either
the `meadowcap` crate or a house re-implementation against the Final
spec, checked against the two existing implementations — and bound to
p2panda as a header-extension authorisation token: an _area_ is a
p2panda topic × path-prefix × time window; the owned-namespace root key
is the estate's root of authority (Meadowcap is generic over signature
schemes, so an Ed25519 root aligned with NodeIds — or a P-384
instantiation aligned with the interim IdP CA — is spec-legal either
way; that choice is decision point 4 in §5). p2panda-auth is demoted to
what it is good at: replicated _membership_ state. The fleet broker runs
the renewal service Meadowcap's model requires: seats receive
short-window capabilities at check-in; revocation is non-renewal plus
supersession hygiene writes.

- **What it buys.** Receiver-bound, offline-verifiable, self-certifying
  write authorisation that travels with the data; attenuation that is
  cryptographically monotone (widening is unverifiable, not merely
  against policy); scoped one-shot grants as a native pattern (a runner
  gets an area+window, nothing else); a **Final** spec with two
  independent implementations to test against — the lowest
  design-risk capability layer in the field; and alignment with
  p2panda's own documented composition story.
- **What it costs.** Revocation latency equals window length — bounded
  by broker check-in cadence, which is short for seats and wrong for a
  peer that goes offline for months (the tension is Meadowcap's own,
  stated in its design essays). Delegation is always transitive (no
  depth caps, no non-transferable grants in the spec). Read enforcement
  is cooperative — peers must honour read capabilities; data already
  replicated is already read. Per-entry chain overhead (area + key +
  signature per delegation step) rides every authorised operation
  forever. Timestamp discipline becomes security-critical operational
  policy, because capability windows bind _claimed_ timestamps.
- **What remains unbuilt.** The Meadowcap↔p2panda extension binding; the
  renewal service in the broker; the signature-scheme instantiation
  decision; clock/future-dating policy; and the coherence rules between
  membership (p2panda-auth) and grants (Meadowcap) so the two never
  disagree about a seat.
- **Surfaces.** Fleet seats/broker: strong. Atlas/id data: strong —
  areas map directly onto dataset slices. Actions runners: good —
  short windows match lease lifetimes. Cloudflare doors: verification
  is pure computation; the TS implementation runs in Workers today.
  mcpx: moderate — a Go cross-check implementation of Meadowcap exists,
  but integration is ours.

### 3.B p2panda data plane + Keyline-model auth (adopt the design, not yet the code) + BeeKEM-shaped group keys

**Shape.** Same p2panda data plane. Authorisation is a house
implementation of **Keyline's semantics** — delegation/revocation
certificate sets evaluated as derived authority — built from Ink &
Switch's design documents and verified against the `keyline` crate's
conformance suite ("where this document and the conformance suite
disagree, the suite wins"). Subjects are p2panda topics/documents; seats
and devices are individuals grouped under seat/person roles; the
Cloudflare doors and bridges hold the **`Relay`** power — inside the
authorisation graph, able to store and forward ciphertext,
cryptographically unable to read. Storage is two append-only SQLite
tables; evaluation follows the implementers' own notes (SQL CTEs for the
passes that are declarative, a driver loop for the non-linear fixpoint).
Group keys follow BeeKEM's _shape_ (per-document trees, blank-on-remove,
conflict keys) as the design reference; until someone builds or audits
that layer, group E2EE stays on p2panda-encryption with its unaudited
status explicit. Subduction is not adopted — p2panda-sync stays.

- **What it buys.** The only **coordination-free, permanent revocation**
  in the field: once any replica applies a revocation, no merge can
  un-apply it — the property Meadowcap structurally lacks and
  p2panda-auth's StrongRemove only approximates retroactively.
  Redundant routes mean no brittle proof chain whose invalidation
  strands a delegate. Late binding means supplying a role with a new
  subject grants it to all members with no re-issuance. Groups, roles,
  device collectives, and documents are one primitive, not four.
  `Relay` is a direct hit on this estate's door architecture: today the
  doors hold ambient trust; under Keyline they would hold a _power in
  the graph_, auditable like everyone else.
- **What it costs.** We write and maintain a security-critical
  authorisation engine. It is bounded (polynomial evaluation; the
  reference answers `members()` in ~1 ms at 180 members) but it is ours
  to get right, with the conformance suite as the only net. **No expiry
  exists anywhere in the model** — short-lived authority must be
  modelled as prompt revocation, which only bites when replicas sync;
  frequently-checking-in runners mitigate, genuinely-offline peers
  don't. The operational folklore is part of the adoption:
  constitutional flatness, fresh-key discipline after compromise,
  rotate-after-admin-removal (an ex-admin's deny-only reach is forever),
  and a root role that is append-only by construction — "treat it like
  a root CA." BeeKEM-shaped keys are a design commitment, not a
  deliverable: adopting the _code_ today would import G1 (a revoked
  member can delegate to a fresh own key and get it seated — a live
  revocation bypass in the CGKA path) and G2 (share keys unchecked
  against prekeys), which is precisely why this composition adopts the
  model and defers the code.
- **What remains unbuilt.** The Keyline engine + SQLite store +
  conformance runner; the binding from Keyline subjects to p2panda
  topics and to broker decisions; the CGKA decision (DCGKA now vs
  BeeKEM later); timestamping/anchoring where back-dating matters (the
  threat model is explicit that Keyhive cannot distinguish a late old
  operation from a fabricated back-dated one without one).
- **Surfaces.** Fleet seats/broker: strong for replicated state (the
  broker still enforces its own live decisions). Atlas/id data: strong.
  Cloudflare doors: **the single best fit in the investigation** (Relay).
  Actions runners: weaker — no expiry; depends on prompt broker
  revocation and runner check-in. mcpx: moderate — policy checks at the
  bridge; a Go implementation would be ours.

### 3.C The live-session layer, whichever of A/B wins: Biscuit tokens + capnp-over-iroh

**Shape.** Where the interaction is a _live session_ rather than
replicated data — Actions-runner exec sessions, the compile
coordinator's handoff, seat↔broker RPC — the stack is: an **iroh stream**
as transport (identity = NodeId, encryption = QUIC/TLS 1.3); **Cap'n
Proto RPC Level 1** over that stream for live, facet-attenuated calls
with promise pipelining; revocation by **interposition** (membrane /
caretaker facets) plus disconnection; and **Biscuit tokens** as the
expiring, attenuable, offline-checkable credential that authorises
_opening_ the session — minted on the broker/CA side, bound to the
holder's NodeId by a Datalog check, revoked via the per-block
revocation-ID deny-list kept in SQLite. Cap'n Web is the same idea at
the JS edge, adopted only where JS is native (Workers/browser surfaces).
UCAN is the semantic reference and the migration watch-item: its
invocation separation and `did:key` (= NodeId) identity are strictly
nicer than Biscuit's bearer shape, but conformant UCAN 1.0 Rust does not
exist yet, its revocation propagation is out of scope by spec, and its
suite excludes P-384.

- **What it buys.** Immediate revocation for live authority — the ocap
  property token systems structurally cannot retrofit — exactly where
  immediacy matters (a leased runner session, a dispatch facet).
  Expiring offline tokens for session establishment, which is the thing
  capnp pointedly lacks (§1.8 of the capnp note) and Keyline refuses on
  principle. A token implementation that is the **Rust reference
  implementation with an official WASM build**, so the same credential
  verifies in the fleet binary and inside a Worker door. And a direct
  match with designs already in flight: the coordinator handoff in
  #13765/#13774 is a CapTP-shaped introduction (gift, bind, connect
  directly) whether or not it uses CapTP's wire format.
- **What it costs.** capnp-rpc is Level 1 only, single-maintainer
  lineage: no sturdy refs, no library handoffs — the handoff protocol is
  our work at the coordinator. Biscuit is a bearer token; NodeId binding
  is a house layer on top. Membrane discipline is a coding standard, not
  a library feature — facets must actually be written attenuated.
  Two more formats to govern (capnp schemas; Biscuit Datalog policy).
- **What remains unbuilt.** The session-opening protocol (Biscuit →
  capnp bootstrap capability over an iroh stream); the facet/membrane
  library for broker surfaces; the deny-list service; the Cap'n Web
  evaluation for edge session surfaces.
- **Surfaces.** Actions runners: this is its home turf — leased,
  short-lived, coordinator-handed-off exec. Fleet seats/broker live RPC:
  strong. Cloudflare doors: strong (Biscuit verifies in-Worker; Cap'n
  Web is first-party there). mcpx: moderate (Go capnp exists but is
  alpha-versioned; mcpx more likely consumes through its existing
  door/bridge shape). Atlas/id data: not its layer — data-plane auth
  stays A or B.

---

## 4. Ideas to borrow regardless of choice

These pay off under every composition above, including "change nothing
this quarter."

1. **Revocation by interposition (ocap family).** Never hand a delegate
   the raw authority; hand a forwarder/caretaker whose forwarding can be
   switched off. It works in any RPC system — including a Rust one over
   iroh — and yields immediate, propagation-free revocation, the
   property token systems structurally lack. Endo's daemon shows the
   retention-based variant: authority dies when its last name does.
2. **Petnames (ocap family).** The human/agent-facing layer for
   authority should be local, meaningful names bound to unguessable
   identifiers — never raw hashes or tokens in prompts, logs, or UIs.
   This is the estate's existing "secrets by name only" discipline,
   generalised to capabilities; Endo's pet daemon is the existence
   proof that the UX works.
3. **Handoff certificates (ocap family).** When A introduces B to C,
   design for a direct B↔C connection with the introduction
   cryptographically bound (CapTP's Gifter/Receiver/Exporter roles are
   the cleanest vocabulary) — not permanent proxying through A, and not
   "send C's token to B." Directly applicable to the coordinator
   handoff in the Actions-exec and placement work.
4. **The netlayer contract as a checklist (ocap family).** OCapN's
   transport requirements — bidirectional, ordered, session-secure,
   designator = self-authenticating key, routing hints separate from
   identity — are a sharp evaluation checklist for the estate's own
   route-form/transport candidates. iroh satisfies it almost by
   definition, which is a useful confirmation of the #13752 direction.
   (Secondary borrows from the same note: promise pipelining as an
   API-design rule — "send the plan, not the steps"; SES/Hardened-JS
   confinement thinking for the JS that already runs at the edge;
   formula/retention GC as the lifecycle metaphor for seats, sessions,
   and leases.)
5. **ATProto's key-split recovery (survey).** Split identity into
   _rotation keys_ (few, priority-ordered, held in escrow) that control
   the identity record and can replace signing keys and repoint the
   host **even if the old host is offline or uncooperative**, and
   _signing keys_ that only sign data. For seat identity this is
   strictly better than "the CA reissues everything": a lost or hostile
   seat is recovered by rotation-key action, with the PLC-style
   operation log as the audit trail. It composes with the estate's
   existing escrow pattern (BWS + 1Password, tested restoration) —
   rotation keys are exactly the class of material that escrow is for.
6. **SPIFFE discipline without SPIRE (survey).** Adopt the ID format
   (`spiffe://<trust-domain>/<path>` URIs) and the short-TTL,
   auto-rotation policy (~1 h) now: a house CA issuing short-lived
   X.509 with a URI SAN, plus the existing OIDC provider issuing JWTs
   with the same URI as `sub`, already _is_ an SVID system in substance.
   Deploy SPIRE itself only when external interop (Istio/Envoy, cloud
   workload-identity federation) or attestation becomes concrete.
   Name the genuine gap honestly: **attestation** — minting identity
   from observed evidence (kernel/platform selectors) with no stored
   secret — is the one identity capability the house stack lacks, and
   neither p2panda nor any candidate in this investigation supplies it
   either.

---

## 5. Decision points that remain Drew's

Stated as choices with consequences. The evidence narrows these; it
cannot settle them, because each turns on risk appetite, sequencing, or
how much machinery the estate wants to own.

1. **Revocation posture for replicated data.** (a) _Short windows +
   broker renewal_ (composition A): revocation latency is bounded and
   predictable, but authority depends on a live renewal path — a
   partitioned seat keeps its window. (b) _Coordination-free permanent
   revocation_ (composition B): revocation survives any merge, forever,
   offline — at the price of owning the engine and its folklore.
   (c) _Accept p2panda-auth as-is_: least new machinery; revocation is
   eventual and retroactive, and delegated/scoped grants stay
   unexpressible. This choice decides how a compromised seat's authority
   actually dies across the estate.
2. **How much security-critical code the estate owns.** Adopt pre-1.0
   upstream code (p2panda spaces/DCGKA; Keyhive with G1/G2 open),
   implement a Final spec in-house (Meadowcap), implement a design-doc
   model in-house (Keyline), or buy boring maturity (Biscuit, capnp
   Level 1). Each option trades audit surface, upstream bus factor, and
   schedule differently; no measurement in the notes prices that trade.
3. **E2EE ambition for replicated fleet/atlas data.** Ship on unaudited
   DCGKA now; wait for an audit or BeeKEM's maturation; or keep
   replicated data channel-encrypted only (iroh/TLS) and skip the
   data-layer E2EE entirely. Consequences either way: membership
   metadata is exposed until control-message encryption lands, and
   harvest-now-decrypt-later exposure stands until a post-quantum story
   exists. What the data _is_ decides how much this matters, and that
   valuation is Drew's.
4. **Identity binding, and the curve question.** Bind author/NodeId keys
   to the fleet CA / IdP CA (certificates attesting key ownership — a
   bridge the estate builds; no candidate consumes X.509), or run
   raw-key identity with the CA as a parallel system indefinitely.
   Entangled with it: Ed25519 everywhere (the p2p/iroh/UCAN world's
   native curve) vs the interim IdP CA's P-384 (spec-legal for
   Meadowcap, excluded from UCAN's suite, unsupported by Biscuit).
   The curve chosen for the token layer constrains which token layers
   exist.
5. **Whether any domain earns the Willow data model.** Mutable, named
   state with traceless deletion fits infrastructure state (registries,
   configs, seat state) arguably better than append-only logs — but
   adopting Willow anywhere means a second data plane and a
   house-written Rust sync, because none exists. Whether a domain's
   mutability needs justify that is a product call, not an evidence
   call.
6. **Turso convergence.** Accept per-node SQLite beside the Turso
   stores (p2panda's shipped shape; two replication stories for fleet
   data), or fund a libsql backend for p2panda-store as estate-owned
   code with an upstream-shaped maintenance tail.
7. **Upstream engagement posture.** Every serious candidate is a tiny,
   grant-funded team (p2panda: three people; worm-blossom: two; Ink &
   Switch: a handful). Depending on their roadmaps without funding,
   contributing, or forking is itself a choice; so is the opposite.
   (One hard fact for this item: the OCapN project prohibits
   AI-generated contributions to its specs and tooling by policy — any
   engagement there is human-authored only.)

---

## 6. Recommended default

**RECOMMENDATION** (Archer's, under the standing decision posture;
recorded here for ratification or override):

> **Adopt composition A + C now; hold B as the tracked design north
> star.**
>
> - **Data plane:** p2panda core/sync/net/store — the leaning was right
>   here; it is the iroh-native, Rust, SQLite, offline-first stack the
>   estate already chose the transport half of.
> - **Authorisation for replicated data:** Meadowcap **as a
>   specification**, bound to p2panda via header extensions, with the
>   broker running short-window renewal; p2panda-auth kept as the
>   membership layer it actually is; p2panda-encryption used with its
>   unaudited status explicit (decision point 3 governs how far).
> - **Live sessions:** Biscuit tokens for session establishment +
>   capnp-over-iroh Level 1 facets with interposition revocation
>   (composition C), serving the Actions-exec and coordinator surfaces
>   already in flight.
> - **Design reference:** Keyline's model is adopted as the estate's
>   reference semantics for revocation and relay power — the standard
>   our own mechanisms are judged against — without adopting Keyhive's
>   code while G1/G2 stand.

**Reasoning, in one paragraph.** The estate's centre of gravity is a
broker with frequently-checking-in seats, replicated operational state,
Cloudflare doors, and a growing live-session surface. A is the only
composition whose capability layer is a Final spec designed to be
borrowed, whose revocation weakness (no revocation, only non-renewal)
is precisely the one this estate can patch cheaply because its seats
already check in. B has strictly better revocation semantics but exists
as pre-alpha code carrying a disclosed revocation bypass in its key
seating path; adopting its _model_ now and its _code_ later captures the
ideas without betting the estate on them. C is not optional garnish:
the live-session surfaces are being built this month, and neither A nor
B says anything about them. p2panda-for-everything, by contrast, asks
the weakest layers of a pre-1.0 stack (auth, encryption integration) to
carry the estate's most security-critical weight, against the project's
own documentation.

**What evidence would change this recommendation.**

- **Keyhive closes G1/G2** and the ciphersuite construction gets its
  independent review (or a full audit lands): composition B's _code_
  becomes adoptable, and Keyline-as-code likely displaces
  Meadowcap-as-spec — better revocation, native Relay, one auth system
  instead of two.
- **p2panda reaches 1.0** with wire-format stability, an encryption
  audit, and encrypted control messages: the data-plane bet strengthens
  materially, and if its capability story grows teeth in the process,
  the auth composition re-opens.
- **A conformant UCAN 1.0 Rust implementation** (working-group or
  third-party) reaches stability: UCAN likely displaces Biscuit in C
  (invocation separation; `did:key` identity) and pressures Meadowcap
  in A.
- **Willow's Confidential Sync unpauses** and a maintained Rust sync
  implementation appears (or iroh-willow is ported to current iroh):
  the data-plane question re-opens for mutable-state domains
  (decision point 5).
- **BeeKEM's paper survives peer review** and a second implementation
  appears: the group-key layer decision re-opens independently of the
  auth decision.
- **OCapN freezes a 1.0 wire version** and a real Rust implementation
  reaches test-suite parity: the live-session layer could standardise
  on OCapN CapTP instead of capnp Level 1. (Near-term probability, per
  the ocap note's evidence: low.)

---

_Sources: the five research notes in `research/` are the evidence base;
every load-bearing claim above traces to a claim in those notes and its
primary-source citation. This document is the investigation's synthesis
deliverable under the master prompt in
`p2panda-program-investigation-prompt.md`._
