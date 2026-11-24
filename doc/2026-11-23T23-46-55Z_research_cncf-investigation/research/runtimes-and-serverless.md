# Container runtimes, sandboxes, Wasm, and serverless — the CNCF landscape slice

Researched 2026-11-23. Scope: the runtime layer Drew will eventually have to pick for the
Kubernetes platform work (Cozystack-based) and for the id project's app sandboxing.
Every project carries its CNCF status and a maturity verdict. "What it is" and "should
we use it" are kept in separate paragraphs on purpose. Performance claims are given in
benchmark shape — who measured what, on what workload — never as bare adjectives.

## Drew's questions, answered up front

- **What container runtimes are there, and are any much faster?** Two layers: high-level
  runtimes (containerd, CRI-O) that pull images and manage lifecycles, and low-level OCI
  runtimes (runc, crun, youki) that set up namespaces/cgroups and start the process. The
  low-level runtime's own execution time differs by tens of milliseconds between runc,
  crun, and youki; a container's _perceived_ startup time is dominated by image pull and
  by the application's own initialisation. No OCI runtime is "much faster" in any sense
  that survives contact with a real workload. The big startup wins live elsewhere: lazy
  pulling (eStargz, Nydus) and P2P distribution (Dragonfly — see the registries file).
- **Is KubeVirt basically the one way people make VMs in Kubernetes now?** Yes.
  KubeVirt is how full VMs run inside Kubernetes: each VM is a pod, managed with the
  same API, with live migration. It graduated from the CNCF on 2026-11-04
  ([CNCF project page](https://www.cncf.io/projects/kubevirt/); announced at KubeCon NA
  on 2026-11-10,
  [CNCF announcement](https://www.cncf.io/announcements/2026/11/10/cloud-native-computing-foundation-announces-kubevirt-graduation/)).
  Cozystack's VM offering is KubeVirt underneath
  ([CNCF blog](https://www.cncf.io/blog/2025/04/28/open-source-paas-cozystack-becomes-a-cncf-sandbox-project/)).
  There is no serious rival for VMs-in-Kubernetes.
- **Which sandbox runtime is fastest?** It depends on which axis you measure, and the
  honest answer is a shape, not a name — see the comparison table in §3. runc is native
  speed with the weakest isolation; microVMs (Firecracker/Kata) are near-native on CPU
  with a ~125 ms boot tax; gVisor starts instantly but taxes every syscall; Wasm starts
  in single-digit milliseconds and runs near-native for compute that fits WASI, but
  can't do arbitrary Linux syscalls at all.
- **Should we use WasmEdge because id apps might run directly on it?** The Wasm _idea_
  is right for id apps — capability-based sandboxing is almost exactly the id security
  model — but WasmEdge specifically is not the obvious engine. Wasmtime is the
  standards reference with the most complete Component Model support, and Spin (built
  on Wasmtime) is the developer-facing framework with CNCF governance. WasmEdge's edge
  is AI-inference plugins and embedding via crun/Kuasar. Decide on the Component Model
  and WASI surface first; the engine is replaceable underneath it. Detail in §4.
- **Are OpenFaaS and OpenFunction the same thing, and are either fast?** No, and
  "sometimes." OpenFaaS is an independent open-core product (not CNCF) with its own
  gateway and watchdog process model; OpenFunction is a CNCF Sandbox framework that
  assembles Knative, KEDA, and Dapr into a FaaS. Drew's memory of slowness is accurate
  for the classic shape — gateway hops plus container cold starts — and the fix in both
  ecosystems is the same: keep-warm pools (which spend the money scale-to-zero was
  supposed to save) or Wasm functions (Spin), which cold-start in milliseconds. The
  pieces worth owning are Knative Serving and KEDA, not necessarily a FaaS wrapper.
  Detail in §5.
- **What is "Yuki"? What is "Urun C"?** "Urun C" is **urunc**, Nubificus's OCI runtime
  that runs unikernels as containers (§4.4). "Yuki" is almost certainly **youki**, the
  Rust OCI runtime, mis-heard/mis-transcribed: a widely quoted runwasi talk credits
  "Yuki's libcontainer" for the shim that dispatches between Linux and Wasm workloads,
  and that library is youki's
  ([The New Stack](https://thenewstack.io/webassembly-and-containers-love-affair-on-kubernetes/)).
  No distinct CNCF project named "Yuki" exists; the other Yuki on GitHub (USTC LUG) is
  a software-repository mirroring tool, not a runtime. Uncertainty: medium-high
  confidence on the youki identification; it rests on the libcontainer detail, which
  only fits youki.

---

## 1. The OCI runtime stack (the boring layer that runs everything)

Two layers, and confusing them is the source of most bad runtime takes:

- **High-level runtime** — talks to the kubelet via CRI, pulls and unpacks images,
  manages the container lifecycle, delegates the actual process start.
- **Low-level (OCI) runtime** — takes a bundle (rootfs + config.json), creates
  namespaces and cgroups, applies seccomp/capability policy, starts PID 1, exits.

| Project                                        | Layer             | CNCF status                                                                                                             | What it is                                                                                                                                                                                                             | Maturity verdict                                                                                             |
| ---------------------------------------------- | ----------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| [containerd](https://containerd.io/)           | High-level        | **Graduated**                                                                                                           | The default high-level runtime under Docker and most Kubernetes. Daemon + shims; snapshotters (overlayfs, stargz, nydus) plug in here.                                                                                 | The default. Boring, universal, what Cozystack/Talos nodes effectively run.                                  |
| [CRI-O](https://cri-o.io/)                     | High-level        | **Graduated**                                                                                                           | A Kubernetes-only CRI implementation from the Red Hat/OpenShift world; minimal by design — only what the kubelet asks for.                                                                                             | Equally legitimate; you get it by choosing OpenShift-flavoured stacks, not by seeking it out.                |
| [runc](https://github.com/opencontainers/runc) | Low-level         | Not CNCF (OCI project)                                                                                                  | The reference OCI runtime, in Go, extracted from Docker. What containerd and CRI-O call by default.                                                                                                                    | The compatibility baseline. Everything is tested against runc first.                                         |
| [crun](https://github.com/containers/crun)     | Low-level         | Not CNCF (containers/Podman ecosystem)                                                                                  | The same OCI spec implemented in C by Red Hat's containers team; default runtime for Podman. Can optionally embed a Wasm engine (WasmEdge/Wasmtime) and run Wasm workloads directly.                                   | Production-proven at Podman scale. The speed pick of the three — see below.                                  |
| [youki](https://github.com/youki-dev/youki)    | Low-level         | **Sandbox** (accepted 2024-10-17)                                                                                       | The OCI spec implemented in Rust. Its `libcontainer` crate is reused as a library by runwasi's shims (§4.3).                                                                                                           | Real and used (AKS, runwasi), but the youngest of the three; pick it for the Rust/Wasm story, not for speed. |
| iSulad                                         | High-level        | Not CNCF (openEuler)                                                                                                    | A lightweight C container engine with built-in CRI, from the openEuler ecosystem; pairs with Kuasar below.                                                                                                             | Regional (China/openEuler) ecosystem. Relevant only as Kuasar's home turf.                                   |
| [Kuasar](https://github.com/kuasar-io/kuasar)  | Sandbox framework | Not CNCF — sandbox _application_ filed 2023, donation announced as planned by Huawei/openEuler, acceptance not verified | A Rust framework that runs several sandbox kinds side by side on one node — microVM (Cloud Hypervisor, StratoVirt, QEMU), app-kernel (Quark), Wasm (WasmEdge, Wasmtime), plain runc — behind containerd's Sandbox API. | Interesting architecture, small blast radius so far. Watch, don't adopt.                                     |

**Where "speed" actually differs.** The low-level runtime only exists for the
create→start window; after that, your process talks to the kernel directly and the
runtime is out of the picture. Measured on that window, youki's own benchmark suite
reports a full create→start→delete cycle at ~111 ms mean for youki, roughly 2× faster
than runc and roughly 2.5× slower than crun on the same harness
([youki benchmark write-up](https://dev.to/bhkbdbhatt/i-replaced-runc-with-a-rust-container-runtime-youki-for-a-week-4022)).
Those are real differences — and they are tens of milliseconds, once per container,
against image pulls measured in seconds and application init measured in
hundreds of milliseconds to seconds. Swapping runc for crun is a legitimate free win
on dense nodes (and Podman shops already get it by default); it is not a performance
strategy.

**The startup lever that actually matters: don't pull the whole image first.**
The classic measurement (Harter et al., FAST '16) found pulling accounts for ~76% of
container start time while only ~6.4% of the pulled data is read at startup
([lazy-pulling talk, NTT/Ant Group](https://www.scribd.com/document/591327806/lazypullslides)).
Two formats exploit that:

| Project                                                                          | CNCF status                                         | What it is                                                                                                                                                                                                               | Maturity verdict                                                                                                                                                                                                                                                           |
| -------------------------------------------------------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [stargz-snapshotter / eStargz](https://github.com/containerd/stargz-snapshotter) | containerd subproject (not separately CNCF-listed)  | A containerd snapshotter plus an image format (eStargz) that stays a valid OCI/gzip layer, so lazy-pulled images still run on unaware tooling; files are fetched on demand with a prefetch list for boot-critical files. | The conservative choice: standard-compatible, graceful fallback. Measured 14.0 s → 3.3 s on a cold `python:3.13` run in Lima's docs ([Lima example](https://github.com/lima-vm/lima/blob/HEAD/website/content/en/docs/examples/containers/containerd/advanced/stargz.md)). |
| [Nydus](https://github.com/dragonflyoss/nydus)                                   | Part of Dragonfly (Graduated — see registries file) | A separate image format (RAFS) with chunk-level dedup and a userspace daemon; more aggressive optimisation than eStargz, at the cost of leaving the plain OCI image format behind.                                       | Proven at Alibaba scale via Dragonfly. Worth it when image distribution is a measured bottleneck, not before.                                                                                                                                                              |

Honest caveat on lazy pulling: the cost is deferred, not deleted. A 2026 measurement
of eStargz and AWS's SOCI snapshotter against model-serving workloads found cold
time-to-first-prediction becomes size-independent (~17 s from 2 GB to 140 GB images,
~33× faster than eager at the large end), but a full read of a 14 GB model through the
lazy mount took 105 s versus 72 s for the eager pull it replaced, and cache exhaustion
on the node can fail reads in already-running pods
([arXiv paper](https://arxiv.org/pdf/2608.19412v1)). Lazy pulling is a cold-start
optimisation with an operational tail; treat it that way.

**Should we use it (our estate):** containerd + runc is the default we inherit from
Talos/Cozystack and there is no reason to fight it. crun is a drop-in worth taking if
a node-density problem ever measures out. eStargz is the only startup optimisation
with a sane fallback story, and it composes with Nix-built images (convert at publish
time). Kuasar and iSulad are not on our path.

## 2. VMs and microVMs

| Project                                                                                   | CNCF status                            | What it is                                                                                                                                                                                                                                                                                                                                                                             | Maturity verdict                                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------- | -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [KubeVirt](https://kubevirt.io/)                                                          | **Graduated** (2026-11-04)             | Adds `VirtualMachine`/`VirtualMachineInstance` resources to Kubernetes; each VM runs inside a pod (QEMU/KVM in a `virt-launcher` pod), so VMs get Kubernetes scheduling, networking, storage, and RBAC. Live migration between nodes is a first-class operation. Companion CDI imports disk images from URLs/registries/uploads.                                                       | The standard. Ten years old, ~2,088 contributors across ~520 organisations at graduation, production at Cloudflare, CoreWeave, NVIDIA ([CNCF](https://www.cncf.io/announcements/2026/11/10/cloud-native-computing-foundation-announces-kubevirt-graduation/)). Cozystack builds its VM product on it. |
| [Firecracker](https://firecracker-microvm.github.io/)                                     | Not CNCF (AWS, Apache-2.0)             | A minimal KVM virtual machine monitor in Rust — five virtio devices, ~50k lines — built for AWS Lambda/Fargate. Boots to application code in as little as ~125 ms with <5 MiB overhead per microVM and up to 150 creations/sec/host (NSDI '20 paper figures, [comparison with sources](https://www.agentnative.dev/compare/firecracker-vs-gvisor-vs-containers-for-agent-sandboxing)). | The microVM engine itself, not a Kubernetes system. You meet it inside Kata, Kuasar, or a serverless platform, rarely directly.                                                                                                                                                                       |
| [Kata Containers](https://katacontainers.io/)                                             | Not CNCF (OpenInfra Foundation)        | Runs each container/pod in a lightweight VM (QEMU, Cloud Hypervisor, or Firecracker underneath) while presenting an ordinary OCI/CRI interface. The "containers, but with a hypervisor boundary" option.                                                                                                                                                                               | Mature and widely deployed for multi-tenant clusters. The way to get VM isolation without changing how workloads are packaged.                                                                                                                                                                        |
| [Cloud Hypervisor](https://www.cloudhypervisor.org/)                                      | Not CNCF (Linux Foundation / rust-vmm) | A Rust VMM focused on cloud workloads; the hypervisor Kata and Kuasar prefer when QEMU is more than they want.                                                                                                                                                                                                                                                                         | Solid component, not a platform.                                                                                                                                                                                                                                                                      |
| [VM API](https://blogs.vmware.com/cloud-foundation/2026/11/10/vm-api-and-kubevm-service/) | **Sandbox** (accepted 2026-11-09)      | Broadcom's contribution of its VM Operator lineage: a provider-neutral `VirtualMachine` API contract (Cluster-API-shaped) so the same VM intent can be fulfilled by different backends — vSphere first. The VMs stay native on their hypervisor; Kubernetes is the API surface, not the host.                                                                                          | Two weeks old as a CNCF project, alpha API (`v1alpha1`). It answers a different question than KubeVirt: "one VM API across infrastructures I already have," not "run VMs on my cluster." Watch only.                                                                                                  |

**KubeVirt vs running VMs outside Kubernetes.** KubeVirt pays when the platform is
already Kubernetes: one API, one scheduler, one network/storage model, GitOps for VMs
and containers alike, and live migration handled by the same control plane —
Cozystack is the proof this composes into a sellable cloud. It costs you the
Kubernetes way of doing everything: VM disks are PVCs (live migration needs
ReadWriteMany storage), VM networking is pod networking plus Multus/bridges when the
VM needs a real LAN identity, and your operators now debug two systems wearing one
hat. If the estate were mostly VMs with a few containers, a plain hypervisor platform
(Proxmox-class) is simpler; for Drew's direction — Kubernetes as the platform, VMs as
one workload type on it — KubeVirt is the correct default and effectively the only
one.

**Should we use it:** yes, via Cozystack, when VM workloads arrive. Not before —
KubeVirt on an empty cluster is machinery without a job.

## 3. Sandboxed runtimes: which is "fastest"?

The four isolation shapes, with the trade stated as measured behaviour:

| Shape            | Example                                                   | Startup                                                                         | Steady-state cost                                                                                                                                                                                                                                                                                                                                    | Isolation                                                                                                                    | Compatibility cost                                                                                                                                                   |
| ---------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Shared kernel    | runc/crun/youki                                           | Runtime window only (tens of ms, §1)                                            | None — native syscalls                                                                                                                                                                                                                                                                                                                               | Namespaces/cgroups only; a kernel exploit is a host exploit                                                                  | None — it _is_ Linux                                                                                                                                                 |
| Userspace kernel | [gVisor](https://gvisor.dev/) (`runsc`, Google, not CNCF) | Milliseconds — no VM to boot                                                    | A tax on every syscall; gVisor's own engineering blog measured its userspace network stack at ~17 Gbps down / ~8 Gbps up vs ~42/43 Gbps native ([figures collected here](https://www.agentnative.dev/compare/firecracker-vs-gvisor-vs-containers-for-agent-sandboxing)); commonly summarised as ~10–30% on I/O-heavy work, near-zero on pure compute | Strong: the app talks to a Go re-implementation of the Linux ABI (the Sentry), which makes only a small set of host syscalls | Real: the ABI is a subset (historically ~211 of ~319 syscalls implemented; io_uring and exotic calls are the usual gaps) — arbitrary binaries occasionally just fail |
| microVM          | Firecracker / Kata                                        | ~125 ms to userspace (Firecracker's published figure)                           | Near-native CPU; device I/O crosses virtio                                                                                                                                                                                                                                                                                                           | Strongest commodity option: hardware virtualisation boundary, tiny device model                                              | Almost none — a real guest kernel runs anything Linux runs; no GPU in Firecracker (Kata/QEMU can pass devices through)                                               |
| Wasm sandbox     | Wasmtime / WasmEdge (§4)                                  | Single-digit ms; Fermyon reports ~0.5 ms for Spin apps in favourable conditions | Near-native for compiled compute inside the module; every host interaction goes through WASI calls                                                                                                                                                                                                                                                   | Capability-based: the module has _no_ ambient authority — no filesystem, network, or env unless granted                      | The whole story: only WASI APIs exist. No fork, no threads-as-OS-threads (improving), no arbitrary syscalls, language/library support varies                         |

So "fastest" decomposes: fastest to _start_ is Wasm, then gVisor, then microVM, then
a cold container pull (which is why §1's lazy pulling matters more than this table).
Fastest at _steady state_ is runc (nothing between app and kernel), then microVM,
then Wasm (workload-dependent), then gVisor on syscall-heavy work. Fastest _per unit
of isolation bought_ is the actual question, and for untrusted multi-tenant code the
field has converged on microVMs (Lambda, Fargate, Fly.io, E2B) and gVisor (GKE
Sandbox, Cloud Run, Modal) as the two production answers.

**Should we use it:** default runc for our own code; gVisor or Kata as the
RuntimeClass for anything multi-tenant or untrusted on the cluster; microVM thinking
returns with the id app-sandbox question (§4) and with agent sandboxes generally.

## 4. WebAssembly runtimes — and the id-project question

| Project                                                                        | CNCF status                            | What it is                                                                                                                                                                                                                                                                                                                   | Maturity verdict                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------------------------ | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Wasmtime](https://wasmtime.dev/)                                              | Not CNCF (Bytecode Alliance)           | The reference Wasm runtime: Rust, Cranelift JIT/AOT, the most complete Component Model and WASI implementation (WASI 0.2 complete; WASI 0.3 — native async in the Component Model, shipped 2026-06-11 — enabled by default from Wasmtime 46). Embeds as a library.                                                           | The standards centre of gravity. Spin and runwasi build on it. If a Wasm feature exists anywhere first, it's here.                                                                                                                                                                     |
| [WasmEdge](https://wasmedge.org/)                                              | **Sandbox** (since 2021)               | A C++ Wasm runtime aimed at edge and AI: AOT compiler, plugin host functions (WASI-NN for inference, crypto, TensorFlow heritage), embeddable, and wired into crun, Kuasar, and Docker's Wasm support.                                                                                                                       | Genuinely deployed at edge/AI niches. Its Component Model support has historically trailed Wasmtime's ("in progress" in current comparisons, [runtime map](https://asopi.tech/en/blog/20260724_3/)) — verify the exact WIT/WASI surface you need before betting an app platform on it. |
| [Spin](https://www.spinframework.dev/) + [SpinKube](https://www.spinkube.dev/) | **Sandbox** (both accepted 2025-01-21) | Spin is Fermyon's developer framework for Wasm apps — an HTTP/event component model on top of Wasmtime, `spin up` locally. SpinKube is the Kubernetes operator + containerd shim that runs those apps as pods via RuntimeClass. Fermyon was acquired by Akamai (Dec 2025); both projects were already under CNCF governance. | The best developer experience in Wasm-land and the cleanest Kubernetes path. Sandbox maturity: small maintainer pool, no completed security audit yet ([hands-on review](https://dev.to/bhkbdbhatt/running-wasm-on-kubernetes-with-spinkube-a-hands-on-guide-13m6)).                   |
| [wasmCloud](https://wasmcloud.com/)                                            | **Incubating** (since 2024-11-08)      | A full platform for distributed Wasm components: components declare capabilities (HTTP, KV, messaging) in WIT, the platform links providers (often over NATS) at runtime, so the same component runs on laptop, cluster, or edge unchanged.                                                                                  | The most opinionated — and most complete — answer to "apps as components." Incubating maturity with real production users. The lattice/NATS coupling is an architectural commitment, not a detail.                                                                                     |
| [runwasi](https://github.com/containerd/runwasi)                               | containerd subproject                  | The containerd shim library that lets containerd run Wasm modules as if they were containers (dispatching on the binary: Linux → youki's libcontainer, Wasm → Wasmtime/WasmEdge/Spin embedded in the shim).                                                                                                                  | Plumbing, not a product — but it's the plumbing everything above stands on.                                                                                                                                                                                                            |
| Wasmer                                                                         | Not CNCF (company-led)                 | A Rust runtime with its own ecosystem (WASIX extensions, package registry, hosting).                                                                                                                                                                                                                                         | Capable, but its extensions pull away from the standard WASI centre; not where we'd build.                                                                                                                                                                                             |

### 4.1 Should id apps run on Wasm — and on which engine?

**What it is (the model):** a Wasm component is a single compiled artifact with no
ambient authority. It can only touch what the host grants through WASI interfaces —
files under a preopened directory, an HTTP handler, a KV store — described in typed
WIT interfaces. Startup is milliseconds, density is thousands of instances per node,
and the artifact is architecture-independent. For an app platform whose security
story is capability attenuation (the id design), this is a strikingly good fit
on paper: the sandbox _is_ the capability model, not a fence around it.

**The honest state of the platform:** WASI 0.2 (Component Model + wasi:http) is
stable and shippable; WASI 0.3 landed 2026-06-11 with native async, and the runtime
support is fresh. What still bites in 2026 assessments: language/library coverage
(Rust, Go, JS/TS are good; much else is partial), debugging and observability are
years behind containers, no general filesystem/socket programming model beyond what
WASI grants, and the Kubernetes integration — while real (SpinKube, wasmCloud,
runwasi) — is thin compared to containers; even sympathetic 2026 verdicts conclude
Wasm _complements_ containers for short-lived HTTP functions, plugins, and untrusted
code rather than replacing them for general services
([2026 verdict with evidence](https://www.qovery.com/blog/webassembly-replace-containers-kubernetes-2026-verdict)).
Azure retiring its AKS WASI node pools (May 2025) and Docker deprecating its Wasm
workload support are the cautionary market signals in the same piece.

**Verdict for id:** design id apps against the **Component Model + WASI interfaces**
(the durable standard), not against a runtime's plugin API. Default engine:
**Wasmtime**, directly or via Spin, because it tracks the standard fastest and the
id project's Rust centre of gravity matches it. WasmEdge becomes the pick only if a
workload needs its specific host functions (WASI-NN inference at the edge is the
real example) or the crun/Kuasar embedding path. Choosing WasmEdge _as the platform_
because it's the CNCF one would be governance-driven, not engineering-driven — and
either way, keep a container-shaped escape hatch: apps that need a real Linux
userspace (a database, a shell tool, a weird native library) don't fit WASI today,
and the platform should run those as ordinary pods rather than force them into
components.

### 4.2 urunc — "Urun C"

[urunc](https://urunc.io/) (Nubificus) is an OCI/CRI-compatible low-level runtime —
"the runc for unikernels." Instead of starting a Linux process, it starts a unikernel
(Unikraft, rumprun, MirageOS) on a monitor the image's annotations select: QEMU,
Firecracker, Cloud Hypervisor, or seccomp/hvt-style solo5 monitors. The unikernel
binary ships inside a normal OCI image, so Kubernetes, containerd, and registries
handle unikernels with entirely ordinary tooling; it has been demonstrated on
Kubernetes, k3s, and Knative
([CNCF sandbox application #353](https://github.com/cncf/sandbox/issues/353), filed
2025-03-10; the project is a sandbox _applicant_, not yet a CNCF project). Its
packaging tooling even includes `bunix`, which builds unikernel OCI images with Nix.

**Verdict:** a genuinely elegant bridge — if you already have unikernel workloads.
We don't, and unikernels trade away the entire Linux userspace (debugging, tooling,
library ecosystem) for boot time and attack surface that Wasm and microVMs mostly
cover with better ecosystems. File under "know it exists"; revisit only if a specific
appliance-shaped workload appears.

## 5. Serverless: OpenFaaS, OpenFunction, and the pieces underneath

| Project                                   | CNCF status                                                 | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Maturity verdict                                                                                                                                                                                               |
| ----------------------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Knative](https://knative.dev/)           | **Incubating**                                              | The substrate: Serving (request-driven containers with scale-to-zero, traffic splitting, an activator that buffers requests during cold start) + Eventing (CloudEvents routing). Runs ordinary containers, not just functions.                                                                                                                                                                                                                              | The piece of "serverless on Kubernetes" that survived contact with production. If we adopt one thing from this section, it's Knative Serving.                                                                  |
| [KEDA](https://keda.sh/)                  | **Graduated**                                               | Event-driven autoscaling for any workload: scalers for queues, Kafka, cron, Prometheus metrics, etc., including scale-to-zero for Deployments/Jobs. Not a FaaS — a better HPA.                                                                                                                                                                                                                                                                              | Useful with or without any serverless story. The highest value-per-effort project in this file after KubeVirt.                                                                                                 |
| [OpenFaaS](https://www.openfaas.com/)     | Not CNCF — independent, open-core commercial (OpenFaaS Ltd) | A FaaS with its own model: functions are containers fronted by a gateway; a tiny watchdog process inside each function container receives HTTP and invokes your handler. faasd is the same stack on a single VM without Kubernetes. Community Edition carries commercial-use limits; scale-to-zero and the polished autoscaling live in the paid tier ([comparison](http://itechguides.com/9-best-free-and-open-source-functions-as-a-service-platforms/)). | A real product with happy users, but it's a vendor relationship, not community infrastructure — and its licensing shape is exactly the HashiCorp-pattern risk Drew is moving away from.                        |
| [OpenFunction](https://openfunction.dev/) | **Sandbox**                                                 | A FaaS _framework_ assembled from other CNCF parts: functions-framework + Shipwright/Buildpacks for builds, Knative Serving or its own serving layer to run, KEDA to scale, Dapr for events/bindings ([design write-up](https://www.kubesphere.io/blogs/faas-openfunction/)).                                                                                                                                                                               | Architecturally honest (it composes the substrate instead of reimplementing it) but small and quiet relative to Knative itself. Adopt the substrate directly unless the FaaS-shaped DX is specifically wanted. |

**Are they the same thing?** No. OpenFaaS predates and ignores Knative; its unit is
"a container with a watchdog, behind my gateway." OpenFunction's unit is "a function
that becomes a Knative/KEDA workload." They solve the same user problem with
opposite instincts — product vs composition.

**Are either actually fast?** Drew's memory is correct about the mechanism. A
scale-to-zero FaaS request pays: gateway hop → activator/queue-proxy hop → pod
schedule → image pull → runtime init → your handler. On Kubernetes that is routinely
seconds cold, and the mitigations are all ways of not being cold: min-instances /
warm pools (you pay for idle capacity — the thing scale-to-zero was meant to
eliminate), or tiny artifacts with lazy pulling (§1), or Wasm functions via Spin
(§4), where cold start stops being the dominant term. Warm, both are as fast as the
container behind them minus a proxy hop. The honest summary: FaaS on Kubernetes
trades latency for utilisation; whether that's a good trade depends on how spiky
and how idle the workload is, and for our own services (long-lived, ours) it
usually isn't one we need to make.

**What serverless devs actually do differently:** less than the marketing suggests.
Stateless handlers, externalised state (queues, object storage, managed DBs),
event-driven triggers instead of cron-and-daemons, idempotency because delivery is
at-least-once, and small deploy units with short timeouts. Every one of those is
adoptable with plain Deployments + KEDA + a queue (NATS — see the messaging lane);
the FaaS wrapper is optional packaging.

**Should we use it:** KEDA early (it improves ordinary workloads), Knative Serving
when scale-to-zero HTTP services become a real pattern, neither FaaS wrapper now.
faasd is a legitimate single-box experiment if Drew wants the OpenFaaS feel without
a cluster — with the licensing caveat understood.

## 6. Developer-loop and build plumbing

| Project                                      | CNCF status                   | What it is                                                                                                                                                                                                                                                         | Maturity verdict                                                                                                                                                                                                                                                                                                          |
| -------------------------------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Telepresence](https://telepresence.io/)     | **Sandbox** (Ambassador Labs) | Local development against a remote cluster: a traffic manager in the cluster plus an agent sidecar lets a service running _on your laptop_ receive the cluster service's traffic (intercept, replace, wiretap modes) and resolve cluster DNS/services as if local. | Exists because "rebuild image → push → redeploy → discover the bug" is a miserable inner loop for microservices, and running the whole stack locally stops scaling. Solves a real pain the day our cluster has services worth intercepting; pointless before.                                                             |
| [BuildKit](https://github.com/moby/buildkit) | Not CNCF (Moby/Docker)        | The build engine inside modern Docker (`docker build` _is_ BuildKit), also runnable standalone as `buildkitd` with `buildx`/`buildctl` frontends: DAG-based layer builds, cache mounts, secrets mounts, multi-platform output.                                     | Already in our life wherever a Dockerfile is built. For Nix-built images it's irrelevant (Nix is the build system); it matters when consuming third-party Dockerfiles — which the Cozystack/Helm ecosystem will occasionally force — and as the conversion engine behind tools like urunc's `bunny`. No decision to make. |

## Bottom line for this slice

1. **Runtime:** containerd + runc by default (inherited), crun as a drop-in if
   density ever measures out, gVisor/Kata RuntimeClasses reserved for untrusted or
   multi-tenant workloads.
2. **VMs:** KubeVirt via Cozystack, when VMs arrive. It just graduated; it's the
   only game in town and it's a good one.
3. **Startup performance:** eStargz lazy pulling is the lever with real measured
   wins and a safe fallback; runtime-swapping is not.
4. **Wasm:** design id apps to the Component Model/WASI, run them on Wasmtime/Spin;
   WasmEdge only for its specific host-function niches. Keep containers as the
   general-purpose path.
5. **Serverless:** KEDA yes, Knative when needed, FaaS wrappers no — and OpenFaaS's
   open-core licensing is the HashiCorp pattern in miniature.
6. **Watch list:** VM API (provider-neutral VM contracts), urunc (unikernels as
   OCI), Kuasar (multi-sandbox nodes). None are adoption candidates today.
