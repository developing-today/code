# Supplement 3 — Agent sandbox / agent dev-environment field survey

_Research lane note, as of late 2026 (sources checked Nov 2026, mostly
verified 2026-11-14). Web research only._

> **Method / caveats:** Most cross-vendor numbers come from secondary
> aggregators that state they verified vendor docs/pricing on 2026-11-14
> (`agentsandboxes.org`, `sandbox.watch`, `seminal.ai` checked 2026-09-06)
> plus vendor pricing pages captured in search. Vendor cold-start claims
> are **not** independent measurements unless labelled as such. One
> independent burst benchmark cited in multiple sources: ComputeSDK,
> 2026-08-21, 100 concurrent launches from N. Virginia, create→first
> command median/p95: Vercel 0.67s/1.04s, Modal 0.88s/1.00s, Runloop
> 0.89s/3.27s, E2B 1.61s/1.77s — Daytona excluded / capacity-limited in
> that run. This contradicts most vendor headline figures. **Flagged =
> could not source from a primary vendor page in this pass.**

## 1. E2B — e2b.dev

- **Isolation / state:** Firecracker microVM, own kernel per sandbox.
  Pause/resume preserves FS + memory/processes; snapshots of running
  sandboxes, FS-only snapshots, forking from snapshot (up to 20/call per
  Oct 2026 secondary source — flagged: secondary only).
- **Cold start:** Vendor-associated ~150 ms from snapshot. Independent
  burst median 1.61s. Pause ~4s/GiB RAM, resume ~1s (secondary buyer guide).
- **Persistence:** Ephemeral by default; pause is the persistence
  mechanism, volumes (beta). Session cap 1h Hobby / 24h Pro, clock resets
  on resume; paused kept until deleted.
- **Self-hostable:** **Yes / partial-enterprise.** Apache-2.0 SDKs +
  runtime; single-node "E2B Embed" (since 2026-09-28); BYOC on
  AWS/GCP/Azure is Enterprise. Self-hosting is Terraform/Nomad/forked-
  Firecracker work, not `docker compose`.
- **Pricing:** Per-second: $0.000014/vCPU-s ($0.0504/vCPU-hr) +
  $0.0000045/GiB-s ($0.0162/GiB-hr). Hobby free + $100 one-time credit, 20
  concurrent; Pro $150/mo, 100 concurrent (expandable to 1,100), 24h
  sessions; Enterprise from ~$2,000-3,000/mo commit (sources differ —
  flagged range).
- **Agent features:** Python + JS/TS SDKs, Code Interpreter SDK, Desktop
  sandbox, templates (custom images), preview hosts per port, MCP Gateway
  (200+ MCP servers in-sandbox per secondary).
- **Maturity:** $21M Series A Jul 2025, Insight Partners, $32M total
  disclosed; claims 88% of Fortune 100, 200M+/1B+ sandboxes started
  (vendor claim, counts differ by source date — flagged), 10M+ monthly
  SDK downloads (secondary).

## 2. Modal — modal.com (Sandboxes)

- **Isolation / state:** gVisor by default, or VM runtime with own kernel
  (Modal chooses if unset); GPU sandboxes gVisor-only. FS snapshots,
  directory snapshots, memory snapshots (alpha, incl. GPU memory snapshot
  alpha), Volumes (persistent distributed FS).
- **Cold start:** Vendor: "containers boot in about one second" /
  sub-second incl. large images; typical 2-5s with imports in older
  guides; memory snapshot reduces large-dep starts. Independent burst
  median 0.88s.
- **Persistence:** Sandboxes ephemeral, 5 min default, 24h max; state
  beyond that via snapshots / Volumes.
- **Self-hostable:** **No.** Proprietary, managed only.
- **Pricing:** Sandbox rate $0.00003942/physical-core-s (1 core = 2 vCPU)
  - $0.00000667/GiB-s — ~3× the standard Function CPU rate quoted in many
    comparisons. Starter $0 + $30/mo credits, 100 containers; Team $250/mo
  - $100 credits, 5,000 containers. GPUs per-second: T4 ~$0.59/hr, H100
    ~$3.95/hr, up to B200/B300.
- **Agent features:** Python primary, JS/TS + Go SDKs; Sandboxes API with
  tunnels, secrets, images defined in Python code; tested scheduler claim
  > 1M concurrent sandboxes (vendor).
