# Storage: LINSTOR and DRBD

Cluster storage is designed, not built. This chapter records the
design exactly as the record carries it — and then states plainly
what exists today: no cluster storage pools, and a local↔cloud
sync story that today runs entirely outside the cluster.

Tiered storage is the design centre of this chapter, per Drew's
direction of 2026-11-23: LINSTOR device tiers built from the
lab's actual hardware, with a defined fallback where LINSTOR
cannot serve. Everything tier-shaped below is **planned — design
only**, written against a planned system that does not exist yet.

## The settled storage preference: LINSTOR first, Ceph avoided

**Ceph is avoided where possible (Drew, settled 2026-11-23).**
Where the wider research weighed distributed storage options —
the CNCF landscape census carried the LINSTOR-over-Rook/Ceph
line for the Cozystack stack — this book now states the settled
preference plainly: **LINSTOR-first is the direction; Ceph is
not the fallback plan and is not carried as a co-equal
candidate.** Factual note, kept brief: Ceph is a widely deployed
distributed storage system and Rook is its Kubernetes operator;
neither is in this estate's design, and this book gives Ceph no
further treatment. Status: **settled direction (2026-11-23);
nothing built.**

## The replication design (planned)

Cozystack's storage layer is LINSTOR on ZFS/DRBD, per the research
brief. Drew's sketch adds the shape: three machines replicating
via DRBD/LINSTOR, so data loss needs the same failure on two
machines at once. The storage work sits in
[#1813](https://github.com/developing-today/code/issues/1813)
(open) under epic
[#1085](https://github.com/developing-today/code/issues/1085).

Status: **planned — design only.** The only DRBD artefact that
ever ran is the torn-down v1.9 test cluster's `siderolabs/drbd`
Talos extension — historical, see
[Talos and Cozystack](talos-and-cozystack.md). The NIC and
switch-gear sanity check that LINSTOR/DRBD replication traffic
depends on is still open
([#1097](https://github.com/developing-today/code/issues/1097));
with the dictated "at least 200 Gb ports" figure unverified
against a 100GbE-max fabric of record, the design cannot be
sanity-checked until that lands (gap 2).

## Tiered storage: the design centre (planned)

The tiers are LINSTOR storage pools over device classes taken
from the [Hardware register](../site/hardware-register.md).
Anything about the tiers that the register does not support is
marked **design intent**, not inventory.

### Tier 1 — fast: Samsung 990 SSDs (PCIe 4)

- **Hardware of record:** Samsung 990 Pro NVMe — the register's
  best tier ("prosumer Samsung 990 Pro"), present as boot drives
  across the core fleet and as the laptop `amd`'s collected
  Samsung SSD 990 PRO 4TB. The PCIe 4 designation is Drew's
  2026-11-23 tier naming (**design intent** as a tier label; the
  register records the drives, not a per-drive bus audit).
- **What lands here:** the data that pays for latency —
  Postgres primary and streaming-replica data and WAL (see
  "Postgres: CloudNativePG and streaming replicas" below),
  etcd-class and control-plane volumes, hot workload volumes,
  and the read cache / landing zone for the join pattern below.
- **Shape:** LINSTOR pool over the 990-class devices, DRBD
  replication across three machines per the replication design
  above; the register's earlier named pools — (1) 4 drives
  RAID 10 and (2) 6 drives RAID 100 — are carried as the pool
  layouts of record, to be validated.

### Tier 2 — mid: mixed NVMe and consumer SSDs

- **Hardware of record:** 980s, smaller 990s and good consumer
  SSDs — the register's mid tier, and the one specification that
  differs between the otherwise identical core boxes.
- **What lands here:** general workload volumes, build and CI
  scratch that must survive a node, and the second copy class
  for data whose hot copy lives on Tier 1.
- **Shape:** stripe 3 (about 990-class performance) × mirror 2,
  "RAID 100" (3 stripes, 2 mirrors, 6 drives), so two drives
  must die to down a machine (idea register, 2026-10-20).

### Tier 3 — bulk: helium HDDs

- **Hardware of record:** 15–16 × 16 TB helium **Western
  Digital** server pulls (Drew also said "at least 12" once;
  the register records both figures); eight are still in the
  box, uninstalled, with no SMART or baseline data captured.
  **Flagged, not reconciled:** Drew's 2026-11-23 direction for
  this pass named a *Seagate* helium HDD bulk tier. The register
  — this book's hardware source — records Western Digital.
  This chapter follows the register and flags the difference;
  the per-drive inventory
  ([#1096](https://github.com/developing-today/code/issues/1096),
  open) is what settles it, not an editorial choice.
- **What lands here:** capacity data — S3 buckets on the HDD
  pools (the SeaweedFS layer of
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md)),
  cold-ish buckets, backups, media and sealed archive bundles
  awaiting the cold store.
- **Shape:** 3 hosts × 4 × 16 TB in RAID 10 (idea register,
  carried into epic #1085 / #1813), feeding the object layer.

### Goal and open inventory work

- Goal: 3–5 devices with near parity in disk size and
  capability.
- Still open: install the 8 still-boxed HDDs and capture their
  baseline SMART/firmware data; per-drive records/passports so
  TALM and LINSTOR decisions have real data
  ([#1096](https://github.com/developing-today/code/issues/1096),
  open). The NVMe baseline already exists on a backup drive —
  see the [Hardware register](../site/hardware-register.md).

## Where LINSTOR cannot serve: the fallback design (planned)

LINSTOR/DRBD is block storage for cluster workloads. Two classes
of need sit outside it, and the design names a fallback for
each. Status: **design intent — nothing below is built.**

- **Object-shaped and off-cluster data → S3.** Anything that is
  an object rather than a volume — backups, sealed bundles,
  media, data that must be reachable from outside the cluster
  or survive it — goes to the S3 layer: SeaweedFS as shipped by
  Cozystack inside the cluster (the verdict of
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md),
  record dated 2026-11-24), and the existing R2/Drive cold
  store outside it (see the sync sections below). LINSTOR
  volumes are never the only copy of anything in this class.
- **Data spread across locations → pull-from-multiple-locations-
  and-join.** Where a dataset legitimately lives in more than
  one place (for example a sealed bundle in the cold store, a
  working copy on the laptop, a bucket copy in-cluster), the
  reader does not pick one home and copy the whole dataset
  first. The design shape:
  1. A manifest/index — held outside the bulk data, as the
     cold-store design already specifies for Drive bundles —
     names every location holding each part, with checksums.
  2. The reader pulls each part from the nearest or cheapest
     available location, in parallel where the parts are
     independent.
  3. The join verifies each part against the manifest checksum
     before assembly; a part that fails verification is pulled
     from another location, never trusted.
  4. The assembled result lands on Tier 1 or Tier 2 as a cache
     or working copy; the locations of record are unchanged —
     the join is a read pattern, not a migration.

### Open questions (explicit, by design)

This tier/fallback/join design is a design centre, not a solved
system. The open questions are part of the design record:

- **Placement policy.** What decides which tier a new volume or
  object lands on, and who moves data between tiers as it cools
  — LINSTOR pool rules, application choice, or a mover job? No
  policy is written yet.
- **Join consistency.** The manifest is the join's only truth:
  what updates it, what happens when two locations disagree
  about a part, and what a reader sees mid-update are all
  undecided. The design assumes checksums catch corruption; it
  does not yet define conflict resolution.
- **Rebuild behaviour.** When a Tier 1 or Tier 3 device dies,
  what rebuilds first, at what priority, and what does DRBD
  resync traffic do to the unverified fabric (gap 2) while it
  runs? RAID 100's "two drives must die" property is a sketch
  figure, not a measured rebuild story.
- **Quorum.** Split-brain decisions in two- or four-storage-node
  layouts still want a witness (see
  [The cluster programme](the-cluster-programme.md)); the
  witness's own storage class is unnamed.

## Postgres: CloudNativePG and streaming replicas (planned)

**The basis is settled: Postgres in the cluster is
CloudNativePG (CNPG), as shipped by Cozystack — Drew's lean and
Cozystack's shipped operator agree, and the research brief's
decision stands: use Cozystack's CNPG-based managed Postgres;
do not deploy a second operator.** Status: **planned — no
Postgres runs in any cluster, because there is no cluster.**

The shape, against the tiered design above:

- **What runs where.** Each CNPG cluster is a primary plus
  streaming replicas, spread across machines so a machine loss
  loses at most one member. Data and WAL sit on Tier 1
  (Samsung 990) LINSTOR volumes — Postgres is the workload the
  fast tier exists for. Nothing Postgres-shaped lands on the
  HDD bulk tier except its backups.
- **Streaming-replica shape.** The primary streams WAL to its
  replicas continuously (Postgres streaming replication, as
  CNPG manages it); replicas serve failover first and read
  traffic second. Replica count, synchronous-versus-asynchronous
  commit, and read scaling are **open tuning questions**, not
  settled figures — this book records the shape, not numbers
  the record does not carry.
- **Backups.** CNPG backups and WAL archiving go to an S3
  endpoint — under the 2026-11-24 object-storage verdict that
  is the in-cluster SeaweedFS layer (Cozystack's bundled
  `cozy-backups` SeaweedFS bucket) first, with the external
  cold store as the off-cluster copy. See
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md).
- **Relation to the estate's existing Postgres uses.** The
  estate's Postgres today lives outside any cluster: the
  `supabase-backups` R2 bucket in the backup topology below is
  the record's named trace of a Supabase Postgres in the
  backup set. CNPG in the cluster does not replace that estate
  by declaration; whether any existing Postgres workload
  migrates onto CNPG is a per-workload decision with no record
  yet, and this book claims none.

## Local↔cloud sync: what exists today (proven, outside the cluster)

- The general backup topology of record (2026-10-27): sandbox →
  laptop + R2 + Drive; laptop → Drive + R2. "Sandbox is not
  storage" is a standing rule.
- Five R2 buckets are named in the cold-store design's source
  inventory: `agent-inbox-mail`, `labops-files`,
  `nix-cache-nars`, `supabase-backups`,
  `webhook-gateway-payloads`. No source read by the census
  established byte totals for them, and this book invents none.
- The lab's first Cloudflare Tunnel is live from the laptop — the
  first working local→cloud path carrying the lab name. See
  [The Osseo site](../site/the-osseo-site.md).

## Local↔cloud sync: what is designed (planned)

- **Cold-store layout design (2026-10-25)** — explicitly
  "design only; this pass moved nothing, created no bucket or
  folder, changed no lifecycle rule". Google Drive is the cold,
  immutable archive vault: data packed into sealed 4–16 GB
  bundles with the searchable index/manifest held outside Drive.
  Cloudflare R2 is the hot/operational tier: current backups,
  served media, build/cache objects, with free egress. Existing
  tenant/bucket boundaries are kept; nothing is consolidated for
  neatness.
- **Cluster tiering direction:** object tiering to R2 (epic
  [#1091](https://github.com/developing-today/code/issues/1091));
  cluster backups into the cold store like every other store,
  proven by a recorded restore (P9 draft phase 3 — draft,
  unratified); an etcd + TALM config recovery vault as encrypted
  snapshots offsite, for example R2 (epic
  [#1090](https://github.com/developing-today/code/issues/1090));
  CNPG Postgres backups to the in-cluster SeaweedFS S3 endpoint
  (verdict of 2026-11-24, see the Postgres section above). The
  wider object-storage verdict —
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md) —
  is object storage, not this chapter's block storage.
- **Atlas sync shape (cross-link):** the Framework laptop stays
  the canonical Atlas home and ingest factory; a cluster Atlas
  serve mode would read the Turso hot copy (P9 draft phase 2 —
  draft, unratified).
- **DNS/domain side:** a standing domain register with a regular
  full DNS-config backup exists as a completed first pass
  (2026-10-25, 40 domains), with recurring machinery on
  [#902](https://github.com/developing-today/code/issues/902) —
  in progress, recorded here because the register's own text ties
  it to the cluster programme's files; it is not lab-cluster
  storage proper.

## The honest summary

There is **no cluster sync running** — there is no cluster.
Local↔cloud sync today is the laptop/sandbox backup topology plus
one tunnel. Everything cluster-shaped in this chapter is design,
and the chapter's status words keep it that way.
