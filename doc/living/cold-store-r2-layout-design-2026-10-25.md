# Drive cold-store / Cloudflare R2 layout design

**Date:** 2026-10-25  
**Status:** Design only. This pass moved nothing, created no bucket or folder, changed no lifecycle rule, and spent nothing.  
**Scope:** Drew's Google Drive capacity under Google AI Pro, the existing Cloudflare R2 tenants, and the future path for data that may later be tiered from R2 or the lab cluster.

## Executive decision

Use the two stores for different jobs:

- **Google Drive is the cold, immutable archive vault.** It is already paid for through Google AI Pro, has 5 TiB of account capacity, and is best suited to large, sealed, rarely restored objects. It is not a CDN, query store, or high-churn file system. This is also the settled framing in `~/workspace/google-drive-cold-store/DESIGN.md`.
- **Cloudflare R2 is the hot and operational tier.** R2 is the place for current backups, served media, build/cache objects, payloads still being processed, and objects whose restore may be needed quickly. At the supplied planning rate of **$0.015/GB-month** and with free egress, R2 is inexpensive for working sets, but it should not become the default permanent home for multi-terabyte cold data when the Drive capacity is already paid for.
- **Keep the existing tenants and bucket boundaries.** Do not consolidate the R2 buckets merely for neatness. In particular, `supabase-backups` already has a bucket-scoped credential and a working content-addressed layout; the avatar backup already has its own Drive folder, manifest document, tags, and daily process.
- **Bundle cold Drive objects.** Drive's practical constraints are a 750 GB upload/copy limit per user per 24 hours and roughly three sustained writes/second. Those limits, recorded in `~/memory/2026-10-25.md`, make many-small-files the wrong shape. Cold data should be packed into immutable 4–16 GB bundles, with the searchable index held outside Drive.

## 1. Inventory — what is actually known

### 1.1 Google account storage: the “~732 GiB” number

A live Drive `about.get` check on 2026-10-25, recorded in `~/memory/2026-10-25.md`, reported:

| Measure | Bytes | GiB | Decimal GB | Interpretation |
|---|---:|---:|---:|---|
| Account quota | 5,499,705,622,528 | 5,120.00 GiB (5 TiB) | 5,499.71 GB | Google AI Pro storage tier |
| Total account usage | 785,960,403,110 | 731.98 GiB | 785.96 GB | The verified “~732 GiB used” figure |
| Usage in Drive files | 191,252,342,752 | 178.12 GiB | 191.25 GB | Only this portion is Drive-resident |
| Derived non-Drive remainder | 594,708,060,358 | 553.87 GiB | 594.71 GB | Gmail and Photos combined; the split is not established |
| Remaining quota | 4,713,745,219,418 | 4,388.02 GiB (4.29 TiB) | 4,713.75 GB | Available before other account growth |

Two consequences matter:

1. **The 732 GiB is not a Drive inventory.** Only 178.12 GiB is Drive files. The other 553.87 GiB is the combined Gmail/Photos remainder; no source read for this design establishes how much is Gmail versus Photos.
2. **A 3 TiB project budget remains reasonable.** The prior design reserves no more than 3 TiB for project cold storage, leaving about 1.29 TiB for personal/family growth and other uses. The budget and the 700 GB daily upload hard stop are also encoded in `~/workspace/google-drive-cold-store/schema.sql`.

### 1.2 Drive tenants that sources establish

| Tenant | Verified size / state | Evidence | Confidence |
|---|---:|---|---|
| **Muse Avatar Backup** | 46 recorded files; 67,013,227 bytes total (63.9 MiB); last successful backup recorded 2026-10-25 09:15:39 UTC | `~/workspace/goals/avatar-backup-to-google-drive/GOAL.md`; `~/workspace/.jarvis/idea-executions/bc7ea377-2b10-4e9a-b966-f417c6cb64c2/scratch/state.json` | High for the recorded backup set |
| **Cloud/cold-store folder tree** | The prior design records the root, `bundles/`, `manifests/`, `db-backups/` (with `supabase/`, `turso/`, and `d1/`), and `media-cold/` as created; no payload-byte inventory is given there | `~/workspace/google-drive-cold-store/DESIGN.md`, “Build status” | Layout recorded; payload size unverified |
| **All other Drive files** | At most about 191,185,329,525 bytes (178.06 GiB) after subtracting the recorded avatar set from total Drive usage | Derived from `~/memory/2026-10-25.md` and the avatar state file | **Unattributed.** Personal files, family files, project files, shared files, and any other app folders cannot be separated from the sources read |

