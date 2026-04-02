---
session: ses_2b10
updated: 2026-04-02T22:41:38.242Z
---

## Summary of Work Done So Far

### Overall Project
Building a network infrastructure for a homelab with IBM/Lenovo switches and SONiC-based switches, documented across two main files.

### Completed Work

#### 1. Network Topology Design (`docs/network-topology.md`) ✅
- Designed 3-VLAN hybrid port topology with DX010 MC-LAG pair and Mono routers
- Decided on MTU, PMTUD, ARP sysctls, hybrid port design

#### 2. Inventory Documentation Fixes (`doc/inventory/routing-and-switching.md`) ✅
- **G8264 stacking**: Corrected from "No" → "Yes (up to 8 switches, QSFP+ 40G, ring or daisy-chain)" based on Lenovo Press TIPS1272
- **G8264e stacking**: Corrected from "No" → "Likely yes (same platform/firmware)"
- **Stacking Reference Table** (lines 508-510): Updated G8264=Yes, G8264e=Likely, G8316=No
- **Summary Table** (lines 528-529): Updated G8264="Yes (8)", G8264e="Likely"
- TIPS1272 and TIPS0842 already added to references

#### 3. G8316 Link Review (8 URLs from user) ✅
Reviewed 8 user-provided links for G8316 stacking/vLAG info:
- **Key discovery**: TIPS0842 and IBM Support page both list "Stacking LEDs to indicate Master/Member" in physical specs, despite stacking NOT being listed as a feature. This is ambiguous — may refer to vLAG master/member roles.
- **G8316 confirmed capabilities**: vLAG, VRRP, LACP (64 trunk groups/32 ports each), Hot Links, full L3 (OSPF, BGP, RIP), OpenFlow 1.0/1.3.1, CEE/DCB, FCoE transit, 128K MAC, 9216 jumbo
- Several links were dead, paywalled, or unreadable PDFs; identified 2-3 new links worth adding to references

### Remaining Work (This Task)
1. 🔲 **Report G8316 "Stacking LEDs" finding** to user and discuss implications — possibly update G8316 stacking from "No" to "Possible/Unclear"
2. 🔲 **Add new reference links** to G8316 section (IBM Install Guide PDF, Switch Center 8.1.4 Release Notes)

### Remaining Work (Broader Project)
- 🔲 SONiC MC-LAG configuration for DX010-1/DX010-2
- 🔲 OpenWrt config for Monos
- 🔲 Intermediate switch selection
- 🔲 SONiC version/build selection

### Hardware Inventory
- 2× G8316 (16×40G)
- 2× G8264 (48×10G SFP+ + 4×40G)
- 1× G8264e (48×10GBase-T + 4×40G)
- DX010 MC-LAG pair (SONiC)
- Mono routers (OpenWrt)

### Key Constraint
- Prosumer switch supports LACP but cannot form MC-LAGs itself.
