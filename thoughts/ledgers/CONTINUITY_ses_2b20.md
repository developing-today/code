---
session: ses_2b20
updated: 2026-04-02T11:46:49.788Z
---

## Summary

### Task
The user asked me to search for and verify reference links for 5 networking devices: **Calix GP1101X**, **Cisco 2811**, **Cisco 1841**, **Cisco SG300-52**, and **Netgear GS116E**. For each device, I needed to find 3-8 valid links (preferring manufacturer PDFs/datasheets, then specs/reviews/benchmarks), actually fetch each link to verify it's live (HTTP 200), and report dead links separately. Archive.org should be used for dead pages.

### What Was Done
- Attempted searches via Google, DuckDuckGo, and Bing — all blocked automated queries (CAPTCHAs or generic results ignoring the quoted product names)
- Performed **50+ direct URL fetches** against known/likely URLs for each device across: manufacturer sites, archive.org, FCC database, third-party spec sites (router-switch.com, itprice.com, speedguide.net, manualslib.com, smallnetbuilder.com, bhphotovideo.com, newegg.com, amazon.com, techpowerup.com, etc.)
- Archive.org CDX API returned 503 errors; Wayback Machine browse pages require JavaScript rendering that webfetch can't execute

### Key Findings

**All 5 devices are retired products with deliberately removed documentation:**

| Device         | Live Links Found                              | Dead Links | Issue                                                                            |
| -------------- | --------------------------------------------- | ---------- | -------------------------------------------------------------------------------- |
| **Calix GP1101X**  | 0                                             | 9 tested   | Partner login wall; FCC ID not found; SpeedGuide URLs redirect to wrong products |
| **Cisco 2811**     | 1 (retired products confirmation page)        | 8 tested   | Cisco deliberately purges all retired product docs                               |
| **Cisco 1841**     | 1 (retired products confirmation page)        | 6 tested   | Same Cisco purge policy                                                          |
| **Cisco SG300-52** | 2 (retired products + support redirect pages) | 5 tested   | Same Cisco purge policy, listed under Small Business 300 Series                  |
| **Netgear GS116E** | 0                                             | 12 tested  | Product pages return blank/404; datasheet PDF returns 403                        |

**Live verified URLs:**
- `https://www.cisco.com/c/en/us/obsolete/routers/cisco-2800-series-integrated-services-routers.html` — Cisco retired routers page (2800 series including 2811)
- `https://www.cisco.com/c/en/us/obsolete/routers/cisco-1800-series-integrated-services-routers.html` — Cisco retired routers page (1800 series including 1841)
- `https://www.cisco.com/c/en/us/obsolete/switches/cisco-small-business-300-series-managed-switches.html` — Cisco retired switches page (300 Series including SG300-52)

### Recommendations Provided to User
1. Use Wayback Machine in a real browser (needs JS) to find archived Cisco datasheets
2. For Calix GP1101X: ISP community forums or r/networking; Calix Community requires partner login
3. For Netgear GS116E: Try Wayback Machine snapshots of the product page URL in a browser
4. eBay listings often have seller-uploaded spec sheets as informal documentation

### What Remains
- The user hasn't responded yet to the report
- No files were modified — this was purely a web research task
- The core problem is that **search engines are not usable via webfetch** due to bot detection, and most of these products' documentation has been deliberately removed by manufacturers