- **Maturity:** Largest funded here. Secondary sources cite a $355M
  Series C at $4.65B (2026-05-21) and a further $750M round Oct 2026 —
  flagged: conflicting tracker data, verify before quoting valuation.

## 3. Daytona — daytona.io

- **Isolation / state:** Linux containers by default (namespaces,
  resource limits); Linux VM / Windows sandboxes are full VMs with own
  kernel — only VM tier gets pause/resume/fork/hot snapshots. Also macOS,
  GPU sandboxes.
- **Cold start:** Vendor headline **sub-90ms** creation. Independent
  breakdown cited: 71ms creation (warm pool) + 67ms exec, 150-200ms
  realistic cold.
- **Persistence:** Persistent by default — workspace lives until deleted;
  auto-stop 15 min idle default (containers), VM auto-pause 60 min idle,
  stopped containers archived after 7 days. Stateful snapshots,
  declarative images, volumes.
- **Self-hostable:** **Partial / changed.** AGPL-3.0 repo public at
  v0.190.0 but README states core development moved to a private codebase
  June 2026, repo no longer maintained. BYOC: attach own runners as
  custom region (control plane stays Daytona); dedicated regions available.
- **Pricing:** Per-second, no platform fee: $0.0504/vCPU-hr,
  $0.0162/GiB-hr, storage $0.000108/GiB-hr after 5 GiB free; Windows
  +$0.0858/vCPU-hr; GPUs $0.57/hr (RTX 4090) to ~$3.44-3.59/hr
  (B200/MI355X — sources differ). $200 free compute.
- **Agent features:** SDKs Python, TS, Ruby, Go, Java; Git + LSP APIs,
  Computer Use, preview URLs, MCP server, CLI.
- **Maturity:** $24M Series A Feb 2026 FirstMark-led; Series B $60M
  reported Oct 2026 in tracker data — flagged: tracker-only.

## 4. Fly.io Sprites — fly.io/sprites

- **Isolation / state:** Firecracker microVM, own kernel, persistent ext4
  FS. Checkpoints = copy-on-write FS snapshots (~1s, restorable, forkable
  into new Sprites); memory preserved only on warm suspend, lost on cold
  stop — long-lived processes must be registered as "Services" to
  auto-restart.
- **Cold/warm:** Vendor docs: warm wake 100-500ms with processes intact;
  cold wake 1-2s. Sleeps after ~30s idle, wakes on request.
- **Persistence:** Strongest persistence story in list: 100 GB durable
  disk per Sprite, **no lifetime limit**, persists until destroyed.
- **Self-hostable:** **No.**
- **Pricing:** Billed on **actual use**, per-second, no compute while
  idle: ~$0.0385/CPU-hr + $0.021875/GB-hr memory actually used; cold
  storage $0.02/GB-month while sleeping. Creation rate is the constraint:
  10/min PAYG, up to 240/min top tier.
- **Agent features:** SDKs JS/TS, Go, Python, Elixir + CLI/REST;
  public/private HTTPS URL per Sprite; no OCI/Docker images — boots
  standard base image, install onto FS.
- **Maturity:** Launched Jan 2026 on mature Fly.io Machines platform.

## 5. Vercel Sandbox — vercel.com

- **Isolation / state:** Firecracker microVM, dedicated kernel,
  root/system privileges (Docker-in-sandbox, FUSE work).
- **Cold start:** Vendor: "start in milliseconds" — **no published
  figure**. Independent burst median 0.67s (fastest in that run).
  Snapshot resume faster than fresh.
- **Persistence:** Persistent by default: FS auto-snapshotted on stop,
  restored on resume by name; manual snapshots, 30-day default retention;
  Drives (beta) + S3 mounts. Background processes do **not** survive
  stop. Session 45 min Hobby / 24h Pro, unbounded via persistent resume.
- **Self-hostable:** **No.**
- **Pricing (iad1):** Active CPU $0.128/hr (only while CPU used) +
  provisioned memory $0.0212/GB-hr + $0.60/1M creations + $0.08/GB-mo
  snapshots + $0.15/GB egress. Hobby free tier (5 active-CPU-hr, 5,000
  creations/mo, 10 concurrent); Pro $20/mo credit, up to 10,000
  concurrent per secondary.
- **Agent features:** JS/TS + Python SDKs + CLI; up to 15 exposed ports,
  credential brokering, fine-grained egress (deny-all configurable), 19
  regions.
- **Maturity:** GA 2026-01-30.

## 6. Cloudflare Sandbox SDK / Containers

