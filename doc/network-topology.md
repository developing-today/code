# Network Topology — Homelab Kubernetes Cluster

## Physical Layout

```
                            ┌─────────────────────────────────────────────────────────────────┐
                            │                        INTERNET                                 │
                            └──────────────────────────────┬──────────────────────────────────┘
                                                           │ 2 Gbps
                                                           │
                                                    ┌──────┴──────┐
                                                    │     ONT     │
                                                    │  (Ethernet) │
                                                    └──────┬──────┘
                                                           │ access port, VLAN 10
                                                           │
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│                              INTERMEDIATE 10G MANAGED SWITCH                                      │
│                              MTU is per-port (not per-VLAN) — see port map below                  │
│                                                                                                  │
│  Access ports:                      Hybrid ports (all 15 Mono ports):                            │
│  ┌──────────────┐ ┌──────────────┐  ┌────────────────────────────────────────────────────────┐   │
│  │ ONT          │ │ DX010-1 up   │  │ Mono 1 ×5: PVID=10 (untagged) + VLAN 20 (tagged)     │   │
│  │ VLAN 10 only │ │ VLAN 30 only │  │ Mono 2 ×5: PVID=30 (untagged) + VLAN 20 (tagged)     │   │
│  └──────────────┘ │ DX010-2 up   │  │ Mono 3 ×5: PVID=30 (untagged) + VLAN 20 (tagged)     │   │
│                   │ VLAN 30 only │  │                                                        │   │
│                   └──────────────┘  │ VLAN 20 (transit) is ALWAYS tagged, NEVER a default    │   │
│                                     └────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
       │          │  │                    │ │ │ │ │        │ │ │ │ │        │ │ │ │ │
       │          │  │                    └─┤─┤─┤─┤────────┤─┤─┤─┤─┤────────┤─┤─┤─┤─┘
       │          │  │                      │ │ │ │        │ │ │ │ │        │ │ │ │
  ┌────┴─────┐  ┌─┴──┴──┐          ┌───────┴─┴─┴─┴┐  ┌───┴─┴─┴─┴─┴┐  ┌───┴─┴─┴─┴─┐
  │  MONO 1  │  │DX010  │          │   MONO 2      │  │   MONO 3    │  │           │
  │  Edge    │  │MC-LAG │          │   Int. GW     │  │   Int. GW   │  │           │
  │  Router  │  │Pair   │          │   VRRP Master │  │   VRRP Bkup │  │           │
  └──────────┘  └───────┘          └───────────────┘  └─────────────┘  │           │
                                                                       │           │
          ┌────────────────────────────────────────────────────────────┘           │
          │                                                                        │
┌─────────┴────────────────────────────────────────────────────────────────────────┴───────┐
│                                                                                          │
│                         DX010-1  ◄══  4×100G Peer Link  ══►  DX010-2                     │
│                                                                                          │
│                    Celestica DX010 MC-LAG Pair (Pure L2)                                  │
│                    Broadcom Tomahawk — 32× QSFP28 100G each                              │
│                    Single VLAN — MTU 9216 everywhere                                     │
│                    No SVIs, No L3, No anycast gateway                                    │
│                                                                                          │
└───┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬──────────────────────────────┘
    │    │    │    │    │    │    │    │    │    │    │    │
 MC-LAG (dual-homed LACP bonds, one link to each switch)
    │    │    │    │    │    │    │    │    │    │    │    │
┌───┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐
│ N1 ││ N2 ││ N3 ││ N4 ││ N5 ││ N6 ││ N7 ││ N8 ││ N9 ││N10││N11││N12│
│dual││dual││dual││dual││dual││dual││dual││dual││dual││dual││dual││dual│
│NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC │
└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘
                     ~12 Kubernetes Nodes (LINSTOR/DRBD/Cozystack)
                     Default GW: 10.30.0.1 (Mono 2/3 VRRP VIP)
```

