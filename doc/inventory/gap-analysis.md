# Enrichment Gap Analysis — 2026-04-03

All 21 devices in `routing-and-switching.md` have been enriched against the
`standard-attributes.md` template (sections A–K). This document summarises
the remaining gaps found by comparing each device's enrichment rows with every
applicable attribute ID.

---

## 1. Systemic Gaps (missing from nearly every applicable device)

| Attr ID | Description | Scope | Notes |
|---------|-------------|-------|-------|
| **C4** | Protocol-based VLAN | All switches | Rarely documented; many devices don't support it — mark N/A where appropriate |
| **C7** | Trunk negotiation (DTP / manual) | All switches | Easy to fill — most are "manual only" or "DTP (Cisco)" |
| **C9** | STP convergence time | All STP-capable | Needs quantified ms value for best STP variant |
| **D7** | LAG failover time | All LAG-capable | Typically <50 ms for LACP; state explicitly |
| **D8** | Min-links | All LAG-capable | Supported or not — one-liner |
| **J7** | CoPP / CPU rate-limiting | Most switches | Present on enterprise gear, absent on prosumer |
| **J8** | Port security | DC switches especially | Often supported but not documented in enrichment |
| **K7** | DNS client | All 21 devices | Trivial — most support DNS client for management |

## 2. Per-Device-Class Gaps

### 2a. DC Switches (#1–#6)

| Device | Key Missing Attrs |
|--------|------------------|
| #1 DX010 | C4, C5(Q-in-Q), C9, D8, E5/E6(partial), J8, K6(PTP), K7 |
| #2 G8264 | C9, D7, D8, G5, G7, G8, G9, J3–J5, J7, J8, K3(RSPAN/ERSPAN), K7 |
| #3 G8264e | Same as G8264 |
| #4 G8316 | C9, D7, D8, G5, G8(partial), G9, J3–J5, J8, K3(RSPAN/ERSPAN), K7 |
| #5 SX6036 | C2–C5, C9, C10, D3(partial), D4, D6–D8, F1/F5–F7, G4–G9, J2–J5, J8, K2, K3(RSPAN), K6(PTP), K7 |
| #6 Arista 7050 | C5, C9, D6–D8, F7, J5, J8, K7 |

### 2b. Routers (#7–#10)

| Device | Key Missing Attrs |
|--------|------------------|
| #7 Mono GW | C10, C11, D7, D8, F4/F6/F7, H1–H5(partial)/H7, J4–J8, K3, K6(PTP) |
| #8 Cisco 2811 | C2–C7, C9–C11, D3/D7/D8, H3/H7, J5/J6/J8, K4, K7 |
| #9 Cisco 1841 | C2–C12, D1/D3–D8, F6/F7, G5/G10, H3/H5–H7, J2–J6/J8, K3/K4/K7 |
| #10 Cisco 881 | C2–C7, C9–C11, D(all N/A), F3/F6/F7, G5/G9/G10, H3/H5–H7, J4–J6/J8, K3/K4/K7 |

### 2c. Prosumer Switches (#11–#14)

| Device | Key Missing Attrs |
|--------|------------------|
| #11 XS712T | B6, C4/C7/C9/C14, D7/D8, J3–J7, K2/K4/K7 |
| #12 TEG-30284 | C4/C7/C9/C14, D7/D8, J6/J7, K2/K7 |
| #13 SG3210XHP | A7, C4/C7/C9/C14, D7/D8, J6/J7, K2/K7 |
| #14 PowerConnect 5448 | C4/C7/C9/C14, D7/D8, J4–J7, K2/K7 |

### 2d. SMB / Consumer Switches (#15–#16)

| Device | Key Missing Attrs |
|--------|------------------|
| #15 SG300-52 | B5/B8, C4/C7/C9, D7/D8, G6–G10(mostly N/A), J7, K4/K7 |
| #16 GS116E | C4/C7, J4/J5/J7, K4/K7 (most other gaps are correctly N/A) |

### 2e. Enterprise Switches (#17–#18)

| Device | Key Missing Attrs |
|--------|------------------|
| #17 Cisco 3560 | A4/A7/A10/A12(partial), B5/B8, C4/C7/C9, D7/D8, F4/F5, G5/G9, K4(partial)/K7 |
| #18 Cisco 2960 | A4/A7/A12(partial), B5/B7/B8, C4/C7/C9, D7/D8, J7, K4(partial)/K7 |

### 2f. Firewall (#19)

| Device | Key Missing Attrs |
|--------|------------------|
| #19 ASA 5505 | A4–A7(N/A unstated), B6/B8, C10/C14, H6/H7, J1(partial)/J7, K4/K7 |

### 2g. Wireless Controller (#20)

| Device | Key Missing Attrs |
|--------|------------------|
| #20 Cisco 4402 WLC | A1/A3/A12, B1/B2, K2/K4/K7 |

### 2h. ONT (#21)

| Device | Key Missing Attrs |
|--------|------------------|
| #21 Calix GP1101X | B2, C10, C14 (all minor for device class) |

## 3. Priority for Gap-Filling

**High (systemic, easy to fill):**
- C7, C9, D7, D8, K7 — one-liner answers for most devices

**Medium (improves completeness):**
- C4, J7, J8, K4 — require brief research per device
- IBM G8264/e/G8316: G5, G7–G9, J3–J5

**Low (N/A or negligible impact):**
- Router C-section gaps (expected for device class)
- GS116E gaps (unmanaged+ switch, most features don't apply)
- GP1101X gaps (ISP-managed ONT)
