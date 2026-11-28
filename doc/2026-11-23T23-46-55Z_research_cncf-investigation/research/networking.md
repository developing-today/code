# CNCF Networking Landscape — the slice that matters for our platform

_Research lane CNCF-NET, 2026-11-23. Covers 21 projects/initiatives across pod networking (CNI), cluster DNS, load balancing, ingress / API gateways, service mesh, and egress — plus two items that turned out not to be networking at all. Every project is labelled with its CNCF status (graduated / incubating / sandbox / not CNCF) and a maturity verdict. "What it is" is kept separate from "should we use it," and the second half is always judged against our actual estate: self-hosted Kubernetes on Talos Linux, bare metal on Drew's own LAN with router-assigned address space, Cozystack in the picture as the platform layer, and iroh handling peer-to-peer connectivity at a completely different layer (see §6 for where they meet — mostly, they don't)._

---

## TL;DR — the headline answers

| Question                                  | Answer                                                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Do we need Cilium?                        | **Yes, almost certainly — and with Cozystack you get it whether you ask or not.** Cozystack's default stack runs Cilium in chained mode for policy enforcement and kube-proxy replacement on top of Kube-OVN. On a hand-rolled Talos cluster, Cilium is the default recommendation: it replaces flannel + kube-proxy + (optionally) your ingress and LoadBalancer plumbing with one eBPF dataplane.         |
| Do we need a service mesh?                | **Not on day one, probably not for a long while.** Cilium's identity-based NetworkPolicy + Hubble covers the "who can talk to whom, and show me the flows" need. Add a mesh only when you need per-request L7 policy, automatic mTLS with workload identity everywhere, or canary routing _between services inside the cluster_. If that day comes: Linkerd for simplicity, Istio (ambient mode) for depth. |
| What replaces ingress-nginx?              | **Gateway API is the standard now; the implementation is a separate choice.** ingress-nginx was retired in March 2026. Envoy Gateway is the vendor-neutral default implementation; Cilium itself is also a conformant Gateway API implementation, which makes it the zero-new-components option on a Cilium cluster.                                                                                        |
| Is MetalLB what I remember?               | **Yes, exactly.** It hands `LoadBalancer` services real IPs from a pool you carve out of your LAN and announces them by ARP/NDP (L2) or BGP. It is the thing that made your Talos cluster's services reachable at router-range addresses.                                                                                                                                                                   |
| What about APISIX vs an "NGINX operator"? | APISIX is a full API gateway (Apache Software Foundation, **not CNCF**) — pick it when you want gateway features (auth plugins, rate limiting, API management) as a product. The old community ingress-nginx is dead; F5's separate NGINX Ingress Controller / NGINX Gateway Fabric lives on, but the centre of gravity has moved to Gateway API implementations.                                           |
| What is "egress" here?                    | A _direction_, not a product: controlling and pinning **outbound** traffic from the cluster (fixed source IPs, allow-listed destinations). Cilium has an Egress Gateway feature; Istio has egress gateways; Gateway API is growing egress support. It does not compete with NATS or APISIX (see §4).                                                                                                        |
| What is Drasi doing in a networking list? | Nothing — it's a **data-change processing platform** (Microsoft, CNCF sandbox) that sits in the CNCF landscape's Streaming & Messaging category. It wandered into the networking conversation by landscape adjacency. It belongs in the messaging/data lane (§5).                                                                                                                                           |
| What is "Cloud Native Network"?           | Most plausibly the CNCF's **CNF (Cloud Native Network Function)** telecom initiative — a working group, best-practices documents, and a test suite, not a product you install. Relevant only if you ever ship software _to telcos_. Marked as an identification, not a certainty (§5).                                                                                                                      |

---

## 1. Pod networking — the CNI layer

The CNI (Container Network Interface) plugin gives every pod an IP and makes pod-to-pod traffic work across nodes. The CNI _specification_ itself is a CNCF incubating project; the implementations below are what you actually choose between.

### 1.1 Cilium — CNCF **graduated** (Oct 2023)

**What it is.** A networking, security, and observability platform built on eBPF — small sandboxed programs the Linux kernel runs in its packet path. Instead of programming iptables rules (which degrade as service counts grow), Cilium installs eBPF programs that implement pod networking, service load balancing, and network policy by _workload identity_ rather than by IP address. It graduated from the CNCF in October 2023, having been adopted by well over 100 organisations at that point, and it is the CNI underneath GKE Dataplane V2, Azure's CNI, and AKS/EKS offerings ([CNCF graduation announcement](https://www.cncf.io/announcements/2023/10/11/cloud-native-computing-foundation-announces-cilium-graduation/); [Wikipedia overview](<https://en.wikipedia.org/wiki/Cilium_(computing)>)). Current release line: 1.20.x (1.20.4, Oct 2026).

The pieces that matter:

- **CNI dataplane** — flat L3 pod network, overlay (VXLAN/Geneve) or native-routing modes.
- **kube-proxy replacement** — Cilium can take over Kubernetes Service load balancing entirely in eBPF. On Talos this is the standard pattern: disable kube-proxy and flannel, let Cilium own both jobs.
- **Hubble** — the observability sub-project: identity-aware L3/L4/L7 flow logs, DNS visibility, a live service-dependency map, and Prometheus/OpenTelemetry metrics. This is the single best answer to "what is talking to what in my cluster" that exists in the open-source world.
- **Tetragon** — the sibling sub-project for runtime security observability and enforcement (process/file/network events from eBPF). Adjacent lane, same family.
- **Cilium Cluster Mesh** — multi-cluster connectivity: services discoverable and reachable across clusters with identity preserved.
- **Sidecarless "mesh" features** — Cilium markets L7 policy and mutual authentication without per-pod proxies; recent releases add ztunnel-based transparent mTLS in beta. Real, but younger than the CNI core (see §3).
- **Gateway API implementation + LB-IPAM + L2 announcements** — Cilium can also serve as your Gateway API controller and can announce LoadBalancer service IPs on the local network itself, which can eliminate MetalLB on simple L2 setups (see §3).
- Created by Isovalent; Cisco completed its acquisition of Isovalent in April 2024. Governance remains CNCF; the code remains Apache-2.0.

