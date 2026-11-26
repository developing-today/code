# Design: tenant data split — UserDO SQLite, per-user Turso, and D1 (#15482)

Design record for [#15482](https://github.com/developing-today/code/issues/15482),
gap 4 of the #9080 gap analysis (component B of #9078). Docs-only; no
implementation in this record.

Checked against main at `65e69239a` on 2026-11-26. The hosted worker is
`pkgs/id/hosted-worker/` (HW below). Inputs this record must answer rather
than restate: the #9083 tenancy architecture
(`doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture/`), the #5681
serverless storage decision (`doc/2026-11-10_decision_id5681_serverless_storage.md`),
and the #14866 Cloudflare-first ruling
(`doc/2026-11-25_design_cloudflare-first-migration.md` §2).

## 1. The decision in one table

Per data family of a hosted tenant:

| Data family                 | Lives in                                                                                    | Why (one line)                                                                                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Signed events and heads     | **Per-user Turso** (record); UserDO keeps ingress session state only                        | The tenant DB is the tenant's durable log a native peer syncs against (#9083 §2.2); the DO's SQLite ceiling and single-object throughput are the wrong growth path for an append-only log |
| KV                          | **UserDO SQLite** (unchanged)                                                               | Edge-local working state served only by the DO route; single-object consistency, no network hop, exported with the tenant                                                                 |
| Blob references             | **Per-user Turso** (record); bytes stay in **R2**; UserDO keeps a rebuildable serving index | #9083 §1/§2.2 place references in the tenant DB content layer; the DO index keeps the hot-path reference check local                                                                      |
| Usage / metering            | **UserDO SQLite** (detail, unchanged); rollup in the **central tenancy DB**                 | Already decided: #9083 §3 rule 4 and §9 — #3811 metering is authoritative per tenant, the tenancy DB holds the content-free rollup                                                        |
| Export state                | **UserDO SQLite** (unchanged)                                                               | Export is produced from the DO (#13157); the bundle is the portable artifact (#5869), the bookkeeping is DO operational state                                                             |
| World journals              | **Per-user Turso** (the tenant's world primary)                                             | #5681 §6.3: a world's primary is per-deployment and starts on Turso; hosted V1 does not evaluate worlds, it stores their synced signed envelopes in the same Turso event store            |
| Any per-tenant family in D1 | **None**                                                                                    | §4 below                                                                                                                                                                                  |

Nothing in this split moves content into the central tenancy DB; #9083
§2.3's never-central list is untouched.

## 2. Where the families live today

All of them are in the UserDO's SQLite, because the per-user Turso
database is provisioned by the tenancy plane (E2 #12797, landed) but no
store adapter serves it yet (E6 #12801, open):

- `meta`, `kv`, `blobs` tables in HW `src/user-do.ts` (versioned
  bootstrap); usage is derived from these tables on read
  (`/v1/usage`), and export reads them (`src/export.ts`, `user-do.ts`).
- `bridge_chains`, `bridge_events`, `bridge_forks`, `bridge_waiting`,
  `bridge_proof_ids`, `bridge_proof_headers` in HW `src/bridge-store.ts`
  — the signed-event store behind the bridge ingress.
- Blob bytes in R2 (`BLOBS` binding, `src/object-plane-r2.ts`); the
  `blobs` table is the reference record (hash, size, owner, created_at).
- Metering rollups land in the central tenancy DB (one Turso database,
  #9083 §2.1) — control plane, not a per-tenant family.

## 3. The split, family by family

### 3.1 Signed events and heads → per-user Turso

The signed journal is the system of record and every store is a
materialisation of it (#5681 §2, #5869). The question is only which
materialisation is the hosted tenant's durable one. #9083 §2.2 already
assigns the tenant's event log to the tenant DB; this record makes that
assignment operational:

- The tenant DB, behind the `store_traits` adapter (E6 #12801), holds
  the event log and the chain heads of record. A native instance
  syncing with its hosted tenant reads and appends against this store
  through the sync path (E3 #12798) — the hosted tenant is a peer with
  a full log, not a cache in front of one.
- The UserDO keeps the bridge's _session_ state: the waiting room,
  fork evidence, proof headers, and the watermark a hibernating
  session resumes from. All of it is rebuildable from the tenant DB
  plus the journal; none of it is the record. Fork evidence detected
  at ingress is recorded against the tenant-DB chain, not only in DO
  tables.
- Growth argument: the event log is the one family that grows without
  bound for an active tenant. DO SQLite is capped at 10 GB per object
  and serialises through one object; a per-user Turso database is the
  store #9083 provisioned for exactly this content layer.

### 3.2 KV → stays in UserDO SQLite

KV (`/v1/kv`) is small key/value working state of the hosted instance,
written and read only through the DO route. It is per-tenant already by
construction (one DO per tenant), needs no cross-peer sync of its own
(it is not journal content), and is included in the export bundle
today. Moving it to the tenant DB would put a Hrana round-trip on
every KV call and split the DO's single-object consistency for its
own state, in exchange for no isolation or durability gain. It stays.

### 3.3 Blob references → per-user Turso (record); bytes stay in R2

#9083 §1's plane table and §2.2 both place blob _references_ in the
tenant DB's content layer, and the access rule is program-wide: a user
may access an object iff their database holds a reference to it or it
is public-published. The reference record therefore belongs with the
tenant's other content records, where export, sync and GC reasoning
already look for it. The UserDO's `blobs` table does not disappear: it
becomes a rebuildable serving index so the hot-path reference check
stays local to the object serving the request. Bytes never move —
they are in R2 under #3811/#9954 and are not placed by this record.

### 3.4 Usage / metering → UserDO detail, tenancy-DB rollup (unchanged)

This family was already decided, and this record confirms rather than
re-decides it: #3811's metering stays authoritative per tenant
(#9083 §3 rule 4), derived on read from the DO's own tables
(`/v1/usage`), and the tenancy DB holds the nightly rollup — a
content-free aggregate, the one metering fact #9083 §2.3 permits
centrally. One bound this record adds: no per-event or per-object
metering rows are written to the tenant DB; the tenant DB is a content
store, not a second meter.

### 3.5 Export state → stays in UserDO SQLite

Export is produced from the DO (#13157, HW `src/export.ts`) and the
export bundle — journal + snapshot in an open format (#5869 §4) — is
the portable artifact. The bookkeeping behind it (export watermarks,
what was produced when) is operational state of the producing object
and stays with it. The split does create one real implication, named
in §5: after §3.1 lands, an export reads its events from the tenant
DB and its KV/meta from the DO, so the bundle must stamp both
watermarks.

### 3.6 World journals → per-user Turso

Worlds stay native for hosted V1 (#9080 gap analysis, V1 surfaces):
the hosted instance syncs their signed events and a native peer
evaluates them. For the tenant's own world, #5681 §3/§6.3 decides the
primary: per-deployment, starting on Turso. In tenancy terms the
per-deployment store _is_ the per-user Turso database, so a tenant's
world journal lives there, and the synced envelopes the hosted
instance holds are rows of the same Turso event store as §3.1. Two
escalations are inherited from #5681, not re-decided here: a world
that outgrows one database shards and is placed per-shard by §5.1 of
the #5681 record (Turso or D1 by size, heat and access pattern), and
cold shards sink to the home-lab tier. Both are corpus-scale events,
not per-tenant defaults.

## 4. D1's role, decided: no per-tenant family

D1 holds no per-tenant data family. Three independent reasons, any
one of which would suffice:

1. **#14866 §2's ruling.** The SQL plane is per data family,
   following its owning design; D1 is the plane for the fleet queue,
   decision events, and _new small state with no owning design_. ID
   tenant data has owning designs — #9083 and #5681 — and neither
   chooses D1 for it. This record is the owning design #14866 defers
   to for this data, and it does not choose D1.
2. **The credential model.** D1 access is a Workers binding; there
   is no per-tenant scoped credential (§5681 §4.7). #9083 §7's
   isolation rests on per-database Turso tokens minted per tenant.
   Per-tenant D1 would need a database and binding per tenant inside
   one account and would break that token model.
3. **#5681's assignment.** D1's chosen role is first-class placement
   for very large _shared-corpus_ shards (atlas), placed per-shard —
   a shared plane with Workers-only access, the opposite shape from
   per-tenant data that endpoints must reach with embedded replicas.

What would change this: a superseding ruling recorded on #9079 or
#5681 (the mechanism #14866 §2 itself names), or #5681 reversal
condition R1 — a landed spike showing Turso cannot expose a contract
hook for a family without weakening a journal guarantee. Absent one
of those, a PR placing a per-tenant family in D1 contradicts three
records and should be refused on that basis.

## 5. Migration and consistency implications

Named here so the implementation slices inherit them as requirements,
not discoveries:

- **One write path, one materialisation of record.** The UserDO is
  the only ingress writer. After the flip, its appends land in the
  tenant DB through the store adapter. There is no dual-write: at
  any moment a tenant's event log is DO-resident _or_ Turso-resident,
  and the `tenant_db` registry row (backend + state, #9083 §2.1) is
  where that fact is read from.
- **Movement is export/replay, once.** Rehoming a tenant runs the
  #5681 §6.5 / #5869 path — bundle out of the DO, replay into the
  tenant DB behind the storage contract, verify (snapshot + delta,
  signature check), flip the registry row, drain the DO's event
  tables after one retention interval. It is the same mechanism as
  exit, so every rehome rehearses exit. There is no live
  cross-store replication: per #5681 §4.2, per shard exactly one
  system replicates for availability, and the signed stream is the
  replication of record between stores.
- **Freshness is read-at-watermark.** Any reader of the Turso
  materialisation uses the #5863 contract's read-at-≥N primitive
  (implemented on the hosted store by #15483). After the flip the DO
  never serves a head it cannot prove against the tenant DB's
  watermark.
- **Two-source export.** Export (§3.5) stamps both the tenant-DB
  event watermark and the DO state watermark in the bundle; an
  import is complete only when both are replayed.
- **Ceilings move with the family.** The event log's growth leaves
  the DO's 10 GB object ceiling; the DO's remainder (KV, reference
  index, session state, export state) is bounded working state. The
  #5868 budget ladder, implemented by #15483, meters both planes —
  a tenant can now be at cap on either, and `export_only` remains
  the degradation (#9083 §4 step 9).
- **Nothing about the tenancy DB changes.** No content, key, token
  or per-object count becomes central as a side effect of this
  split (#9083 §2.3).

## 6. How this squares with the three inputs

- **#5681.** Followed, not amended: journal as system of record,
  stores as materialisations behind one contract, Turso default for
  world primaries, per-deployment granularity (here: per-tenant),
  per-shard escalation for oversized corpus stores, export/replay as
  the only movement mechanism. This record is the per-tenant
  application of that decision to the hosted families #5681 did not
  enumerate.
- **#9083.** Its §2.2 content layer is the Turso side of this split;
  its §9 phrase — the per-user Turso database as the tenant's data
  store _alongside_ the DO's SQLite — is the sentence this record
  resolves into per-family placements. Its metering rule (§3 rule
  4), credential model (§7) and never-central list (§2.3) are
  load-bearing here and unchanged.
- **#14866.** Its §2 per-family ruling is the reason D1 is answered
  the way §4 answers it: #14866 moves the _queue_ to D1 and keeps
  the board family in tenant Turso under #9079; ID tenant data
  follows the same pattern under its own designs. No #14866 slice
  changes target because of this record.

## 7. Who implements the move

- **E6 #12801** — the Turso/SQLite-over-HTTP adapter behind
  `store_traits`, with the same conformance tests against both
  backends. This is the enabling implementation for §3.1 and §3.3.
- **#15483** — the #5863 storage contract, the #5868 budget ladder
  and the #5867 backend manifest on the hosted store; the manifest
  is where the per-tenant placement fact is served from.
- **E3 #12798** — SyncCoord and hosted sync ingress, which read and
  append through the tenant DB store once the adapter exists.
- **Rehome slice (filed with this record)** — point the hosted
  worker's event/heads store and blob-reference record at the tenant
  DB via the adapter, run the §5 export/replay migration for
  DO-resident tenants, and make export two-source. Blocked by
  #12801 and #15483; provisioning of the tenant DB itself already
  landed (E2 #12797).

No other new slices: KV, metering and export state do not move, so
they need no implementation beyond the export change inside the
rehome slice.

## 8. Rejected alternatives

| Alternative                                              | Why rejected                                                                                                                                                                                |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Everything into the per-user Turso DB, DO as pure router | KV, metering detail and export state would pay a network hop per call and lose the DO's single-object consistency; contradicts #9083 §3 rule 4 (metering stays in TenantDO)                 |
| Everything stays in UserDO SQLite (status quo)           | The event log grows into the DO's 10 GB ceiling and single-object throughput; no embedded-replica story for endpoints; contradicts #9083 §9's "alongside" and leaves §2.2's tenant DB empty |
| Per-tenant D1 databases                                  | §4: breaks the per-database token model, contradicts #14866 §2's owning-design rule and #5681's D1 assignment                                                                               |
| DO as record, Turso as its replica                       | Two availability paths racing on one shard — forbidden by #5681 §4.2; the signed stream, not store replication, is the replication of record                                                |

## 9. Acceptance mapping (issue #15482)

- [x] Per data class, placement in UserDO SQLite, per-user Turso, or
      D1, with the reason for each — §1 and §3 (D1: §4).
- [x] Migration/consistency implications of the split named, not
      left implicit — §5.
- [x] Recorded in enough detail for the sibling implementation
      slices to proceed without re-deciding placement — §7 names the
      implementing issue per moving family, including E6 #12801 for the
      Turso adapter, as the issue's done-when requires.
- [x] How the decision squares with #5681, #9083 and #14866 — §6.

---

_Design record for #15482 (gap 4 of #9080, component B of #9078).
Relates to #5681, #5863, #5869, #9079, #9083, #12797, #12798,
#12801, #14866, #15483._
