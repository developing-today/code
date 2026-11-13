# Research: Part A graph and social primitives

- **Research date:** 2026-11-12 (America/Chicago); external pages retrieved on this date.
- **Scope:** #9125–#9132, S5.1–S5.64 in the [proto note](../../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md#5-graph--user-attributes--social-apps).
- **Boundary:** source review + a deterministic stdlib encoding micro-experiment; the only executable added is the toy harness; no production-code change, production benchmark, or deployment inspection.
- **Provenance rule:** “evidence” below names a source fact; “inference” names a design conclusion, not an empirical result.

## Executive result

The repo already has signed-object, ordered-index, bounded query, board, version-DAG, moderation, and delegation precedents. These are building blocks, not proof that the proposed universal triple graph or append-list protocol is implemented. V2 should add a shared logical contract while keeping existing domain APIs intact. Storage format and indexes should be benchmarked separately: signed wire bytes are not a DB file format. Start with a bounded triple-pattern API; named language and visualization breadth belong later. Append-list acceptance order is server-local. Versioning is immutable and branching. Deletion is local policy, never global erasure. Delegation, identity equivalence and client-side signer validation remain distinct.

## #9125 — Triple terms, encoding, and storage alternatives

### Repo evidence

- `pkgs/id/crates/id-core/src/tuple.rs:1–27,74–89` defines sort-preserving typed tuples and prefix-scan key encoding. It is a primitive/index precedent, **not** an arbitrary array-address or graph encoding.
- `pkgs/id/crates/id-store/src/tag_index.rs:3–16,90–236` uses subject-first, value-first and author-first indexes, rebuilt from live claims and scanned by prefix. This shows an implemented indexing pattern; it is not evidence of graph-scale economics.
- `pkgs/id/crates/id-store/src/tags.rs:61–69,187+` keeps tag values as arbitrary bytes; JSON serialization is lossy for non-UTF-8. Do not mistake text JSON for a canonical term model.
- `doc/2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md:99–102,172–181` requires bounded graph neighborhoods and explicit truncation.
- `doc/2026-11-10_design_id5865.md:117–142,241–255` uses SQLite-family tables/BLOBs for exact journal bytes; this is byte-fidelity precedent, not a triple benchmark.

### Alternatives (evidence vs inference)

| Option | Evidence | Tradeoffs / disposition |
|---|---|---|
| Pretty UTF-8 JSON documents | Human-readable and broadly toolable, but the task explicitly excludes pretty JSON as canonical storage. JSON serialization of arbitrary bytes is not type-preserving without extra framing. | Inference: good diagnostic/export view, poor signed canonical wire bytes and potentially costly escaping/parsing. Keep as a projection/export only. |
| Canonical typed binary records, independently framed; optional block compression | IETF CBOR RFC 8949 (retrieved 2026-11-12) defines typed binary data, byte strings, tags, and indefinite-length arrays/maps/strings for streaming; it also warns its data model is trees/arrays/maps, not graph cycles. Repo already uses canonical CBOR in signed event bodies (e.g. `doc/2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md:31–45`). | Inference: strong candidate for wire interchange and independent record decoding; storage still needs indexes. Compression across a whole graph harms partial access; use independently compressed blocks with a manifest/index if measured. CBOR is not automatically the chosen term model or RDF conformance. |
| SQLite JSONB | Official SQLite JSON documentation (`https://www.sqlite.org/json1.html`, retrieved 2026-11-12) says JSONB avoids text parse/render and is somewhat smaller/faster than text; SQLite JSONB is private/opaque to SQLite, not PostgreSQL-compatible, and most operations remain O(N), not O(1). | Evidence rules out treating SQLite JSONB as portable canonical replication bytes. Inference: valid backend-specific materialization for bounded documents; not a strong primary index for unbounded triple scans. Must verify the exact Turso/libSQL version before relying on feature parity. |
| Normalized typed rows / byte-key→byte-value maps | Repo has tuple-encoded ordered keys and prefix scans (`tuple.rs`, `tag_index.rs` above). | Inference: selective lookups and streaming range scans are explicit; alternate access orders require extra indexes and write amplification. Robust candidate for storage backend; benchmark index cost and transaction semantics. |
| One compressed monolithic graph blob | No repo evidence supporting it. | Inference: simplest compact archive, but individual point lookup / query paging can require broad decompression. Reject for live primary access unless chunk boundaries and index recover bounded reads. |

