## Stacking Capability Reference

| Device                     | Stacking       | Technology                                        | Max Stack Size              | Notes                                                                                                                                                         |
| -------------------------- | -------------- | ------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Dell PowerConnect 5448** | Yes            | Proprietary stacking ports/cables                 | Up to 12 units              | Single management IP for the stack, shared config                                                                                                             |
| **Cisco 2960-S/X**         | Yes            | FlexStack / FlexStack-Plus                        | Up to 4-8 units             | Original 2960 (non-S/X): No stacking. S/X variants only                                                                                                       |
| **Cisco 3560**             | No             | N/A                                               | N/A                         | 3750 is the stackable variant of this generation                                                                                                              |
| **IBM G8264**              | **Yes**        | QSFP+ 40G stacking links                          | Up to 8 units               | Single IP management, ring or daisy-chain topology; also supports vLAG (pairs of 2) independently                                                             |
| **IBM G8264e**             | **Likely yes** | QSFP+ 40G stacking links (same platform as G8264) | Up to 8 units (unconfirmed) | Same ASIC/NOS as G8264; no dedicated product guide to confirm independently. Also supports vLAG                                                               |
| **IBM G8316**              | **Unclear**    | Uses vLAG instead                                 | 2 (vLAG pair)               | Spine/aggregation switch; IBM Support page lists "Stacking LEDs" in hardware specs but TIPS0842 does not list stacking as a software feature. vLAG confirmed. |
| **Celestica DX010**        | No             | Uses MC-LAG instead                               | 2 (MC-LAG pair)             | SONiC MC-LAG for multi-chassis                                                                                                                                |
| **Arista 7050QX-32-F**     | No             | Uses MLAG instead                                 | 2 (MLAG pair)               | MLAG for multi-chassis, ECMP for beyond 2                                                                                                                     |
| **Mellanox SX6036**        | No             | N/A                                               | N/A                         | No stacking or MC-LAG                                                                                                                                         |
| All others                 | No             | N/A                                               | N/A                         | —                                                                                                                                                             |

> **Stacking vs MC-LAG/MLAG/vLAG:** Stacking merges multiple physical switches into one logical switch with a single management plane and shared forwarding table. MC-LAG/MLAG/vLAG keeps switches as independent control planes but coordinates LAG membership across two chassis for link redundancy. Both achieve multi-chassis redundancy but stacking is tighter coupling (single config) while MC-LAG is looser (independent configs with coordination protocol).

---

## Summary Table

