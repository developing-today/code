# Object storage: RustFS and SeaweedFS

The cluster needs an S3-compatible object store — for
CloudNativePG backups, for buckets on the HDD pools, for the
cold tier. Two candidates are on the table, a comparison is
running, and **this chapter records the question and the state,
not a verdict**. Status: **researched; decision deferred** to
the comparison lane's dated record.

First, a distinction this book keeps plain: part 1's
[Storage: LINSTOR and DRBD](../cluster/storage-linstor-and-drbd.md)
is **block storage** — replicated volumes for workloads. This
chapter is **object storage** — S3 objects in buckets. Different
layers, different tools, no overlap in the choice.

## The two candidates

- **RustFS** — an S3-compatible object store written in Rust.
  The research brief (2026-10-03) records RustFS 1.0 GA on
  2026-09-16, and the cluster programme's brief designated
  RustFS as its S3 target for CNPG backups and buckets on the
  HDD pools (part 1's
  [The cluster programme](../cluster/the-cluster-programme.md)).
- **SeaweedFS** — a distributed object and file store with an
  S3 interface, and the object store **Cozystack ships**: its
  buckets are SeaweedFS underneath, and its bundled
  `cozy-backups` uses a SeaweedFS bucket. Cozystack is the
  cluster's chosen platform (settled by Drew, 2026-11-23 — see
  [Talos and Cozystack](../cluster/talos-and-cozystack.md)), so
  SeaweedFS arrives with the platform whether or not it is also
  chosen as the general store.

## The state of the decision

- **Drew's current lean, recorded as a lean** (2026-11-23): a
  slight lean to **SeaweedFS, because Cozystack ships it**. A
  lean is not a verdict, and this book does not promote it to
  one.
- **A comparison lane is running now** under the lab programme —
  a separate lane weighing benchmarks, features, data-loss
  stories, and Cozystack fit. Its dated record is forthcoming
  and will land in `doc/` in the code repository.
- **The verdict is explicitly deferred to that record.** When it
  lands, this book's sync rule (see
  [Keeping this book current](../development/keeping-this-book-current.md))
  folds the verdict into this chapter and into the part 1
  chapters that name RustFS as the designated target.
- Programme home: epic
  [#1088](https://github.com/developing-today/code/issues/1088).

## Gaps

- No benchmarks, feature comparisons, or data-loss records for
  either candidate are in the record this book was written from;
  producing them is the comparison lane's job, and this chapter
  asserts none of them.
- Until the verdict lands, part 1's "designated S3 target"
  wording for RustFS and this chapter's lean coexist as what
  they are: a brief's designation and a recorded lean, with the
  decision still open.

## Sources

The cluster research brief and status records behind part 1
(2026-10-03 / 2026-10-27), the platform census of 2026-11-23
(PR [#13959](https://github.com/developing-today/code/pull/13959)),
and Drew's recorded lean of 2026-11-23.
