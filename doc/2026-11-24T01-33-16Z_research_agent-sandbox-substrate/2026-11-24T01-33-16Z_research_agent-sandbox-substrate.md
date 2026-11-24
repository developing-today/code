# Agent Substrate and the agent-sandbox field — full analysis

_Lane AGENT-SANDBOX-SURVEY, 2026-11-23. Companion research to the CNCF
landscape investigation (`doc/` research records, platform & GitOps and
runtimes lanes). Drew's questions, verbatim in substance: tell me more about
Agent Substrate and comparable alternatives; what does it mean to decouple
agent state from a warm pool of sandbox workers with snapshot/restore; how
does it compare to boat.dev, "chai systems", Boxer.dev, Freestyle.sh; are
there other competitors to consider?_

_Method: three parallel research lanes (Agent Substrate primary sources;
the four named products; the wider field), cross-checked against Google's
announcement posts and the project's own architecture doc. Vendor numbers
are labelled vendor claims throughout. Full lane notes are supplements in
this folder._

---

## Part 1 — Agent Substrate, deep

### What it is

Agent Substrate is a Google-originated, Apache-2.0, Kubernetes-native
runtime for running large numbers of stateful, mostly-idle, untrusted agent
sessions at high density. Repo: `github.com/agent-substrate/substrate`
(Go; docs at ate.dev). Public development began 13 May 2026. It reached
**v1.0.0 on 5 November 2026**, the same day Google announced its donation —
alongside gVisor and OpenRL — to the **CNCF Sandbox** (application
cncf/sandbox#523; CNCF's project page dates acceptance 28 September 2026,
secondary trackers put the application close at 20 October — the safe
statement is "accepted Sept–Oct 2026, donation announced 5 Nov 2026").
A managed form, **Agent Substrate on GKE**, went GA the same day. It is
_not_ an agent SDK, and it is _not_ the separate
`kubernetes-sigs/agent-sandbox` project (Sandbox CRD + warm pools, no
memory multiplexing) — secondary coverage conflates them constantly.

Maturity in one paragraph: roughly 8,200 GitHub stars and ~100–130 human
contributors (Google + Solo.io led) at research time; 12 releases in six
months; a real architecture doc, security guide and threat model; an
in-repo benchmark harness. Counter-signals: pre-1.0 minors were
reinstall-only with no upgrade path; older docs still circulating
contradict current ones (Redis vs PostgreSQL state store; older, better
headline numbers — see below); no external security audit found; **no
independent, named, direct production deployment verified** — Nous
Research and SPIRITT are "building on it" per Google, and kagent (Solo.io)
runs on it. Anthropic/OpenAI/Modal/Snowflake are _gVisor_ users, not
Substrate users, despite secondary articles misattributing them.

### Architecture in brief

Kubernetes is deliberately kept **off the hot path**: K8s provisions and
heals a pool of long-running **Worker Pods** ahead of time; Substrate's own
control plane (an API server backed by PostgreSQL, plus a controller that
reconciles `WorkerPool` CRDs into Deployments) assigns sessions to
already-running workers in milliseconds, because per-activation Pod
creation and etcd churn are exactly what doesn't scale to millions of
mostly-idle sessions. Only the capacity side is Kubernetes-native
(`WorkerPool`, `SandboxConfig`, `CSIDriverConfig` CRDs); sessions
("Actors"), templates and assignments are records in Substrate's own
store, managed via a `kubectl ate` plugin.

Each Actor runs in its own sandbox inside a shared Worker Pod, isolated by
**gVisor (`runsc`)** or by a **Kata + Cloud Hypervisor microVM** — plain
containers are explicitly not supported, and a checkpoint taken under one
class cannot restore on the other. Networking runs through gateways: an
ingress router that parks requests for suspended Actors, and an egress
gateway that enforces destination policy and **injects credentials outside
the sandbox** — Actors hold a SPIFFE identity but no secrets. Only
HTTP(S) ingress is supported today; arbitrary raw-TCP ports are a stated
limitation.

### The decoupling, in plain words (the section Drew asked for)

An agent session spends most of its life _waiting_: it runs code for a few
seconds, then pauses for a model turn, a tool call, or a human. Google's
own figure is that agents are idle 90–95% of the time. The conventional
setup gives every session its own container or Pod for its whole lifetime —
so you pay full memory and CPU for thousands of sessions that are, at any
given moment, almost all asleep.

Substrate splits the session into two things that are usually fused:

