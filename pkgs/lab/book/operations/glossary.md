# Glossary

**AppFS** — The badge.team application filesystem adopted from
the Tanmatsu: an app is a standard, unmodified ESP-IDF binary
with slug/title/version metadata in a dedicated partition. See
[ESP32](../microcontrollers/esp32.md).

**BMC / IPMI** — Baseboard management controller / Intelligent
Platform Management Interface: the enterprise remote management
(KVM, power, sensors) built into the core machines. Versions and
settings are still to be audited (epic #1081).

**Ceph** — A distributed storage system avoided where possible
in this estate (Drew, settled 2026-11-23); LINSTOR-first is the
direction.

**Cozystack** — The Kubernetes platform planned for the cluster:
it decides storage, tenancy and GitOps, and provides LINSTOR
storage, CNPG Postgres and SeaweedFS buckets. Planned, not
installed.

**CNPG** — CloudNativePG, the managed Postgres operator the
programme uses via Cozystack. The research brief's decision: use
Cozystack's CNPG-based Postgres; do not deploy a second operator.
Runs as a primary with streaming replicas; see
[Storage: LINSTOR and DRBD](../cluster/storage-linstor-and-drbd.md).

**DRBD** — Distributed Replicated Block Device: block-level
replication between machines, managed by LINSTOR in the cluster
design. Drew's sketch replicates across three machines so data
loss needs the same failure on two at once.

**DX010** — Celestica DX010: the 100GbE fabric switch of the
research inventory (32× QSFP28, SONiC). The dictated "DXO10" and
the issue-title spelling "Celestia DX010" are the same device.

**Falco** — A runtime-detection tool (eBPF/syscall behaviour
watching); the design's tripwire layer — detection, not
prevention. Not deployed. See
[Security: Falco runtime detection](../platform/security-and-falco.md).

**LINSTOR** — The storage management layer in Cozystack that
provisions DRBD-replicated volumes on ZFS. Design only in this
programme.

**P9** — The fleet-drafted lab-integration design
(`doc/living/lab-integration.md`). A draft awaiting Drew's
ratification (#8873); nothing in it is decided.

**RAID 100** — The register's name for the mid NVMe tier's layout:
stripe 3 × mirror 2 (6 drives), so two drives must die to down a
machine.

**RustFS** — An S3-compatible object store, one of the two
candidates for CNPG backups and buckets on the HDD pools. The
RustFS-versus-SeaweedFS verdict landed 2026-11-24: SeaweedFS
stays as Cozystack's S3 layer, and RustFS is pilot-only for
standalone large-object workloads outside Cozystack,
re-evaluated after a year of GA exposure. See
[Object storage: RustFS and SeaweedFS](../platform/object-storage.md).

**TALM** — `cozystack/talm`: the tool that templates and applies
Talos machine configuration per node from files in git. Its
per-node disk flow (#1812) is the book's one proven cluster
piece.

**Talos** — Talos Linux, the cluster's operating system:
installed declaratively via TALM, never hand-built. No node is
installed yet.

**Tanmatsu** — The handheld terminal (Nicolai Electronics /
badge.team lineage) built on an ESP32-P4 plus ESP32-C6, named as
the reference design for worlds-as-apps on the embedded tier.
The dictated "tenmazu" is this device.