## Switch Port Map

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                     INTERMEDIATE SWITCH PORT CONFIGURATION                       │
├─────────────────┬───────────┬─────────────────────────┬─────────────────────────┬───────┤
│  Port           │ Mode      │ Untagged (PVID)         │ Tagged                  │ MTU   │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  ONT            │ Access    │ VLAN 10                 │ —                       │ 1500  │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  Mono 1 SFP+0   │ Hybrid    │ VLAN 10 (WAN)           │ VLAN 20 (transit)       │ 1500  │
│  Mono 1 SFP+1   │ Hybrid    │ VLAN 10 (WAN)           │ VLAN 20 (transit)       │ 1500  │
│  Mono 1 RJ45-0  │ Hybrid    │ VLAN 10 (WAN)           │ VLAN 20 (transit)       │ 1500  │
│  Mono 1 RJ45-1  │ Hybrid    │ VLAN 10 (WAN)           │ VLAN 20 (transit)       │ 1500  │
│  Mono 1 RJ45-2  │ Hybrid    │ VLAN 10 (WAN)           │ VLAN 20 (transit)       │ 1500  │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  Mono 2 SFP+0   │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 2 SFP+1   │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 2 RJ45-0  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 2 RJ45-1  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 2 RJ45-2  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  Mono 3 SFP+0   │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 3 SFP+1   │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 3 RJ45-0  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 3 RJ45-1  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
│  Mono 3 RJ45-2  │ Hybrid    │ VLAN 30 (internal)      │ VLAN 20 (transit)       │ 9216  │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  DX010-1 uplink │ Access    │ VLAN 30                 │ —                       │ 9216  │
│  DX010-2 uplink │ Access    │ VLAN 30                 │ —                       │ 9216  │
├─────────────────┼───────────┼─────────────────────────┼─────────────────────────┼───────┤
│  Port MTU note: MTU is per-port. Mono 2/3 ports set to 9216 because they carry │       │
│  VLAN 30 (jumbo). VLAN 20's 1500 limit enforced at Mono interface (.20 sub-if). │       │
└─────────────────┴───────────┴─────────────────────────┴─────────────────────────┴───────┘

Switch requirements: ≥ 6 SFP+ 10G + 11 RJ45 1G, hybrid/general port mode,
                     per-port MTU config, 9216 MTU on SFP+ and RJ45 (for Mono 2/3 + DX010 ports)
