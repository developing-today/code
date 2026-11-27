# id: AT Protocol read compatibility — sizing and go/no-go (#13787)

Research packet for #13787, routed by its triage (2026-11-24): size the
read-only AT Protocol path against what `pkgs/id` has today and recommend
go or no-go with a first slice. The protocol research itself is already on
main in
[`doc/2026-11-24T00-53-05Z_research_p2panda-investigation/research/bluesky-atproto.md`](../2026-11-24T00-53-05Z_research_p2panda-investigation/research/bluesky-atproto.md)
(Part B); this note does not repeat it. It adds the codebase fit, the
dependency choice, a slice plan with sizes and oracles, and a default answer
to each open question on the issue.

Status: research and recommendation only. No code, no decision. The go or
no-go is Drew's.

## Recommendation

**Go, scoped to accounts, verified ingest only.** One importer crate,
`id-import-atproto`, that resolves a handle or DID, fetches the account's
whole repo as a CAR file, verifies it end to end (MST root and commit
signature against the DID document), and emits the records verbatim with
provenance. No writes, no live follow, no lexicon interpretation, no blobs
in the first three slices.

Why go: the verified path is bounded (the repository spec is
self-contained), it fits an importer precedent that already exists, and
it is the only form that is "reading the network" rather than trusting an
AppView. Why scoped: following "bsky" as a whole is an indexing programme;
following named accounts is an importer.

Drop the light XRPC path (`listRecords`/`getRecord`) as a separate slice. It
is strictly weaker than `getRepo` (the PDS can omit records undetectably),
and a per-account `getRepo` is small, so the verified path is also the
simple one.

## Fit with `pkgs/id` today (main, 2026-11-25)

- **Importer precedent**: `crates/id-import-notion` (~3,900 lines) is a pure
  crate. The network transport is a caller-supplied trait (`NotionApi`),
  output is conversion structs and a resumable `Manifest`, and nothing in the
  workspace consumes it yet. The atproto importer should copy that shape:
  transport injected, no store writes inside the crate, fixtures instead of
  network in tests.
- **Workspace constraints**: edition 2024, `rust-version = "1.96"`,
  `sha2 = "0.11"` across crates (`id-core` already carries `sha2 0.10` under
  an alias, so a second copy is tolerated but not wanted). HTTP is
  `reqwest 0.13` with rustls in `id-sync` and `id-worlds`. There is no DNS
  resolver and no secp256k1 or P-256 code in the workspace; signing today is
  `ed25519-dalek 3`.
- **Prior mentions**: none in code. The only atproto material is the two
  research notes under `doc/`.

## Dependency choice

Read from crates.io on 2026-11-25:

| Crate                                  | Version | License           | MSRV | Last release | Fit                                                                                                                        |
| -------------------------------------- | ------- | ----------------- | ---- | ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `atrium-repo`                          | 0.2.0   | MIT               | 1.85 | 2026-10-27   | Pins `sha2 ^0.10`, `serde_ipld_dagcbor ^0.6`, `thiserror ^1`; async tokio stream API. Pulls `atrium-api` (large codegen).  |
| `atrium-identity`                      | 0.2.0   | MIT               | 1.85 | 2026-10-27   | DNS via `hickory-proto ^0.24` (optional); client-session centred.                                                          |
| `atproto-repo` / `-identity` / `-dasl` | 0.14.5  | MIT               | 1.90 | 2026-04-02   | `sha2 ^0.11`, `cid ^0.11`, `thiserror ^2` match the workspace; pulls tokio and tempfile; no release in about eight months. |
| `jacquard-repo` / `-identity`          | 0.12.2  | MPL-2.0           | —    | 2026-09-05   | Pre-1.0, frequent breaking releases, file-level copyleft. Not recommended.                                                 |
| `cid`                                  | 0.11.3  | MIT               | 1.81 | 2026-04-27   | Primitive.                                                                                                                 |
| `serde_ipld_dagcbor`                   | 0.7.0   | MIT/Apache-2.0    | 1.81 | 2026-08-03   | Primitive.                                                                                                                 |
| `k256` / `p256`                        | 0.14.0  | Apache-2.0 OR MIT | 1.85 | 2026-07      | Verify-only signature primitives (RustCrypto).                                                                             |