| Device                   | Qty | Max Port Speed | Total High-Speed Ports   | Managed | L3       | MC-LAG     | Stacking     | Class      | Era   |
| ------------------------ | --- | -------------- | ------------------------ | ------- | -------- | ---------- | ------------ | ---------- | ----- |
| **Celestica DX010**      | 4*  | 100GbE         | 32x QSFP28               | Yes     | Yes      | Yes        | No           | DC         | 2016  |
| **Arista 7050QX-32-F**   | 1   | 40GbE          | 32x QSFP+                | Yes     | Yes      | Yes (MLAG) | No           | DC         | 2013  |
| **IBM Mellanox SX6036**  | 1   | 56G IB / 40GbE | 36x QSFP                 | Yes     | Limited  | No         | No           | HPC/DC     | 2013  |
| **IBM G8316**            | 4   | 40GbE          | 16x QSFP+                | Yes     | Yes      | Yes (vLAG) | **Unclear**  | DC Spine   | 2012  |
| **IBM G8264**            | 3   | 10GbE / 40GbE  | 48x SFP+ + 4x QSFP+      | Yes     | Yes      | Yes (vLAG) | **Yes (8)**  | DC TOR     | 2012  |
| **IBM G8264e**           | 1   | 10GbE / 40GbE  | 48x 10GBASE-T + 4x QSFP+ | Yes     | Yes      | Yes (vLAG) | **Likely**   | DC TOR     | 2012  |
| **Mono Gateway**         | 3   | 10GbE          | 2x SFP+ + 3x 1G          | Yes     | Yes      | No         | No           | Router     | 2022  |
| **Calix GP1101X**        | 1   | 10GbE          | 1x 10GBASE-T             | No      | No       | No         | No           | ISP CPE    | 2022  |
| **Netgear XS712T**       | 1   | 10GbE          | 12x 10GBASE-T            | Smart   | No       | No         | No           | Prosumer   | 2014  |
| **TRENDnet TEG-30284**   | 1   | 10GbE          | 4x SFP+                  | Yes     | L2+      | No         | No           | Prosumer   | 2018  |
| **TP-Link SG3210XHP-M2** | 2   | 10GbE          | 2x SFP+                  | Yes     | L2+      | No         | No           | Prosumer   | 2022  |
| **Cisco SG300-52**       | 1   | 1GbE           | 2x SFP combo             | Yes     | L3-lite  | No         | No           | SMB        | 2010  |
| **Dell PC 5448**         | 4   | 1GbE           | 4x SFP combo             | Yes     | No       | No         | **Yes (12)** | Prosumer   | 2007  |
| **Cisco 3560**           | 1   | 1GbE           | 4x SFP                   | Yes     | Yes      | No         | No           | Enterprise | 2004+ |
| **Cisco 2960**           | 1   | 1GbE           | 2-4x SFP                 | Yes     | No       | No         | **S/X only** | Enterprise | 2006+ |
| **Netgear GS116E**       | 1   | 1GbE           | None                     | Plus    | No       | No         | No           | Consumer   | 2013  |
| **Cisco 2811**           | 2   | 1GbE           | 2x RJ45                  | Yes     | Yes      | No         | No           | Router     | 2005  |
| **Cisco 1841**           | 1   | 100Mbps        | 2x FE                    | Yes     | Yes      | No         | No           | Router     | 2005  |
| **Cisco 881**            | 1   | 100Mbps        | 5x FE                    | Yes     | Yes      | No         | No           | Router     | 2008  |
| **Cisco ASA 5505**       | 1   | 100Mbps        | 8x FE                    | Yes     | Firewall | No         | No           | Firewall   | 2006  |
| **Cisco 4402 WLC**       | 1   | 1GbE           | 4x RJ45                  | Yes     | N/A      | No         | No           | WLAN Ctrl  | 2006  |

- *: 1 DX010 is missing fans/psu

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

# Routing & Switching Inventory

> Last updated: 2026-04-02

---

## Core Fabric Switches

### Celestica DX010 (4x) (*- 1 is missing fans/psu)