That last row is the main inventory gap. No migration should assume the unattributed 178 GiB is project data, disposable data, or in scope for the cold-store. A folder-level inventory is a prerequisite to moving any existing Drive content.

### 1.3 R2 tenants are separate from the Google total

The latest bucket inventory cited in `~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/hidden_files/cloud-ideas-log.md` lists five R2 buckets:

- `agent-inbox-mail`
- `labops-files`
- `nix-cache-nars`
- `supabase-backups`
- `webhook-gateway-payloads`

The sources read for this design do **not** establish current byte totals for those buckets. Do not add an invented R2 total to the Google account total.

The one concrete database size is small: `~/workspace/supabase/muse-dev/backup/README.md` describes the `muse-dev` database as about 13 MB when that backup design was written and states that full copies were a rounding error against R2's storage scale at that size. The current total size and snapshot count of `supabase-backups` remain unverified in the sources read here.

### 1.4 Metadata planes are not bulk storage

D1, Turso, Postgres, and any future manifest database hold indexes, state, and pointers. They are operationally critical but are not where the bulk bytes should live. The cold-store manifest design in `~/workspace/google-drive-cold-store/schema.sql` models `objects`, `bundles`, `members`, a transfer ledger, and project-budget metadata. That index—not folder browsing in Drive—is the system of record for bundled data.

## 2. Target layout

### 2.1 Google Drive layout

Keep the existing avatar tenant outside the cold-store tree. Do not rename or reorganize it as part of this design.

```text
My Drive/
├── Muse Avatar Backup/                 # existing tenant; unchanged
└── Cloud/
    └── cold-store/
        ├── bundles/
        │   └── <yyyy>/                  # sealed 4–16 GB immutable bundles
        ├── manifests/                   # immutable manifest snapshots
        ├── db-backups/
        │   ├── supabase/
        │   ├── turso/
        │   ├── d1/
        │   └── postgres/                # proposed for non-Supabase Postgres
        ├── media-cold/                  # large originals not kept hot
        ├── snapshots/                   # proposed subdivision
        │   ├── cluster/
        │   ├── services/
        │   └── vms/
        └── inventory/                   # proposed: dated inventory exports
            └── <yyyy-mm-dd>/
```

The first four cold-store branches and the three named database subfolders are the layout recorded in `~/workspace/google-drive-cold-store/DESIGN.md`. `postgres/`, `snapshots/`, and `inventory/` are proposed additions in this layout, not claims that they already exist.

#### Drive rules

- Drive folders are a coarse physical layout only. Retrieval must go through the manifest; people and jobs should not need to browse thousands of objects.
- A bundle, database archive, or snapshot placed in Drive is **immutable**. Corrections create a new object and a new manifest record; they do not overwrite the verified object.
- Human-readable material that is intentionally browsed—especially avatar versions—may remain as individual files in its existing tenant. Bundling is for machine-managed archives.
- Drive trash is a grace state, not a lifecycle strategy. A monthly process may permanently remove only trash that has passed the class retention rule and the 30-day grace described below.
- No public Drive links are part of this design. Cold-store access is owner-authenticated backup/restore traffic.

### 2.2 R2 layout

Keep the existing five buckets. Add discipline through prefixes rather than creating a new bucket for every data class.

| Bucket | Target prefixes / role |
|---|---|
| `supabase-backups` | Keep the established database-backup layout: `pg/<project>/<yyyy>/<mm>/<sha256>.sql.zst`, plus `pg/<project>/manifest.json` as the mutable pointer. Add immutable manifest snapshots under `pg/<project>/manifests/<utc-timestamp>.json` if the backup tooling later supports them. |
| `labops-files` | General operational bucket. Use `artifacts/<project>/…`, `media/<project>/…`, `snapshots/<system>/…`, `rehydrated/<class>/…`, and `staging/<yyyy-mm-dd>/…`. Keep staging explicitly temporary. |
| `nix-cache-nars` | Build/cache data only. No archive role; objects here must be regenerable or re-downloadable. |
| `agent-inbox-mail` | Existing mail-payload tenant. Keep its current application layout; apply the retention decision in §4 rather than silently converting it into a permanent archive. |
| `webhook-gateway-payloads` | Existing webhook-payload tenant. Keep payloads hot only for the processing/debugging window, then delete or archive according to §4. |
| Any existing application bucket not listed above | Its application owner decides the prefix; this design only sets the rule that long-cold, sealed payloads are candidates for Drive bundles and hot/served payloads stay in R2. |

