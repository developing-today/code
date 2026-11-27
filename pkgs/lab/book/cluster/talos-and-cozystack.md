# Talos and Cozystack

The cluster's operating system is Talos Linux, configured
declaratively with TALM; the platform on top is Cozystack. This
chapter states what is planned, what is history, and the one piece
that is proven.

## The planned state

- **Talos Linux** on the first 3–5 of the core machines, installed
  only through the TALM bootstrap flow: "no node is installed
  until the TALM bootstrap flow is proven" (status record
  2026-10-27). Bootstrap epic:
  [#1086](https://github.com/developing-today/code/issues/1086).
- **Cozystack** on top of Talos: **the chosen platform — settled
  by Drew on 2026-11-23 ("we will definitely use the Cozystack"),
  no longer an open evaluation.** It was researched in depth
  before the decision (CNCF Sandbox; LINSTOR on ZFS/DRBD for
  storage; Kube-OVN, Cilium and MetalLB for networking; SeaweedFS
  buckets; KubeVirt; CNPG Postgres). Storage preference, settled
  by Drew 2026-11-23: LINSTOR-first, and Ceph avoided where
  possible — see
  [Storage: LINSTOR and DRBD](storage-linstor-and-drbd.md).
  Install epic:
  [#1087](https://github.com/developing-today/code/issues/1087),
  "proposed/Later" in the roadmap. The CNCF census adds the caveat
  that Cozystack "decides storage/tenancy/GitOps for you"
  (`doc/2026-11-23T22-33-41Z_research_cncf-landscape/`) — a
  property of the chosen platform, recorded with the decision.
  Chosen is not built: the status below stands unchanged.
- **Data services** as decided in the research brief:
  CloudNativePG managed Postgres (do not deploy a second
  operator), and S3 object storage — the RustFS-versus-SeaweedFS
  verdict landed 2026-11-24 (record
  `doc/2026-11-24T01-33-33Z_research_rustfs-vs-seaweedfs/`, PR
  #13990): SeaweedFS stays as Cozystack's S3 layer; see
  [Object storage: RustFS and SeaweedFS](../platform/object-storage.md).
  Epic [#1088](https://github.com/developing-today/code/issues/1088)
  remains the programme home.
- **Platform services** from the register's day-one list: NATS,
  FluxCD, a Cloudflare tunnel, Tailscale on-cluster (epic #1088).
- **First workload:** the ID app on FluxCD, epic
  [#1092](https://github.com/developing-today/code/issues/1092) —
  with the register's open question #1 still open: which app
  exactly is "the ID app" that FluxCD runs first (gap 12 in
  [Gaps and open questions](../plans/gaps-and-open-questions.md)).

Status of everything in this section: **planned**. No node is
installed, no Cozystack deployment exists.

## History: the v1.9 test cluster, torn down

A real Talos cluster ran once. The v1.9-era test cluster — VIP
10.10.0.42, controllers a1/a2/b1, workers a3/a4/b2–b6, recorded in
`infra/talos/01b.us/` and `doc/talos-addresses.md` — ran Talos
v1.9.0 from an image factory schematic with the `siderolabs/drbd`,
`siderolabs/tailscale` and `siderolabs/xen-guest-agent`
extensions, with PXE, ISO and raw assets recorded. It was a test,
and it was **torn down** — confirmed by Drew on 2026-10-26.

Status: **proven, then retired**. Everything about it in this book
is past tense. Nothing from it is running, and the current
programme starts from bare machines. The only DRBD artefact that
ever ran is that cluster's `siderolabs/drbd` Talos extension —
historical; see
[Storage: LINSTOR and DRBD](storage-linstor-and-drbd.md).

## The proven piece: the TALM per-node disk flow (#1812)

Drew's open design question — how do you declaratively configure,
in Talos/TALM, a bunch of hard drives that are all different per
machine? — is **answered and closed**:
[#1812](https://github.com/developing-today/code/issues/1812),
landed as `infra/cluster/README.md` (with worked examples
`infra/cluster/examples/node-file.yaml` and
`infra/cluster/examples/user-volume.yaml`) via PR #5052. It is the
one proven cluster piece in this book, proven as design plus a
local container flow.

The answer, in brief:

- One TALM node file per node in git
  (`infra/cluster/<cluster>/nodes/<hostname>.yaml`).
  Running `talm template -i` against a node booted in Talos
  maintenance mode harvests its real hardware into that file as
  comments; anything below the file's modeline is a strategic
  merge patch over the shared cluster template at `talm apply`
  time. A non-empty body pins the file to exactly one node — a
  disk patch can never leak onto the wrong machine.
- Match disks by **identity, not description**: the boot disk via
  `machine.install.diskSelector` on serial/wwid (never size or
  model alone — a replacement drive must fail loudly, not
  silently match a sibling); data volumes via Talos v1.10+
  `UserVolumeConfig` CEL selectors with the load-bearing
  `!system_disk` guard. `machine.disks[]` is deprecated in v1.10 —
  do not build on it.
- The runbook covers adding a node, replacing a disk (a boot-disk
  swap is a rebuild of that node from config, by design), and
  wiping and rebuilding a node from config only — "the node
  rejoins the cluster from git alone". That runbook is the
  mechanism behind [Boot and auto-join](boot-and-auto-join.md).

The local container-flow proof recorded on #1812: `talm template
-i` harvest, hand-edit of disk fields only, `talm apply --dry-run`
showing the resolved install disk, then `talm apply` — observed in
Drew's local container runs and verified in triage on 2026-11-03.
What it does **not** prove: real drive-swap behaviour, or the
rebuild drill on physical nodes. Both are gated on the hardware
inventory,
[#1810](https://github.com/developing-today/code/issues/1810).

## The P9 draft — unratified

A fleet-drafted integration design, `doc/living/lab-integration.md`
(P9), lays out a phased landing plan for the lab: Atlas serve mode,
queue workers, PM surfaces and build overflow on the cluster,
backups proven by a recorded restore, and more. Its status is
exact: **a draft awaiting Drew's ratification**
([#8873](https://github.com/developing-today/code/issues/8873),
open). This book cites its contents only as draft contents. Where
a later chapter mentions a P9 phase, "planned (draft)" is the
whole claim.

## Version pins of record

From the research brief (2026-10-03): TALM is `cozystack/talm`
v0.35.0 — the canonical repository is cozystack/talm; Talos
v1.14.1; Cozystack v1.6.4; RustFS 1.0 GA 2026-09-16; managed
Postgres is CloudNativePG. These are the pins the programme was
researched against; they are research-baseline facts, not a claim
that anything runs them.
