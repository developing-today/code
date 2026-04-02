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
                                                           │ ETH (access, VLAN 10)
                                                           │
                    ┌──────────────────────────────────────────────────────────────────────┐
                    │                    INTERMEDIATE 10G MANAGED SWITCH                    │
                    │                                                                      │
                    │  VLAN 10 (WAN)        VLAN 20 (Transit)       VLAN 30 (Internal)     │
                    │  MTU 1500             MTU 1500                MTU 9216               │
                    │  ┌───────────────┐    ┌───────────────┐       ┌──────────────────┐   │
                    │  │ ONT           │    │ Mono1 RJ45×3  │       │ Mono2 SFP+×2     │   │
                    │  │ Mono1 SFP+×2  │    │ Mono2 RJ45×3  │       │ Mono3 SFP+×2     │   │
                    │  └───────────────┘    │ Mono3 RJ45×3  │       │ DX010-1 uplink   │   │
                    │                       └───────────────┘       │ DX010-2 uplink   │   │
                    │                                               └──────────────────┘   │
                    └──────────────────────────────────────────────────────────────────────┘
                         │                    │  │  │  │                │  │  │  │
                    SFP+ 10G            SFP+/RJ45 mixed           SFP+ 10G × 4
                         │                    │  │  │  │                │  │  │  │
              ┌──────────┘    ┌───────────────┘  │  │  │    ┌───────────┘  │  │  └──────────┐
              │               │                  │  │  │    │              │  │              │
     ┌────────┴────────┐     ┌┴──────────────────┴┐│ ┌┴────┴──────────────┴┐│      ┌───────┴───────┐
     │   MONO 1        │     │    MONO 2          │ │ │    MONO 3          │ │      │               │
     │  (Edge Router)  │     │ (Internal GW)      │ │ │ (Internal GW)      │ │      │               │
     │                 │     │  VRRP Master        │ │ │  VRRP Backup       │ │      │               │
     │  NXP LS1046A    │     │  NXP LS1046A        │ │ │  NXP LS1046A       │ │      │               │
     │  DPAA offload   │     │  DPAA offload       │ │ │  DPAA offload      │ │      │               │
     │                 │     │  Priority: 120      │ │ │  Priority: 110     │ │      │               │
     │ bond-wan        │     │                     │ │ │                    │ │      │               │
     │  SFP+0+1 (2×10G)│     │ bond-sfp (10G+10G)  │ │ │ bond-sfp (10G+10G) │ │      │               │
     │  → ONT/VLAN 10  │     │   → DX010s/VLAN 30  │ │ │   → DX010s/VLAN 30 │ │      │               │
     │  metric 10      │     │ bond-rj45 (3×1G)    │ │ │ bond-rj45 (3×1G)   │ │      │               │
     │ bond-transit    │     │   → Mono1/VLAN 20   │ │ │   → Mono1/VLAN 20  │ │      │               │
     │  RJ45×3 (3×1G)  │     │                     │ │ │                    │ │      │               │
     │  → Mono2,3/V20  │     │ DHCP, DNS           │ │ │ DHCP, DNS          │ │      │               │
     │  metric 100     │     │ VRRP VIP: 10.0.200.1│ │ │ VRRP VIP: 10.0.200.1│      │               │
     │                 │     └─────────────────────┘ │ └────────────────────┘ │      │               │
     │ NAT / Firewall  │
                                                     │                       │      │               │
                    ┌────────────────────────────────┘                       │      │               │
                    │                    ┌───────────────────────────────────┘      │               │
                    │                    │                                          │               │
          ┌────────┴────────────────────┴──────────────────────────────────────────┴───────┐       │
          │                                                                                │       │
          │                         DX010-1  ◄══  4×100G Peer Link  ══►  DX010-2           │       │
          │                                                                                │       │
          │                    Celestica DX010 MC-LAG Pair (Pure L2)                        │       │
          │                    Broadcom Tomahawk — 32× QSFP28 100G each                    │       │
          │                    Single VLAN — MTU 9216 everywhere                            │       │
          │                    No SVIs, No L3, No anycast gateway                           │       │
          │                                                                                │       │
          └───┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────────────────────┘       │
              │    │    │    │    │    │    │    │    │    │    │    │                              │
           MC-LAG (dual-homed LACP bonds, one link to each switch)                                │
              │    │    │    │    │    │    │    │    │    │    │    │                              │
          ┌───┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐┌──┴┐                        │
          │ N1 ││ N2 ││ N3 ││ N4 ││ N5 ││ N6 ││ N7 ││ N8 ││ N9 ││N10││N11││N12│                       │
          │    ││    ││    ││    ││    ││    ││    ││    ││    ││    ││    ││    │                       │
          │dual││dual││dual││dual││dual││dual││dual││dual││dual││dual││dual││dual│                      │
          │NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC ││NIC │                     │
          │4-16││4-16││4-16││4-16││4-16││4-16││4-16││4-16││4-16││4-16││4-16││4-16│                     │
          │NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe││NVMe│                     │
          └────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘└────┘
                         ~12 Kubernetes Nodes (LINSTOR/DRBD/Cozystack)
                         Default GW: 10.0.200.1 (Mono 2/3 VRRP VIP via DHCP)