A manifest read replica may live at a reserved system prefix such as:

```text
labops-files/_system/cold-store/manifest/current.json
labops-files/_system/cold-store/manifest/snapshots/<utc-timestamp>.json.zst
```

That is a replica for the rehydration path, not the canonical database. The canonical manifest remains the database described by `~/workspace/google-drive-cold-store/schema.sql`, with immutable snapshots also stored in Drive.

### 2.3 Assignment by data class

| Data class | Primary home | R2 role | Drive role | Why |
|---|---|---|---|---|
| **Database backups** | **Both, by age** | Latest restore points and the active backup stream | Independent cold copies of selected daily/weekly/monthly restore points | R2 gives fast, scriptable restore and already contains the `supabase-backups` system. Drive adds an independent failure domain without paying again for every frequent snapshot. |
| **Avatar renders and versions** | **Drive, existing folder** | None by default | Permanent version archive with the existing manifest document and tags | Tiny (63.9 MiB recorded), human-browsed, tag-driven, and already protected by a working daily process described in `~/workspace/goals/avatar-backup-to-google-drive/GOAL.md`. Moving it would add risk for no material saving. |
| **Media originals** | **Drive when cold; R2 when served or recent** | Current/served media, thumbnails, proxies, and rehydrated originals | Sealed bundles of originals that have left the hot set | Drive is the economical permanent home; R2's free egress and CDN adjacency are wasted on originals nobody is requesting. |
| **Cluster / VM / service snapshots** | **Both, by criticality and age** | Latest boot-critical snapshots and recent operational restore points | Monthly or milestone archives in sealed bundles | A rebuild should start from a fast source. Old snapshots are insurance, not working data. |
| **Project archives and exports** | **Drive** | Manifest replica, recent exports still being shared or processed | Sealed bundles with a manifest record for every member | Archives are large, immutable, and rarely read—the exact workload Drive is reserved for. |
| **Build artifacts and caches** | **R2 only** | Current cache/artifacts | None | Churny and regenerable. Sending caches to Drive spends upload/write budget and creates archive clutter. |
| **Mail and webhook payloads** | **R2 during their useful window** | Processing, replay, debugging, and dispute handling | Only payloads explicitly marked for long retention; otherwise no Drive copy | Payloads can contain sensitive or low-value data. Indefinite retention should be a decision, not an accident of storage layout. |
| **Manifests and indexes** | **Database first; replicas in both stores** | Read replica for restore/rehydration | Immutable snapshots as disaster recovery for the index itself | If the manifest is lost, Drive becomes opaque bundles. The index must be easier to restore than the data it describes. |

## 3. Naming and object conventions

### 3.1 General rules

- Use lowercase kebab-case tenant and system names: `supabase`, `turso`, `d1`, `lab-cluster`, `media`, `snapshots`.
- Use UTC timestamps in names that represent a point in time: `YYYYMMDDTHHMMSSZ` (for example, `20261025T221500Z`).
- Use `<yyyy>/<mm>` partitions for date-ordered series. Do not create a folder or prefix per day unless the expected object count requires it.
- Never encode a secret, personal name, email address, or opaque database row ID in a public-facing key when a tenant/system/date/hash path will do.
- A mutable name is allowed only for a pointer such as `manifest.json`, `current.json`, or `latest.json`. Everything a pointer references must be immutable and independently addressable.

### 3.2 Bundles and members

Retain the bundle model from `~/workspace/google-drive-cold-store/DESIGN.md`:

