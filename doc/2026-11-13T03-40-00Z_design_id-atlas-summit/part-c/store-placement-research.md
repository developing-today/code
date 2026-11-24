# Store placement research — store ownership and identity seams (C9135-01)

- **Issue:** [#10167](https://github.com/developing-today/code/issues/10167) (slice C9135-01), parent [#9135](https://github.com/developing-today/code/issues/9135).
- **Statements:** S2.7, S2.8, S2.9. **Design basis:** [part-c versioned design](versioned-design.md), section "#9135 store placement adapter (S2.7–S2.9)".
- **Tier:** crucial. **Kind:** research-matrix (inventory only; no placement recommendation, no migration claim).
- **Baseline:** `main` at `3be23e4943af3527f8c3b4b7d895f39a5820ea51` (2026-11-23); source inspection only, no build, runtime, or deployment verification. "Implemented" below means source exists at that commit.
- **Blockers, re-checked fresh at this baseline:** #9082 (store abstraction ownership) CLOSED; 9110-01 is [#9418](https://github.com/developing-today/code/issues/9418) (store ownership/port map) CLOSED. No native blocker named by #10167 is open.
- **Machine-checked companion:** `store-placement-research-report.json` in this directory, validated by `validate_store_authority.py` (same directory) and by `pkgs/id/tests/store_authority_contract.rs`. Run:
  `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/validate_store_authority.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/store-placement-research-report.json --check-test`

## Verdict

ID does not have one store. It has four separately owned storage
authorities — blob (`id-store`), event (`id-events`), world
(`id-worlds`), sync (`id-sync`) — plus a metadata/index layer that
currently lives inside `id-store`. The design's five placement
surfaces (BlobStore, MetadataIndex, EventStore, WorldStore,
SyncStore) therefore map onto four current authorities, with
MetadataIndex not yet a separate seam. Each authority establishes
identity in its own way, at its own seam, before storage; none of the
four identity profiles is interchangeable with another, and none is
interchangeable with Atlas `content_hash`, whose producer remains
unavailable. Placement work (S2.7–S2.9) is per-surface or it is a
flattening.

## 1. Authority inventory

| Surface | Owner       | Seam (source)                                                                                                   | Backing storage                                                                                                     | Authority                                                    |
| ------- | ----------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| blob    | `id-store`  | `pkgs/id/crates/id-store/src/store.rs` — `StoreType::Persistent(FsStore)` / `Ephemeral(MemStore)`               | iroh-blobs `FsStore`: redb metadata plus blob data files under `.iroh-store/`; `MemStore` in memory                 | content-addressed blob bytes and name-to-hash tags           |
| event   | `id-events` | `pkgs/id/crates/id-events/src/event_store/mod.rs` — `EventStore` behind the `Storage` / `FileStorage` file seam | one redb database file per store directory; direct redb API use across `event_store/`                               | signed event log: chains, heads, fork proofs, waiting pool   |
| world   | `id-worlds` | `pkgs/id/crates/id-worlds/src/world_store.rs` — `FileJournal`, `ModuleDir`                                      | append-only `journal.jsonl` (fsynced per append in the default ack mode) plus `modules/<blake3>.wasm`; **not redb** | durable world state: journal replay and installed modules    |
| sync    | `id-sync`   | `pkgs/id/crates/id-sync/src/envelope_outbox.rs` — `Outbox`                                                      | append-only outbox journal file (`outbox.jsonl`), replayed on open                                                  | pending envelope delivery only; **no stored-data authority** |

Adjacent surface — **metadata-index** (`id-store`,
`pkgs/id/crates/id-store/src/tags.rs`): name, tag, and namespace
mappings over blob identity — alpha/omega iroh-docs namespaces, and
`.iroh-meta/registry.json` mapping logical names to namespace IDs.
It is not a fifth authority: a name is a mapping, never an identity.
The S2.7–S2.9 design separates MetadataIndex as its own placement
surface; in current code it sits inside `id-store` beside the blob
surface, so a placement adapter for blob storage does not
automatically place the index, and placing the index must not move
blob authority.

Non-authority is part of the inventory: the event store decides
nothing about blob bytes, the world journal decides nothing about
event-log heads, and sync owns no stored data at all — it moves
envelopes and log entries between the other surfaces and records
only what is still pending delivery. (This extends the 9110-01 seam
map in `pkgs/id/ARCHITECTURE.md`, "Store Port Boundaries", with the
identity facts that slice did not cover.)

## 2. Identity — where it is established, where it is only consumed

Identity is established by the owning crate, at its own seam, before
or at the moment of storage. Everything downstream — names, tags,
search, caches — consumes identity; it does not create it.

- **Blob content identity** is established by iroh-blobs at import:
  `Hash::new` over the exact bytes. File names and tags in `id-store`
  map names to that hash; they are not identity, and renaming a file
  does not change what it is.
- **Event identity** is established by the event header: the header
  commits `payload_hash` (p2panda `Hash::digest`, BLAKE3, plus the
  payload size) and the author's Ed25519 key, and chains to the
  previous header. Ingest verifies the body against the header
  commitment before anything is stored
  (`pkgs/id/crates/id-events/src/event_store/ingest.rs`: a body that
  is not the one the header commits to, by BLAKE3 hash and size, is
  refused). The store never assigns event identity; it keeps what
  the signed header established.
- **World identity** is split: the journal is an unhashed append-only
  log whose replay order is the authority, and installed modules are
  content-addressed — the module file name is the BLAKE3 hash of its
  exact wasm bytes, established at install.
- **Artifact identity** (the signed records `id-sync` transports and
  keeps, `pkgs/id/crates/id-sync/src/artifact.rs`) is established at
  authoring: the record hash is hex SHA-256 over `signed_bytes` —
  the canonical JSON of `SignedBody` — and the chain links
  author/seq/prev under the author's account key. Under
  `LEGACY_PROFILE` those bytes are never re-serialized, re-hashed,
  or re-signed; receipt refuses duplicates and noncanonical values
  rather than rewriting them.
- **Signer identity** (who may establish any of the above) is the
  Ed25519 node/account key. A valid signature proves key possession
  over bytes — not truth, authority, freshness, or permission;
  authorization is a separate decision at each seam.

Consumers that must not be mistaken for identity producers: the
tag/registry layer (names → hashes), the sync outbox (pending
deliveries), Atlas search (joins by `content_hash`, §5), and every
cache or index, which is a rebuildable projection over the
authorities above.

## 3. Exact hash/profile facts — stated without conflation

Four distinct typed commitments appear in this inventory. Two share
an algorithm name; so do the other two. A shared algorithm name does
not make two profiles interchangeable.

| Profile                | Algorithm          | Input bytes                                                 | Established by                                                                  |
| ---------------------- | ------------------ | ----------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `iroh-blob-blake3`     | BLAKE3-256         | exact stored bytes                                          | iroh-blobs `Hash::new` at import                                                |
| `event-payload-blake3` | BLAKE3-256         | raw event payload bytes                                     | p2panda `Hash::digest` committed in the signed event header; verified at ingest |
| `world-module-blake3`  | BLAKE3-256         | exact wasm module bytes                                     | module install; the hash is the file name                                       |
| `artifact-sha256`      | SHA-256            | canonical JSON bytes of `SignedBody` (`serde_json::to_vec`) | `SignedArtifact` signing at authoring                                           |
| `atlas-content-hash`   | SHA-256 (observed) | exact UTF-8 chunk text (observed)                           | **no checked-in producer** — see §5                                             |

Forbidden equivalences (checked mechanically by the validator and
the contract test): `iroh-blob-blake3` ≠ `event-payload-blake3`
(same algorithm, different input and different commitment site);
`artifact-sha256` ≠ `atlas-content-hash` (same algorithm, different
input bytes and different provenance); `iroh-blob-blake3` ≠
`atlas-content-hash`. The summit's `ContentId` / `PayloadId` are
design profiles over exact profile/payload bytes; no current code
maps any row above onto them, and this record does not create such a
mapping.

## 4. redb inventory

redb appears in this workspace at two major versions in one
lockfile, backing two different authorities by two different modes:

- `pkgs/id/Cargo.toml` pins the workspace dependency `redb = "=4.6.0"`;
  `pkgs/id/Cargo.lock` resolves **4.6.0** (root `id`, `id-events`,
  `id-store`, `id-web`, benchmarks, and iroh-blobs 0.103.1) and
  **3.1.3** (pulled in by iroh-docs 0.101.0, which resolves both).
  (The issue's evidence list cites the redb 4.3.0 API docs; the
  locked reality at this baseline is 4.6.0 / 3.1.3 as above.)
- **Direct use:** `id-events` calls the redb API itself
  (`Database`, `TableDefinition` across `event_store/`) — redb is
  the event log's storage engine.
- **Via iroh-blobs:** the blob surface's `FsStore` keeps its
  metadata in redb 4.6.0; `id-store` does not call redb for blob
  bytes, which live in data files.
- **Via iroh-docs:** the metadata/index namespaces ride on iroh-docs
  storage, which is where the second redb major version enters.
- **No redb:** `id-worlds` (journal files) and `id-sync` (journal
  files) use no redb at all.

Consequence for placement: "ID is on redb" is false as a
whole-system description, and "replace redb" is not one migration.
Each surface that touches redb touches it differently, and two
surfaces never touch it.

## 5. Atlas `content_hash` producer gap

Status: **producer unavailable** — recorded, not inferred. No
checked-in production producer computes the Atlas corpus chunk
`content_hash` in this repository; the value is declared in schemas
and consumed as a cross-store join identity (`pkgs/atlas-search`
crates and Python surface). The authority record is slice 9110-R2:
`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/research-results/atlas-content-hash-producer.md`.
The V1 audit's external-snapshot observation — stored values equal
SHA-256 of exact UTF-8 chunk text across the inspected corpora — is
a data fact about those snapshots, not a producer contract, and it
does not license an interoperability claim. Per the design basis,
until a producer is located and pinned with a known-answer vector,
**interop claims are forbidden**: in particular, no claim that an
Atlas chunk `content_hash` equals an ID blob hash, event commitment,
or artifact hash for the same bytes. A store placement design must
carry Atlas chunks as an external consumer identity with an
unpinned profile, not as a fifth row of the ID identity table.

## 6. The world journal is not a database

The world store's durable record is an append-only file:
`journal.jsonl`, one entry per line, fsynced per append in the
default ack mode, with compaction by sibling-temp-file + fsync +
rename. Its correctness contract is crash-prefix and replay: after
a crash, the committed prefix replays to the same state. Any
placement surface for WorldStore must preserve exactly that
contract — ordered append, per-append durability in the declared
ack mode, and prefix recovery — and must declare it in its
capabilities rather than emulating it silently over a key-value
API whose transaction scope differs.

## 7. Seams a placement implementation must respect

Numbered for citation by the implementation slices that follow.
Each seam names the authority that owns the decision; ownership of
the store abstraction and per-surface capability contract itself
sits with #9082 (closed design), and this record does not restate
or supersede it.

1. **Per-surface placement.** Placement is chosen per surface
   (BlobStore, MetadataIndex, EventStore, WorldStore, SyncStore in
   the design's terms). There is no placement decision for "the ID
   store"; a change that moves two authorities together needs its
   own justification per authority.
2. **Identity is established before storage, at the owning seam.**
   The design's `put_if_absent` verifies hash/signature/
   authorization before commit. Current seams already work this
   way (ingest refuses a body that fails the header commitment;
   blob identity exists before any name points at it). An adapter
   must consume the established identity, never re-derive it under
   a different profile.
3. **Never re-hash or re-sign legacy bytes.** Signed artifact bytes
   under `LEGACY_PROFILE` are immutable evidence: no re-serializing,
   re-hashing, or re-signing during placement, migration, or
   backend change. A backend that cannot store the exact bytes
   cannot hold the artifact surface.
4. **Exact-byte identity at the blob seam.** Blob identity is
   BLAKE3 over exact bytes; no normalization, transcoding, or
   re-chunking at the storage seam. (Document-layer validation —
   e.g. NFC media types — lives above the seam and does not change
   blob identity.)
5. **Declared capabilities, no silent emulation.** Per the design,
   a placement candidate declares `StoreCapabilities` (transaction
   scope, durability, read consistency, max object/db size, sync
   mode, conditional-write support) per surface, and may not
   promise semantics it cannot provide. Where a candidate cannot
   meet a surface's contract (e.g. the world journal's crash-prefix
   replay, §6), the answer is an explicit unsupported result, not
   an emulation.
6. **redb is per-surface too.** The event log's direct redb use,
   the blob metadata's redb-via-iroh-blobs, and iroh-docs' second
   redb major version are three separate facts (§4). A backend
   change for one says nothing about the others.
7. **Names are not identity.** The registry and tag layer map
   names to identities; placing or replacing that layer must keep
   the mapping rebuildable from the authorities and must never
   promote a name, path, or locator into an identity.
8. **Sync owns no authority.** The outbox is a delivery record.
   Placing sync storage is a durability-of-pending-delivery
   question only; no placement of it may make it a source of truth
   for blobs, events, or world state.
9. **Atlas stays external.** Until the 9110-R2 producer gap closes,
   Atlas `content_hash` is an unpinned external consumer identity
   (§5). Placement designs must not map it onto `ContentId`,
   `PayloadId`, or any profile in §3.
10. **Multi-writer prohibition stands.** Per the design, multi-writer
    object-store SQLite is prohibited unless a separately proven
    coordination protocol is specified; single-writer WAL
    replication is the SQLite-file candidate shape.

## 8. What this record does not claim

- No placement recommendation. The design's V4 rule applies:
  placement options are compared only after the V2 benchmark
  harness has equivalent workload evidence; until then the
  conservative default is retention of current per-surface
  authority. This record is the inventory that makes such a
  comparison possible, not the comparison.
- No generalized store migration claim. Nothing here asserts that
  any surface should move, that a candidate backend fits any
  surface, or that the five design surfaces should be built as
  written; those are implementation-slice decisions under #9135
  and #9082's contract.
- No deployment claim. All findings are source facts at the stated
  baseline commit.

## Corrections

Per the slice's rollback rule, this inventory is append-only:
errors are corrected by a dated correction row appended here,
never by rewriting the record or the identity history above.

| Date | Correction       |
| ---- | ---------------- |
| —    | none at issuance |
