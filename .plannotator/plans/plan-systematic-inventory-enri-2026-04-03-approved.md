
# Plan: Systematic Inventory Enrichment

## Overview
Enrich all 21 devices in `doc/inventory/routing-and-switching.md` with detailed power, latency, and feature data. Process one device at a time, commit after each, using a standard questions file and a device checklist file.

---

## Phase 0: Create Framework Files

### Step 0.1 — Create Standard Questions File
**File:** `doc/inventory/standard-attributes.md`

Write a comprehensive list of attributes every device must answer. Organized by category:

#### A. Power Draw
1. System idle power (W) — no transceivers, minimal config
2. System typical power (W) — normal operation, typical port utilization
3. System max power (W) — all ports active, worst-case
4. Per-port power by media type:
   - DAC cable (passive copper)
   - Active optical (SR/LR optic)
   - Active copper SFP (RJ45 PHY module)
   - Empty port (cage powered but no module)
5. PoE budget (W) — if applicable, separate from switch draw
6. PoE per-port max (W) — 802.3af/at/bt class
7. Total system + PoE max draw (W)
8. PSU rating / redundancy (single, 1+1, etc.)
9. Power supply type (AC/DC, voltage range)

#### B. Packet Latency
1. Baseline switching latency — DAC-to-DAC, L2 forwarding, 64-byte frames
2. Latency measurement method (cut-through vs store-and-forward, which mode?)
3. Modifiers:
   - Fiber optic transceiver (SR/LR) — additional µs from serialization
   - Copper SFP (RJ45 PHY) — additional µs from PHY processing
   - 10GBASE-T native port — PHY processing latency
   - Speed mismatch (e.g., 10G→1G) — buffering latency
4. L3 routing latency vs L2 switching latency
5. ACL/QoS processing impact on latency

#### C. L2 Feature Matrix
1. VLAN support — 802.1Q, max VLANs, private VLAN, voice VLAN, protocol-based VLAN
2. Trunking — 802.1Q trunk, native VLAN, allowed VLAN filtering, trunk negotiation (DTP or manual)
3. STP variants — STP (802.1D), RSTP (802.1w), MSTP (802.1s), PVST+, proprietary
4. MAC table size
5. Storm control / broadcast suppression
6. IGMP snooping

#### D. Link Aggregation
1. Static LAG (manual port-channel)
2. LACP (802.3ad / 802.1AX) — yes/no
3. Max LAG groups, max ports per LAG
4. Hash modes — L2 (src/dst MAC), L3 (src/dst IP), L4 (src/dst port), L2+L3, L3+L4, symmetric
5. Cross-stack LAG support
6. Latency impact: LAG hashing overhead (typically negligible)

#### E. Multi-Chassis LAG / Redundancy
1. MC-LAG variant supported: MC-LAG / VPC / VLT / VSX / vLAG / MLAG / VSS / StackWise Virtual / none
2. Protocol/standard: proprietary or standard-based
3. Interoperable with other vendors (downstream sees standard LACP)
4. Max MC-LAG peers
5. Failover time (ms) — single link failure, peer failure
6. Split-brain handling (orphan ports, peer-link, keepalive)

#### F. First-Hop Redundancy
1. VRRP (v2/v3) — standard, interoperable
2. HSRP (v1/v2) — Cisco proprietary
3. GLBP — Cisco proprietary, load-balancing
4. Anycast gateway / distributed gateway — EVPN-based
5. Failover time (ms) per protocol
6. Preemption support
7. Tracking (interface, route, object)

#### G. L3 Routing (if applicable)
1. Static routing
2. OSPF (v2, v3)
3. BGP (v4, v6)
4. RIP (v1/v2)
5. IS-IS
6. Policy-based routing
7. VRF / VRF-lite
8. Route table capacity (IPv4/IPv6)
9. BFD support (failover acceleration)
10. ECMP — max paths, hash algorithm

#### H. Router-Specific (routers, firewalls)
1. NAT performance (sessions/sec, concurrent sessions)
2. VPN throughput (IPsec, SSL/TLS)
3. Firewall throughput (stateful inspection)
4. Hardware offload capabilities (crypto, NAT, ACL)
5. Routing protocol performance (routes, convergence)
6. Interface queue depth / QoS architecture

#### I. Wireless Controller-Specific
1. Max APs managed
2. Max clients
3. RF standards supported (a/b/g/n/ac/ax)
4. Roaming protocol (802.11r, proprietary)

#### J. Security Features
1. ACL types (standard, extended, port-based, VLAN-based)
2. 802.1X port authentication
3. DHCP snooping
4. Dynamic ARP inspection
5. IP source guard
6. MACsec (802.1AE)
7. Control plane policing

#### K. Monitoring & Management
1. SNMP (v1/v2c/v3)
2. sFlow / NetFlow / IPFIX
3. SPAN / RSPAN / ERSPAN
4. REST API / OpenConfig / NETCONF / gNMI
5. Syslog, NTP, DNS

**Commit** after creating this file.

### Step 0.2 — Create Device Checklist File
**File:** `doc/inventory/enrichment-checklist.md`