### Deterministic stdlib micro-experiment (toy, not a production benchmark)

**Question tested:** how do compact JSON, framed/deflate binary, a toy normalized key/value encoding, and this local SQLite JSONB serializer represent one small repeatable typed-byte triple corpus? SQLite JSONB is included in the measured toy serialization output using tagged octet arrays. This does **not** instantiate redb or Turso, a production CBOR codec, any query/storage index, or concurrency/durability behavior; it is not an engine candidate benchmark.

**Method:** executable [`graph-encoding-toy.py`](graph-encoding-toy.py) using Python 3 stdlib (`json`, `struct`, `zlib`, `hashlib`, `sqlite3`) via `python3 -I`. It accepts no positional args and only optional `--check-oracles`; argument parsing with abbreviations disabled happens before corpus creation, SQLite connection, or output. Exact equality checks use explicit exceptions and run in both normal and optimized Python; `--check-oracles` adds a summary line. A mutation probe corrupts one decoded row and verifies the independent equality check rejects it. Generate exactly 1,000 records, `i=0..999`: subjects repeat over `urn:user:0..99`; predicate cycles `name`, `follows`, `comment`; object is a matching `User n`, deterministic follow URI `(i*17+3)%100`, or deterministic `b3:` hex value `(i*2654435761) mod 2^32`. The reference corpus is the ordered sequence of byte triples. Encode as (1) compact JSON list of UTF-8 strings, (2) 1-byte record tag + three 4-byte big-endian length-prefixed byte fields per triple, (3) same framed binary then zlib level 6, (4) toy normalized byte-map: sort tuples, encode each component as type-tag + length-prefixed bytes into composite key, then length-prefix key and empty value, (5) actual local SQLite JSONB storing each term as a tagged octet array so the exact same byte triples round-trip without assuming UTF-8. SHA-256 hashes identify exact raw bytes. An additional typed probe round-trips non-UTF-8 bytes `00 ff`, `80`, `fe 00`. SQLite JSONB availability/version is printed; its encoding is engine-private. The script checks exact byte equality against the reference corpus for every format before printing any PASS line. Raw stdout from `--check-oracles` is saved in [`experiment-output.txt`](experiment-output.txt).

**Raw output (single run, deterministic):**

```text
corpus=1000 triples; deterministic=subject urn:user:0..99, predicates{name,follows,comment}, fixed formula objects; oracle=exact tuple equality (map order normalized)
compact_json_utf8: bytes=37498 sha256=30b605e93ea4dfef1985d96a43246e9762743ddd17c2934672adb764309f078d
framed_binary_typed: bytes=39497 sha256=1df438ae052fbe6662139e87dc31861960683537100e290e41ecb9abbb8660e8
framed_binary_zlib6: bytes=4194 sha256=7440d2a9f1e7ac1692ef9d63d393c4cfe1f885672968f611b99d6e13698d691a
normalized_byte_map_typed: bytes=49497 sha256=5815ef18990a2c1307041d02cabfe11c51e6d44eb6f91cd04eaf31b3dad317e7
sqlite_jsonb_typed_octet_arrays: bytes=115656 sha256=1cba765a5f74c2f6554b9adbac1a34406beeb8e854c92ec0b251cd46f2e10290
ratios_vs_compact_json=framed_binary_typed:1.053 framed_binary_zlib6:0.112 normalized_byte_map_typed:1.320 sqlite_jsonb_typed_octet_arrays:3.084
sqlite_version=3.53.3; sqlite_jsonb_available=yes
roundtrip=PASS for compact JSON, framed binary, compressed binary, byte map, and SQLite JSONB; mismatch=0
arbitrary_non_utf8_typed_bytes=PASS (00 ff / 80 / fe 00); probe_bytes=91
mutation_rejection=PASS
check_oracles=PASS; explicit equality and mutation-rejection checks ran
```