- **The Actor** — everything that makes the session _itself_: its running
  processes and RAM, its files, its environment, and its identity. This is
  "agent state."
- **The Worker** — an anonymous, reusable sandbox (a warm Pod with a
  gVisor or microVM runtime inside) that merely _hosts_ an Actor while the
  Actor is awake. Workers hold no identity and no state worth keeping.

**The warm pool** is a set of Workers kept running and empty, so that
waking a session never pays container startup, image pulls, or Kubernetes
scheduling. Think hotel rooms kept made-up, not houses built per guest.

**Snapshot/restore** is the mechanism that lets the two halves separate.
When an Actor suspends, the runtime freezes its process tree and writes
its memory and filesystem to storage (gVisor's native checkpoint/restore
on the default class — _not_ CRIU; VM memory snapshots with demand paging
on the microVM class). The Worker is wiped and returned to the pool for
someone else. When the next request for that Actor arrives, the router
holds the request, the control plane claims any free Worker, the snapshot
is loaded into it, and the session continues where it left off — possibly
on a different machine than before. Google claims restore in **under
500 ms** and **more than 500 suspend/resume activations per second**, with
10× the density of one-Pod-per-agent.

Fidelity is tiered per template: `VOLUMES` (durable directories only),
`ROOTFS` (plus files written to the container filesystem; restore is a
cold boot with files intact), and `MEMORY` (the default: plus processes
and RAM — the full "it never stopped" illusion).

**What breaks or degrades — the honest list:**

- **Open connections die.** Database sessions, MCP connections, any live
  TCP: not preserved. Code must reconnect. Listening sockets are
  recreated; in-flight connections are reset.
- **The fast numbers are the local, warm case.** The one independent load
  test found (v0.6.0, GKE, simulated coding agents) measured local
  restores under 1 s, but cross-node restores of 5–10 s on fast disks and
  ~30 s p50 on a saturated disk — because a snapshot that isn't already on
  the node must be uploaded and downloaded in series while the request
  waits. Measured density advantage was **~2–3× the pod-per-session
  baseline**, not 10×, and the binding constraint was **node disk write
  throughput** (~300 MB of snapshot writes per suspend for a ~100 MB-RAM
  agent), not CPU or RAM.
- **There is a durability window.** By default the snapshot is written to
  node-local disk first and uploaded to object storage in the background
  (default delay ~5 minutes). If that node dies in the window, the state
  is gone. A synchronous mode closes the window at latency cost.
- **Clocks behave like a laptop lid closing** (per the inherited gVisor
  semantics; Substrate itself documents nothing here): monotonic clocks
  pause during suspension, wall clocks jump forward on restore, and
  timers that fell due while suspended fire on wake.
- **Snapshots are version-locked.** A memory snapshot restores only on a
  worker offering the same sandbox class and runtime version; otherwise
  it silently degrades to a ROOTFS cold boot — files survive, memory
  doesn't.
- **GPU doesn't snapshot.** On the GKE path, GPU passthrough into Actors
  isn't supported and live CUDA contexts can't be checkpointed; inference
  runs outside the sandbox.
- **Entropy is undocumented.** No statement was found anywhere on
  randomness reseeding across restores or golden-snapshot clones. For a
  snapshot/restore system this is a genuine open question, not a detail.

The one-sentence version: **Substrate turns an agent session from a
_place_ (a container that must stay alive) into a _file_ (a snapshot that
can sleep in storage and wake on any warm worker) — and charges you disk
bandwidth, reconnect logic, and a small durability window for the
privilege.**

---

## Part 2 — The four names Drew dictated

| Name as dictated | Verdict                                                                                                      |
| ---------------- | ------------------------------------------------------------------------------------------------------------ |
| Freestyle.sh     | **Identified, real, exactly in this space.**                                                                 |
| boat.dev         | **Identified, literal — not a misdictation.**                                                                |
| "chai systems"   | **Not pinned in this space.**                                                                                |
| Boxer.dev        | **Identified literally, but the domain is probably not the sandbox product; two namesakes untangled below.** |

**Freestyle** (freestyle.sh) — hosted full Linux VMs for AI agents: root,
SSH, systemd, nested virtualisation, default-deny networking, on its own
bare metal. YC Summer 2024, ex-Apple founders, pivoted twice to land on
agent VMs (Launch HN April 2026). Its differentiator is state handling:
memory-preserving suspend/resume (vendor claim <100 ms resume), live
forking of a running VM _including memory_, immutable snapshot layers.
Per-second usage pricing ($0.04032/vCPU-hr, $0.0129/GiB-hr RAM) over
monthly minimums ($0 free tier / $50 / $500 / enterprise). Not
self-hostable; proprietary hosted service, very small team.

