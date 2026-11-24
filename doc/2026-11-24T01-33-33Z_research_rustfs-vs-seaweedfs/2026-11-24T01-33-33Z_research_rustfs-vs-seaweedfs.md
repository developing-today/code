# RustFS vs SeaweedFS — object storage compared

Research record, 2026-11-24. Question: for an S3/object layer, how do RustFS and
SeaweedFS compare on benchmarks, features, durability record, and project
health — and (supplemental section at the end) which fits a Cozystack-based
estate. Every factual claim carries its source and the date it was checked.
Vendor numbers are labelled vendor numbers; nothing here blends incomparable
benchmark runs into a ranking.

## Verdict

- **SeaweedFS wins** on small-object write throughput and latency in the only
  independent head-to-head found, on object-count scaling (its packed-needle
  metadata model), on breadth beyond S3 (FUSE, WebDAV, SFTP, HDFS, CSI, filer
  sync, cloud tiering, built-in Iceberg/Lance table buckets), and on a decade
  of production exposure.
- **RustFS wins** on large-object throughput (roughly 3× SeaweedFS on 8 MiB
  PUTs in the same test), on resource footprint (~4.3× less RAM, ~1.6× less
  CPU in that test), on operational simplicity (one binary, no metadata
  database to size and back up), and on erasure coding applied at write time
  with flexible parity.
- **Overall call for a Cozystack-shaped estate: stay on SeaweedFS**, which is
  Cozystack's built-in S3 layer. The data-loss record (section 5) is real but
  concentrated in maintenance tooling, single-copy configurations, and
  metadata-store failures — not in acknowledged writes on replicated volumes —
  and it has concrete guardrails. RustFS is a credible watch/pilot candidate
  for standalone large-object workloads outside Cozystack, to be re-evaluated
  after it has a year of GA exposure; its 2025–2026 security record argues
  against trusting it yet with the platform's only copy of anything.

## 1. The two designs

**SeaweedFS** (Go, Apache-2.0 core plus a proprietary Enterprise edition) uses
a Haystack design: small files are packed as "needles" into append-only volume
files, at roughly 40 bytes of on-disk metadata overhead per file and a 16-byte
in-memory index entry per blob, giving O(1) one-seek reads. Large files are
split into chunks of a few megabytes, each stored as a blob, with a meta chunk
listing the chunk file ids. Hot data is replicated (rack/DC-aware three-digit
replication codes such as 001/010); warm data is converted in the background
to Reed-Solomon RS(10,4) erasure coding at 1.4× storage overhead, tolerating
four shard losses — the open-source build fixes the ratio at 10+4, custom
ratios are Enterprise. Metadata lives in two places: the volume layer in the
master (Raft), and the **filer database, which is the scaling ceiling** — one
row per object on the S3 path, with pluggable backends (LevelDB `leveldb2`
default, PostgreSQL, MySQL, Redis, Cassandra, etcd, TiKV, FoundationDB, and
more). (Project wiki and docs, checked 2026-11-23.)

**RustFS** (Rust, Apache-2.0, no paid tier yet) is a MinIO-design
reimplementation: per-object Reed-Solomon erasure coding with an inline
`xl.meta` metadata file per object per shard and no metadata database;
objects under 128 KiB are stored inline. That is fast per object, but very
large object counts become billions of tiny files — inode pressure and
ListObjects scan cost are its ceiling (a May-2026 small-object scalability
analysis rated SeaweedFS the default open-source answer at 30 billion objects
and RustFS, then pre-GA, as not yet that answer). Erasure sets run 2–16
drives with automatic parity (EC:0 on 1 drive up to EC:4 on 8–16 drives; a
16-drive set is 12 data + 4 parity, ~75% efficiency; parity never exceeds
N/2). Erasure blocks are 1 MiB, checksummed with HighwayHash-256 — the same
family and defaults as MinIO, so MinIO-migrated objects stay readable. **Pool
topology is fixed at initialisation**: a mismatch is a permanent
`PoolTopologyMismatch` startup failure. The production layout is multi-node
multi-drive (≥4 nodes); single-node layouts exist for small deployments.
(Project docs, checked October–November 2026.)

**Hashing and splitting, specifically.** SeaweedFS splits large files into
few-megabyte chunk blobs referenced by file id from a meta chunk; integrity
at the storage layer is needle CRC-class checks, and the filer/S3 layer
carries the standard S3 checksums and etags. RustFS splits every object
across erasure shards in 1 MiB blocks with HighwayHash-256 per-shard bitrot
checksums, plus the full S3 checksum algorithm family on copy operations
(CRC32/64, SHA-1/256/512, MD5, xxHash, per its compatibility matrix).
**Neither system offers content deduplication as a storage feature**
(checked 2026-11-24): chunking in both is splitting, not dedup, and
SeaweedFS's "dedup" references in its codebase are read-path request
coalescing, not content addressing.