| Attribute                              | Value                                                                                                                                                                                                                                                           |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Ports**                              | 32x QSFP28 100GbE                                                                                                                                                                                                                                               |
| **Breakout**                           | 4x25GbE, 2x50GbE, or 4x10GbE per QSFP28 (up to 128x 25G or 128x 10G)                                                                                                                                                                                            |
| **ASIC**                               | Broadcom BCM56960 (Tomahawk)                                                                                                                                                                                                                                    |
| **Switching Capacity**                 | 3.2 Tbps                                                                                                                                                                                                                                                        |
| **Forwarding Rate**                    | 2,380 Mpps                                                                                                                                                                                                                                                      |
| **MTU / Jumbo**                        | 9216 bytes                                                                                                                                                                                                                                                      |
| **MAC Table**                          | 128K entries                                                                                                                                                                                                                                                    |
| **Route Table**                        | 16K IPv4 / 8K IPv6 (default), up to 128K with memory_profile_l3                                                                                                                                                                                                 |
| **Form Factor**                        | 1RU                                                                                                                                                                                                                                                             |
| **OS**                                 | SONiC (Open Network Install Environment / ONIE compatible)                                                                                                                                                                                                      |
| **Management**                         | CLI, REST API, gNMI, SNMP (v1/v2c/v3), LLDP, NTP, syslog                                                                                                                                                                                                        |
| **Class**                              | Enterprise / Data Center                                                                                                                                                                                                                                        |
| **Released**                           | ~2016                                                                                                                                                                                                                                                           |
| **Status**                             | Community-supported via SONiC, active open-source development                                                                                                                                                                                                   |
| **Notes**                              | Bare-metal white-box switch. Originally designed for hyperscale data centers. Used/surplus units, potential for hardware failure. Intel Atom C2000 management CPU, 4GB DDR3, mSATA storage. 5 CPLDs for board control, 1x RJ45 mgmt, 1x serial console, 1x USB. |
|                                        |                                                                                                                                                                                                                                                                 |
| **— Power —**                          |                                                                                                                                                                                                                                                                 |
| **PSU**                                | 2x 800W Delta (1+1 redundant, hot-swap), AC input 100-240V                                                                                                                                                                                                      |
| **System Idle**                        | ~120-140W estimated (ASIC ~110W TDP + CPU ~10W + fans ~10-20W, no transceivers)                                                                                                                                                                                 |
| **System Typical**                     | ~150-200W (normal traffic, mix of DACs and optics)                                                                                                                                                                                                              |
| **System Max**                         | ~250-300W (all 32 ports active with optics, high traffic, fans full speed)                                                                                                                                                                                      |
| **Per-Port: DAC (passive copper)**     | ~0.5-1W (SerDes only, no PHY/laser)                                                                                                                                                                                                                             |
| **Per-Port: Active Optic (SR/LR)**     | ~1.5-3.5W (QSFP28 100G SR4 ~2.5W typical; LR4 ~3.5W)                                                                                                                                                                                                            |
| **Per-Port: Empty cage**               | ~0W (cage unpowered when empty on Tomahawk)                                                                                                                                                                                                                     |
| **Per-Port: Active copper (RJ45 SFP)** | N/A (QSFP28 — no RJ45 SFP modules at 100G)                                                                                                                                                                                                                      |
| **PoE**                                | Not supported                                                                                                                                                                                                                                                   |
| **Power Source**                       | STH review, Broadcom Tomahawk datasheet, Delta PSU specs, QSFP28 MSA power specs                                                                                                                                                                                |
|                                        |                                                                                                                                                                                                                                                                 |
| **— Latency —**                        |                                                                                                                                                                                                                                                                 |
| **Baseline (DAC, L2, 64B)**            | ~400ns cut-through (Memory BCM56960 ASIC specification)                                                                                                                                                                                                         |
| **Typical (SONiC software path)**      | ~1-2µs (includes SONiC control plane overhead for ACL/QoS lookup)                                                                                                                                                                                               |
| **Switching Mode**                     | Cut-through (default), store-and-forward configurable                                                                                                                                                                                                           |
| **Modifier: Fiber optic (SR)**         | +0ns switching (same SerDes path as DAC; fiber latency is cable-length, ~5ns/m)                                                                                                                                                                                 |
| **Modifier: Fiber optic (LR)**         | +0ns switching (same SerDes path; LR optic has ~10-50ns internal laser/driver delay)                                                                                                                                                                            |
| **Modifier: Active copper SFP**        | N/A at 100G (no QSFP28 RJ45 modules exist)                                                                                                                                                                                                                      |
| **Modifier: Speed mismatch**           | +variable (breakout 100G→25G/10G adds minor buffering; cross-speed L2 adds µs-range store-and-forward)                                                                                                                                                          |
| **L3 Routing vs L2**                   | ~same (Tomahawk does L2/L3 in single pipeline pass in hardware)                                                                                                                                                                                                 |
| **ACL/QoS Impact**                     | Negligible in hardware TCAM; complex sobftware ACLs may add ~1-5µs                                                                                                                                                                                              |
| **Latency Source**                     | Broadcom Tomahawk datasheet, SONiC community benchmarks                                                                                                                                                                                                         |
|                                        |                                                                                                                                                                                                                                                                 |
| **— L2 Features —**                    |                                                                                                                                                                                                                                                                 |
| **VLANs**                              | 802.1Q, up to 4094 VLANs                                                                                                                                                                                                                                        |
| **Private VLAN**                       | Not supported in SONiC (as of 2024)                                                                                                                                                                                                                             |
| **Voice VLAN**                         | Not natively supported (use LLDP-MED + VLAN assignment)                                                                                                                                                                                                         |
| **Protocol-based VLAN**                | No — not supported in SONiC                                                                                                                                                                                                                                     |
| **Trunking**                           | 802.1Q tagged trunks, native VLAN (PVID), allowed VLAN filtering                                                                                                                                                                                                |
| **Trunk Negotiation**                  | Manual only (no DTP — DTP is Cisco proprietary)                                                                                                                                                                                                                 |
| **STP**                                | PVST+ and RSTP supported in SONiC; MSTP partial/limited                                                                                                                                                                                                         |