**boat** (boat.dev) — cloud sandboxes for AI agents: full Ubuntu VMs with
own kernel, root, Docker, /dev/kvm, a public URL per port, and a
streamable desktop; seven built-in agent harnesses (Claude Code, Codex,
OpenCode, Pi, and others) behind one API. Paris-based, 3 people, YC Fall 2026. **It was previously ASCII's "Box" (box.ascii.dev), renamed Box →
boat in September 2026** — which matters for the Boxer question below.
Disk (not memory) snapshots every minute while running and at stop;
stopped disks kept ≥14 days; resume is effectively a reboot unless within
~10 minutes on the same machine. EU-only regions, strikingly cheap
($0.036/h for 4 vCPU/8 GB). Not self-hostable.

**"chai systems"** — no product by that name exists in the agent-sandbox
space that research could find. The literal namesakes are all something
else: chai-systems.dev is a one-founder "personal/founder operating
system" (agent-adjacent philosophy, no infrastructure product);
chai-systems.ai is an AI consultancy; CHAI by CloudHedge is enterprise
app-modernisation. The closest _spoken_ in-space match is **shai**
(shai.run) — an open-source CLI that sandboxes coding agents in Docker
with restrictive resource sets. If Drew saw "CHAI Systems" written, it's
the founder-OS company and doesn't belong in a sandbox comparison; if he
heard it spoken in a sandbox discussion, shai is the plausible intent.
Filed honestly as **not pinned**.

**Boxer** — three different things share the name. (a) **boxer.dev**
itself is Dan Phillips' open-source Wasm compute project (containers →
WebAssembly "Boxes", with the Weave live-migration/snapshot work) —
sandbox-adjacent research, pre-release, not a hosted agent environment.
(b) **BarakChamo/boxer** is the in-space product: parallel coding agents,
one sandbox per git worktree (microVM or Docker), local/self-hosted,
Apache-2.0 — if the mention was about parallel local coding agents, this
is the fit; it does not own boxer.dev. (c) **BoxLite** (boxlite-ai) is a
separate, real, self-hostable _embeddable_ microVM runtime for agents —
a library, not a service — and arguably the most interesting of the three
for us (Part 3). Near-names also found: Boxes.dev (YC F26, per-agent
cloud computers on E2B) and DevBoxer (hosted background coding agents).

---

## Part 3 — The wider field

Headline: **the field has converged on one architecture with many
prices.** Nearly every serious entrant is Firecracker or gVisor (or Kata)
underneath, per-second billing, snapshot-based pause/resume, and an SDK.
Differentiation has moved to: persistence defaults (ephemeral vs
keep-my-disk-forever), forking/branching as a first-class operation,
self-hostability, and who is still alive — **Runloop is winding down**
(support ended 2026-11-13 per its transition guidance) and **Blaxel was
acquired by Baseten** (Sept 2026), which relaunched sandboxes on its
primitives in November. An independent burst benchmark (ComputeSDK,
Aug 2026, 100 concurrent launches) put create→first-command medians at
Vercel 0.67 s, Modal 0.88 s, Runloop 0.89 s, E2B 1.61 s — a useful
antidote to every vendor's headline number, Daytona's "sub-90 ms"
included.

