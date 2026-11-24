# Switches

The switches research is the most complete research in the lab
programme — and nothing in it is installed. This chapter folds in
the dedicated research records, states the conclusions they
support, and names the gaps the physical checks still have to
close. Status of the whole chapter: **researched-only,
thoroughly; integration planned, not started.** No switch is
recorded as racked, configured for the cluster, or carrying
cluster traffic.

## The research records

The research lives in `doc/inventory/` in the code repository —
four companion files:

1. `doc/inventory/routing-and-switching.md` — 2,745 lines, last
   updated 2026-04-02. The master inventory: **21 devices**, each
   enriched with a full attribute set (ports/ASIC/capacity, power,
   latency, L2, LAG, MC-LAG, first-hop redundancy, L3 routing,
   security, monitoring, stacking).
2. `doc/inventory/standard-attributes.md` — the standard attribute
   template (sections A–K) every device is enriched against.
3. `doc/inventory/enrichment-checklist.md` — all 21 devices marked
   enriched, one device at a time, with a one-line capability
   digest per device.
4. `doc/inventory/gap-analysis.md` — "Enrichment Gap Analysis —
   2026-04-03": what is still missing after enrichment (below).

An older gap closed by the census: `doc/platform/hardware.md`
explicitly flagged that its own pass had not folded the switching
file in ("switch make/model/count is MISSING here until that file
is folded in"). The fold-in follows.

## The inventory — every switch and router of record

Quantities and classes are as recorded in the summary table of
`routing-and-switching.md`. **Location and state — racked, in
use, boxed — are not stated in that document for any device.**
That absence is exactly what
[#13920](https://github.com/developing-today/code/issues/13920)
now owns.

### Core fabric switches

| Device                         | Qty | Top speed      | Ports                    | Managed     | L3      | Multi-chassis       | Stacking      | Class / era    |
| ------------------------------ | --- | -------------- | ------------------------ | ----------- | ------- | ------------------- | ------------- | -------------- |
| Celestica DX010                | 4   | 100GbE         | 32× QSFP28               | Yes (SONiC) | Yes     | MC-LAG (pairs of 2) | No            | DC, 2016       |
| Arista DCS-7050QX-32-F         | 1   | 40GbE          | 32× QSFP+                | Yes (EOS)   | Yes     | MLAG                | No            | DC, 2013       |
| IBM Mellanox SX6036            | 1   | 56G IB / 40GbE | 36× QSFP                 | Yes         | Limited | No                  | No            | HPC/DC, 2013   |
| IBM RackSwitch G8316           | 4   | 40GbE          | 16× QSFP+                | Yes         | Yes     | vLAG                | Unclear       | DC spine, 2012 |
| IBM RackSwitch G8264           | 3   | 10/40GbE       | 48× SFP+ + 4× QSFP+      | Yes         | Yes     | vLAG                | Yes (up to 8) | DC ToR, 2012   |
| IBM RackSwitch G8264e (G8264T) | 1   | 10/40GbE       | 48× 10GBASE-T + 4× QSFP+ | Yes         | Yes     | vLAG                | Likely        | DC ToR, 2012   |

One of the four DX010 units is missing fans/PSU, noted in the
inventory heading itself — at most three are deployable as-is.

### Routers

| Device              | Qty | Notes from the record                                                                                                                                                     |
| ------------------- | --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Mono Gateway Router | 3   | NXP LS1046A, OpenWrt; summary table: 2× SFP+ 10GbE + 1G ports; enrichment digest: 5× 2.5GbE + 1× SFP+, WiFi 6 tri-radio M.2, DPAA hardware offload, about 15–25 W typical |
| Cisco 2811          | 2   | ISR G1, 2× GbE, HSRP pair candidate                                                                                                                                       |
| Cisco 1841          | 1   | ISR G1 compact, 2× FastEthernet — "lab/learning device" per the enrichment digest                                                                                         |
| Cisco 881           | 1   | ISR G1 SOHO, fanless desktop                                                                                                                                              |

### Managed switches, 10GbE and above

| Device                        | Qty | Notes                               |
| ----------------------------- | --- | ----------------------------------- |
| Netgear ProSafe XS712T-100NES | 1   | 12× 10GBASE-T, smart-managed, no L3 |
| TRENDnet TEG-30284            | 1   | 24× GbE + 4× SFP+, L2+, fanless     |

### Managed switches, 1GbE / 2.5GbE

| Device                     | Qty | Notes                                                                                         |
| -------------------------- | --- | --------------------------------------------------------------------------------------------- |
| TP-Link Omada SG3210XHP-M2 | 2   | 8× 2.5G PoE+ + 2× SFP+, 240 W PoE budget, Omada SDN                                           |
| Dell PowerConnect 5448     | 4   | 48× GbE + 4× SFP combo, stackable up to 12 units (proprietary stacking, single management IP) |
| Cisco SG300-52             | 1   | 50× GbE, L3-lite                                                                              |
| Netgear ProSAFE GS116E     | 1   | 16× GbE, "Plus" web-GUI only                                                                  |

### Cisco Catalyst, specialty and CPE

| Device                             | Qty | Notes                                                            |
| ---------------------------------- | --- | ---------------------------------------------------------------- |
| Cisco Catalyst 3560                | 1   | L3 enterprise; not stackable (the 3750 is the stackable variant) |
| Cisco Catalyst 2960                | 1   | L2; stacking only on S/X variants (FlexStack)                    |
| Cisco ASA 5505                     | 1   | Firewall, 8× FE                                                  |
| Cisco 4402 Wireless LAN Controller | 1   | 802.11a/b/g era, up to 50 APs                                    |
| Calix GP1101X GigaPoint ONT        | 1   | ISP CPE — see [The Osseo site](../site/the-osseo-site.md)        |

## Conclusions the research supports

- **The cluster fabric candidates are the DC-class switches, and
  among them the Celestica DX010 is the only 100GbE fabric:**
  32× QSFP28 (Broadcom Tomahawk, 3.2 Tbps), breakout to 4×25G /
  2×50G / 4×10G per port (up to 128× 25G), SONiC on ONIE, MC-LAG
  pairs, VRRP plus static anycast gateway, BGP/OSPF/IS-IS via
  FRR, VXLAN EVPN, about 150–200 W typical, about 400 ns
  cut-through. One of the four recorded units is missing
  fans/PSU.
- **Dictation resolved:** the "DXO10" of the 2026-11-04 build-out
  dictation is the **DX010**. Drew confirmed on 2026-11-05 that
  the model is the DX010 (recorded in `~/memory/2026-11-05.md` as
  "Celestia DX010" — the inventory's spelling is **Celestica**;
  the GitHub issue title
  [#6071](https://github.com/developing-today/code/issues/6071)
  also reads "Celestia DX010", a spelling variant of the same
  device). #6071 = install **3 more** DX010 units. The tension
  the book must state honestly: the April 2026 inventory already
  lists 4× DX010 owned, so "3 more" reads as installing three of
  the inventoried units, or expanding beyond four — the record
  does not say which. Physical check required (gap 4).
- **No stacking on the top fabric tier:** DX010, Arista 7050QX
  and SX6036 have no stacking — redundancy is MC-LAG/MLAG pairs
  of 2, or ECMP beyond 2. Stacking exists only lower down: IBM
  G8264 (up to 8, QSFP+ 40G links), Dell PowerConnect 5448 (up to
  12), Cisco 2960-S/X only. The inventory's own reference section
  states the trade plainly: stacking = one logical switch /
  single management plane; MC-LAG = independent control planes
  coordinating LAG membership.
- **The IBM fleet is the 10/40G workhorse tier:** G8264 (48× SFP+
  - 4× QSFP+, vLAG, stacking) ×3, its copper sibling G8264e ×1
    (48× 10GBASE-T, about 450–550 W typical — the power-hungriest
    switch in the inventory), G8316 40G spines ×4. The Mellanox
    SX6036 is the InfiniBand/RDMA outlier (36× QSFP, FDR 56G IB or
    40GbE, no MC-LAG, no stacking).
- **The Dell switches of the build-out tickets are not
  model-identified in any record read.**
  [#6073](https://github.com/developing-today/code/issues/6073),
  [#6074](https://github.com/developing-today/code/issues/6074)
  and [#6075](https://github.com/developing-today/code/issues/6075)
  (all open) each require, per switch: rails built and racked →
  firmware updated (version recorded on the ticket) → hardware
  verified → initial config built and deployed → **config stored
  in the code repository** (the ticket establishes a
  `lab/switches/<hostname>/` home if none exists) →
  physical labels (hostname, role, management address). Each
  ticket's record section is where model, hostname and management
  address will land. The only Dell switches in the research
  inventory are the PowerConnect 5448 ×4 — **the record nowhere
  states that the three build-out Dells are 5448s**, and this book
  does not assert it. This is the single biggest switches gap
  (gap 3) and the first slice of #13920.
- **NIC/switch sanity check is still open:** the cluster design
  needs the exact high-speed NIC specification on the core
  machines and the switch gear between them confirmed, to
  sanity-check LINSTOR/DRBD replication traffic —
  [#1097](https://github.com/developing-today/code/issues/1097)
  (open), under network-fabric epic
  [#1084](https://github.com/developing-today/code/issues/1084).
  The register's dictated "at least 200 Gb ports" remains
  unverified — with a 100GbE-max fabric of record, that figure
  cannot be taken at face value.
- **Documentation-survival caveat (from the inventory's
  References note):** many of these devices are end-of-life or
  discontinued; Cisco removes documentation for retired products
  and IBM/Lenovo/Mellanox legacy documentation is partially gone.
  The inventory links archived copies and community sources where
  the manufacturer's are dead, and lists dead or unresolvable
  links per device rather than hiding them.

## What the gap analysis says is still missing (2026-04-03)

All 21 devices are enriched; the remaining gaps are
attribute-level, per `doc/inventory/gap-analysis.md`:

- **Systemic (nearly every applicable device):** protocol-based
  VLAN (C4), trunk negotiation (C7), quantified STP convergence
  time (C9), LAG failover time (D7), min-links (D8), CoPP/CPU
  rate-limiting (J7), port security (J8), DNS client (K7). High
  priority because they are one-liners for most devices: C7, C9,
  D7, D8, K7.
- **Heaviest per-device gaps:** the Mellanox SX6036 (whole
  attribute families missing — C, D, F, G, J, K sections), the
  Cisco 1841 and 881 routers, and the IBM G8264/G8264e/G8316
  routing, security and monitoring rows (G5, G7–G9, J3–J5).
- Router C-section gaps and GS116E/GP1101X gaps are assessed as
  expected or negligible for their device classes.

## The owning issue and the physical-check list

The owning issue for completing the inventory and then
integrating and racking is
[#13920](https://github.com/developing-today/code/issues/13920)
(open, filed under
[#13905](https://github.com/developing-today/code/issues/13905)
because the census found the seed had no owning issue). The
per-switch physical tickets are #6073–#6075, the fabric epic is
#1084, and the interconnect that depends on them is
[#6077](https://github.com/developing-today/code/issues/6077)
(open, labelled blocked).

One dating fact the book keeps visible: the switches research is
dated 2026-04-02/03 — the newest fact in it is seven months older
than the rest of the programme. Firmware versions, device
survival, and the DX010's missing fans/PSU may have changed. A
physical check list is #13920's stated output, and until it
exists, this chapter is a research record, not a site survey.
