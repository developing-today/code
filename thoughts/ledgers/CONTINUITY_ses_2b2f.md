---
session: ses_2b2f
updated: 2026-04-02T11:27:21.745Z
---



## Conversation Summary

### Task
Add a **References section** to `~/code/docs/inventory/routing-and-switching.md` — 3-8 valid links per device (21 devices total), preferring manufacturer PDFs/datasheets, then specs/reviews/benchmarks. Dead links use web.archive.org; unresolvable links go in a subsection (don't count toward 3-8 minimum).

### What Was Done
1. **Research phase COMPLETE** for all 21 devices across multiple sessions
2. **References section WRITTEN** — appended after line 544 (Summary Table) with per-device `###` subsections
3. **G8316 FIXED** — User pointed out I should have just Googled it instead of guessing TIPS numbers. Found and verified 5 links:
   - Lenovo Press TIPS0842 (19-page product guide)
   - IBM Support overview page
   - karma-group.ru datasheet PDF
   - IBM Boulder Networking OS 7.4 Release Notes PDF
   - IT Jungle launch article (2011, $35,999 pricing)
4. **Key learning**: Google search finds links that URL-guessing misses. User wants me to apply same approach to remaining hard cases.

### Current Coverage (after G8316 fix)
| Coverage                       | Devices                                                                                                                                                |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **3-5 links**                      | DX010(5), G8264(5), G8316(5), Cisco 881(3), Netgear XS712T(3)                                                                                          |
| **1-2 links**                      | G8264e(2), Arista 7050QX-32(1), Mono Gateway(2), TEG-30284(1), SG3210XHP-M2(1), Dell PC5448(1), Cisco 3560(1), Cisco 2960(2), ASA 5505(2), 4402 WLC(1) |
| **0 links (noted as unavailable)** | SX6036, Cisco 2811, Cisco 1841, Cisco SG300-52, Netgear GS116E, Calix GP1101X                                                                          |

### What Needs to Be Done Next
1. **Google search for remaining hard cases** (as user instructed): SX6036, Cisco 2811, Cisco 1841, Cisco SG300-52, Netgear GS116E, Calix GP1101X — use actual Google/web search rather than guessing URLs
2. **Verify found links** and update each device's section in the file
3. **Consider searching for more links** for devices with only 1-2 links to try to reach the 3-link minimum
4. Update core_memory when complete

### Key Findings
- **Cisco deliberately removes ALL docs for retired products** (confirmed on their retired products page)
- **Lenovo Press TIPS numbers are NOT sequential by product** — TIPS0842=G8316, TIPS1272=G8264, TIPS1273=G8264CS, TIPS1271=G8124E
- **ManualsLib manual IDs are NOT predictable** — random IDs, URL guessing returns wrong products
- **Mellanox docs absorbed by NVIDIA** — many legacy PDFs gone (404)
- **Calix requires partner login** for all documentation
- **Acclinet** (third-party reseller with G8316 page) has expired SSL cert — skipped

### File Being Modified
- `~/code/docs/inventory/routing-and-switching.md` — References section starts after line 544

### Key User Instruction
User explicitly said: search Google for the other hard-case devices the same way they found the G8316 links, rather than guessing URLs. The approach of just googling `"ibm g8316"` immediately found TIPS0842, IBM Support page, karma-group PDF, IBM Boulder PDF, and IT Jungle — all of which I had failed to find by URL guessing.
