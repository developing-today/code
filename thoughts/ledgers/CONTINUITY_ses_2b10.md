---
session: ses_2b10
updated: 2026-04-02T16:05:12.268Z
---



## Conversation Summary

### Task/Goal
Designing a complete network topology for a homelab Kubernetes cluster.

### Hardware Inventory
- **4× Celestica DX010 switches** — Broadcom Tomahawk ASIC, 32× QSFP28 100G ports each, running SONiC (used/surplus, may fail)
- **3× Mono Gateway routers** (mono.si) — NXP LS1046A with DPAA hardware offload, 2× SFP+ 10G + 3× RJ45 1G each, running OpenWrt
- **1× prosumer 10G managed switch** — supports LACP, cannot form MC-LAGs
- **1× ONT** — ethernet, 2G ISP
- **~12 cluster nodes** — dual NICs, 4–16 NVMe each (Samsung 990 Pro), running Kubernetes with LINSTOR/DRBD/Cozystack

### Key Decisions Made

**DX010 allocation:**
- DX010-1 + DX010-2: MC-LAG pair (pairs only, always exactly 2 switches), all client downlinks
- DX010-3 & DX010-4: cold spares on shelf
- 4×100G peer link between DX010-1 and DX010-2 (user's choice despite analysis showing 1×100G suffices — peer link carries almost zero data when all nodes dual-home to both switches)

**Mono roles:**
- Mono 1: edge router (NAT, firewall, WAN-facing)
- Mono 2: internal gateway (DHCP, DNS, VRRP master)
- Mono 3: internal gateway (DHCP, DNS, VRRP backup)

**Intermediate switch** replaces DX010-3 for WAN aggregation (3.2Tbps DX010 overkill for 2Gbps WAN).

### Final Topology — 3-VLAN Design (DECIDED in latest exchange)

```
ONT → VLAN 10 ← Mono 1 → VLAN 20 ← Mono 2/3 → VLAN 30 ← DX010-1/DX010-2
```

All VLANs on the intermediate switch, all access ports (untagged), no trunking.

| VLAN               | Purpose           | Ports                                     | MTU  |
| ------------------ | ----------------- | ----------------------------------------- | ---- |
| VLAN 10 (WAN)      | ONT ↔ Mono 1      | ONT + Mono 1 SFP+0                        | 1500 |
| VLAN 20 (transit)  | Mono 1 ↔ Mono 2/3 | Mono 1 SFP+1/RJ45 + Mono 2/3 RJ45         | 1500 |
| VLAN 30 (internal) | Mono 2/3 ↔ DX010s | Mono 2/3 SFP+ + DX010-1 + DX010-2 uplinks | 9216 |

**DX010 is now pure L2 (DECIDED — gateway removed):**
- Single VLAN, all ports (peer link, downlinks, uplinks) — 9216 MTU everywhere
- No SVIs, no L3, no anycast gateway
- Pure MC-LAG L2 switch — much simpler SONiC config
- VLAN 30 on intermediate switch extends L2 domain from nodes to Mono 2/3 SFP+ ports
- MTU boundary at Mono 2/3 (routes from 9216 SFP+ side to 1500 RJ45 side, sends ICMP "too big" for PMTUD)

**Node default gateway** = Mono 2/3 VRRP VIP (e.g., 10.0.200.1), handed out via DHCP. No DX010 involvement in routing.

### Bonding & VRRP Design
- **Mono bonding:** balance-tlb (mode 5) for mixed 10G+1G speeds. Two bonds per Mono: bond-sfp (DX010-facing) and bond-rj45 (Mono1-facing)
- **VRRP (Keepalived):** Mono 2 priority 120, Mono 3 priority 110, track_interface weights (bond-sfp down: -30, bond-rj45 down: -50). Failover chain: M2(20G)→M3(20G)→M2(3G)→M3(3G)
- **Route metrics:** bond-sfp metric 10 (primary), bond-rj45 metric 100 (backup)

### Traffic Flows
- **Node → internet:** Node → DX010 (L2) → intermediate switch VLAN 30 → Mono 2/3 (VRRP VIP, first L3 hop) → VLAN 20 → Mono 1 (NAT) → VLAN 10 → ONT
- **Node → node:** Pure L2, stays within DX010 MC-LAG pair

### Technical Principles Established
- MC-LAG and spine-leaf are fundamentally at odds
- LS1046A DPAA = hardware-accelerated routing (~10–50μs for NAT)
- VLAN isolation eliminates STP entirely (SONiC STP unreliable)
- L2 switches silently drop oversized frames (no ICMP), so MTU boundaries must be at L3 routers
- Intermediate switch VLAN 30 must support jumbo frames (9216)

### Remaining Work
1. **SONiC MC-LAG config** for DX010-1/DX010-2 (now simpler: pure L2, single VLAN, no SVIs)
2. **OpenWrt config for Monos** — bonding, VRRP, routing, NAT, DHCP/DNS, jumbo MTU on SFP+
3. **Intermediate switch hardware selection** — needs ≥8× SFP+ 10G + 9× RJ45 1G, VLAN support, jumbo frame support on VLAN 30
4. **SONiC version/build** not yet confirmed
5. User was offered to start writing configs — hasn't responded yet
