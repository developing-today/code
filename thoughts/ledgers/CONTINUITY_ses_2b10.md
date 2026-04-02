---
session: ses_2b10
updated: 2026-04-02T22:50:41.146Z
---



## Summary of Work Done

### Project Overview
Building network infrastructure documentation for a homelab with IBM/Lenovo switches and SONiC-based switches, documented across two main files.

### Completed Work

#### 1. Network Topology Design ✅
- **File:** `docs/network-topology.md`
- 3-VLAN hybrid port topology with DX010 MC-LAG pair and Mono routers
- MTU, PMTUD, ARP sysctls all decided

#### 2. Inventory Doc Stacking Corrections ✅
- **File:** `doc/inventory/routing-and-switching.md` (913 lines)
- **G8264 stacking** (line ~51): Changed "No" → "Yes (up to 8 switches, QSFP+ 40G, ring or daisy-chain)" per Lenovo Press TIPS1272
- **G8264e stacking** (line ~76): Changed "No" → "Likely yes (same platform/firmware)"
- **Stacking Reference Table** (lines 508-510): Updated G8264=Yes, G8264e=Likely, G8316=No
- **Summary Table** (lines 528-529): Updated stacking columns
- Added TIPS1272 and TIPS0842 references

#### 3. G8316 Deep-Dive Link Review ✅
Reviewed 8 user-provided URLs for the G8316:

- **Key discovery:** IBM Support overview page lists **"Stacking LEDs to indicate Master/Member"** under Physical specs → LEDs, yet stacking is NOT listed as a software feature in TIPS0842. This is ambiguous.
- **G8316 confirmed capabilities from TIPS0842:** vLAG, VRRP, LACP (64 trunk groups/32 ports), Hot Links, MSTP, RSTP, full L3 (OSPF v2, BGP, RIP), OpenFlow 1.0/1.3.1, VMready, CEE/DCB, FCoE transit, 128K MAC, 9216 jumbo, 1.28 Tbps, <1µs latency, single ASIC.
- Several URLs were dead, paywalled, or raw PDFs; cataloged which are new vs already referenced.

### Pending Edits (Immediate Next Steps)
These specific changes have NOT yet been applied to the inventory doc:

1. **G8316 stacking** in Stacking Reference Table (line 510) and Summary Table (line 527): Change "No" → **"Unclear"**
2. **Add 3 new reference links** after line 652:
   - IBM Install Guide PDF
   - Switch Center 8.1.4 Release Notes PDF
   - Scribd document
3. **Add dead/unresolvable links subsection** with officecentral.com URL

### Pending Work (Broader Project)
- 🔲 SONiC MC-LAG config for DX010-1/DX010-2
- 🔲 OpenWrt config for Monos
- 🔲 Intermediate switch selection
- 🔲 SONiC version/build

### Hardware Inventory
- 2× G8316 (16×40G), 2× G8264 (48×10G SFP+ + 4×40G), 1× G8264e (48×10GBase-T + 4×40G)
- 4× Celestica DX010 (1 missing fans/PSU), 1× Arista 7050QX-32-F, 1× Mellanox SX6036
- 3× Mono Gateways, various Cisco/Dell/Netgear switches

### Key Constraints
- Prosumer switch supports LACP but cannot form MC-LAGs itself
- Build system uses `just` and `nix flake`; changes must keep builds working
