# Storage: LINSTOR and DRBD

Cluster storage is designed, not built. This chapter records the
design exactly as the record carries it — Drew's sketch, to be
validated — and then states plainly what exists today: no cluster
storage pools, and a local↔cloud sync story that today runs
entirely outside the cluster.

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

## Storage tiers (planned)

From the idea register (2026-10-20), carried into epic #1085 /
#1813. Drew's sketch, to be validated:

- **HDD tier:** 3 hosts × 4 × 16 TB in RAID 10, feeding RustFS
  cold-ish buckets. The drives are the 15–16 × 16 TB helium
  Western Digital server pulls of the
  [Hardware register](../site/hardware-register.md) — eight of
  them still boxed.
- **Fast NVMe tier:** all Samsung 990 Pros.
- **Mid NVMe tier:** 980s, smaller 990s and good consumer SSDs —
  stripe 3 (about 990-class performance) × mirror 2, "RAID 100"
  (3 stripes, 2 mirrors, 6 drives), so two drives must die to
  down a machine.
- Two named pools from earlier work: (1) 4 drives RAID 10;
  (2) 6 drives RAID 100.
- Goal: 3–5 devices with near parity in disk size and capability.
- Still open: install the 8 still-boxed HDDs and capture their
  baseline SMART/firmware data; per-drive records/passports so
  TALM and LINSTOR decisions have real data
  ([#1096](https://github.com/developing-today/code/issues/1096),
  open). The NVMe baseline already exists on a backup drive — see
  the [Hardware register](../site/hardware-register.md).

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
- **Cluster tiering direction:** RustFS cold tiering to R2 (epic
  [#1091](https://github.com/developing-today/code/issues/1091));
  cluster backups into the cold store like every other store,
  proven by a recorded restore (P9 draft phase 3 — draft,
  unratified); an etcd + TALM config recovery vault as encrypted
  snapshots offsite, for example R2 (epic
  [#1090](https://github.com/developing-today/code/issues/1090));
  CNPG Postgres backups to an S3 endpoint — in-cluster RustFS,
  Cozystack's bundled `cozy-backups` SeaweedFS bucket, or external
  (decision open, see [Talos and Cozystack](talos-and-cozystack.md)).
  The wider RustFS-versus-SeaweedFS object-storage question —
  object storage, not this chapter's block storage — is
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md).
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