```

## VLAN Summary

```
┌────────────────┬────────────────┬─────────────────────────────────────────────────┐
│  VLAN          │  MTU           │  Purpose                                        │
├────────────────┼────────────────┼─────────────────────────────────────────────────┤
│  10 (WAN)      │  1500          │  ONT ↔ Mono 1. ISP-facing.                      │
│  20 (Transit)  │  1500          │  Mono mesh. Always tagged, never a PVID.        │
│  30 (Internal) │  9216 (jumbo)  │  Nodes ↔ Mono 2/3. DRBD/storage traffic.        │
└────────────────┴────────────────┴─────────────────────────────────────────────────┘
```

## Addressing and Segmentation Plan (proposed, not deployed)

This is the target IPv4 plan for the VLANs above, replacing addresses from the
torn-down 01b.us test cluster. It is a design, not a record of configured
devices or physical acceptance. Before configuration, confirm the private
ranges do not overlap the home LAN, VPN/tailnet routes, or any cloud networks;
also confirm the gateway and DHCP failover choices below. No per-machine
addresses are assigned until the inventory and NIC facts in #1080/#1097 are
verified.

| VLAN          | IPv4 allocation                                                              | Gateway and assignment policy                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 10 — WAN      | ISP-provided address on the ONT/Mono 1 WAN handoff; no locally chosen subnet | Mono 1 is the only WAN-facing router. Obtain addressing from the ISP/ONT as provisioned; never serve node DHCP or bridge nodes directly onto this VLAN.                                                                                                                                                                                                                                                                                                                       |
| 20 — Transit  | `172.20.0.0/29` (`172.20.0.0` network, `.7` broadcast)                       | Static router addresses: Mono 1 `172.20.0.1`, Mono 2 `172.20.0.2`, Mono 3 `172.20.0.3`. Mono 2/3 send upstream traffic to Mono 1; no DHCP clients or non-router endpoints.                                                                                                                                                                                                                                                                                                    |
| 30 — Internal | `10.30.0.0/24` (`10.30.0.0` network, `10.30.0.255` broadcast)                | VRRP default-gateway VIP `10.30.0.1`; Mono 2 `10.30.0.2`; Mono 3 `10.30.0.3`. Router addresses are static. Use DHCP reservations for cluster nodes after their MAC/NIC inventory is verified; reserve `.16–.63` for those nodes. Keep `.4–.15` for confirmed network infrastructure, `.64–.127` for statically managed cluster/service addresses, `.128–.191` unallocated growth, and `.192–.239` as a bounded temporary/provisioning DHCP pool. `.240–.254` remain reserved. |

VLAN 30 remains one internal L2 subnet for the current node/storage fabric; the
address ranges above are allocation conventions, not extra VLANs or security
boundaries. The edge firewall and internal gateways enforce the routed
boundaries: nodes reach the WAN only through Mono 2/3 → VLAN 20 → Mono 1, and
VLAN 10 has no direct node attachment. Keep the service-address range out of
DHCP. Do not assign BMC/KVM addresses or put management interfaces on VLAN 30
by assumption: the management VLAN/subnet and per-machine reachability remain
gated on the verified inventory and physical work in #1080/#1097.

Mono 2/3 provide the VLAN 30 gateway, DNS, and DHCP roles. DHCP reservations
for nodes are created only after inventory; dynamic leases are limited to the
temporary pool. The pair must use one explicit, tested lease-coordination or
active/standby failover design before both gateways serve DHCP—never run
independent overlapping scopes. Static routes and the VRRP VIP are configured
on the routers, not obtained from DHCP. VLAN 20 remains statically addressed.

## Bonding Detail

```
┌───────────────────────────────────────────────────────────────────────────────┐
│  ALL 3 MONOS — IDENTICAL BONDING PATTERN (balance-tlb / mode 5)              │
│                                                                               │
│  bond-sfp  (SFP+0 + SFP+1, 2×10G, metric 10)    ← primary path             │
│  ├── untagged → home VLAN (Mono 1: V10, Mono 2/3: V30)                      │
│  └── .20     → VLAN 20 (tagged sub-interface)                                │
│                                                                               │
│  bond-rj45 (RJ45×3, 3×1G, metric 100)            ← backup path              │
│  ├── untagged → home VLAN (Mono 1: V10, Mono 2/3: V30)                      │
│  └── .20     → VLAN 20 (tagged sub-interface)                                │
│                                                                               │
│  Both bonds carry both VLANs. If bond-sfp dies, bond-rj45                    │
│  still reaches BOTH VLANs at 3G. No VLAN goes dark.                          │
└───────────────────────────────────────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────────┐
│              MONO 1 (Edge Router)                │
├─────────────────────┬───────────────────────────┤
│  bond-sfp           │  bond-rj45               │
│  2×10G, metric 10   │  3×1G, metric 100        │
│                     │                           │
│  bond-sfp           │  bond-rj45               │
│    → VLAN 10 (WAN)  │    → VLAN 10 (WAN)       │
│  bond-sfp.20        │  bond-rj45.20            │
│    → VLAN 20 (transit)  → VLAN 20 (transit)    │
├─────────────────────┴───────────────────────────┤
│  Functions: NAT, Firewall, WAN termination      │
│  Default: WAN (VLAN 10). Transit is explicit.   │
└─────────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────────┐
│              MONO 2 / MONO 3 (Internal GW)      │
├─────────────────────┬───────────────────────────┤
│  bond-sfp           │  bond-rj45               │
│  2×10G, metric 10   │  3×1G, metric 100        │
│                     │                           │
│  bond-sfp           │  bond-rj45               │
│    → VLAN 30 (LAN)  │    → VLAN 30 (LAN)       │
│  bond-sfp.20        │  bond-rj45.20            │
│    → VLAN 20 (transit)  → VLAN 20 (transit)    │
├─────────────────────┴───────────────────────────┤
│  Functions: DHCP, DNS, default gateway          │
│  Default: LAN (VLAN 30). Transit is explicit.   │
│  VRRP VIP: 10.30.0.1                           │
│  Mono 2: priority 120 (master)                  │
│  Mono 3: priority 110 (backup)                  │
│  track_interface:                               │
│    bond-sfp  down → weight -30                  │
│    bond-rj45 down → weight -50                  │
│  Failover: M2(20G)→M3(20G)→M2(3G)→M3(3G)      │
└─────────────────────────────────────────────────┘
```

## Traffic Flows

```
NODE-TO-INTERNET (e.g., node pulls container image):
══════════════════════════════════════════════════════

  Node ──LACP──► DX010 MC-LAG (L2 forward)
                    │
                    ▼ VLAN 30 (untagged, 9216 MTU)
              Intermediate Switch (access port)
                    │
                    ▼ VLAN 30 (untagged on Mono hybrid port)
              Mono 2 or 3  (VRRP VIP 10.30.0.1)
              ┌─ MTU boundary: 9216 → 1500 ─┐
              │  Routes + sends ICMP         │
              │  "Frag Needed" for PMTUD     │
              └──────────────────────────────┘
                    │
                    ▼ VLAN 20 (tagged .20 sub-interface)
              Intermediate Switch (hybrid port, tagged)
                    │
                    ▼ VLAN 20 (tagged on Mono 1 hybrid port)
              Mono 1  (NAT + Firewall)
                    │
                    ▼ VLAN 10 (untagged on Mono 1 hybrid port)
              Intermediate Switch (hybrid port, PVID=10)
                    │
                    ▼ VLAN 10 (access port)
                   ONT ──► Internet


