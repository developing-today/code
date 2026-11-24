# Research: the CNCF landscape — the slice this estate will actually need

This bundle is the research record for Drew's cloud-native landscape review
(2026-11-23): every major CNCF project in the areas this estate cares about —
including incubating, sandbox, proposed, and non-CNCF-but-same-shaped projects —
surveyed from primary sources, with each project's CNCF status
(graduated / incubating / sandbox / not CNCF) and maturity labelled, "what it
is" kept separate from "should we use it," and every named question Drew asked
answered in the lane that owns it. Roughly 150 projects are covered across the
seven pages below.

## The estate the verdicts were judged against

NixOS hosts today; Talos-based Kubernetes planned; Cozystack as the platform
layer on that Kubernetes; bare metal on Drew's own LAN with router-assigned
address space; images built with Nix, CI on GitHub Actions; Postgres-first
data posture (Postgres in services, Turso/SQLite in Atlas); DRBD/LINSTOR
storage under Cozystack; the fleet / id / atlas / mcpx projects as workloads,
including a fleet CA program and secrets broker we built ourselves — several
famous projects below compete with things we own, and that is said out loud
where it is true. Small team, self-hosted, performance-sensitive (performance
claims must be benchmark-proven, never asserted), no enterprise-compliance
requirements. The old HashiCorp stack (Vault / Consul / Terraform) is closed
source now (BSL) and is being routed around, not renewed.

## The landscape map

| Area                              | Page                                                  | The short shape                                                                                                                                                                                      |
| --------------------------------- | ----------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Observability & operations        | [observability](observability.md)                     | OpenTelemetry as the instrumentation contract; Prometheus as the metrics _language_, VictoriaMetrics as the store Cozystack already runs; Grafana the dashboard layer; no open-source Datadog exists |
| Networking                        | [networking](networking.md)                           | Cilium (eBPF) as the CNI; MetalLB for LAN LoadBalancer IPs; Gateway API as the ingress standard after ingress-nginx's retirement; no service mesh yet                                                |
| Platform, GitOps & control planes | [platform-and-gitops](platform-and-gitops.md)         | Cozystack as the tenant-cloud layer; Flux as the GitOps reconciler; Crossplane understood and declined for now                                                                                       |
| Data layer                        | [data-layer](data-layer.md)                           | CloudNativePG Postgres; NATS for messaging/streaming; LINSTOR/DRBD storage; TiKV and the distributed-SQL field declined                                                                              |
| Security, identity & secrets      | [security-and-identity](security-and-identity.md)     | cert-manager + Kyverno + cosign + External Secrets Operator when Kubernetes lands; our CA and broker keep their jobs; Keycloak and OpenBao are different tools, neither replaces the other           |
| Runtimes & serverless             | [runtimes-and-serverless](runtimes-and-serverless.md) | containerd + runc by default; KubeVirt for VMs; Wasm apps designed to the Component Model; FaaS wrappers declined                                                                                    |
| Registries & builds               | [registries-and-builds](registries-and-builds.md)     | GHCR until friction (not price) says otherwise, then Zot; Nix + Actions already win the build/app-definition category                                                                                |

## Headline verdicts, per area

**Observability.** Prometheus stays — mostly as a language and data model, not
always as the server: inside Cozystack the metrics store is VictoriaMetrics
(PromQL-style surface, less RAM); outside Kubernetes a standalone Prometheus
3.x is still the simplest correct thing. Grafana stays the dashboard layer
(it and the LGTM stack are Grafana Labs' vendor stack, not CNCF — AGPL,
open-core gravity toward Grafana Cloud; Perses is the CNCF watch item).
**There is no open-source Datadog**: the honest options are the assembled
OTel + metrics/logs/traces stack, or SigNoz as the closest single-product
attempt (pre-1.0, open-core). OpenTracing is finished — OpenTelemetry
absorbed it; Jaeger v2 or Tempo for traces, chosen by UI gravity. OpenFeature

- flagd for feature flags. Pixie stalled; Trickster is a niche cache;
  HolmesGPT earns a bounded read-only trial only.

