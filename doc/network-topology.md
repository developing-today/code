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
                     Default GW: 10.0.200.1 (Mono 2/3 VRRP VIP via DHCP)
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
┌──────────────┬───────────────┬───────────────────────────────────────────────────┐
│  VLAN        │ MTU           │ Purpose                                           │
├──────────────┼───────────────┼───────────────────────────────────────────────────┤
│  10 (WAN)    │ 1500          │ ONT ↔ Mono 1. ISP-facing.                        │
│  20 (Transit)│ 1500          │ Mono mesh. Always tagged, never a PVID.           │
│  30 (Internal│ 9216 (jumbo)  │ Nodes ↔ Mono 2/3. DRBD/storage traffic.          │
└──────────────┴───────────────┴───────────────────────────────────────────────────┘
```

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
│  VRRP VIP: 10.0.200.1                          │
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
              Mono 2 or 3  (VRRP VIP 10.0.200.1)
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
