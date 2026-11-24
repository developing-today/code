# P9 lab integration — sufficient-design record

Date: 2026-11-24. Status: provisional fleet design baseline, not Drew's ratification or statement. Tracking issue: [code#8873](https://github.com/developing-today/code/issues/8873). Living overview: [`doc/living/lab-integration.md`](../../living/lab-integration.md). Program: P9 / epic #8805; cluster program #1817.

## 1. Problem and context

The cluster stand-up work (X5, roadmap `planning/roadmaps/basement-cluster.md`) can deliver Talos and Cozystack without establishing which fleet workloads should run there, how they reach users and dependencies, or how they recover. That leaves a running cluster with no accountable tenant or migration proof. Current inputs describe a greenfield first 3–5 of roughly 12 Threadripper hosts, a laptop that remains canonical Atlas/ingest home, an orchestration-only sandbox, scarce 2 Gbps egress, and an explicit no-HashiCorp / declarative-bootstrap constraint. The roadmap orders inventory and firmware (#1080–#1081), physical readiness (#1083), storage (#1085), Talos/TALM (#1086), Cozystack (#1087), platform services (#1088), observability/recovery (#1090), cloud extension (#1091), and ID first workload (#1092). Existing service homes and readiness are not assumed to be movable until measured.

The gap is integration: define a safe progression from a reproducible platform to real workloads, then prove service replacement, data recovery, and placement across machines. Existing work establishes the desired first workload and broad phases; this record makes the decisions, boundaries, and acceptance sequence explicit.

## 2. Goals and non-goals

**Goals**

- Make Git-managed declarative state the source of truth for cluster platform and workload configuration; a node is rebuildable from recorded inputs, not hand-built state.
- Give ID a small, reversible first tenant deployment, then migrate only services with a named owner, dependency map, cutover, and rollback.
- Preserve existing canonical-data and security boundaries: laptop remains canonical Atlas/ingest; Turso remains the named shared ledger/hot-copy dependency where P6/P7 call for it; sandbox remains orchestration-only.
- Prove service health, network exposure, and backup restoration before declaring each phase complete.
- Keep placement visible across basement, laptop, sandbox, work machines, and approved cloud/CI capacity; route bulk egress to the established economical edge/cold tiers.

**Non-goals**

- This does not design electrical/rack work, hardware selection, TALM/Talos bootstrap internals, Cozystack installation, storage internals, the ID application, or P7/P8/P6 systems. Their roadmap/issues remain authoritative.
- No big-bang migration, automatic workload scheduler, replacement of laptop canonical data, or promise that basement-hosted services are available during a power/WAN outage.
- No HashiCorp products; no new cloud spend or credential/account assumptions; no claim that an unidentified “ID app” is already deployable.
- ESP32 wing and full 12-host expansion are outside the first integrated baseline.

## 3. Alternatives and tradeoffs

| Option | Benefits | Costs / risks | Decision |
| --- | --- | --- | --- |
| ID-first on the cluster, then migrate selected fleet services (chosen) | Exercises GitOps, ingress, identity, monitoring, and rollback with a bounded real tenant; aligns with #1092 and day-one intent | Does not immediately relieve laptop load; the exact ID service remains an open question | Use as provisional default; resolve service identity and readiness in slice 1 before deployment |
| Move Atlas serve or queue workers first | Potentially relieves laptop contention sooner; measurable workload | Couples first cluster proof to data sync, service ownership, and cutover coordination; less representative of the full platform path | Keep as an alternative if readiness evidence shows ID cannot be the first safe tenant; decide in slice 1, record rationale |
| Migrate all services together | One cutover | High blast radius, hidden dependency coupling, and poor rollback; contradicts per-service proof | Rejected |
| Leave workloads on current hosts; cluster is only a lab | Lowest migration risk | Does not meet integration purpose or validate operational value | Rejected as end state; acceptable temporary state before readiness gates |
| Make cluster a new canonical home for all data/services | Centralizes operations | Contradicts current laptop/Turso ownership and creates basement availability coupling | Rejected absent a separately reviewed data-ownership design |

## 4. Chosen approach and contracts

### Placement and ownership contract

Every workload placement row in the living overview must state: workload/service, current and target host, data-of-record and replicas, owner, dependencies, ingress path, backup/restore source, migration stage, and rollback target. A missing field means “not ready to migrate,” not permission to infer. Placement changes are explicit Git-reviewed changes; this design does not introduce runtime placement automation.

Laptop remains canonical for Atlas and ingest; cluster Atlas serve reads its named Turso hot copy. Turso-backed coordination consumers continue to use the same ledger, not a cluster-local fork. Cluster compute is a replaceable consumer, never the authority for those records. Sandbox stays orchestration-only. Any service that cannot tolerate basement power/network loss remains on its current/cloud home until its owner documents acceptable degradation and a tested fallback.

### Deploy and health contract

FluxCD reconciles only committed, reviewed desired state. A workload is “deployed” only when its declared readiness/health check passes, its dependencies are reachable, its version/configuration is recorded, and the operator can identify the active revision. Manual emergency changes must be captured back into Git before the next normal reconciliation. Platform service specifics and their upgrade policy remain owned by #1087/#1088.

### Exposure and trust boundaries

Public access is via the existing Cloudflare tunnel path (C7); private access is via Tailscale/tailnet (C9). Each service declares which path it needs; private-by-default is the safe default. Do not expose cluster control-plane, database, object-store administration, or orchestration endpoints through public ingress. Tunnel credentials and workload secrets are supplied by the platform's reviewed secret mechanism, never committed as plaintext. Restrict service-to-service access to declared dependencies; do not treat cluster membership as user authorization. ID's own authorization remains the ID application's contract. Confirm actual ingress/authn behavior in the service-specific slice before enabling external access.

### Data, backup, and egress contract

Each migrated service names its source of truth and recovery point. No migration may silently create two writable authorities. RustFS is a storage tier, not by itself a backup; backup completeness is proven by restoring into an isolated target and validating application-level data. Cold bundles continue to seal to the existing cold-store/Drive path; R2 remains edge tier unless P8 changes that policy. Bulk transfer must be bounded and scheduled against the documented egress constraint. Exact retention, RPO/RTO, and storage allocation remain P8/#1090 decisions and must be filled in before a stateful service's production cutover.

### Availability and rollback contract

Cluster-hosted availability is bounded by basement power, network, and the dependent services' availability. Never describe a cluster tenant as highly available until failure-domain and restore evidence supports that claim. Every cutover is reversible: deploy/validate the cluster copy without authority, switch traffic or consumers only after a recorded readiness check, retain the previous home intact through the observation window, then revert routing/consumer configuration on failed health or data divergence. Stateful rollback must specify write fencing and reconciliation before cutover; if that cannot be proven, do not switch writes.

## 5. Dependency-ordered build slices

Slices are intentionally one-lane-sized and produce artifacts/proofs rather than assuming physical work is complete. Existing roadmap issues are dependencies, not duplicated implementation scopes.

| Order | Slice and dependency | Acceptance criteria |
| --- | --- | --- |
| 1 | **Readiness and tenant contract** — after #1080, #1086, #1087, #1088 are sufficiently complete to name actual endpoints; coordinate with #1092. Identify the deployable ID component, owner, data stores, dependencies, ingress, and acceptable outage. Compare ID-first with Atlas/queue-first using current readiness evidence. | A checked-in service row and deployability checklist name image/build source, config, secrets mechanism, probes, data authority, exposure, backup, and rollback. If the ID app is not ready, choose the next safe candidate with evidence; no deployment proceeds on ambiguity. |
| 2 | **Declarative ID canary** — depends on slice 1 and a working FluxCD platform (#1088). | From reviewed Git state, deploy the selected ID component to a non-authoritative/canary environment. Flux reconciliation and health are observable; public/private paths are explicitly tested; no control-plane or admin endpoint is exposed. Revert the Git revision and demonstrate the prior state returns. |
| 3 | **ID first-tenant cutover** — depends on slice 2 and the C7/C9 paths. | Record the service/version, readiness, access path, data/backup plan, and operator instructions. If stateful, prove no dual-writer interval and restore a backup before any authority switch. Demonstrate rollback to prior home; keep prior home available through a named observation period set by the service owner. |
| 4 | **One-service migration template and first fleet service** — depends on phase-1 evidence. Select Atlas serve, queue workers, or PM surface using measured contention/availability and dependency readiness; do not migrate all three together. | A service-specific row lists owner, baseline, target, dependencies, data authority, traffic/consumer cutover, rollback, and checks. Run on cluster with old home disabled for seven consecutive days where safe; record incidents and resource/egress observations. If disabling old home is unsafe, document why and use a bounded shadow/canary proof instead. |
| 5 | **Storage/backup integration** — depends on #1085, #1090 and P8 policy. | Add cluster tier to the named allocation policy without changing source-of-truth semantics. Restore a cluster-held backup into an isolated target; validate application-level data and record command, artifact identity, and result. State retention and measured recovery time. |
| 6 | **Placement and overflow accounting** — depends on P7 dynamic-compute placement and slices 1–5. | Living placement table accounts for every known fleet workload × eligible host/pool, including explicit “not eligible” reasons. Show one real overflow path in each direction only where supported; no unmeasured claim of bidirectional capacity. Review table against current service inventory. |

## 6. Rollout and rollback

Roll out in the six slices above, stopping at any failed readiness or recovery check. First deploy as a non-authoritative canary. External traffic and worker consumption switch one service at a time, with a named operator, change window, health signal, and retained previous home. For stateless services, revert ingress or consumer configuration to the previous home. For stateful services, rollback is permitted only after write fencing, captured migration delta, and a tested reconciliation procedure; otherwise freeze writes and restore from the chosen authority rather than creating two writers. Remove the old home only after the service owner records the observation-window evidence and recovery proof. A cluster-wide outage reverts only services with a documented alternate home; others are declared unavailable rather than silently redirected to an unsafe copy.

## 7. Verification plan

- **Declarative rebuild:** wipe/rebuild a designated test node from committed configuration and recorded hardware identity; compare resulting platform state against the declared inventory. Owner/proof source: #1086.
- **GitOps:** change a canary revision, observe Flux reconciliation and health, revert it, and observe restoration. Preserve revision IDs and timestamps.
- **Access/security:** test the declared public tunnel and tailnet paths from their intended networks; verify undeclared/admin/control-plane paths are unreachable from those networks. Capture service-level auth checks, not just tunnel reachability.
- **Migration:** baseline current service behavior, run canary, perform cutover, disable old home for the agreed period where safe, and exercise rollback. Compare error/latency/queue-lag or other service-specific oracle to the named baseline.
- **Recovery:** restore a real cluster-held backup to isolation and run the service's application-level consistency check. A successful object copy alone is not acceptance.
- **Placement/egress:** inspect placement entries against service inventory; record bytes and route for one representative bulk transfer before claiming egress policy works.

Each slice's issue must state its oracle and attach command/output, dashboard/export, or an auditable run record. “Pod running” alone is not proof of application service, data integrity, access control, or recovery.

## 8. Open questions and deciders

These are decisions within follow-on slices, not a global gate awaiting Drew. The named worker/service owner decides a safe default, records evidence and rationale in the slice issue, and continues; later requirements may amend the baseline.

| Question | Provisional default | Decider / latest point |
| --- | --- | --- |
| Which component is “the ID app”? | No deployment until a concrete deployable component and owner are identified; favor the smallest user-facing component that can be canaried without moving canonical data. | ID service owner with slice-1 worker; before slice 2. |
| ID-first versus Atlas/queue-first? | ID-first if slice 1 readiness passes; otherwise choose the safest measured laptop-relief candidate. | Slice-1 worker with candidate service owner; record evidence before deployment. |
| Which workloads must never depend on basement power/connectivity? | Any service with a documented continuous-availability requirement or no safe alternate remains off-cluster. | Service owner for each migration; before that service's slice. |
| First-node count and full-fleet pace? | Keep 3–5 initial nodes; do not scale out until power, inventory, and recovery capacity are verified. | Cluster operator in #1080/#1083 and each phase review. |
| Is OCI free-tier part of P9? | Treat as an external capacity option under #1091/P7, not a prerequisite for cluster integration. | Cloud-extension owner; before slice 6 claims cloud overflow. |
| Exact RPO/RTO, retention, RustFS allocation? | No stateful production cutover until P8/#1090 records these per service. | P8/storage owner with service owner; before slice 3 or 5 as applicable. |
| Exact service mesh/network policy and secrets implementation? | Use the platform's supported, reviewed mechanism; deny undeclared ingress and avoid introducing a new controller solely for this design. | Platform operator in #1088; before slice 2. |

## 9. Tracking and dated record

This is the dated design record for the living P9 overview, linked from tracking issue #8873. That issue's 2026-11-22 decision comment establishes that the fleet draft may be used as a provisional working baseline and that Drew's ratification is not a release condition. The design remains explicitly not Drew's words. Build work proceeds through the existing roadmap issues and the slices above; amend this record when evidence changes a contract or phase order. No implementation is included in this design task.

### Sources checked

- `planning/roadmaps/basement-cluster.md` (roadmap order and initial cluster shape).
- `doc/living/lab-integration.md` (P9 phase intent, C7/C9, machine roles, and stated open questions).
- `doc/living/github-dynamic-compute.md` (P7 placement/overflow context).
- `doc/living/storage-program.md` (P8 tier and backup allocation context).
- Tracking issue [#8873](https://github.com/developing-today/code/issues/8873), including the 2026-11-22 provisional-baseline decision.
