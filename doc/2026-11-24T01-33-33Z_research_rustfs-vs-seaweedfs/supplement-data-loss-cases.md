# Supplement: data-loss case notes

Gathered 2026-11-23 from project issue trackers, CVE records, postmortems,
and forum threads. Each case: what happened, version, classification.

## SeaweedFS

### 1. fsck delete vs moved files — issue #12527 (opened 2026-11-10)

Versions 4.53 / 4.41, etcd-backed filer. `volume.fsck
-reallyDeleteFromVolume` can permanently delete live chunks of files MOVED
during its filer walk; the walk is directory-by-directory. SQL stores got a
snapshot fix in #11883; etcd and other non-`EntryScanner` stores still walk.
Reproduction: 50 moved files all falsely reported orphans on 4.53
(report-only mode); on 4.41 with delete + `volume.vacuum`, all 50 still
listed but 0/50 readable (chunk GETs 404). Loss is silent and delayed —
tombstones plus `readDeleted=true` mask it until vacuum.
Classification: real product bug in maintenance tooling, triggered by a
specific filer backend + concurrent moves. Mitigation: SQL filer store;
never run fsck-delete while clients move files.

### 2. Filer LevelDB corruption after power loss — issue #12563 (2026-11-11)

Windows power loss left a zeroed block in the filer's LevelDB store; entries
in the damaged block are lost (their chunks orphaned). goleveldb recovery
itself was buggy up to 4.54; fix #12780 recovers in place, dropping only
damaged blocks. The case also exposed that master state `-mdir` defaults to
the OS temp dir. Object data itself was not corrupted.
Classification: metadata-store failure, not a volume/write-path failure.
Mitigation: filer metadata backups (`weed filer.meta.backup`).

### 3. Claimed scripted purge — issue #5794 (2024-07-17, v3.68) — DISPUTED

Reporter claimed a Postgres filer connection loss (12 h) on a third filer
plus a scripted maintenance suite (`ec.rebuild`, `volume.deleteEmpty
-force`, `volume.balance`, `s3.clean.uploads`) purged all volume data
within an hour. The maintainer disputed the filer-connectivity mechanism
and suspected `s3.clean.uploads`. Attribution unresolved in the thread.
Classification: disputed; the dangerous ingredient was an automated `-force`
maintenance script, on a `defaultReplication=000` (single-copy) cluster.

### 4. Delete resurrection as empty object — issue #12786 (2026-11-19, v4.54)

A delete whose tombstone reached the `.dat` file before its `.idx` row can,
after a crash, resurrect as an EMPTY object (HTTP 200, 0 bytes) instead of 404. Documented in the code as an intentional tradeoff: an empty placeholder
beats a wrong tombstone dropping an acknowledged write.
Classification: known, documented edge behaviour; data-integrity smell, not
silent loss of acknowledged content.

### 5. Iceberg orphan cleanup deleted live files — PR #11978 (2026-10-23)

Iceberg table maintenance `remove_orphans` deleted LIVE data files because
writers recorded percent-escaped paths (`%3D`) while the S3 layer stored
decoded paths. Fixed by dual-spelling coverage in the same PR.
Classification: real product bug in a young feature (S3 Tables), fixed on
discovery.

### 6. Anecdote layer

- Hacker News: one unattributed comment ("should absolutely not be used for
  anything beyond testing — otherwise you're almost certain to lose data")
  with no evidence attached; against it, multiple production reports
  ("rock solid", "serve 1M users daily with 56 million images", billions of
  XML/docs; corruption from cheap SSDs and power outages recovered).
- Goldentooth postmortem, March 2026, "SeaweedFS CSI: The FUSE Mount
  Massacre": operator error — restarting CSI mount pods without restarting
  consumers left dead FUSE mounts (ENOTCONN). No object data lost.

### Counterweight

Independent head-to-head (see supplement-benchmarks.md §A): zero
acknowledged objects lost or corrupted across 6 `kill -9` cycles per store,
26,111 writes.

## RustFS

### 1. CVE-2025-68926 (published 2025-12-30, CVSS 9.8)

Hardcoded static gRPC auth token `"rustfs rpc"` in client and server,
non-configurable, valid across all deployments, alpha.13–alpha.77. Anyone
with network access to the gRPC port could run privileged operations
including data destruction. Public proofs of concept exist. Fixed in
alpha.78.
Classification: fundamental security-hygiene failure, pre-GA. Not a
data-loss bug in the write path, but a standing invitation to destruction
for anyone who deployed those alphas with the port reachable.

### 2. CVE-2026-73288 (published 2026-08-12, CVSS 6.1)

Object Lock enforcement treated unreadable/unparseable bucket metadata as
"no lock config", so COMPLIANCE-retained objects could be deleted/expired.
Fixed in 1.0.0-rc.1. A secondary comparison (stormdevelopments.ca, checked
2026-10-21) describes RustFS Object Lock as "bypassed twice in 2026".
Classification: durability-semantics bug in a compliance feature — the class
of bug that matters most for WORM workloads.

### 3. Replication checksum-default flip — project postmortem, 2026-09-03

A replication checksum-default flip in 1.0.0-rc.5 silently broke every
replication PUT to Object Lock targets; reported by a customer two days
after release. Process finding recorded by the project: no Object Lock
target was exercised between the change and the RC.
Classification: release-process gap around exactly the features (replication

- Object Lock) a durability story rests on.

### 4. Absence of production data-loss postmortems

None found comparable to SeaweedFS's issue history. RustFS reached GA on
2026-09-16, so absence of reports is at least partly absence of exposure.
