# Roadmap: basement cluster

The basement cluster is a home lab built from the Threadripper machines in the basement: the first 3–5 machines become a Talos Linux cluster running Cozystack, bootstrapped declaratively with TALM, with Postgres (CloudNativePG), RustFS, NATS, FluxCD, a Cloudflare tunnel and Tailscale on top. The fleet behind it is ~12 Threadripper machines (core boxes identical in motherboard and CPU, differing in SSDs), 15–16 × 16 TB helium HDDs and at least 3 TB of DDR4 ECC RAM. Storage sketch: 3 hosts × 4 × 16 TB in RAID 10 for RustFS cold buckets, an all-990-Pro NVMe fast tier, a striped-and-mirrored NVMe mid tier, and DRBD + LINSTOR replication between machines.

Hard rules: **no HashiCorp anything** (all-in on Kubernetes), and **everything bootstrappable from config** — no hand-built state that only exists on one machine. Egress is the scarce resource (2 Gbps symmetric Brightspeed fiber), not compute, so bulk traffic is shunted to cheap or free-egress services. Basement power and space block the full fleet; the target is 3–5 machines first.

The earlier v1.9-era Talos test cluster (the layout in [`doc/talos-addresses.md`](../../doc/talos-addresses.md) and [`infra/talos/01b.us/`](../../infra/talos/01b.us/README.md)) was only a test and has been torn down; those files are historical reference only. How to read this file: [planning README](../README.md). Latest status: [2026-10-27 status record](../../doc/2026-10-27T03-14-08Z_status_basement-cluster/2026-10-27T03-14-08Z_status_basement-cluster.md).

## Now

| Title                                       | Status  | Intent                                                                                                                   | Links |
| ------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------ | ----- |
| Hardware inventory and first-node selection | planned | Gather hardware stats for all ~12 machines and pick the first 3–5 cluster nodes.                                         | #1080 |
| Firmware, BIOS and BMC audit                | planned | Confirm firmware versions and every setting are identical across the core machines, verified by routine KVM screenshots. | #1081 |
| Basement power and space                    | planned | Dedicated 20A circuits, cleanup and rack layout, so the first nodes can actually run.                                    | #1083 · [electrician brief](basement-cluster-electrician-brief.md) |

## Next

| Title                                  | Status  | Intent                                                                                        | Links |
| -------------------------------------- | ------- | --------------------------------------------------------------------------------------------- | ----- |
| Network fabric                         | planned | Switching and a new addressing plan, replacing the torn-down 01b.us test layout.              | #1084 |
| Storage assembly and design validation | planned | Build the HDD RAID 10 pools and NVMe tiers, validated against per-drive records.              | #1085 |
| TALM and Talos bootstrap               | planned | Prove the declarative bootstrap flow, including per-machine drives that are all different.    | #1086 |
| Cloud extension                        | planned | The OCI always-free fleet, egress offload, and cold tiering to R2 alongside the home cluster. | #1091 |

## Later

| Title                              | Status   | Intent                                                                                                        | Links                      |
| ---------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------- | -------------------------- |
| Cozystack platform                 | proposed | Install Cozystack on the Talos cluster and settle its upgrade path.                                           | #1087                      |
| Platform services                  | proposed | CNPG Postgres, RustFS (against Cozystack's bundled SeaweedFS), NATS, FluxCD, Cloudflare tunnel and Tailscale. | #1088                      |
| Observability, backup and recovery | proposed | Metrics, an etcd and TALM recovery vault, restore drills, and a power-return boot order.                      | #1090                      |
| First workload: the ID app         | proposed | Run the `id` app on FluxCD as the cluster's first real workload.                                              | #1092; [id roadmap](id.md) |

## Ideas

| Title      | Status | Intent                                                                                                             | Links |
| ---------- | ------ | ------------------------------------------------------------------------------------------------------------------ | ----- |
| ESP32 wing | idea   | A low-end section of ESP32 sensors and LoRa alongside the compute fleet. A side project of a side project for now. | #1093 |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
