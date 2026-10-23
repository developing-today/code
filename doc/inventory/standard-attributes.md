# Standard Attributes for Inventory Enrichment

Every device in the inventory must answer each applicable question below.
If a question does not apply to a device class, answer "N/A — {reason}".
If the answer cannot be determined, answer "Unknown — {what was checked}".
Every answer must cite a source (datasheet, vendor doc, community test, or estimation basis).

---

## A. Power Draw

| # | Attribute | Description |
|---|-----------|-------------|
| A1 | System idle power (W) | No transceivers installed, minimal config, fans at low speed |
| A2 | System typical power (W) | Normal operation, typical port utilization (~50% ports active) |
| A3 | System max power (W) | All ports active, worst-case traffic, fans at full speed |
| A4 | Per-port power: DAC (W) | Passive copper direct-attach cable — typically ~0.5-1W |
| A5 | Per-port power: Active optical (W) | SR/LR/ER optic module — typically ~1-3.5W depending on reach |
| A6 | Per-port power: Copper SFP / RJ45 PHY (W) | Active copper SFP with built-in PHY (e.g., 10GBASE-T SFP+) — typically ~2-4W |
| A7 | Per-port power: Empty cage (W) | Powered cage with no module inserted — typically ~0.1-0.5W |
| A8 | PoE budget (W) | Total PoE power budget available to endpoints, separate from switch draw. N/A if no PoE. |
| A9 | PoE per-port max (W) | Max per-port PoE delivery. Specify standard: 802.3af (15.4W), at (30W), bt Type 3 (60W), bt Type 4 (90W) |
| A10 | Total max draw with PoE (W) | System max power + full PoE budget |
| A11 | PSU configuration | Rating per PSU, redundancy model (single, 1+1, N+1), hot-swap capability |
| A12 | Power input type | AC (voltage range) or DC (voltage range), connector type |

---

## B. Packet Latency

| # | Attribute | Description |
|---|-----------|-------------|
| B1 | Baseline L2 latency | Port-to-port, same-speed, DAC-to-DAC, L2 forwarding, 64-byte frames. Specify cut-through or store-and-forward. |
| B2 | Forwarding mode | Cut-through, store-and-forward, or adaptive. Which is default? |
| B3 | Modifier: Fiber optic | Additional latency from SR/LR transceiver vs DAC (serialization + PHY) |
| B4 | Modifier: Copper SFP (RJ45 PHY) | Additional latency from 10GBASE-T SFP+ module PHY processing (~2-4µs typical) |
| B5 | Modifier: Native RJ45 (10GBASE-T) | PHY processing latency for built-in copper ports (~2-4µs typical) |
| B6 | Modifier: Speed mismatch | Latency added when forwarding between different port speeds (e.g., 40G→10G) due to buffering/serialization |
| B7 | L3 routing latency | Additional latency for L3 forwarded packets vs L2 switched (hardware-routed vs software-routed) |
| B8 | ACL/QoS impact | Measured or estimated latency impact of applying ACLs, QoS policies, or sobecause queuing |

---

## C. L2 Feature Matrix

| # | Attribute | Description |
|---|-----------|-------------|
| C1 | 802.1Q VLANs | Supported? Max VLAN IDs. |
| C2 | Private VLAN (PVLAN) | Promiscuous/isolated/community port support |
| C3 | Voice VLAN | Automatic voice VLAN assignment for IP phones |
| C4 | Protocol-based VLAN | VLAN assignment by protocol (e.g., IPv4, IPv6, ARP) |
| C5 | Q-in-Q (802.1ad) | Double-tagging / VLAN stacking |
| C6 | Trunk ports | 802.1Q trunking, native VLAN config, allowed VLAN filtering |
| C7 | Trunk negotiation | DTP (Cisco), manual only, or other |
| C8 | STP variant | STP (802.1D), RSTP (802.1w), MSTP (802.1s), PVST+, RPVST+ |
| C9 | STP convergence time | Typical failover time in ms for the best supported STP variant |
| C10 | MAC table size | Maximum MAC address entries |
| C11 | Storm control | Broadcast/multicast/unicast storm suppression — threshold types (pps, %, bps) |
| C12 | IGMP snooping | Version (v1/v2/v3), querier support |
| C13 | LLDP / CDP | Link Layer Discovery Protocol support |
| C14 | Jumbo frames | Max MTU supported |