- **Isolation / state:** Each Container instance is a microVM with own
  kernel/network (hypervisor not named by Cloudflare); paired with a
  Durable Object per sandbox. JS-only alternative: Dynamic Workers (V8
  isolates).
- **Cold start:** Vendor docs: container cold starts "often 1-3 second
  range" depending on image. A <50ms figure in one April 2026 bake-off
  blog appears to conflate Worker/Durable-Object start with container
  start — **do not quote <50ms for Containers**.
- **Persistence:** Ephemeral disk by default; container snapshots (up to
  20 GB, 30 days), directory backups to R2, S3/R2 mounts, Durable Object
  storage. Up to 6h inactivity timeout via DO.
- **Self-hostable:** **No.** SDK is open source (Apache-2.0) but runs
  only on Cloudflare.
- **Pricing:** Requires Workers Paid $5/mo. Containers billed in 10ms
  increments: $0.000020/vCPU-s active ($0.072/vCPU-hr) +
  $0.0000025/GiB-s ($0.009/GiB-hr) + disk. Included: 25 GiB-hr memory,
  375 vCPU-min, 200 GB-hr disk/mo. Max instance in sources: 4 vCPU /
  12 GiB.
- **Agent features:** TypeScript (Workers) SDK only — no Python SDK
  documented; Sandbox SDK 1.0 current; egress host allow/deny +
  credential injection (2026); Agents SDK / Durable Objects for per-agent
  state + hibernation (hibernating DO not billed for duration).
- **Maturity:** Containers + Sandboxes GA 2026-04-13.

## 7. Runloop — runloop.ai — ⚠️ WINDING DOWN

- **Isolation / state:** Dedicated microVMs, full root, own kernels.
- **Cold start:** <2s for 10 GB images (vendor); sub-second resume from
  pre-warmed snapshot pools (secondary). Independent burst median 0.89s /
  p95 3.27s.
- **Persistence:** Disk snapshots + suspend/resume (**disk only, memory
  not kept**; daemons must be restarted manually). Blueprints = pre-built
  disk templates. Devboxes ephemeral: 1h default keep-alive, 48h max.
- **Self-hostable:** **Partial:** VPC deployment on AWS/GCP/Azure on
  Enterprise; no self-host product.
- **Pricing:** Basic free + usage, $50 credit; Pro $250/mo + usage;
  $0.108/CPU-hr + $0.0252/GB-hr, per-second; storage $0.00034236/GB-hr.
- **Agent features:** Python + TS SDKs, Repo Connect (GitHub), credential
  gateway, Axons event streams, built-in benchmarks (SWE-Bench Verified,
  SWE-Smith, R2E-Gym). SDK 3.0.0 (2026-10-23) removed Agents API / named
  shells that docs still describe — flagged: docs/API drift.
- **Maturity:** Founded 2024, $7M seed. **Critical: winding down, support
  for existing customers through 2026-11-13, transition guides point to
  Tensorlake / Modal / Freestyle** (verified 2026-09-30).

## 8. Morph (Morph Cloud) — morphllm / cloud.morph.so

- **Isolation / state:** Full VMs, hypervisor not named. Core primitive
  **Infinibranch**: snapshot a _running_ VM — memory + processes + disk —
  and branch many copies from that exact state.
- **Cold start:** Vendor: snapshot/branch/restore **<250ms**. Fresh-boot
  figure not published — flagged.
- **Persistence:** Snapshots (immutable, bootable, optional TTL),
  pause/resume preserving memory (paused bills storage only), volumes
  (S3-compatible), wake-on-request, TTL lifecycle.
- **Self-hostable:** **Not documented / No** in aggregator profiles.
- **Pricing:** Morph Compute Units: $0.05/MCU, 1 MCU = 1 vCPU-hr + 4 GB
  RAM-hr + 16 GB disk-hr. Developer $0/mo, Team $40/mo incl. 1,000 MCUs,
  Scale $250/mo incl. 7,500 MCUs. No GPU documented.
- **Agent features:** Python + TS SDKs/CLI/REST; Devboxes (VS Code/Cursor
  over SSH, tmux), Morph Actions (GH Actions runners), VNC/noVNC.
- **Maturity:** Specialist for branching workloads (search, RL, evals).
  Funding/users not sourced in this pass — flagged.

## 9. CodeSandbox → Together Code Sandbox

- **Isolation / state:** Firecracker microVMs. Memory + FS checkpointing,
  hibernation, live cloning/forking.