| System                            | Isolation                                  | State / snapshot model                                                 | Self-host                                                        | Pricing shape                                         | One-line verdict                                                                         |
| --------------------------------- | ------------------------------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| **E2B**                           | Firecracker                                | Pause/resume keeps FS+memory; FS snapshots; fork from snapshot         | Yes (Apache-2.0; real work; BYOC enterprise)                     | Per-sec vCPU+RAM; $150/mo Pro                         | The default hosted choice; deepest agent ecosystem                                       |
| **Modal**                         | gVisor (or VM)                             | FS/dir snapshots; memory snapshots alpha; Volumes                      | No                                                               | Per-sec, ~3× E2B CPU rate for sandboxes; GPU strength | Best if GPUs + Python are the centre; managed only                                       |
| **Daytona**                       | Containers; VMs on VM tier                 | Persistent workspaces; pause/fork only on VM tier                      | Partial — OSS repo unmaintained since core went private Jun 2026 | Per-sec, no platform fee                              | Fast and capable, but the self-host story regressed                                      |
| **Fly.io Sprites**                | Firecracker                                | CoW FS checkpoints; memory only on warm suspend                        | No                                                               | Actual-use per-sec; idle free                         | Best persistence story (100 GB disk, no lifetime limit)                                  |
| **Vercel Sandbox**                | Firecracker                                | FS auto-snapshot on stop, resume by name                               | No                                                               | Active-CPU + provisioned-mem + per-creation           | Fastest in the independent burst test; Vercel gravity                                    |
| **Cloudflare Sandbox/Containers** | MicroVM per container + Durable Object     | Container snapshots, R2 backups; DO hibernation for agent state        | No                                                               | Workers Paid + 10 ms-billed container usage           | Interesting because our doors already live on Cloudflare; TS-only SDK                    |
| **Runloop**                       | MicroVM                                    | Disk snapshots, suspend/resume (disk only)                             | VPC on enterprise                                                | Per-sec + platform tiers                              | **Winding down — do not adopt**                                                          |
| **Morph**                         | Full VMs                                   | **Infinibranch**: branch running VMs (memory+disk) in <250 ms (vendor) | No                                                               | "Compute units" bundles                               | The branching specialist — evals/RL/search trees                                         |
| **CodeSandbox → Together**        | Firecracker                                | Memory+FS hibernation, live forking                                    | No                                                               | Two pricing shapes in sources — unreconciled          | Solid tech, ownership/pricing in transition                                              |
| **Ona (ex-Gitpod)**               | Not published                              | Ephemeral environments from devcontainer                               | Vendor-run single-tenant only; community self-host gone          | Subscription compute units                            | Now an agent _orchestration_ layer (and inside OpenAI since Aug 2026), not a sandbox API |
| **Blaxel → Baseten**              | MicroVM                                    | Standby snapshots memory+FS, indefinite at $0 compute                  | BYOC enterprise                                                  | Per-GB-RAM-second — cheapest standby math             | Tech lives on under Baseten; evaluate Baseten Sandboxes, not Blaxel-the-company          |
| **Northflank**                    | Kata+Cloud Hypervisor, Firecracker, gVisor | Volumes/services; no memory-snapshot semantics found                   | **Strongest BYOC** (your VPC, their control plane)               | Cheapest list CPU here                                | A full app platform that also does sandboxes well                                        |
| **microsandbox**                  | libkrun microVM                            | Snapshots (semantics thin in docs)                                     | **Yes — local-first, Apache-2.0**                                | Self-host $0; cloud beta                              | The embeddable/local end of the spectrum                                                 |
| **BoxLite**                       | KVM/HVF microVM                            | Persistent boxes, CoW disks, .boxlite archives                         | **Yes — embeddable library by design**                           | Self-host $0; cloud per-hour                          | A runtime you link, not a service you rent                                               |
| **Agent Substrate**               | gVisor or Kata/Cloud Hypervisor            | Tiered snapshots (VOLUMES/ROOTFS/MEMORY), warm-pool multiplexing       | **Yes — OSS on any K8s**                                         | Infra cost only (+ GKE managed option)                | The only entry whose _unit of design_ is 10⁴–10⁶ mostly-idle sessions per cluster        |

Also credible, one line each (details in supplement 3): Tensorlake
(memory snapshots, warm pools, BYOC; a named Runloop migration target),
AWS Bedrock AgentCore + Lambda MicroVMs, Azure Container Apps dynamic
sessions, Novita, Prime Intellect (RL-focused), Cube Sandbox (Tencent,
E2B-compatible API, self-host), Docker Sandboxes, Upstash Box (persistent
per-user agent computer, hardened containers not microVMs).

