# Object storage: RustFS and SeaweedFS

The cluster needs an S3-compatible object store — for
CloudNativePG backups, for buckets on the HDD pools, for the
cold tier. Two candidates were compared, and **the verdict has
landed: SeaweedFS stays as Cozystack's S3 layer.** Status:
**decided (2026-11-24); not deployed** — there is no cluster.

The verdict rests on the dated comparison record
`doc/2026-11-24T01-33-33Z_research_rustfs-vs-seaweedfs/` in the
code repository (PR
[#13990](https://github.com/developing-today/code/pull/13990),
record dated 2026-11-24), read in full for this update. Drew's
earlier lean to SeaweedFS (2026-11-23, because Cozystack ships
it) is **confirmed by that record's evidence**, and this chapter
now states the decision rather than deferring it.

First, a distinction this book keeps plain: part 1's
[Storage: LINSTOR and DRBD](../cluster/storage-linstor-and-drbd.md)
is **block storage** — replicated volumes for workloads. This
chapter is **object storage** — S3 objects in buckets. Different
layers, different tools, no overlap in the choice.

## The verdict (2026-11-24)

- **SeaweedFS stays as Cozystack's S3 layer.** It is the object
  store Cozystack ships — its buckets are SeaweedFS underneath,
  and its bundled `cozy-backups` uses a SeaweedFS bucket — and
  the comparison record's overall call for a Cozystack-shaped
  estate is to stay on it: it won the record's independent
  head-to-head on small-object write throughput and latency and
  on object-count scaling, and it carries a decade of
  production exposure. Adopting anything else inside Cozystack
  would mean a second, self-managed S3 control surface with no
  COSI `Bucket` or tenant integration.
- **RustFS is a credible pilot candidate only for standalone
  large-object workloads outside Cozystack** — the shape where
  the record's evidence favours it (large-object throughput,
  small footprint, one binary, erasure coding at write time) —
  **to be re-evaluated after a year of GA exposure.** It is not
  the platform S3 layer, and per the record it is not yet to be
  trusted with the platform's only copy of anything: its
  2025–2026 security record (a hardcoded gRPC token CVE in
  pre-GA builds; an Object Lock bypass CVE) argues for the
  waiting period.
- Programme home: epic
  [#1088](https://github.com/developing-today/code/issues/1088).

## SeaweedFS operating rules (guardrails of record)

The record's data-loss investigation is part of the verdict:
SeaweedFS's real losses concentrate in maintenance tooling,
single-copy configurations and metadata-store failures — not in
acknowledged writes on replicated volumes — and each carries a
guardrail. Carried here in substance, as operating rules for
when the cluster exists:

- **Use a SQL filer store — not LevelDB, not etcd.** The
  record's fsck case ran on an etcd-backed filer whose store
  still walks non-snapshot paths, and its metadata-corruption
  case was a LevelDB filer store zeroed by a power loss; the
  SQL filer stores carry the snapshot fix.
- **Back up the filer metadata** — scheduled filer-metadata
  backups (`weed filer.meta.backup` in the record's wording).
  The object data surviving while its metadata is lost is the
  record's documented failure shape; the metadata is therefore
  a backup item in its own right.
- **Never run fsck-delete during file moves** — never
  `volume.fsck -reallyDeleteFromVolume` while clients are
  moving files: the record's reproduction lost moved files
  silently, with the loss masked until vacuum.
- **No scripted `-force` maintenance purges** — no automated
  maintenance suite running `-force` delete/balance/rebuild
  commands unattended; the record's disputed total-loss report
  had exactly that ingredient.
- **Replication is never left at 000** for data that matters —
  a single-copy (`defaultReplication=000`) configuration is
  the other ingredient in that same report.
- **Pin versions rather than tracking latest** (the record's
  recommended posture, carried with the guardrails above).

## The two candidates, as compared

- **RustFS** — an S3-compatible object store written in Rust.
  The research brief (2026-10-03) records RustFS 1.0 GA on
  2026-09-16, and the cluster programme's brief designated
  RustFS as its S3 target for CNPG backups and buckets on the
  HDD pools. That designation is **superseded by the
  2026-11-24 verdict** above for everything inside Cozystack;
  it survives only as the pilot candidacy outside it.
- **SeaweedFS** — a distributed object and file store with an
  S3 interface, and the object store **Cozystack ships**.
  Cozystack is the cluster's chosen platform (settled by Drew,
  2026-11-23 — see
  [Talos and Cozystack](../cluster/talos-and-cozystack.md)), so
  SeaweedFS arrives with the platform — and, per the verdict,
  it is also the chosen general store, not merely the bundled
  one.

## Gaps

- No deployment exists: the verdict decides the layer, it does
  not build it. Benchmarks and data-loss records are the
  comparison record's, cited there; this chapter asserts none
  beyond its summary.
- The RustFS re-evaluation has a condition (a year of GA
  exposure) but no calendar owner yet; when it comes due, this
  chapter's sync rule (see
  [Keeping this book current](../development/keeping-this-book-current.md))
  is what folds the outcome back in.

## Sources

The comparison record
`doc/2026-11-24T01-33-33Z_research_rustfs-vs-seaweedfs/`
(record dated 2026-11-24, PR #13990) — verdict, benchmark,
feature, project-health and data-loss sections and its
Cozystack-fit supplement; the cluster research brief and status
records behind part 1 (2026-10-03 / 2026-10-27); the platform
census of 2026-11-23 (PR
[#13959](https://github.com/developing-today/code/pull/13959));
and Drew's recorded lean of 2026-11-23, confirmed 2026-11-24.
Updated 2026-11-24.
