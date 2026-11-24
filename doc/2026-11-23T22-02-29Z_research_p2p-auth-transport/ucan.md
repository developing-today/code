# UCAN — the deepest capability-token semantics, with principals that are our keys

Full lane notes: research lane UCAN-DEEP for
[#13779](https://github.com/developing-today/code/issues/13779), from the
UCAN Working Group spec repos (core, delegation, invocation, revocation),
the implementation repos (rs-ucan, go-ucan, ucanto), third-party 1.0 work
(pq-ucan, ucantone), and production users' own documentation (Storacha).
Read 2026-11-23.

## Verdict up front

UCAN 1.0 is the most completely specified _delegation semantics_ in this
programme: a required delegation sub-spec with mandatory monotonic
attenuation, a separate signed **invocation** object (authority vs its
exercise, with receipts and provenance chains), a revocation model that is
per-proof-path and irreversible, and principals that are bare `did:key`
identities — for Ed25519, mechanically the same key as an iroh NodeId and
a p2panda author key. It is the only candidate in the survey whose
identity layer is _free_ for this estate.

The blocker is not the design, it is the code and the calendar. The spec
family reached 1.0.0 text in 2025–2026, but the ecosystem is
mid-transition: the WG's own Rust crate self-describes as implementing
**UCAN v1.0.0-rc.1** and "work in progress… not formally audited"; the one
Rust crate implementing the final 1.0.0 tags is a third-party pre-release;
the revocation sub-spec is still at **v1.0.0-rc.1**; and the largest
production deployment (Storacha's ucanto) still runs **0.10-era
semantics**, not 1.0. Judgment (end of page): adopt UCAN as the estate's
delegation-layer _design_ now; as a code dependency it is a
compatibility/pilot layer today, with a named trigger for promotion.

## What UCAN is, and the spec family as it stands

UCAN (User Controlled Authorization Networks) is a **certificate-capability**
system in the SPKI/SDSI lineage: authority is carried by signed,
serializable tokens that form chains, verifiable offline by anyone, with
no central authority consulted at exercise time. The core spec is explicit
that this is _not_ an object-capability system — ocap's consistency
requirements "rule out partition tolerance," and UCAN chooses partition
tolerance ([core spec](https://github.com/ucan-wg/spec)). It originated at
Fission and was used in production there in its 0.9-era JWT form; Fission
the company shut down in April 2024 and the work continues under the UCAN
WG.

The 1.0 family is four sub-specifications under one core document:

| Sub-spec   | Requirement  | Status (2026-11-23)                                                                                     |
| ---------- | ------------ | ------------------------------------------------------------------------------------------------------- |
| Delegation | **REQUIRED** | README states 1.0.0, tag `ucan/dlg@1.0.0`                                                               |
| Invocation | **REQUIRED** | README states 1.0.0, tag `ucan/inv@1.0.0`; the rendered spec site still shows `ucan/inv@1.0.0-rc.1`     |
| Promise    | RECOMMENDED  | Extends invocation with distributed promise pipelining; implemented in Go (ucantone), not found in Rust |
| Revocation | RECOMMENDED  | **v1.0.0-rc.1** — a release candidate, not final, while its siblings are 1.0.0                          |

The tag skew in that table is real and load-bearing: an independent Rust
implementer reports that rs-ucan and the JavaScript implementation emit
the `-rc.1` tags, and built dual-tag acceptance into their decoder.
Anyone adopting 1.0 today must decide which tag set they emit and accept;
the specs' own text has moved faster than the WG's implementations.

Governance: specs live under the Community Specification License v1.
Editorship is Zelenka (Witchcraft Software); authors span Common Tools,
Bluesky, n0 — the iroh company — and Storacha-adjacent contributors. The
core spec repo is small and slow-moving by design. This is a
handful-of-people, grant/consulting-paced working group, not a standards
body — competent and still active, but there is no conformance programme
beyond shared test fixtures.

## Wire format and identity

Every UCAN artifact is an **envelope**: `[signature, {h: VarsigHeader,
"ucan/<subspec>@<version>": payload}]`, canonically encoded in **DAG-CBOR**
for signing. The token's identity is its **CID**: CIDv1 + dag-cbor
multicodec + SHA-256 multihash. Signatures MUST be canonical per the
varsig rules — otherwise a third party could derive a second valid
signature and therefore a second CID for the same token.

**Varsig** is UCAN's algorithm-agility mechanism: a self-describing
header naming the signature algorithm and the payload encoding signed
over, carried _inside_ the envelope, so the header can evolve without
invalidating previously issued artifacts.

**Principals are DIDs; `did:key` is required.** The mandatory cryptosuite
is SHA-256 plus **Ed25519 (RECOMMENDED/preferred), P-256, and secp256k1**.
The FAQ is blunt about anything else: other algorithms are possible but
"off spec" (non-interoperable) and MUST be declared in the varsig header.

**P-384 status — the question this estate cares about, since the IdP CA
is P-384.** P-384 is **not** in UCAN's required suite and neither Rust
implementation ships it. Varsig _can express_ ES384 today — the WG's Go
varsig library exposes a constructor — so a P-384 UCAN profile is
mechanically possible and self-describing, but it would be a private
dialect: no interop guarantee, no fixture coverage, no implementation to
borrow. The honest reading: **in practice UCAN is an Ed25519-first
system**, and a P-384 estate should treat UCAN as living one layer
_below_ its CA — the CA certifies/enrols the Ed25519 seat keys; UCAN
never needs to speak P-384. That layering is the estate's design to make;
UCAN neither helps nor hinders it beyond not covering it.

## Capability model

A capability in 1.0 is a triple: **subject × command × policy**
([delegation spec](https://github.com/ucan-wg/delegation)). This replaces
the 0.x-era `with` (resource URI) / `can` (ability string) pair:

- **Subject** (`sub`): the DID the authority is _over_. Chains are
  self-certifying, rooted at the subject's own key. There is no separate
  resource registry; the subject _is_ the resource root.
- **Command** (`cmd`): lowercase, slash-led, slash-separated, no trailing
  slash (`/space/blob/add`, `/crypto/sign`). Commands have **prefix
  semantics**: holding `/crypto` MAY prove `/crypto/sign`, and MUST NOT
  prove `/stack/pop` or `/cryptocurrency`. The command `/` ("Top") is the
  wildcard, and the spec flags it "handle with care." The `/ucan`
  namespace is reserved for protocol operations such as revocation.
- **Policy** (`pol`, required in delegations): an array of statements,
  implicitly ANDed, each `[operator, selector, argument]` evaluated
  against the eventual invocation's `args`. Selectors are a deliberately
  small jq-inspired subset (no pipes, arithmetic, regex, or recursive
  descent). Operators: comparisons, `like` (glob, `*` only), plus
  `and`/`or`/`not` and `all`/`any` quantifiers. Semantic conditions that
  can't be expressed syntactically (day of week) are explicitly out of
  scope.

**Attenuation is mandatory and structural.** The core rule: each direct
delegation MUST either directly restate or attenuate (diminish) its
capabilities; every delegated capability MUST be equal-or-narrower than
its delegator's. It is enforced three ways at validation time, not by
issuer honesty:

1. **Command narrowing is syntactic** — a child command must be the
   parent's command or a path beneath it.
2. **Policies accumulate** — the invocation's args must pass the policies
   of _every_ delegation in the chain, so a child policy can only further
   restrict. Widening by policy is impossible even if a delegate tries.
3. **Time intersects** — chain validity is latest `nbf` → earliest `exp`.

Validation itself has exactly three criteria, performed on receipt of an
invocation before execution: **time bounds**, **principal alignment**
(each proof's `aud` MUST equal the next UCAN's `iss`, forming an unbroken
chain back to the subject), and **signature validation**. Validators MUST
NOT reject a chain merely because one capability in it is irrelevant or
not understood.

Two more load-bearing details:

- **Powerline** (`sub: null`): a delegation with a null subject
  automatically delegates _all future delegations regardless of subject_.
  This is the multi-device pattern: a root identity powerlines `/` to
  each of its device keys once, and every later delegation to the root is
  automatically held by every device.
- **No confinement, by design.** Delegates can sub-delegate without
  alerting the delegator; the spec calls revocation "a last resort" and
  is explicit that you cannot enumerate all sub-delegations that exist.
  There is also **no protection against a person-in-the-middle**: you
  must delegate only to authenticated agents over secure channels. A UCAN
  delegation is _addressed_ (it names its `aud`, and exercising it
  requires the audience's key), so a leaked delegation is not a bearer
  token; but _delivering_ it confidentially is the transport's problem,
  not UCAN's.

`meta` fields exist on both delegations and invocations: asserted,
signed, but explicitly **not** delegated authority — the natural home for
audit annotations, job IDs, and (for this estate) handoff coordinates.

## Invocation: the separate signed object, and why the split matters

The single most consequential design decision in 1.0 is that **exercising
authority is a different signed object from holding it**
([invocation spec](https://github.com/ucan-wg/invocation)). The spec's
own rationale is the function-reference vs function-call distinction: a
delegation carries no task intent, no concrete arguments, no chosen
executor, and no replay identity. An invocation supplies all four:

- `iss` — the invoker, who MUST be the delegator at the end of the proof
  chain; `sub` — the subject (executor by default); `aud` — an _optional
  different intended executor_, which is what makes brokers, proxies,
  gateways and replicated state machines first-class: you can invoke
  _through_ an intermediary at a named executor.
- `cmd` — a concrete, dispatchable command; `args` — the argument map;
  `prf` — the delegation CIDs, root-first, in strict sequence. **The args
  MUST pass the policies of ALL delegations in `prf`** or the invocation
  MUST be rejected.
- `nonce` — REQUIRED; SHOULD be empty for idempotent commands so
  identical tasks share an ID. `exp` — REQUIRED but nullable; **a timeout
  within a few minutes is RECOMMENDED**, giving invocations expressive
  timeout and replay-prevention semantics delegations don't have.
- `cause` — the optional CID of the **Receipt** that enqueued this task;
  `meta` — optional (the spec's own examples: Wasm fuel, internal job
  IDs, GitHub issue references).

The **Task ID is the CID of {subject, command, arguments, nonce}** — a
content-addressed, deduplicable, replay-detectable identity for a unit of
work, derived from the artifact itself rather than assigned by a server.

Execution produces a **Receipt**, and a Receipt MAY enqueue further Tasks
via `cause` — so work composes into provenance chains: every task can
name the receipt that caused it, back to a root invocation. UCAN Promise
(RECOMMENDED) extends this with distributed promise pipelining — awaiting
another invocation's result inside an invocation, the CapTP-flavoured
idea rebuilt for partition-tolerant certificate chains.

Why this matters beyond tidiness: most token systems (Biscuit, macaroons,
JWT) have only the delegation artifact; "the request" is an unsigned HTTP
call whose binding to the token is conventional. In UCAN the request is
itself signed by the chain's end-holder, names its proofs, carries its
own short expiry and replay identity, and yields a signed receipt. For an
estate whose centre of gravity is _dispatching work to agents and later
auditing who authorised what_, the invocation/receipt pair is the audit
trail, not a log you reconstruct. The core spec also explicitly blesses
the composition this estate wants: its "Beyond Single System Image"
section applies UCAN to CRDTs/RSMs by lifting the Executor=Subject
requirement and carrying provenance in an invocation log.

## Revocation

The revocation sub-spec (still **v1.0.0-rc.1**) makes revocation _an
invocation of the reserved command_ `/ucan/revoke`, with args naming one
delegation by CID ([revocation spec](https://github.com/ucan-wg/revocation)).
Semantics, precisely:

- Revocation names **one delegation by CID**. Any issuer whose proofs
  appear in a chain MAY revoke a delegation in it — not only the direct
  issuer.
- Revoking one proof does **not** kill a capability reachable through an
  alternate unbroken chain (the spec's two-tickets analogy). Revocation
  is per-proof-path, not per-capability. This is the price of the
  additive, set-semantics merging, and it is the sharpest semantic
  difference from ACL-thinking: to truly cut off a delegate you must
  revoke every path, or let expiry do it.
- Revocations are **immutable and irreversible**; the store is append-only
  and recipients SHOULD treat it as a monotonically growing set. An
  erroneous revocation is repaired by issuing a _new_ delegation, never
  by retracting.
- The agent controlling a resource MUST maintain a **revocation cache**
  for its subject; validation checks each delegation's canonical CID
  against it. Entries may be evicted once the revoked UCAN has expired
  (plus clock-skew buffer).
- **Delivery is out of scope.** How a validator learns of a revocation is
  explicitly not specified — the format doesn't change; transport is the
  deployment's problem. The spec observes that single-source-of-truth
  resources get near-ACL semantics by sending the revocation to the
  resource's agent, and CRDT resources should embed the store at a
  well-known path.

**Checking cost, for offline seats**: the check itself is trivial — set
membership of each proof's CID in a local set, O(chain length), no
cryptography beyond the signature validation already being done. The real
cost is **freshness, not computation**: a seat offline since time T knows
no revocations issued after T. UCAN's own posture is the mitigation —
expiry is the workhorse, revocation the backstop: short `exp` bounds the
window in which a stale revocation cache can hurt you, and revocations
themselves are small signed objects that gossip well and can ride any
sync the seat already does.

## Expiry and TTL semantics

Delegations carry `nbf` (optional; omitted = valid from the epoch) and
`exp` (required; **if a token never expires, `exp` MUST be explicitly
`null`** — non-expiry is a stated decision, never a default). Chain
validity is the intersection: latest `nbf` to earliest `exp` across all
proofs. A ±60-second clock-drift buffer is RECOMMENDED. Every token
carries a REQUIRED `nonce` giving each delegation a unique CID and replay
resistance.

Invocations are stricter in practice: `exp` is REQUIRED (nullable) and
the spec RECOMMENDS timeouts within a few minutes, with replay prevention
REQUIRED of executors (a seen-CID set). The two-tier posture — long-lived
_authority_, minutes-lived _exercise_ — is deliberate and is what makes
the weak revocation story survivable: most real-world "revocations" are
just invocations expiring.

## Identity and groups

UCAN has **no group, role, or membership model** — deliberately.
Principals are DIDs; authority structure is expressed entirely as
delegation graphs. The patterns that substitute for groups: **powerline**
(the device-group pattern); a _role_ as a DID whose key is held by a
service or shared under application discipline (late binding and roster
management are the application's problem — contrast Keyline, where roles
are native graph nodes); and key rotation via mutable DID documents for
`did:web`-style identities (bare `did:key` identities rotate only by
re-delegation, with the powerline pattern as the intended bridge).

## Implementations — who implements what, exactly

| Implementation                                                              | Spec generation actually implemented                                                                                                                                                                                            | State                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [rs-ucan](https://github.com/ucan-wg/rs-ucan) (WG, Rust)                    | README: "These libraries conform to **UCAN v1.0.0-rc.1**"                                                                                                                                                                       | "Work in progress… **This code has not been formally audited**" (its own README). Still pushed to (2026-06) — but **no GitHub release since v0.4.0 in June 2023**: the entire rc.1 line is unreleased.                                                                                                                                                                                                                          |
| [pq-ucan](https://crates.io/crates/pq-ucan) 0.1.3 (ekayana-labs, Rust)      | **Final 1.0.0** — `decode` accepts the released `ucan/...@1.0.0` tags; rc.1 tags only under an explicit flag. Full delegation + invocation + policy language + execution-time chain validation; interops with the WG's fixtures | Pre-release, not externally audited. `no_std`+alloc core. Extras beyond spec: optional replay guard, a revocations set, `max_proofs` cap. One sharp edge documented by the implementer: the policy `like` operator compares text literally per spec, so `/data/h1/*` matches `/data/h1/../../../etc/passwd` — **path args must be normalized before validation**. Also covers ML-DSA-44/65/87 via _provisional_ varsig headers. |
| [go-ucan](https://github.com/ucan-wg/go-ucan) (WG, Go)                      | Delegation spec as immutable DAG-CBOR/varsig tokens with a full policy package — but its own worked examples emit `ucan/dlg@1.0.0-rc.1`                                                                                         | The WG's most actively _released_ implementation (v1.2.0, 2026-09); the rc.1 tags in its examples corroborate the tag-transition state of the WG's own code.                                                                                                                                                                                                                                                                    |
| [ucantone](https://github.com/alanshaw/ucantone) (Go, Alan Shaw — Storacha) | UCAN 1.0 including a **promise** package                                                                                                                                                                                        | The most complete 1.0 implementation found, in Go, from the production ecosystem rather than the WG org.                                                                                                                                                                                                                                                                                                                        |
| ts-ucan (WG, TypeScript)                                                    | 0.9/0.10-era JWT library; the v0.10 update stalled in draft                                                                                                                                                                     | Legacy. Production JS does not use it (see ucanto). A fresh third-party JS package implementing Delegation 1.0 appeared in Nov 2026 — evidence new implementers are targeting the final spec directly.                                                                                                                                                                                                                          |
| [ucanto](https://github.com/storacha/ucanto) (Storacha, JS) + go-ucanto     | **0.10-era semantics** (`can`/`with`/`nb` caveats) as an RPC framework                                                                                                                                                          | Production — next section. The model is proven at scale; the 1.0 wire format is not what production runs.                                                                                                                                                                                                                                                                                                                       |
| Others                                                                      | SpruceID's `ssi` crate carries an `ssi-ucan` module (0.x generation); a third-party Python profile (Nov 2026) implements the 1.0.0 wire tags for a restricted Ed25519 profile                                                   | Long tail of partial/profile implementations — a good sign for the spec's implementability, none of them a dependency to build an estate on.                                                                                                                                                                                                                                                                                    |

Algorithm coverage in Rust today: pq-ucan covers Ed25519, ES256 (P-256),
ES256K per spec, plus ML-DSA via provisional headers. **No ES384 / P-384
in either Rust implementation.**

## Production evidence — Storacha, and what it taught

Storacha (the web3.storage successor, w3up) is the existence proof, and
the invocation spec itself cites **ucanto** as "a production system that
uses UCAN as the basis for an RPC layer"
([invocation spec](https://github.com/ucan-wg/invocation)). The shape: a
**Space** is a `did:key` principal (the resource); an **Agent** is the
client's key; an **Account** (`did:mailto:…`) delegates to the agent at
login, and that delegation is _stored by the service_ so any device can
reclaim it via email verification — "password reset" for capabilities.
ucanto adds what an RPC system needs: declarative capability definitions
(≈ routes), providers bound into services, a validation system, batched
invocations, pluggable transports, and **signed receipts**.

What production added that the WG specs do not say — the most valuable
part of this section for the estate:

1. **`ucan/attest` sessions.** Principals that cannot sign (a `did:mailto`
   account) create delegations with a blank signature; a trusted
   authority (the service) issues a `ucan/attest` delegation whose
   caveats name the delegation's CID, and validators accept the pair.
   Lesson: _real identity systems include principals that can't hold
   keys; the bridge is an attestation capability from an authority — an
   estate with a CA/IdP already owns exactly that authority._
2. **`ucan/conclude`.** Receipts are represented as a special UCAN
   capability — production folded the receipt back into the token system.
3. **Revocation in production is a server hook.** ucanto server v9.0.0+
   lets a server reject explicitly revoked delegations — an
   application-level deny-list consulted at validation time. Nobody runs
   the revocation spec's gossip model; the resource's agent checks a
   list, exactly the "near-ACL for single-source-of-truth resources"
   outcome the revocation spec predicts.
4. **Delegation storage/retrieval is a service.** The specs declare token
   resolution "transport specific"; Storacha filled the gap with service
   endpoints that store delegations and release them after out-of-band
   (email) confirmation. Any estate deployment needs the equivalent — for
   us, the broker.
5. **Ergonomics gaps are real at the seams.** A documented third-party
   account hit the account-level vs space-level capability split:
   `plan/get` is scoped to the account DID while the CLI can only mint
   space-scoped delegations; Storacha's engineers confirmed the workaround
   is dropping to the core library and hand-building the delegation +
   proof chain in a script. At login the CLI agent receives a root
   delegation granting essentially everything, plus the service's
   attestation. Lesson: _the model composes; the tooling for scoping
   authority across subject boundaries lags, and production systems
   quietly hand out root-shaped grants at bootstrap._ An estate adopting
   UCAN should budget for its own minting tooling and treat bootstrap
   grants as a design surface, not a default.

Fission is the second production data point, historically: Webnative/ODD
and WNFS ran 0.9-era UCANs for user-owned file systems before the
company's April 2024 shutdown; the protocols and WG editorship survived
the company, which is itself evidence about governance risk — the spec
outlived its originating firm, but pace since has been WG-volunteer pace.

## Estate fit, concretely

### `did:key` Ed25519 = iroh NodeId = p2panda author

An Ed25519 `did:key` is `did:key:z6Mk…` — multibase base58btc over the
multicodec varint for `ed25519-pub` (0xed) followed by the raw 32-byte
public key. An iroh **NodeId is the same raw 32-byte Ed25519 public key**.
A p2panda author ID is likewise the author's Ed25519 verifying key. So
one keypair is, without any translation layer:

- the seat's **transport endpoint** (iroh NodeId — reachable, encrypted,
  NAT-traversed by the transport the estate is already choosing);
- the seat's **replication identity** (p2panda author);
- the seat's **UCAN principal** (issuer, audience, subject).

What that buys: delegating authority _to a seat_ is delegating to the
exact key that will terminate the connection and sign the operations —
there is no binding problem between "who may" and "who is on the wire,"
no certificate mapping table to keep in sync, and a delegation's `aud` is
checkable against the transport peer ID at connection time for free.
What it does not buy: anything for the P-384 world — the IdP CA stays the
enrolment layer that vouches for seat keys, and UCAN operates beneath it.
That is a clean layering, but it is ours to build: UCAN has no opinion
about how a `did:key` becomes trusted, only about what holders can prove.

### UCAN over the fleet broker — dispatch authority vs Biscuit

The job (from the fleet-actions-exec design,
[#13774](https://github.com/developing-today/code/issues/13774)): the
central coordinator hands a short-lived exec session inside a GitHub
Actions runner to an agent; the runner proves it is inside the run
(GitHub OIDC), receives commands through the broker, under a lease with a
hard max lifespan. Head to head for _this_ job:

| Axis                            | UCAN 1.0                                                                                                                                                                                                        | Biscuit                                                                                                                                                             |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Token binding                   | Delegation names its `aud`; exercising it requires an **invocation signed by the audience's key**. A leaked delegation alone is useless.                                                                        | **Bearer token**: whoever holds it, uses it. Audience binding is a check you add; there is no invocation object to sign.                                            |
| The work order                  | The invocation _is_ the work order: signed, content-addressed **Task ID**, minutes-scale `exp`, `cause`/`meta` carry the session handle and workflow coordinates. Receipts give signed provenance for what ran. | The request is out-of-band (HTTP); the token authorises it by convention. Provenance/receipts are the broker's own logging.                                         |
| Attenuation for budgets         | Command-prefix narrowing + policy accumulation over args. Policies are a jq-subset over the invocation's args — a budget must be shaped _into_ the args to be policed.                                          | **Datalog checks are strictly more expressive** for data-dependent conditions (e.g. build-minute budgets as facts evaluated at verification) — Biscuit's home turf. |
| Expiry                          | Structural: `exp` required-or-explicitly-null, chain-intersected, invocations minutes-lived by recommendation.                                                                                                  | Idiomatic: a `time` check the author must remember to include.                                                                                                      |
| Revocation at the broker        | Revoke-by-CID set consulted at validation; broker is online, so freshness is free.                                                                                                                              | Per-block revocation-ID deny-list; same online lookup.                                                                                                              |
| Implementation risk today       | **High**: rc.1 reference crate (unaudited) or third-party pre-release for final tags.                                                                                                                           | **Low**: `biscuit-auth` 6.0 is the reference implementation, production-hardened, official WASM build.                                                              |
| Fit with the rest of the estate | Same Ed25519 keys as transport/auth everywhere else; chains extend naturally seat → coordinator → runner → sub-agent with provenance intact.                                                                    | A second, unrelated token system scoped to this one surface.                                                                                                        |

Verdict for this job: **UCAN is the better-shaped tool** (audience-bound
authority + a signed work order with a content-addressed ID is almost
literally the coordinator-handoff design), **Biscuit is the safer
dependency today**, and its Datalog genuinely wins on budget-style
policies. A coherent split exists: Biscuit (or the broker's existing
tokens) as the broker's _internal_ dispatch gate where policy richness
and maturity matter, UCAN as the authority agents hand _each other_
where provenance chains and transport-key identity matter. Forcing one
winner for both jobs would sacrifice something real either way.

### UCAN carried in p2panda operation extensions

This composition is designed-for from both sides. p2panda's own
high-level docs name Meadowcap and **UCAN** as external capability
systems integrable via header extensions "for permission checks and
delegated access control" (see the [p2panda page](p2panda.md)). The UCAN
core spec's CRDT section describes exactly the carrying pattern.
Mechanically: a p2panda operation's extension carries the invocation (or
its CID) plus the proof CIDs; validators resolve delegations from a local
CID-indexed store — and both the delegation store and the revocation set
are append-only, content-addressed sets, which is to say they replicate
with the same log-sync machinery as everything else in p2panda. Costs:
every validating peer needs the store, validation adds signature checks
per operation, and the group layer (p2panda-auth) and the capability
layer (UCAN) must be reconciled by application policy — groups say who
is _in_, UCAN says who may _do what_. No other candidate in the
programme has both sides of this composition pre-designed.

### UCAN vs Meadowcap vs Keyline as the delegation layer

| Axis                   | UCAN 1.0                                                                                                           | Meadowcap                                                                                                               | Keyline (Keyhive)                                                                                                             |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Shape                  | Certificate chains, presented as proofs with each invocation                                                       | Certificate chains, area-scoped, travel with each entry                                                                 | **No chains**: a CRDT _set_ of delegations/revocations; authority is derived by evaluating the whole set (stratified Datalog) |
| What authority is over | subject × command × policy-over-args — computation-shaped (verbs, RPC)                                             | subspace × path-prefix × time-range, read/write only — data-shaped                                                      | subject scopes (incl. roles as first-class nodes) × power ladder Relay<Read<Edit<Admin                                        |
| Attenuation            | Mandatory monotonic: command prefix + accumulating policies + time intersection, enforced at validation            | Cryptographically monotone by construction: a delegation step is valid only if the previous area _includes_ the new one | Effective power = max-over-routes of min-along-route; the library refuses to sign above the issuer's effective power          |
| Expiry                 | Native, first-class (`nbf`/`exp`, explicit-null rule)                                                              | Time range is part of the area; expiry constrains _claimed entry timestamps_, not creation                              | **None, by design** (no trusted time); short-lived authority must be modelled as prompt revocation                            |
| Revocation             | Per-CID, per-proof-path, irreversible; alternate paths survive; delivery unspecified; expiry is the real workhorse | **None** — non-renewal + supersession hygiene are the designed substitutes                                              | Best in field: coordination-free, permanent, scoped by frozen admin reach, implicit cascade                                   |
| Groups/roles           | None (powerline + role-DIDs as patterns)                                                                           | None (namespaces + delegation discipline)                                                                               | **Native**: roles are graph nodes, membership is a delegation, late binding throughout                                        |
| Offline story          | Verify offline; revocation freshness bounded by expiry                                                             | Verify offline forever; writes self-certifying, tokens travel with data                                                 | Evaluate offline identically on every replica (the CRDT property); revocation converges by merge                              |
| Leaked token           | Useless without the audience's key (must sign the invocation)                                                      | Useless without the receiver's key (must sign the entry/handover)                                                       | N/A — there is no token to leak; authority is graph state                                                                     |
| Maturity               | Final specs; implementations mid-transition; production on the _previous_ generation                               | Final spec (Nov 2025); alpha Rust, recent hash-correctness incident in its support crates                               | Pre-alpha, unaudited, with disclosed live threat-model gaps (G1–G4)                                                           |

The three are not really substitutes: Meadowcap authorises _data_,
Keyline governs _membership-derived standing_, UCAN authorises
_computation by named agents_. An estate that picked p2panda for sync
could still need UCAN-shaped tokens for broker dispatch and agent RPC —
which is why the split above is coherent rather than indecisive.

## The honest gaps

1. **Rust 1.0 support is the gap.** The WG's reference crate implements
   rc.1 and calls itself unaudited work-in-progress; the only final-tag
   Rust implementation is a third-party pre-release, also unaudited.
   Building on UCAN in Rust today means vendoring one of those, pinning
   it, and owning the review — or implementing from the spec, which two
   independent parties have now done (evidence the spec is implementable,
   and evidence of how young the ecosystem is).
2. **Spec/implementation skew.** Spec text says 1.0.0; the WG's Go
   examples and (per pq-ucan) its Rust and JS implementations emit rc.1
   tags; the rendered invocation spec disagrees with its own repo README.
   The revocation spec hasn't reached 1.0.0 at all. Expect one more
   settling pass before "UCAN 1.0" names a single interoperable thing.
3. **Production runs the previous generation.** Storacha proves the
   _model_ at scale and simultaneously proves that 1.0 has no production
   deployment of that scale yet; production also routed around the specs
   on sessions/attestation, receipts, revocation, and delegation storage
   — each workaround is a small spec the estate would have to write for
   itself (or adopt Storacha's).
4. **Revocation delivery is unspecified**, per-path semantics surprise
   ACL-trained intuition, and there is no confinement: you cannot
   enumerate or bound sub-delegations except by command/policy/expiry
   construction at mint time.
5. **No PITM protection**: delegation confidentiality and peer
   authentication are the transport's job (for us, iroh — which does it
   well — but the composition must be deliberate).
6. **Policy language is deliberately weak**: jq-subset over args, no
   semantic conditions, and one documented footgun (un-normalized path
   args vs the `like` operator). Budget/state-dependent policy belongs in
   the verifier (Biscuit's Datalog, or application checks), not in UCAN
   policies.
7. **Tooling**: no hosted validator/playground/CLI story comparable to
   Biscuit's; the Storacha ergonomics record shows minting/scoping
   tooling is where adopters bleed.

## Judgment

**(a) As the estate's delegation layer — as a design, yes; as today's
dependency, not yet.** UCAN 1.0 is the best delegation _semantics_
available to this estate: mandatory monotonic attenuation, the
delegation/invocation split with receipts and provenance, powerline
multi-device, and principals that are mechanically our transport keys.
The estate should specify its delegation layer _as_ UCAN 1.0 — command
taxonomy, policy shapes, expiry posture, revocation-set sync, the
attestation bridge from the IdP CA (Storacha's `ucan/attest` pattern with
our CA as the authority) — so that implementation choice becomes a
detail. Writing that specification costs little and survives any
implementation outcome.

**(b) As a compatibility layer only — that is the correct _code_ posture
today.** Depend on UCAN where interop or a pilot demands it (a bounded
pilot is cheap: pq-ucan behind the broker for one dispatch path, or
UCAN-in-p2panda-extensions for one topic), pin the tag set explicitly
(accept both 1.0.0 and rc.1 on read; emit 1.0.0), and do not let a
pre-release unaudited crate become load-bearing for dispatch authority
without a spike and a review. For the Actions-runner handoff shipping
soonest, Biscuit remains the lower-risk internal gate.

**(c) As a watch item — one named trigger, not a shrug.** Promote UCAN
from (b) to (a) when **either** rs-ucan ships a final-1.0.0 release with
the rc.1 disclaimer gone, **or** pq-ucan (or a successor) reaches a
stable, externally reviewed 1.x — and re-check the revocation spec's
1.0.0 at the same moment. Until then the file stays open; the design work
in (a) means promotion is a crate swap, not a re-architecture.

What UCAN is best at, in one line: **proving, offline, that a named agent
was authorised by a chain of named agents to perform this specific signed
action, with the authority only ever narrowing along the way** — and in
this estate, the names are the same keys that carry the traffic.