The original four pre-SQLite raw digests are reproduced exactly by the current fixture; the fixture was not changed. SQLite JSONB is measured locally using the same typed byte terms encoded as tagged arrays of octets. Its size is highly representation-dependent (octet arrays are intentionally explicit, not a JSON string shortcut); the SQLite JSONB blob is private to SQLite and not canonical interchange. This result says only that this encoding is larger on this corpus. The toy map is a serialized map, not a real DB with indexes. The result does not establish production space/latency ranking: repeated URIs make the corpus highly compressible; no redb/Turso candidate-engine benchmark, scale sweep, query latency, IO, write amplification, index, concurrency, cold-cache or tail-latency test was run. The binary framing is not CBOR, and “streamable” needs a real codec + page/index experiment.

**Next meaningful benchmark:** run real candidate engines/adapters at several graph sizes and skew distributions, with the same canonical byte triple corpus. Measure engine file size including all indexes, insert/update cost, point lookups in each leading component order, selective scans, first-row and page latency, rows/bytes visited, and cold/warm behavior. Include arbitrary bytes, null/absent, typed literal distinctions, version/provenance records, and index rebuild. Correctness oracle remains the in-memory typed set/query reference, not timing.

## #9126 — Graph query and visualization alternatives

### Repo and standards evidence

- Proto note S5.8–S5.12 puts query, visualization and metadata-through-graph in scope; default is triple-pattern API before language selection (`note.md:82–84,145–158`).
- `doc/2026-11-08_decision_id5909_sql_pgq_capability.md:25–57,67–90,126–149` discusses typed pattern IR, backend lowering, bounded graph patterns, and limitations of current redb/SQLite PGQ support.
- `doc/2026-11-08_design_id5837.md:14–23,40–55,68–76` recommends bounded subgraph queries, server-side auth, visible result caps. `doc/2026-11-08T08-10-00Z_design_id5782_graphical_client_tooling.md:99–102,140–181` recommends a neighborhood explorer with explicit truncation and list/matrix fallbacks.
- W3C RDF 1.1 Concepts (`https://www.w3.org/TR/rdf11-concepts/`, retrieved 2026-11-12) defines RDF graphs as sets of subject-predicate-object triples with typed terms, and says RDF graphs are static snapshots; an RDF data model is not implied merely by using a 3-field tuple.
- W3C SPARQL 1.1 Query (`https://www.w3.org/TR/sparql11-query/`, retrieved 2026-11-12) specifies basic graph patterns plus OPTIONAL, UNION, negation, arbitrary-length property paths, aggregates, subqueries, and federated-query relations. That is a substantially larger semantic surface than a basic pattern matcher.
- Cytoscape.js docs (`https://js.cytoscape.org/`, retrieved 2026-11-12) document a client graph-rendering/analysis library. Renderer is not a query engine or storage authority.

### Options and recommendation

1. **Typed triple-pattern API / IR first (recommended).** Easy to bound, expose stable validation, and lower to redb prefix scans or parameterized SQL without promising portable SQL text. Tradeoff: not immediately interoperable as a named query language.
2. **SPARQL subset or full SPARQL.** Standards-based, meaningful only if RDF term semantics and RDF interchange are required. Full language brings substantial semantics and cost/authorization questions, and it does not automatically fit custom ID references, provenance or tenancy. Recommendation: evaluate in V4 after V2 API becomes a reference oracle.
3. **SQL/PGQ.** Useful logical/backend target where supported; current backend mix lacks a uniform PGQ execution surface, so it should not be the only portable API.
4. **GraphQL or traversal DSL (Gremlin-like).** Can form convenient application-facing reads or path traversals, but leaves recursion, query budgets and graph semantics to project-defined rules and creates an additional API surface.