A checklist of all 21 devices with columns: `[x]`, Device Name, Class, Status/Notes.

```
## Device Enrichment Checklist

| # | Done | Device | Class | Notes |
|---|------|--------|-------|-------|
| 1 | [ ] | Celestica DX010 | DC Fabric (SONiC) | |
| 2 | [ ] | IBM G8264 | DC TOR (ENOS) | |
| 3 | [ ] | IBM G8264e | DC TOR (ENOS) | |
| 4 | [ ] | IBM G8316 | DC Spine (ENOS) | |
| 5 | [ ] | Mellanox SX6036 | HPC/DC (MLNX-OS) | |
| 6 | [ ] | Arista 7050QX-32-F | DC (EOS) | |
| 7 | [ ] | Mono Gateway | Router (OpenWrt) | |
| 8 | [ ] | Cisco 2811 | Router (IOS) | |
| 9 | [ ] | Cisco 1841 | Router (IOS) | |
| 10 | [ ] | Cisco 881 | Router (IOS) | |
| 11 | [ ] | Netgear XS712T | Prosumer 10GbE | |
| 12 | [ ] | TRENDnet TEG-30284 | Prosumer L2+ | |
| 13 | [ ] | TP-Link SG3210XHP-M2 | Prosumer PoE | |
| 14 | [ ] | Dell PowerConnect 5448 | Prosumer stackable | |
| 15 | [ ] | Cisco SG300-52 | SMB L3-lite | |
| 16 | [ ] | Netgear GS116E | Consumer | |
| 17 | [ ] | Cisco 3560 | Enterprise L3 | |
| 18 | [ ] | Cisco 2960 | Enterprise L2 | |
| 19 | [ ] | Cisco ASA 5505 | Firewall | |
| 20 | [ ] | Cisco 4402 WLC | WLAN Controller | |
| 21 | [ ] | Calix GP1101X | ISP CPE (ONT) | |
```

**Commit** after creating this file.

---

## Phase 1: Device-by-Device Enrichment (repeat 21 times)

For **each device** in the checklist, in order:

### Step 1.N.1 — Research
- Read `doc/inventory/standard-attributes.md` to load the question set
- Read the device's current section in `doc/inventory/routing-and-switching.md`
- Read the device's references section for existing links
- Research via web (datasheets, vendor docs, community benchmarks) to answer every standard question
- For power: find official PSU specs, TDP ratings, per-port breakdowns
- For latency: find published cut-through/store-and-forward numbers, PHY latency specs
- For features: find feature matrices, config guides, release notes

### Step 1.N.2 — Update Inventory
- Add new attribute rows to the device's detail table in `routing-and-switching.md`
- Organize under sub-headers within the table: Power, Latency, L2 Features, LAG, MC-LAG, FHRP, L3, Security, Monitoring
- Add any new references found during research to the device's references section
- If a question truly cannot be answered, write "Unknown — no public data found" with brief explanation
- If research reveals a new class-specific question not yet in `standard-attributes.md`, add it there too

### Step 1.N.3 — Commit & Check Off
- `git add` the changed files
- `git commit -m "inventory: enrich {Device Name} with power/latency/features"`
- Update `enrichment-checklist.md`: mark device `[x]`, add short summary in Notes column (e.g., "150W typ, 480ns L2, LACP L3+L4 hash, vLAG, VRRP")
- `git commit -m "checklist: mark {Device Name} complete"`
- If `standard-attributes.md` was updated with new questions, commit that too

### Step 1.N.4 — Context Management
- Since research burns context, after committing, the enrichment for that device is **done**
- The next device starts fresh by re-reading the standard questions file

---

## Phase 2: Gap Analysis & Remediation

### Step 2.1 — Final Review
- Re-read `doc/inventory/standard-attributes.md` for the complete question set
- For **each** of the 21 devices, verify every question has an answer in the inventory
- Create a gap list in `enrichment-checklist.md` under a new `## Gaps` section:
  ```
  | # | Device | Missing Attribute | Reason |
  ```

### Step 2.2 — Fill Gaps (repeat until none remain)
- For each gap, do targeted research
- Update the device's section in `routing-and-switching.md`
- Commit after each device update
- Remove the gap from the list when resolved

### Step 2.3 — Final Verification
- Confirm all gaps resolved
- Confirm all 21 devices checked off
- Commit final checklist state

---

## Phase 3: Summary

- Output a summary of:
  - Total devices enriched
  - Attributes added per device (count)
  - Any remaining unknowns with justification
  - New questions added to standard-attributes.md during the process
  - Total commits made

---

## Execution Rules
1. **One device at a time** — never batch multiple devices
2. **Commit after every file change** — atomic commits
3. **Re-read standard questions** at the start of each device — context may have been compacted
4. **Add new questions discovered** during research to the standard file immediately
5. **Every answer cites a source** — datasheet, vendor doc, community test, or "estimated based on {reasoning}"
6. **Order:** Core fabric first (DX010 → G8264 → G8264e → G8316 → SX6036 → 7050QX), then routers, then managed switches, then security/specialty, then ISP CPE
