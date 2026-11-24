# Supplement: feature and architecture notes

Gathered 2026-11-23/24 from project documentation, wikis, README feature
tables, and the Cozystack repository/docs.

## SeaweedFS

- Design: Haystack — small files packed as needles into append-only volume
  files; ~40 B disk metadata overhead per file; 16 B in-memory index entry
  per blob; O(1) one-seek reads including from erasure-coded data.
- Large files: split into chunks of a few MB, each a blob; a meta chunk
  lists the chunk file ids.
- Durability: hot data replicated (rack/DC-aware 3-digit replication, e.g.
  001/010); warm data converted in the background to RS(10,4) erasure
  coding (1.4× overhead, tolerates 4 shard losses). OSS fixes 10+4; custom
  ratios are Enterprise.
- Metadata: volume layer in master (Raft); filer DB is the scaling ceiling
  — pluggable (LevelDB `leveldb2` default, PostgreSQL, MySQL, Redis,
  Cassandra, etcd, TiKV, FoundationDB, and more); one row per object on the
  S3 path.
- S3 surface (project wiki, checked 2026-11): versioning yes; Object Lock
  yes (governance/compliance/legal hold); s3-testsuite score 174/207 (84%),
  P0 75/79, versioning 14/14, object lock 8/8, lifecycle 2/8 — expiration
  works, transition rules NOT supported; multipart all 7 operations yes;
  bucket policies/IAM/STS partial vs MinIO; no Object Select.
- Independent test note: a file and a folder cannot share a path (`a/b` and
  `a/b/c` cannot coexist).
- Access modes beyond S3: FUSE mount, WebDAV, SFTP, HDFS, Kubernetes CSI +
  operator, TUS resumable uploads, HTTP.
- Tiering: transparent cloud tiering/offload. Cross-cluster `filer.sync`
  active-active or active-passive, resumable with checkpoints.
  `filer.backup`/`filer.replicate` to external S3/GCS/Azure/B2 (decrypts
  SSE first).
- Encryption: AES256-GCM at rest; TLS/mTLS between components; JWT-signed
  volume access; SSE-S3/SSE-KMS/SSE-C with KMS providers including
  OpenBao/Vault/AWS/Azure/GCP.
- Extras in OSS: built-in S3 Tables (Iceberg REST catalog) and Lance table
  buckets.
- Operational caveats: missing replicas are NOT auto-repaired (run
  `volume.fix.replication` or the maintenance worker); self-healing, data
  recovery, and point-in-time recovery are Enterprise-only.
- Security-advisory volume in 2026: 25 advisories June–September 2026, 9
  critical (stormdevelopments.ca, checked 2026-10-21).

## RustFS

- Design: MinIO-design reimplementation — per-object Reed-Solomon EC with
  inline `xl.meta` metadata (one per object per shard, no metadata DB);
  objects < 128 KiB inline. Huge object counts become billions of tiny
  files: inode pressure and ListObjects scan cost are the ceiling. A
  May-2026 small-object scalability analysis rated SeaweedFS the default
  OSS answer at 30B objects and RustFS (then pre-GA) as not yet that
  answer.
- EC geometry (project docs, October 2026): erasure sets of 2–16 drives;
  automatic STANDARD parity EC:0 (1 drive) / EC:1 (2–3) / EC:2 (4–5) /
  EC:3 (6–7) / EC:4 (8–16); 16-drive set = 12 data + 4 parity ≈ 75%
  efficiency; parity ≤ N/2; Reed-Solomon over GF(2^8) Vandermonde; 1 MiB
  erasure blocks; HighwayHash-256 bitrot checksums (same family/defaults as
  MinIO, so MinIO-migrated objects stay readable).
- Pool topology is fixed at initialisation; mismatch = permanent
  `PoolTopologyMismatch` startup failure. MNMD (≥ 4 nodes) is the
  production layout; SNSD/SNMD single-node options exist.
- README feature table (re-checked 2026-09-21 and 2026-11-24): S3 Core,
  Versioning, Object Lock (WORM), Lifecycle (ILM) + ILM tiering to remote
  S3, Bucket Replication, Site Replication, IAM/Policies, Server-Side
  Encryption + RustFS KMS, Distributed Mode, Bitrot Protection, Healing &
  Scanner, Pool Expansion/Decommission, Bucket Quota, S3 Select, Event
  Notifications — all "Available". S3 Tables (Iceberg REST) and MinIO
  On-Disk Compatibility = Preview. Its own compatibility matrix is more
  careful: "broad S3 API compatibility for supported features", explicitly
  NOT complete coverage. Site replication requires RustFS-compatible peers;
  bucket replication to a generic S3 target is supported via a per-target
  version ledger.
- Protocols beyond S3 (rustfs.com multi-protocol page, checked
  2026-11-24): WebDAV and FTP/FTPS are built-in gateways, disabled until
  enabled; Swift requires an optional build (SFTP likewise an opt-in cargo
  feature per the README); MCP is a separate server pointed at the S3
  endpoint. No native FUSE/POSIX mount; third-party s3fs-fuse is the
  documented fallback.
- Dedup: neither system offers content deduplication as a core storage
  feature (checked 2026-11-24). SeaweedFS "dedup" references are read-path
  request coalescing (singleflight chunk downloads), not content
  addressing.

## Cozystack fit

- SeaweedFS is Cozystack's built-in S3: `Bucket` app via COSI bucket
  claims; BucketClasses (including a `-lock` class and a `-readonly`
  BucketAccessClass in a recent commit); the system SeaweedFS chart under
  `packages/system`; tenant integration (buckets available to everything
  under a tenant tree once `seaweedfs: true`); documented multi-DC mode
  since Cozystack v0.34.0 (`SeaweedFS` resource, `topology: MultiZone`,
  per-zone replicas/size); remote/Client mode for consuming another
  cluster's SeaweedFS with mTLS client certs. Upgrades, tenancy, and
  credentials (per-user admin/readonly secrets) ride the platform.
- RustFS inside Cozystack would be a self-run add-on (its own upstream Helm
  chart exists): no COSI `Bucket` integration, no tenant plumbing, a second
  S3 control surface duplicating the built-in one.
- Migration either way is an S3-API copy: objects re-ingested; version
  histories, Object Lock configuration, and IAM need deliberate
  re-creation, not a lift-and-shift. RustFS's MinIO on-disk compatibility
  does not help here (Cozystack holds no MinIO data).
