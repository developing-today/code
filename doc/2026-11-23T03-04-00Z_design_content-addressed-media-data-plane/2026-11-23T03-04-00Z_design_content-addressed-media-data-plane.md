# Content-addressed media/data plane: reference-based access and GC (component F)

Status: proposed V1 design for component F of #9078 (hosted editions program), issue #9084. The recommendation in §3 is **proposed, not decided** — Drew posed the reference-vs-copy question as open (2026-11-12), so ratification stays with the design mill or Drew. Everything else in this document (contracts, tier table, GC safety protocol) is written to hold under any of the three placement models, so implementation slices filed from it do not need to wait for that ratification unless a slice says otherwise.

## Context and constraints

Media and large data in the hosted editions are content-addressed and deduplicated. The database estate is many Turso databases (per-user DBs plus a public DB, component E #9083); the object estate is R2 on the hosted tier and RustFS (S3-compatible) on the self-hosted tier (component I #9087). The central question (#9084): does content get copied into each database that uses it, referenced from each database while living once in the object store, or split by a hybrid rule?

Binding inputs, treated as inputs rather than decisions:

- Addressing is already specified by program #9106 Part A: content IDs and `hash.metadata` sidecars (#9113/#9114) and the harvested location index (#9115: all locations per hash). This design consumes those ID shapes; it does not invent a second addressing layer.
- Shared-bucket tenancy is designed in #9136 and GC large-file priority in #9137; this design states the contracts those must satisfy, not their internals.
- Component I (#9087) requires F's contracts to be adapter-friendly: provider-neutral immutable object identity, staged/checksummed publication, explicit replica and placement state, authorization-aware reads/fallback, outbox references rather than bytes, and reference/retention-safe GC. R2 and RustFS satisfy one contract; nothing here may assume R2-specific consistency, URL, multipart, or lifecycle behavior.
- Jurisdiction and sensitivity isolation come from #3401: dedup is only allowed inside a jurisdiction + sensitivity partition (see §4). Cross-partition "dedup" would leak existence and defeat isolation, so it is not a missed saving — it is prohibited.
- Atlas workstream F (#9098) measures the Atlas-specific case of this same plane; its verdict is an input to the threshold defaults in §3, not a dependency.

Binding constraints:

- A content hash is an identifier, never a credential. Knowing or guessing a hash grants nothing (§5).
- Database/reference state is authoritative; object bytes are a replica of record content, never the only record of who may access it.
- No client-visible protocol, identity, or authorization branch may depend on which object-store adapter or tier serves the bytes (component I peer principle).

## The three models

1. **Reference-only (Drew's sketch).** Every database holds references (content ID + metadata sidecar pointer); bytes live once per dedup partition in the object store. Access iff the caller's database holds a live reference, or a publication reference exists in the public DB. GC collects bytes when the last reference in the partition drops.
2. **Copy-into-each-DB.** Content bytes are stored as rows/blobs in each tenant database that uses them. No shared object plane for tenant content; the object store, if used at all, is per-tenant backup/export.
3. **Hybrid.** A placement rule splits content: small content inline in the referencing database, large content referenced in the object store; the split may also key on sensitivity or temperature instead of, or in addition to, size.

## Evaluation

| Axis | 1. Reference-only | 2. Copy-into-each-DB | 3. Hybrid (reference-first) |
| --- | --- | --- | --- |
| Storage cost under dedup | Best: one copy per partition regardless of fan-out; popular content costs O(1) | Worst: O(fan-out); the same media in 100 user DBs is 100 copies, inside DB storage priced/limited as database, not object storage | Near-best: large content (where dedup pays) is O(1); small inline payloads duplicate, but they are small by definition of the threshold |
| Access-control soundness | One choke point: the metadata/reference service mints capabilities after a reference check. Hash enumeration yields existence inside a partition at most, never bytes (§5) | Per-DB access is naturally isolated, but every copy is a new enforcement surface; a copy exported with a DB export bypasses object-plane controls entirely | Same choke point as (1) for referenced content; inline content inherits the referencing DB's access control, which is exactly the control that authorized the reference — no new surface |
| GC correctness | Hardest: liveness is a cross-database question; needs the mark-and-sweep protocol of §6. Reference counting alone is insufficient (drift, cross-DB transactions unavailable) | Trivial: content dies with its row/DB. No cross-DB GC exists | GC exists but only for the referenced tier, and §6 applies unchanged; inline content dies with its row |
| Deletion / privacy | A tenant deleting their reference never deletes bytes others still reference — correct sharing semantics, but a sole-referenced erasure and a shared-content erasure request are different operations and must be modelled differently (§6) | Deletion is exact per tenant; one tenant's erasure cannot affect another's copy — at the cost of the storage blow-up above | Same as (1) for referenced content; inline copies delete exactly, which is why sensitive small payloads may prefer inline placement (§4) |
| Latency | Extra hop: metadata check, then object read (or CDN for public). Mitigated by capabilities + CDN + hot replica class (#9087), not eliminated | Best possible: content is local to the query | Inline tier is local; referenced tier pays the hop only for large content, where transfer time dominates the hop anyway |
| What "published public" means | A publication reference in the public DB: an explicit, revocable reference record, not a flag on the bytes (§5) | Publication means copying bytes into the public DB — a second copy whose later edits/deletes can diverge from the source copy | Same as (1): publication is a reference, so unpublishing is reference removal, not a copy hunt |

## Recommendation (proposed)

**Adopt model 3, hybrid with reference as the default and inline as the bounded exception:**

- Content **above** the inline threshold, and all media, is reference-only in the object plane. The threshold is a per-partition policy value owned by component E, with a conservative starting default of **16 KiB** until #9098's measurements justify moving it; the threshold is policy, never protocol — changing it changes placement of new content, not any content's identity or access rule.
- Content **at or below** the threshold *may* be stored inline in the referencing database when its sensitivity tier permits (§4). Inline is an optimization for latency and exact deletion, not a second data plane: the inline bytes are still named by their content ID, and an inline payload can be promoted to a referenced object without any client-visible change.
- Publication is always a reference (never a copy), in every model variant above.

Why not pure reference-only: the GC and latency costs of the referenced tier are real, and for small payloads they buy nothing — dedup of a 2 KiB payload saves less than the reference machinery costs. Why not copy-into-each-DB: it abandons the program's central saving (dedup), prices media as database storage, and makes publication a divergent copy. The hybrid keeps the saving where it is large and the simplicity where the saving is negligible.

## Sensitivity tier table

Placement and access are decided per content class, inside a dedup partition of (jurisdiction, sensitivity tier). Content never dedups across partitions.

| Tier | Content classes | Placement | Access check |
| --- | --- | --- | --- |
| `public` | Published media, public assets | Referenced object; CDN-eligible replica class | A live publication reference in the public DB. CDN serves only under a capability or a publication-scoped signed URL minted from that reference; unpublishing removes the reference and invalidates the CDN path at the next sweep epoch (§6) |
| `tenant-private` | Ordinary user media/files, shared within a tenant's DBs | Referenced object, private bucket class, no CDN | A live reference in a database the caller is authorized for (component E check), then a short-lived, object+operation-scoped capability |
| `tenant-restricted` | Sensitive payloads (per #3401 classes), small operational secrets-adjacent data | Inline in the referencing DB if ≤ threshold, else referenced object in the partition's restricted bucket class; never CDN, never cross-jurisdiction dedup | The referencing DB's own authorization, evaluated at read time; for referenced objects, capability minting additionally requires the restricted class on the reference record |
| `staging` | In-flight uploads, unverified content | Staged object only (adapter staging area), not yet referenceable | Uploader session only. Publication (making it referenceable) happens only after checksum/size verification per the #9087 adapter contract; abandoned staging is reaped by staging TTL, never by GC |

## Access rule

A caller may read content `C` in partition `P` **iff** at least one holds, evaluated against authoritative reference state at capability-minting time:

1. a live (non-tombstoned) reference to `C` exists in a database the caller is authorized to read, and the reference's class permits the caller's operation; or
2. a live publication reference to `C` exists in the public DB (tier `public` only).

Then, and only then, the service mints a short-lived capability binding (content ID, operation, expiry, and byte range where the adapter supports it) and the adapter serves bytes from a verified replica, falling back to another verified replica during placement movement (#9087). Rules that make this sound:

- **The hash is not a capability.** Content IDs are hashes (#9106): they are enumerable in principle and leak by design (they appear in sidecars and the location index). Existence of a hash in a partition may be learnable; bytes are not served on hash presentation alone, ever.
- **References cannot be forged across databases.** A reference is a row in a database, written only through that database's authorized write path (component E). Holding write access to your own DB lets you reference content — and referencing is precisely the act that grants your DB's readers access, which is the model's intended semantics (it mirrors "attach a copy"). What it must never grant is access to a *different partition's* bytes: reference resolution checks the partition of the reference against the partition of the object, and restricted-tier references additionally require the restricted class to have been set by an authorized writer of the source content.
- **Revocation is reference removal.** Removing the last authorizing reference (or the publication reference) ends new capability minting immediately; already-minted capabilities expire on their own short TTL, which bounds the revocation lag and must be stated wherever a tier's revocation expectations are documented.

## GC design

**Home of reference tracking.** There is no separate reference registry. The references *are* the rows in the tenant and public databases (component E owns those schemas); GC reads them. A derived per-partition index (content ID → set of referencing DBs, built from reference-change outbox events, in the style of the #9115 location index) exists to make sweeps cheap, but it is a cache: the sweep's delete decision re-verifies against authoritative rows (§ below), so index drift can delay collection, never cause a wrongful delete.

**Why mark-and-sweep, not reference counting.** A stored counter is a second copy of the truth maintained across databases that share no transaction; every crash between "row written" and "counter incremented" drifts it, and drift in the delete direction destroys live content. Counters may be kept as a heuristic for *prioritizing* sweep candidates (#9137 large-file priority), never as delete evidence.

**Sweep protocol (per partition, per epoch):**

1. **Mark.** Open epoch `E`. Snapshot the partition's live reference set from authoritative DBs (or the derived index, with step 3 as the guard). Content with no live reference enters `candidate` state with the epoch number and a tombstone check timestamp.
2. **Grace.** Candidates wait a configurable grace period (starting default: 7 days for tenant tiers, 24 hours for `public`, policy-owned by E) spanning at least one full sweep of every database in the partition. New references to a candidate are legal at any time and simply clear the candidate state — referencing is idempotent and never needs to know GC exists.
3. **Re-verify.** At grace expiry, re-read authoritative reference state for the candidate. Any live reference → back to live, no delete. This re-check is what makes the derived index and the mark phase safe to be stale.
4. **Fenced delete.** Delete replicas only under the epoch fence: the delete operation records (content ID, epoch, re-verify timestamp) and the adapter performs a conditional/idempotent replica delete (#9087 contract). A reference created between re-verify and delete is the one remaining race; it is closed by ordering — reference creation for a content ID in `candidate` state past grace must pass through the same per-partition serialization point (E's outbox/sequence for that partition) that issues the delete fence, so a create either lands before the fence (seen at re-verify) or after it (its write path finds no verified replica and re-stages the bytes from the writer, who by definition holds them — a re-upload, not data loss). A delete of bytes a writer is concurrently re-referencing from a *read-only* position (no local copy) is prevented by step 3's grace: that reader had the whole grace period holding a live reference, which would have cleared the candidate.
5. **Audit.** Every delete emits an audit event (content ID, partition, epoch, last re-verify evidence). Object-store lifecycle rules may reap *staging* only; they must never independently expire published content.

**Deletion and privacy semantics.** Removing a reference is immediate and exact for the remover's access. Byte deletion is a GC outcome, not a user-facing promise, *except* sole-referenced content in `tenant-restricted`, where an erasure request escalates: if re-verify shows the requester was the sole referencer, the grace period may be shortened to zero by policy and the delete proceeds under the same fence. Where other references live, erasure of shared bytes is not mechanically possible without destroying other tenants' content; the honest operation is reference removal plus a recorded erasure note — this limitation is stated here so no later slice promises otherwise. Jurisdiction partitions (#3401) bound every sweep: a sweep runs inside one partition and can neither see nor delete another partition's replicas.

## Adapter and boundary requirements

- **For the object adapter (F's own contract, aligned with #9087):** `put_staged` / verify / publish, `head`, capability-scoped `read` with replica fallback, conditional replica `delete_replica`, and a capability matrix (conditional-write strength, range reads, multipart bounds). Publication is the service's verification step, never multipart completion.
- **For component E (#9083):** reference rows carry (content ID, partition, class, created-by, tombstone state); reference create/remove emits outbox events the derived index and GC consume; E owns threshold, grace, and tier policy values.
- **For #9136 / #9137:** shared-bucket tenancy must preserve partition isolation at the adapter layer (a bucket is a partition container, not a mixing point); large-file GC priority orders *candidates*, and may not shorten grace or skip re-verify.

## Risks and open measurements

- **Revocation lag** is bounded by capability TTL, not zero; tier documentation must not claim instant byte-level revocation.
- **Derived-index drift** delays collection (storage cost), bounded by periodic full-partition reconciliation sweeps; measure drift before tightening grace defaults.
- **Threshold default (16 KiB)** is a placeholder pending #9098's Atlas measurements and pm-stack media size distribution; the design's claim is that the threshold is policy, not that 16 KiB is right.
- **Public CDN invalidation** on unpublish is epoch-paced, not instant; if a future requirement needs instant takedown, it is a CDN-purge integration slice, not a change to the access rule.

## Implementation slices filed from this design

Filed as granular issues under #9084:

1. #12803 — Reference record schema + access-check in component E (reference rows, classes, tombstones, outbox events).
2. #12804 — Object-plane staged publication for F (adapter contract implementation for R2, conformance suite shared with RustFS).
3. #12805 — Derived reference/location index builder (outbox consumer; #9115 alignment).
4. #12806 — GC mark-and-sweep worker (epochs, grace, re-verify, fenced delete, audit events; per-partition).
5. #12807 — Publication semantics (public-DB publication reference, capability minting, CDN path, unpublish flow).
6. #12808 — GC race + revocation conformance tests (reference-during-sweep, sole-referencer erasure, cross-partition reference refusal).
