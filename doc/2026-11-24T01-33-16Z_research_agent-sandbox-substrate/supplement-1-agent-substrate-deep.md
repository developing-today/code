# Supplement 1 — Agent Substrate deep-dive research note

_Research lane note, 2026-11-23 (America/Chicago). Web research only. All
performance figures are vendor/project claims unless explicitly labelled
independent. Primary sources preferred; secondary sources flagged._

## TL;DR

Agent Substrate is a Google-originated, Apache-2.0, Kubernetes-native
runtime that multiplexes many stateful agent sessions (**Actors**) onto a
small warm pool of sandboxed **Worker Pods**, checkpointing idle Actors to
object storage and restoring them — Google claims — in **<500 ms**.
Isolation is **gVisor (`runsc`) or Cloud Hypervisor microVMs (via Kata) —
plain containers are explicitly not supported**. It was accepted into the
**CNCF Sandbox** in autumn 2026 and Google announced the donation,
alongside gVisor and OpenRL, on **5 Nov 2026**, the same day **v1.0.0**
shipped and **Agent Substrate on GKE** went GA. It is _not_ an agent SDK
and _not_ the separate `kubernetes-sigs/agent-sandbox` project.

## 1. What it is, origin, CNCF status, licence

| Fact                        | Detail                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What                        | "Security-hardened, zero-trust runtime for sandboxed agents, engineered to safely run millions of agents and tools with 10x higher density than standard containers" — README. "A low-opinion runtime, not an agent SDK"; workloads need not literally be AI agents, but autonomous, untrusted, mostly-idle agents are the target. Runs standard OCI images; framework-agnostic (ADK, LangChain, Claude Code, Codex, Antigravity, MCP servers listed).                                                                                 |
| Originator                  | Google. Public development / repo creation **13 May 2026** (GitHub repo metadata; CNCF page confirms first commit 13 May 2026). First introduced publicly May 2026 alongside Agent Sandbox on GKE.                                                                                                                                                                                                                                                                                                                                     |
| Repo                        | **https://github.com/agent-substrate/substrate** — org `agent-substrate`. Docs site: `ate.dev`; learning site: `learn.agentsubstrate.dev`. Language: Go.                                                                                                                                                                                                                                                                                                                                                                               |
| Licence                     | **Apache-2.0**. v1.0.0 release notes include: copyright changed to "The Agent Substrate Authors" on donation.                                                                                                                                                                                                                                                                                                                                                                                                                          |
| CNCF status                 | **Sandbox**. Donated alongside **gVisor** and **OpenRL**, both also Sandbox; Google says gVisor is "on a path toward Incubation."                                                                                                                                                                                                                                                                                                                                                                                                      |
| Dates — discrepancy flagged | Google Cloud Blog (2026-11-05): donation announced "following their acceptance **last month** into the CNCF Sandbox," at KubeCon NA, Salt Lake City, Nov 9–12. Sandbox application is **cncf/sandbox#523** (gVisor #521, OpenRL #518). **CNCF's project page says "accepted to CNCF on September 28, 2026."** Secondary trackers say application created 2026-09-08, closed as accepted **2026-10-20**. Not reconcilable from a single primary source — "accepted Sept–Oct 2026, donation announced 5 Nov 2026" is the safe statement. |
| Managed sibling             | **Agent Substrate on GKE** GA 5 Nov 2026. Distinct again from Google's **Agent Compute Engine** (Titanium microVM service) and from **kubernetes-sigs/agent-sandbox**. Secondary coverage frequently conflates these three — do not.                                                                                                                                                                                                                                                                                                   |

## 2. Architecture

### Core split: Kubernetes provisions, Substrate schedules

Kubernetes is kept **off the hot path**. K8s schedules/heals Worker Pods
ahead of time; Substrate's own control plane assigns Actors to
already-running workers in milliseconds, avoiding kube-scheduler, Pod
creation, and etcd churn per activation — Google's stated rationale is
that kube-apiserver/etcd "is not designed to handle millions of
resources," and Pod startup (seconds) dwarfs agent turns (ms–low seconds).

