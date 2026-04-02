---
session: ses_2b2f
updated: 2026-04-02T09:26:24.937Z
---

## Summary of What We've Done So Far

Here's a comprehensive summary of our network topology design session for your homelab cluster:

---

### Hardware Inventory
- **4× Celestica DX010 switches** — Broadcom Tomahawk ASIC, 32× QSFP28 100G ports each, running SONiC (used/surplus, may fail)
- **3× Mono Gateway routers** (mono.si) — NXP LS1046A with DPAA hardware offload, 2× SFP+ 10G + 3× RJ45 1G each, running OpenWrt
- **1× prosumer 12-port 10G managed switch** — supports LACP but cannot form MC-LAGs
- **1× ONT** — ethernet, 2G ISP
- **~12 cluster nodes** — dual NICs, 4–16 NVMe each (Samsung 990 Pro), running Kubernetes with LINSTOR/DRBD/Cozystack

---

### Key Decisions Made

1. **MC-LAG is pairs only** (confirmed across all implementations — always exactly 2 switches)

2. **DX010 allocation:**
   - DX010-1 + DX010-2: MC-LAG pair handling all client downlinks
   - DX010-3 & DX010-4: cold spares on shelf
   - 4×100G peer link between DX010-1 and DX010-2 (your choice despite analysis showing 1×100G is plenty — peer link carries almost zero data in steady state since all nodes dual-home to BOTH switches)

3. **Mono roles:**
   - Mono 1: edge router (NAT, firewall, WAN-facing)
   - Mono 2: internal gateway (DHCP, DNS, VRRP master, cluster-facing)
   - Mono 3: internal gateway (DHCP, DNS, VRRP backup, cluster-facing)

4. **Intermediate switch replaces DX010-3 for WAN aggregation** — a 3.2Tbps DX010 was overkill to move 2Gbps of WAN traffic. The prosumer managed switch handles this role instead, freeing DX010-3 as a spare.

---

### Final Topology

**All Monos + ONT + DX010 uplinks connect to the intermediate managed switch.**

Port count on intermediate switch (~18 ports needed):
- ONT: 1 port | Mono 1/2/3: 5 ports each (2× SFP+ + 3× RJ45) | DX010-1 & DX010-2: 1× SFP+ each (via QSFP28 breakout)

**VLAN design on intermediate switch (two options discussed):**

|                  | VLAN 10 (WAN)     | VLAN 20 (transit/LAN) | VLAN 30 (internal)    |
| ---------------- | ----------------- | --------------------- | --------------------- |
| **2-VLAN (minimum)** | ONT + Mono1 SFP+0 | Everything else       | N/A                   |
| **3-VLAN (secure)**  | ONT + Mono1 SFP+0 | Mono1↔Mono2/3 only    | Mono2/3 SFP+ + DX010s |

All switch ports are **access (untagged)**. No trunking needed.

**DX010 VLAN design (no loops, no STP):**
- VLAN 100: uplink to intermediate switch (access port)
- VLAN 200: peer link + all client downlinks, anycast gateway 10.0.200.1
- SVIs on both VLANs for L3 routing at ASIC line rate
- Jumbo frames (MTU 9216) on VLAN 200, MTU 1500 on VLAN 100 (PMTUD handles transition)

---

### Bonding & VRRP Design

- **Mono bonding:** balance-tlb (mode 5) for mixed 10G+1G speeds. Two bonds per Mono: bond-sfp (DX010-facing) and bond-rj45 (Mono1-facing)
- **VRRP (Keepalived):** Mono 2 priority 120, Mono 3 priority 110, with track_interface weights (bond-sfp down: -30, bond-rj45 down: -50). Failover chain: M2(20G) → M3(20G) → M2(3G) → M3(3G)
- **Route metrics:** bond-sfp metric 10 (primary), bond-rj45 metric 100 (backup)

---

### Traffic Flows
- **Node → internet:** Node → DX010 anycast gw (L3 ASIC, ~1μs) → VLAN 100 → intermediate switch → Mono 2/3 (VRRP) → Mono 1 (NAT) → ONT
- **Node → node:** Pure L2 VLAN 200, never leaves MC-LAG pair

---

### Key Technical Principles Established
- DX010 L3 SVIs route at ASIC line rate (~1–2μs), same as L2 switching
- MC-LAG and spine-leaf are fundamentally at odds
- LS1046A DPAA = hardware-accelerated routing (~10–50μs for NAT)
- VLAN isolation eliminates STP entirely (SONiC STP support is unreliable)

---

### Still Open / Pending
- **2-VLAN vs 3-VLAN** design on intermediate switch — not yet chosen
- **SONiC version/build** not confirmed
- **Actual SONiC MC-LAG config** not yet written
- **Intermediate switch hardware** not finalized (needs ≥8× SFP+ 10G + 9× RJ45 1G)
- ONT management to be handled later
