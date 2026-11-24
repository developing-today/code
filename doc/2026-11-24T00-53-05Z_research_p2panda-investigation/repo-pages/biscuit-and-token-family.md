# Biscuit and the token family — the survey's remaining pieces

Full lane notes: Part 2 of research lane CAPNP-SURVEY for
[#13779](https://github.com/developing-today/code/issues/13779), from
project docs, specs, repos, and crates.io, read 2026-11-23. The capnp half
of the same lane lives in [Cap'n Proto / Cap'n Web](capnp-capnweb.md);
UCAN, the deep dive the survey flagged as the most promising token format,
has its [own page](ucan.md).

## Biscuit — the deployable offline token

**What it is.** Biscuit is an authorization token format designed for
offline verification and decentralized attenuation: a chain of
public-key-signed blocks (Ed25519), where the root key signs the first
block and each block may _contain_ the public key that signs the next, so
attenuation is just appending a more restrictive block — no round trip to
the issuer, no shared secret
([biscuitsec.org](https://www.biscuitsec.org/),
[biscuit-auth on crates.io](https://crates.io/crates/biscuit-auth)). The
logic language is **Datalog**: blocks carry facts, rules, and checks;
the verifier evaluates the token's world together with its own policies
and denies unless a policy grants. Originally by Geoffroy Couprie
(Eclipse's Eclipse Biscuit programme now hosts it); the reference
implementation is `biscuit-auth` in Rust, with maintained bindings for
Java, Go, Haskell, Python, WASM, and C FFI. There is a public
specification, a web playground, and a CLI.

**Version/maturity at research time.** `biscuit-auth` 6.0 (September 2025) — a major release adding token inspection from a WASM component,
third-party block requests, and snapshot (de)serialization; the 5.x line
was steady through 2025. This is the most production-hardened artefact in
the whole survey lane: real semver, real changelog discipline, real
multi-language surface.

**Capability model.** Authority is data: the authority block states facts
(`right("file1", "read")`, `user("alice")`), later blocks add _checks_
that can only narrow (a check block must succeed for the token to verify;
there is no way to widen). Attenuation is **offline and unilateral** —
anyone holding a token can mint a strictly-weaker copy locally, which is
the property that makes Biscuit the natural fit for edge/gateway chains:
mint broad at the origin, narrow at each hop. **Expiry is idiomatic, not
structural**: the canonical pattern is a check comparing a verifier-supplied
`time()` fact against an expiration fact in the token — nothing forces an
author to include one, and there is no non-expiry sentinel rule; a
forgotten check is a non-expiring token. **Revocation is external and
explicit**: every block carries a unique revocation ID; the verifier
consults a revocation list (a set of IDs) and rejects on a hit. That is
the whole mechanism — a deny-list lookup, cheap and operationally boring,
which is a compliment: it composes with any storage you already run
(a SQLite table, a config push, a gossip topic). **Third-party blocks**
(5.x/6.x) let a _different_ key sign a block into a token — cross-domain
attestation ("the payment service confirms this session paid") without
sharing keys.

**Identity.** None, and that is by design: Biscuit is an authorization
format, not an identity system. A token _carries_ identity facts asserted
by its issuer (`user("alice")`); who "alice" is, how she authenticated,
and how her key relates to a transport peer are all outside the format.
There is no invocation object, no audience binding in the token itself —
a Biscuit is a **bearer token**: possession is use. (Audience/resource
scoping is conventionally enforced by checks against verifier-supplied
facts; that is policy discipline, not a cryptographic binding.)

**Algorithms.** Ed25519 only, per spec (the design leans on the
append-only signature chain). **No P-384, no algorithm agility** — same
estate caveat as elsewhere: Biscuit cannot be verified _by_ the P-384 CA
world; it lives below it, keyed by Ed25519 seat/issuer keys the CA
enrols.

**Offline/local-first.** This is Biscuit's home ground: verification is
pure local computation (Datalog evaluation with explicit resource limits —
facts/iterations/time caps are verifier-configurable, a real
DoS-resistance story); attenuation is offline; the only online-ish
dependency is revocation-list freshness, and the failure mode is the
usual one (stale list = recently-revoked token still verifies until the
list syncs — bounded by short TTLs).

**Cloudflare fit.** First-class: `biscuit-auth` compiles to WASM and
Eclipse Biscuit ships an official WASM build; Workers can verify and
attenuate tokens at the edge with the reference implementation, not a
port. Rust fit is the reference implementation itself. iroh fit:
orthogonal and clean — a Biscuit rides inside any message/stream as an
opaque blob; iroh supplies peer identity, Biscuit supplies portable
authority; the composition is a paragraph of application code.

**Boundary (what Biscuit does not provide):** identity/enrolment;
audience-bound exercise (bearer semantics); an invocation/receipt object;
groups (facts can _name_ groups; membership lifecycle is the issuer's);
encryption (it's a signed, readable token — contents are visible to
holders; do not put secrets in facts); transport; sync; storage.

**Estate verdict.** Biscuit is the survey's **deployable-now** answer for
offline-checkable, attenuable authority where bearer semantics are
acceptable and the verifier is ours (broker gates, edge checks, dispatch
leases). It loses to UCAN on binding/provenance and wins on maturity,
policy expressiveness (Datalog > UCAN's jq-subset for data-dependent
checks), and operational simplicity. The two are complements more than
rivals; see the head-to-head in the [UCAN page](ucan.md).

## Macaroons — the ancestor, and why not

Macaroons (Google, 2014) are the design Biscuit and UCAN both descend
from conceptually: bearer tokens attenuated by appending caveats, each
caveat narrowing the token, verified by chained HMACs. The caveat model
(including third-party caveats with discharge proofs) is genuinely
elegant and battle-tested inside Google/Uber-style infrastructures.
**Why not for this estate:** verification is **symmetric** — the verifier
must hold the root secret, so any verifier can also _mint_. In a
public-key estate (Ed25519 seats, a CA, third-party verifiers, offline
peers), that property is disqualifying for cross-party authority: you
cannot hand verification to parties you don't also trust with issuance.
Macaroons remain the right mental model and prior art; Biscuit is their
public-key successor for our purposes. (Rust: `macaroon` crates exist but
are thin next to `biscuit-auth`.)

## The p2p also-rans, honestly graded

Three systems from the p2p world were checked for capability designs
worth stealing:

**Holochain — the interesting one.** Holochain's capability system is
the most complete _capability_ design in the p2p set: **CapGrants**
(grantor-side records naming a grantee — by agent key, or _unrestricted_
— a set of functions on a zome, and a secret) paired with **CapClaims**
(grantee-side receipts), exercised by presenting the secret as a membrane
proof at call time; grants are revocable by deleting/updating the grant
entry, and because Holochain is a DHT of source chains, grant state is
data with provenance, not server state. Two estate-relevant facts:
(1) Holochain's networking layer (Kitsune2) is built on **iroh** as its
transport in current versions — the composition "capability membrane over
iroh" is not hypothetical, it is shipping architecture; (2) the
grant/claim _pair_ (authority recorded at both ends) is a design pattern
worth borrowing for auditability wherever we land. What Holochain is
not: a library you adopt for auth alone — the capability system is
welded to its runtime/DHT/zome model. **Mine the design; don't adopt the
platform.**

**Holepunch (Pear / Hypercore) — best sync model, wrong substrate.**
Autobase + signed append-only logs (Hypercore) give a genuinely elegant
multi-writer replication model, and its capability story is essentially
"possession of the log's keys + encryption keys as capabilities" —
simple, effective, coarse. The stack is JavaScript through and through;
there is no Rust path that isn't a rewrite. **Verdict: steal the sync
_model_ (signed logs, writer-set linearisation) if ever needed — it is
the model p2panda also follows — but there is nothing to depend on.**

**Freenet (Hyphanet/Locutus) — competes with iroh, loses.** Freenet's
contract/state model with capability-ish access via keys is a full
alternative _platform_ (its own networking, its own storage contracts),
not a layer. Against iroh for transport and p2panda for sync it offers
nothing this estate lacks, at the cost of a second platform. Noted and
set aside.

## SPIFFE — not a capability system, but two disciplines worth adopting

SPIFFE (with SPIRE as its implementation) issues short-lived identity
documents (SVIDs — X.509 or JWT) to workloads after node/workload
**attestation**: the workload proves what it is (cloud metadata, k8s
service account, process attributes) and receives a credential with a
TTL measured in minutes-to-hours, rotated automatically. It is
authentication infrastructure, not authorization: an SVID says _who_,
never _what they may do_.

Why it appears in this survey at all: two of its disciplines are the
estate's known weak spots.

1. **Short-TTL + automatic rotation as the default posture** — the same
   posture UCAN recommends and the fleet broker design assumes; SPIFFE
   is the proof it operates at industry scale.
2. **Attestation-based issuance is the one genuine capability gap in the
   house stack.** The fleet's runner-handoff problem (prove "I am inside
   _this_ GitHub Actions run" before receiving exec authority) _is_ an
   attestation problem — GitHub's OIDC token is an attestation — and
   SPIFFE's selector model (issuance policy = predicates over attested
   attributes) is the cleanest existing vocabulary for it.

**Verdict: adopt the ID format (`spiffe://…` URIs are a fine workload
naming scheme) and the issuance discipline; do not deploy SPIRE** — the
estate's CA + broker already occupy that operational niche, and SPIFFE
grants no authority by itself.

## ATProto's patterns — one borrowing

ATProto has no capability system (its delegated-authority story is OAuth,
covered in the [Bluesky page](bluesky-atproto.md)). What the survey flags
from it is the **identity key discipline**: a priority-ordered set of
rotation keys (PLC) separate from the everyday signing key, with
adversarial recovery semantics (a higher-priority key can override a
compromised lower one within a time window). That split — _signing keys
are cheap and replaceable; recovery keys are few, cold, and senior_ — is
directly borrowable into the estate's IdP/CA programme, where seat-key
compromise recovery is otherwise "the CA re-enrols and everyone
re-pins."

## Noise / WireGuard — the channel layer, already subsumed

Noise-protocol-framework handshakes and WireGuard appear in capability
surveys because channel binding _is_ a form of capability (reachability

- a session key = authority to talk). For this estate the point is
  settled: iroh's QUIC/TLS stack already provides authenticated encrypted
  channels keyed by the same Ed25519 identities everything else uses.
  Nothing to adopt separately; noted so the layer isn't double-counted in
  stack diagrams.

## Family summary

| Piece           | One-line verdict                                                                                                             |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Biscuit         | Deployable now: offline, attenuable, Datalog-checked bearer authority; revocation = deny-list; the broker-gate default.      |
| Macaroons       | Prior art; symmetric verification disqualifies it for a public-key estate.                                                   |
| Holochain caps  | Most complete capability _design_ in the p2p set, already composed over iroh (Kitsune2); mine the design, skip the platform. |
| Holepunch       | Best-in-class sync model, JS-only; model already shared with p2panda.                                                        |
| Freenet         | Alternative platform, not a layer; set aside.                                                                                |
| SPIFFE          | Adopt the ID format + short-TTL/attestation issuance discipline; don't deploy SPIRE.                                         |
| ATProto keys    | Borrow the rotation-key/signing-key split with adversarial recovery into the IdP/CA programme.                               |
| Noise/WireGuard | Channel layer; subsumed by iroh.                                                                                             |