- **Cold start:** Vendor claims: 2.7s P95 cold from template; ~500ms P95
  start/resume from memory snapshot; fork/clone <1s.
- **Persistence:** Hibernation snapshots disk+memory, resumes exact
  state, hibernated sandboxes forkable; sessions run until terminated
  unless TTL set.
- **Self-hostable:** **No.** Client SDK MIT.
- **Pricing:** ⚠️ **Two shapes in sources, reconcile before budgeting:**
  (a) legacy CodeSandbox VM credits $0.01486/credit, per-minute billing;
  tiers Build free + $100 credit, Pro $150/mo, Scale $170/mo. (b) Together
  pricing restated as $0.0446/vCPU-hr + $0.0149/GiB-hr.
- **Agent features:** TS/JS SDK (`@codesandbox/sdk`, now deprecated in
  Together docs) + new `together-sandbox` Python/TS SDK; preview URLs,
  file watch, any Dockerfile template; 2-64 vCPU / 1-128 GB.
- **Maturity:** CodeSandbox SDK GA May 2025; CodeSandbox acquired by
  Together AI Dec 2024, relaunched as Together Code Sandbox.

## 10. Ona (formerly Gitpod) — ona.com

- **Isolation / state:** Ephemeral, production-like environments from
  `devcontainer.json`; environment is the unit, not a low-level sandbox
  API. Isolation tech not published in sources found — flagged.
- **Cold start:** **No published figure sourced — flagged.**
- **Persistence:** Ephemeral environments by design; agent sessions +
  parallel environments (Free: up to 3 parallel, up to 4 vCPU/16 GB/80 GB
  disk).
- **Self-hostable:** **Partial, changed:** community self-hosting is
  **gone** — old AGPL-3.0 `gitpod-io/gitpod` repo deprecated by its own
  README. Current "self-host" = vendor-operated single-tenant install in
  your AWS or GCP account (Enterprise).
- **Pricing:** Free plan; Core from $20/mo incl. 80-2,200 Ona Compute
  Units/mo depending on tier; unit→compute mapping not published on
  pricing page (secondary criticism); Enterprise quote-only.
- **Agent features:** Repositioned Sep 2025 from cloud dev environments
  to autonomous coding-agent orchestration/control plane: agent sessions,
  BYOC, devcontainer standard.
- **Maturity:** Rebrand Gitpod→Ona Sep 2025; Gitpod Classic PAYG switched
  off 2025-10-15. Announced joining OpenAI Jun 2026, acquisition closed
  2026-08-10, now inside Codex team (secondary, checked 2026-11-20).

## 11. Blaxel — blaxel.ai — acquired by Baseten

- **Isolation / state:** Hardware-isolated microVM per sandbox, own
  kernel, root FS held in memory; hypervisor not named.
- **Cold/warm:** Vendor: resume from standby **<25ms** with memory
  intact; new sandboxes <1s. Standby auto after ~5-15s idle.
- **Persistence:** "Perpetual" model: standby snapshots
  memory+processes+FS, indefinite standby at $0 compute, forkable
  snapshots, volumes, shared Agent Drive (distributed FS, beta).
- **Self-hostable:** **Partial:** no self-host documented; BYOC /
  bring-your-own-metal into own VPC offered (Enterprise-flavoured).
- **Pricing:** No base subscription. Active sandbox $0.0000115/GB RAM-s
  (~$0.041/GB-hr, CPU scales with allocated memory); standby snapshot
  storage $0.20/GB-mo; images $0.045/GB-mo; volumes $0.12/GB-mo. Batch
  jobs $0.000006/GB-s, MCP hosting $0.000007/GB-s.
- **Agent features:** TS/Python/Go SDKs; colocated serverless agent
  hosting, MCP server hosting, batch jobs, model gateway,
  secret-injecting egress proxy; first-class provider in OpenAI Agents
  SDK. Claims 50k+ concurrent per workspace (vendor).
- **Maturity:** $7.3M seed (First Round / YC S25); claimed 7.5M+ agent
  requests/day; **acquired by Baseten, announced 2026-09-10, platform
  continues; Baseten launched its own Sandboxes 2026-11-05 on Blaxel
  primitives.**

## 12. Northflank — northflank.com

- **Isolation / state:** CPU sandboxes: microVMs via **Kata Containers +
  Cloud Hypervisor** primary; Firecracker and gVisor where supported.
  GPU sandboxes: gVisor. Not a sandbox-only product — full app platform
  (services, jobs, DBs, preview envs) on same control plane.