**Networking.** **Cilium: yes** — with Cozystack it arrives whether asked or
not (chained with Kube-OVN); hand-rolled on Talos it is the default
recommendation (eBPF dataplane, kube-proxy replacement, Hubble flow
visibility from day one). **Service mesh: not yet** — Cilium policy + Hubble
covers the need; add one only for per-request L7 policy, universal mTLS
identity, or in-cluster canary routing (then Linkerd for simplicity, Istio
ambient for depth; Kuma and Aeraki: no). **Gateway API is the ingress
standard now** — ingress-nginx was retired in March 2026; the implementation
is a separate choice (Cilium's own Gateway support, or Envoy Gateway).
APISIX is a full API gateway (Apache Foundation, not CNCF) — a product
decision, not an ingress default. **MetalLB is exactly what Drew remembers**:
it hands LoadBalancer services real IPs from a pool carved out of the LAN and
announces them by ARP/NDP or BGP — the thing that made the Talos cluster
reachable at router-range addresses. Egress is a _direction_ (pinning and
allow-listing outbound traffic), not a product, and it competes with neither
APISIX nor NATS.

**Platform & GitOps.** **Flux over Argo CD for us** — Flux is the reconciler
(already inside Cozystack); Argo CD only if a deployment UI for other humans
becomes a requirement. **Cozystack fits** — trial it as the tenant-cloud
layer on Talos — **with the caveat stated plainly: it decides storage,
tenancy, and GitOps for you**, so adopting it is adopting its opinions.
**Crossplane: not now** — it is a real infrastructure control plane
(Kubernetes APIs for cloud resources), but Nix + Cozystack + fleet already
cover its ground here; revisit when a real cloud-API surface appears. kcp is
a watch item (control planes without workloads, for SaaS-scale API tenancy).
K3s is the laptop/dev/CI cluster, complementing Talos rather than competing
with it. OpenTofu is indeed the Terraform replacement (Linux Foundation;
also a CNCF sandbox project) — the default _if_ HCL is ever needed; Atlantis
is the PR front-end for an OpenTofu estate we don't have. Cadence (now CNCF
sandbox) is durable execution inside a service — a different layer from
fleet's queue, not a competitor. Backstage is overkill at our scale.

**Data layer.** **TiKV: no.** It is a genuinely good, honestly Jepsen-tested
system — it does not "totally fail Jepsen"; its history is the instructive
part — but it is a _storage layer for a distributed SQL database_, not a
database you adopt alone, and its SQL layer (TiDB) speaks MySQL, not
Postgres. It cannot replace CockroachDB for us (and CockroachDB itself left
open source in 2024). Vitess is the same shape for MySQL fleets; we are
Postgres-first. **The Postgres answer is CloudNativePG + streaming
replication and read replicas** — the operator to standardise on, and the
one Cozystack already ships. **NATS: yes, the plan is right, with a shape** —
core NATS for service communication and request/reply, JetStream for durable
streams and work queues, JetStream KV for small watched state; do not treat
JetStream as a Kafka-class infinite log. Kafka (or Redpanda for the same
API) only if a true log workload appears. **Rook: no** — Rook means running
Ceph; **LINSTOR/DRBD already fills the role** at our scale (latency,
footprint, operational simplicity), which is why Cozystack ships it. Add
Rook only if a specifically Ceph-shaped need (very large scale, native
object storage, ReadWriteMany files) appears.