## 2. Benchmarks

**There is no trustworthy like-for-like distributed head-to-head.** The
closest evidence, in descending order of comparability (full extracts in
`supplement-benchmarks.md`):

### 2.1 Independent single-node head-to-head (the anchor)

One independent test (iamkunal9, published 2026-10-19) ran RustFS 1.0.3,
SeaweedFS 4.48 (`weed mini`), and the last MinIO community release on
identical single-node conditions, defaults only, median of five runs:

| Workload | RustFS | SeaweedFS | MinIO |
| --- | --- | --- | --- |
| 64 KiB PUT (MiB/s) | 56.8 | **99.4** | 49.7 |
| 64 KiB GET (MiB/s) | **149.5** | 114.0 | 121.7 |
| 8 MiB PUT (MiB/s) | **325.5** | 106.9 | 318.8 |
| 8 MiB GET (MiB/s) | **455.8** | 410.3 | 426.6 |
| 256 MiB multipart PUT (MiB/s) | 276.4 | 125.7 | **323.8** |
| 256 MiB GET (MiB/s) | 1,036.3 | 580.8 | **1,279.6** |

Latency (p99): 64 KiB PUT — SeaweedFS 39.3 ms beats RustFS 61.6 ms; 8 MiB PUT
— RustFS 158.3 ms beats SeaweedFS 580.5 ms. Resource use: SeaweedFS peaked at
~1.4 GiB RAM, about 4.3× RustFS's 348 MiB (MinIO 394 MiB), and used ~1.6×
RustFS's CPU time in every run. Durability under the same test: **zero
acknowledged objects lost or corrupted across six `kill -9` cycles per store
(26,111 writes)**. S3 compatibility spot-checks: each store passed 14 of 15
(RustFS and SeaweedFS both drop metadata signed into presigned PUT URLs;
MinIO lacks per-bucket CORS).

Reading: SeaweedFS's design centre (small files) shows exactly where the
design says it should — small-object PUT throughput and latency. RustFS's
MinIO-class engine shows on large objects. Neither result generalises to a
distributed cluster; treat the ratios as single-node evidence only.

### 2.2 Community 4-node warp comparison (dated, alpha-era RustFS)

