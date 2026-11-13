# Lab integration (P9)

> **DRAFT — drafted by the fleet 2026-11-12 from Drew's reprioritization order; awaiting Drew's ratification. Not Drew's own words.**

- Living document · Status: DRAFT · Program: P9 Lab build-out + integration (band 40) · Register entry: X5 · Tracking epic: developing-today/code#8805 · Program epic: developing-today/code#1817 · Milestone: "Lab cluster" · Label: `epic/lab-cluster`

## Purpose

X5 stands the cluster up. This design is the other half of Drew's 2026-11-12 order: **integration** — how the fleet's existing work (ID, Atlas, the fleet services, storage, the PM stack) lands on the Talos/Cozystack cluster and the other machines, and what "fully using the cluster" concretely delivers. Stand-up without integration is a cluster that runs nothing that matters; this document defines the landing so the build-out has a target beyond "Kubernetes is up."

## Current state (from the register and goals)

- **X5 — Lab cluster program (PARENT):** goal `stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs`; program epic code#1817 ("EPIC: Basement Talos/Cozystack cluster program", OPEN); roadmap at `planning/roadmaps/basement-cluster.md` (PRs #1082/#1109 merged).
- **X5.2 — the 2026-10-20 brain-dump (the design basis):** ~12 Threadripper machines (same motherboard/CPU, differing SSDs), 15–16 × 16 TB HDDs, ≥3 TB DDR4 ECC, NVMe boot drives, enterprise BMC/IPMI, high-speed NICs (exact spec TBD); target 3–5 machines first (power and basement state are the long tail). Hard constraints: **no HashiCorp anything; everything bootstrappable and declarative**; TALM-harvested per-node Talos configs; on the cluster from day one: CloudNativePG Postgres, RustFS (S3) with DRBD/LINSTOR tiers, Cloudflare tunnel, Tailscale, NATS, FluxCD running "the ID app". Egress is the scarce resource: 2 Gbps fiber; bulk bandwidth shunts to Cloudflare/Turso; cloud budget rules (Turso $5 + Cloudflare $5 in play, $25–30 later maybe, $75 out).
- **X5.3 — project structure seeded (2026-10-26):** 12 epics filed (code#1080–#1093: inventory, firmware/BMC, power, network, storage, TALM+Talos, Cozystack, platform services, observability/backup, cloud extension, first workload, ESP32 wing) with detailed tasks #1094–#1106; every cluster issue states its execution mode (online / hybrid / physical); all seated in Project #4, Area=lab-cluster. The old v1.9-era test cluster is torn down — greenfield by Drew's correction.
- **Execution tickets (goal record):** code#1086 (TALM + Talos bootstrap) + code#1087 (Cozystack platform) under #1817; physical bring-up parent code#6066 (racking, switches, edge routing, mesh Wi-Fi; cluster interconnect code#6077; generic containers code#6076).
- **Already landed:** the lab's first Cloudflare Tunnel is live (C7, code#6455, connector on amd, `lab.security.cab` seed ingress); the dual-exposure direction (C9: Tailscale Serve ↔ private Cloudflare Tunnel) is drafted.
- **The other machines:** the Framework laptop (canonical Atlas home, ingest factory, 4 TB), the sandbox (orchestration only), Drew's work machines — integration covers them, not just the basement.

## The design — what "fully using the cluster" means

Integration proceeds in phases; each phase names what lands, where, and the proof.

**Phase 0 — Declarative ground (X5.2/X5.3, in progress).** Inventory + BMC audit complete; TALM per-node configs harvested and committed; Talos + Cozystack up on the first 3–5 machines; platform services (CNPG, RustFS tiers, NATS, FluxCD, cloudflared, Tailscale) running from GitOps only — nothing hand-built. _Proof: a node can be wiped and rebuilt from config alone._

**Phase 1 — The first workload is ours.** FluxCD runs the ID app (X5.2's day-one list) — the cluster's first real tenant is the fleet's own flagship (P1), not a demo. The ID hosted backstop (P4's ID face) gets its self-hosted reference deployment here. _Proof: ID served from the cluster, reachable via the C7 tunnel + tailnet dual exposure (C9)._

**Phase 2 — Fleet services land.** The services the fleet already runs elsewhere get cluster homes, one at a time, each replacing (not duplicating) its current host where that host is a constraint:

- Atlas serve mode (A1.12) — the search service on cluster compute, reading the Turso hot copy; the laptop stays canonical.
- Fleet coordination consumers — queue workers / brokers that today contend for the laptop run as cluster workloads against the same Turso ledger (P6 pattern: ledger in Turso, compute wherever it fits).
- PM stack surfaces — the aggregator/projector class of services, with the coordinated-switch discipline (ports hand over, sessions survive) as the deployment rule.
- Build/compute overflow — cluster nodes join the dynamic-compute placement table (P7) for heavy, parallel, non-latency-sensitive work that today saturates the laptop.
  _Proof per service: it runs on the cluster for a week with its old home off, and nobody pages._

**Phase 3 — Storage integrates (with P8).** RustFS tiers on the HDD/NVMe pools become a named tier in the storage program's allocation policy: cluster-local S3 for working sets and replicas, cold bundles still sealing to Drive, R2 still the edge tier. Cluster backup/observability epic (#1080s family) closes the loop: the cluster backs up into the cold-store like every other store. _Proof: a restore from cluster-held backup media, recorded._

**Phase 4 — The other machines are first-class.** Laptop, sandbox, and work machines appear in the same inventory/placement view as cluster nodes (what runs where, and why); the sandbox stays orchestration-only; the laptop keeps canonical Atlas + ingest; overflow flows both ways (cluster → Codespaces/Blacksmith per P7, machines → cluster per Phase 2). _Proof: one placement table (this doc, kept current) accounts for every fleet workload._

## Deliverables

- [ ] This design ratified or amended by Drew.
- [ ] Phase 0 exit: rebuild-from-config proof on one node, recorded.
- [ ] Phase 1 exit: ID app live on the cluster via FluxCD, dual-exposed (tunnel + tailnet), with the access paths documented.
- [ ] Phase 2: per-service landing list (Atlas serve, queue workers, PM surfaces, build overflow) each with owner, order, and the week-on-cluster proof.
- [ ] Phase 3: RustFS tiers written into the P8 storage allocation policy; cluster backup into cold-store proven by a recorded restore.
- [ ] Phase 4: the fleet placement table (every workload × every machine/cluster/cloud pool) maintained in this doc.
- [ ] X5.2's open questions closed or re-asked in one batch: which app exactly is "the ID app" FluxCD runs first; OCI account details; exact NIC spec; Turso account/DB state for cluster use.

## Open questions for Drew

1. Which app is "the ID app" — the Phase 1 tenant definition gates everything downstream (open since 2026-10-20).
2. Phase order: is ID-first right, or should Atlas serve / queue workers land first because they relieve the laptop soonest?
3. What may the cluster NOT host? (Anything that must never depend on basement power/connectivity — e.g. is the PM stack's dead-man's-switch class of service excluded by design?)
4. Power/bring-up pace: is 3–5 machines still the target for the first fully-integrated phase, with the rest of the ~12 as a long tail?
5. OCI free-tier fleet (X5.2 #507, cloud-extension epic code#1091): part of "fully using" for this program, or a separate track?