**The isolation layer, in one breath:** Firecracker = minimal KVM
microVMs (~125 ms boot, a building block, no platform); gVisor = a
user-space kernel intercepting syscalls (no KVM needed, an I/O tax on
syscall-heavy work, and the only one of the three with native
checkpoint/restore — which is why Substrate's fast path is gVisor-shaped);
Kata = a real VM per Pod with container UX (heaviest, best K8s fit, GPU
and confidential-computing options). 2026 consensus in the sources: plain
shared-kernel containers are not a security boundary for untrusted agent
code, and egress control matters as much as the isolation tier.

---

## Part 3.5 — Per-connection ephemeral shells: ContainerSSH vs an Agent-Substrate-style front door

_Added question from Drew (2026-11-23 ~19:34 CST): "We might not use
ContainerSSH. We might have a load balancer use something like Agent
Substrate to make one per connection, or would we use ContainerSSH?" —
for the specific job of giving a human or an agent a shell, one ephemeral
environment per connection._

The two shapes differ in _where the per-connection environment comes
from_:

- **ContainerSSH** is purpose-built for exactly this job. It is an SSH
  server that, on each incoming connection, launches a fresh container
  (via Kubernetes, Docker, or a custom backend) for that connection,
  authenticates the user against your own auth/config servers over HTTP
  callbacks, and destroys the container at disconnect. Session recording
  to S3-compatible storage and audit logging are built in — it comes out
  of the "SSH bastion with compliance recording" world, and it shows.
- **Agent-Substrate-style** puts a request router (a load balancer in
  the general shape Drew describes) in front of a warm pool: the
  connection is routed to a sandbox that is either claimed fresh from the
  pool or _restored from a snapshot_, and on disconnect the environment
  can be snapshotted rather than destroyed — so the "per connection"
  environment can be the _same_ environment across many connections.

| Axis                                | ContainerSSH                                                                                                                                                                                     | Agent-Substrate-style                                                                                                                                                                                                                                                                                       |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Connection latency**              | Pays a container start per connection (sub-second to a few seconds depending on image warmth and runtime; it can pre-pull images but the container itself is created on connect)                 | Warm-pool claim is milliseconds of scheduling; a snapshot restore is the <500 ms vendor-claim path (measured: <1 s local, 5–30 s if the snapshot has to cross nodes — see Part 1). Fresh-from-golden starts are comparable to a container start                                                             |
| **Statefulness across reconnects**  | **None, by design.** Disconnect destroys the environment; reconnect is a brand-new container. Any persistence must be bolted on (home-directory volumes) and is filesystem-only, never processes | **The whole point.** Disconnect can suspend; reconnect restores memory + filesystem, processes intact. A dropped SSH session resumes _mid-work_, not at a fresh login                                                                                                                                       |
| **Audit / recording**               | **Built in and mature:** full session recording (asciinema-style) to S3, audit logs, designed for regulated bastion use                                                                          | Not a feature. Substrate records actor lifecycle and identity, but terminal session recording would be ours to build (or record at the gateway layer)                                                                                                                                                       |
| **Identity binding per connection** | Per-connection auth via pluggable auth server (password/public-key via HTTP callback), mapped to a per-connection container config; identity is a _connection_ property                          | Identity is a property of the **Actor**, not the connection — SPIFFE identity that persists across suspend/resume and across workers. Stronger session identity, weaker notion of "this specific connection was authenticated just now" unless the router layer adds it                                     |
| **Operational weight**              | One focused service + a container runtime/Kubernetes to launch into. Small, single-purpose, CNCF Sandbox, but a comparatively small project/community                                            | A whole control plane (Postgres state store, per-node agents, gateways, object storage for snapshots) whose design centre is millions of mostly-idle actors — vastly more machinery than shell access alone justifies                                                                                       |
| **Protocol fit**                    | Speaks SSH natively — it _is_ the SSH server                                                                                                                                                     | **Does not speak SSH today.** Substrate's ingress is HTTP(S)-only; arbitrary raw-TCP ports are a stated limitation (Part 1). A shell front door would need an SSH-terminating router in front that then tunnels into the sandbox — i.e. we'd build the ContainerSSH-shaped piece anyway, minus its maturity |

**Verdict: ContainerSSH for shells; Substrate's model for agent
sessions — and they compose rather than compete.** For the shell job
specifically, ContainerSSH wins on every axis that matters for a bastion:
it terminates SSH itself, records sessions out of the box, and its
statelessness is a _feature_ for human shell access (a fresh, known-good
environment per connection, nothing weird left running between logins).
The Substrate shape's one decisive advantage — state surviving
disconnect — is worth little for an interactive shell and everything for
an _agent_ session, which is precisely the split we already have: our
fleet's temporary sessions (Actions exec today) are agent sessions with
leases and identity, not shells. If shell-into-the-cluster is ever
wanted, the honest architecture is ContainerSSH as the SSH front door,
optionally launching _into_ whatever sandbox tier sits behind it —
ContainerSSH's backend is pluggable (any Kubernetes pod it can create),
so "ContainerSSH in front, warm/snapshot sandboxes behind" is available
later without re-deciding. What we should not do is stretch Agent
Substrate into an SSH server: we'd re-build auth, recording, and TCP
ingress on top of a control plane designed for a different job, to get a
worse bastion and a distracted substrate.

---

## Part 4 — Fit for us

What we already have: **fleet Actions exec** — temporary GitHub Actions
runner sessions with broker-issued handles, enforced timeouts, tiered
lease extensions, R2/artifact egress — plus the fleet queue's session and
placement model, on a stated trajectory toward our own Talos + KubeVirt +
Cozystack cluster. In other words, we already built the _control_ slice of
this problem (identity, leases, dispatch, audit) and rent the _compute_
slice from GitHub's runners.

**What an external substrate would give us that we don't have:**
(1) snapshot/resume of a _live_ session — our Actions sessions are
start-from-scratch per lease; nothing today can freeze a running agent
environment and wake it later; (2) density — idle sessions costing disk
instead of a runner slot; (3) a self-hosted sandbox tier on our own
cluster for work that shouldn't leave the estate (the hosted field is
almost entirely someone else's cloud, which sits badly with our
secrets-by-name and laptop-first posture); (4) fork-from-snapshot, which
is the primitive eval/RL-style fan-out wants.