- Physical bundle name: `b_<first-16-hex-of-bundle-sha256>.tar.zst`.
- Target bundle size: 4–16 GB; seal at the size threshold or after 24 hours, whichever comes first.
- A bundle contains individually framed members so one member can be range-fetched without downloading or scanning the whole bundle. Offsets and lengths are manifest data, not filename data.
- The manifest records, for every object: full SHA-256, byte size, content type, source bucket/key when it came from R2, state, bundle ID, member offset, and member length. This follows `~/workspace/google-drive-cold-store/schema.sql`.
- The bundle filename is only a checksum claim. It becomes trusted after the manifest records the full bundle SHA-256 and the upload verification in §5 passes.

### 3.3 Database backups

Retain the established `supabase-backups` key shape from `~/workspace/supabase/muse-dev/backup/README.md`:

```text
pg/<project>/<yyyy>/<mm>/<sha256>.sql.zst
pg/<project>/manifest.json
```

For other databases, use the same pattern with a system-specific first segment:

```text
db/<system>/<database>/<yyyy>/<mm>/<utc-timestamp>-<sha256-prefix>.dump.zst
```

The manifest—not the filename—is authoritative for the full hash, source host/project, schema or migration version, row/object counts when available, transport, and restore instructions.

### 3.4 Snapshots and media

Recommended logical names before bundling:

```text
snapshots/<system>/<yyyy>/<mm>/<system>-<utc-timestamp>-<version>.zst
media/<collection>/<yyyy>/<mm>/<sha256>.<extension>
artifacts/<project>/<yyyy>/<mm>/<artifact-name>-<utc-timestamp>.zst
```

For media, the manifest should also preserve the original filename/path as metadata. The physical archive identity remains the hash; user-friendly names belong in the index and any human-facing manifest.

## 4. Lifecycle and retention rules

These are **recommended defaults for a future implementation**, not rules changed by this design pass.

| Class | Hot / R2 rule | Cold / Drive rule | Deletion rule |
|---|---|---|---|
| Database backups | Keep the latest successful backup plus the last 30 days in R2. | Copy one sealed restore point per day; retain daily × 30, weekly × 12, monthly × 12. | A source snapshot may leave R2 only after the Drive copy is verified. Permanent deletion follows the retention schedule, with trash grace where the store supports it. |
| Avatar versions | No routine R2 copy. | Keep all recorded versions indefinitely. “Favorite” versions are never auto-deleted or overwritten. | Manual decision only. This preserves the tag model in `~/workspace/goals/avatar-backup-to-google-drive/GOAL.md`. |
| Media originals | Keep served/recent originals and derivatives in R2. A rehydrated original may be evicted after 30 days without access. | Keep originals indefinitely unless Drew sets a collection-specific rule. | Do not delete the Drive original merely because a proxy or thumbnail exists. |
| Cluster/service snapshots | Keep daily snapshots 14 days and weekly snapshots 8 weeks. | Keep monthly snapshots 12 months; keep named milestone snapshots until Drew retires them. | Expire by manifest state, not by filename guesswork. |
| Project archives | Keep an R2 copy only while actively shared or processed, default 90 days. | Keep according to the project’s own retention; review annually. | Tombstone in the manifest first; remove the physical bundle only when no live members remain or a verified successor bundle exists. |
| Build/cache objects | Lifecycle-expire after 30–90 days, by cache type and last use. | None. | Safe to expire without Drive verification because the class is regenerable. |
| Mail/webhook payloads | Default 180 days for processing/debug history. | Archive only items explicitly flagged for legal, financial, or project-history reasons. | Delete unflagged payloads at expiry; do not bundle everything “just in case.” |
| Manifest snapshots | Keep the current replica and the last 90 days of snapshots in R2. | Keep monthly snapshots indefinitely and daily snapshots for 90 days. | Never delete the last verified manifest snapshot for a bundle that still exists. |
| Staging / rehydration temp objects | Delete after 7 days unless tied to an active verified restore. | Not applicable. | A failed or abandoned staging object must not be mistaken for an archive copy. |

### Bundle compaction

- When a bundle's live bytes fall below 30% of its total bytes, it becomes a compaction candidate (the threshold is from `~/workspace/google-drive-cold-store/DESIGN.md`).
- Re-pack live members into a new bundle, verify the new bundle, update the manifest, and only then trash the old bundle.
- Compaction uploads count against the same daily Drive budget as new ingest. New backups take precedence over compaction.
- Permanent deletion of trashed bundles waits 30 days.

