---
session: ses_2b22
updated: 2026-04-02T11:07:06.567Z
---

## Summary of Conversation

### Task
Research and verify web links for 7 network hardware devices to add to a documentation file. For each device, find 3-8 valid web links (prioritizing manufacturer PDFs/datasheets > official spec pages > detailed reviews). Verify each link actually loads.

### Devices Researched
1. **IBM RackSwitch G8264e** (48x 10GBASE-T + 4x QSFP+ 40G)
2. **IBM RackSwitch G8316** (16x QSFP+ 40G spine switch)
3. **IBM/Mellanox SX6036** (36-port QSFP FDR InfiniBand 56Gbps / 40GbE VPI switch)
4. **Arista DCS-7050QX-32-F** (32x QSFP+ 40G, EOS, 550ns latency)
5. **Mono Gateway Router** (NXP LS1046A, 2x SFP+ 10G + 3x RJ45, OpenWrt)
6. **Cisco 2811 ISR** (G1 router, 2x GbE, IOS)
7. **Cisco 1841 ISR** (2x FastEthernet, IOS)

### What Was Done
- Attempted dozens of URLs across manufacturer sites (LenovoPress, Cisco, Arista, NVIDIA/Mellanox, NXP, mono.si), Wayback Machine, and review sites (ServeTheHome)
- Discovered LenovoPress uses non-obvious TIPS numbering: mapped G8264→TIPS1272, G8264CS→TIPS1273, G8332→TIPS1274, G8272→TIPS1267, etc. No dedicated G8264e or G8316 pages exist.

### Verified LIVE Links Found

| Device           | Live Links | Key URLs                                                                                                                                     |
| ---------------- | ---------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| G8264e           | 1          | `lenovopress.lenovo.com/tips1272-lenovo-rackswitch-g8264` (G8264 family, not G8264e specifically)                                              |
| G8316            | 1          | `lenovopress.lenovo.com/tips1274-lenovo-rackswitch-g8332` (successor G8332 only)                                                               |
| SX6036           | 2          | `nvidia.com/en-us/networking/infiniband-switching/` and `nvidia.com/.../enterprise/networking/` (portals only; all legacy Mellanox PDF URLs 404) |
| Arista 7050QX-32 | 1          | `arista.com/en/products/7050x-series` (full specs inline, PDF links need auth)                                                                 |
| Mono Gateway     | 3          | `nxp.com/.../LS1046A` product page, `nxp.com/docs/en/data-sheet/LS1046A.pdf` (confirmed PDF), `nxp.com/docs/en/fact-sheet/LS1046AFS.pdf`           |
| Cisco 2811       | 1          | `cisco.com/.../obsolete/routers/cisco-2811-...` (redirects to retired listing)                                                                 |
| Cisco 1841       | 1          | `cisco.com/.../obsolete/routers/cisco-1841-...` (redirects to retired listing)                                                                 |

### Key Findings
- **Cisco** intentionally purges ALL documentation for retired products (datasheets, EOL bulletins, config guides all 404)
- **Mellanox/NVIDIA** legacy `network.nvidia.com` PDF URLs all return 404 (SX6036 is EOL)
- **mono.si** appears completely down (all URLs return 404)
- **OpenWrt wiki** uses Anubis bot protection, blocking automated fetches
- **ServeTheHome** also returned 404 for the IBM G8264 review
- **Arista** product page is live with full specs but direct PDF download URLs return 400 (may need auth/session)
- **NXP LS1046A** has the best surviving documentation of all devices researched

### Remaining Work
- No additional link verification was requested yet
- The user has not yet indicated where/how to insert these links into a documentation file
- Could attempt more Wayback Machine URL patterns or third-party sources (university mirrors, reseller sites) if more links are needed