```

## VLAN Map

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                        INTERMEDIATE SWITCH VLAN MAP                          │
│                        (All access/untagged ports)                           │
├──────────────┬───────────────────────────────────────┬───────────────────────┤
│  VLAN 10     │  VLAN 20                              │  VLAN 30             │
│  WAN         │  Transit                              │  Internal            │
│  MTU: 1500   │  MTU: 1500                            │  MTU: 9216           │
│              │                                       │                      │
│  • ONT       │  • Mono 1 — RJ45×3 (3×1G)            │  • Mono 2 — SFP+×2   │
│  • Mono 1    │  • Mono 2 — RJ45×3 (3×1G)            │  • Mono 3 — SFP+×2   │
│    SFP+×2    │  • Mono 3 — RJ45×3 (3×1G)            │  • DX010-1 uplink    │
│              │                                       │  • DX010-2 uplink    │
│              │                                       │  • DX010-1 uplink    │
│              │                                       │  • DX010-2 uplink    │
└──────────────┴───────────────────────────────────────┴───────────────────────┘
```

## Traffic Flows

```
NODE-TO-INTERNET (e.g., node pulls container image):
══════════════════════════════════════════════════════

  Node ──LACP──► DX010 MC-LAG (L2 forward)
                    │
                    ▼ VLAN 30 (9216 MTU)
              Intermediate Switch
                    │
                    ▼
              Mono 2 or 3 (VRRP VIP 10.0.200.1)
              ┌─ MTU boundary: 9216 → 1500 ─┐
              │  Routes + sends ICMP         │
              │  "Frag Needed" for PMTUD     │
              └──────────────────────────────┘
                    │
                    ▼ VLAN 20 (1500 MTU)
              Intermediate Switch
                    │
                    ▼
              Mono 1 (NAT + Firewall)
                    │
                    ▼ VLAN 10 (1500 MTU)
              Intermediate Switch
                    │
                    ▼
                   ONT ──► Internet


NODE-TO-NODE (e.g., DRBD replication):
══════════════════════════════════════

  Node A ──LACP──► DX010-1 or DX010-2 ──L2──► Node B
                   (pure L2 switching, 9216 MTU)
                   Never leaves MC-LAG pair
```

## Bonding Detail

```
┌─────────────────────────────────────────────────┐
│              MONO 2 / MONO 3 BONDING            │
│              (balance-tlb / mode 5)             │
├─────────────────────┬───────────────────────────┤
│  bond-sfp           │  bond-rj45               │
│  (DX010-facing)     │  (Mono1-facing)          │
│                     │                           │
│  SFP+0  10G ──┐    │  RJ45-0  1G ──┐          │
│  SFP+1  10G ──┤    │  RJ45-1  1G ──┤          │
│               │    │  RJ45-2  1G ──┤          │
│         bond-sfp   │          bond-rj45        │
│         20G max    │          3G max           │
│         MTU 9216   │          MTU 1500         │
│         metric 10  │          metric 100       │
│  (primary path)    │  (backup path)            │
├─────────────────────┴───────────────────────────┤
│  VRRP (Keepalived)                              │
│  VIP: 10.0.200.1                                │
│  Mono 2: priority 120 (master)                  │
│  Mono 3: priority 110 (backup)                  │
│  track_interface:                               │
│    bond-sfp  down → weight -30                  │
│    bond-rj45 down → weight -50                  │
│                                                 │
│  Failover chain:                                │
│    M2(20G) → M3(20G) → M2(3G) → M3(3G)        │
└─────────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────────┐
│              MONO 1 BONDING                      │
│              (Edge Router, balance-tlb / mode 5) │
├─────────────────────┬───────────────────────────┤
│  bond-wan           │  bond-transit             │
│  (WAN / ONT-facing) │  (Transit / Mono2,3)     │
│                     │                           │
│  SFP+0  10G ──┐    │  RJ45-0  1G ──┐          │
│  SFP+1  10G ──┤    │  RJ45-1  1G ──┤          │
│               │    │  RJ45-2  1G ──┤          │
│         bond-wan   │          bond-transit      │
│         20G max    │          3G max           │
│         MTU 1500   │          MTU 1500         │
│         metric 10  │          metric 100       │
│  (VLAN 10)         │  (VLAN 20)               │
│  primary path      │  lower-priority path     │
├─────────────────────┴───────────────────────────┤
│  Functions: NAT, Firewall, WAN termination      │
│  Link redundancy on WAN (if 1 SFP+ fails,      │
│  ONT still reachable via other SFP+)            │
│  Transit 3G > WAN 2G = no bottleneck           │
└─────────────────────────────────────────────────┘
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