- **Cold start:** Vendor: "sub-second" boot on Northflank Cloud — **no
  precise figure published**.
- **Persistence:** Persistent volumes/files across sessions; lifecycle
  create→run→scale-to-zero→destroy; **no fixed session time limit**.
- **Self-hostable:** **Partial / BYOC (strongest BYOC story here):**
  self-service BYOC into AWS, GCP, Azure, Oracle, CoreWeave, Civo,
  Nebius, on-prem/bare-metal; Northflank runs control plane, data stays
  in your VPC. Not a download-and-self-host open-source product.
- **Pricing:** Cheapest list CPU in this set: $0.01667/vCPU-hr +
  $0.00833/GB-hr, per-second, no seat fees PAYG; H100 $2.74/hr
  all-inclusive. Free Sandbox tier: 2 services, 1 DB, 2 cron jobs,
  always-on (no sleep).
- **Agent features:** Any OCI image unmodified, thousands concurrent
  sessions (vendor), CPU+GPU, egress/ingress network policies incl.
  deny-all on BYOC, SOC 2 Type II / HIPAA.
- **Snapshot model:** ⚠️ **Flag — no snapshot/pause/fork semantics
  sourced in this pass**; persistence is volume/service-based in sources
  found.
- **Maturity:** Production workloads since 2019, microVM sandbox
  orchestration since 2021 (vendor).

## 13. microsandbox — github.com/superradcompany/microsandbox

- **Isolation / state:** Hardware-virtualised microVMs on **libkrun**;
  each sandbox own kernel; runs OCI images. Requires KVM (Linux), Apple
  Silicon/HVF (macOS), or Windows Hypervisor Platform.
- **Cold start:** Aggregator table lists <100ms — flagged: no
  methodology/vendor primary source in this pass.
- **Persistence:** Snapshots supported (aggregator); detailed pause/fork
  semantics not documented in sources found — flagged.
- **Self-hostable:** **Yes — the point of it.** Apache-2.0, local-first,
  self-host free. Managed cloud optional, private beta.
- **Pricing:** Self-host $0. Cloud (private beta): Builder $20/mo, Teams
  $200/mo (incl. equal usage credit); usage $0.048/vCPU-hr +
  $0.003/GiB-hr.
- **Agent features:** SDKs TS, Python, Rust, Go, Ruby + CLI + **MCP
  server**; per-secret host allowlisting shipped.
- **Maturity:** Beta-labelled, small/new vs hosted vendors; no funding
  sourced — flagged.

## Isolation layer — brief notes

| Primitive                       | What it isolates                                                                                                                                         | Overhead shape                                                                                                                            | Notes                                                                                                                                                                 |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Firecracker** (AWS, Rust VMM) | Hardware virtualisation via KVM: own guest kernel per microVM; minimal virtio device model; VMM process itself jailed                                    | ~125ms boot, <5 MiB VMM overhead/VM, up to 150 microVMs/s/host; snapshot restore ~125ms (AWS/vendor figures)                              | A building block, not a platform: you supply kernel, rootfs, lifecycle, networking. No GPU/VFIO passthrough. Powers Lambda/Fargate, E2B, Vercel, Sprites, CodeSandbox |
| **gVisor** (Google, Go)         | No VM: user-space kernel (`Sentry`, `runsc` OCI runtime) intercepts app syscalls; host sees only ~50 vetted syscalls from Sentry                         | Start <200ms on GKE (secondary); **syscall/I/O tax is the cost** — 10-30% on I/O-heavy workloads in secondary sources, minimal on compute | Runs without KVM/nested virt; incomplete syscall coverage (~290/352 amd64 syscalls); GPU via `nvproxy`. Powers Modal, GKE Sandbox, Google Agent Sandbox               |
| **Kata Containers**             | OCI runtime that boots a real VM **per pod**: container UX, guest kernel per pod; hypervisor pluggable — QEMU, Cloud Hypervisor, Firecracker, Dragonball | Heaviest of the three: ~1-2s start, fixed ~250 mCPU + 130 MiB per pod (GKE figures, secondary)                                            | Best K8s fit (`RuntimeClass`); near-full Linux compatibility; GPU + confidential computing (TDX/SEV-SNP) via QEMU                                                     |

Rule of thumb from sources: boundary choice is VM-with-own-kernel (Kata /
raw Firecracker) vs user-space kernel (gVisor); plain runc containers
(shared kernel) are not considered a security boundary for untrusted
agent code in 2026 consensus sources, and egress control is repeatedly
rated higher-value than the isolation tier alone.