## 5. Restore and verification story

An archive copy is not “done” when an upload reports success. It is done when a restore path has been proven for that class.

### 5.1 Verification at ingest

For every object or bundle:

1. Compute SHA-256 before or while streaming the source.
2. Record the expected size and hash in the manifest in a non-final state (`staging`, `sealed`, or `uploading`, following the states in `~/workspace/google-drive-cold-store/schema.sql`).
3. Upload with a resumable mechanism where the platform supports it.
4. Compare the stored byte count with the expected count.
5. For Drive bundles, cross-check the Drive-reported MD5 recorded in the manifest. MD5 is a useful provider-side cross-check; it does not replace the SHA-256 verification.
6. Download or range-read the object/member back, decompress it if framed, and verify its SHA-256 before marking it verified/cold.
7. Only after that verification may the manifest state change to cold and any R2 source become eligible for lifecycle removal.

Failure at any step leaves the source in place. A failed Drive copy is staging debris, not a backup.

### 5.2 Ordinary cold-object restore

1. Look up the object hash in the manifest.
2. Identify the verified bundle and the member's offset/length.
3. Range-pull only that member's bytes from Drive.
4. Decompress the member frame and verify the object SHA-256.
5. Put the verified object into its original R2 bucket or the appropriate `rehydrated/` prefix.
6. Mark it hot in the manifest and restart its hot-retention clock.
7. Serve from R2, not directly from Drive.

For a small object, the restore may complete within the triggering request. For a large object, return a “warming” state and let the client retry into the R2 copy. The important invariant is **fill and verify R2 first, then serve**.

### 5.3 Database restore

1. Select the backup from the manifest, not by choosing the newest filename by eye.
2. Download the backup and verify that its SHA-256 matches the manifest and, for the established Supabase layout, its content-addressed filename.
3. Restore into a scratch database or isolated target—never directly over production as the first test.
4. Verify the migration/schema version and application-level counts or smoke queries.
5. Record the restore target, elapsed time, backup hash, and checks performed.
6. Only a separately approved production restore may replace a live database.

The Supabase backup README at `~/workspace/supabase/muse-dev/backup/README.md` already documents the shape of this flow and a 2026-10-21 verification in which 13 tables / 4,362 rows restored to 4,362 insert statements plus migrations. That is the standard to repeat: content hash plus a real restore, not merely a downloadable file.

### 5.4 Avatar restore

1. Choose the version by its friendly label and tags in the avatar manifest.
2. Download the Drive file.
3. Compare its SHA-256 and size with the avatar backup state.
4. Confirm the image decodes and, for an animation, that the animation opens/plays.
5. Record the restore in the avatar state; a restored older version becomes current by tag/state, not by overwriting its historical file.

### 5.5 Drills and scrubbing

Recommended defaults:

- **Monthly:** verify one randomly selected bundle or at least 1% of cold bytes, whichever is smaller, plus verify that the current manifest snapshot can be opened and queried.
- **Quarterly restore drill:** restore three random cold objects and one database backup into scratch targets; verify hashes and application-level checks.
- **After any migration stage:** verify 100% of the objects moved in that stage before the source is eligible for deletion.
- **Annually:** run a broader bundle-verification campaign within the 750 GB/day Drive limit and reconcile the manifest against both stores.
- **Restore log:** keep a dated record with class, object/bundle ID, hashes checked, destination, elapsed time, and result. A failed drill blocks further source deletion for that class until resolved.

## 6. Migration order if Drew later approves movement

Each stage is independently reversible. No stage deletes a source merely because a copy was attempted.

