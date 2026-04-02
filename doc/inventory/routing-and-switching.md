# Routing & Switching Inventory

> Last updated: 2026-04-02

---

## Core Fabric Switches

### Celestica DX010 (4x)

| Attribute | Value |
|---|---|
| **Ports** | 32x QSFP28 100GbE |
| **Breakout** | 4x25GbE or 4x10GbE per QSFP28 (up to 128x 25G or 128x 10G) |
| **ASIC** | Broadcom Memory BCM56960 (Tomahawk) |
| **Switching Capacity** | 3.2 Tbps |
| **Forwarding Rate** | 2,380 Mpps |
| **Latency** | ~1-2 us (cut-through capable) |
| **MTU / Jumbo** | 9216 bytes |
| **Form Factor** | 1RU |
| **OS** | SONiC (Open Network Install Environment / ONIE compatible) |
| **Management** | CLI, REST API, gNMI, SNMP |
| **L2 Features** | VLAN, LACP, MC-LAG, STP (limited in SONiC) |
| **L3 Features** | Static routes, BGP, OSPF, ECMP, SVIs, anycast gateway |
| **MC-LAG** | Yes (SONiC MC-LAG, pairs of 2) |
| **Stacking** | No (not applicable, spine-class switch) |
| **Class** | Enterprise / Data Center |
| **Released** | ~2016 |
| **Status** | Community-supported via SONiC, active open-source development |
| **Notes** | Bare-metal white-box switch. Originally designed for hyperscale data centers. Used/surplus units, potential for hardware failure. |

---

### IBM RackSwitch G8264 (3x)