NODE-TO-NODE (e.g., DRBD replication):
══════════════════════════════════════

  Node A ──LACP──► DX010-1 or DX010-2 ──L2──► Node B
                   (pure L2 switching, 9216 MTU)
                   Never leaves MC-LAG pair


MONO-TO-MONO (transit, e.g., Mono 2 → Mono 1 for NAT):
═══════════════════════════════════════════════════════

  Mono 2 bond-sfp.20 ──tagged──► Intermediate Switch
                                       │
                                  VLAN 20 (tagged)
                                       │
                                       ▼
                                 Mono 1 bond-sfp.20
                                 (or bond-rj45.20 if SFP+ down)
```

## Internet Egress Policy (design constraints, not deployed configuration)

The roadmap records a **2 Gbps symmetric Brightspeed service**. Treat that as
the shared WAN ceiling, not measured throughput or a promise of application
bandwidth. All node internet traffic follows VLAN 30 → Mono 2/3 → VLAN 20 →
Mono 1 NAT/firewall → VLAN 10/ONT. Node-to-node storage traffic stays on the
internal fabric and does not consume this WAN capacity. No direct node-to-WAN
path or unsolicited inbound port-forward is part of this design. Any shaping,
priority, or rate limit must be configured and measured after the real link and
hardware are verified; this document does not claim that such controls exist.

- **Cloudflare Tunnel:** use only for explicitly approved public HTTP(S)
  services. The connector initiates outbound connections; do not treat that as
  a general network route or publish admin/BMC interfaces. Limit hostnames and
  origins to the intended service.
- **Turso:** clients use outbound TLS to the managed database service. Turso is
  not a LAN tunnel; database traffic is still WAN egress. Keep replication,
  exports, and other bulk transfers bounded, and do not route them through a
  second overlay unless a separate design requires it.
- **Tailscale:** use the overlay for private administration and private
  service access, not as the default public-ingress path. Prefer direct
  peer-to-peer paths when available; DERP relay fallback adds latency and
  consumes WAN capacity. Do not assume every connection is direct or that
  relay traffic is free of the Brightspeed limit.
- **Compression:** no compression is assumed at the tunnel or TLS layer.
  Compress/deduplicate bulk data at the application or file-transfer layer
  only where supported and measured; skip already-compressed or encrypted
  content. Consider CPU cost and compression ratio, and schedule or rate-limit
  bulk transfers so they cannot starve interactive and control-plane traffic.

These are design constraints and operating options, not a statement that
Cloudflare, Turso, or Tailscale connectivity or traffic shaping is deployed.
Physical uplink, NIC/switch, and per-machine management verification remain
gated on #1080/#1097.

## Cold Spares

```
┌─────────────────────────────────────────────────┐
│  ON SHELF (unpowered)                           │
│                                                 │
│  • DX010-3  —  spare for DX010-1 or DX010-2    │
│  • DX010-4  —  spare for DX010-1 or DX010-2    │
│                                                 │
│  Swap procedure: power on, load identical       │
│  SONiC config, re-cable, MC-LAG reforms.        │
└─────────────────────────────────────────────────┘
```

## Design Principles

1. **VLAN 20 is always tagged** — transit is never a default, always explicit
2. **Hybrid ports, not trunks** — fail-closed (only explicitly added VLANs), no VLAN hopping risk
3. **All 5 Mono ports active** — both bonds carry both VLANs, full failover
4. **Two firewalls deep** — node → Mono 2/3 → Mono 1 → internet (no shortcut possible)
5. **No STP needed** — Monos route between VLANs (L3), never bridge (no L2 loops)
6. **MTU is per-port** — Mono 2/3 + DX010 ports at 9216; Mono 1 + ONT ports at 1500. VLAN 20's 1500 enforced at Mono interface (.20 sub-if), not switch
7. **DX010 pure L2** — no SVIs, no routing, just MC-LAG switching at 9216 MTU
8. **Switch needs 9216 on both SFP+ and RJ45** — for Mono 2/3 fallback path on RJ45

## Required Sysctls (Monos only)

Both bonds per Mono share the same subnets (two interfaces on same VLAN).
These sysctls ensure ARP respects routing metrics — preferred bond answers,
backup stays silent until needed. Kernel-driven failover, no scripts.

```
# Only reply to ARP on the interface the kernel would route through
net.ipv4.conf.all.arp_filter = 1

# Use the outgoing interface's own IP as ARP source (no cross-contamination)
net.ipv4.conf.all.arp_announce = 2
```

Not needed on nodes (single LACP bond), DX010s (pure L2), or intermediate switch (L2 only).