| Stage | Work | Verification gate | Rollback |
|---:|---|---|---|
| 0 | **Inventory only.** Produce a folder-level Drive inventory and a byte-accurate R2 bucket/prefix inventory; classify the unattributed Drive data as in scope, out of scope, or unknown. | Drew reviews the classes; unknowns default to out of scope. | Nothing moved. |
| 1 | **Manifest first.** Stand up or confirm the manifest, restore the manifest from a snapshot in a test, and register existing tenants without moving their payloads. | Manifest opens from backup; avatar and Supabase backup records reconcile to their existing manifests/state. | Remove the test index; original tenants unchanged. |
| 2 | **Database cold copies.** Copy one selected daily restore point per database from R2 to Drive on the chosen cadence; leave every R2 backup in place. | Hash verification plus one scratch restore per database. | Stop copying; trash the unverified or unwanted Drive copies after review. R2 remains authoritative. |
| 3 | **Already-cold R2 archives.** Bundle a small pilot set of project archives, verify them in Drive, and keep the R2 originals through a grace period. | 100% member hash verification; one pilot restore from Drive to R2. | Point the manifest back to R2 and stop; no source has been deleted yet. |
| 4 | **Media originals.** Seed cold originals to Drive in campaigns below the 700 GB/day hard stop recorded in `~/workspace/google-drive-cold-store/schema.sql`; keep hot/served media in R2. | Bundle and member verification; sample playback/restore per collection. | Keep serving from the existing source; Drive bundles remain an extra copy or are trashed after review. |
| 5 | **Snapshots.** Add monthly/milestone Drive copies while retaining the current R2 restore window. | One cluster/service restore test from the Drive copy into a scratch target. | Continue restoring from R2; Drive copies are additive. |
| 6 | **Lifecycle automation.** Only after stages 2–5 pass, let verified cold objects age out of R2 under §4. Introduce compaction after lifecycle removal is stable. | At least one quarterly drill passes with the automation enabled. | Disable lifecycle rules; rehydrate or retain objects in R2. Drive archives remain intact. |
| 7 | **Optional account graduation.** Revisit a dedicated archive-owner Google account only if the triggers in the prior design occur: seeding needs exceed a day’s window, daily deltas routinely exceed roughly 300 GB, or Drew wants the main account credential separated from always-on workers. | A new design decision, not an automatic consequence of this layout. | Stay on the main account layout. |

## 7. Monthly cost model

### 7.1 Assumptions

- R2 storage is modeled at **$0.015 per GB-month**, using decimal GB for billing-style calculations.
- The table is **storage-only**. It does not invent a request/operation total because the sources read do not establish current R2 operation counts or a byte-accurate R2 inventory.
- Google AI Pro is an existing bundle, not a cold-store-only purchase. Its price is recorded in `~/memory/2026-10-25.md` as $19.99/month or $199.99/year. If Drew keeps AI Pro for its other benefits, the incremental cost of using otherwise-free Drive capacity is $0. If the plan were bought solely for storage, allocate the full plan price instead.
- “2×” means twice the stated payload, not a forecast.

### 7.2 Storage scenarios

| Scenario | Current payload | If stored in R2 at $0.015/GB-mo | 2× payload | If 2× stored in R2 | Recommended-home cost |
|---|---:|---:|---:|---:|---|
| Current Drive-resident files | 191.25 GB | **$2.87/mo** | 382.50 GB | **$5.74/mo** | Incremental **$0** in Drive under the existing AI Pro plan |
| Entire current Google account usage (counterfactual; Gmail/Photos are not a simple R2 migration) | 785.96 GB | **$11.79/mo** | 1,571.92 GB | **$23.58/mo** | Not applicable as a single class; classify first |
| Proposed project cold-store budget | 3 TiB = 3,298.53 GB | **$49.48/mo** | 6 TiB = 6,597.07 GB | **$98.96/mo** | Drive incremental **$0** until the cap; **2× exceeds the current free headroom and is not valid without a new quota/plan decision** |
| Recorded avatar backup set | 0.067 GB | **<$0.01/mo** | 0.134 GB | **<$0.01/mo** | Incremental **$0** in its existing Drive folder |

The economic conclusion is not that R2 is expensive for today's small operational sets—it is not. The conclusion is that **Drive wins decisively at the multi-terabyte cold tier**, while R2's cost remains acceptable for hot data, caches, current backups, and rehydrated objects.

### 7.3 Budget guardrails for a future build

- Project cold-store soft alert at **2.7 TiB** and hard design cap at **3 TiB** in Drive.
- Daily Drive upload hard stop at **700 GB**, preserving margin below the official 750 GB/day limit.
- Track R2 bytes by bucket/prefix monthly. Any R2 class that is cold, sealed, and larger than 100 GB should either have a Drive migration stage or a written reason to stay hot.
- Review the unattributed Drive inventory before raising the 3 TiB cap. The cap is a project-allocation rule, not a claim that the whole account belongs to projects.