Visualization must consume authorized paged query results and source provenance, mark cycles/references, and expose truncation. It must not fetch an unbounded whole graph or silently sample while presenting a complete view. Language and visual layout remain adapters over same semantics.

## #9127 — User attributes and append-list protocol

### Repo evidence

- Proto note S5.13–S5.29 explicitly distinguishes user attributes from Atlas/location metadata; requires signed object+hash submission, centrally ordered acceptance, explicit opened status, and pending external submissions (`note.md:86–88`).
- `pkgs/id/src/boards/records.rs:44–74,90–148` has typed thread/post structures and signed board-related records; adjacent pattern, not an append-list contract.
- `doc/2026-11-08T13-45-00Z_design_id5857_curated_board_submissions/2026-11-08T13-45-00Z_design_id5857_curated_board_submissions.md:72–133,135–180` derives pending/approved views from signed submissions and decisions and separates intake from post-publication moderation.

### Options and recommendation

1. **Server-sequenced append log + derived list view (recommended).** Producers sign object bytes; server validates, then stamps acceptance/order. Clear fit for S5.20–29. Order is per server/attribute log, not global federation order.
2. **Client-authored list mutation.** Offline-friendly, but submitter controls list/order, contrary to requirement.
3. **CRDT/mergeable list.** Converges offline, but ordering conflicts with centrally ordered append absent explicit sequencer. Defer rather than quietly mix.
4. **Mutable pending flags.** Operationally convenient but can diverge; prefer signed state-transition records + deterministic projection.

Open details to settle at implementation handoff: duplicate object policy, queue quotas/rate limits, review role source, cross-server sequence semantics, rejection retention and pending visibility.

## #9128 — Social object model and freshness

### Evidence and alternatives

- Proto S5.30–40 requires object nesting, feed/profile/forum-like surfaces, comments-on-comments, addressable topics, and freshness cursor (`note.md:90–92`).
- `pkgs/id/src/boards/records.rs:44–74,150–160` models thread/post parent links and forks. Graph-tooling sources above demand bounded neighborhood/page output. W3C ActivityPub Recommendation (`https://www.w3.org/TR/activitypub/`, retrieved 2026-11-12) defines actor inbox/outbox collections, ordered collections and pagination; useful prior art, not a drop-in replacement for ID signing/identity.
- RFC 9110 §8.8, §13 (`https://www.rfc-editor.org/rfc/rfc9110.html`, retrieved 2026-11-12) provides HTTP validator/conditional-request concepts, but an ETag does not define this system’s scope, feed ordering, retention or federation behavior.

Alternatives: separate app-owned tables (simple UX but fragmented authority); object references + query projection (recommended, composable); ActivityPub-compatible model (interop option but changes actor/protocol semantics). A freshness token based only on timestamp can miss clock skew, equal timestamps and deletes. The proposed cursor position is a durable, monotonically allocated projection-ledger sequence for the complete `(server, tenant, query, projection-generation)` feed, not any source's local sequence. Each projected entry retains source authority, source attribute, epoch and sequence as provenance; snapshot cursor pins a vector of source frontiers. The same projection ledger orders entries from different source attributes even if both are at source sequence 1. Access/moderation changes advance the projection generation and invalidate old cursors. This proves freshness only for that server's selected view/frontier, not disconnected peers globally. Expired cursor requires restart/full refresh, never false “unchanged.” The separate [`feed-cursor-oracle.py`](feed-cursor-oracle.py) exercises source-attribute sequence collisions, different source epochs, restart, snapshot cutoff, stable paging and policy-generation invalidation as a deterministic model, not a runtime/replication claim.

## #9129 — `succeeds` versioning and old comments

### Evidence and alternatives

- Proto S5.41–45 requires `succeeds`, default latest display and old-version comment carry-forward markers (`note.md:92–94`).
- `doc/2026-11-09T03-57-04Z_design_id_edit_fork_graph/2026-11-09T03-57-04Z_design_id_edit_fork_graph.md:49–107` models immutable revisions in a DAG and exposes concurrent branches rather than silently resolving. `pkgs/id/src/boards/records.rs:162–172` has explicit immutable `Supersede` records.
- `doc/2026-11-01_design_id2054.md:52–54,91–102` similarly preserves signed immutable posts and forks.

