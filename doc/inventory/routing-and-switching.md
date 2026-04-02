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