### Components (from `docs/concepts/architecture.md` on `main`, and learn.agentsubstrate.dev/topology)

**Control plane**

- **`ate-api-server` / `ateapi`** — "the brain." gRPC API (mTLS :443);
  scheduler (claims a ready worker), workflow engine for
  Resume/Suspend/Revert/Delete; golden-snapshot reconciler. AuthN via
  mTLS + OIDC, per-atespace authZ with an embedded **OpenFGA** model.
- **State store: PostgreSQL** — atespaces, ActorTemplates, Actors,
  Workers, ActorAssignments, tags, leases. Bundled StatefulSet by
  default; BYO / Cloud SQL supported. _Docs-lag warning: older material
  describes `ateapi` as backed by Redis/Valkey — current `main` says
  PostgreSQL._
- **`ate-controller`** — reconciles `WorkerPool` CRDs into worker
  Deployments; resolves/protects `SandboxConfig`s.
- **`ate-idp-server`** — OIDC discovery / JWKS for actor JWTs.
- **`podcertificate-controller`** — signs pod certificates / SPIFFE pod identity.

**Data plane (per node / per worker)**

- **`atelet`** — DaemonSet, mTLS gRPC :21998. Receives
  Run/Checkpoint/Restore, stages runtimes/images, drives the worker over
  a Unix socket, moves snapshots via its sidecar, keeps node-local
  snapshot + sandbox-asset caches.
- **`ateworker`** — inside each Worker Pod: `ateworker-gvisor` or
  `ateworker-microvm`. gRPC: `RunWorkload` / `CheckpointWorkload` /
  `RestoreWorkload`. Invokes `runsc` for gVisor, or Kata + Cloud
  Hypervisor for microVMs.
- **`atunnel`** — in the worker pod; `atenet-router` opens an mTLS
  `CONNECT` tunnel to it (:8443), it forwards to the Actor over a private
  veth; also intercepts actor egress.
- **`objectstore-plugin`** — sidecar to `atelet` and `ate-api-server`;
  the _only_ component holding object-storage credentials. Backends:
  **GCS or S3**.

**Networking (`atenet`)**

- **`atenet-router`** — ingress gateway, Envoy (default) or AgentGateway.
  Client names the target in header **`ate-target-actor:
<atespace>/<actor>`**; it calls `ResumeActor`, **parks** the request
  while the actor resumes, then tunnels it.
- **`atenet-egress`** — egress gateway / policy enforcement point.
  Terminates `atunnel` CONNECT, identifies the actor, enforces
  destination policy, **injects credentials outside the sandbox**.
- Arbitrary-port ingress: only **HTTP(S) over the CONNECT tunnel** today;
  "raw TCP or other non-HTTP protocols on a non-default port are not yet
  reachable" — architecture doc, verbatim limitation.

**Security model (README)**

- Every actor treated as untrusted / mutually hostile.
- Per-actor short-lived **SPIFFE X.509** certificate by default; egress
  gateway can mint per-actor JWTs on demand.
- **Zero credentials in the sandbox.**
- **Default-deny networking**: no inbound except via router, no lateral
  actor-to-actor or control-plane access, outbound only via egress gateway.

### Kubernetes integration — CRDs vs. API records

| Resource                             | Where it lives                                                                                                                                            |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `WorkerPool`                         | **K8s CRD** — pool of warm worker pods; reconciled to a Deployment; selects a sandbox class                                                               |
| `SandboxConfig`                      | **K8s CRD, cluster-scoped** — sandbox runtime versions/assets per class (`gvisor` or `microvm`); a memory restore requires a worker offering that runtime |
| `CSIDriverConfig`                    | **K8s CRD** (per topology docs)                                                                                                                           |
| `Atespace`                           | Substrate API / Postgres — logical grouping + identity scope; _not_ a K8s namespace                                                                       |
| `ActorTemplate`                      | Substrate API / Postgres — immutable actor-version: OCI image (digest), CPU/memory, env, volumes, snapshot config                                         |
| `Actor`, `Worker`, `ActorAssignment` | Substrate API / Postgres — high-frequency dynamic state                                                                                                   |