Options: mutable record + version counter (easy but loses immutable provenance); linear last-writer-wins successor chain (simple but unsafe under concurrency); immutable `succeeds` DAG with surfaced competing heads and explicit merge (recommended, aligns prior art). Comments stay anchored to exact version; inherited display is a projection labeled with its origin, not a rewritten target. Carry-forward policy must state transitivity and treatment of deleted/resolved comments.

## #9130 — True deletion and moderation

### Repo evidence

- `doc/2026-10-30T19-35-00Z_investigation_boards_model/2026-10-30T19-35-00Z_investigation_boards_model.md:145–179` distinguishes signed view moderation, host withholding and stored bytes; rejects global deletion/retroactive history rewrite.
- `doc/2026-11-01_design_id2054.md:29–30,62–72,91–109` separates stored/served/counted/rendered, local host availability, board projection and personal overlay. `doc/2026-11-10T20-12-00Z_design_id_content_model/2026-11-10T20-12-00Z_design_id_content_model.md:20–24,62–63` favors immutable revisions and tombstone identity. `doc/2026-11-10T20-00-00Z_design_agent_note_placement.md:42,50` treats retraction as relation, not silent deletion.
- Matrix redactions and server-server API (`https://spec.matrix.org/v1.13/client-server-api/#redactions`, `https://spec.matrix.org/v1.2/server-server-api`, retrieved 2026-11-12) are primary protocol examples of redaction/removal semantics that do not promise erasure of every retained copy.

Alternatives: universal deletion consensus (unrealistic with disconnected/hostile replicas); immutable-only log (audit-friendly but may not satisfy server legal/safety rules); local true-delete under explicit server policy with separate signed request/moderation states (recommended). GC requires reference/retention checks; stale/cyclic/attacker-created references and abuse by pinning are risks. Surface local deletion/withholding without claiming universal removal.

## #9131 — Client-bound signer spaces, petnames

### Evidence and alternatives

- Proto says array positions `[null, user public ID]` makes space client-validatable and petnames are distinct (`note.md:96`).
- `doc/2026-11-01_design_id2054.md:30,50` treats verifying key as author identity and petnames as local. `pkgs/id/crates/id-core/src/petname.rs:1–6` states petnames stay in owner’s file and are not replicated.
- Iroh Documents docs (`https://docs.iroh.computer/protocols/documents`) distinguish public-key namespace write authority from application-level author semantics; endpoint docs (`https://docs.iroh.computer/concepts/endpoints`) treat endpoint key identity separately from app authorization. Retrieved 2026-11-12.

Alternatives: server-asserted owner (simple but server becomes identity authority); key-derived/canonical binding plus client verification (recommended; less human-readable); local petname overlay for human UX. Positional sentinel alone is not proof: signed canonical bytes, exact encoding/version, signer key verification are required. Key proof establishes control/attribution—not a person, reputation, completeness, availability, or SSO identity.

## #9132 — Delegation, transfer, cross-signing

### Evidence and alternatives

- Proto separates grants and transfers and proposes mutual publication for cross-signing (`note.md:96`).
- Existing delegation design `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md:254–283,757–762` distinguishes explicit authority chain, attenuation/revocation and successor transfer; a separate review at `.../2026-10-11T08-08-56Z_design_review.md:59` stresses validation of each link.
- UCAN delegation spec (`https://ucan.xyz/delegation`, retrieved 2026-11-12) describes proof-chain principal alignment and attenuated delegated authority without transferring private keys. Matrix cross-signing spec (`https://github.com/matrix-org/matrix-spec/blob/main/content/client-server-api/modules/end_to_end_encryption.md`, retrieved 2026-11-12) distinguishes master/self-signing/user-signing relationships.