A RustFS GitHub discussion (#1500, crawled 2026) compared MinIO
RELEASE.2024-10-29, RustFS 1.0.0-alpha.79, and SeaweedFS 4.02 on four nodes
(32 cores, 128 GB, one SATA SSD each), dual-replica writes, `warp mixed`:
SeaweedFS GET throughput was far ahead at small sizes (4 KiB ≈ 76–112 MiB/s
vs RustFS ≈ 12.6; 32 KiB ≈ 497–920 vs ≈ 100), large-size GETs converged at
≈ 1.8–1.9 GiB/s for both, RustFS small-size PUTs were very low (≈ 4.2 MiB/s
at 4 KiB), and **RustFS crashed at 512 concurrency on 1 MiB objects**. The
RustFS build was an alpha; this is evidence about that era, not about 1.0.x.

### 2.3 Vendor and project numbers (labelled, not comparable)

- RustFS's original "2.3× MinIO" claim is the project's own benchmark (2-core
  Xeon Platinum 8475B, 4 GB RAM, 15 Gbps network, 4× 40 GB NVMe, 4 KB
  objects). Its Beta.10 blog (build dated 2026-07-17; Azure, 4 nodes × 4
  disks, 8 CPU/16 GB, warp, 64 concurrent, 5 minutes) reports PUT ahead of
  MinIO at all 11 object sizes (up to 2.85×) with the GET gap "narrowing";
  independent summaries of mid-2025 testing still had MinIO ahead on reads
  (53 Gbps vs 23 Gbps on 20 MiB objects). The vendor's 323 GB/s read /
  183 GB/s write cluster figures are evaluation claims, not third-party
  results.
- SeaweedFS's project numbers are labelled unscientific by the project
  itself: MacBook SSD, `weed benchmark`, 1M × 1 KB files at concurrency 16 —
  15,708 writes/s (p50 0.8 ms, p99 2.6 ms), 47,019 random reads/s (p50
  0.3 ms, p99 0.7 ms); a local warp mixed run totalled 3,302.88 MiB/s
  (GET 2,477 MiB/s at 10 KB-class objects).

## 3. Features, side by side

| Area | SeaweedFS | RustFS |
| --- | --- | --- |
| Chunking | Few-MB chunk blobs + meta chunk for large files; small files packed as needles | Every object erasure-split in 1 MiB blocks across a set |
| Dedup | None (storage layer) | None |
| Durability scheme | Replication for hot data (rack/DC-aware); background RS(10,4) EC for warm data, fixed ratio in OSS, 1.4× overhead, tolerates 4 losses | Reed-Solomon EC at write time, automatic parity EC:0–EC:4, up to ~75% efficiency at 16 drives |
| Metadata | Master (Raft) + filer DB (pluggable; the scaling ceiling; one row/object on S3 path) | Inline `xl.meta` per object per shard; no metadata DB; inode/ListObjects cost is the ceiling |
| S3: versioning | Yes | Yes |
| S3: Object Lock | Yes (governance/compliance/legal hold) | Yes — but see the 2026 bypass CVE in section 5 |
| S3: lifecycle | Expiration works; **transition rules not supported** (s3-testsuite lifecycle 2/8; overall 174/207 = 84%, versioning 14/14, object lock 8/8) | ILM lifecycle + tiering to remote S3 listed available |
| S3: multipart | All 7 operations | Yes |
| S3: Select | **No Object Select** | S3 Select listed available |
| S3: IAM/policies/STS | Present, partial vs MinIO's depth | IAM/policies listed available; its own matrix says "broad compatibility for supported features", explicitly not complete coverage |
| S3: replication | `filer.sync` cross-cluster (active-active or active-passive, resumable); `filer.backup`/`filer.replicate` to external S3/GCS/Azure/B2 | Site replication (RustFS peers only) + bucket replication to generic S3 targets via a per-target version ledger |
| Access beyond S3 | FUSE mount, WebDAV, SFTP, HDFS, Kubernetes CSI + operator, TUS resumable uploads, HTTP; built-in S3 Tables (Iceberg REST) and Lance table buckets in OSS | WebDAV and FTP/FTPS gateways built in (off by default); SFTP and OpenStack Swift as opt-in build features; a separate MCP server; **no native FUSE/POSIX mount** (third-party s3fs-fuse is the documented fallback). (rustfs.com multi-protocol page, checked 2026-11-24.) |
| Tiering | Transparent cloud tiering/offload built in | ILM tiering to remote S3 |
| Encryption | AES256-GCM at rest; TLS/mTLS between components; JWT-signed volume access; SSE-S3/SSE-KMS/SSE-C with KMS providers incl. OpenBao/Vault/AWS/Azure/GCP | TLS; server-side encryption + RustFS KMS listed available |
| Operational caveats | Missing replicas are **not** auto-repaired (run `volume.fix.replication` or the maintenance worker); self-healing, data recovery, point-in-time recovery are Enterprise-only; a file and a folder cannot share a path (`a/b` and `a/b/c` cannot coexist) | Pool topology fixed at initialisation; site replication needs RustFS-compatible peers |

Sources: project wikis/docs and README feature tables checked 2026-11-23/24;
SeaweedFS s3-testsuite scores from the project wiki (checked 2026-11).

## 4. Project health

**SeaweedFS.** Repository created 2014-07-14; 36,219 stars, 3,129 forks,
Apache-2.0, 715 open issues (GitHub API, 2026-11-24). The "one guy" claim is
**substantially true**: chrislusf has 11,159 commits (the top contributor
share is ~68% per Stackscape); the next human is kmlebedev at 393, with
dependabot at 1,687. Chris Lu (San Francisco) is an independent,
sponsor/Patreon-funded maintainer who also sells the Enterprise edition.
Releases land every one to two weeks (4.51 on 2026-10-25 through 4.55 on
2026-11-23, per the GitHub API). Production receipts exist but are thin in
public: Kubeflow 1.11 and Epinio adopted SeaweedFS as their default S3 store
replacing MinIO; Stackscape counts only three public adopters excluding the
creator. One caution signal: security-advisory volume was high in 2026 (25
advisories June–September 2026, nine critical, per stormdevelopments.ca,
checked 2026-10-21).

**RustFS.** Repository created 2023-11-23; 36,435 stars, 1,686 forks,
Apache-2.0, only 73 open issues (young, aggressively triaged; GitHub API,
2026-11-24). The contributor list is a real team (overtrue 3,348, houseme
1,742, weisd 715, cxymds 467, and more; the project claims 7 core engineers
and 160+ contributors). The company claim is **true but opaque**: owner is
RustFS, Inc. (founded 2024, holds the trademark), yet public sources
contradict each other on geography — the about page gives an Atlanta,
Georgia address, the USPTO trademark filing is by a Beijing company (Beijing
The Ganges River Sand Technology Co., Ltd.), a rustfs.com.cn site exists,
and Stackscape lists "RustFS, Inc. — China". No published investors, funding,
or enterprise support terms; no paid tier yet. GA 1.0.0 shipped 2026-09-16;
1.0.6 followed on 2026-11-15. Vendor deployment claims (2.7M at GA, 3.7M+
later) are unverified vendor figures.

Both projects, in other words, carry a stewardship risk — SeaweedFS's is a
bus factor of one with a decade of track record; RustFS's is a young company
whose structure is hard to verify from public sources, with two months of GA
history at the time of writing.

## 5. The data-loss question, investigated

Full case notes in `supplement-data-loss-cases.md`. Summary:

**SeaweedFS — real reports, but the losses concentrate in tooling, topology
choices, and the metadata store, not in acknowledged writes:**

1. **fsck delete vs moved files (issue #12527, opened 2026-11-10; 4.53/4.41,
   etcd-backed filer).** `volume.fsck -reallyDeleteFromVolume` can permanently
   delete live chunks of files moved during its directory-by-directory walk.
   SQL filer stores got a snapshot fix (#11883); etcd and other
   non-`EntryScanner` stores still walk. Reproduction: 50 moved files all
   falsely reported as orphans on 4.53; on 4.41 with delete plus
   `volume.vacuum`, 0 of 50 remained readable. The loss is silent and delayed
   (tombstones mask it until vacuum). Guardrail: use a SQL filer store, and
   never run fsck-delete while clients are moving files.
2. **Filer metadata store corruption (issue #12563, 2026-11-11).** A Windows
   power loss zeroed a block in the filer's LevelDB store; entries in the
   damaged block were lost (their chunks orphaned). goleveldb recovery was
   itself buggy up to 4.54; fix #12780 recovers in place, dropping only
   damaged blocks. The object data was not corrupted — the metadata was.
   Also exposed: master state `-mdir` defaults to the OS temp dir. Guardrail:
   back up filer metadata (`weed filer.meta.backup`).
3. **Disputed scripted purge (issue #5794, 2024-07-17, v3.68).** A reporter
   claimed a Postgres filer connection loss plus a scripted maintenance suite
   (`ec.rebuild`, `volume.deleteEmpty -force`, `volume.balance`,
   `s3.clean.uploads`) purged all volume data within an hour. The maintainer
   disputed the mechanism and suspected `s3.clean.uploads`. Attribution is
   unresolved; the dangerous ingredient was an automated `-force` maintenance
   script running against a single-copy (`defaultReplication=000`) cluster.
4. **Delete-resurrection edge (issue #12786, 2026-11-19, v4.54).** A delete
   whose tombstone reached the `.dat` file before its `.idx` row can, after
   a crash, resurrect as an *empty* object (200, 0 bytes) instead of a 404 —
   documented in the code as an intentional tradeoff (an empty placeholder
   beats a wrong tombstone dropping an acknowledged write).
5. **Iceberg orphan-cleanup bug (PR #11978, 2026-10-23).** Table maintenance
   `remove_orphans` deleted live data files because writers recorded
   percent-escaped paths while the S3 layer stored decoded paths; fixed in
   the same PR.
6. **Anecdote layer.** Hacker News sentiment is split: one unattributed
   comment claims near-certain data loss with no evidence attached; multiple
   production reports describe it as rock solid at real scale (56M images,
   billions of documents; corruption from cheap SSDs and power outages
   recovered). A March 2026 postmortem titled "SeaweedFS CSI: The FUSE Mount
   Massacre" was operator error — restarting CSI mount pods without
   restarting consumers left dead FUSE mounts; no object data was lost.

And the counterweight from section 2.1: in the independent head-to-head, six
`kill -9` cycles per store across 26,111 writes lost **zero** acknowledged
objects. The half-remembered stories are real; their centre of gravity is
maintenance tooling run with delete flags, single-copy replication choices,
and filer-metadata failures — each with a named guardrail.

**RustFS — a shorter record because a younger product, with two serious
marks:**

1. **CVE-2025-68926 (published 2025-12-30, CVSS 9.8).** A hardcoded static
   gRPC auth token (`"rustfs rpc"`) shipped in client and server,
   non-configurable, valid across all deployments, in alpha.13–alpha.77:
   anyone with network access to the gRPC port could run privileged
   operations including data destruction. Public proofs of concept exist;
   fixed in alpha.78. A fundamental security-hygiene failure, albeit pre-GA.
2. **CVE-2026-73288 (published 2026-08-12, CVSS 6.1).** Object Lock
   enforcement treated unreadable or unparseable bucket metadata as "no lock
   config", so COMPLIANCE-retained objects could be deleted or expired;
   fixed in 1.0.0-rc.1. A secondary comparison (stormdevelopments.ca,
   checked 2026-10-21) describes RustFS Object Lock as "bypassed twice in
   2026".
3. **Project postmortem (2026-09-03).** A replication checksum-default flip
   in 1.0.0-rc.5 silently broke every replication PUT to Object Lock targets;
   a customer reported it two days after release. The project's own finding:
   no Object Lock target was exercised between the change and the RC.
4. **No public production data-loss postmortems** comparable to SeaweedFS's
   issue history were found — but RustFS reached GA on 2026-09-16, so the
   absence of reports is at least partly absence of exposure.

## 6. Supplemental: fit for a Cozystack estate

*(This section is about our decisions specifically; everything above stands
generically.)*

Cozystack ships SeaweedFS as its built-in S3 (Cozystack repo and docs,
checked 2026-11): a `Bucket` application via COSI bucket claims,
BucketClasses (including a `-lock` class and a `-readonly`
BucketAccessClass), the system SeaweedFS chart under `packages/system`,
tenant integration (buckets available to everything under a tenant tree once
`seaweedfs: true` is set), a documented multi-DC mode since Cozystack
v0.34.0 (`SeaweedFS` resource, `topology: MultiZone`, per-zone replicas and
sizes), and a remote/Client mode for consuming another cluster's SeaweedFS
with mTLS client certificates. Upgrades, tenancy, and per-user credentials
ride the platform.

Adopting RustFS inside Cozystack would mean running it as a self-managed
add-on (its own upstream Helm chart): no COSI `Bucket` integration, no
tenant plumbing, and a second S3 control surface duplicating the built-in
one. Migration either way is an S3-API copy — objects re-ingested; version
histories, Object Lock configuration, and IAM need deliberate re-creation,
not a lift-and-shift. RustFS's MinIO on-disk compatibility does not help
here: Cozystack holds no MinIO data.

**Recommended posture:**

1. SeaweedFS, as shipped by Cozystack, is the platform S3 layer. Adopt the
   guardrails from section 5 as operating rules: SQL filer store (not
   etcd), scheduled filer-metadata backups, never run
   `volume.fsck -reallyDeleteFromVolume` while clients move files, no
   scripted `-force` maintenance purges, replication never left at 000 for
   data that matters, and pin versions rather than tracking latest.
2. RustFS goes on the watch list, with a pilot shape if a standalone
   large-object workload (build artifacts, model files, media) appears
   outside Cozystack: it is the better engine for that shape on the evidence
   above. Re-evaluate for anything more after a year of GA exposure and a
   cleaner security year than 2025–2026.

## Sources

- Independent head-to-head: iamkunal9, "MinIO alternatives: RustFS vs
  SeaweedFS benchmark", published 2026-10-19 —
  https://iamkunal9.in/blog/minio-alternatives-rustfs-vs-seaweedfs-benchmark
- Community warp comparison: rustfs/rustfs GitHub Discussion #1500 (crawled
  2026).
- Project documentation and wikis: seaweedfs.com / SeaweedFS GitHub wiki;
  docs.rustfs.com; rustfs.com multi-protocol access page (checked
  2026-11-24); RustFS README feature table (re-checked 2026-09-21 and
  2026-11-24).
- GitHub API repository, contributor, and release data for
  seaweedfs/seaweedfs and rustfs/rustfs, read 2026-11-23/24 (raw output in
  `supplement-project-health.md`).
- SeaweedFS issues #12527, #12563, #5794, #12786 and PR #11978; RustFS
  CVE-2025-68926 and CVE-2026-73288 records; RustFS project postmortem of
  2026-09-03 (case notes in `supplement-data-loss-cases.md`).
- Cozystack repository and documentation (SeaweedFS system package, Bucket
  application, multi-DC mode), checked 2026-11.
- Secondary comparisons: stormdevelopments.ca storage comparison (checked
  2026-10-21); Stackscape project profiles; LLMS3 notes on mid-2025
  independent RustFS testing.