---

## D. Link Aggregation

| # | Attribute | Description |
|---|-----------|-------------|
| D1 | Static LAG | Manual port-channel without negotiation protocol |
| D2 | LACP (802.3ad/802.1AX) | Dynamic LAG negotiation — standard, interoperable |
| D3 | Max LAG groups | Maximum number of LAG/port-channel groups |
| D4 | Max ports per LAG | Maximum member ports in a single LAG |
| D5 | Hash modes | L2 (src/dst MAC), L3 (src/dst IP), L4 (src/dst port), L2+L3, L3+L4, symmetric — list all supported |
| D6 | Cross-stack LAG | LAG members spanning stacked switches |
| D7 | LAG failover time | Time to redistribute traffic when a LAG member link fails (typically <50ms for LACP) |
| D8 | Min-links | Support for minimum active links before LAG goes down |

---

## E. Multi-Chassis LAG / High Availability

| # | Attribute | Description |
|---|-----------|-------------|
| E1 | MC-LAG variant | MC-LAG, VPC, VLT, VSX, vLAG, MLAG, VSS, StackWise Virtual, or none |
| E2 | Protocol type | Standard-based or proprietary? Name the protocol/implementation. |
| E3 | Downstream interop | Does the downstream device see standard LACP? (i.e., vendor-agnostic from partner perspective) |
| E4 | Max MC-LAG peers | Number of chassis in the MC-LAG domain (typically 2) |
| E5 | Peer link requirements | Dedicated peer-link? How many ports? Same-speed required? |
| E6 | Keepalive mechanism | Out-of-band keepalive, in-band heartbeat, or both |
| E7 | Failover time: link failure (ms) | Time for traffic to reconverge when a single MC-LAG member link fails |
| E8 | Failover time: peer failure (ms) | Time for traffic to reconverge when an entire MC-LAG peer switch fails |
| E9 | Split-brain handling | Orphan port behavior, peer-link failure behavior, recovery mechanism |

---

## F. First-Hop Redundancy Protocol (FHRP)

| # | Attribute | Description |
|---|-----------|-------------|
| F1 | VRRP | Version (v2 RFC 3768, v3 RFC 5798). Standard, interoperable. |
| F2 | HSRP | Version (v1, v2). Cisco proprietary. |
| F3 | GLBP | Cisco proprietary, active-active load balancing. |
| F4 | Anycast gateway | EVPN distributed anycast gateway, or similar (e.g., SONiC VARP). |
| F5 | FHRP failover time (ms) | Default and tuned (with sub-second timers or BFD). |
| F6 | Preemption | Can higher-priority router reclaim active/master role? |
| F7 | Object tracking | Interface tracking, route tracking, IP SLA tracking for FHRP |

---

## G. L3 Routing (if applicable)

| # | Attribute | Description |
|---|-----------|-------------|
| G1 | Static routing | Supported, max static routes |
| G2 | OSPF | v2 (IPv4), v3 (IPv6). Max routes, max areas, max neighbors. |
| G3 | BGP | v4 unicast, v6 unicast, EVPN. Max prefixes, max peers. |
| G4 | RIP | v1, v2. |
| G5 | IS-IS | Supported? |
| G6 | Policy-based routing (PBR) | Route-map based forwarding decisions |
| G7 | VRF / VRF-lite | Virtual routing and forwarding instances. Max VRFs. |
| G8 | Route table capacity | Max IPv4 and IPv6 routes (hardware FIB / RIB) |
| G9 | BFD | Bidirectional Forwarding Detection for fast failure detection. Min interval. |
| G10 | ECMP | Max equal-cost paths, hash algorithm (same options as LAG hash) |

