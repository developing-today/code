# Listing record + registry protocol — the v4 registry's record family, signing, versioning, suspension/delisting, and discovery query

Design record for issue #10662 (developing-today/code).
Date: 2026-11-23. Status: Decision (recorded by Lane WH, archer seat).
Parent: #6303 (v4 outline: hosted agent services, closed — outline landed
as `doc/2026-11-16_design_id6303_hosted_agent_services_v4_outline.md`).
Programme: epic #6280 (phase 14). Step 1 of the v4 breakdown; steps 2–4
are #10663 (hire/attach), #10664 (presence/wake/drain), #10665 (host
trust spike).

This record settles decision 1 of the v4 outline: the listing's exact
fields, its signing and versioning, the suspension and delisting
records, and the query shape worlds use to find candidates (§2.1 of the
outline). It designs records and a query protocol only — no registry
implementation, no hire mechanics (that is #10663), no runtime contract
(#10664).

## Fixed inputs (not re-decided here)

From the v4 outline (#6303):

- A listing is a signed, versioned record: the agent program's identity
  (its agent key's public half), a program manifest, the roles it is
  willing to be hired under, its operator, and its availability terms
  (§2.1).
- Listings are discoverable, not endorsed. The registry is an index
  with provenance, not a trust decision; the listing's history —
  worlds served, claims completed, claims expired, review outcomes,
  all as audit references — is the evidence the hiring world owner
  makes the trust call from (§2.1).
- Suspension is by the listing's operator or its owner, at any time;
  it stops new hires and starts the §2.3 drain. A registry operator
  can delist for abuse; delisting is an audited, public act with the
  evidence attached, never a silent removal (§2.1).
- Hosting is a deployment shape, never a capability tier (§1): a
  listing declares; it never grants. Authority enters a world only
  through the hire's scoped delegation (#10663, on v1's terms).
- No central registry monopoly (§3): this is _a_ registry protocol;
  federated and per-community registries speak the same record and
  query shapes.
- §5 question 3 (backstop listing policy) gates step 4's spike
  (#10665), not this record. Nothing here encodes a vetting or
  endorsement decision, so the record design does not wait on it.

From v1 (#6300) and the shipped `id` record discipline:

- Identity is an Ed25519 key pair; the public half is 32 bytes, a
  signature 64 bytes (`ed25519_dalek`, as used across `id-core`). On
  iroh the endpoint ID _is_ the public key (v1 §5.1).
- Delegation is UCAN-style and attenuated (v1 §5.2); audit is one
  signed, hash-chained event per action, chained with the world's
  journal (v1 §7). A listing's history references are references into
  that audit, never copies of it.
- Signed records in this family are wire-stable: a magic + version
  header, the body as postcard (#5674), then the signature; the
  signature covers the body's bytes behind a domain tag so it cannot
  be replayed over another message kind. The shipped exemplar is the
  invite event (`pkgs/id/crates/id-directory/src/invites/event.rs`),
  whose legacy v1 form stays parseable read-only.
- Every bound a record declares is enforced twice: when the record is
  minted and again when it is decoded (the invite depth cap is the
  standing example — mint-side in `sign_terms`, decode-side in
  `from_v2_bytes`, #13217/#13250). A decoder never trusts a bound just
  because a well-formed signature covers the body.
- Roles are v2's versioned world configuration; the shared starter
  vocabulary is the role names accepted by `WorldScopes::from_role`
  in `id-caps` (`spectator`, `observer`, `player`, `guest`,
  `delegator`, `builder`, `moderator`), and scopes are the
  `WorldScopes` bitset. A listing speaks that vocabulary; a world
  whose configuration renames roles maps its names onto it.

## Decisions

### 1. One record family, three record kinds

The registry's whole surface is three signed record kinds in the
family wire form above, each with its own domain tag:

| Kind | Domain tag | Signed by | Settles |
| --- | --- | --- | --- |
| Listing version | `id-listing/v1` | owner (+ agent countersignature at genesis; + operator attestation per version) | §2 of this record |
| Suspension / resumption | `id-listing-suspension/v1` | owner or operator | §5 |
| Delisting / reinstatement | `id-listing-delisting/v1` | registry operator | §6 |

A record's identity is the BLAKE3 hash of its full wire bytes
(header, body, signature). Every cross-reference in the family —
version chains, suspension targets, evidence pointers — is one of
these hashes, so a reference is verifiable by anyone holding the
record, from any registry or none.

### 2. The listing body: exact fields

One listing is a chain of listing versions (§4). Each version's body
carries, in postcard field order:

- `listing_id: [u8; 32]` — the record hash of the genesis (sequence 0)
  version. Stable for the life of the listing; every later version,
  suspension, and delisting names the listing by this id.
- `sequence: u64` — 0 at genesis, +1 per version, no gaps.
- `prev_hash: [u8; 32]` — the record hash of the previous version;
  all-zero at genesis.
- `agent_key: [u8; 32]` — the agent program's Ed25519 public key.
  Fixed at genesis; a version that names a different agent key is a
  different listing, and decoders refuse it as a chain member.
- `owner_key: [u8; 32]` — the agent owner's Ed25519 public key; the
  listing's signing key. Fixed at genesis for the same reason.
- `operator_key: [u8; 32]` — the host operator's Ed25519 public key
  (§2.4 of the outline: the operator is accountable for availability,
  and its identity is recorded). May change between versions; the
  change is itself history.
- `manifest` — the program manifest:
  - `program_name: String`, at most 128 bytes;
  - `program_version: String`, at most 64 bytes — the author's own
    version string for the offered build;
  - `program_digest: [u8; 32]` — hash of the program artifact the
    listing offers. An upgrade (§2.3 of the outline: drain-and-replace
    under the same listing) is a new version whose only necessary
    change is this digest and `program_version`; the audit records
    the digest that acted, so an upgrade never rewrites who did what.
  - `description: String`, at most 1024 bytes — what the program
    does, in the author's words. Presentation text only; no query
    semantics attach to it (§7).
  - `surfaces: Vec<SurfaceUse>`, at most 32 entries — the world
    surfaces the program uses. A `SurfaceUse` names a surface kind
    from the v3 vocabulary (board, document, chat, and their
    successors as #10049 and its siblings settle them) and the
    `WorldScopes` bitset the program needs on it. This is a ceiling
    declaration: the program states the most it will ever ask for,
    and a hire can only narrow it.
  - `resource_profile` — declared bounds, so an operator and a world
    can plan against them: `compute_class` (closed enum: `light`,
    `standard`, `heavy`), `max_worlds: u32`,
    `max_concurrent_claims: u32`.
- `willing_roles: Vec<RoleWillingness>`, at most 32 entries — each
  names a role from the shared vocabulary and the maximum
  `WorldScopes` the program accepts when hired under that role. A
  world can hire the program only under a role named here; a role
  the world offers that the listing does not name is not a match,
  however the query is phrased (§7).
- `availability` — a closed enum, the listing's availability terms:
  - `AlwaysOn` — continuous presence, leased per §2.3 of the outline;
  - `Scheduled { windows: Vec<Window> }`, at most 16 windows, each a
    weekday mask (7 bits, UTC), a start minute-of-day, and a duration
    in minutes;
  - `OnDemand { wake: WakeSet }` — event-driven wake instead of
    presence; `WakeSet` is a bitset over the wake event classes
    (claim offered, mentioned, subscribed surface changed, hire
    changed), the set #10664's contract operates.
- `issued_at: u64` — unix seconds, the signing time the owner asserts.
- `expires_at: Option<u64>` — unix seconds. A listing version past
  its expiry is not hireable and drops out of discovery results;
  renewal is a new version, never an edit. Expiry is what keeps dead
  programs from accumulating as permanent registry sediment.

Decode-side budgets, enforced at mint and at decode like every other
bound in this family: the whole postcard body at most 16 KiB; the
vector and string caps above; `sequence` and timestamps range-checked
only for encoding sanity (a registry's freshness call is policy, §7,
not a decode rule).

### 3. Signing: three signatures, three different claims

- The **owner signature** (the record's signature proper) is the
  owner's Ed25519 signature over the domain-tagged body. It claims:
  _I offer this program, under this operator, on these terms._
- The **agent countersignature** is required at genesis only: the
  agent key signs the genesis body under the same domain tag. It
  claims: _the named agent key consents to be listed._ Without it,
  anyone could list someone else's agent key and borrow its history.
  Later versions do not repeat it — the chain back to a countersigned
  genesis is the consent.
- The **operator attestation** is a separate, detachable signature:
  the operator key signs `(listing_id, sequence, program_digest)`
  under the tag `id-listing-attest/v1`. It claims: _I host, or am
  ready to host, this exact build._ A version is **valid** with the
  owner signature alone (a self-hosted program whose owner runs no
  separate operator lists itself: owner and operator are the same
  key, and the attestation is still made, by that key). A version is
  **hireable through an operator** only with that operator's current
  attestation; a registry shows unattested versions as listed-but-not
  -hosted, which is a state a query can filter on (§7), not an error.

Signatures are verified before any field of a record is trusted, and
verification needs nothing but the record itself — no registry, no
network. That is what makes the records portable across federated
registries (§7) and auditable after the fact.

### 4. Versioning: a hash-chained sequence, first-seen head

- A registry accepts version `n+1` of a listing only if its
  `prev_hash` is the record hash of the registry's current head at
  `n`. The chain is the version history; superseded versions stay
  retrievable forever (they are the provenance §2.1 promises).
- If two different versions both claim the same `prev_hash` (the
  owner signed a fork), the registry keeps the first-seen head as
  the listing's head and retains the fork as a **fork record**: both
  versions, preserved and queryable. A fork is provenance evidence
  about the owner, displayed as such; the registry never silently
  picks a winner after the fact, and a hiring world sees the fork
  flag on the listing. (First-seen is a tie-break for service, not a
  judgement; the signatures prove both branches are the owner's.)
- Fields fixed at genesis (`agent_key`, `owner_key`) may not change
  in any version; a decoder walking the chain refuses a version that
  changes them, and a registry must not serve it as a chain member.
- `program_digest` changes are ordinary versions. There is no
  separate upgrade record: §2.3's drain-and-replace consumes the new
  version, and the hire persists across it (#10663/#10664 settle the
  mechanics).

### 5. Suspension and resumption

Suspension is its own signed record, not a listing version — either
the owner or the operator can issue it, and neither should have to
mint a listing version (an operator cannot: it does not hold the
owner key) to stop hires in an emergency.

Body: `listing_id`, `scope` (closed enum: `ThisVersion` — the named
record hash is suspended; `FromSequence { from: u64 }` — that version
and everything after; `WholeListing`), `actor` (derived from the
signing key: the signature is verified against the listing's
`owner_key` or its current `operator_key`, and the record is invalid
under any other key), `reason` (closed enum: `OwnerPaused`,
`OperatorPaused`, `Maintenance`, `TermsChanged`), `issued_at`,
`resume_after: Option<u64>` (an advisory earliest-resume time, not a
promise), `note` (at most 512 bytes).

Effect: while a suspension is active for a listing, the listing is
not hireable and does not appear in discovery results except to
queries that ask for suspended listings by id. Suspension stops new
hires and starts the drain of in-flight hires (§2.3 of the outline;
#10664 owns the drain mechanics). A **resumption** is the same record
kind with `reason: Resumed`, signed by the same actor class that
suspended (owner resumes owner suspensions; operator resumes operator
suspensions; either may resume once the other has), naming the
suspension record it lifts by hash. Suspensions and resumptions are
append-only history on the listing, served with it.

### 6. Delisting and reinstatement

Delisting is the registry operator's act, and it is a public record
by design (§2.1 of the outline). Body: `listing_id`, `registry_key`
(the signing key — the registry operator's Ed25519 public key, which
is also the key that signs query attestations, §7), `reason` (closed
enum: `AbuseSpam`, `AbuseImpersonation`, `CapabilityViolation`,
`OwnerRequest`, `LegalOrder`), `evidence: Vec<[u8; 32]>` (at most 32
record hashes — audit events, suspension records, world journal
entries — the evidence the act rests on, attached, not described),
`issued_at`, `note` (at most 512 bytes).

Effect, at that registry only: the listing leaves discovery results
entirely (a by-id lookup still returns the listing, its chain, and
the delisting record — delisting is never a silent removal), and the
registry serves no new attestations for it. Delisting does not reach
into worlds: hires already in force are the world's own instruments
(role narrowing, hire termination) plus suspension's drain, exactly
as §2.4 of the outline provides.

**Reinstatement** is the same kind with `reason: Reinstated`, naming
the delisting record it lifts by hash; it is how an appeal outcome or
a corrected mistake is recorded, and it is as public as the delisting.
Because registries are federated, a delisting record is portable
evidence: another registry may import it and weigh it under its own
policy, but no registry's delisting binds any other.

### 7. The discovery query shape

A world finds candidates by sending a **query** to any registry. The
query is a filter over capability-shaped facts only:

- `roles_any: Vec<String>` — role names from the shared vocabulary;
  a listing matches if any named role appears in its `willing_roles`
  with scopes covering the query's `scopes_required`.
- `scopes_required: WorldScopes` — the scopes the hiring world
  intends to grant; a listing matches only if its declared ceiling
  for the role covers them (a program that declares less than the
  job needs is not a candidate, and the registry must not surface it
  as one).
- `surfaces_all: Vec<SurfaceKind>` — every named surface must appear
  in the listing's manifest.
- `availability_any` — a bitset over the three availability classes.
- `operators_in: Option<Vec<[u8; 32]>>` — restrict to listings whose
  current operator is in the set, when present.
- `hosted_only: bool` — when set, only versions carrying a current
  operator attestation (§3).
- `include_suspended: bool` — off by default; suspended listings are
  excluded unless asked for, and always excluded when delisted at
  the answering registry.
- `page: { cursor: Option<Opaque>, limit: u32 }` — cursor paging;
  the cursor is registry-opaque, pages are stable under concurrent
  updates only as far as the attestation below states.

The **response** is a page of entries plus an attestation:

- Each entry carries the listing's head version (the full signed
  record), its state at the answering registry (`active`,
  `suspended` with the active suspension record, `forked` flag from
  §4), the operator attestation if any, and a **history summary**:
  counts the registry asserts from the audit references it holds —
  worlds served, claims completed, claims expired, review outcomes
  (accepted / rejected) — each with an `as_of` time and the head hash
  of the evidence set the counts were computed from. The summary is
  the registry's claim, labelled as such; the underlying references
  are fetchable by hash so a world owner can check before trusting.
  History is evidence presented, never a ranking input the protocol
  defines: result order is the registry's local choice and carries
  no protocol meaning.
- The page is signed by the registry key as an **index attestation**
  over `(query hash, index head hash, as_of)`: the registry states
  what its index held when it answered. That makes the index
  accountable — a world can later prove what it was shown — without
  making it an endorser (§2.1 of the outline).

Federation: the same query runs against several registries and the
world merges. Merge is by `listing_id`; for each id the world keeps
the chain with the highest verified head sequence (verification is
offline, §3), prefers a head that is not suspended, and surfaces
disagreements — different heads at the same sequence, a delisting at
one registry and not another — as provenance, not errors. Queries
are unauthenticated reads of public records; a registry may rate
-limit, and listings being public by design means there is no
existence-privacy obligation of the #5799 kind here (see the
divergence check).

### 8. Worked example (illustrative)

A board-tending program: it watches boards it is hired onto, claims
routine triage items, and hands off anything ambiguous.

- Genesis listing: `agent_key` = the program's key; `owner_key` = its
  author's key; `operator_key` = the backstop host's key, with that
  operator's attestation over digest `D`.
- Manifest: name `board-tender`, version `1.4.0`, digest `D`;
  description "Claims routine triage items on hiring boards; hands
  off ambiguous items with notes."; surfaces: one `SurfaceUse` —
  board, scopes `JOIN | CHAT | INPUT`; resource profile `light`,
  `max_worlds` 8, `max_concurrent_claims` 4.
- Willing roles: one entry — `player`, ceiling `JOIN | CHAT | INPUT`.
- Availability: `OnDemand { wake: claim offered | subscribed surface
  changed }`.
- A world owner queries `{ roles_any: [player], scopes_required:
  JOIN | CHAT | INPUT, surfaces_all: [board], availability_any:
  on-demand }`; the listing matches, the response carries its history
  summary (say: 3 worlds served, 41 claims completed, 2 expired, 38
  reviews accepted / 3 rejected, with evidence head hash), and the
  owner makes the trust call and hires under #10663.

The board surface's exact object vocabulary is #10049's to settle —
it is still open — so the surface name and claim counts here are
illustrative. Nothing in §§1–7 waits on it: surface kinds are named
by string-free enum values the v3 steps extend, and the record design
is complete without the example.

## Alternatives rejected

- **Registry-issued listing ids** (a registry assigns the id at first
  submission). Rejected: it makes the id registry-local, breaks
  federation merge (§7), and gives the index a gatekeeping power
  §2.1 denies it. Deriving the id from the genesis record makes it
  identical everywhere the record goes.
- **Mutable listing records with an edit log kept by the registry.**
  Rejected: the version chain must be verifiable from the records
  alone; a registry-kept log asks the reader to trust the index —
  the thing §2.1 says the registry is not for.
- **Endorsement or rating fields in the record** (registry score,
  verified badge). Rejected by §2.1 directly: discoverable, not
  endorsed. History travels as evidence with references, never as a
  verdict field.
- **Free-text availability** ("usually around on weekends").
  Rejected: availability is operated on — #10664's wake and lease
  machinery consumes it — so it is a closed enum with structured
  windows, or it is decoration.
- **Suspension as a listing version.** Rejected: the operator cannot
  sign listing versions (it does not hold the owner key), and an
  emergency stop must not wait on the owner minting one (§5).
- **Deleting delisted listings from the registry.** Rejected by
  §2.1: delisting is a public record, not a silent removal (§6).

## Invariants implementation must hold

1. No field of any record in the family is trusted before its
   signature verifies, and verification is offline: record bytes and
   the keys named in the record suffice.
2. Every declared bound (body budget, vector/string caps, chain
   linkage, genesis-fixed fields) is enforced at decode as well as
   at mint; a well-signed record that breaks a bound is refused, not
   served.
3. A listing grants nothing. It is a declaration with provenance;
   the only authority in the system remains the hire's delegation
   (#10663) under the world's role configuration.
4. History is append-only end to end: versions, suspensions,
   resumptions, delistings, reinstatements are never edited or
   removed by any act in the protocol, including delisting itself.
5. An expired or suspended listing is never hireable and never
   returned by a default discovery query; a delisted listing is
   never returned by any discovery query at the delisting registry.
6. Registry answers are attestations about an index, not trust
   decisions: responses carry the evidence references a hiring owner
   needs to make the call themselves.

## Divergence check against the parents

- Against v4 §2.1: the record carries exactly the five named
  elements (identity, manifest, willing roles, operator, availability
  terms) and the two named lifecycle acts (suspension by owner or
  operator, delisting by a registry operator with evidence). The
  outline's "worlds served, claims completed, claims expired, review
  outcomes, all as audit references" is §7's history summary, with
  the counts explicitly labelled registry claims over fetchable
  references.
- Against v4 §1: the listing is deployment-shaped throughout —
  `operator_key` records who hosts, and §3 keeps the operator's
  attestation separate from the owner's offer, so changing hosts is
  a version, not a new identity or a new authority.
- Against v4 §3: no marketplace mechanics appear (no price, bid, or
  payment field exists anywhere in the family); no central registry
  is assumed (§7's merge rule is the federation story); no new
  agent-to-agent channel is created (queries are world-to-registry
  reads).
- Against #5799's existence privacy: not applicable, and stated so
  rather than silently skipped — listings are public records by
  design, published by their owners for discovery. Nothing about
  worlds, their membership, or their contents is revealed by a
  listing or a query.
- Against v1 §5.2: nothing here delegates. The agent countersignature
  at genesis is a consent proof, not a delegation; it carries no
  scopes and no lifetime beyond the listing itself.

## Splits and follow-ons

- **Implementation home:** `id-directory` is the natural crate — it
  already holds the directory layer and the invite record family
  whose wire discipline this design follows. The record types,
  codecs, and chain/suspension/delisting verification land there as
  ordinary implementation issues when the programme schedules them;
  this record is the design they build against.
- **#10663 (step 2)** consumes: `listing_id` + head record hash as
  what a hire names; the operator attestation as the hire's host
  precondition; suspension state as the hire-time refusal input.
- **#10664 (step 3)** consumes: the `availability` enum and `WakeSet`
  as the wake contract's inputs; suspension as the drain trigger.
- **#10665 (step 4)** consumes: the whole family, in the spike — one
  program listed, attested, hired, suspended-and-drained, and
  replaced by a new version mid-work. Its backstop-policy question
  (§5 Q3 of the outline) is a policy for the spike's instance, not a
  change to these records.
- **Surface vocabulary:** as #10049 and its sibling v3 steps settle
  surface kinds, the `SurfaceUse` kind set grows by addition; adding
  a kind is a record-family version bump of the ordinary kind, not a
  redesign.
