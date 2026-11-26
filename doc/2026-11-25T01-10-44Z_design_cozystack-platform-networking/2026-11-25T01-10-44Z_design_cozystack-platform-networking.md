# Cozystack bring-up and platform networking — design record (Phase 2)

Date: 2026-11-25. Status: design of record for the five "must produce" items of [#14017](https://github.com/developing-today/code/issues/14017), written under [#14215](https://github.com/developing-today/code/issues/14215). Fleet-drafted; it records Drew's settled directions of 2026-11-23 and does not claim his ratification of anything new. Programme: [#1817](https://github.com/developing-today/code/issues/1817), Phase 2, install epic [#1087](https://github.com/developing-today/code/issues/1087).

Execution is gated on Phase 1 (Talos nodes up, [#1086](https://github.com/developing-today/code/issues/1086)). Nothing here changes a cluster, and no address range in this record is real: every range is a rule plus the fact it waits on.

## 1. Context and inputs

- **Settled by Drew, 2026-11-23:** Cozystack is the platform; ingress is authored as Gateway API resources and ingress-nginx is never installed anywhere new; MetalLB pools come from the LAN outside DHCP, L2 first; Hubble on from day one; management access only through a Cloudflare tunnel and Tailscale. Sources: the CNCF landscape record `doc/2026-11-23T22-33-41Z_research_cncf-landscape/` (`networking.md`, `platform-and-gitops.md`, PR #13827) and the sequenced plan comment on #1817 (2026-11-24).
- **Already in the repo:** the TALM skeleton at `infra/cluster/basement/` (PR #14153, #14016 slice 1) with the cozystack preset, `image: ghcr.io/cozystack/cozystack/talos:v1.14.2`, `podSubnets: [10.244.0.0/16]`, `serviceSubnets: [10.96.0.0/16]`, an empty `endpoint`, an empty `floatingIP`, and an RFC 5737 placeholder `advertisedSubnets`. The preset's built-in kernel module list already contains `drbd` and `drbd_transport_tcp`.
- **Upstream facts read 2026-11-25** (sources in §10): Cozystack v1.7.0 is the newest stable release (published 2026-10-29); v1.6.5 (2026-10-16) is the newest v1.6 patch. Issue cozystack/cozystack#5165 (open) lists fixes deferred to v1.7.1, including a gateway release that reads a TenantGateway with a reconcile error as Current. No v1.7.1 tag was visible on 2026-11-25.
- **Open facts this design waits on:** LAN addressing and the DHCP range ([#6077](https://github.com/developing-today/code/issues/6077)); NIC, switch and VLAN facts and the first-node shortlist ([#1810](https://github.com/developing-today/code/issues/1810)); Phase 1 itself (#1086). The egress design ([#5030](https://github.com/developing-today/code/issues/5030), which absorbed the closed #1814) owns bandwidth budgets and the tunnel topology beyond what §6 needs.

## 2. Goals and non-goals

**Goals.** Name the exact install path and version policy; write down the networking stack as Cozystack ships it and the few deltas we apply; give MetalLB carving rules that can be filled in mechanically once #6077/#1810 land; design the first Gateway and routes for the dashboard and Kubernetes API, with a TLS path that works on a LAN-only gateway; and fix the management access path with no public exposure.

**Non-goals.** Running the install or touching any node; storage pool layout (#1813, Phase 3); platform services beyond reachability (#1088, Phase 4); the Phase 6 security stack (cert-manager chained to the fleet CA, Kyverno, ESO, Falco); egress budgets and failure matrix (#5030); a service mesh; BGP.

## 3. Install path and version policy (item 1)

### 3.1 Path

1. Phase 1 finishes with real node files under `infra/cluster/basement/nodes/` (from `talm template -i`), `endpoint` and `floatingIP` set (the Talos control-plane VIP, see §5.2), and the nodes applied with `talm apply`. This is the existing #14016 runbook; nothing new.
2. Install Cozystack by the upstream "install on Talos" procedure at the pinned tag: the installer manifest for that tag, then the platform `Package` (`cozystack.cozystack-platform`) carrying our values. Our values live in git beside the TALM chart, at `infra/cluster/basement/platform/` (new directory, created by build slice B1 in §9), so the platform rebuilds from git like the nodes do.
3. Platform values set at install (illustrative shape; key names are checked against the pinned tag's docs in slice B1, not trusted from this record):

```yaml
# infra/cluster/basement/platform/values.yaml (shape only)
bundle: paas-full # Talos bare-metal bundle; carries Multus for Kube-OVN VPCs
networking:
  variant: kubeovn-cilium # the shipped default; not changed in v1 (see §4.3)
  podCIDR: 10.244.0.0/16 # must equal talm podSubnets; v1.7 needs /20 or wider
  serviceCIDR: 10.96.0.0/16 # must equal talm serviceSubnets
gateway:
  enabled: true # platform services published through Gateway API, not Ingress
  className: cilium # the default GatewayClass
publishing:
  host: <apex> # open question Q3
  exposedServices: [dashboard, api]
  certificates:
    solver: dns01 # see §5.4: HTTP-01 cannot work on a LAN-only gateway
```

4. Verify with the checks in §8 before Phase 2 is called done.

### 3.2 Pin

- **Line:** Cozystack v1.7. The v1.6.4 research baseline is stale; v1.7.0 is the current stable release, its Talos requirement (v1.14.2 with `drbd_transport_tcp` loaded) matches what the TALM skeleton already pins, and v1.7 is the line where tenant Gateway API routes must declare hostnames, which we want from day one.
- **Exact tag at install time:** the newest v1.7 patch release that is at least v1.7.1. If Phase 1 finishes before v1.7.1 exists, install v1.7.0 and add one manual check to §8: read each `TenantGateway`'s status directly instead of trusting the gateway release's Ready (cozystack/cozystack#5165, item 1).
- **Where the pin lives:** one place in git, `infra/cluster/basement/platform/`, as an exact tag (never a range, never `latest`). The talm preset lock already pins Talos and Kubernetes for the node side.
- **Never pre-releases** (`-alpha`, `-rc`) on the cluster, even when an rc fixes something we want; the fix waits for the tag.

### 3.3 Upgrade policy

- **Patches inside the pinned minor:** take them after reading the release notes, within a few weeks of release, through the rehearsal below.
- **A new minor (v1.8 and on):** only after its first patch release is out and at least two weeks old, and after reading the upstream "Upgrading from vX to vY" page end to end. Its pre-flight checks become a checklist on the upgrade issue.
- **Rehearsal, every time:** (1) run the target version's pre-flight checks (for v1.7 these were: route hostnames set, API certificate SANs, free disk, VM disk imports finished, FoundationDB lines); (2) take an etcd snapshot and confirm the TALM config and Tier A secrets are recoverable (Phase 5 vault, #1090; until Phase 5 lands, the snapshot is taken by hand and stored off the cluster); (3) where a spare node or mini PC exists, upgrade a single-node throwaway cluster at the old pin first; (4) upgrade; (5) read the migration hook log (`kubectl logs -n cozy-system job/cozystack-migration-hook` on v1.7); (6) run §8 again.
- **Rollback:** Cozystack migrations are forward-only hooks, so there is no supported in-place downgrade. Rollback means restoring the pre-upgrade etcd snapshot onto nodes rebuilt from git, which is why step (2) is mandatory and why the Phase 1 teardown drill matters. While no tenant holds data that matters (all of Phase 2), the cheaper rollback is a full rebuild from git at the old pin; that is the stated rollback until Phase 5 proves a restore.
- **Rehearsals run on lab hardware.** The laptop and agent sandboxes never host a rehearsal cluster (Drew, 2026-11-23: builds and heavy jobs are dispatched remotely, never run on laptops or sandboxes).

## 4. Platform networking as shipped (item 2)

### 4.1 Roles

| Layer                          | Component                              | Job in this cluster                                                                                                                                                                                                                                                             |
| ------------------------------ | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pod network, IPAM              | Kube-OVN                               | GENEVE overlay between nodes, central IP allocation from `podCIDR`, VPC/subnet objects for tenant VMs (VPCs need Multus, carried by `paas-full`).                                                                                                                               |
| Policy, service load balancing | Cilium, chained (`generic-veth`)       | kube-proxy replacement in eBPF; identity-based NetworkPolicy; Cozystack's per-tenant isolation policies; Envoy DaemonSet for Gateway API (TLS termination and passthrough). On v1.7, Cilium also takes addresses from a reserved block inside `podCIDR`, hence the /20 minimum. |
| External load balancing        | MetalLB                                | Assigns `LoadBalancer` Service addresses from our pools and announces them on the LAN by ARP (L2). Cozystack ships MetalLB but renders no pools; the pools in §5 are ours to write.                                                                                             |
| North-south HTTP/TLS           | Cilium Gateway (GatewayClass `cilium`) | The platform Gateway and tenant Gateways, created by `cozystack-controller` from `TenantGateway` objects.                                                                                                                                                                       |
| Control-plane endpoint         | Talos Layer2 VIP (`floatingIP`)        | One floating address for the Kubernetes API across control-plane nodes. Not MetalLB.                                                                                                                                                                                            |
| Flow visibility                | Hubble (part of Cilium)                | Flow logs and service map, on from day one (§4.4).                                                                                                                                                                                                                              |

### 4.2 Tenant isolation model

Each Cozystack tenant is a `tenant-<name>` namespace tree. Isolation between tenants is Cilium eBPF policy that Cozystack renders and keeps on; we do not hand-write tenant isolation policy. Tenant VMs and tenant Kubernetes clusters can additionally get a Kube-OVN VPC with their own subnet. Hostnames are fenced by Cozystack's admission policies: a route's hostnames must equal or sit under the namespace's `namespace.cozystack.io/host` label, and that label cannot be changed after it is set. Inside a tenant, our own workloads add NetworkPolicy for declared dependencies only (P9 exposure contract, `doc/2026-11-24T16-22-01Z_design_p9_lab-integration/`).

### 4.3 What v1 deliberately does not change

- The networking variant (`kubeovn-cilium`) and the chaining mode.
- `podCIDR` 10.244.0.0/16 and `serviceCIDR` 10.96.0.0/16, unless #6077 shows the LAN or the tailnet's routed subnets overlap them; then both are moved together in the talm values and the platform values before the first install, never after.
- No Cilium LB-IPAM or Cilium L2 announcements: MetalLB is the only service-address allocator, so there is exactly one thing answering ARP for service addresses.
- No BGP (MetalLB or Cilium) until the router and scale justify it; that is its own design.
- No service mesh, no Cilium mutual-auth or ztunnel features.
- No Cilium Egress Gateway or fixed egress IPs; egress belongs to #5030.
- No ingress-nginx: the root tenant's `ingress` option stays off and no tenant enables it. §8 checks that no ingress-nginx release exists.
- `gateway.tenantSelectableClasses` stays empty: every tenant uses `cilium`.

### 4.4 Hubble

Hubble relay and UI are switched on through the Cilium package's values override at install (the exact value path is read from the pinned tag's Cilium package in slice B1). Hubble UI is reachable only on the tailnet (§6), never through the Cloudflare tunnel. Hubble metrics feed the Phase 5 metrics store when it exists.

## 5. Addresses and the MetalLB pool plan (item 3)

### 5.1 Rules

1. **Every cluster address comes from the LAN prefix and lies outside the router's DHCP pool.** Addresses are reserved on the router (or the DHCP pool is shrunk) before first use, so nothing else can be handed the same address.
2. **One contiguous reserved block for the cluster**, split into the uses below, recorded in one table in `infra/cluster/basement/platform/README.md` with each address's owner. Nothing is assigned that is not in the table.
3. **The control-plane VIP is not in any MetalLB pool.** It is the Talos `floatingIP` and must equal the host part of `endpoint`.
4. **Node addresses are static** (DHCP reservations or Talos static config), outside the block's MetalLB part.
5. **Pools:**
   - `platform`: small (2–4 addresses), `autoAssign: false`. Holds the platform Gateway's address and the Hubble/admin addresses if any need their own. The platform Gateway's address is pinned by pre-creating its LoadBalancer Service with `loadBalancerIP` (Cozystack's Tenant has no gateway IP field).
   - `tenants`: the bulk of the block, `autoAssign: true`, for tenant Gateways and tenant LoadBalancer Services.
   - `reserved`: unassigned headroom kept in the table so growth never has to renumber.
6. **L2 announcement only**, one `L2Advertisement` per pool, limited with `nodeSelectors` to nodes on the LAN segment and `interfaces` to the LAN-facing link, so ARP is never answered on a storage or management network.
7. **No overlap** with `podCIDR`, `serviceCIDR`, Docker/Tailscale/Kube-OVN join subnets, or any subnet the tailnet routes.

### 5.2 What the numbers wait on

| Fact                                                                                          | Where it comes from   | Used for                                 |
| --------------------------------------------------------------------------------------------- | --------------------- | ---------------------------------------- |
| LAN prefix and gateway                                                                        | #6077                 | the block, rule 7 checks                 |
| Router DHCP range, and whether it can be shrunk or reservations made                          | #6077 (router config) | rule 1                                   |
| Which NIC on each node faces the LAN; VLANs; whether storage traffic is on a separate segment | #1810, #6077, #13920  | `interfaces`, `nodeSelectors`, `vipLink` |
| First node count and roles                                                                    | #1810 shortlist       | static node addresses, VIP placement     |

When those land, slice B2 fills the table and writes `IPAddressPool`/`L2Advertisement` manifests from it. No range is written before then.

## 6. Gateway API design (item 4)

### 6.1 Path

`gateway.enabled: true` on the platform Package moves the platform's own services from Ingress to Gateway API. `cozystack-controller` creates, from the publishing tenant's `TenantGateway`: the Gateway (class `cilium`), an HTTP-to-HTTPS redirect route, and the per-hostname listeners. Cilium creates the Gateway's LoadBalancer Service, and MetalLB gives it its pinned `platform` address (§5.1).

### 6.2 First routes

| Route          | Kind and namespace                          | Backend                       | Listener                       |
| -------------- | ------------------------------------------- | ----------------------------- | ------------------------------ |
| Dashboard      | `HTTPRoute` `dashboard` in `cozy-dashboard` | `incloud-web-gatekeeper:8000` | per-hostname HTTPS listener    |
| Kubernetes API | `TLSRoute` `kubernetes-api` in `default`    | `kubernetes:443`              | `tls-api` passthrough listener |

Both render from `publishing.exposedServices: [dashboard, api]`; we do not hand-write them. The other passthrough services (`vm-exportproxy`, `cdi-uploadproxy`) stay off in v1 (`tlsPassthroughServices` limited to `api`) until a VM workflow needs them.

Hostnames are `dashboard.<apex>` and `api.<apex>` under the cluster's `publishing.host` (Q3). The API server certificate must include `api.<apex>` in its SANs: added through `certSANs` in the talm values in slice B1.

The test workload for the Phase 2 "done" check is one tenant with `tenant.spec.gateway: true` and one `HTTPRoute` with a non-empty `spec.hostnames` to a hello-world Service.

### 6.3 TLS seam with Phase 6

- **HTTP-01 is rejected.** It is Cozystack's default, but it needs the public internet to reach port 80 on the Gateway, and this Gateway is LAN-only by design (§7).
- **Chosen: DNS-01** (`publishing.certificates.solver: dns01`) against the Cloudflare zone of the apex: one wildcard certificate per gateway (`<tgw>-gateway-tls`), which also avoids the 64-listener cap. This needs a Cloudflare API token scoped to DNS edit on that one zone.
- **Interim, until Phase 6's ESO wiring exists:** the token is a Tier A secret held in the fleet secrets broker (#13948). The bootstrap runbook creates the Kubernetes Secret from the broker at install time; it never enters git (Tier A rule, PR #13992). Phase 6 replaces that one step with an `ExternalSecret`; nothing else in this design changes.
- **Phase 6 proper:** cert-manager chained to the fleet CA is for internal service identity; the public-name gateway certificates stay ACME via DNS-01 unless Phase 6 decides otherwise.
- **If even the interim token is not available at install:** run the gateway in `existingSecret` mode (`publishing.certificates.wildcardSecretName`) with a certificate issued off-cluster, or reach the dashboard only on the tailnet with the API on passthrough (the API serves its own certificate). Both are stopgaps that §8 flags until DNS-01 works.

## 7. Access path (item 5)

- **Nothing is published to the internet directly.** No router port-forward to any cluster address; the MetalLB pools are LAN-private addresses.
- **Tailscale is the management path.** The Tailscale Kubernetes operator (Phase 4 platform service, #1088) runs on the cluster and advertises the `platform` pool and the control-plane VIP to the tailnet as subnet routes, with tailnet ACLs restricting them to Drew's devices and the fleet admin tags. `kubectl`/`talosctl` and Hubble UI are used only over this path. Until the operator is installed, an existing tailnet node on the same LAN can advertise the same routes.
- **Cloudflare tunnel for the dashboard only, and only behind Cloudflare Access.** An in-cluster `cloudflared` (Phase 4, #1088) routes `dashboard.<apex>` to the platform Gateway's address. The tunnel never routes `api.<apex>` (kubectl stays on the tailnet). A Cloudflare Access policy restricted to Drew's identity sits in front of the hostname before the tunnel route is enabled. The tunnel's credential is Tier A in the broker, like the DNS token.
- **DNS:** `dashboard.<apex>` resolves to the tunnel publicly; on the tailnet, split DNS (or Tailscale MagicDNS overrides) resolves `dashboard.<apex>` and `api.<apex>` to the `platform` address, so the same names work on both paths and the same certificate is valid on both.
- Bandwidth and failure behaviour of these paths are #5030's; this design adds only management traffic, which is negligible against the 2 Gbps link.

## 8. Verification (Phase 2 "done" checks)

Each check records its command and output on the #1087 issue.

1. `kubectl get packages -A` / HelmReleases all Ready; the installed tag equals the pin in git.
2. `cilium status` healthy on every node; `hubble observe` shows flows; Hubble UI loads over the tailnet.
3. No ingress-nginx anywhere: no HelmRelease, Deployment or IngressClass with that name.
4. `kubectl get gateway -A` shows the platform Gateway Programmed with its pinned `platform` address; on v1.7.0, each `TenantGateway`'s own status is read (cozystack/cozystack#5165).
5. A test tenant with `gateway: true` serves a hello-world `HTTPRoute` over HTTPS with a valid DNS-01 certificate, from a LAN client and from a tailnet client.
6. A LoadBalancer Service in the test tenant receives an address from the `tenants` pool, and ARP for it is answered only on the LAN link.
7. `kubectl` works against `api.<apex>` over the tailnet and fails from a host on neither the LAN nor the tailnet.
8. The dashboard loads through the Cloudflare tunnel only after an Access login, and `api.<apex>` is not reachable through the tunnel.
9. A port scan of the home WAN address shows no cluster ports open.
10. A tenant-to-tenant connection attempt between two test tenants is denied, and Hubble shows the drop.

## 9. Build slices (after Phase 1)

| Slice | Content                                                                                                                                                                       | Depends on                                      |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| B1    | `infra/cluster/basement/platform/`: pin file, platform values, Cilium/Hubble override, `certSANs` for `api.<apex>`, install runbook; key names checked against the pinned tag | #1086 done, Q1, Q3                              |
| B2    | Address table + MetalLB `IPAddressPool`/`L2Advertisement` manifests from the §5 rules                                                                                         | #6077, #1810                                    |
| B3    | Install and §8 checks 1–6                                                                                                                                                     | B1, B2                                          |
| B4    | DNS-01 issuer with the broker-held token; tailnet routes; tunnel + Access for the dashboard; §8 checks 7–10                                                                   | B3, #1088 (Tailscale operator, cloudflared), Q2 |

Each slice is filed as a child of #1087 when its dependency lands; they are listed here so nobody re-derives them.

## 10. Open questions

| #   | Question                                                                                          | Default if nobody decides                         | Decider                           |
| --- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------- | --------------------------------- |
| Q1  | Final Cozystack tag                                                                               | Newest v1.7 patch ≥ v1.7.1 at install time (§3.2) | slice B1 owner, recorded on #1087 |
| Q2  | Is a scoped Cloudflare DNS token for the apex zone acceptable as the interim, broker-held secret? | Yes, scoped to DNS edit on the one zone           | Drew (account credential)         |
| Q3  | Which apex domain the cluster publishes under                                                     | None assumed; B1 does not start until named       | Drew                              |
| Q4  | LAN prefix, DHCP range, NIC/VLAN facts                                                            | Wait; no ranges invented                          | #6077, #1810                      |

### Sources checked (2026-11-25)

- Cozystack Gateway API docs: <https://cozystack.io/docs/networking/gateway-api/> (GatewayClass `cilium`, `TenantGateway`, `gateway.enabled`, `publishing.exposedServices`, HTTP-01/DNS-01/`existingSecret`/edge modes, hostname policies, "ships MetalLB but renders no pools", Gateway address pinned by pre-created Service).
- Cozystack v1.7.0 release notes: <https://github.com/cozystack/cozystack/releases/tag/v1.7.0> (Talos v1.14.2 + `drbd_transport_tcp`, `/20` podCIDR minimum on Kube-OVN variants, mandatory route hostnames, migration hook log).
- Cozystack releases list: <https://github.com/cozystack/cozystack/releases> (v1.6.5 latest v1.6 patch; v1.7 rc history).
- Upgrade guide v1.6 → v1.7: <https://cozystack.io/docs/operations/cluster/upgrade-to-v1.7/>.
- v1.7.1 follow-ups: <https://github.com/cozystack/cozystack/issues/5165> (open on 2026-11-25).
- In-repo: `infra/cluster/basement/` (README, `values.yaml`, `.talm-preset.lock`); `doc/2026-11-23T22-33-41Z_research_cncf-landscape/networking.md`; `pkgs/lab/book/cluster/talos-and-cozystack.md`; `doc/2026-11-10_design_cluster_egress_5030.md`; `doc/2026-11-24T16-22-01Z_design_p9_lab-integration/`; #1817 plan comment of 2026-11-24.
