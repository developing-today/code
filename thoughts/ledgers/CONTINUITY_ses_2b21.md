---
session: ses_2b21
updated: 2026-04-02T11:10:51.928Z
---

## Summary of Conversation

### Task
Research and verify 3-8 valid web links for each of 8 network hardware devices. Priority: manufacturer datasheets/PDFs > spec pages > reviews/benchmarks. All links must be verified as LIVE using webfetch before reporting.

### Devices Being Researched
1. Cisco 881 ISR
2. Netgear ProSafe XS712T
3. TRENDnet TEG-30284
4. TP-Link Omada SG3210XHP-M2
5. Dell PowerConnect 5448
6. Cisco SG300-52
7. Netgear ProSAFE GS116E
8. Calix GP1101X ONT

### What Was Done
Multiple rounds of webfetch calls were made to search (DuckDuckGo) and verify direct URLs. DuckDuckGo started CAPTCHA-blocking after the first 2 searches, so most research relied on direct URL guessing and verification.

### Verified LIVE Links So Far

**1. Cisco 881 ISR** — ✅ Well covered
- Cisco 880 Series Data Sheet HTML: `https://www.cisco.com/c/en/us/products/collateral/routers/887-integrated-services-router-isr/data_sheet_c78_459542.html` — LIVE
- Cisco 881 Product Page: `https://www.cisco.com/c/en/us/products/routers/881-integrated-services-router-isr/index.html` — LIVE
- ManualsLib Cisco 881 Datasheet: `https://www.manualslib.com/manual/713954/Cisco-881.html` — LIVE (confirmed Cisco 881 content)
- Netgear XS712T datasheet PDF (from initial search, confirms PDFs downloadable): `https://www.downloads.netgear.com/files/GDC/datasheet/en/XS712T.pdf` — LIVE (PDF binary received)

**2. Netgear ProSafe XS712T** — ✅ Well covered
- Netgear XS712T Datasheet PDF (original): `https://www.downloads.netgear.com/files/GDC/datasheet/en/XS712T.pdf` — LIVE (PDF)
- Netgear XS712T+XS728T Combined Datasheet PDF: `https://www.downloads.netgear.com/files/GDC/datasheet/en/XS712T-XS728T.pdf` — LIVE (PDF)
- Netgear XS712T Support Page: `https://www.netgear.com/support/product/xs712t` — returned empty but no error (likely JS-rendered)
- Archive.org XS712T Datasheet PDF: `https://archive.org/download/manualzilla-id-5923700/5923700.pdf` — LIVE (PDF binary received)

**3. TRENDnet TEG-30284** — ✅ Well covered
- TRENDnet TEG-30284 Product Page (new URL format): `https://www.trendnet.com/products/managed-switch/28-Port-Gigabit-Web-Smart-Switch-TEG-30284` — LIVE (full product page with specs)
- Old URL `https://www.trendnet.com/products/product-detail?prod=100_TEG-30284` — shows "product not currently available"
- Support URL `https://www.trendnet.com/support/support-detail.asp?prod=265_TEG-30284` — shows "product not currently available"

**4. TP-Link Omada SG3210XHP-M2** — ✅ Well covered
- TP-Link Product Page: `https://www.tp-link.com/us/business-networking/omada-sdn-switch/sg3210xhp-m2/` — LIVE (full product page with detailed specs)
- TP-Link Specs page (same content, #spec anchor): `https://www.tp-link.com/us/business-networking/omada-sdn-switch/sg3210xhp-m2/#spec` — LIVE

**5. Dell PowerConnect 5448** — ✅ Partially covered
- Dell Support Overview Page: `https://www.dell.com/support/home/en-us/product-support/product/powerconnect-5448/overview` — LIVE (support page loaded)
- Dell Support Manuals URL tried: `https://www.dell.com/support/manuals/en-us/powerconnect-5448/PC5424_UG/introduction` — 404

**6. Cisco SG300-52** — ⚠️ Difficult (retired product)
- Cisco 300 Series Retired Products page: `https://www.cisco.com/c/en/us/products/switches/small-business-300-series-managed-switches/index.html` — LIVE but redirects to retired products listing
- Direct datasheet URL `data_sheet_c78-610061.html` — 404 (Cisco removed docs for retired products)
- ManualsLib manual/677126 — returned wrong product (Sharp microwave, likely ID collision)
- Manualzz.com — returned 403

**7. Netgear ProSAFE GS116E** — ⚠️ Difficult
- `https://www.netgear.com/business/wired/switches/plus/gs116e/` — 404
- `https://www.netgear.com/business/wired/switches/plus/gs116ev2/` — 404
- `https://www.netgear.com/support/product/gs116e/` — empty response (JS-rendered)
- `https://www.netgear.com/support/product/gs116ev2/` — empty response
- `https://www.downloads.netgear.com/files/GDC/datasheet/en/GS116E.pdf` — 403
- `https://www.downloads.netgear.com/files/GDC/datasheet/en/GS116Ev2.pdf` — 403
- ServTheHome review URL tried — 404

**8. Calix GP1101X ONT** — ⚠️ Difficult
- Multiple Calix URL patterns tried — all 404:
  - `/platforms/ont/gp1101x.html`
  - `/platforms/systems/ont/gp1101x.html`
  - `/platforms/intelligent-access-edge/premises-systems/gp1101x.html`
  - `/systems/gpon/gp1101x.html`
  - `/content/dam/calix/mycalix-misc/datasheets/gp1101x-datasheet.pdf`
- Broadbandbuyer — 403

### Remaining Work
- **Cisco 881**: Have 3 verified links. Could try for more (e.g., zen.co.uk PDF, hi-network.com PDF).
- **Netgear XS712T**: Have 3-4 verified links. Solid coverage.
- **TRENDnet TEG-30284**: Have 1 strong verified link. Need more (try datasheet PDF URL).
- **TP-Link SG3210XHP-M2**: Have 1-2 verified links. Need TP-Link datasheet PDF URL.
- **Dell PowerConnect 5448**: Have 1 verified link. Need more (try archive.org, third-party reviews).
- **Cisco SG300-52**: 0 verified product-specific links. Need to try archive.org for cached datasheet, or third-party sites.
- **Netgear GS116E**: 0 verified links. Netgear blocks PDFs (403) and product pages are 404 or JS-rendered. Need to try archive.org, ManualsLib with correct ID, or third-party sites.
- **Calix GP1101X**: 0 verified links. Calix requires login/partner access. Need to try ISP documentation sites, FCC filings, or third-party sources.

### Critical Context
- DuckDuckGo HTML search is CAPTCHA-blocked after ~2 queries per session
- Many manufacturer sites (Netgear downloads, Calix) block or require authentication
- Cisco removed all documentation for retired 300-series switches
- ManualsLib URL IDs don't always correspond to the expected product
- Webfetch can download PDFs (returns binary), confirming the link is live
- The final output format should list each device with numbered links marked LIVE or DEAD with reasons
