# Bluesky / AT Protocol — state of the network (as of 2026-11-23), and the id read-compatibility scope

Lane BSKY-ID, for the auth+transport investigation
([#13779](https://github.com/developing-today/code/issues/13779)) and the id
program (epic [#2056](https://github.com/developing-today/code/issues/2056)).
Primary sources: atproto.com specs and blog, the did:plc specification,
proposals repo, IETF announcements, PLC Organization. Ecosystem counts are
community-measured (PLC-directory-derived) and are labelled as such — the PLC
log counts raw identity registrations, including bots and mass-registration
junk, not people.

## Part A — What Bluesky / the AT Protocol is up to

### A.1 Direction in one paragraph

The public-data protocol is declared "broadly complete" by its own team and
is being handed to formal governance: an IETF working group (chartered March 2026) for the repository/sync core, and an independent Swiss association for
the PLC identity directory. The protocol team's active frontier has moved to
**permissioned data** — "Spaces", an access-controlled (explicitly _not_
encrypted) second data protocol, in alpha since August 2026 — plus account
experience inside the PDS. OAuth grew from app-password-equivalent access
into a granular permission system (shipped early 2026). Bluesky-the-company
still operates the default PDS fleet, the main relay, the main AppView, and
the primary moderation labeler; independent infrastructure is real but small
by account share.

### A.2 Governance and standardisation

- **IETF**: two Internet-Drafts (repository+sync as proposed standard;
  architecture overview as informational) were submitted in September 2025,
  followed by a non-WG-forming BoF at IETF 124 (Montreal, Nov 2025). The
  **ATP working group was formally approved in late March 2026** (chairs:
  Mallory Knodel, Shuping Peng). Charter scope is deliberately narrow:
  the public repository data structure, the sync mechanism (firehose), the
  AT URI scheme, and requirements for account-identifier resolution systems
  with a registry of recommended systems (PLC, `did:web`). Explicitly _not_
  in the initial charter: non-public data, Lexicon, the OAuth profile, auth
  scopes, PLC itself, the handle system. Major changes to public-data sync
  are now expected to happen inside the IETF process.
  ([roadmap](https://atproto.com/blog/2026-spring-roadmap),
  [WG announcement](https://atproto.com/blog/kicking-off-the-atp-working-group),
  [Taking AT to the IETF](https://atproto.com/blog/taking-at-to-the-ietf))
- **PLC governance**: the Public Ledger of Credentials Organization was
  announced as a **registered Swiss Association** on 2026-09-28, with a board
  drawn from outside Bluesky (Richard Barnes, Thyla van der Merwe, Wendy
  Seltzer, Filippo Valsorda, plus Bluesky's Bryan Newbold), bootstrap-funded
  by Bluesky Social PBC. It is _preparing_ to take over directory assets and
  operations; as of the latest sources it does not operate the directory
  yet — plc.directory still runs under Bluesky by default.
  ([PLC Org first steps](https://blog.plcred.org/3mwlphq42d227))
- **PLC replicas**: WebSocket streaming of directory updates landed in
  January 2026 and a reference replica implementation in February 2026, so
  third parties can run live, independently auditable copies of the identity
  log. ([roadmap](https://atproto.com/blog/2026-spring-roadmap))

### A.3 OAuth and the permission system — the delegated-authority answer

This is the part that matters for our auth comparison. ATProto's answer to
delegated authority is **not** capability tokens. It is a hardened,
federated profile of mainstream OAuth, plus a protocol-native permission
vocabulary:

- **The profile** ([OAuth spec](https://atproto.com/specs/oauth)): OAuth 2.1
  baseline — authorization-code grant only, PKCE (S256) mandatory,
  **PAR** (pushed authorization requests) and **DPoP with mandatory server
  nonces** (tokens are bound to a client-instance key, not pure bearer).
  There is no shared `client_secret` and no RFC 7591 dynamic registration:
  a `client_id` _is_ an HTTPS URL pointing at a public client-metadata JSON
  document, fetched by the authorization server. Confidential clients
  authenticate with `private_key_jwt` (ES256 recommended) against a JWKS
  published in their metadata; refresh tokens are bound to that key, and
  removing a compromised key from the JWKS is the incident-response lever.
  The PDS is the resource server and usually also the authorization server
  (an "entryway" AS can front several PDSes). A mandatory `sub`-check in the
  token response makes the flow double as account authentication, because
  OIDC cannot currently express atproto identity generically.
- **Permissions** ([Permissions spec](https://atproto.com/specs/permission),
  [proposal 0011](https://leaflet.pub/b7540e0e-6621-4be1-8a40-a4ec10164391)):
  shipped in the six months before the Spring 2026 roadmap. Five resource
  types — `repo` (record writes, parameterised by collection NSIDs and
  `action=create|update|delete`), `rpc` (authenticated calls to a service,
  parameterised by endpoint NSID and audience DID), `blob` (media upload by
  MIME type), `identity` (handle/DID attributes), `account` (email, hosting
  status). Full wildcards exist (`repo:*`); partial wildcards do not.
  Permissions are expressed as OAuth scope strings, and can be bundled as
  **permission sets**: Lexicon schemas with human-readable, localisable
  titles/descriptions, published under an NSID and pulled into a scope with
  `include:<nsid>` — a permission set may only cover resources under its own
  NSID hierarchy, and cannot contain `blob` or `account` permissions. The
  old transitional scopes (`transition:generic` = app-password-equivalent,
  `transition:chat.bsky`, `transition:email`) remain for migration and are
  slated for deprecation, with no removal date.
- **Inter-service auth** is separate: `com.atproto.server.getServiceAuth`
  mints short-lived JWTs signed by the account's repo signing key, audience
  set to the target service's DID — the mechanism `rpc` permissions gate.

**How this compares to capability tokens (UCAN/Biscuit/Meadowcap), for the
#13779 decision:**

| Axis                  | ATProto OAuth + permissions                                                                                                          | Capability tokens (UCAN/Biscuit)                                               |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| Where authority lives | The PDS (authorization server) issues and validates; enforcement is server-side on every request                                     | The token itself; any verifier checks the chain offline                        |
| Delegation            | One hop: user → client, at interactive consent time. No client→sub-client re-delegation, no attenuation by the holder after issuance | Arbitrary depth, holder-side attenuation, designed for agent→agent chains      |
| Revocation            | Session/token revocation at the PDS; JWKS key removal for confidential clients. Immediate and central                                | Deny-lists or short expiry; the known weak axis of token systems               |
| Offline / p2p use     | None — requires reaching the account's PDS/AS                                                                                        | Native                                                                         |
| Consent legibility    | Strong: permission sets are lexicon-published, titled, localised; the consent screen is the product surface                          | Weak-to-none standardised; DIY                                                 |
| Fit to our estate     | Mirrors what our IdP + Cloudflare doors already do (interactive OAuth, server-checked scopes)                                        | Matches the fleet's seat→runner delegation problem, which OAuth does not reach |

Verdict for the comparison: Bluesky validated the _permission vocabulary_
idea — resource-typed, parameterised, bundleable grants described in the
same schema language as the data — but deliberately stayed inside
server-mediated OAuth. It is evidence that a legible permission layer is
worth building; it is not a candidate substrate for offline capability
delegation, and Bluesky does not pretend otherwise.

### A.4 Identity: did:plc and did:web

- **did:plc** spec is at **v0.3.0 (December 2025)**
  ([spec](https://github.com/did-method-plc/did-method-plc/blob/main/website/spec/v0.1/did-plc.md)).
  The DID is a hash of the genesis operation (self-certifying). Control
  rests with up to five priority-ordered **rotation keys** (secp256k1 or
  P-256), kept separate from the repo **signing key** by best practice; a
  higher-priority rotation key can recover/undo a hostile update inside a
  **72-hour recovery window**. The directory validates and timestamps an
  append-only, publicly exportable operation log per DID; because every
  operation is signed by the previous operation's rotation keys, the log is
  verifiable without trusting the server for anything except availability
  and fork choice. (The CAPNP-SURVEY note already flagged the rotation/
  signing key split as a pattern worth borrowing into our CA program.)
- **did:web** remains the second method: hostname-only (no paths) in
  atproto, resolved from `/.well-known/did.json`; portability and recovery
  are exactly as strong as control of the domain — no rotation/recovery
  machinery of its own.

### A.5 Architecture: PDS / Relay / AppView, Sync 1.1, portability

- **Sync 1.1**: Bluesky released **Tap** (December 2025), a self-hosted,
  application-agnostic sync consumer that is the reference implementation
  for reliable firehose consumption and historical backfill; the
  `bsky.network` relay was upgraded to the Sync 1.1 implementation in
  January 2026. Remaining Sync 1.1 pieces per the roadmap: defined block
  ordering for repo CAR exports, and exporting repo subsets by collection.
  ([roadmap](https://atproto.com/blog/2026-spring-roadmap),
  [sync spec](https://atproto.com/specs/sync))
- **Firehose shape** (unchanged fundamentals): relays aggregate PDS
  firehoses into `com.atproto.sync.subscribeRepos` — CBOR-over-WebSocket,
  sequence-cursor resumption, `#commit` events carrying a CAR-slice diff
  plus the op list, `#sync`, `#identity`, `#account`. In practice much of
  the ecosystem consumes **Jetstream** (Bluesky's JSON, per-collection/DID
  filtered firehose simplification) instead of the raw CBOR stream.
- **Account experience moves into the PDS**: the reference PDS is gaining
  account management (email/password change, deactivate/delete, data
  export) and additional 2FA methods, precisely so apps stop needing
  account-level permissions for these flows.
- **Portability** is the repo CAR: `com.atproto.sync.getRepo` returns the
  full signed repo unauthenticated; account migration between PDSes
  (export repo + blobs, import at the new PDS, PLC operation repointing
  the `#atproto_pds` service) is an established, practised flow — the
  mechanism the independent-PDS account counts below are built on.

### A.6 Permissioned data — "Spaces" (the encryption question)

Proposal
[0016](https://github.com/bluesky-social/proposals/tree/main/0016-permissioned-data)
(private data → permissioned data → briefly "buckets" → **Spaces**) launched
in **alpha on 2026-08-20** with running code, SDKs, a sample app, and a
hosted alpha PDS ([announcement](https://atproto.com/blog/atproto-spaces-alpha)).
Design facts that matter for us:

- A Space is a gated context with a single **space authority** (a DID);
  records live in per-space permissioned repos on authors' PDSes, with
  their own repo format, sync mechanism (lighter-weight, real-time
  capable), addressing, and resolution path. Each record is in exactly
  one space.
- **Access control, not confidentiality** — stated repeatedly and without
  hedging: data is _not_ end-to-end encrypted; any member, app, or server
  handling it can read it. E2EE is explicitly out of scope, "may be layered
  on top by an application."
- Access credentials are **bound to the application** (a key held by the
  app the credential was issued to), short-lived (~10 minutes default),
  and scoped to a single space.
- Parallel community architectures (Blacksky, Northsky, Habitat) fed into
  the design process. GA timing is "later in 2026" per Bluesky; third-party
  trackers list a December 2026 beta — treat the exact date as unconfirmed
  against primary sources.

So on the axis where our investigation cares most — end-to-end encrypted,
capability-gated shared data — Bluesky's answer is: not this, not yet, and
not in this layer. Spaces is a locality/permissioning primitive for social
apps, enforced by servers.

### A.7 Ecosystem state (community-measured, November 2026)

- PLC directory: ~132M `did:plc` identities ever registered, but the raw
  count is dominated by junk (one unreachable host accounts for ~56M
  genesis operations). Community enumeration finds **~47.8M repos**, of
  which **~96.9% sit on Bluesky-operated PDS infrastructure**; ~3,600
  independent PDS hosts currently online (of ~8,400 ever seen).
  ([sifa.id stats](https://sifa.id/stats),
  [atproto.barn.city](https://atproto.barn.city/overview))
- Named independent providers by current accounts: Blacksky ~44.9K,
  Eurosky ~43.1K, Tangled ~42.6K, then a long tail (Sprk, Gems,
  surf.social, Northsky ~2K, self-hosters). Blacksky additionally runs an
  independent full-network relay and AppView and maintains **rsky**, a
  Rust PDS implementation. The microcosm project runs independent
  indexing/AppView-adjacent services (Constellation, Spacedust, Slingshot)
  in Rust. Bluesky-the-app announced 40M users in November 2025.
- Lexicon growth is broad (the reference tooling — a refreshed TS SDK and
  the `lex`/`goat` tools for resolving, publishing, and linting lexicons —
  was a headline roadmap item), but the gravitational centre remains
  `app.bsky.*`; most third-party lexicons are app-specific namespaces
  consumed by their own authors' AppViews.

### A.8 Bluesky-the-company's priorities, as observable

Protocol team (per its own roadmap): permissioned data as the major focus,
account experience in the PDS, active IETF participation, SDK parity across
languages, and a protocol conformance test suite. Company-side, Bluesky PBC
still operates the default infrastructure (grouped under the name "Bluesky
Protocol Services" in August 2026, per secondary reporting) while
institutionally divesting the two governance crown jewels — protocol
standardisation (IETF) and identity (PLC Organization). The pattern is
consistent: keep operating the product, give away the protocol's control
points.

## Part B — The id compatibility question: reading Bluesky natively in `pkgs/id`

Drew's steer: "we should probably at least support it for compatibility
layer — allow reading bsky in id natively, consider." This scopes a
**read-only** AT Protocol ingestion path for the id program. Nothing here
proposes write support, account creation, or Spaces.

### B.1 What "reading bsky" decomposes into

1. **Handle → DID**: DNS TXT at `_atproto.<handle>` or
   `https://<handle>/.well-known/atproto-did`; the result must be verified
   bidirectionally against the DID document's `alsoKnownAs`.
2. **DID resolution**:
   - `did:plc` — HTTPS GET `https://plc.directory/<did>` returns the DID
     document. The strong form also fetches `/<did>/log/audit` and verifies
     the operation chain (each op signed by the prior op's rotation keys;
     genesis hash = the DID). The directory is trusted for availability and
     fork choice only if the chain is verified; a plain document fetch
     trusts it for content too.
   - `did:web` — GET `https://<host>/.well-known/did.json`; trust = DNS+TLS.
     From the document: the `#atproto` verification method (repo signing key)
     and the `#atproto_pds` service endpoint.
3. **Record reads (light path)**: XRPC against the account's PDS —
   `com.atproto.repo.describeRepo`, `com.atproto.repo.listRecords`,
   `com.atproto.repo.getRecord`. Unauthenticated for public data. This path
   trusts the PDS for completeness (it can omit records undetectably).
4. **Repo ingest (verifiable path)**: `com.atproto.sync.getRepo` returns a
   CAR v1 file with the current signed commit, all MST nodes, and all
   records. Verification is self-contained given the DID document:
   - decode DAG-CBOR (the spec's current text says DRISL, the DASL CBOR
     subset; blessed CIDs are CIDv1 / dag-cbor / SHA-256);
   - walk the **MST** (Merkle Search Tree: keys are
     `<collection>/<rkey>`; a key's depth = count of leading zero bits in
     SHA-256(key) ÷ 2, fanout 4; nodes use mandatory prefix compression)
     and recompute the root — it must equal the commit's `data` CID;
   - verify the **commit signature**: SHA-256 over the unsigned commit's
     CBOR, signed with the account signing key (secp256k1 or P-256;
     the key and curve come from the DID document — the commit itself
     does not say which key signed it, and rotation makes verification of
     _old_ commits ambiguous; the current commit must verify against the
     current document);
   - check `rev` (TID) monotonicity against previously seen state.
     ([repository spec](https://atproto.com/specs/repository))
5. **Live follow (optional, later)**: subscribe to a relay's
   `com.atproto.sync.subscribeRepos` and apply `#commit` diffs. The spec's
   verification chain is elegant and cheap: each `#commit` carries the op
   list and `prevData` (previous MST root); applying the ops in reverse to
   the diff's partial tree must reproduce `prevData` — a consumer keeps
   only (DID → rev, root CID) per repo instead of full MSTs. `#identity`
   events invalidate cached DID documents; `#account` events carry hosting
   status (takendown/suspended/deleted are hop-by-hop authoritative).
   The full-network firehose is a high-volume stream; a _filtered_
   consumption shape (Jetstream-style, or per-account `getRepo` polling
   with `since`) is the realistic starting point for id.
   ([sync spec](https://atproto.com/specs/sync))
6. **Blobs**: records reference blobs by CID; fetching is
   `com.atproto.sync.getBlob` from the PDS. Likely out of scope for a
   first slice (metadata/reference only).

### B.2 Rust library state

- **atrium** ([atrium-rs/atrium](https://github.com/atrium-rs/atrium), MIT,
  actively released through late 2026): the established library.
  `atrium-api` (lexicon-codegen'd XRPC types, incl. `com.atproto.repo` /
  `com.atproto.sync`), `atrium-xrpc` + `atrium-xrpc-client`, and in the
  wider workspace `atrium-identity` (DID/handle resolution) and
  `atrium-repo` (MST/repository handling), `atrium-oauth` (not needed for
  read-only). Caveat: its centre of gravity is the client/agent session
  model; a pure ingest pipeline uses the lower crates, not `AtpAgent`.
- **jacquard** ([zbrox/jacquard](https://github.com/zbrox/jacquard),
  MPL-2.0): newer suite with `jacquard-identity`, `jacquard-repo` (MST,
  commits, CAR I/O), zero-copy borrowed deserialization — performance
  doctrine-friendly. Caveats: single-maintainer-shaped, explicitly
  pre-1.0 with frequent breaking releases (0.12.x in mid-2026), streaming
  and repo crates self-described as needing more third-party testing.
- **atproto-crates** ([atproto-crates.com](https://www.atproto-crates.com),
  MIT, extracted from the smokesignal.events app): 18 separable primitive
  crates — `atproto-identity`, `atproto-repo` (CAR reading without an HTTP
  client), `atproto-dasl` (CIDs) — designed to be taken à la carte, which
  matches an importer crate's needs; published at 0.14.x with the workspace
  ahead in unpublished RCs.
- **microcosm-rs** (Constellation/Slingshot et al.): production Rust
  firehose/indexing services — prior art for the live-follow slice rather
  than a dependency.
- Fallback: the MST + CAR + commit-verification core is bounded,
  spec-complete work (the repo spec is self-contained) if no crate's
  verification semantics fit; both atrium and jacquard already implement
  MST, so this is a last resort, not the plan.

### B.3 Where it sits in `pkgs/id`

Precedent exists: `pkgs/id/crates/id-import-notion` is "Notion importer
manifest, inventory, and page/block conversion primitives" — importer-shaped
work already lives in its own crate, outside `id-core`. The atproto path
should follow it:

- A new read-only importer crate (working name `id-import-atproto`):
  resolve (handle/DID) → fetch (XRPC light path and/or CAR ingest) →
  **verify** (MST root + commit signature) → emit foreign records with
  provenance (DID, collection, rkey, CID, commit rev) into id's existing
  store/events shapes.
- Explicitly **not** in `id-core` (no core model changes for a foreign
  protocol) and **not** in `id-sync` (that crate is id's _own_ log-sync /
  meta protocol; atproto ingest is foreign-protocol import, and overloading
  the name would blur exactly the boundary the crate split created).
- The open modelling question: are ingested records mapped into id's object
  model, or kept verbatim as DAG-CBOR with an index over (DID, collection,
  rkey)? Verbatim-with-provenance is the honest first slice; interpretation
  is per-lexicon and should be opt-in.

### B.4 Costs and design questions, honestly

- **MST/CAR verification is real work** — deterministic tree reconstruction,
  prefix-compression parsing, CID discipline (non-blessed links in MST
  structure = invalid repo), size/depth limits from the spec's security
  considerations. It is also the entire value proposition over "just call
  the AppView": without it, id is trusting Bluesky's indexer, not reading
  the network.
- **PLC trust posture is a decision, not a detail**: (a) trust
  plc.directory HTTPS responses; (b) verify the audit-log chain per DID
  (cheap, per-account); (c) run/consume a PLC replica. (b) is the sweet
  spot for an importer; (a) is what almost every client actually does.
- **Lexicon openness**: repos can contain any NSID. Unknown collections
  must ingest as opaque records, not fail. Interpreting `app.bsky.*` is a
  separate, optional layer.
- **Scale asymmetry**: one-shot per-account ingest is small (repos are
  specced for single-digit millions of records, typically far fewer). The
  firehose is network-scale; following "bsky" wholesale is an indexing
  product (that is what AppViews are), not an importer feature. Scope must
  say which one id wants: _accounts_ (tractable) or _the network_ (an
  AppView programme).
- **Deletions and takedowns**: repos are tombstone-free by design; mirrors
  are expected to honour deletions and `#account` status promptly. An id
  ingest that retains deleted records verbatim is making a policy choice
  that should be explicit.
- **Spaces/permissioned data**: out of scope — non-public, alpha, and its
  access model (server-enforced, app-bound credentials) does not compose
  with a read-only public-data importer.
- **Key/curve surface**: secp256k1 + P-256 verification joins id's existing
  crypto dependencies; neither is exotic, both must be constant-shape
  (verify-only path).

### B.5 Filed

This scope is filed as a consideration issue in developing-today/code,
linked to the id epic #2056 and the investigation #13779 (see the lane
report for the issue number). Recommendation shape, if Drew takes it up:
slice 1 = DID/handle resolution + `getRepo` CAR ingest with full MST +
commit verification for a named set of accounts, verbatim records with
provenance; slice 2 (optional) = filtered live follow. No writes, no
Spaces, no AppView ambitions in either slice.
