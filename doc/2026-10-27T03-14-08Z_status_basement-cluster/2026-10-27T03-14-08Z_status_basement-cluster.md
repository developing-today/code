# Basement cluster status, 2026-10-26

- Status: status record
- Date: 2026-10-27T03-14-08Z
- Roadmap item: [Basement cluster roadmap](../../planning/roadmaps/basement-cluster.md)
- Issues: epics being filed

## State

Nothing has been built yet under the current program. The basement machines have no OS installed, by design: the whole fleet must be rebuildable from declarative config, so no node is installed until the TALM bootstrap flow is proven. The earlier v1.9-era Talos test cluster was only a test and has been torn down (confirmed by Drew, 2026-10-26).

What exists is research and planning:

- A component research brief (2026-10-03): TALM is `cozystack/talm`, Talos v1.14, Cozystack v1.6, managed Postgres is CloudNativePG, RustFS 1.0. The stack fits as TALM → Talos configs → Cozystack → CNPG Postgres + RustFS.
- An idea register (2026-10-20) capturing the full program: hardware facts, the storage sketch, hard rules (no HashiCorp, everything bootstrappable from config), the egress strategy, and the cloud program around the cluster.

The cloud extension is blocked at Oracle: the OCI always-free tenancy has been stuck in provisioning since 2026-10-21 (ticket CPCOE-29381; the recorded cloud account name is restricted and no valid replacement has been assigned). The next support check-back is 2026-10-27.

## Next steps

1. Hardware inventory: gather stats for all ~12 machines and pick the first 3–5 cluster nodes.
2. Firmware, BIOS and BMC audit: confirm versions and every setting are identical across the core machines.
3. Basement power and space: dedicated 20A circuits and rack layout, so the first nodes can run.