**Security & identity.** **Keycloak ≠ OpenBao — completely different
things**: Keycloak is an identity provider (people logging in); OpenBao is
Vault's MPL fork (secrets storage/delivery). Keycloak competes with _our_
IdP and beats it on breadth while losing on footprint and fit; OpenBao
competes with _our secrets broker_ and loses at our current shape (dynamic
secrets are its real advantage, for problems we don't yet have). **cert-manager:
yes when Kubernetes lands** — in-cluster issuance/renewal is solved; it
composes _under_ fleet cert rather than replacing it. **OPA: management
plane only** — capabilities and workload definitions in our systems are
cryptographic objects, not policy evaluations; in-cluster, Kyverno is the
likelier admission-policy pick. **SPIFFE/SPIRE: the ID format and
short-TTL discipline are already adopted; SPIRE itself comes later, chained
under the fleet CA**, only when stored-secret workload identity becomes the
actual problem. OpenFGA (Zanzibar-style relationship authorisation): nothing
we run needs ReBAC yet. Notary/Notation is the PKI-rooted artifact-signing
option; cosign keyless signing is the ecosystem default for our Actions
pipelines. SOPS is already ours — and yes, it has wider uses than a few
secrets in a repo (whole-file and structured encryption for config at rest),
with real limits stated in the page. Cartography does **not** manage
secrets — it is a read-only infrastructure asset graph; skip at our scale.
ContainerSSH does what Drew guessed (a fresh container per SSH connection);
niche, watch only.

**Runtimes & serverless.** No OCI runtime is "much faster": runc / crun /
youki differ by tens of milliseconds at process start, and perceived startup
is dominated by image pull and application init — the real levers are lazy
pulling (eStargz/Nydus) and P2P distribution (Dragonfly). Default stays
containerd + runc; gVisor/Kata RuntimeClasses reserved for untrusted
workloads. **KubeVirt is basically the one way people run VMs in Kubernetes
now** — graduated, no serious rival, and Cozystack's VM offering is KubeVirt
underneath. Telepresence exists to give a laptop process a two-way bridge
into a cluster for the dev loop; it is plumbing, not platform. OpenFaaS and
OpenFunction are not the same thing, and neither is the fast path — KEDA
yes, Knative when needed, FaaS wrappers no (OpenFaaS's open-core licensing
is the HashiCorp pattern in miniature). **Wasm: the model is right for id
apps — design to the Component Model / WASI and run on Wasmtime/Spin, not
to WasmEdge** (kept only for its specific host-function niches). youki is
the Rust OCI runtime ("Yuki"); urunc runs unikernels as OCI — a watch item.
Firecracker remains the microVM reference point under Kata.

**Registries & builds.** **Do we need a registry? Not yet — GHCR until
friction, then Zot.** GitHub's container storage/bandwidth is currently
free with advance notice promised before that changes; self-hosting earns
its keep on LAN-local pulls, air-gap survival, egress at scale, and
sovereignty — none bind yet. Keep image references templated so the exit
stays a hostname change. **Harbor and Dragonfly are not alternatives**:
Harbor is a registry (where images live); Dragonfly is a P2P distribution
layer in front of a registry (so a thousand nodes don't each pull from
origin) — graduated, excellent, unnecessary at our node count. **Zot** is
the lightweight OCI-native registry that fits as the first self-hosted step.
Artifact Hub is a catalogue — a search engine for other people's charts and
packages; it stores nothing of ours. And the app-definition / image-build /
packaging category mostly dissolves under our workflow: **with Nix +
Dockerfiles + GitHub Actions we already have reproducible builds and a
registry**; the category is solving YAML-era problems we don't have. Helm
survives as a client for consuming the Cozystack ecosystem.

## Status corrections (the landscape moved in Oct–Nov 2026)

Several projects changed CNCF tier in the weeks before this research;
older notes and landscape snapshots are stale on all of these:

- **Graduated**: Crossplane (2025-11-06), KubeVirt (2026-11-04), NATS
  (2026-11-02), Dragonfly (2025-10-28) — and earlier than commonly assumed:
  cert-manager (2024-09-29) and Kyverno (2026-03-16).
- **Promoted to incubating**: CloudNativePG (2026-11-02), OpenFGA
  (2025-11-11).
- **Newly placed**: Cadence entered the CNCF sandbox in 2025 (donated by
  Uber); Atlantis is a CNCF sandbox project (accepted 2024-06-18);
  Cartography is a CNCF sandbox project now (it began at Lyft); OpenTofu
  was accepted as a CNCF sandbox project in April 2025 alongside its Linux
  Foundation governance; Agent Substrate (sandbox, Nov 2026) and AI Runway
  (sandbox, Sep 2026) are new sandbox entries for in-cluster agent
  sandboxing and self-hosted inference respectively.
- **Left or ended**: ingress-nginx retired March 2026 (unpatched — do not
  install anywhere new); OpenTracing is absorbed into OpenTelemetry;
  Pixie stalled; Tremor is archived.
- **Not CNCF at all** (frequent sources of confusion): Grafana and the LGTM
  stack (Grafana Labs), APISIX (Apache Foundation), OpenBao (OpenSSF),
  Sigstore/cosign (OpenSSF), TiDB / CockroachDB / Kafka / Redpanda (Apache
  or vendor), LINSTOR/DRBD, Calico, Kustomize (Kubernetes SIG).

## Uncertain names, identified honestly

Drew's questions named several projects from memory; five of those names
didn't resolve as remembered. The identifications (each argued in its page):

- **"Argonize" does not exist** as an infrastructure project; the nearest
  match is Argonaut, a hosted SaaS built on Argo CD — so the
  "Argonize vs Crossplane" comparison has no second term.
- **"Trimmer" → Tremor** — a Rust event _processor_ (formerly CNCF sandbox,
  now archived), never a broker; it was never a Kafka/NATS rival.
- **"Yuki" → youki** — the Rust OCI container runtime.
- **"Percy" → Pixie** — the eBPF auto-instrumentation platform; the thing
  that was supposed to feel like Datadog without agents. It stalled.
- **"Spiffy" → SPIFFE** — the workload-identity framework (with SPIRE as
  its implementation); it is not a tracing tool, so "SPIFFE or Jaeger" is
  identity _and_ tracing, not a choice between them.

Also resolved along the way: "Cloud Native Network" is most plausibly the
CNCF's CNF (Cloud Native Network Function) telecom initiative — documents
and a test suite, not a product; Drasi is a data-change processing platform
that wandered into the networking conversation by landscape adjacency;
"CubeVIP" is kube-vip (a control-plane VIP), and the "Coordinator /
Conveyor / CubeBurner" cluster of names resolves to sandbox-stage cluster
plumbing evaluated in the platform page.

## The pages

- [Observability & operations](observability.md) — 23 projects: Prometheus,
  VictoriaMetrics, Grafana and the LGTM stack, SigNoz, OpenTelemetry,
  OpenTracing, Jaeger, Pixie, Trickster, HolmesGPT, OpenCost, OpenFeature,
  Chaos Mesh, Thanos, Cortex, Fluent Bit, Perses, Kepler.
- [Networking](networking.md) — 21 projects/initiatives: Cilium, Kube-OVN,
  Calico, Flannel, MetalLB, kube-vip, Gateway API, Envoy Gateway, APISIX,
  Linkerd, Istio, Kuma, Aeraki, egress as a direction, Drasi, the CNF
  initiative, and where cluster networking meets iroh (mostly: it doesn't).
- [Platform, GitOps & control planes](platform-and-gitops.md) — 26
  projects: Cozystack, Flux, Argo family, Crossplane, kcp, Cluster API,
  Sveltos, K3s, OpenYurt, OpenTofu, Atlantis, Cadence/Temporal, Backstage,
  Helm, Kustomize, Agent Substrate, AI Runway, kagent, and the "Argonize"
  identification.
- [Data layer](data-layer.md) — TiKV, TiDB, CockroachDB, Vitess, the
  Postgres operator field (CloudNativePG, Zalando, Crunchy), Rook/Ceph vs
  LINSTOR/DRBD vs Longhorn, NATS, Kafka, Redpanda, Pulsar, Tremor, etcd,
  FoundationDB — including the full Jepsen discussion for TiKV.
- [Security, identity & secrets](security-and-identity.md) — 17 projects:
  OpenBao, Consul, Keycloak, SPIFFE/SPIRE, cert-manager, OPA/Gatekeeper,
  Kyverno, OpenFGA, Notary/Notation/Ratify, Sigstore/cosign, SOPS,
  External Secrets Operator, Cartography, Falco, ContainerSSH, Teleport,
  in-toto/SLSA — with an adoption order for when Kubernetes lands.
- [Runtimes & serverless](runtimes-and-serverless.md) — containerd, CRI-O,
  runc, crun, youki, gVisor, Kata, Firecracker, KubeVirt, the Wasm runtimes
  (Wasmtime, WasmEdge, Spin), urunc, Kuasar, OpenFaaS, OpenFunction,
  Knative, KEDA, Telepresence — with benchmark-shaped performance answers.
- [Registries & builds](registries-and-builds.md) — GHCR, Harbor,
  Dragonfly, Zot, Artifact Hub, and the application-definition /
  image-build category judged against a Nix + Dockerfiles + GitHub
  workflow.
