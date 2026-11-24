# The cluster programme

The cluster programme is the plan to turn the basement fleet of
[The Osseo site](../site/the-osseo-site.md) into a Kubernetes
cluster: Talos Linux installed declaratively with TALM, running
Cozystack, with replicated storage and the project's own workloads.
This chapter gives the programme's structure and its current
state. The design areas have their own chapters:
[Talos and Cozystack](talos-and-cozystack.md),
[Storage: LINSTOR and DRBD](storage-linstor-and-drbd.md), and
[Boot and auto-join](boot-and-auto-join.md).

## Structure of record

- Parent epic:
  [#1817 — Basement Talos/Cozystack cluster program](https://github.com/developing-today/code/issues/1817).
- Project home: `planning/roadmaps/basement-cluster.md` in the
  code repository (PR #1082, merged).
- First status record: `doc/2026-10-27T03-14-08Z_status_basement-cluster/`.
- Twelve epics,
  [#1080](https://github.com/developing-today/code/issues/1080)
  through
  [#1093](https://github.com/developing-today/code/issues/1093),
  in org Project #4, with the Area option literally `lab-cluster`
  and execution-mode labels for online, hybrid and physical work.
- Thirteen detailed tasks,
  [#1094](https://github.com/developing-today/code/issues/1094)
  through
  [#1106](https://github.com/developing-today/code/issues/1106),
  under epics 1–3.
- A dashboard visualisation
  ([#1108](https://github.com/developing-today/code/issues/1108))
  is closed.
- Predecessors dezren39/nix#501–#508 (filed 2026-10-20) are all
  closed with superseded-pointer comments; their content lives on
  as code issues in the #1810–#1815 range and elsewhere.
- Physical bring-up parent:
  [#6066](https://github.com/developing-today/code/issues/6066) —
  racking, Dell switches ×3, edge routing and mesh Wi-Fi,
  interconnect, cluster bring-up.
- A fleet-drafted integration design, `doc/living/lab-integration.md`
  (P9), is a **draft awaiting Drew's ratification**
  ([#8873](https://github.com/developing-today/code/issues/8873),
  open). Nothing in it is decided until he ratifies it, and this
  book cites it only as a draft.

The twelve epics, by subject: #1080 hardware inventory and
first-node selection · #1081 firmware/BIOS/BMC audit · #1083
basement power and space · #1084 network fabric · #1085 storage
assembly · #1086 TALM and Talos bootstrap · #1087 Cozystack
platform · #1088 platform services · #1090 observability, backup
and recovery · #1091 cloud extension · #1092 first workload ·
#1093 the ESP32 wing (see [ESP32](../microcontrollers/esp32.md)).

## Hard rules

Two rules govern the whole programme, from the idea register,
stated explicitly there:

- **No HashiCorp anything.** Nomad was abandoned; the programme is
  all-in on Kubernetes.
- **Everything bootstrappable from config.** No hand-built state —
  the same rule that leaves the basement machines with no OS today
  (see [Boot and auto-join](boot-and-auto-join.md)).

## Current state

As of the latest status record (2026-10-27): **nothing in the
current cluster programme has been built yet.** What exists is
thorough research and planning, one proven local TALM container
flow, one live Cloudflare Tunnel on the laptop, and a large
ticketed programme. Component by component:

| Component | State |
|---|---|
| Talos Linux (current programme) | Planned — no node installed; "no node is installed until the TALM bootstrap flow is proven" (status record 2026-10-27). Epic [#1086](https://github.com/developing-today/code/issues/1086) open. |
| Talos (historical v1.9 test cluster) | Proven, then retired — torn down 2026-10-26. History only. |
| TALM bootstrap flow | Proven locally (containers); planned on bare metal. See [Talos and Cozystack](talos-and-cozystack.md). |
| Cozystack | Planned — the platform is **chosen, settled by Drew 2026-11-23**; not yet installed. Install epic [#1087](https://github.com/developing-today/code/issues/1087) is "proposed/Later" in the roadmap. |
| LINSTOR + DRBD | Planned — design only. See [Storage: LINSTOR and DRBD](storage-linstor-and-drbd.md). |
| Postgres (CloudNativePG via Cozystack) | Planned — decision recorded in the research brief: use Cozystack's CNPG-based managed Postgres; do not deploy a second operator. |
| RustFS (S3) | Planned; the RustFS-versus-SeaweedFS decision is with the comparison lane (Drew's current lean: SeaweedFS, 2026-11-23); epic [#1088](https://github.com/developing-today/code/issues/1088). See [Object storage: RustFS and SeaweedFS](../platform/object-storage.md). |
| Platform services (NATS, FluxCD, Cloudflare tunnel, Tailscale on-cluster) | Planned — the day-one list from the register; epic #1088. |
| First workload: the ID app on FluxCD | Planned — epic [#1092](https://github.com/developing-today/code/issues/1092). Which app exactly is "the ID app" is still an open question (gap 12). |
| Observability, backup and recovery | Planned — epic [#1090](https://github.com/developing-today/code/issues/1090), including the power-return boot order. |
| save-secrets tooling for Talos | Proven fix — [#6554](https://github.com/developing-today/code/issues/6554) closed via PR #8000 (atomic encrypt, fail loudly). |

### The one live artefact

The programme's first lab tunnel is its one live artefact: the
Cloudflare Tunnel of
[#6455](https://github.com/developing-today/code/issues/6455)
(closed), live since 2026-11-06, connector on the laptop (`amd`),
seed ingress `lab.security.cab`, NixOS service
`services.lab-tunnel`. It is proven — and it runs on the laptop,
not on any cluster machine. It is mentioned here, in the programme
chapter, because there is nowhere more built to mention it.

## Research baseline

The programme's research brief (2026-10-03, in the lab-cluster
goal workspace) pins the stack: TALM is `cozystack/talm` v0.35.0
(the goal's original `talos-systems/talm` name is wrong — canonical
is cozystack/talm), Talos v1.14.1, Cozystack v1.6.4, RustFS 1.0 GA
2026-09-16, managed Postgres = CloudNativePG. Stack fit as
recorded: TALM produces Talos configs, Cozystack runs on top, and
CNPG Postgres plus RustFS provide data services.

## Adjacent programme pieces

- **Containerised builds.** Generic containers for ID and fleet
  purposes are closed on the record
  ([#6076](https://github.com/developing-today/code/issues/6076),
  closed with label `status:done`): Nix-built generic containers,
  no hand-maintained Dockerfiles. The census read the issue body
  and its closed/done state but did not independently re-verify
  the proof artefacts, and this book cites it as closed-per-record.
  Containerised build/compile *in the cluster* is planned and
  waits on the cluster — see the
  [High-level roadmap](../roadmap/high-level-roadmap.md).
- **VMs near the lab.**
  [#5772](https://github.com/developing-today/code/issues/5772)
  (open) covers declarative guests (nixvirt / microvm.nix). Its
  recorded current state (2026-11-04 inventory): libvirtd enabled
  in the laptop profile and zero guest definitions anywhere; one
  seed microVM (`prometheus`, 2 vCPU / 1 GB) exists as a flake
  input with its profile commented out on `amd`. Planned.
- **Spare-PC roles and the quorum question.** The register
  addendum (2026-10-21, marked unverified dictation) names
  candidate roles for the spare PCs: basic compile/test runners;
  headless graphics via the integrated GPU serving remote
  sessions; Talos nodes in the 3–5-node cluster (open question
  whether they are strong enough — verify against actual specs);
  and quorum / tie-breaker witness nodes for LINSTOR/DRBD
  split-brain decisions in layouts with only two (or four)
  storage nodes — a Raspberry Pi is the named alternative for the
  witness role. The SBC question was taken up as
  [#6078](https://github.com/developing-today/code/issues/6078),
  which is closed; its body acceptance checkboxes stand unchecked
  and the census did not extract a verdict from its comments, so
  the per-role verdict is a gap (gap 8), not a claim in this book.
- **Cloud extension.** Alongside the cluster, an OCI always-free
  maximisation was planned in detail (epic
  [#1091](https://github.com/developing-today/code/issues/1091);
  child [#4904](https://github.com/developing-today/code/issues/4904)
  still open). But the memory of record states plainly that
  Oracle/OCI is not a backend — stood down 2026-10-31. The
  epic and child issues remain open on GitHub. This book records
  both facts and flags the pair as **unreconciled**, in
  [Gaps and open questions](../plans/gaps-and-open-questions.md) —
  it does not resolve the contradiction by picking a side.
- **Egress.** The egress strategy for the 2 Gbps home fiber is
  [#5030](https://github.com/developing-today/code/issues/5030)
  (open, design), with enforcement implementation
  [#8408](https://github.com/developing-today/code/issues/8408)
  (open). Direction of record: tunnel or shunt bulk traffic to
  cheap-or-free-egress services, compress aggressively, and only
  cross the home link with what must. (Budget figures from the
  record are cost material and are not reproduced in this book.)
- **boats.dev**, a container-hosting service with a free tier —
  not one of the project's domains, corrected 2026-10-25 — is a
  research item
  ([#1815](https://github.com/developing-today/code/issues/1815),
  open). Researched-only.

## What `pkgs/lab-ops-hub` is not

A footnote, because the name invites the mistake:
`pkgs/lab-ops-hub` is **not** the physical lab. Its own README
describes it as a Cloudflare full-stack demo — one Worker
exercising the Workers platform surface at personal scale — built
and deployed 2026-10-20. Its database was seeded with three
fictional machines named `talos-1/2/3` "matching the
basement-cluster sketch", which is a demo dataset, not the lab's
inventory. The lab connection is nominal and thematic only, and
this book gives it no chapter.