**What adopting one would cost or conflict with:** every hosted option
duplicates the fleet broker's job (identity, session lifecycle, policy)
inside a vendor control plane we'd then have to reconcile with ours —
the exact "two reconcilers" problem the CNCF lane flagged for Argo vs
Flux. The self-hostable runtimes (microsandbox, BoxLite, Cube) are
libraries in search of a control plane — we'd still build the fleet-side
integration, which is most of the work. And the field is consolidating
fast (Runloop dead, Blaxel absorbed, Daytona's OSS story regressed):
picking a young vendor here is picking a migration.

**The call on Agent Substrate specifically: WATCH, trending to TRIAL —
do not adopt yet.** Reasoning: it is the only entrant designed for
exactly our shape (Kubernetes-native, self-hostable, actor identity
decoupled from compute — which rhymes with our own fleet-CA identity and
broker model, including credentials injected at a gateway rather than
held in the sandbox). It is also six months old, its headline numbers
shrank at 1.0 and remain unverified independently at anywhere near claim,
its measured bottleneck (snapshot disk bandwidth) is precisely the
resource our LINSTOR/DRBD storage story would need to prove, it supports
only the two newest Kubernetes minors, and it has no verified independent
production user yet. The trial trigger, when the Talos/Cozystack cluster
exists: run Substrate's own benchmark harness on our storage, measure
suspend/resume against LINSTOR, and compare against the boring baseline
(KubeVirt VM snapshots + our existing queue leases). Until then it costs
nothing to watch — it is CNCF Sandbox, Apache-2.0, and moving fast enough
that a quarter changes the picture.

**And the other names:** Freestyle and boat are hosted products to rent
for overflow or experiments, not foundations (neither self-hosts). The
self-hostable runtimes — BoxLite and microsandbox — are the sleeper
category for us: an embeddable microVM runtime could give the fleet a
local sandbox primitive without a second control plane, and deserves a
spike before any vendor commitment.

---

## Sources (primary first)

- Google Cloud Blog, "Open infrastructure for autonomous agents comes to
  CNCF" (2026-11-05) — cloud.google.com/blog/products/containers-kubernetes/open-infrastructure-for-autonomous-agents-comes-to-cncf
- Google Open Source Blog, "Sandboxing Untrusted AI Agents at Scale"
  (2026-11-05) — opensource.googleblog.com/2026/11/sandboxing-untrusted-ai-agents-at-scale.html
- Agent Substrate repo + architecture doc — github.com/agent-substrate/substrate
  (docs/concepts/architecture.md); v1.0.0 release notes
- CNCF project page — cncf.io/projects/agent-substrate
- gVisor checkpoint/restore docs — gvisor.dev/docs/user_guide/checkpoint_restore
- Independent load test (Cofano, v0.6.0 on GKE) — medium.com/google-cloud,
  "From 20 to 120 agent sessions on three GKE nodes"
- Qovery, primary-source roundup (2026-11-18) — qovery.com/blog/agent-substrate-density-resume-own-kubernetes-cluster
- Vendor pages for Freestyle (freestyle.sh/pricing, /docs), boat
  (boat.dev, docs.boat.dev), BoxLite (github.com/boxlite-ai/boxlite),
  boxer.dev, BarakChamo/boxer docs
- Field aggregators used with verification dates stated in supplement 3:
  agentsandboxes.org, sandbox.watch, seminal.ai; ComputeSDK burst
  benchmark (2026-08-21) as cited therein

_Unverified / flagged items are listed at the end of each supplement.
Nothing in this record relies on a single aggregator for a load-bearing
claim about Agent Substrate itself._