---

## H. Router & Firewall Specific

| # | Attribute | Description |
|---|-----------|-------------|
| H1 | NAT performance | Translations/sec, max concurrent NAT sessions |
| H2 | IPsec VPN throughput | Throughput with encryption (AES-128/256, 3DES). Hardware crypto? |
| H3 | SSL/TLS VPN throughput | If supported, max throughput and concurrent tunnels |
| H4 | Stateful firewall throughput | Packets/sec and Mbps with stateful inspection enabled |
| H5 | Max firewall sessions | Concurrent session table size |
| H6 | Hardware offload | Crypto accelerator, NAT offload, ACL offload — what's in hardware vs software |
| H7 | Routing convergence | Estimated convergence time for OSPF/BGP with BFD |

---

## I. Wireless Controller Specific

| # | Attribute | Description |
|---|-----------|-------------|
| I1 | Max APs managed | Maximum access points |
| I2 | Max clients | Maximum concurrent wireless clients |
| I3 | RF standards | 802.11a/b/g/n/ac/ax/be supported |
| I4 | Roaming protocol | 802.11r (fast BSS transition), OKC, proprietary |
| I5 | CAPWAP / LWAPP | Control and provisioning protocol for APs |
| I6 | Rogue AP detection | Built-in wireless IDS/IPS |

---

## J. Security Features

| # | Attribute | Description |
|---|-----------|-------------|
| J1 | ACL types | Standard, extended, port-based (PACL), VLAN-based (VACL), MAC-based |
| J2 | 802.1X | Port-based network access control. MAB fallback? |
| J3 | DHCP snooping | Trusted/untrusted port model |
| J4 | Dynamic ARP inspection (DAI) | ARP validation against DHCP snooping binding table |
| J5 | IP source guard | Filter traffic by IP+MAC against snooping table |
| J6 | MACsec (802.1AE) | Link-layer encryption. Line-rate? |
| J7 | Control plane policing (CoPP) | Rate-limit control plane traffic to protect CPU |
| J8 | Port security | Max MAC addresses per port, violation actions |

---

## K. Monitoring & Management

| # | Attribute | Description |
|---|-----------|-------------|
| K1 | SNMP | v1, v2c, v3 (auth+priv) |
| K2 | sFlow / NetFlow / IPFIX | Flow telemetry. Which variant? Sampling rate? |
| K3 | SPAN / RSPAN / ERSPAN | Port mirroring. Local, remote (over VLAN), or encapsulated (over IP)? |
| K4 | Programmability | REST API, OpenConfig, NETCONF/YANG, gNMI, OpenFlow |
| K5 | Syslog | Remote syslog support, severity filtering |
| K6 | NTP / PTP | Time synchronization. PTP (802.1AS / 1588v2) for precision? |
| K7 | DNS | DNS client for management lookups |

---

## Applicability Matrix

| Category | Switches | Routers | Firewalls | WLC | ONT/CPE |
|----------|----------|---------|-----------|-----|---------|
| A. Power | ✅ | ✅ | ✅ | ✅ | ✅ |
| B. Latency | ✅ | ✅ | ✅ | ✅ | ✅ |
| C. L2 Features | ✅ | Partial | Partial | Partial | Partial |
| D. LAG | ✅ | Partial | ❌ | ❌ | ❌ |
| E. MC-LAG | ✅ | ❌ | ❌ | ❌ | ❌ |
| F. FHRP | L3 switches | ✅ | ❌ | ❌ | ❌ |
| G. L3 Routing | L3 switches | ✅ | Partial | ❌ | ❌ |
| H. Router/FW | ❌ | ✅ | ✅ | ❌ | ❌ |
| I. Wireless | ❌ | ❌ | ❌ | ✅ | ❌ |
| J. Security | ✅ | ✅ | ✅ | Partial | ❌ |
| K. Monitoring | ✅ | ✅ | ✅ | ✅ | Partial |