Recommendation: **implement the CAR reader, MST rebuild and commit check in
the crate, over `cid`, `serde_ipld_dagcbor`, `sha2 0.11`, `k256` and `p256`**,
and use `atproto-repo` as a dev-dependency cross-check in tests only. This
reverses the research note's "own implementation is a last resort" for two
reasons found here: atrium's repo crate would bring a second `sha2`, an old
`thiserror` and an async streaming surface into a crate that wants a pure,
synchronous verifier; and the verification rules (blessed CIDs only, depth
and size limits from the spec's security section, prefix-compression
parsing) are exactly the part id must own and test rather than inherit.
The sizes below assume this choice. If Drew prefers a library, `atproto-repo`
is the better fit than atrium on dependencies, at the cost of a slower
release cadence.

## Slices

Each slice is one PR with a named oracle. Sizes: S under 400 lines, M 400 to
1,200, L above. Remote builds and tests only.

1. **Verified repo core (M–L)**. CAR v1 reader; DAG-CBOR decode with CID
   discipline (CIDv1, dag-cbor, SHA-256, anything else in MST structure is an
   invalid repo); MST walk with key depth from leading zero bits of
   SHA-256(key) in 2-bit steps, fanout 4, prefix-compressed entries; root
   recomputation equal to the commit's `data`; commit signature over the
   unsigned commit's CBOR with a supplied secp256k1 or P-256 key; limits on
   depth, node size and record count. Oracle: checked-in fixture CARs with
   known roots and signatures, plus agreement with `atproto-repo` on the same
   fixtures. The AT Protocol interop-tests repository is the candidate
   source for MST and signature vectors; its coverage was not read for this
   note.
2. **Identity (M)**. `did:plc` document plus audit-log chain verification
   (genesis hash equals the DID; each operation signed by a rotation key of
   the previous one); `did:web` via `/.well-known/did.json`; handle to DID
   through DNS TXT `_atproto.<handle>` or `/.well-known/atproto-did`, checked
   both ways against `alsoKnownAs`. The transport trait has `https_get` and
   `resolve_txt`, so the crate needs no DNS resolver; the CLI slice supplies
   one. Oracle: recorded audit logs (valid, forked, wrong-key) as fixtures.
3. **Ingest manifest (S–M)**. Records out verbatim as DAG-CBOR bytes with
   provenance `(did, collection, rkey, cid, commit rev, fetched_at)`; `rev`
   must not go backwards across ingests of the same DID; a re-ingest that no
   longer contains a record emits a deletion mark. Unknown collections ingest
   as opaque records. Oracle: two fixture commits of one repo, before and
   after a delete and an edit.
4. **CLI and store application (M)**. `id import atproto <handle|did>` with a
   seed list in config; reqwest-backed transport; applies the manifest to
   the store. Blocked on question 6 below.
5. **Re-ingest on a schedule (S, optional)**. Per-account `getRepo` with
   `since`, through the same verifier. Live relay follow stays out.

Slices 1 to 3 need nothing outside the new crate, and 1 and 2 can run in
parallel. Rough total for 1 to 3: two to three agent days.

## Defaults for the open questions on #13787

1. **PLC trust posture**: verify the audit-log chain per DID (b). It is
   per-account and cheap, and it is what makes plc.directory an availability
   service rather than an authority.
2. **Record modelling**: verbatim DAG-CBOR with a provenance index. Mapping
   `app.bsky.*` into id objects is a later, opt-in layer.
3. **First-slice scope**: a named seed list of accounts in config. Any DID on
   demand comes free once the CLI exists, but the seed list keeps the first
   evidence run reproducible.
4. **Live follow**: out of scope. If wanted later, per-account polling with
   `since` (slice 5) before any relay subscription.
5. **Blobs**: reference only (the blob CID inside the record), no fetch.
6. **Store path**: the crate emits a manifest and writes nothing, like
   `id-import-notion`. Whether slice 4 writes to `id-store` events directly
   or to a staging shape for the atlas source-registry line is the one
   question that needs an owner before slice 4; slices 1 to 3 do not depend
   on it.

## Decisions that are Drew's

- Go or no-go on the importer at all.
- Deletion policy: the default above honours deletions (a record removed
  from the repo is marked deleted in id). Keeping deleted records verbatim
  would be a deliberate archival choice and should not be a default.

Everything else above is a default an implementing lane can take.

## Risks

- **Key rotation**: a commit verifies only against the signing key in the
  current DID document. Older commits seen before a rotation cannot be
  re-verified later, so the manifest records which document version verified
  each ingest.
- **Spec drift**: the repository spec now names DRISL, the DASL CBOR subset;
  the decoder must reject what DRISL rejects (non-canonical encodings,
  floats in MST nodes), not just what `serde_ipld_dagcbor` accepts. Slice 1
  carries negative fixtures for this.
- **Size**: repos can hold millions of records. Slice 1's limits and a
  streaming CAR reader keep memory bounded; a full in-memory MST is fine for
  the seed list but not as the only mode.