CLI: **`kubectl-ate`** plugin; installer: `ate-setup`. Supported K8s:
**only latest stable + previous minor** (at Nov 2026: **1.37 / 1.36**);
GKE docs narrow the managed path to **GKE Standard, 1.37+, x86 nodes —
Autopilot not supported** (secondary reporting of GKE docs).

### How sandboxes are provisioned

1. Admin creates `WorkerPool`(s) → controller starts long-running Worker
   Pods, idle and waiting.
2. Developer creates an `ActorTemplate` → Substrate boots it once and
   bakes a **"golden snapshot" (Version 0)**.
3. `CreateActor` creates _only a Postgres record_, state `SUSPENDED`,
   pointing at the golden snapshot — no pod, no sandbox yet.
4. Request → router parks it → control plane claims a worker →
   `atelet`/`ateworker` restores the golden (first run) or the actor's
   own latest snapshot → state `RUNNING` → router tunnels the request.

### Isolation

- **gVisor class (default):** `ateworker-gvisor` drives **`runsc`**.
  gVisor's Sentry is a userspace application kernel (Go) intercepting
  syscalls; a Gofer mediates file access. Because gVisor virtualises its
  own memory/process tables, it can freeze/write/restore memory pages —
  the path the <500 ms claim rests on.
- **microVM class:** `ateworker-microvm` drives a **Kata Containers guest
  on Cloud Hypervisor**, needs `/dev/kvm`.
- **Not plain containers.** A WorkerPool is single-class, and **a gVisor
  checkpoint cannot be restored on a microVM worker and vice-versa**.
- Each actor gets its own sandbox, cgroup, and network namespaces
  _inside_ a shared Worker Pod — a worker hosts _many_ actors
  concurrently. Older v0.x-era write-ups saying "one actor per worker"
  are superseded.

## 3. State model, warm pool, snapshot/restore

### What exactly is "agent state"?

An **Actor** = one agent session, holding:

- **Memory state** — running process tree + RAM
- **Filesystem state** — container rootfs writable overlay + durable volumes
- **Environment** — env vars, filesystem, network rules from its template
- **Discrete identity** — atespace/name, SPIFFE identity, egress policy —
  held by Substrate/gateways, _not_ by the pod and _not_ (credentials)
  inside the sandbox

Fidelity is tiered, via `snapshotConfig.preferredFidelity`:

1. **`VOLUMES`** — only `DurableDir` volumes.
2. **`ROOTFS`** — `VOLUMES` + files written to each container's writable
   rootfs overlay. Restores by **cold-booting** on those files.
3. **`MEMORY`** (default) — `ROOTFS` + full process tree and RAM.

Ties: `MEMORY`/`ROOTFS` state is tied to the `ActorTemplate` UID;
`MEMORY` is _also_ tied to the `sandboxRuntime` version. Changing
template while suspended keeps only `VOLUMES`; if no worker offers the
snapshot's runtime version, resume **falls back to `ROOTFS` + cold boot**.

State machine: `SUSPENDED → RESUMING → RUNNING → SUSPENDING → SUSPENDED`,
plus `CRASHED`; `RevertActor` discards in-progress state and returns to
the last _durable_ snapshot.

### Decoupling, concretely