## Other credible entrants (one line each)

- **BoxLite** — Apache-2.0 self-hostable KVM/HVF microVM runtime + BoxLite
  Cloud; closest microsandbox analogue.
- **Baseten Sandboxes** — launched 2026-11-05 on acquired Blaxel
  primitives, standby/resume, $0.0655/vCPU-hr + $0.0211/GB-hr + $0.20/GB-mo
  snapshot storage.
- **Novita Agent Sandbox** — GPU-cloud entrant: per-second stateful
  sandboxes, pause/resume/snapshots/templates, ~$0.035/vCPU-hr equivalent
  rates, 60 GB persistent storage free.
- **AWS Bedrock AgentCore + Lambda MicroVMs** — AgentCore Code
  Interpreter $0.0895/vCPU-hr + $0.00945/GB-hr; Lambda MicroVMs =
  Firecracker restored from snapshot with suspend/resume.
- **Google / Kubernetes Agent Sandbox** — K8s-native, gVisor/Kata
  pluggable `RuntimeClass`, GKE Pod Snapshots, sub-second startup claim;
  self-host (K8s) shape. _(Distinct from Agent Substrate — see
  supplement 1.)_
- **Azure Container Apps dynamic sessions** — Hyper-V-isolated sessions;
  model-API code-interpreter shape rather than general sandbox SDK.
- **Tensorlake Sandboxes** — microVMs with memory snapshots,
  copy-on-the-fly cloning, warm pools, versioned FS + hosted Git,
  Apache-2.0 runtime, BYOC on AWS; named Runloop migration target.
- **Freestyle** — full-VM/KVM sandboxes with memory-surviving pause; also
  a Runloop migration target. _(See supplement 2.)_
- **Upstash Box** — hardened-container (not microVM) persistent per-user
  agent computer with built-in coding agent, git-to-PR, browser, cron,
  MCP; active-CPU billing.
- **Prime Intellect Sandboxes** — RL-focused; launch rates $0.02/vCPU-hr
  - $0.0125/GiB-hr + disk (valid through 2026-12-22 per Fly comparison).
- **Beam, Koyeb, Namespace, HopX, Leap0, Cube Sandbox (Tencent,
  E2B-compatible API, Apache-2.0), Docker Sandboxes, Smithery** — Beam =
  gVisor, BYOC, GPU; Koyeb/Namespace = microVM compute platforms;
  HopX/Leap0 = Firecracker newcomers with desktop/VNC +
  credential-injecting firewalls; Cube = self-host E2B-API clone, ~13k
  stars (secondary); Docker = local+cloud microVM agent sandboxes,
  experimental; **Smithery is not a sandbox — it's an MCP
  registry/gateway, adjacent tooling only.**

## Sources

- https://agentsandboxes.org/sandboxes/ (per-system pages: e2b, modal,
  vercel-sandbox, cloudflare-sandbox, fly-sprites, morph, codesandbox,
  blaxel, microsandbox) and
  https://agentsandboxes.org/guides/agent-sandbox-pricing/
- https://seminal.ai/reference/categories/agent-sandboxes-and-code-execution/
- https://fly.io/learn/sprites-vs-e2b-vs-modal-vs-daytona/ ;
  https://fly.io/learn/rl-sandbox-pricing/
- https://sandbox.watch/p/fly-io-sprites ; /p/morph ; /p/codesandbox
- https://blaxel.ai/pricing ; https://northflank.com/blog/northflank-vs-daytona ;
  https://runloop.ai/pricing ; https://www.anchorterminal.com/tools/runloop
- https://www.softwr.com/software/gitpod ;
  https://upstash.com/blog/ai-agent-sandbox-providers-compared-2026
- Full URL list in the lane's working notes; aggregators above are the
  load-bearing secondary sources, each stating its own verification date.

**Numbers explicitly NOT sourced / do not quote without
re-verification:** Ona cold-start (none published found); Morph funding &
fresh-boot time; microsandbox <100ms; Northflank snapshot semantics &
precise boot time; Modal 2026 valuation rounds (tracker-conflicting);
Daytona Series B $60M (tracker-only); CodeSandbox credit vs per-vCPU
pricing reconciliation; Sprites $0.0385 vs $0.03825 CPU rate; E2B
Enterprise floor ($2k vs $3k/mo).