**Should we use it?** Yes. It is the closest thing the ecosystem has to a consensus default for new clusters, and for our estate it collapses three or four separate components (CNI, kube-proxy, network policy engine, flow observability, optionally ingress and LB announcement) into one well-maintained dataplane. The costs are real but bounded: eBPF debugging is a skill unto itself (their `pwru` packet-tracing tool helps), and it wants a reasonably modern kernel — a non-issue on Talos. **The Cozystack wrinkle:** if Cozystack is the platform, you do not pick Cilium as _the_ CNI — Cozystack's default `kubeovn-cilium` variant on Talos layers three pieces: **Kube-OVN** does pod networking and IPAM (GENEVE overlay, centralised allocation), **Cilium runs chained** (`generic-veth` mode) owning policy enforcement and service load balancing with its own policy engine replacing Kube-OVN's, and **MetalLB** does external load balancing ([Cozystack tenant-isolation write-up](https://cozystack.io/blog/2026/07/network-isolation-between-tenants-cilium-ebpf-kube-ovn-vpc/)). So "do we need Cilium?" under Cozystack answers itself: its eBPF policies are the tenant-isolation mechanism, always on, zero config.

**When the default CNI is enough instead:** a single-node or throwaway cluster, a dev cluster on k3s on a laptop, or a cluster where you will never write a NetworkPolicy and never wonder what a pod is talking to. Flannel + kube-proxy works fine there, and Cilium's advantages never get exercised. The moment you care about policy, observability, or multi-node performance, the default stops being enough.

### 1.2 Kube-OVN — CNCF **sandbox**

**What it is.** A CNI built on OVN/Open vSwitch (the OpenStack networking lineage) that brings virtual-network concepts into Kubernetes: per-namespace subnets, VPCs with virtual routers/switches, fixed IPs and MACs as custom resources, and live-migration-friendly networking for KubeVirt VMs. It entered the CNCF sandbox as, notably, the first CNI implementation project CNCF accepted ([Kube-OVN sandbox announcement](https://www.kube-ovn.io/news/kube-ovn-now-joins-cncf-as-a-sandbox-project)).

**Should we use it?** Only via Cozystack, where it is the chosen pod-network/IPAM layer and its VPC feature is how tenants get dedicated subnets for VM workloads (VPCs require the Multus-equipped `paas-full` bundle). Standalone, we would not pick it over Cilium for a greenfield cluster — its centre of gravity is OpenStack-style virtual networking and KubeVirt, which matters to us only through Cozystack's choices.

### 1.3 The rest of the CNI field (brief)

| Project     | CNCF status                          | What it is                                                                                                                              | Verdict for us                                                                                                                                                                         |
| ----------- | ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Calico**  | Not CNCF (Tigera)                    | The veteran policy-focused CNI; iptables or eBPF dataplane, BGP routing, very mature. Some advanced features are commercial-only.       | Fine, battle-tested, but no reason to prefer it over Cilium in 2026 for a new build.                                                                                                   |
| **Flannel** | Not CNCF (Rancher/SUSE)              | The simplest overlay CNI — pod connectivity, no policy engine. Ships as the default in several distros (incl. Talos upstream defaults). | The "default CNI" of §1.1. Enough until it isn't.                                                                                                                                      |
| **Antrea**  | CNCF **sandbox**                     | OVS-based CNI from the VMware/Broadcom orbit; strong policy tiers, egress SNAT policies, good Windows support.                          | Respectable; smaller community than Cilium/Calico. No pull for us. ([Antrea security self-assessment, incl. comparison table](https://hackmd.io/@antoninbas/HJCK_y3Sa))                |
| **Multus**  | Not CNCF (Kubernetes NPWG community) | A "meta-CNI" that attaches _multiple_ interfaces to one pod.                                                                            | Not a CNI choice but an add-on — needed when a pod (typically a KubeVirt VM) needs a second network: a VPC subnet, a physical LAN segment, SR-IOV. Cozystack's VPC path depends on it. |

### 1.4 CoreDNS — CNCF **graduated**

**What it is.** The cluster DNS server: every Kubernetes cluster runs it to resolve service names. Graduated early (2019) and listed among the CNCF's graduated projects ([CNCF DevStats project list](https://teststats.cncf.io)). **Should we use it?** You already do — it is not a decision, it is plumbing. The only choice is occasionally replacing upstream resolvers or adding stub domains; leave it alone.

---

## 2. Getting traffic _in_ — LoadBalancer services, VIPs, and the gateway layer

On bare metal there is no cloud provider to answer a `Service type: LoadBalancer`. Two small projects fill that hole, and then a gateway routes the HTTP/gRPC traffic once it arrives.

### 2.1 MetalLB — CNCF **sandbox** (accepted Sep 2021)

**What it is.** A load-balancer implementation for bare-metal Kubernetes. You define an `IPAddressPool` — a range of addresses you control — and MetalLB's controller assigns addresses from it to LoadBalancer services while its speaker DaemonSet announces them to your network, either at **Layer 2** (answering ARP for IPv4 / NDP for IPv6, so traffic lands on one elected node) or via **BGP** (advertising routes to your router, so traffic can spread across nodes). CNCF's own project page records it as accepted at Sandbox level on 2021-09-14 ([MetalLB | CNCF](https://www.cncf.io/projects/metallb/)). The project itself self-describes its maturity as "beta" while noting wide production use and bundling by major on-prem distributions ([MetalLB maturity page](https://metallb.io/concepts/maturity/)); it was created by Dave Anderson and is now run by a small maintainer team with no corporate sponsorship ([maintainers page](https://metallb.io/community/maintainers/)). Current release: 0.16.2 (Nov 2026); the 0.16 line made the frr-k8s BGP backend the default ([0.16.2 release notes summary](https://forum.unchainedstack.org/t/metallb-0-16-2-bare-metal-load-balancing-for-kubernetes-l2-and-bgp-announcements-frr-k8s-by-default-and-a-grpc-security-fix/756)).

**Drew's memory check — confirmed.** Yes: MetalLB is precisely "that thing from the Talos cluster that passed IPs from my router's range down to Kubernetes." You reserve a slice of your LAN (outside the router's DHCP pool, so nothing collides), MetalLB hands those addresses to services, and other machines on the LAN reach them as ordinary IPs. That is its whole job, and it does it well.

**Should we use it?** Yes for bare-metal clusters generally; **with Cozystack it's already the chosen external load balancer** in the default stack (§1.1). The one alternative worth knowing: Cilium's own LB-IPAM + L2 announcements can do the L2 half of MetalLB's job with one less component — reasonable on a hand-rolled Cilium cluster, pointless to substitute inside Cozystack where MetalLB is integrated and tested. Use BGP mode if the router supports peering and you outgrow single-node L2 announcement; L2 is the right start on a home/small LAN.

### 2.2 kube-vip — CNCF **sandbox** (accepted Jun 2021)

**What it is.** A small Go daemon that provides a **virtual IP (VIP) for the Kubernetes control plane** — one stable address for the API servers that floats to whichever control-plane node is leader (ARP mode with leader election, or BGP mode) — replacing the classic HAProxy + keepalived pair. It can _also_ advertise LoadBalancer service IPs, overlapping MetalLB's territory ([kube-vip architecture docs](https://kube-vip.io/docs/about/architecture/); [CNCF project page](https://www.cncf.io/projects/kube-vip/)). Actively maintained (regular releases, ~23 in the last 24 months per project-health trackers).

**Should we use it?** Its control-plane VIP role is the one MetalLB doesn't cover: MetalLB serves _workload_ services; kube-vip is for the API endpoint itself. On Talos, the built-in KubePrism/local-API patterns reduce the need somewhat, but for a multi-control-plane kubeadm-style build, kube-vip is the standard lightweight answer. If Cozystack/Talos handles control-plane HA its own way, kube-vip is a tool to know rather than a component to add. Don't run kube-vip _and_ MetalLB both announcing service IPs — pick one per job.

### 2.3 Gateway API — the standard that replaced Ingress (Kubernetes SIG-Network)

**What it is.** Not a program but the **successor API** to the Ingress resource: a role-oriented model — `GatewayClass` (which implementation), `Gateway` (the listener/TLS configuration, owned by the platform team), and route objects (`HTTPRoute`, `GRPCRoute`, `TLSRoute`, and since v1.6 also `TCPRoute`/`UDPRoute` at GA) owned by application teams. The Ingress API itself is frozen (still functional, no new features). Gateway API releases through 2026 moved fast: v1.5 (Feb 2026) promoted ListenerSet, TLSRoute, and the CORS filter to Standard; **v1.6 (June 2026) promoted TCPRoute and UDPRoute to GA**, completing L4 coverage; v1.7 followed with HTTPRoute retries at GA ([Gateway API 2026 status write-up](https://dev.to/saaro_net/kubernetes-gateway-api-2026-the-ingress-successor-matures-7he)). The **GAMMA** initiative extends the same API model to service-mesh (east-west) configuration and has been part of the Standard channel since v1.1 — one API family for both edge and mesh is the long-term play. Conformance is tested per-implementation; the conformant list includes Envoy Gateway, Istio, Cilium, Traefik, Kong, NGINX Gateway Fabric, and kgateway ([gateway comparison, Nov 2026](https://www.devopstraininginstitute.com/blog/top-12-kubernetes-gateways-for-api-traffic)).

**Why it matters to us:** whatever we pick below, we should _author_ Gateway API resources, not Ingress resources and not controller-specific annotations. That keeps the choice of implementation reversible.

### 2.4 The retirement that forces the question: ingress-nginx — **dead** (retired Mar 2026)

The community `ingress-nginx` controller — for a decade the default way to get HTTP traffic into a cluster — was retired: announced Nov 2025, maintenance ceased **March 2026**, repositories read-only, no further security patches. The proximate causes were maintainer exhaustion (one or two unpaid maintainers) and the snippet-annotation security surface. Its intended in-house successor, InGate, was retired alongside it, unfinished. Existing installs keep running but accumulate unpatched CVEs ([Google Open Source Blog, "The End of an Era"](https://opensource.googleblog.com/2026/02/the-end-of-an-era-transitioning-away-from-ingress-nginx.html); [migration walkthrough with ingress2gateway 1.0](https://github.com/the-devops-daily/devops-daily/blob/HEAD/content/posts/ingress-nginx-eol-gateway-api-migration.md)). The `ingress2gateway` tool (1.0, Mar 2026) translates Ingress manifests to Gateway API resources, silently dropping the dangerous snippet/auth annotations — those need re-expression per implementation.

Two confusions to avoid: (1) the **Ingress API** is not deleted — other controllers can still serve Ingress resources; it's the community NGINX _controller_ that died. (2) **F5's** NGINX Ingress Controller and its **NGINX Gateway Fabric** (a Gateway API implementation) are different, commercially maintained products and are alive ([Broadcom/Bitnami migration guide](https://techdocs.broadcom.com/us/en/vmware-tanzu/bitnami-secure-images/bitnami-secure-images/services/bsi-doc/apps-tutorials-migrate-kubernetes-ingress-to-gateway-api-index.html)).

### 2.5 Gateway / ingress implementations compared

**Envoy Gateway** — CNCF (sub-project of **Envoy**, graduated). A control plane that turns Gateway API resources into config for stock, unpatched Envoy proxies, with a small set of policy CRDs for what the standard doesn't reach. 1.0 GA'd in 2024; current line v1.9.x tracking Gateway API v1.6, quarterly releases ([Envoy Gateway reference summary](https://kb.ekarisky.com/service-mesh/envoy-gateway/)). **Verdict:** the vendor-neutral default if you want a dedicated gateway and aren't getting one from your CNI. This is the "just the nginx operator" successor in spirit — except the thing it operates is Envoy.

**Cilium Gateway** — part of Cilium (graduated). If Cilium is your CNI, it is already a conformant Gateway API implementation (eBPF for L3/L4, embedded Envoy for L7). **Verdict:** the zero-new-components option for a hand-rolled Cilium cluster; on Cozystack, check what the platform already exposes before adding anything.

**Apache APISIX** — **not CNCF** (Apache Software Foundation top-level project). A complete API _gateway product_: NGINX/OpenResty + LuaJIT data plane, etcd-backed configuration with hot reload, 100+ plugins (auth incl. OIDC/JWT/key-auth, rate limiting, traffic shaping, observability), an Admin API, and an ingress controller that consumes Ingress, Gateway API, and its own richer CRDs ([apisix.apache.org](https://apisix.apache.org/); [APISIX gateway comparison](https://apisix.apache.org/learning-center/open-source-api-gateway-comparison/)). **Verdict:** choose APISIX when the gateway itself is a product surface — public APIs with consumers, quotas, key management — not merely "route host X to service Y." For plain cluster ingress it is more machinery (etcd, plugin model) than Gateway-API-native options. It is a legitimate, healthy project; it simply lives in Apache's house, not CNCF's.

**Traefik** — **not CNCF** (Traefik Labs). A Go reverse proxy/ingress beloved for automatic configuration discovery; conformant Gateway API implementation. **Verdict:** the simplicity pick for small clusters (and the default bundled with k3s). Loses to Envoy Gateway on standards-depth and to APISIX on gateway features, wins on "it just works."

**Also in the room:** **kgateway** (Envoy-based, the former Gloo Gateway, now CNCF sandbox) — capable, more enterprise-shaped; **Kong** (NGINX/OpenResty, its own foundation status via Kuma below) if you buy into the Kong ecosystem. Neither has a pull for our estate over the four above.

---

## 3. East–west traffic — the service mesh question

A service mesh manages traffic _between_ services inside the cluster: automatic mTLS, per-request routing and retries, L7 authorization policy, and uniform telemetry, usually via a proxy beside every workload (sidecar) or, in newer designs, a per-node proxy or the kernel itself.

**Do we need one at all? Start from no.** A mesh is the most operationally expensive layer in this document. What most small platforms actually want from a mesh — "service A may talk to service B, not to service C; encrypt traffic on the wire; show me the flows" — is already covered on our stack by Cilium NetworkPolicy (identity-based, L3/L4 and some L7) plus Hubble's flow visibility, with TLS between internet-facing things terminated at the gateway. The needs that genuinely require a mesh are narrower: per-request L7 authorization _inside_ the cluster, automatic mTLS with SPIFFE-style workload identity for every hop, sophisticated in-cluster traffic management (header-based canaries, mirroring, fault injection), or a compliance story that demands them. None of those are day-one needs for our estate, and every mesh below can be added later — GAMMA (§2.3) even means the config model will look familiar when it happens.

### 3.1 Linkerd — CNCF **graduated** (Jul 2021, the first mesh to graduate)

**What it is.** The simplicity-first mesh. Its data plane is a purpose-built **Rust micro-proxy** (`linkerd2-proxy`) injected as a sidecar — dramatically smaller and faster than an Envoy sidecar (vendor benchmarks put it at ~18 MB max memory at 2,000 RPS vs ~155 MB for a contemporary Istio sidecar) — and the feature set is deliberately narrow: automatic mTLS, golden-metrics telemetry, retries/timeouts, and policy, with almost no knobs you don't need ([Linkerd profile](https://www.mrvitej.in/devops/cncf-linkerd/); [mesh comparison](https://kb.ekarisky.com/service-mesh/comparisons/service-mesh-comparison/)).

**The governance/funding state you asked about — it changed, and it's the thing to know.** Linkerd remains Apache-2.0 and CNCF-graduated, and development is overwhelmingly funded by one company, Buoyant. Since **Linkerd 2.15 (Feb 2024)** the open-source project publishes only weekly **edge** releases; _stable_ release artifacts come from Buoyant Enterprise for Linkerd, which is free for organisations under 50 employees and paid above that. Buoyant's position — argued at length in their licensing explainer — is that the code never stopped being open; what moved behind the business model is the production of supported stable binaries, and the revenue now funds full-time engineering (recent funded work includes VM mesh expansion, post-quantum crypto on by default in 2.19, and a 2024 security audit) ([Buoyant licensing explainer](https://www.buoyant.io/articles/how-linkerd-licensing-actually-works-apache-2-0-edge-releases-and-what-bel-pays-for)). Current line: 2.20 (Jun 2026).

**Should we use it?** If we adopt a mesh for a small estate, this is the one whose operational shape fits a one-person platform team: install it, mTLS and metrics happen, almost nothing to tune. The caveat is structural, not technical: your stable builds ride on a single small vendor's business model. For a sub-50-person operation the enterprise tier is free anyway, and the Apache-2.0 source + free edge releases are a real exit hedge. Verdict: **first choice if a mesh is ever needed.**

### 3.2 Istio — CNCF **graduated** (Jul 2023)

**What it is.** The maximal mesh: Envoy sidecars, the deepest traffic-management and authorization feature set in the field, multi-cluster and VM support, and a control plane (`istiod`) of corresponding weight. The historically decisive objection — sidecar cost and complexity — was substantially answered by **ambient mode, GA since Istio 1.24 (Nov 2024)**: a per-node Rust proxy (**ztunnel**) provides L4 mTLS, identity, and telemetry for every workload with no per-pod proxy, and optional per-namespace **waypoint** proxies (Envoy) add L7 features only where wanted. Sidecar and ambient modes interoperate. Current line: 1.32 (Nov 2026), quarterly releases with a strict support window — upgrades are not optional for long on this project ([Istio ambient GA coverage](https://www.xenonstack.com/insights/istio-service-mesh-architrcture); [mesh comparison](https://www.devopstraininginstitute.com/blog/which-service-mesh-tools-are-leading-the-future-of-cloud-native-networking)).

**Should we use it?** Only if we outgrow Linkerd's deliberate narrowness — e.g., we need Istio's fine-grained `AuthorizationPolicy`, JWT request authentication, multi-cluster topologies, or its Gateway API implementation doubling as our edge. Ambient mode lowered the entry cost a lot (start L4-only, add waypoints per namespace), so "Istio = unmanageable" is out of date; but for our scale it remains the most machinery for the least marginal benefit. Verdict: **the depth option; not the default.**

### 3.3 Kuma — CNCF **sandbox** (since Jun 2020 — _not_ incubating)

**What it is.** Kong's mesh: a Go control plane driving Envoy sidecars, designed as a _universal_ mesh — one control plane spanning Kubernetes _and_ VMs/bare-metal workloads, with multi-zone (multi-cluster/multi-datacentre) federation built in. Kong donated it to the CNCF as a sandbox project in June 2020 ([Kong donation announcement](https://konghq.com/company/press-room/press-release/kong-inc-donates-open-source-kuma-cloud-native-computing-foundation)), and the honest maturity flag is that it has **never been promoted past sandbox** in six years; the commercial superset is Kong Mesh ([Kuma profile](https://www.mrvitej.in/devops/cncf-kuma/)).

**"Is Kuma the service mesh for me?"** Its one genuinely distinctive strength is the K8s+VM single-mesh story. Our estate does run VMs — but through KubeVirt _inside_ Kubernetes (Cozystack), where the VM is already a pod on the pod network, so Kuma's universal mode solves a problem we don't have. For a pure-Kubernetes estate it offers nothing over Linkerd/Istio to offset sandbox-tier community size. Verdict: **no — right answer for hybrid VM estates, not for ours.**

### 3.4 Aeraki Mesh — CNCF **sandbox**

**What it is.** Not a mesh. **Aeraki is an add-on management plane for Istio** (created by Huabing Zhao; primarily maintained by Tetrate; CNCF sandbox since Jun 2022) that extends Istio/Envoy to manage **non-HTTP Layer-7 protocols** — Dubbo, Thrift, Redis, Kafka, MySQL, and arbitrary RPC via its MetaProtocol framework. Stock Istio treats those as opaque TCP: no request-level routing, rate limiting, or telemetry. Aeraki translates high-level rules into EnvoyFilter configuration, supplies a general dynamic-routing (RDS) service for MetaProtocol proxies, and lets you add a custom protocol by writing a codec instead of a C++ Envoy filter ([Aeraki overview](http://rawkode.academy/technology/aerakimesh); [project README](https://github.com/aeraki-mesh/aeraki)).

**Why would anyone choose Aeraki over the popular ones? They wouldn't — that's the wrong frame, and the project says so itself**: Aeraki only handles non-HTTP L7 traffic and deliberately leaves HTTP to the mesh underneath. You choose it _in addition to_ Istio, on the day you run serious Dubbo/Thrift estates (its heartland is large Chinese microservice stacks) or need L7 governance of Redis/Kafka traffic through the mesh. **For us:** our non-HTTP traffic (NATS, databases) is infrastructure we operate directly, not mesh-managed microservice RPC. Verdict: **know what it is; no use case here.**

### 3.5 Honourable mentions in mesh-land

- **Cilium's built-in mesh features** (§1.1) — L7 policy and (beta) ztunnel mTLS without sidecars. If our "mesh need" stops at encrypted, policy-checked service traffic, this may be the whole answer, at zero additional components.
- **Kmesh** — CNCF sandbox, kernel-native (eBPF) mesh from the Huawei orbit; interesting, young.
- **HashiCorp Consul** — a real mesh, but BSL-licensed and now an IBM product post-acquisition; off our open-source shortlist by the same logic that retired the rest of the HashiStack.
- **Open Service Mesh** — archived by the CNCF. Do not start anything on it.

---

## 4. Egress — the outbound direction

**What "egress" means in this landscape.** Not a project: the problem of controlling traffic _leaving_ the cluster. Three concrete needs recur: (1) **stable source IPs** — an external system (a bank API, a partner firewall, a cloud database) allow-lists your cluster, so pods must egress through a fixed address instead of whatever node they landed on; (2) **destination control** — only approved external endpoints may be reached, enforced in one place; (3) **visibility** — an audit trail of what left.

Where it shows up:

- **Cilium Egress Gateway** — a Cilium feature that steers egress traffic for selected pods through designated gateway nodes so it leaves with a stable source IP ([Cilium-on-k3s egress walkthrough](https://gist.github.com/yankcrime/2e4d7842425c72a20fd2a6cb17e1ff53)). Combined with Cilium's FQDN-aware policy (`toFQDNs`), this covers needs (1) and (2) for most estates.
- **Istio egress gateways** — route outbound traffic through dedicated Envoy egress proxies for central policy/logging; the mesh-priced version of the same idea.
- **Gateway API** — egress is arriving in the standard itself: v1.6 added an experimental `Backend`/XBackend concept for external targets, with egress on the SIG's roadmap — expect this to become the portable way to express it ([Gateway API 2026 status](https://dev.to/saaro_net/kubernetes-gateway-api-2026-the-ingress-successor-matures-7he)).
- **Antrea Egress** — SNAT policies per workload group, same idea in the OVS world.

**How does egress compare to APISIX?** APISIX is a _north–south API gateway_: it sits at the edge managing inbound API traffic (and can proxy outbound calls as one feature among many). Egress control is a _cluster networking function_: source-IP pinning and destination policy for everything pods initiate. They overlap at exactly one point — "an HTTP call leaves through a managed proxy" — and differ everywhere else. You don't choose between them; you might run both for different jobs.

**Does egress compete with NATS? No — different layers entirely.** NATS is application-level messaging _inside_ your system design (services publishing/subscribing). Egress is IP-level plumbing _underneath_ everything, including underneath NATS clients reaching an external broker. The two words co-occur in architecture diagrams and nowhere else.

**Should we use it?** Not preemptively. The day an external dependency demands a fixed source IP, Cilium Egress Gateway is a feature flag on infrastructure we already run — that's the right order of operations.

---

## 5. Misfiled and mystery items

### 5.1 Drasi — CNCF **sandbox** (Jan 2025) — _not a networking tool_

Drasi is a **change-data-processing platform** from Microsoft's Azure Incubations team: you register _sources_ (PostgreSQL/MySQL change feeds, Cosmos DB, Kubernetes resources, Event Hubs), write _continuous queries_ in Cypher that are kept incrementally up to date as data changes, and attach _reactions_ (webhooks, queues, SignalR dashboards, stored procedures, an MCP reaction) that fire when the query's result set gains, loses, or changes a row. It exists so you don't hand-build the poll-the-database / CDC-pipeline / stream-processing chain for "tell me when an order sits unprocessed for an hour." It is written substantially in Rust, runs on Kubernetes, as a standalone server, or embedded as a library, and entered the CNCF sandbox on 2025-01-21 ([Drasi deep-dive](https://www.mrvitej.in/devops/cncf-drasi/); [rawkode profile](http://rawkode.academy/technology/drasi)).

**Why it appeared next to networking projects:** the CNCF landscape files it under _App Definition & Development → Streaming & Messaging_, and any alphabetical or category-adjacent browse of the landscape puts it a few tiles away from networking entries. It belongs in the **messaging/data lane**, next to NATS and Debezium comparisons — where, for the record, its interesting question is whether continuous-query-reaction is a better fit than hand-rolled NATS consumers for state-change detection. It has no interaction with APISIX or egress (§4).

### 5.2 "Cloud Native Network" / CNN — identification, honestly uncertain

There is no CNCF project product called "Cloud Native Network." The most plausible referent for what Drew saw is the CNCF's **CNF initiative**: a **Cloud Native Network Function** is the telecom industry's term for a network function (a firewall, a 5G core component, a load balancer) built the cloud-native way, and CNCF ran a whole programme around it — the **CNF Working Group** (best practices for building CNFs), the **CNF Test Suite** (an open-source checker that scores a network application against cloud-native principles: declarative APIs, immutable infrastructure, observability, resilience), a Telecom User Group, and a CNF Certification programme ([CNCF telco initiatives overview](https://www.cncf.io/blog/2021/12/08/people-processes-and-tools-for-the-cloud-native-telco/); [CNF Test Suite introduction](https://www.cncf.io/blog/2022/03/24/testing-cloud-native-best-practices-with-the-cnf-test-suite/)). The LF Networking asset list notes the Telecom User Group as currently inactive and the testbed inactive; the working group and test suite are the durable artefacts.

**Verdict:** this is telco-industry process machinery, not infrastructure. It matters to us only if we ever ship a network function _to_ a carrier. If "CNN" was something else — a landscape category label ("Cloud Native Network" is also how some landscape views title the networking category itself) — that reading is equally consistent with what was seen; either way there is no installable project being missed. Flagged as an identification with residual uncertainty, not a finding.

---

## 6. Where cluster networking meets iroh (mostly: it doesn't)

These are different layers solving different problems, and keeping them apart is a design virtue:

- **Cluster networking (this document)** assumes a platform: nodes you control, a scheduler, IPAM, DNS. It moves traffic between _workloads on infrastructure you operate_.
- **iroh** assumes no platform: two endpoints with keys find each other across hostile NATs and talk peer-to-peer, with relays as fallback. It moves traffic between _identities on infrastructure nobody operates_.

The meeting points are thin and specific: (1) an iroh endpoint can run _inside_ a pod — then the cluster network is just its underlay, and iroh's relay traffic is ordinary egress (§4) as far as Cilium is concerned; (2) if we ever expose a cluster service to iroh peers, the seam is an application-level gateway we write, not a CNI or mesh feature — no project in this document bridges the two, and none should be bought expecting to. **The load-balancing/relay similarity is verbal, not architectural:** MetalLB announces IPs on a LAN you own; iroh relays forward encrypted streams between keys on the open internet. Do not let the shared vocabulary ("relay", "gateway", "mesh") blur them.

---

## 7. Recommendations for our estate, in order

1. **CNI:** Cilium — via Cozystack's stack (Kube-OVN + Cilium chained + MetalLB) if Cozystack is the platform; standalone Cilium with kube-proxy replacement if hand-rolling on Talos. Enable Hubble from day one; it pays for the whole decision the first time something's traffic is a mystery.
2. **LoadBalancer services:** MetalLB, L2 mode on a reserved LAN range outside DHCP; BGP later if the router and scale justify it. kube-vip only if a hand-built multi-control-plane cluster needs an API VIP that Talos patterns don't already cover.
3. **Ingress:** author **Gateway API** resources from the start. Implementation: Cilium's built-in Gateway support or Envoy Gateway. Do not install ingress-nginx anywhere new (retired, unpatched); if APISIX-class API-management features are ever needed, evaluate APISIX as a product decision, separately.
4. **Service mesh:** none initially. Cilium policy + Hubble first. Revisit when a named requirement appears (in-cluster L7 authz, universal mTLS identity, in-cluster canary routing) — then Linkerd, with Istio ambient as the depth option. Kuma and Aeraki: no.
5. **Egress:** nothing until an external party demands a fixed source IP; then Cilium Egress Gateway.
6. **Watch list (no action):** Gateway API's egress standardisation (v1.7+); Cilium's ztunnel mTLS maturing out of beta (it could retire the mesh question for our scale entirely); Linkerd's stable-release model (the Buoyant bet is working so far — check again if we adopt).

---

## Appendix — project index

| Project                     | CNCF status                   | Layer                                       | One-line verdict                              |
| --------------------------- | ----------------------------- | ------------------------------------------- | --------------------------------------------- |
| Cilium (+ Hubble, Tetragon) | Graduated                     | CNI / policy / observability                | **Adopt** — the default                       |
| Kube-OVN                    | Sandbox                       | CNI (OVN/VPC)                               | Adopt only as part of Cozystack               |
| Calico                      | Not CNCF                      | CNI                                         | Mature alternative; no pull                   |
| Flannel                     | Not CNCF                      | CNI                                         | Fine until policy/observability matter        |
| Antrea                      | Sandbox                       | CNI (OVS)                                   | Respectable; no pull                          |
| Multus                      | Not CNCF                      | Meta-CNI                                    | Add-on for multi-interface pods/VMs           |
| CoreDNS                     | Graduated                     | Cluster DNS                                 | Plumbing; already running                     |
| MetalLB                     | Sandbox                       | LoadBalancer (L2/BGP)                       | **Adopt** — Drew's memory confirmed           |
| kube-vip                    | Sandbox                       | Control-plane VIP / LB                      | Know it; use for hand-built HA control planes |
| Gateway API (+ GAMMA)       | K8s SIG standard              | Ingress/mesh API                            | **Adopt as the authoring model**              |
| Envoy                       | Graduated                     | Proxy data plane                            | The substrate under half this list            |
| Envoy Gateway               | Graduated (Envoy sub-project) | Gateway API implementation                  | **Default dedicated gateway**                 |
| ingress-nginx               | Retired (was community)       | Ingress controller                          | **Dead — migrate off, never install**         |
| NGINX Gateway Fabric (F5)   | Not CNCF                      | Gateway API implementation                  | Alive; the NGINX-shop path                    |
| Apache APISIX               | Not CNCF (Apache TLP)         | API gateway product                         | Only for API-management needs                 |
| Traefik                     | Not CNCF                      | Ingress/gateway                             | Simplicity pick for small clusters            |
| Linkerd                     | Graduated                     | Service mesh                                | **First choice if a mesh is needed**          |
| Istio                       | Graduated                     | Service mesh                                | Depth option (ambient mode GA)                |
| Kuma                        | Sandbox                       | Service mesh (K8s+VM)                       | For hybrid VM estates; not ours               |
| Aeraki Mesh                 | Sandbox                       | Istio add-on, non-HTTP L7                   | Not a mesh; no use case here                  |
| Drasi                       | Sandbox                       | **Not networking** — change-data processing | Belongs in the messaging/data lane            |

_Sources are linked inline at each claim. Statuses verified against CNCF project pages and project sources on 2026-11-23; where a project's self-description and foundation listing differ (MetalLB's "beta" self-label vs its CNCF sandbox listing), both are reported._