| Attribute | Value |
|---|---|
| **Ports** | 48x SFP+ 10GbE + 4x QSFP+ 40GbE |
| **Breakout** | 4x10GbE per QSFP+ (up to 64x 10GbE total) |
| **ASIC** | Memory (BNT design, acquired by IBM) |
| **Switching Capacity** | 1.28 Tbps |
| **Forwarding Rate** | 952 Mpps |
| **Latency** | ~1-2 us |
| **MTU / Jumbo** | 9216 bytes |
| **Form Factor** | 1RU |
| **OS** | IBM ENOS (Enterprise NOS) or Lenovo CNOS; ONIE compatible |
| **Management** | CLI, SNMP, web GUI |
| **L2 Features** | VLAN (4094), LACP, STP/RSTP/MSTP, vLAG (IBM's MC-LAG) |
| **L3 Features** | Static routes, OSPF, BGP, ECMP, VRRP, SVIs |
| **MC-LAG** | Yes (vLAG - IBM/Lenovo virtual LAG, pairs of 2) |
| **Stacking** | No (uses vLAG for multi-chassis) |
| **Class** | Enterprise / Data Center TOR |
| **Released** | ~2012 |
| **EOL** | ~2018 (IBM sold networking to Lenovo 2014, became G8272 line) |
| **Notes** | Originally IBM System Networking, later Lenovo RackSwitch. Mature enterprise TOR switch with rich L2/L3 feature set. CEE/FCoE capable for converged fabrics. |

---

### IBM RackSwitch G8264e (1x)

| Attribute | Value |
|---|---|
| **Ports** | 48x 10GBASE-T RJ45 + 4x QSFP+ 40GbE |
| **Breakout** | 4x10GbE per QSFP+ (up to 64x 10GbE total) |
| **ASIC** | Memory (same platform as G8264) |
| **Switching Capacity** | 1.28 Tbps |
| **Forwarding Rate** | 952 Mpps |
| **Latency** | ~2-4 us (copper PHY adds latency vs SFP+) |
| **MTU / Jumbo** | 9216 bytes |
| **Form Factor** | 1RU |
| **OS** | IBM ENOS or Lenovo CNOS; ONIE compatible |
| **Management** | CLI, SNMP, web GUI |
| **L2 Features** | VLAN, LACP, STP/RSTP/MSTP, vLAG |
| **L3 Features** | Static routes, OSPF, BGP, ECMP, VRRP, SVIs |
| **MC-LAG** | Yes (vLAG) |
| **Stacking** | No |
| **Class** | Enterprise / Data Center TOR |
| **Released** | ~2012 |
| **EOL** | ~2018 |
| **Notes** | Copper 10GBASE-T variant of G8264. Higher power consumption (~300-400W vs ~200W for SFP+ model) due to copper PHY. Uses standard Cat6a/Cat7 cabling. Useful for connecting servers without SFP+ NICs. |

---

### IBM RackSwitch G8316 (2x)

| Attribute | Value |
|---|---|
| **Ports** | 16x QSFP+ 40GbE |
| **Breakout** | 4x10GbE per QSFP+ (up to 64x 10GbE) |
| **ASIC** | Memory |
| **Switching Capacity** | 1.28 Tbps |
| **Forwarding Rate** | 952 Mpps |
| **Latency** | ~1-2 us |
| **MTU / Jumbo** | 9216 bytes |
| **Form Factor** | 1RU |
| **OS** | IBM ENOS or Lenovo CNOS; ONIE compatible |
| **Management** | CLI, SNMP, web GUI |
| **L2 Features** | VLAN, LACP, STP/RSTP/MSTP, vLAG |
| **L3 Features** | Static routes, OSPF, BGP, ECMP, VRRP, SVIs |
| **MC-LAG** | Yes (vLAG) |
| **Stacking** | No |
| **Class** | Enterprise / Data Center Spine/Aggregation |
| **Released** | ~2012 |
| **EOL** | ~2018 |
| **Notes** | Spine/aggregation switch designed to sit above G8264 TOR switches. All 40GbE ports, no 10GbE access ports. Intended for leaf-spine or aggregation tier. |

---

### IBM Mellanox SX6036 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 36x QSFP (FDR InfiniBand 56Gbps or 40GbE Ethernet) |
| **Breakout** | 4x10GbE per QSFP in Ethernet mode (up to 144x 10GbE) |
| **ASIC** | Mellanox SwitchX-2 |
| **Switching Capacity** | 4.0 Tbps (InfiniBand) / 2.88 Tbps (Ethernet) |
| **Forwarding Rate** | ~1,080 Mpps (Ethernet mode) |
| **Latency** | ~170 ns (InfiniBand), ~300 ns (Ethernet) |
| **MTU / Jumbo** | 9216 bytes (Ethernet), 4KB (InfiniBand) |
| **Form Factor** | 1RU |
| **OS** | MLNX-OS (Mellanox/NVIDIA Networking OS) |
| **Management** | CLI, SNMP, web GUI, REST API |
| **L2 Features** | VLAN, LACP, STP/RSTP, IGMP snooping (Ethernet mode) |
| **L3 Features** | Static routes, OSPF, BGP (Ethernet mode, varies by firmware) |
| **MC-LAG** | No (not natively in MLNX-OS for this generation) |
| **VPI** | Yes - Virtual Protocol Interconnect (can mix InfiniBand + Ethernet ports) |
| **Stacking** | No |
| **Class** | Enterprise / HPC |
| **Released** | ~2013 |
| **EOL** | ~2019 (Mellanox acquired by NVIDIA 2020) |
| **Notes** | Dual-personality switch: can run InfiniBand FDR (56Gb/s, RDMA native) or Ethernet (40GbE) per port. Extremely low latency. Originally designed for HPC clusters. In Ethernet mode it's a competent 40GbE switch. VPI allows mixed InfiniBand+Ethernet deployment if both are needed. |

---

### Arista DCS-7050QX-32-F (1x)

| Attribute | Value |
|---|---|
| **Ports** | 32x QSFP+ 40GbE (7050QX-32S variant adds 4x SFP+ 10GbE) |
| **Breakout** | 4x10GbE per QSFP+ (up to 128x 10GbE) |
| **ASIC** | Memory Memoria (Memory FM6000 series) |
| **Switching Capacity** | 2.56 Tbps |
| **Forwarding Rate** | 1,440 Mpps |
| **Latency** | 550 ns (sub-microsecond, cut-through) |
| **MTU / Jumbo** | 9216 bytes |
| **Form Factor** | 1RU, front-to-back airflow (-F suffix) |
| **OS** | Arista EOS (Extensible Operating System, Linux-based) |
| **Management** | CLI, eAPI (JSON-RPC), SNMP, CloudVision, OpenConfig/gNMI |
| **L2 Features** | VLAN (4094), LACP, STP/RSTP/MSTP, storm control |
| **L3 Features** | Static routes, OSPF, BGP, IS-IS, ECMP (64-way), VRRP, PBR, VRF |
| **MC-LAG** | Yes (MLAG - Arista Multi-Chassis LAG, pairs of 2) |
| **VXLAN** | Yes (hardware VTEP) |
| **Stacking** | No (uses MLAG/ECMP for multi-chassis) |
| **MAC Table** | 288K entries |
| **Route Table** | 208K IPv4 host routes |
| **Class** | Enterprise / Data Center |
| **Released** | ~2013 |
| **EOL** | ~2020 (EOS software support continues longer) |
| **Notes** | Top-tier data center switch with Arista EOS (best-in-class NOS). MLAG is rock-solid and well-documented. EOS is Linux underneath with full shell access. Extremely programmable via eAPI. SSU (Smart System Upgrade) for hitless upgrades. This is arguably the most capable switch in this inventory for general data center use. |

---

## Routers

### Mono Gateway Router (3x)

| Attribute | Value |
|---|---|
| **Ports** | 2x SFP+ 10GbE + 3x RJ45 1GbE per unit |
| **SoC** | NXP LS1046A (quad-core ARM Cortex-A72, 1.8GHz) |
| **Hardware Offload** | DPAA (Data Path Acceleration Architecture) for L3/NAT at near-line-rate |
| **Throughput** | ~10Gbps routing/NAT (hardware-accelerated) |
| **Latency** | ~10-50 us (DPAA hardware path) |
| **OS** | OpenWrt |
| **Management** | LuCI web GUI, UCI CLI, SSH |
| **L3 Features** | Static routes, NAT/masquerade, firewall (nftables), DHCP, DNS, VRRP (keepalived) |
| **VPN** | WireGuard, OpenVPN, IPsec (strongSwan) |
| **Bonding** | Yes (balance-tlb recommended for mixed SFP+/RJ45 speeds) |
| **Class** | Prosumer / SMB |
| **Manufacturer** | mono.si |
| **Released** | ~2021-2023 |
| **Notes** | Purpose-built OpenWrt routers with 10G SFP+ and hardware offload. DPAA acceleration means NAT/routing is not software-only. Three units available for edge + internal gateway redundancy (VRRP). |

---

### Cisco 2811 (2x)

| Attribute | Value |
|---|---|
| **Ports** | 2x GbE RJ45 (built-in) + HWIC/NM expansion slots |
| **CPU** | MIPS-based |
| **RAM** | 256MB-768MB (model dependent) |
| **OS** | Cisco IOS |
| **Management** | CLI (console/telnet/SSH), SNMP, SDM (web GUI with some images) |
| **L3 Features** | Static, OSPF, BGP, EIGRP, RIP, PBR, NAT, HSRP/VRRP, GRE, IPsec VPN |
| **Firewall** | IOS Firewall / ZBF (Zone-Based Firewall) |
| **Voice** | CME (CallManager Express) capable |
| **Class** | Enterprise Branch Router |
| **Released** | ~2005 |
| **EOL** | 2011 (End of Sale), 2016 (End of Support) |
| **Notes** | Classic ISR G1 (Integrated Services Router). Very capable L3 router for its era but ancient by modern standards. Throughput ~100-200Mbps with services enabled. Expansion slots (HWIC, NM, AIM) for serial, T1/E1, additional Ethernet, voice. Power-hungry for what it delivers today. |

---

### Cisco 1841 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 2x FastEthernet 10/100 RJ45 + 2x HWIC slots |
| **CPU** | MIPS-based |
| **RAM** | 128MB-384MB |
| **OS** | Cisco IOS |
| **Management** | CLI, SNMP |
| **L3 Features** | Static, OSPF, BGP, EIGRP, RIP, NAT, HSRP, GRE, IPsec VPN |
| **Class** | Enterprise Small Branch Router |
| **Released** | ~2005 |
| **EOL** | 2010 (End of Sale), 2015 (End of Support) |
| **Notes** | ISR G1. FastEthernet only (no GbE built-in). Max throughput ~40-80Mbps with services. Primarily useful as a lab/learning device for IOS. Not practical for production use at any modern traffic level. |

---

### Cisco 881 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 4x FastEthernet 10/100 LAN RJ45 + 1x FastEthernet 10/100 WAN RJ45 (some models have GbE WAN) |
| **CPU** | MIPS/ARM (Cavium) |
| **RAM** | 256MB |
| **OS** | Cisco IOS |
| **Management** | CLI, SNMP, CCP (web GUI) |
| **L3 Features** | Static, OSPF, EIGRP, NAT, DHCP, IPsec VPN |
| **Firewall** | IOS ZBF |
| **Class** | Enterprise Small Branch / SOHO |
| **Released** | ~2008 |
| **EOL** | ~2015 (End of Sale), ~2020 (End of Support) |
| **Notes** | ISR G1 compact form factor. Integrated 4-port FE switch. Designed for small branch/SOHO. Very limited throughput by modern standards (<100Mbps). Desktop form factor, no rack mount. |

---

## Managed Switches (10GbE+)

### Netgear ProSafe XS712T-100NES (1x)

| Attribute | Value |
|---|---|
| **Ports** | 12x 10GBASE-T RJ45 + 2x SFP+ 10GbE combo (shared with 2 of the 12 RJ45) |
| **Switching Capacity** | 240 Gbps |
| **Forwarding Rate** | 178 Mpps |
| **Latency** | ~3-5 us (10GBASE-T copper PHY) |
| **MTU / Jumbo** | 9198 bytes |
| **Form Factor** | 1RU (half-depth, fanless at low loads / quiet fans) |
| **OS** | Netgear Smart Managed firmware |
| **Management** | Web GUI, SNMP |
| **L2 Features** | VLAN (up to 256), LACP (up to 8 groups), STP/RSTP, IGMP snooping |
| **L3 Features** | None (L2 only) |
| **MC-LAG** | No |
| **Stacking** | No |
| **Class** | Prosumer / SMB |
| **Released** | ~2014 |
| **Status** | Discontinued but still widely available refurbished |
| **Notes** | One of the few affordable 10GBASE-T copper switches. 12 ports of 10G copper using standard Cat6a cabling. Smart managed (not full CLI). Good for connecting servers/NAS with 10GBASE-T NICs. No L3. No SFP+ dedicated uplinks (combo only). Higher power consumption than SFP+ equivalents due to 10GBASE-T PHY (~50-80W typical). |

---

### TRENDnet TEG-30284 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 24x Gigabit RJ45 + 4x 10G SFP+ |
| **Switching Capacity** | 128 Gbps |
| **Forwarding Rate** | 95.2 Mpps |
| **Latency** | ~3-5 us |
| **MTU / Jumbo** | 12,288 bytes |
| **Form Factor** | 1RU, fanless |
| **OS** | TRENDnet Web Smart firmware (TRENDnet Hive cloud optional) |
| **Management** | Web GUI, CLI (Telnet/SSH), SNMP v1/v2c/v3 |
| **L2 Features** | VLAN (256 groups, ID 1-4094), LACP, STP/RSTP/MSTP, port mirroring, IGMP snooping |
| **L3 Features** | IPv4/IPv6 static routing (up to 32 routes, 6 IP interfaces) |
| **MC-LAG** | No |
| **Stacking** | No |
| **Security** | 802.1X, RADIUS, TACACS+, ACLs (L2-L4), DHCP snooping, DAI, DoS defend |
| **Class** | Prosumer / SMB |
| **Released** | ~2018 |
| **Status** | Current (v2.5R) |
| **Notes** | Affordable L2+ switch with 10G SFP+ uplinks. Fanless = silent operation. Limited static routing. Good for small office aggregation or as an intermediate/management switch. 4x 10G SFP+ slots provide uplink capability to 10G fabric. |

---

## Managed Switches (1GbE / 2.5GbE)

### TP-Link Omada SG3210XHP-M2 (2x)

| Attribute | Value |
|---|---|
| **Ports** | 8x 2.5GBASE-T RJ45 PoE+ + 2x 10G SFP+ |
| **PoE Budget** | 240W (802.3at/af) |
| **Switching Capacity** | 80 Gbps |
| **Forwarding Rate** | 59.52 Mpps |
| **Latency** | ~3-5 us |
| **MTU / Jumbo** | 9,000 bytes |
| **Form Factor** | 1RU (half-depth) |
| **OS** | TP-Link Omada firmware (Omada SDN cloud managed) |
| **Management** | Web GUI, CLI, SNMP v1/v2c/v3, Omada Cloud Controller |
| **L2 Features** | VLAN (4K), LACP (8 groups x 8 ports), STP/RSTP/MSTP, IGMP snooping |
| **L3 Features** | Static routing (48 routes, 64 IP interfaces), DHCP server/relay |
| **MC-LAG** | No |
| **Stacking** | No |
| **Security** | 802.1X, RADIUS, TACACS+, ACLs, DHCP snooping, ARP inspection, port security |
| **Class** | Prosumer / SMB |
| **Released** | ~2022 |
| **Status** | Current |
| **Notes** | 2.5GbE PoE+ switch ideal for WiFi 6 APs and PoE devices. Omada SDN provides centralized cloud management across sites. 2x 10G SFP+ uplinks. Good for PoE access layer (cameras, APs, phones). |

---

### Dell PowerConnect 5448 (4x)

| Attribute | Value |
|---|---|
| **Ports** | 48x Gigabit RJ45 + 4x SFP combo (shared with 4 of the 48 RJ45) |
| **Switching Capacity** | 96 Gbps |
| **Forwarding Rate** | 71.4 Mpps |
| **Latency** | ~5-10 us |
| **MTU / Jumbo** | 9,216 bytes |
| **Form Factor** | 1RU |
| **OS** | Dell Networking OS (proprietary web/CLI managed) |
| **Management** | Web GUI, CLI (console/telnet/SSH), SNMP |
| **L2 Features** | VLAN (up to 256), LACP, STP/RSTP, port mirroring, IGMP snooping |
| **L3 Features** | None (L2 only) |
| **MC-LAG** | No |
| **Stacking** | Yes (proprietary stacking ports/cables, up to 12 units) |
| **Stack Bandwidth** | Up to 48 Gbps stacking backplane (model-dependent) |
| **Class** | Prosumer / SMB |
| **Released** | ~2007 |
| **EOL** | ~2012 |
| **Notes** | Basic 48-port GbE managed switch. No 10G uplinks (SFP is 1GbE only). Stackable for management simplification — up to 12 units managed as a single logical switch. Four units means 192 GbE ports if stacked. Stacking uses dedicated rear-panel ports and proprietary cables. |

---

### Cisco SG300-52 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 50x Gigabit RJ45 + 2x combo GbE/SFP |
| **Switching Capacity** | 104 Gbps |
| **Forwarding Rate** | 77.4 Mpps |
| **Latency** | ~5-10 us |
| **MTU / Jumbo** | 9,000 bytes |
| **Form Factor** | 1RU |
| **OS** | Cisco Small Business firmware |
| **Management** | Web GUI, CLI (console/telnet/SSH), SNMP v1/v2c/v3 |
| **L2 Features** | VLAN (4094), LACP (8 groups), STP/RSTP/MSTP, port mirroring, IGMP snooping |
| **L3 Features** | Static routing (IPv4/IPv6), inter-VLAN routing (with L3 mode enabled) |
| **MC-LAG** | No |
| **Stacking** | No |
| **Security** | 802.1X, RADIUS, TACACS+, ACLs (L2-L4), DHCP snooping, ARP inspection |
| **Class** | SMB |
| **Released** | ~2010 |
| **EOL** | ~2017 |
| **Notes** | Cisco Small Business line (not Catalyst). Decent L2/L3-lite switch. Can do inter-VLAN routing in L3 mode (limited static routes, no dynamic routing). 52 ports total is good density. No 10G. Separate product line from enterprise Catalyst. |

---

### NETGEAR ProSAFE GS116E (1x)

| Attribute | Value |
|---|---|
| **Ports** | 16x Gigabit RJ45 |
| **Switching Capacity** | 32 Gbps |
| **Forwarding Rate** | 23.8 Mpps |
| **Latency** | ~3-5 us |
| **MTU / Jumbo** | 9,216 bytes |
| **Form Factor** | Desktop (wall-mountable, not rackmount) |
| **OS** | Netgear Plus firmware |
| **Management** | Web GUI (ProSafe Plus utility), limited SNMP |
| **L2 Features** | VLAN (up to 64), basic QoS (802.1p), IGMP snooping, port mirroring |
| **L3 Features** | None |
| **MC-LAG** | No |
| **LACP** | No |
| **Stacking** | No |
| **Class** | Consumer / Prosumer (Plus managed) |
| **Released** | ~2013 |
| **Status** | Discontinued (replaced by GS116Ev2/GS316E) |
| **Notes** | Basic "Plus" managed switch - step above unmanaged but far below fully managed. No CLI, no LACP, limited VLAN support. Fanless, silent, low power (~8W). Best suited as a desk/lab switch or for extending ports in low-traffic areas. |

---

## Cisco Catalyst Switches

### Cisco Catalyst 3560 (1x)

| Attribute | Value |
|---|---|
| **Ports** | Model-dependent: typically 24x or 48x GbE RJ45 + 4x SFP 1GbE (3560G models) |
| **Switching Capacity** | Up to 32/68 Gbps (model dependent) |
| **Forwarding Rate** | Up to 38.7/65.5 Mpps |
| **Latency** | ~3-6 us |
| **MTU / Jumbo** | 9,198 bytes |
| **Form Factor** | 1RU |
| **OS** | Cisco IOS (IP Base or IP Services license) |
| **Management** | CLI (console/telnet/SSH), SNMP, web GUI (optional) |
| **L2 Features** | VLAN (4094), LACP, STP/RSTP/MSTP, port security, storm control |
| **L3 Features** | Full L3 routing with IP Services: OSPF, EIGRP, BGP, static, PBR, HSRP/VRRP, inter-VLAN routing |
| **MC-LAG** | No |
| **PoE** | Model-dependent (3560-xxPS models have PoE) |
| **Stacking** | No (3560 is standalone; 3750 is the stackable variant) |
| **Class** | Enterprise |
| **Released** | ~2004 (3560), ~2006 (3560G), ~2010 (3560X) |
| **EOL** | 3560: ~2008, 3560G: ~2012, 3560X: ~2016 |
| **Notes** | Classic Cisco enterprise L3 switch. The 3560 series spans multiple hardware generations. With IP Services license, it's a full L3 router-switch. Without exact model number, specs are approximate. The "G" suffix means Gigabit throughout; original 3560 had FastEthernet models. |

---

### Cisco Catalyst 2960 (1x)

| Attribute | Value |
|---|---|
| **Ports** | Model-dependent: typically 24x or 48x GbE RJ45 + 2-4x SFP 1GbE |
| **Switching Capacity** | Up to 16/32/100 Gbps (model dependent) |
| **Latency** | ~3-6 us |
| **MTU / Jumbo** | 9,198 bytes (2960G/S/X models) |
| **Form Factor** | 1RU |
| **OS** | Cisco IOS (LAN Base or LAN Lite) |
| **Management** | CLI (console/telnet/SSH), SNMP, web GUI |
| **L2 Features** | VLAN, LACP, STP/RSTP/MSTP, port security |
| **L3 Features** | None (L2 only; 2960-S/X have limited static routing) |
| **MC-LAG** | No |
| **Stacking** | 2960-S: FlexStack (up to 4 units, 20 Gbps). 2960-X: FlexStack-Plus (up to 8 units, 80 Gbps). Original 2960: No |
| **PoE** | Model-dependent |
| **Class** | Enterprise Access Layer |
| **Released** | ~2006 (original), ~2010 (2960-S), ~2013 (2960-X) |
| **EOL** | Original: ~2013, 2960-S: ~2016, 2960-X: ~2019 |
| **Notes** | Workhorse Cisco L2 access switch. Very common in enterprise environments. L2 only (no routing). Used for connecting end devices to the network. Without exact model number, specs are approximate. |

---

## Security / Specialty Appliances

### Cisco ASA 5505 (1x)

| Attribute | Value |
|---|---|
| **Ports** | 8x FastEthernet 10/100 RJ45 (integrated switch, 2x PoE) |
| **Throughput** | 150 Mbps (firewall), 100 Mbps (VPN) |
| **Concurrent Sessions** | 10,000 (base license), 25,000 (Security Plus) |
| **OS** | Cisco ASA OS |
| **Management** | CLI (console/telnet/SSH), ASDM (Java web GUI), SNMP |
| **Features** | Stateful firewall, NAT, VPN (IPsec, SSL), VLAN (trunk capable with Security Plus license) |
| **Class** | Enterprise SOHO / Small Branch |
| **Released** | ~2006 |
| **EOL** | 2013 (End of Sale), 2017 (End of Support) |
| **Notes** | Entry-level ASA firewall. FastEthernet only. Very limited by modern standards (~150Mbps). VLAN trunk requires Security Plus license. Desktop form factor. Useful only for lab/learning or very low-bandwidth firewall scenarios. |

---

### Cisco 4402 Wireless LAN Controller (1x)

| Attribute | Value |
|---|---|
| **Ports** | 4x GbE RJ45 (2x distribution system ports + 2x service ports) |
| **AP Support** | Up to 50 lightweight access points (license-dependent, base: 12 or 25) |
| **OS** | Cisco AireOS |
| **Management** | CLI, web GUI, SNMP, WCS/Prime Infrastructure |
| **Features** | Centralized WLAN management, 802.11a/b/g, LWAPP/CAPWAP, rogue AP detection, RF management |
| **Class** | Enterprise Wireless Infrastructure |
| **Released** | ~2006 |
| **EOL** | ~2012 (End of Sale), ~2017 (End of Support) |
| **Notes** | This is a wireless LAN controller, not a switch or router. Manages lightweight Cisco APs (not autonomous). Only relevant if you have Cisco lightweight APs. 802.11a/b/g era - does NOT support 802.11n/ac/ax. Essentially useless for modern WiFi. |

---

### Cisco 2811 - see [Routers section above](#cisco-2811-2x)
### Cisco 1841 - see [Routers section above](#cisco-1841-1x)
### Cisco 881 - see [Routers section above](#cisco-881-1x)

---

## ISP CPE

### Calix GP1101X GigaPoint ONT (1x)

| Attribute | Value |
|---|---|
| **Ports** | 1x 10GBASE-T RJ45 (LAN) + 1x POTS RJ11 (VoIP) |
| **WAN** | XGS-PON SC/APC fiber (10G symmetric) |
| **LAN Speed** | 10 Gbps (10GBASE-T) |
| **PON Standard** | XGS-PON (ITU-T G.9807.1), 10G/10G symmetric |
| **Form Factor** | Small indoor desktop unit, wall-mountable |
| **OS** | Calix firmware (ISP-managed, not user-configurable) |
| **Management** | ISP-managed via Calix Smart Activate / AXOS; limited local status LEDs |
| **L2 Features** | 802.1Q VLAN tagging (ISP-configured), QoS |
| **L3 Features** | None (bridge mode / L2 transparent) |
| **Power** | 12V DC adapter, optional battery backup |
| **Class** | ISP CPE |
| **Manufacturer** | Calix (GigaPoint series) |
| **Released** | ~2022 |
| **Notes** | XGS-PON ONT with single 10GbE copper output. The GP1101X is the 10GE variant (GP1100X is 2.5GE). Operates as a transparent L2 bridge from fiber to Ethernet. Single LAN port means all downstream routing/switching must happen on the next device. ISP-locked firmware - no user configuration of the ONT itself. VoIP port for carrier-grade telephony if subscribed. |

---

## Stacking Capability Reference

| Device | Stacking | Technology | Max Stack Size | Notes |
|---|---|---|---|---|
| **Dell PowerConnect 5448** | Yes | Proprietary stacking ports/cables | Up to 12 units | Single management IP for the stack, shared config |
| **Cisco 2960-S/X** | Yes | FlexStack / FlexStack-Plus | Up to 4-8 units | Original 2960 (non-S/X): No stacking. S/X variants only |
| **Cisco 3560** | No | N/A | N/A | 3750 is the stackable variant of this generation |
| **IBM G8264** | No | Uses vLAG (MC-LAG) instead | 2 (vLAG pair) | vLAG provides multi-chassis redundancy, not stacking |
| **IBM G8264e** | No | Uses vLAG instead | 2 (vLAG pair) | Same as G8264 |
| **IBM G8316** | No | Uses vLAG instead | 2 (vLAG pair) | Same as G8264 |
| **Celestica DX010** | No | Uses MC-LAG instead | 2 (MC-LAG pair) | SONiC MC-LAG for multi-chassis |
| **Arista 7050QX-32-F** | No | Uses MLAG instead | 2 (MLAG pair) | MLAG for multi-chassis, ECMP for beyond 2 |
| **Mellanox SX6036** | No | N/A | N/A | No stacking or MC-LAG |
| All others | No | N/A | N/A | — |

> **Stacking vs MC-LAG/MLAG/vLAG:** Stacking merges multiple physical switches into one logical switch with a single management plane and shared forwarding table. MC-LAG/MLAG/vLAG keeps switches as independent control planes but coordinates LAG membership across two chassis for link redundancy. Both achieve multi-chassis redundancy but stacking is tighter coupling (single config) while MC-LAG is looser (independent configs with coordination protocol).

---

## Summary Table

| Device | Qty | Max Port Speed | Total High-Speed Ports | Managed | L3 | MC-LAG | Stacking | Class | Era |
|---|---|---|---|---|---|---|---|---|---|
| **Celestica DX010** | 4 | 100GbE | 32x QSFP28 | Yes | Yes | Yes | No | DC | 2016 |
| **Arista 7050QX-32-F** | 1 | 40GbE | 32x QSFP+ | Yes | Yes | Yes (MLAG) | No | DC | 2013 |
| **IBM Mellanox SX6036** | 1 | 56G IB / 40GbE | 36x QSFP | Yes | Limited | No | No | HPC/DC | 2013 |
| **IBM G8316** | 2 | 40GbE | 16x QSFP+ | Yes | Yes | Yes (vLAG) | No | DC Spine | 2012 |
| **IBM G8264** | 3 | 10GbE / 40GbE | 48x SFP+ + 4x QSFP+ | Yes | Yes | Yes (vLAG) | No | DC TOR | 2012 |
| **IBM G8264e** | 1 | 10GbE / 40GbE | 48x 10GBASE-T + 4x QSFP+ | Yes | Yes | Yes (vLAG) | No | DC TOR | 2012 |
| **Mono Gateway** | 3 | 10GbE | 2x SFP+ + 3x 1G | Yes | Yes | No | No | Router | 2022 |
| **Calix GP1101X** | 1 | 10GbE | 1x 10GBASE-T | No | No | No | No | ISP CPE | 2022 |
| **Netgear XS712T** | 1 | 10GbE | 12x 10GBASE-T | Smart | No | No | No | Prosumer | 2014 |
| **TRENDnet TEG-30284** | 1 | 10GbE | 4x SFP+ | Yes | L2+ | No | No | Prosumer | 2018 |
| **TP-Link SG3210XHP-M2** | 2 | 10GbE | 2x SFP+ | Yes | L2+ | No | No | Prosumer | 2022 |
| **Cisco SG300-52** | 1 | 1GbE | 2x SFP combo | Yes | L3-lite | No | No | SMB | 2010 |
| **Dell PC 5448** | 4 | 1GbE | 4x SFP combo | Yes | No | No | **Yes (12)** | Prosumer | 2007 |
| **Cisco 3560** | 1 | 1GbE | 4x SFP | Yes | Yes | No | No | Enterprise | 2004+ |
| **Cisco 2960** | 1 | 1GbE | 2-4x SFP | Yes | No | No | **S/X only** | Enterprise | 2006+ |
| **Netgear GS116E** | 1 | 1GbE | None | Plus | No | No | No | Consumer | 2013 |
| **Cisco 2811** | 2 | 1GbE | 2x RJ45 | Yes | Yes | No | No | Router | 2005 |
| **Cisco 1841** | 1 | 100Mbps | 2x FE | Yes | Yes | No | No | Router | 2005 |
| **Cisco 881** | 1 | 100Mbps | 5x FE | Yes | Yes | No | No | Router | 2008 |
| **Cisco ASA 5505** | 1 | 100Mbps | 8x FE | Yes | Firewall | No | No | Firewall | 2006 |
| **Cisco 4402 WLC** | 1 | 1GbE | 4x RJ45 | Yes | N/A | No | No | WLAN Ctrl | 2006 |

---

## References

> **Note on link availability:** Many of the devices in this inventory are end-of-life (EOL)
> or discontinued. Cisco [deliberately removes all documentation for retired products][cisco-retired].
> IBM/Lenovo has migrated legacy docs to Lenovo Press but coverage is incomplete.
> Mellanox documentation was absorbed into NVIDIA's portal and many legacy PDFs are gone.
> Where original manufacturer documentation is no longer available, we link to archived copies
> (web.archive.org), third-party datasheets, or community resources. Devices with no surviving
> documentation are listed with a note explaining what was attempted.
>
> [cisco-retired]: https://www.cisco.com/c/en/us/obsolete/routers/cisco-2811-integrated-services-router.html

### Celestica Haliburton (DX010)

1. [ServeTheHome — Inside a Celestica Seastone DX010 32x 100GbE Switch](https://www.servethehome.com/inside-a-celestica-seastone-dx010-32x-100gbe-switch/) — Hardware teardown with photos and component analysis
2. [ServeTheHome Forums — PSA: SONiC builds on Celestica DX010](https://forums.servethehome.com/index.php?threads/psa-new-builds-of-older-sonic-versions-no-longer-work-on-the-celestica-dx010.41603/page-2) — Community discussion on SONiC compatibility and firmware issues
3. [YouTube — Celestica DX010 Overview](https://www.youtube.com/watch?v=fkc2pFFGCtE) — Video overview of the DX010 switch
4. [YouTube — DX010 Initial Setup With SONiC](https://www.youtube.com/watch?v=MJzfOVnbZf8) — Walkthrough of initial SONiC installation and setup on the DX010
5. [YouTube — 100GbE Homelab: DX010 + Mellanox ConnectX-4](https://www.youtube.com/watch?v=2gs1gK2F0UE) — 100GbE homelab build with DX010, ConnectX-4, and QSFP28 DAC cables
6. [YouTube — 100GbE Homelab Cable Choices](https://www.youtube.com/watch?v=_RmLXMPNRl8) — QSFP28/QSFP+ breakout, module, and fiber options for the DX010
7. [SONiC GitHub — Celestica Platform Modules](https://github.com/sonic-net/sonic-buildimage/tree/master/platform/broadcom/sonic-platform-modules-cel) — SONiC platform driver source code for Celestica switches (dx010, haliburton directories)
8. [SONiC GitHub — DX010 Platform Definition (platform.json)](https://github.com/sonic-net/sonic-buildimage/blob/master/device/celestica/x86_64-cel_seastone-r0/platform.json) — Hardware spec: 32x QSFP28, 5 fan drawers, 2 PSUs, breakout modes (1x100G, 2x50G, 4x25G)
9. [SONiC GitHub — PR #3775: Enable FEC RS by Default for 100G](https://github.com/sonic-net/sonic-buildimage/pull/3775) — Pull request to enable Forward Error Correction (RS) by default for 100G ports in SONiC
10. [SONiC Foundation](https://sonicfoundation.dev/) — SONiC project home at the Linux Foundation
11. [SONiC Wiki — Supported Devices & Architecture](https://github.com/sonic-net/SONiC/wiki) — Supported platforms list, building guides, and architecture documentation
12. [SONiC Dev Mailing List — DX010 Discussions](https://lists.sonicfoundation.dev/g/sonic-dev/messages?msgnum=35) — Groups.io mailing list with threads about DX010 SFP ports, SONiC releases, and running SONiC in Docker
13. [STH Forums — DX010 Intel Avoton C2358 AVR54 C0 Stepping Failure](https://forums.servethehome.com/index.php?threads/celestica-dx010-100gbe-switch-w-intel-avoton-c2358-cpu-avr54-c0-stepping-failure.34912/) — Discussion of the Intel Atom C2000 AVR54 silicon bug affecting DX010 switches
14. [STH Forums — 100 Gbps Ethernet Switch $1000 New](https://forums.servethehome.com/index.php?threads/100-gbps-ethernet-switch-1000-new.22994/) — Thread about Celestica Seastone DX010 32x100G switches available for ~$1000
15. [STH Forums — Getting 100GbE Link Between DX010 and Mellanox ConnectX-4](https://forums.servethehome.com/index.php?threads/getting-a-100gbe-link-between-celstica-dx010-and-mellanox-connectx-4.32981/) — Troubleshooting 100GbE link-up between DX010 and CX-4 (solution: set FEC to RS)
16. [STH Forums — Help with Seastone DX010](https://forums.servethehome.com/index.php?threads/help-with-seastone-dx010.33822/) — Getting started with DX010: console setup, OS loading, initial configuration
17. [STH Forums — DX010 Replacement Fans & PSUs](https://forums.servethehome.com/index.php?threads/celestica-dx010-replacement-fans-psus.42616/) — Sourcing replacement fans and power supplies for the DX010
18. [STH Forums — Mellanox Support Contract for CX4/100G](https://forums.servethehome.com/index.php?threads/anybody-w-mellanox-support-contract.24613/) — Finding Mellanox support to debug ConnectX-4 / 100G switch link-up issues
19. [STH Forums — Celestica D4040](https://forums.servethehome.com/index.php?threads/celestica-d4040.24256/) — Discussion of running ICOS/Cumulus/SONiC on the related Celestica D4040 40GbE switch
20. [STH Forums — Celestica D4040 (page 4: C2000 LPC Bug)](https://forums.servethehome.com/index.php?threads/celestica-d4040.24256/page-4) — Serial console troubleshooting, C2000 LPC clock bug, USB recovery procedures
21. [STH Forums — Mellanox Switches Tips & Tricks (page 19)](https://forums.servethehome.com/index.php?threads/mellanox-switches-tips-tricks.39394/page-19) — Cumulus/ONYX firmware guidance and configuration for Mellanox switches
22. [STH Forums — 40GbE Throughput Troubleshooting](https://forums.servethehome.com/index.php?threads/cant-get-more-than-20gbps-out-of-a-40gbe-network-suggestions.11448/) — Troubleshooting 40GbE throughput capped at 21Gbps
23. [Reddit r/homelab — Initial Configuration of a Celestica DX010 100GE Switch](https://old.reddit.com/r/homelab/comments/n5opo2/initial_configuration_of_a_celestica_dx010_100ge/) — Detailed guide: SONiC install, L2 config, breakout, fan control
24. [Reddit r/homelab — DX010 100GE Switch](https://www.reddit.com/r/homelab/comments/tdeh78/dx010_100ge_switch/) — Community discussion about the DX010
25. [Reddit r/homelab — Celestica Seastone DX010 Questions](https://www.reddit.com/r/homelab/comments/udq1vx/celestica_seastone_dx010_questions_about_how_to/) — Q&A about DX010 setup and configuration
26. [Reddit r/homelab — Does Anybody Have a Celestica D4040 with ICOS?](https://www.reddit.com/r/homelab/comments/16o1vtw/does_anybody_have_a_celestica_d4040_with_icos/) — Discussion of ICOS on the related Celestica D4040
27. [Intel Atom C2000 Family Specification Update (PDF)](https://www.intel.com/content/dam/www/public/us/en/documents/specification-updates/atom-c2000-family-spec-update.pdf) — Official Intel errata document covering the AVR54 C0 stepping bug that affects DX010 switches
28. [LinkedIn — Minimal OS for PCI Device Listing (Atom C2000 Diagnosis)](https://www.linkedin.com/posts/danielesalvatorealbano_os-c-pci-activity-6931012539417866240-R6H9/) — Post about writing a minimal OS to diagnose Atom C2000 AVR54 bug via PCI device registry dumps
29. [GitHub — list-pci-devices-os](https://github.com/danielealbano/list-pci-devices-os) — Minimal OS that dumps PCI device registries on VGA/serial terminals, useful for diagnosing C2000 AVR54 bug
30. [STH Forums — Celestica Seastone DX010 32Port 100G QSFP28 $250](https://forums.servethehome.com/index.php?threads/celestica-seastone-dx010-32port-100g-qsfp28-250.42935/) — Deal thread: DX010 switches at $250 (normally $1000–1500), price-per-Gbps comparison to 40GbE
31. [STH Forums — Inside a Celestica Seastone DX010 32x 100GbE Switch (forum thread)](https://forums.servethehome.com/index.php?threads/inside-a-celestica-seastone-dx010-32x-100gbe-switch.31226/) — Forum discussion companion to the STH teardown article
32. [The Server Store — Celestica Seastone DX010](https://www.theserverstore.com/celestica-seastone-dx010-32-port-100g-onie-switch-.html) — Marketplace listing with specs and pricing (~$630)
33. [eBay — Celestica Seastone DX010](https://www.ebay.com/p/21040159119) — eBay product page with new/used listings ($600–$790 range)
34. [Network Outlet — Celestica Seastone DX010](https://networkoutlet.com/products/celestica-seastone-dx010-32-port-100g-qsfp28-onie-switch) — Marketplace listing with breakout specs (4x10G, 2x50G, 4x25G per port)

#### Dead/Unresolvable Links

- ~~[ServeTheHome — Celestica Haliburton DX010 Teardown (old URL)](https://www.servethehome.com/celestica-haliburton-dx010-teardown/)~~ — Original teardown URL (moved/renamed, 404; replaced by link #1 above)
- ~~[Broadcom BCM56960 Product Page](https://www.broadcom.com/products/ethernet-connectivity/switching/memory-memories-interfaces)~~ — Memory & Interfaces product family including BCM56960 (Broadcom reorganized site, 404)
- ~~[Azure SONiC Project](https://azure.github.io/SONiC/)~~ — Old SONiC documentation URL (moved to Linux Foundation, replaced by sonicfoundation.dev)
- ~~[OCP Networking — SONiC](https://www.opencompute.org/projects/onic)~~ — Open Compute Project SONiC resources (OCP reorganized, 404)

### IBM/Lenovo RackSwitch G8264

1. [Lenovo Press TIPS1272 — RackSwitch G8264 Product Guide](https://lenovopress.lenovo.com/tips1272) — Comprehensive product guide with specs, components, transceivers, and configuration (withdrawn product)
2. [Lenovo Press TIPS1272 — RackSwitch G8264 Product Guide (direct PDF, 29 pages)](https://lenovopress.lenovo.com/tips1272.pdf) — Same content as above in downloadable PDF format
3. [Reddit r/homelab — Question About the IBM/Lenovo G8264 Switch](https://www.reddit.com/r/homelab/comments/1b8z9kz/question_about_the_ibmlenovo_g8264_switch/) — Community discussion about acquiring and configuring the G8264
4. [Reddit r/HomeNetworking — Deploying 10Gbe / 40Gbe Through the Home — IBM RackSwitch G8264?](https://www.reddit.com/r/HomeNetworking/comments/uvhaqz/deploying_10gbe_40gbe_through_the_home_ibm/) — User evaluating the G8264 for home 10G/40G deployment with community advice on noise, power, and alternatives
5. [Lenovo Press TIPS0815 — RackSwitch G8264 Product Guide (IBM era)](https://lenovopress.lenovo.com/tips0815) — IBM-era product guide covering the G8264 as a port-aggregation and data center switch (withdrawn product)
6. [IBM Support — RackSwitch G8264 Installation Guide](https://www.ibm.com/support/pages/ibm-rackswitch-g8264-installation-guide) — Installation instructions for the G8264R (Type 7309) and G8264F (Types 1455) including rack mounting, cabling, and initial setup
7. [IBM — RackSwitch G8264 N/OS Application Guide (PDF, 640 pages)](https://www.ibm.com/support/pages/system/files/support/isg/isgdocs.nsf/0/e042c09ee2b5fe0785257d0700727c3c/$FILE/G8264_AG_7-9.pdf) — Comprehensive N/OS 7.9 application guide covering switch management, VLANs, link aggregation, routing, and CLI configuration
8. [IBM Support — BNT RackSwitch G8264 ISCLI Command Reference Guide 6.4](https://www.ibm.com/support/pages/ibm-bnt-rackswitch-g8264-iscli-command-reference-guide-64) — BLADE OS 6.4 ISCLI command reference for configuring and managing the G8264
9. [ServeTheHome Forums — IBM RackSwitch G8264 Questions](https://forums.servethehome.com/index.php?threads/ibm-rackswitch-g8264-questions.21944/) — Community discussion on fan replacement, boot issues, and noise reduction for the G8264
10. [IBM Support — RackSwitch G8264 Application Guide (N/OS 7.11)](https://www.ibm.com/support/pages/ibm-rackswitch-g8264-application-guide-711) — Landing page for the IBM Networking OS 7.11 application guide (newer than the 7.9 PDF above)
11. [IBM — RackSwitch G8264 Installation Guide (PDF, 96 pages)](https://www.ibm.com/support/pages/system/files/support/isg/isgdocs.nsf/0/aa3de36aa683848d85257a4b0075d56c/$FILE/G8264_Install.pdf) — Detailed hardware installation guide covering rack mounting, cabling, LED indicators, and initial bootstrap

#### Dead/Unresolvable Links

- ~~[DirectIndustry — IBM RackSwitch G8264 Datasheet](https://pdf.directindustry.com/pdf/ibm/system-networking-rackswitch-g8264/27444-337791.html)~~ — Full datasheet PDF (HTTP 410 Gone)
- ~~[ManualsLib — IBM RackSwitch G8264](https://www.manualslib.com/brand/ibm/?q=G8264)~~ — Generic IBM brand page; query parameter does not filter to G8264-specific content
- ~~[karma-group.ru — IBM G8264 Product Brief (PDF)](https://karma-group.ru/upload/iblock/d1e/IBM_System_Networking_RackSwitch_G8264.pdf)~~ — Marketing product brief (404)
- ~~[eyo.com.au — IBM G8264 Brochure (PDF)](https://www.eyo.com.au/wp-content/uploads/2015/08/IBM-System-Networking-RackSwitch-G8264.pdf)~~ — Product brochure with diagrams (404)

### IBM/Lenovo RackSwitch G8264e

The G8264e does not have a dedicated Lenovo Press product guide. It is a variant of the G8264 family with enhanced 10GbE SFP+ port density.

1. [Lenovo Press — RackSwitch G8264 Product Guide (TIPS1272)](https://lenovopress.lenovo.com/tips1272) — G8264 family documentation (closest match; no G8264e-specific guide exists)
2. [Lenovo Press — RackSwitch G8264CS Product Guide (TIPS1273)](https://lenovopress.lenovo.com/tips1273) — G8264CS Converged Switch variant (36 SFP+, 12 Omni Ports with FC, 4x 40G QSFP+) — **not** the G8264e but a related G8264 family member
3. [ALSO Holding AG — Lenovo RackSwitch G8264 Product Brief (PDF, 25 pages)](https://www.also.com/ec/cms5/media/documents/6110/microsites_5/lenovo_5/network_bnt/neue_pdf_s_1/lenovo_rackswitch_g8264_neu.pdf) — Distributor-hosted Lenovo product brief covering G8264 family specs, features, SDN/OpenFlow support, and deployment scenarios
4. [DirectIndustry — IBM RackSwitch G8264 Product Catalog](https://pdf.directindustry.com/pdf/ibm/g8264/15019-589614.html) — Online product catalog with specs, port layout, and OpenFlow/SDN features for the G8264 family
5. [IBM — Networking OS 7.11 Application Guide for RackSwitch G8264 (PDF, 606 pages)](https://www.ibm.com/support/pages/system/files/support/isg/isgdocs.nsf/0/be457ba00a7e386685257d9a0038227e/$FILE/G8264_AG_7-11.pdf) — Comprehensive N/OS 7.11 application guide covering switch management, VLANs, link aggregation, routing, and CLI configuration (applies to G8264e as same firmware family)
6. [karma-group.ru — IBM RackSwitch G8264 Product Brief (PDF, 4 pages)](https://www.karma-group.ru/upload/iblock/04e/XSD03108USEN.22a412cbf0f0f4d8449fcecc935cfe2fc0.pdf) — IBM marketing product brief with G8264 family specs, port layout, and feature highlights

#### Dead/Unresolvable Links

- ~~[Acclinet — IBM RackSwitch G8264 Switch Product Page](https://acclinet.com/ibm-product/ibm-rackswitch-g8264-switch.asp)~~ — Third-party specs page with bandwidth and latency details for the G8264 family (SSL certificate expired)

### IBM/Lenovo RackSwitch G8316

1. [Lenovo Press TIPS0842 — RackSwitch G8316 Product Guide (PDF, 19 pages)](https://lenovopress.lenovo.com/tips0842) — Comprehensive product guide with specs, part numbers, transceivers, and configuration (withdrawn product)
2. [IBM Support — Overview: IBM System Networking RackSwitch G8316](https://www.ibm.com/support/pages/overview-ibm-system-networking-rackswitch-g8316) — Technical overview with physical specs, part numbers, and warranty info
3. [karma-group.ru — IBM RackSwitch G8316 Datasheet (PDF)](https://www.karma-group.ru/upload/iblock/075/ibm_rackswitch_g8316_datasheet.36FB11CA32564FA8ABFCCC6287CF898C.pdf) — Official IBM datasheet PDF
4. [IBM — Networking OS 7.4 for RackSwitch G8316 Release Notes (PDF)](https://download4.boulder.ibm.com/sar/CMA/SNA/03cok/2/G8316_RN_7-4.pdf) — Firmware release notes with port specifications and supported features
5. [IT Jungle — IBM Launches 40 Gigabit Ethernet Rack Switch (2011)](https://www.itjungle.com/2011/10/31/tfh103111-story08/) — Launch announcement with pricing ($35,999) and technical overview
6. [IBM Support — RackSwitch 40G G8316 Firmware Update v6.8.4.0](https://www.ibm.com/support/pages/ibm-rackswitch-40g-g8316-firmware-update-v6840-ibm-bladecentersystem-networking) — Latest firmware download for the G8316 (BLADE OS 6.8.4.0)
7. [Chelsio — 40Gb Ethernet: A Competitive Alternative to InfiniBand (PDF)](https://www.chelsio.com/wp-content/uploads/2013/11/40Gb-Ethernet-A-Competitive-Alternative-to-InfiniBand.pdf) — Technical whitepaper benchmarking 40GbE (including G8316) against InfiniBand for data center workloads

### IBM/Mellanox SX6036

1. [Mellanox SX6036 Product Brief (PDF, archived)](https://web.archive.org/web/20210124132727/https://www.mellanox.com/related-docs/prod_ib_switch_systems/PB_SX6036.pdf) — Official product brief via Wayback Machine (Mellanox site absorbed by NVIDIA, originals removed)
2. [ManualsLib — Mellanox SX60 Series](https://www.manualslib.com/brand/mellanox/) — Mellanox manuals index including SX60 series and MSX6036G gateway variant manuals
3. [NVIDIA InfiniBand OFED Documentation — Fabric Utilities](https://docs.nvidia.com/networking/display/MLNXOFEDv461000/InfiniBand+Fabric+Utilities) — FDR InfiniBand technology reference (covers the FDR 56Gbps technology used in the SX6036)
4. [ServeTheHome Forums — Mellanox Switches Tips & Tricks (31-page mega-thread)](https://forums.servethehome.com/index.php?threads/mellanox-switches-tips-tricks.39394/) — Community thread covering firmware versions, SSD replacement, VPI configuration, recovery procedures, and general Mellanox switch guidance
5. [Reddit r/homelab — Fun with a Mellanox SX6036 40Gb Switch](https://www.reddit.com/r/homelab/comments/1qay6sc/fun_with_a_mellanox_sx6036_40gb_switch/) — Fan replacement (Noctua swap), firmware flashing, and IB-to-Ethernet license conversion
6. [Reddit r/homelab — Mellanox SX6036 Switch License Guidance Needed](https://www.reddit.com/r/homelab/comments/m29so9/mellanox_sx6036_switch_license_guidance_needed/) — Community discussion on Ethernet license options and pricing for SX6036
7. [NVIDIA Firmware Archive — Lenovo/Mellanox](https://network.nvidia.com/support/firmware/lenovo-archive/) — MLNX-OS firmware images for SX6036 and other Mellanox switches
8. [InfiniBand Trade Association — SX6036 Product Brief (PDF)](https://cw.infinibandta.org/files/showcase_product/120330.104655.244.PB_SX6036.pdf) — IBTA-hosted copy of the Mellanox SX6036 product brief with specs and architecture overview
9. [StorageReview — Mellanox SX6036 56Gb InfiniBand Switch Review](https://www.storagereview.com/review/mellanox-sx6036-56gb-infiniband-switch-review) — Professional review with performance benchmarks, 4.032 Tb/s non-blocking bandwidth, 170ns latency measurements, and pros/cons analysis
10. [NVIDIA — SX60XX 1U Switch and Gateway Hardware User Manual (PDF, 112 pages)](https://network.nvidia.com/pdf/user_manuals/1U_HW_UM_SX60XX.pdf) — Complete hardware user manual covering installation, cabling, LED indicators, power supplies, and fan modules for SX6036 and related models
11. [ServeTheHome Forums — Mellanox SX6036 Fan Mod](https://forums.servethehome.com/index.php?threads/mellanox-sx6036-fan-mod.35316/) — Community guide on replacing stock fans with quieter models, RPM readings, and thermal management tips
12. [ServeTheHome Forums — US Mellanox SX6036 $200 Deal Thread](https://forums.servethehome.com/index.php?threads/us-mellanox-sx6036-200.31513/page-4) — Buying advice, QSFP+ backward compatibility discussion, and IB-to-Ethernet conversion tips
13. [Reddit r/homelab — Old Mellanox InfiniBand vs New No-Name Switch](https://www.reddit.com/r/homelab/comments/1dcrcn6/old_mellanox_infiniband_vs_new_noname_switch/) — Community comparison of legacy Mellanox IB switches against newer budget alternatives

### Arista 7050QX-32

1. [Arista Networks — 7050X Series Product Page](https://www.arista.com/en/products/7050x-series) — Full specifications, features, and datasheets (includes 7050QX-32 under the 7050QX tab)
2. [Arista 7050QX-32/32S Datasheet (PDF)](https://www.arista.com/assets/data/pdf/Datasheets/7050QX-32_32S_Datasheet_S.pdf) — Official datasheet with specs, port layout, power, and performance data
3. [Arista Product Documentation Library](https://www.arista.com/en/support/product-documentation) — Hardware installation guides, EOS software docs, and product bulletins for all Arista switch series
4. [ServeTheHome — Get Started with 40GbE SDN with Microsoft Azure SONiC for Under $1K](https://www.servethehome.com/get-started-with-40gbe-sdn-with-microsoft-azure-sonic-for-under-1k/) — Full walkthrough: installing Azure SONiC on an Arista 7050QX-32, including BGP and VLAN configuration
5. [ServeTheHome Forums — Azure SONiC on the Arista 7050QX-32](https://forums.servethehome.com/index.php?threads/azure-sonic-on-the-arista-7050qx-32.17206/) — Community discussion on SONiC installation with iperf benchmarks
6. [GitHub — injinj/sonic-on-arista-7050qx32](https://github.com/injinj/sonic-on-arista-7050qx32) — Config files and documentation for running SONiC on the 7050QX-32 (community maintained)
7. [ServeTheHome — Inside the Arista DCS-7060CX-32S 32x 100GbE Switch](https://www.servethehome.com/inside-the-arista-dcs-7060cx-32s-32x-100gbe-switch/) — Teardown of the 7060CX-32S successor/sibling, references 7050QX SONiC guide
8. [ServeTheHome — Arista Tag](https://www.servethehome.com/tag/arista/) — Aggregated Arista coverage including 7050/7060 series articles
9. [Reddit r/homelab — What Are the Options for Cheap 10G SFP+ Switches?](https://www.reddit.com/r/homelab/comments/81cdwc/what_are_the_options_for_cheap_10g_sfp_switches/) — Comprehensive comparison including Arista DCS-7124S and other homelab-grade switches
10. [ServeTheHome Forums — Arista DCS-7050QX-32](https://forums.servethehome.com/index.php?threads/arista-dcs-7050qx-32.11132/) — Community thread covering acquisition, initial setup, EOS versions, and 40GbE breakout configuration
11. [ServeTheHome Forums — Got Arista 7050QX-32 (non S) — Hmmm, Now What?](https://forums.servethehome.com/index.php?threads/got-arista-7050qx-32-non-s-hmmm-now-what-some-questions-comments-issues.32613/) — Getting started thread: EOS licensing, fan noise, QSFP+ breakout to 4x10G, and general homelab usage questions
12. [Arista — End of Software Support for 7050QX-32 Series](https://www.arista.com/en/support/advisories-notices/end-of-support/11994-end-of-software-support-for-7050qx-32-series) — Official notice: EOS 4.25 and later will not support the 7050QX-32 platform (last supported release is EOS 4.24)
13. [Arista — End of Sale of 7050QX-32S](https://www.arista.com/en/support/advisories-notices/end-of-sale/13429-end-of-sale-7050qx-32s) — Official end-of-sale notice for the DCS-7050QX-32S model

<!-- Arista requires a login to download some documents; the product page has inline specs and the datasheet PDF above is publicly accessible -->

### Mono Gateway (mono.si)

1. [Mono Gateway — Official Product Page](https://mono.si) — Live product page: open-source 10-gigabit NXP-based router dev kit ($600), specs include NXP LS1046A (1.6 GHz quad Cortex-A72), 2x 10G SFP+, 3x GbE RJ-45, 2x M.2 (WiFi 5+BT+Thread, WiFi 6), 32GB eMMC, preloaded with OpenWRT
2. [Mono Gateway Product Page (archived Feb 2025)](https://web.archive.org/web/20250206220702/https://mono.si/) — Wayback Machine snapshot preserving product information in case the live site goes down again
3. [NXP LS1046A Product Page](https://www.nxp.com/products/processors-and-microcontrollers/arm-processors/layerscape-processors/layerscape-1046a-and-1026a-processors:LS1046A) — SoC datasheet and reference manual
4. [NXP LS1046A Fact Sheet (PDF)](https://www.nxp.com/docs/en/fact-sheet/LS1046AFS.pdf) — SoC specifications summary
5. [Jeff Geerling — Testing the Mono Gateway, a Custom-Built 10 Gbps Router](https://www.jeffgeerling.com/blog/2026/testing-mono-gateway-custom-built-10-gbps-router/) — Detailed review and benchmarks of the Mono Gateway including throughput testing, hardware teardown, and comparison with other 10G routers
6. [CNX Software — Mono Gateway: A 10GbE Nano-ITX Router Board Powered by NXP LayerScape LS1046A](https://www.cnx-software.com/2025/04/11/mono-gateway-a-10gbe-nano-itx-router-board-powered-by-nxp-layerscape-ls1046a/) — Tech press coverage with specs breakdown, board photos, and SoC details
7. [Hacker News — Testing the Mono Gateway: Custom-Built 10 Gbps Router](https://news.ycombinator.com/item?id=46466201) — Community discussion on Jeff Geerling's review with commentary on OpenWrt support, hardware offload, and DPAA acceleration
8. [Neowin Forum — Mono Gateway (That's Its Name, It Is a 10 Gbps Router)](https://www.neowin.net/forum/topic/1461085-mono-gateway-thats-its-name-it-is-a-10-gbps-router/) — Forum thread discussing the Mono Gateway's bridge mode capabilities, NAT performance, and ISP integration
9. [YouTube: apalrd's adventures — Apalrd Tries Tomaž Zaman's New MONO Gateway](https://www.youtube.com/watch?v=JCVZq-fG53U) — Video review/livestream testing the Mono Gateway hands-on with performance benchmarks
10. [YouTube: Tomaž Zaman — My ISP Gave Me a Router I Didn't Want. So I Made My Own.](https://www.youtube.com/watch?v=yASyD_i5kj4) — Creator's own video explaining why he built the Mono Gateway, design decisions, and 10G routing architecture
11. [Jeff Geerling on X — Mono Gateway 10 Gbps Router Announcement](https://x.com/geerlingguy/status/2007108922545709292) — Social media post with 700+ likes announcing the Mono Gateway review, with community replies and discussion

### Calix GP1101X

1. [Calix — GigaPoint Optical Network Terminal Product Page](https://www.calix.com/products/platform/unlimited-subscriber/gigapoint-ont-onu.html) — Official GigaPoint ONT/ONU product family page with GP1101X specs (10G XGS-PON, 10GE interface, carrier-grade VoIP)
2. [Astound Broadband — GP1101X Data Sheet (PDF)](https://astound-prod.mindtouch.us/@api/deki/files/1046/gp1101x.pdf?revision=1) — Official Calix GP1101X GigaPoint datasheet PDF
3. [Astound Broadband — Calix GigaPoint GP1100X Standalone ONT Technical Page](https://astound-prod.mindtouch.us/all-equipment/internet-equipment/fiber-to-the-home-equipment/calix-gigapoint-gp1100x-standalone-ont-w-phone-service) — ISP tech support page with XGSPON specs, LED light descriptions, and device documentation links
4. [Vernon Communications — Getting To Know Your Calix ONT and Wi-Fi Router](https://vernoncom.coop/calix-ont-and-router/) — ISP overview of GP1101X specs (10 Gbps XGS-PON, 10GE interface, VoIP) and GigaSpire router pairing
5. [FiberMall — The Ultimate Guide to Calix Gigapoint GP1100X](https://www.fibermall.com/blog/calix-gp1100x.htm) — Detailed third-party guide covering specs, installation, configuration, troubleshooting, and forum resources
6. [Reddit r/HomeNetworking — Is a Calix GP1100X GigaPoint a Good Modem?](https://www.reddit.com/r/HomeNetworking/comments/191oisq/is_a_calix_gp1100x_gigapoint_a_good_modem/) — Community discussion about the GigaPoint as ISP-provided equipment
7. [Reddit r/Ubiquiti — Calix GP1101X to UXG Lite](https://www.reddit.com/r/Ubiquiti/comments/1qz9j1b/calix_gp1101x_to_uxg_lite/) — User discussion connecting GP1101X to Ubiquiti UniFi gateway
8. [Reddit r/Calix — What Is This and How Does It Mount?](https://www.reddit.com/r/Calix/comments/1ewh4mq/what_is_this_and_how_does_it_mount/) — User photos and mounting discussion for the GP1101X
9. [Calix — End-of-Life (EOL) Notices](https://www.calix.com/eol) — Official Calix EOL/EOS notices page for tracking product lifecycle and support status of GigaPoint ONTs
10. [Vernon Communications — ONT In-a-Box Instructions: Calix Model GP1101X (PDF)](https://vernoncom.coop/wp-content/uploads/2026/02/Calix-ONT-Instructions_.pdf) — ISP-provided self-install guide for the GP1101X with setup steps and LED status reference

### Netgear XS712T

1. [Netgear XS712T Support Page](https://www.netgear.com/support/product/xs712t/) — Support downloads, firmware, and documentation
2. [Netgear XS712T Datasheet (PDF)](https://www.downloads.netgear.com/files/GDC/datasheet/en/XS712T.pdf) — Official product datasheet
3. [Reddit r/homelab — 10 Gb Switch Netgear XS712T, Is It Good?](https://www.reddit.com/r/homelab/comments/tyeea0/10_gb_switch_netgear_xs712t_is_it_good/) — Pricing discussion and user opinions (~$200 used)
4. [Reddit r/homelab — Homelab So Far](https://www.reddit.com/r/homelab/comments/1o4fqm4/homelab_so_far/) — User showcasing XS712T in their Proxmox cluster rack
5. [Reddit r/homelab — Sanity Check: Cheap 10Gb Switch That Is Really Loud](https://www.reddit.com/r/homelab/comments/vudn1k/sanity_check_this_for_me_cheap_10gb_switch_that/) — XS712T noise discussion and pricing
6. [Reddit r/homelab — High Speed Home Network: Sell Current Equipment or Upgrade?](https://www.reddit.com/r/homelab/comments/13hkht6/high_speed_home_network_sell_current_equipment_or/) — User debating keeping XS712T vs switching to Mellanox 40GbE
7. [Reddit r/homelab — What Are the Options for Cheap 10G SFP+ Switches?](https://www.reddit.com/r/homelab/comments/81cdwc/what_are_the_options_for_cheap_10g_sfp_switches/) — Comprehensive comparison including XS712T and XS7xxT line
8. [ServeTheHome — Netgear Tag](https://www.servethehome.com/tag/netgear/) — Aggregated Netgear switch reviews and articles
9. [ServeTheHome — Netgear ProSAFE XS712T 12-Port 10GBase-T Review](https://www.servethehome.com/netgear-prosafe-xs712t-12-port-10gbase-t-smart-managed-switch-review-xs712t-100nes/) — Full STH review with pros/cons, performance benchmarks, and web UI walkthrough
10. [Netgear XS712T/XS728T Family Datasheet (PDF)](https://www.downloads.netgear.com/files/GDC/datasheet/en/XS712T-XS728T.pdf) — Combined 10-Gigabit Smart Managed Switches datasheet covering XS712T and XS728T
11. [Netgear XS712T Software Administration Manual (PDF)](https://www.downloads.netgear.com/files/GDC/XS712T/XS712T_SWA_12Apr2013.pdf) — 310-page manual covering web UI management, VLANs, LACP, QoS, ACLs, and monitoring
12. [Netgear XS712T Hardware Installation Guide (PDF)](https://www.downloads.netgear.com/files/GDC/XS712T/XS712T_HIG_28Feb2013.pdf) — Physical installation, port layout, LED indicators, and initial setup
13. [Netgear KB — XS712T Firmware Version 6.1.0.36](https://kb.netgear.com/000038438/XS712T-Firmware-Version-6-1-0-36) — Latest firmware release with download and upgrade instructions
14. [Amazon — NETGEAR ProSAFE XS712T](https://www.amazon.com/NETGEAR-ProSAFE-12-Port-10GBase-T-XS712T-100NES/dp/B00BWBLL6S) — Amazon product page with user reviews and pricing

#### Dead/Unresolvable Links

- ~~[Netgear ProSAFE 10-Gigabit Smart Switches Datasheet (PDF)](https://www.downloads.netgear.com/files/GDC/datasheet/en/ProSAFE_10-Gigabit_Smart_Managed_Switches.pdf)~~ — Combined family datasheet (HTTP 403 Forbidden)

### TRENDnet TEG-30284

1. [TRENDnet TEG-30284 Product Page](https://www.trendnet.com/products/28-port-10g-web-smart-switch-TEG-30284) — Official product page with specifications and downloads
2. [Reddit r/networking — Layer 3 Core with Layer 2 Access and Distribution](https://www.reddit.com/r/networking/comments/5zlgaa/layer_3_core_with_layer_2_access_and_distribution/) — User planning deployment with 3x TEG-30284 as distribution switches
3. [ModuleTek — TRENDnet TEG-30284 Switch Dismantling](https://www.moduletek.com/en/application_notes/an_00147.html) — Teardown/disassembly of the TEG-30284 with internal component photos and SFP+ slot analysis
4. [TRENDnet TEG-30284 Datasheet (PDF, SECOMP mirror)](https://www.secomp.cz/dwnl/driver/datasheet/21221040.pdf) — Official datasheet hosted by SECOMP distributor — specs, features, and performance data
5. [Amazon — TRENDnet TEG-30284 28-Port Web Smart Switch](https://www.amazon.com/TRENDnet-28-Port-Lifetime-Protection-TEG-30284/dp/B01EOPSRP6) — Marketplace listing with pricing and customer Q&A
6. [Newegg — TRENDnet TEG-30284](https://www.newegg.com/trendnet-teg-30284/p/0XP-001A-009D3) — Marketplace listing with user review praising low noise and value for SMB/hobby use
7. [eBay — TRENDnet TEG-30284](https://www.ebay.com/itm/374381304414) — Marketplace listing with current pricing (~$210)
8. [TRENDnet TEG-30284 v2.5R Product Page](https://www.trendnet.com/products/managed-switch/28-port-web-smart-switch-4-10g-sfp-plus-slots-TEG-30284-v2.5) — Current revision (v2.5R) product page with TRENDnet Hive cloud management option
9. [TRENDnet — RB-TEG-30284 Refurbished Product Page](https://www.trendnet.com/products/product-detail?prod=255_RB-TEG-30284) — Factory refurbished unit listing with same v2.5R specs
10. [TRENDnet — RB-TEG-30284 Support Page](https://www.trendnet.com/support/support-detail.asp?prod=255_RB-TEG-30284) — Support downloads and documentation for the refurbished TEG-30284
11. [ServeTheHome Forums — TEG-30284: Anyone Had Luck with LAG?](https://forums.servethehome.com/index.php?threads/teg-30284-anyone-had-luck-with-lag.12820/) — Community discussion on Link Aggregation Group configuration and troubleshooting
12. [Reddit r/homelab — 10G Switch: TRENDnet TEG-30284 Questions](https://www.reddit.com/r/homelab/comments/6wsxjq/10g_switch_trendnet_teg30284_questions/) — Community Q&A on the TEG-30284 for homelab 10G use

### TP-Link SG3210XHP-M2

1. [TP-Link SG3210XHP-M2 Product Page](https://www.tp-link.com/us/business-networking/omada-switch-poe/sg3210xhp-m2/) — Official product page with specifications, datasheet, and firmware
2. [TP-Link SG3210XHP-M2 Specifications](https://www.tp-link.com/us/business-networking/omada-switch-poe/sg3210xhp-m2/#spec) — Detailed hardware specs: 8x 2.5G RJ45, 2x 10G SFP+, 240W PoE budget, 80Gbps switching, L2+ features, Omada SDN
3. [TP-Link SG3210XHP-M2 Support & Downloads](https://www.tp-link.com/us/support/download/sg3210xhp-m2/) — Firmware downloads, release notes, configuration guides, and knowledgebase articles
4. [ServeTheHome — TP-Link Tag](https://www.servethehome.com/tag/tp-link/) — Aggregated TP-Link switch reviews and articles
5. [ServeTheHome Forums — TL-SG3210XHP-M2 Discussion](https://forums.servethehome.com/index.php?threads/tp-link-tl-sg3210xhp-m2-switch-2-x-sfp-8-x-2-5g-poe-l2-switch-337.37449/) — Community deal and discussion thread comparing TP-Link vs Netgear 2.5G PoE switches for home/small office
6. [ServeTheHome Forums — TL-SG3210X-M2 (Non-PoE Variant) Review](https://forums.servethehome.com/index.php?threads/tp-link-tl-sg3210x-m2-managed-switch-8x2-5gbps-2x10gbps.42666/) — Community discussion of the non-PoE sibling (SG3210X-M2), pricing ~$200 less, same management features
7. [Omada Networks — TL-SG3210XHP-M2 Support & Downloads](https://www.omadanetworks.com/baltic/support/download/tl-sg3210xhp-m2/) — Omada SDN portal with firmware, datasheet, and configuration utilities

### Dell PowerConnect 5448

1. [Dell PowerConnect 5448 Support Page](https://www.dell.com/support/home/en-us/product-support/product/powerconnect-5448/overview) — Support overview, drivers, and documentation
2. [Dell PowerConnect 5448 Documentation](https://www.dell.com/support/home/en-us/product-support/product/powerconnect-5448/docs) — Manuals, documents, articles, videos, and advisories
3. [Dell PowerConnect 5448 Drivers & Downloads](https://www.dell.com/support/home/en-us/product-support/product/powerconnect-5448/drivers) — Firmware and software downloads
4. [Dell — PowerConnect 5400 Series Data Sheet (PDF)](https://i.dell.com/sites/csdocuments/shared-content_data-sheets_documents/en/pwcnt-5400-specs.pdf) — Official 3-page specs sheet covering the 5448 and 5424 models: 96 Gbps switching capacity, 71.2 Mpps forwarding, stacking, VLAN, LACP, QoS features
5. [Dell — PowerConnect 5448 User's Guide (PDF, 444 pages)](https://dl.dell.com/manuals/all-products/esuprt_ser_stor_net/esuprt_powerconnect/powerconnect-5448_user%27s%20guide_en-us.pdf) — Complete user manual covering CLI/web management, VLAN configuration, stacking setup, ACLs, SNMP, and firmware upgrade procedures
6. [Spiceworks Community — Config Dell PowerConnect 5448](https://community.spiceworks.com/t/config-dell-powerconnect-5448/942917) — Community discussion on initial configuration, VLAN setup, and management access for the 5448
7. [Dell Community — PowerConnect 5448 Factory Reset](https://www.dell.com/community/en/conversations/networking-general/powerconnect-5448-factory-reset/647f7fcbf4ccf8a8dee58a3f) — Dell support forum thread covering factory reset procedure and recovery steps

<!-- Dell has limited legacy documentation for the PowerConnect 5448 series -->

### Cisco Catalyst 3560

1. [Cisco Catalyst 3560 Series Support Page](https://www.cisco.com/c/en/us/support/switches/catalyst-3560-series-switches/series.html) — Support hub with model listing, EOL notices, and community links (End-of-Support May 2021)
2. [Cisco Catalyst 3560 Series Product Page (archived Nov 2019)](https://web.archive.org/web/20191113050830/https://www.cisco.com/c/en/us/products/switches/catalyst-3560-series-switches/index.html) — Original product page via Wayback Machine showing end-of-sale status and migration to Catalyst 9300
3. [Reddit r/homelab — How Should I Use a 3560?](https://www.reddit.com/r/homelab/comments/d59xyd/how_should_i_use_a_3560/) — Community advice on what to do with a Catalyst 3560 in a homelab environment
4. [Reddit r/homelab — First Home Lab Advice? Cisco 1841 and 3560 a Good Start?](https://www.reddit.com/r/homelab/comments/bw6dm0/first_home_lab_advice_cisco_1841_and_3560_a_good/) — Discussion on starting a homelab with a Cisco 1841 router and Catalyst 3560 switch
5. [Reddit r/homelab — Home Lab Advice with Multiple Older Servers and Cisco Networking Equipment](https://www.reddit.com/r/homelab/comments/17bec6p/home_lab_advice_with_multiple_older_servers_and/) — Homelab planning thread featuring Catalyst 3560 alongside other legacy Cisco gear
6. [Cisco ANZ — Catalyst 3560 Series Hardware View](https://www.cisco.com/web/ANZ/cpp/refguide/hview/switch/3560.html) — Hardware reference with PoE specs, port configurations, and product images
7. [Andover Consulting Group — Cisco 3560 Catalyst Datasheet (PDF, 21 pages)](https://andovercg.com/datasheets/cisco-3560-catalyst-datasheet.pdf) — Third-party hosted copy of the official Cisco Catalyst 3560 datasheet
8. [Cisco Community — Cisco Catalyst 3560 Series Switches Knowledge Base](https://community.cisco.com/t5/networking-knowledge-base/cisco-catalyst-3560-series-switches/ta-p/3112528) — Community knowledge base article with technical overview and feature summary

#### Dead/Unresolvable Links

- ~~[Cisco Catalyst 3560 Series End-of-Life Information](https://www.cisco.com/c/en/us/obsolete/switches/cisco-catalyst-3560-series-switches.html)~~ — EOL notices page (404; replaced by support page above)

<!-- Original datasheets and configuration guides have been removed per Cisco's retirement policy -->

### Cisco Catalyst 2960

1. [Cisco Catalyst 2960 Series Switches](https://www.cisco.com/c/en/us/products/switches/catalyst-2960-series-switches/index.html) — Product family support page (End-of-Sale Oct 2022, End-of-Support Oct 2027)
2. [Cisco Catalyst 2960 Series Support Page](https://www.cisco.com/c/en/us/support/switches/catalyst-2960-series-switches/series.html) — Support hub with data sheets, EOL notices, and retired model listings (End-of-Support Oct 2019 for original 2960)
3. [Reddit r/homelab — Is It Worth Picking Up Cisco 2960 for My Home Lab?](https://www.reddit.com/r/homelab/comments/l9tvyh/is_it_worth_picking_up_cisco_2960_for_my_home_lab/) — Community discussion on whether the 2960 is still useful for homelab learning and CCNA study
4. [Cisco Catalyst 2960 Software Configuration Guide](https://www.cisco.com/c/en/us/td/docs/switches/lan/catalyst2960/software/release/12-2_55_se/configuration/guide/scg_2960.html) — Official Cisco IOS 12.2(55)SE configuration guide for the Catalyst 2960 series
5. [Cisco Catalyst 2960 Series Data Sheets](https://www.cisco.com/c/en/us/products/switches/catalyst-2960-series-switches/datasheet-listing.html) — Index of all official datasheets for the 2960 family (2960, 2960-S, 2960-X, 2960-Plus, 2960-L, 2960-CX)
6. [Cisco Learning Network — 2960 is a Layer 3 Switch!!??](https://learningnetwork.cisco.com/s/question/0D53i00000KszyeCAB/2960-is-a-layer-3-switch) — Community Q&A clarifying the 2960's L2/L3 capabilities (limited static routing only with IOS 12.2(55)SE)
7. [Cisco Learning Network — What 2960 Switches Should I Get?](https://learningnetwork.cisco.com/s/question/0D53i00000Kt1k3CAB/what-2960-switches-should-i-get) — Community buying advice comparing 2960 sub-models for CCNA study
8. [Cisco Learning Network — Old 2960 or 6500 Still Running in Production: What Risks?](https://learningnetwork.cisco.com/s/question/0D5Kd0000BcEdoSKQS/ive-come-across-old-cisco-2960-or-6500-switches-still-running-in-production-so-im-wondering-if-youre-still-running-an-old-cisco-operating-system-what-risks-are-you-facing-today-what-new-features-are-you-missing-out-on) — Discussion of security and feature risks running legacy 2960/6500 switches in production
9. [Audiophile Style — Discussion of Cisco Catalyst 2960 Series Switches](https://audiophilestyle.com/forums/topic/56620-discussion-of-cisco-catalyst-2960-series-switches-and-miscellaneous-chatter-about-sfp-modules-started-with-ot-posts-from-etherregen-thread/) — Audiophile community using 2960 switches with SFP modules for low-noise network audio setups

#### Dead/Unresolvable Links

- ~~[Cisco Catalyst 2960 Series LAN Lite Switches Data Sheet](https://www.cisco.com/c/en/us/products/collateral/switches/catalyst-2960-series-switches/data_sheet_c78-728003.html)~~ — Datasheet (404; removed by Cisco)

### Cisco 2811

1. [Cisco 2800 Series ISR — Retired Products Page](https://www.cisco.com/c/en/us/obsolete/routers/cisco-2800-series-integrated-services-routers.html) — Retirement confirmation and migration guidance; Cisco deliberately removes all documentation for retired products
2. [Cisco 2811 ISR Product Page (archived Apr 2019)](https://web.archive.org/web/20190426175659/https://www.cisco.com/c/en/us/products/routers/2811-integrated-services-router-isr/index.html) — Original product page via Wayback Machine with specs, end-of-sale notice, and ISR 4000 upgrade path
3. [Reddit r/homelab — Help with CISCO 2811 Router](https://www.reddit.com/r/homelab/comments/3mtzj6/help_with_cisco_2811_router/) — Community thread on getting started with a Cisco 2811 router in a homelab environment
4. [Reddit r/ccna — Free Cisco Lab Access! 2811s, 3560s, and More!](https://www.reddit.com/r/ccna/comments/3ax0gv/free_cisco_lab_access_2811s_3560s_and_more/) — Community resource sharing thread offering free access to Cisco 2811 routers and 3560 switches for CCNA lab practice
5. [Cisco ANZ — 2800 Series Router Hardware View](https://www.cisco.com/web/ANZ/cpp/refguide/hview/router/2800.html) — Cisco Australia/NZ reference page covering the full 2800 series (2801, 2811, 2821, 2851) with hardware specs, slot counts, and memory configurations
6. [Cisco Community — Cisco 2811 Integrated Services Router (Knowledge Base)](https://community.cisco.com/t5/networking-knowledge-base/cisco-2811-integrated-services-router/ta-p/3116259) — Knowledge base article with technical overview, specifications, and capabilities of the 2811 ISR
7. [Cisco Community — 2811 vs 2911 CPU](https://community.cisco.com/t5/switching/2811-vs-2911-cpu/td-p/1780875) — Performance comparison discussion: 2811 rated at 120 Kpps vs 2911 at 352 Kpps, CPU usage analysis
8. [Cisco Community — Basic Cisco 2811 Configuration](https://community.cisco.com/t5/routing-and-sd-wan/basic-cisco-2811-configuration/td-p/1419825) — Getting started guide with basic configuration steps for the Cisco 2811 router
9. [Router-Switch.com — Cisco 2811 Data Sheet (PDF)](https://www.router-switch.com/pdf2html/pdf/cisco2811-datasheet.pdf) — Third-party hosted copy of the Cisco 2800 Series Integrated Services Routers datasheet (3 pages)
10. [Cisco Community — Setup Software for Cisco 2811 Router](https://community.cisco.com/t5/routing/setup-software-for-cisco-2811-router/td-p/4585263) — Forum thread on finding and installing IOS software for the 2811 (Apr 2022)
11. [Cisco Community — Software for Cisco 2811](https://community.cisco.com/t5/routing/software-for-cisco-2811/td-p/4819368) — Forum thread on obtaining IOS images and software support for the retired 2811 (Apr 2023)

<!-- Attempted: cisco.com datasheet URLs (removed), archive.org datasheets (404),
     ManualsLib (returned wrong products — manual ID 874874 is a Martindale multimeter, not Cisco 2811). -->

### Cisco 1841

1. [Cisco 1800 Series ISR — Retired Products Page](https://www.cisco.com/c/en/us/obsolete/routers/cisco-1800-series-integrated-services-routers.html) — Retirement confirmation and migration guidance; Cisco deliberately removes all documentation for retired products
2. [Cisco ANZ — 1800 Series Router Hardware View](https://www.cisco.com/web/ANZ/cpp/refguide/hview/router/1800.html) — Cisco Australia/NZ reference page with 1841 hardware specs: 2 WIC/HWIC slots, 2x 10/100Base-T Ethernet, 1 AIM slot
3. [Cisco — ISR 1800 Series Data Sheet (PDF)](https://www.cisco.com/c/dam/global/it_it/solutions/small-business/pdf/net_found/isr_1800ds_en.pdf) — Official 13-page Cisco 1800 series data sheet covering 1841 specs, performance, modularity, and integrated security features
4. [Router-Switch.com — Cisco 1841 Data Sheet (PDF)](https://www.router-switch.com/pdf2html/pdf/cisco1841-datasheet.pdf) — Third-party hosted copy of the Cisco 1841 Integrated Services Router datasheet PDF
5. [Cisco Community — Cisco 1841 Integrated Services Router](https://community.cisco.com/t5/networking-knowledge-base/cisco-1841-integrated-services-router/ta-p/3113710) — Cisco knowledge base article covering 1841 specs, supported modules (WIC/HWIC), 2x 10/100 Fast Ethernet ports, and IOS feature sets
6. [Spiceworks Community — Cisco 1841 as a Home Router](https://community.spiceworks.com/t/cisco-1841-as-a-home-router/330805) — Community discussion on using the 1841 for home networking and CCNA lab practice, with notes on practical limitations and capabilities
7. [Cisco Community — 1841 Router ↔ Home Router](https://community.cisco.com/t5/routing/1841-router-lt-gt-home-router/td-p/4737099) — Forum thread on connecting a Cisco 1841 to a home network setup
8. [Cisco Community — Basic Router Config: 1841 Series](https://community.cisco.com/t5/other-network-architecture-subjects/basic-router-config-1841-series/td-p/530049) — Configuration walkthrough thread for initial 1841 setup including interface and routing basics

<!-- Attempted: cisco.com datasheet URLs (removed), archive.org (404),
     ManualsLib (returned wrong products — manual ID 889024 is a Black & Decker screwdriver, not Cisco 1841) -->

### Cisco 881

1. [Cisco 800 Series Routers Product Page](https://www.cisco.com/c/en/us/products/routers/800-series-routers/index.html) — 800 series family page with datasheets, config guides, and troubleshooting for 881 and all other 800 series variants
2. [Cisco 800 Series Routers Support Page](https://www.cisco.com/c/en/us/support/routers/800-series-routers/series.html) — Support hub with documentation, firmware, and model listings across 800M/810/860/880/890 product lines (881 listed under 880 section)
3. [ManualsLib — Cisco 881 Manuals](https://www.manualslib.com/brand/cisco/?q=881) — Hardware installation and configuration guides (requires JavaScript for search filtering)
4. [Cisco 881 Integrated Services Router — Product Page](https://www.cisco.com/c/en/us/products/routers/881-integrated-services-router-isr/index.html) — Dedicated 881 ISR product page with overview, features, and broadband speeds for small offices and teleworkers
5. [Cisco End-of-Sale and End-of-Life — Cisco Select 881, 898 and 887](https://www.cisco.com/c/en/us/products/collateral/routers/800-series-routers/eos-eol-notice-c51-743664.html) — Official EoS/EoL notice with last order date (Oct 29, 2020) and end-of-support timeline
6. [Cisco 880 Series Integrated Services Routers — Data Sheet](https://www.cisco.com/c/en/us/products/collateral/routers/887-integrated-services-router-isr/data_sheet_c78_459542.html) — Full data sheet covering 881 and all 880 series models with firewall, content filtering, VPN, and WLAN specs
7. [Router-Switch.com — CISCO881-K9 Datasheet (PDF, 5 pages)](https://www.router-switch.com/pdf2html/pdf/cisco881-k9-datasheet.pdf) — Third-party hosted copy of the Cisco 881 datasheet with 10/100 Fast Ethernet specs and broadband/3G/DSL WAN options
8. [Reddit r/homelab — Found a Cisco 881 Router at Goodwill. Any Value for Learning?](https://www.reddit.com/r/homelab/comments/6lcane/found_a_cisco_881_router_at_goodwill_any_value/) — Community discussion (17 comments) on using a secondhand 881 for CCNA study and homelab projects
9. [Reddit r/networking — As a Small IT Firm, Can We Purchase and Deploy a Cisco 881?](https://www.reddit.com/r/networking/comments/2v1ftu/) — Community advice (39 comments) on deploying 881 routers for small business with comparisons to alternatives
10. [YouTube — How to Set Port Forwarding on a Cisco 881 Router with Cisco Configuration Professional (Roel Van de Paar)](https://www.youtube.com/watch?v=ZgpX86hUk9c) — Video walkthrough of NAT/port forwarding setup on Cisco 881 using CCP GUI
11. [Spiceworks Community — What's the Specific Difference Between C881 and Cisco881?](https://community.spiceworks.com/t/whats-the-specific-difference-between-c881-and-cisco881/600695) — Community Q&A clarifying that C881-K9 is the replacement for CISCO881-SEC-K9 (end-of-life part numbers)

#### Dead/Unresolvable Links

- ~~[Cisco 880 Series Integrated Services Routers Data Sheet](https://www.cisco.com/c/en/us/products/collateral/routers/800-series-routers/datasheet-c78-731755.html)~~ — Datasheet (404; removed by Cisco)

### Cisco ASA 5505

1. [Cisco ASA 5505 Adaptive Security Appliance Data Sheet](https://www.cisco.com/c/en/us/products/collateral/security/asa-5500-series-next-generation-firewalls/datasheet-c78-733510.html) — Full datasheet with specs (150Mbps throughput, 10/25 VPN peers, 8-port FE with PoE)
2. [Cisco ASA 5505 Support Page](https://www.cisco.com/c/en/us/support/security/asa-5505-adaptive-security-appliance/model.html) — Support resources, EOL notices, and documentation (End-of-Support Sept 2025)
3. [Cisco ASA 9.6 CLI Configuration Guide](https://www.cisco.com/c/en/us/td/docs/security/asa/asa96/configuration/general/asa-96-general-config.html) — CLI Book 1: General Operations configuration reference (applies to ASA 5505, ASA 5500-X, Firepower 4100/9300, and ISA 3000)
4. [Cisco ASA Software — Installation and Configuration Guides](https://www.cisco.com/c/en/us/support/security/adaptive-security-appliance-asa-software/products-installation-and-configuration-guides-list.html) — Complete list of ASA configuration guides by version, including CLI, ASDM, and firewall service module guides
5. [Reddit r/homelab — What Little Projects / Tasks Could I Do with a Cisco ASA 5505 Firewall at Home?](https://www.reddit.com/r/homelab/comments/bwhnvz/what_little_projects_tasks_could_i_do_with_a/) — Community ideas for using an ASA 5505 in a homelab — VPN, firewall rules, NAT practice
6. [Reddit r/homelab — Getting a Cisco ASA 5505 to Work with Windows 10 and ASDM](https://www.reddit.com/r/homelab/comments/xatgw0/getting_a_cisco_asa_5505_to_work_with_windows_10/) — Troubleshooting ASDM/Java compatibility issues with modern Windows and browser versions
7. [Reddit r/homelab — Cisco ASA 5505 Setup](https://www.reddit.com/r/homelab/comments/exb0m1/cisco_asa_5505_setup/) — Getting started thread for initial ASA 5505 configuration with community guidance
8. [Cisco — Welcome to the Cisco ASA 5505 (Documentation Roadmap)](https://www.cisco.com/c/en/us/td/docs/security/asa/roadmap/asa-5505-welcome.html) — Official documentation roadmap with links to installation, connection, configuration, and troubleshooting guides
9. [Router-Switch.com — ASA5505-BUN-K9 Datasheet (PDF, 5 pages)](https://www.router-switch.com/pdf2html/pdf/asa5505-bun-k9-datasheet.pdf) — Third-party hosted copy of the Cisco ASA 5505 datasheet with 8-port switch, VPN, and firewall specs
10. [YouTube — Cisco ASA 5505 Firewall Initial Setup: Cisco ASA Training 101 (soundtraining.net)](https://www.youtube.com/watch?v=F6qvKRFn-xc) — Video walkthrough of ASA 5505 initial CLI configuration including interface setup and basic firewall rules
11. [Cisco Community — FPR1000: The True ASA 5505 Replacement](https://community.cisco.com/t5/network-security/fpr1000-the-true-asa-5505-replacement/td-p/3870608) — Community discussion on the Firepower 1000 series as the successor to the discontinued ASA 5505
12. [Cisco Community — Some Questions About Cisco ASA 5505 Throughput and Limitations](https://community.cisco.com/t5/network-security/some-questions-about-cisco-asa-5505-throughput-and-limitations/td-p/3361379) — Community Q&A on ASA 5505 throughput limits, user licensing, and performance constraints
13. [Spiceworks Community — Cisco ASA 5505](https://community.spiceworks.com/t/cisco-asa-5505/500412) — Community thread with 11 answers discussing ASDM versions, firmware upgrades, and selling advice

### Cisco SG300-52

1. [Cisco Small Business 300 Series — Retired Switches Page](https://www.cisco.com/c/en/us/obsolete/switches/cisco-small-business-300-series-managed-switches.html) — Retirement confirmation; Cisco deliberately removes all documentation for retired products
2. [Cisco Small Business 300 Series Support Page](https://www.cisco.com/c/en/us/support/switches/small-business-300-series-managed-switches/series.html) — Support hub listing all retired Small Business switch product lines with EOL policy links
3. [Wahl Network — Hands On With a Cisco SG300-52 Switch](https://wahlnetwork.com/2014/11/18/cisco-sg300-52/) — Hands-on review covering features, noise, power, and port speeds — "Overall, I'm happy with this new switch"
4. [Cisco 300 Series — Device Models Reference](https://www.cisco.com/c/en/us/td/docs/switches/lan/csb_switching_general/olh/Sx300/1-3-0/en/Nikola_models.html) — Official documentation listing all SG300 model variants (SG300-52 = SRW2048-K9, 48 GE + 4 special-purpose ports)
5. [Cisco Community — SG 300-52 52-Port Gigabit Managed Switch Issues](https://community.cisco.com/t5/switches-small-business/sg-300-52-52-port-gigabit-managed-switch/td-p/1825467) — Forum thread discussing bizarre issues with newly purchased SG300-52 switches
6. [Cisco Community — SG300-28 / SG300-52 Firmware Upgrade](https://community.cisco.com/t5/switches-small-business/sg300-28-sg300-52-firmware-upgrade/m-p/5350278) — Forum thread on firmware upgrade procedures for SG300 series (Nov 2025)
7. [Spiceworks Community — Cisco SG300-52 Very Weird and Annoying Issue](https://community.spiceworks.com/t/cisco-sg300-52-very-weird-and-annoying-issue/700367) — Troubleshooting thread with 14 answers covering gateway/VOIP phone issues on SG300-52
8. [Amazon — Cisco Small Business SG300-52 SRW2048-K9](https://www.amazon.com/Cisco-Small-Business-SG300-52-SRW2048-K9/dp/B0041ORNCY) — Marketplace listing with product video, specs, and customer reviews
9. [eBay — Cisco SG300-52 Layer 3 52-Port Gigabit Managed Switch](https://www.ebay.com/itm/234968670416) — Marketplace listing with current pricing (~$88)
10. [ServerSupply — Cisco SG300-52-K9 52-Port Gigabit Switch](https://www.serversupply.com/NETWORKING/SWITCH/52%20PORT/CISCO/SG300-52-K9_374453.htm) — Refurbished unit listing (~$124) with 5% checkout discount and 90-day returns
11. [Cisco — 300 Product Family Model Comparison](https://www.cisco.com/c/en/us/support/smb/product-support/small-business/switches-300-comparison.html) — Side-by-side comparison of all 300 Series models including SG300-52, showing ports, PoE, and feature differences

#### Dead/Unresolvable Links

- ~~[Router-Switch.com — SG300-52 Datasheet (PDF)](https://www.router-switch.com/pdf2html/pdf/sg300-52-datasheet.pdf)~~ — 3-page Cisco 300 Series datasheet mirror (redirects to 403)

### Netgear GS116E (ProSAFE Plus)

1. [Netgear GS116Ev2 Product Page (archived Dec 2022)](https://web.archive.org/web/20221201135008/https://www.netgear.com/business/wired/switches/plus/gs116ev2/) — Full product page with specifications via Wayback Machine (Netgear removed the original)
2. [Netgear GS116Ev2 Support Page (archived Jan 2022)](https://web.archive.org/web/20220124201429/https://www.netgear.com/support/product/GS116Ev2) — Firmware downloads (up to v2.6.0.48), user manuals, installation guide, and ProSAFE Plus Utility downloads via Wayback Machine
3. [Reddit r/homelab — Configuration Advice Needed for Netgear 16 Port](https://www.reddit.com/r/homelab/comments/sh5hcb/configuration_advice_needed_for_netgear_16_port/) — IGMP snooping and broadcast forwarding settings discussion for GS116E
4. [Reddit r/homelab — Stuck on a VLAN SSID Project](https://www.reddit.com/r/homelab/comments/eebts5/stuck_on_a_vlan_ssid_project/) — Configuring VLANs on GS116E with Unifi AP
5. [Reddit r/homelab — Help Troubleshooting Latency Issues](https://www.reddit.com/r/homelab/comments/nrcjc9/help_troubleshooting_latency_issues/) — GS116E with Proxmox VMs, diagnosing slow throughput
6. [Netgear GS116E Support Page (live)](https://support.netgear.com/support/product/gs116e) — Current Netgear support page with setup help, user guides, firmware downloads, and troubleshooting for the GS116E
7. [SNBForums — Netgear GS116 vs. GS116E for Novice/Intermediate User](https://www.snbforums.com/threads/netgear-gs116-vs-gs116e-for-novice-intermediate-user.34405/) — Forum comparison thread discussing managed vs unmanaged GS116 variants and feature differences
8. [Neowin Forums — Netgear ProSafe GS116E Plus Switch Any Good?](https://www.neowin.net/forum/topic/1310390-netgear-prosafe-gs116e-plus-switch-any-good/) — Community review thread discussing GS116E quality, reliability, and use cases
9. [Reddit r/HomeNetworking — Netgear GS116E Incorrect IP Help](https://www.reddit.com/r/HomeNetworking/comments/i0tfth/netgear_gs116e_incorrect_ip_help/) — Troubleshooting thread (27 posts) for GS116E IP address and ProSAFE Plus Utility discovery issues

### Cisco 4402 Wireless LAN Controller

1. [Cisco 4400 Series Wireless LAN Controllers End-of-Life Notice](https://www.cisco.com/c/en/us/obsolete/wireless/cisco-4400-series-wireless-lan-controllers.html) — Retirement notification and migration guidance
2. [Cisco 4400 Series WLC Support Page](https://www.cisco.com/c/en/us/support/wireless/4400-series-wireless-lan-controllers/series.html) — Support hub with retirement dates (End-of-Sale June 2011, End-of-Support June 2016)
3. [Cisco ANZ — Wireless LAN Controllers 2000 and 4400 Series Hardware Reference](https://www.cisco.com/web/ANZ/cpp/refguide/hview/wireless/wlc.html) — Port counts, AP capacity per model (4402 supports up to 50 APs with two 1GbE ports)
4. [HPE Support — Cisco 4400 Series Wireless LAN Controller Data Sheet](https://support.hpe.com/hpesc/public/docDisplay?docId=c02985271&docLocale=en_US) — HPE-hosted copy of the official Cisco 4400 Series datasheet
5. [Gamma Solutions — Cisco 4400 Series Wireless LAN Controllers Spec Sheet (PDF)](https://www.gammasolutions.com/wp-content/uploads/pdf/Cisco4400_spec_web.pdf) — Spec sheet covering 4402 and 4404 models with port and AP capacity details
6. [Router-Switch.com — AIR-WLC4402-12-K9 Datasheet (PDF)](https://www.router-switch.com/pdf2html/pdf/air-wlc4402-12-k9-datasheet.pdf) — Third-party hosted datasheet for the 4402-12 model
7. [Cisco Community — WLC 4402 Supported AP's](https://community.cisco.com/t5/wireless/cisco-wlc-4402-supported-ap-s/td-p/3348354) — Community Q&A on which access points the 4402 supports and minimum firmware versions
8. [Cisco Community — WLC 4402](https://community.cisco.com/t5/wireless/wlc-4402/td-p/3864791) — Community discussion about the WLC 4402
9. [Cisco Community — WLC 4402 Interfaces and Config](https://community.cisco.com/t5/wireless/wlc-4402-interfaces-and-config/td-p/2685381) — Community Q&A on 4402 interface configuration

<!-- Original datasheets and configuration guides removed per Cisco's retirement policy -->