Actors (records + snapshots in Postgres/object storage) outlive Workers
(pods). An actor can suspend on Worker A and resume on Worker B ("Actor
Teleport"). Identity follows the Actor record, not the Pod. Total actors
are bounded by storage; concurrent actors by worker slots.

### Warm pool

`WorkerPool` = pre-started Worker Pods, sandboxes idle and waiting. It
exists solely to keep kube-scheduler, image pull, and pod startup out of
the resume path. README demo claim: ~250 stateful actors multiplexed
across 8 physical pods; **10×–30× oversubscription** of actors over
worker pods.

### Snapshot/restore — step by step

**Technology verdict: NOT CRIU.** No primary source describes Substrate
using CRIU. CRIU appears only in GitHub issue #121 (external proposal)
asking for a pluggable backend _including_ a hypothetical CRIU option.
The real mechanisms:

- **gVisor:** gVisor's **native `runsc` checkpoint/restore**
  (`runsc checkpoint --image-path …`, `runsc restore`, filesystem bundles
  via `runsc fscheckpoint`). Architecture doc notes the gVisor backend
  "currently requires a `runsc` version with the
  `--allow-connected-on-save` flag to work around a bug in networking
  resumption during checkpointing."
- **microVM:** memory-only VM snapshot, restored on-demand with
  **`userfaultfd` demand-paging**. Rootfs overlay is host-assembled and
  served over virtio-fs, so `ROOTFS`/`VOLUMES` snapshots ship the upper /
  volume dirs as **tars** and cold-boot.
- **Compression:** secondary sources consistently describe snapshots as
  **zstd-compressed**; not found in the primary architecture doc — treat
  as _well-attested secondary, not primary-verified_.

**Suspend** (explicit/API-triggered per primary docs; no primary
statement of an automatic idle-timeout value found):

1. `atelet` instructs `ateworker` to freeze and capture memory+disk snapshot.
2. Snapshot written **node-locally first** (`/var/lib/ate`).
3. **Async (default):** `SuspendActor` returns once the local checkpoint
   is written, snapshot marked `PENDING`; `atelet` uploads to GCS/S3 in
   the background (`--snapshot-upload-delay`, **default 5 min** per the
   v0.6.0 load test's reading of source) → `COMPLETED`. **Sync
   (`DURABILITY_MODE_SYNCHRONOUS`):** upload finishes before return.
4. Worker is wiped and returned to the pool.
5. Local copy retained as snapshot `locality` until
   `--snapshot-detach-delay` (**default 30 min**) or until the node needs
   the room — enabling fast same-node resume.

**Resume:**

1. Request hits `atenet-router` with `ate-target-actor`; router **parks
   it in memory** and calls `ResumeActor`.
2. `ate-api-server` scheduler claims a warm worker (Postgres records,
   atomic assignment).
3. That node's `atelet` downloads the snapshot from object storage
   _unless_ the actor still has locality on that node, then calls
   `RestoreWorkload`: gVisor restore or microVM `userfaultfd` restore.
4. `atelet` returns the worker pod IP; state → `RUNNING`; router opens
   mTLS CONNECT to that worker's `atunnel`, which forwards the parked
   request to the actor port.

**Tags:** snapshots can be tagged per-atespace — an immutable
alias/retention pin holding its _own copy_ of the snapshot.

### Known limitations

- **Open connections do NOT survive.** GKE documentation as reported by
  multiple secondary sources: "Open network connections, such as database
  sessions or connections to MCP servers, are not preserved when an agent
  suspends" — code must reconnect. gVisor upstream semantics: connected
  sockets return `ECONNRESET` on restore; listening sockets are
  recreated, backlog connections lost.
- **GPU:** GKE Substrate docs: **"GPU device passthrough into Actor
  containers isn't supported"** and gVisor "can't snapshot live CUDA
  contexts" — inference must run outside the sandbox. NVIDIA's Substrate
  post says GPU support is being (re)built on **Dynamic Resource
  Allocation (DRA)**. _Conflict noted:_ Google's Nov 5 Cloud Blog says
  gVisor in general "supports GPUs, TPUs, and checkpoint/restore
  natively." The Substrate-on-GKE limitation is the operative one today.
- **Durability window (async default):** until background upload
  completes, the only copy is on one node's disk. Architecture doc:
  "asynchronous suspends risk state loss if the node fails before
  background upload completes." Spot/autoscaler/upgrade node loss is the
  exposure.
- **Cross-node resume is expensive:** an actor with locality is
  effectively pinned; if its node has no free worker, upload then
  download happen _in series, while the request waits_ (5–10 s on fast
  disks, ~30 s p50 on a saturated default disk in the load test, vs <1 s
  local).
- **Snapshot size / storage cost is the real price.** Cost ≈ snapshot
  size × suspend frequency in disk write bandwidth, plus object storage.
  GKE design guide estimate: snapshot size ≈ ActorTemplate memory limit +
  rootfs/durable writes (an upper bound; sparse touched memory measured
  ~⅓ of it). No primary source publishes a per-snapshot dollar cost.
- **Runtime/class lock-in per snapshot** (see fidelity ties above).
- **CPU-feature constraint (gVisor upstream):** restore host must possess
  all CPU features of the checkpoint host unless stabilised via the
  `dev.gvisor.internal.cpufeatures` annotation.
- **Non-HTTP arbitrary ports unsupported**; **~800+ sandboxes/node needs
  kernel neighbour-table tuning** — README warns hosts running >~1,000
  sandboxes need `net.ipv4.neigh.default.gc_thresh*` raised.
- **GKE-only constraints (managed path):** Standard clusters only, no
  Autopilot, ≥1.37, Dataplane V2, `c3-standard-4` or larger recommended —
  per secondary reporting of the GKE install guide; the GKE doc pages
  were not fetched directly in this lane, so treat as _reported_.

**Could NOT verify — do not assume:**

- **Clocks:** No Substrate-specific clock documentation found. _gVisor
  upstream_ semantics (inherited on the default class):
  `CLOCK_MONOTONIC` does **not** count checkpointed time;
  `CLOCK_BOOTTIME` and `CLOCK_REALTIME` **jump forward** by the suspended
  duration, and timers that fell due during suspension expire on restore.
  Whether Substrate adds anything on top: **not documented**.
- **Randomness / entropy, `/dev/urandom` reseeding, UUID duplication
  across restores from a shared golden snapshot:** **no statement found**
  in README, architecture doc, or GKE-doc reporting. A genuine docs gap —
  an open question for the project, not a known-safe or known-broken area.
- **Snapshot size/cost benchmarks, encryption of snapshots at rest,
  snapshot GC/retention defaults beyond tag semantics:** thin/absent.

## 4. Maturity

| Signal                     | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Age                        | Repo created 13 May 2026 — ~6 months old at 1.0.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Releases                   | **12 releases/tags**. Known sequence: v0.1.0 (2026-09-10, secondary tracker), v0.4.0 (13 Oct), v0.6.0 (25 Oct), v1.0.0-rc.1 (29 Oct), -rc.2 (4 Nov), -rc.3 (5 Nov), **v1.0.0 "Fully armed and operational" (5 Nov 2026**; incl. Envoy 1.39.5, Kata assets 4.3.1, gVisor 20261026.0), **v1.0.1 (20 Nov 2026** — per secondary trackers: `atenet-egress` hardening + Go 1.27.4; _release page not opened directly_). Pre-1.0 minors were **reinstall-only, no in-place upgrade**.                                                                                                                                                                 |
| 1.0 promise                | README at 1.0: compatibility/stability commitment for public API, CLI, config. _Counter-signal:_ NVIDIA's integration post reportedly warns APIs are still changing.                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Activity                   | **8,224 stars, 911 forks, 1,790 commits** at fetch (23 Nov). Growth: ~2.0k stars (Sept), ~5.4k (Oct), 6,610 (14 Nov), 8,224 (23 Nov). Very active.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Contributors — discrepancy | Google (5 Nov): "**more than 100 contributors**"; secondary GitHub-API count ~**129** (14 Nov), "40+ organizations". **CNCF LFX Insights shows 1,088 total contributors / 217 organizations** — irreconcilable, almost certainly a different counting method. Use "~100–130 human contributors, Google + Solo.io led"; LFX number unverified. CNCF health score: 76.                                                                                                                                                                                                                                                                            |
| Maintainers/ecosystem      | Google is steward; Solo.io co-maintains (per secondary) and shipped **kagent 1.0** rebuilt on Substrate; NVIDIA contributing upstream. Weekly Thursday community meeting, CNCF Slack `#substrate-users`/`#substrate-dev`.                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Production users — careful | **Substrate-specific, primary-sourced:** Nous Research — "early design partner… building Hermes Enterprise on it" (Google; _building_, not claimed in production); **SPIRITT** — building SPIRITT Workspace on Agent Substrate on GKE, CEO quote about running customer agents (strongest production-adjacent claim); **kagent** — "already powers other CNCF projects like kagent" (Google). **Warning:** Anthropic, OpenAI, Modal, Snowflake, Ant Group, Tencent, Northflank are cited by Google as **gVisor** production users, _not_ Agent Substrate users. **No independent, named, direct-Substrate production deployment was verified.** |
| Docs quality               | Substantial for a 6-month project: full architecture concept doc, security guide + threat model, API/CLI references, demos, docs site (`ate.dev`) + learn site, benchmarking harness in-repo (`glutton` synthetic workload, `agentsession`/Locust scenario). **But:** fast churn leaves stale material (pre-1.0 READMEs still indexed by pkg.go.dev with different perf numbers; Redis-vs-Postgres contradiction in older docs; GKE limitations live in GKE docs, not launch posts). Docs are candid about trade-offs. No published external security audit found.                                                                              |

## 5. Performance numbers published

### Vendor/project claims (no independent verification, no published methodology in launch posts)

| Claim                                                                | Source / wording                                                                                                                                                                                                                                                |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Resume **<500 ms**                                                   | Current README + both Google blogs. Scope (per Qovery's reading): restore of memory+disk onto an _already-warm_ worker.                                                                                                                                         |
| **>500 activations/sec**                                             | Current README + Google Cloud Blog: "more than 500 suspend and resume activations per second." A "200,000 agents" figure in one secondary summary could not be located in primary posts — unverified.                                                           |
| **10× density** vs standard containers; **10×–30× oversubscription** | README; demo: ~250 actors on 8 pods.                                                                                                                                                                                                                            |
| Cold-start baseline beaten: **>20 s**                                | Google Open Source Blog: scale-to-zero rebuild "climbs past 20 seconds."                                                                                                                                                                                        |
| Idle premise: agents idle **90–95%** of the time                     | Google Open Source Blog. The entire density arithmetic depends on this.                                                                                                                                                                                         |
| **Conflicting older numbers**                                        | An older README snapshot still served by pkg.go.dev claims **<400 ms** resume and **>1,500 activations/sec**. Current `main` README says <500 ms / >500/sec. The project has _lowered_ (or re-scoped) its headline numbers at 1.0 — cite only the current pair. |
| North-star _targets_ (not results)                                   | Architecture doc: activation latency **100 ms p95**, throughput **1,000 wakeups/sec/cluster**, scale target **1 billion** actors/cluster — aspirations, explicitly labelled targets.                                                                            |

### Independent measurement (the only one found)

**Giuseppe Cofano, "From 20 to 120 agent sessions on three GKE nodes:
tuning Agent Substrate," Nov 2026** — v0.6.0, GKE 1.37.1 Standard, 3×
16-vCPU nodes, 45 worker slots, Substrate's own `agentsession` benchmark
(simulated coding agent, ~96 MiB RAM + ~110 MiB in-memory files, ~87%
idle, no real LLM calls). Key findings:

- **Snapshot ≈300 MB** per checkpoint for that agent. At ~1 suspend/sec/node
  that's **~300 MB/s sustained writes/node**, vs 0.6 MB/s for the same
  agents as always-on containers. **Node disk write throughput, not
  CPU/RAM, sets capacity.**
- Capacity ladder: default 100 GB pd-balanced (~176 MB/s) → **20
  sessions** (suspend p95 31 s at 40 sessions; collapse at 80);
  ~300–450 MB/s → 40; ≥750 MB/s → 80; 4× Local SSD → **120 sessions**.
  Yardstick (one GKE Sandbox gVisor pod/session, same nodes): **40
  sessions**. Independently measured density advantage was **~2–3× the
  pod-per-session baseline under this workload**, not 10×.
- **Wake latency:** local restores <1 s; cross-node restores 5–10 s (fast
  disks) / ~30 s p50 (saturated disk). At 120 sessions on 1,200 MiB/s
  Hyperdisk, median wake 0.52 s but p95 ≥16 s.
- Defaults quantified: `--snapshot-upload-delay` 5 min (durability
  window), `--snapshot-detach-delay` 30 min, router parked-request budget
  had to be raised 5 s → 20 s for the tests.

No other independent benchmark, and no benchmark from Google with
hardware/workload/methodology disclosed, was found. Qovery (18 Nov): "No
independent benchmark existed when I wrote this" — Cofano's post
post-dates that and partially fills the gap for v0.6.0 only.

## What this lane could NOT verify

1. Exact CNCF acceptance date (Sept 28 vs Oct 20 vs "last month").
2. v1.0.1 release contents (secondary trackers only) and the full
   12-release date list.
3. Contributor count (100+ / ~129 / LFX 1,088 mutually inconsistent).
4. Zstd as the snapshot codec from a _primary_ source.
5. Any Substrate-specific statement on randomness/entropy across restore,
   or snapshot encryption at rest.
6. GKE-docs limitations quoted verbatim from Google (reported consistently
   by ≥3 secondary sources, but the GKE doc pages were not fetched).
7. Any direct, named, independent production user of open-source
   Substrate.
8. The "200,000 agents" benchmark figure in secondary summaries.

## Sources

**Primary**

- Google Cloud Blog, "Open infrastructure for autonomous agents comes to
  CNCF" (2026-11-05):
  https://cloud.google.com/blog/products/containers-kubernetes/open-infrastructure-for-autonomous-agents-comes-to-cncf
- Google Open Source Blog, "Sandboxing Untrusted AI Agents at Scale"
  (2026-11-05):
  https://opensource.googleblog.com/2026/11/sandboxing-untrusted-ai-agents-at-scale.html
- GitHub repo: https://github.com/agent-substrate/substrate
- Architecture doc (`main`):
  https://github.com/agent-substrate/substrate/blob/main/docs/concepts/architecture.md
- v1.0.0 release: https://github.com/agent-substrate/substrate/releases/tag/v1.0.0
- CNCF project page: https://www.cncf.io/projects/agent-substrate/
- System topology: https://learn.agentsubstrate.dev/topology
- Google Cloud Blog, "Agent Substrate available on GKE" (Sept 2026):
  https://cloud.google.com/blog/products/containers-kubernetes/agent-substrate-available-on-gke
- gVisor checkpoint/restore docs:
  https://gvisor.dev/docs/user_guide/checkpoint_restore/
- GitHub issue #121 (CRIU status):
  https://github.com/agent-substrate/substrate/issues/121
- NVIDIA Technical Blog, "Agent Substrate: Advancing Kubernetes for
  Agents at Scale": https://developer.nvidia.com/blog/?p=124728

**Independent / secondary (labelled as such in text)**

- Cofano, independent load test (v0.6.0, GKE):
  https://medium.com/google-cloud/from-20-to-120-agent-sessions-on-three-gke-nodes-tuning-agent-substrate-9979589a9da2
- Qovery, primary-source roundup (2026-11-18):
  https://www.qovery.com/blog/agent-substrate-density-resume-own-kubernetes-cluster
- Rochette, tracked assessment:
  https://m.tomrochette.com/agents/sandboxing/agent-substrate/
- TechTimes, GKE limitations reporting:
  https://www.techtimes.com/articles/330163/20261106/google-donates-ai-agent-sandbox-linux-foundations-cncf-agent-substrate-lacks-gpu-passthrough.htm