## 8. Open decisions for Drew

All of these can wait for a future build decision. The recommended defaults are chosen so that “no answer” means “keep the safe layout.”

1. **Archive-owner account**  
   *Decision:* Keep the main Google account as archive owner, or eventually graduate to a dedicated family account?  
   *Recommended default:* **Main account for now.** Revisit only on the stage-7 triggers. This matches `~/workspace/google-drive-cold-store/DESIGN.md`.

2. **Project storage cap**  
   *Decision:* Confirm the project cold-store allocation.  
   *Recommended default:* **3 TiB cap, alert at 2.7 TiB, 700 GB/day upload hard stop.**

3. **Existing Drive inventory scope**  
   *Decision:* Which existing Drive folders are project data, personal data, or out of scope?  
   *Recommended default:* **Inventory first; unknown means out of scope.** No existing Drive payload moves based on this design alone.

4. **Database retention split**  
   *Decision:* Confirm how long database restore points stay hot and cold.  
   *Recommended default:* **R2 latest + 30 days; Drive daily × 30, weekly × 12, monthly × 12.**

5. **Payload retention**  
   *Decision:* How long should mail and webhook payloads remain available for replay/debugging?  
   *Recommended default:* **180 days in R2, then delete unless explicitly flagged.** Do not archive payloads by default.

6. **Media retention**  
   *Decision:* Should media originals be permanent, or should collections get individual expiry rules?  
   *Recommended default:* **Originals permanent in Drive; R2 derivatives/rehydrated copies expire after 30 days without access.**

7. **R2 bucket structure**  
   *Decision:* Create a new general cold-store bucket, or keep the existing application buckets?  
   *Recommended default:* **Keep the existing five buckets and use prefixes.** Create a new bucket only for a distinct credential, public-access, or lifecycle boundary.

8. **Avatar tenant**  
   *Decision:* Fold avatar backups into the cold-store tree, or leave the current tenant alone?  
   *Recommended default:* **Leave `Muse Avatar Backup` unchanged.** Register it in inventories; do not reorganize a working, tag-based backup.

9. **Restore service level**  
   *Decision:* How fast must cold restores be?  
   *Recommended default:* **Database scratch restore tested quarterly; ordinary cold objects restored to R2 the same day; no minute-level RTO for archives.** Pay for faster recovery only for classes Drew names as operational.

10. **Permanent deletion authority**  
    *Decision:* Which classes may a future lifecycle job permanently delete without a fresh prompt?  
    *Recommended default:* **Only regenerable caches and expired unflagged payloads.** Backups, media originals, snapshots, and archives require manifest tombstoning plus the stated grace period; avatar favorites are manual-only.

## Source map

- `~/memory/2026-10-25.md` — live Drive `about.get` figures; official Drive limits summarized there; Google AI Pro price; framing that Drive is cold/immutable and R2 stays hot.
- `~/workspace/google-drive-cold-store/DESIGN.md` — prior Drive/R2 design v0.1: identity layout, bundle model, manifest role, reaper/rehydrator flow, compaction, restore drill, and recorded build status.
- `~/workspace/google-drive-cold-store/schema.sql` — manifest tables, bundle/member addressing, 3 TiB budget, and 700 GB daily upload hard stop.
- `~/workspace/goals/avatar-backup-to-google-drive/GOAL.md` — avatar Drive folder, manifest document, tag/favorite model, and rollback purpose.
- `~/workspace/.jarvis/idea-executions/bc7ea377-2b10-4e9a-b966-f417c6cb64c2/scratch/state.json` — recorded avatar file count, sizes, hashes/state, and last successful backup timestamp.
- `~/workspace/supabase/muse-dev/backup/README.md` — `supabase-backups` bucket, content-addressed key, manifest, bucket-scoped credential location by name, restore flow, and historical restore verification.
- `~/workspace/goals/supabase-muse-dev-platform-backups/GOAL.md` — ownership of the muse-dev platform and the R2 backup system.
- `~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/hidden_files/cloud-ideas-log.md` — Cloudflare account inventory, including the five R2 bucket names.