Options: one “owner” key only (simple but no scoped delegation); delegation-as-transfer (conflates continuing grant and replacement); explicit scoped delegation, accepted transfer and bilateral identity-link claim (recommended). Mutual signatures prove both keys approved the link statement; they do not prove two keys belong to the same person or merge accounts. Unilateral claims stay pending/unverified; invalid scope, revoked grant and conflicting links must remain visible.

## V1 boundary and overlap notes

The summit baseline audit [`baseline/v1-audit.md:7–36`](../baseline/v1-audit.md) is the governing source audit. It records existing domain-specific signed events, Iroh namespaces/sync/relay, group/world authorization, discovery, local/remote query, tuple indexes, redb-backed blob store and distinct authoritative stores, plus separate Atlas backends. Correct summary: **domain-specific namespace/sync/authorization mechanisms exist; a uniform cross-domain namespace registry and generic federation protocol are not evidenced.** It also warns current ingress does not demonstrate a universal signature gate; this design’s requested invariant is a proposed new invariant, not a claim about V1.

Current [`baseline/v1-audit.md:11–18,30–40`](../baseline/v1-audit.md) gives exact identity profiles: Iroh blob ID is unkeyed BLAKE3-256 over exact bytes; event payload commitment is BLAKE3; signed artifact is SHA-256 over `serde_json::to_vec(SignedBody)` with Ed25519, and its documentation preimage description mismatches inspected implementation; world module ID is Iroh BLAKE3 with domain-prefixed signed digest. Keep these as distinct types/profiles. The same audit says Atlas `content_hash` is schema/consumer-only here; no checked-in production producer or byte/profile contract was found. This needs an owner-assigned follow-up to define source bytes, algorithm/profile and compatibility vectors before any new producer relies on it. Do not infer a generic unknown hash algorithm where the exact missing boundary is the Atlas producer.

Current [`baseline/seam-map.md:8–14,18–22`](../baseline/seam-map.md) cautions that signing does not equal authorization and new namespace/graph work must coexist with existing domains. #9077 owns SSO equivalence; #9083 tenancy; #9084 ref-vs-copy remains open. It records #6277/#6287–#6290 as open staged designs linked to #6312 plan-of-record, but not settled architecture. Preserve legacy signed artifact JSON bytes and add compatibility vectors; never silently rehash or re-sign.

## Retrieval log and limitations

| Retrieved | Source | Use / limitation |
|---|---|---|
| 2026-11-12 | RFC 8949, https://www.rfc-editor.org/rfc/rfc8949 | CBOR types, streamable indefinite items, extensibility; not this app’s chosen wire contract. |
| 2026-11-12 | SQLite JSON1/JSONB docs, https://www.sqlite.org/json1.html | Private SQLite JSONB behavior and stated complexity; does not prove Turso/libSQL parity. |
| 2026-11-12 | W3C RDF 1.1 Concepts, https://www.w3.org/TR/rdf11-concepts/ | RDF triple/set/term semantics; using triples alone does not declare RDF. |
| 2026-11-12 | W3C SPARQL 1.1 Query, https://www.w3.org/TR/sparql11-query/ | Language feature surface. |
| 2026-11-12 | Cytoscape.js, https://js.cytoscape.org/ | Client visualization capabilities; not independent graph storage. |
| 2026-11-12 | ActivityPub, https://www.w3.org/TR/activitypub/ | Collections/federated social prior art; not drop-in. |
| 2026-11-12 | RFC 9110, https://www.rfc-editor.org/rfc/rfc9110.html | Conditional request validators; does not define app cursor semantics. |
| 2026-11-12 | Matrix redaction specs, URLs above | Redaction and retained-copy limits. |
| 2026-11-12 | Iroh Documents/Endpoints, URLs above | Namespace/transport key distinction; application policy remains separate. |
| 2026-11-12 | UCAN, https://ucan.xyz/delegation; Matrix cross-signing spec, URL above | Delegation chain and distinct key relationship patterns. |

No redb, Turso, PostgreSQL or SQLite production/candidate-engine benchmark was run. No issue body was treated as implementation evidence. The experiment is a reproducible toy serialization-size/round-trip check only; it gives no production throughput, latency or economics result.
