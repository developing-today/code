---
session: ses_2b21
updated: 2026-04-02T11:14:29.556Z
---

## Summary

### Task
Research and verify 3-8 valid web links per device for 6 network hardware devices. Priority: manufacturer datasheets/PDFs > spec pages > reviews/benchmarks. Every URL must be verified with webfetch before claiming LIVE.

### Devices & Results So Far

**1. Cisco Catalyst 3560** — L3 switch
- ✅ LIVE: `https://www.cisco.com/c/en/us/products/switches/catalyst-3560-series-switches/index.html` — Series product/support page (End of Support, lists retired models)
- ❌ DEAD: `https://www.cisco.com/c/en/us/products/collateral/switches/catalyst-3560-series-switches/product_data_sheet0900aecd8034699f.html` — 404
- ❌ DEAD: Several alternate datasheet URL patterns tried (data_sheet_c78-530684, datasheet-c78-530684) — all 404
- ❌ DEAD: archive.org for the datasheet — 404
- ❌ ManualsLib returned wrong product (a Changhong freezer at manual ID 868756)
- ❌ manualzz.com — 403 forbidden
- Still needs: more working links (only 1 found)

**2. Cisco Catalyst 2960** — L2 switch
- ✅ LIVE: `https://www.cisco.com/c/en/us/products/collateral/switches/catalyst-2960-series-switches/product_data_sheet0900aecd806b0bd8.html` — Full LAN Lite datasheet with specs, PoE, QoS, hardware details
- ✅ LIVE: `https://www.cisco.com/c/en/us/products/switches/catalyst-2960-series-switches/index.html` — Redirects to 2960-X series support page (still loads, contains 2960 references)
- Still needs: more links for better coverage

**3. Cisco ASA 5505** — Firewall/VPN appliance
- ✅ LIVE: `https://www.cisco.com/c/en/us/products/collateral/security/asa-5500-series-next-generation-firewalls/datasheet-c78-733510.html` — Full ASA 5505 datasheet (features, specs, ordering info)
- ✅ LIVE: `https://www.cisco.com/c/en/us/support/security/asa-5505-adaptive-security-appliance/model.html` — Redirects to ASA 5500-X support page listing ASA 5505 datasheet among docs
- ❌ DEAD: `https://www.cisco.com/c/en/us/products/collateral/security/asa-5505-adaptive-security-appliance/datasheet-c78-733510.html` — 404 (wrong path)
- ❌ DEAD: `https://www.cisco.com/c/en/us/products/security/asa-5505-adaptive-security-appliance/index.html` — 404
- ManualsLib returned wrong product (OKI printer)

**4. Cisco 4402 WLC** — Wireless LAN Controller
- ✅ LIVE: `https://www.cisco.com/c/en/us/products/wireless/4400-series-wireless-lan-controllers/index.html` — Retirement notification page (EoS 2011-06-13, EoSupport 2016-06-30)
- ❌ DEAD: `https://www.cisco.com/c/en/us/products/collateral/wireless/4400-series-wireless-lan-controllers/product_data_sheet0900aecd802930c5.html` — 404
- ❌ DEAD: `https://www.cisco.com/c/en/us/products/wireless/4402-wireless-lan-controller/index.html` — 404
- ❌ DEAD: `https://www.cisco.com/c/en/us/support/wireless/4402-wireless-lan-controller/model.html` — 404
- ❌ DEAD: archive.org for datasheet — 404
- ❌ DEAD: Various alternate Cisco URL patterns — all 404
- Hardest device — very few live links for this retired product

**5. IBM RackSwitch G8264e** — 10G/40G ToR switch
- ✅ LIVE: `https://lenovopress.lenovo.com/tips1272-lenovo-rackswitch-g8264` — Full product guide for G8264 (base model, covers G8264 family: 48x SFP+ 10G, 4x QSFP+ 40G, 1.28Tbps, specs, models, features)
- ✅ LIVE: `https://lenovopress.lenovo.com/tips1273-lenovo-rackswitch-g8264cs` — Product guide for G8264CS converged variant (FCoE/FC Omni Ports, related model in same family)
- ❌ DEAD: `https://www.ibm.com/docs/en/rackswitch-g8264` — 404
- Note: The "G8264e" specifically (copper 10GBASE-T variant) may not have its own dedicated page; the G8264 and G8264CS are the closest documented variants on Lenovo Press

**6. IBM RackSwitch G8316** — 16x QSFP+ 40G spine switch
- ❌ NO Lenovo Press TIPS page found (tried TIPS1264 through TIPS1275; these map to other products: G8296, G7028, G7052, G8052, G8124E, G8264, G8264CS, G8332, N4610 storage)
- ❌ DEAD: `https://www.ibm.com/docs/en/rackswitch-g8316` — 404
- ❌ DEAD: `https://lenovopress.lenovo.com/tips1265-lenovo-rackswitch-g8316` — 404
- This is likely an older IBM-era product that was never migrated to Lenovo Press. The G8332 (TIPS1274) is the successor/related 40G spine switch.
- Hardest device — no verified live links found yet

### Blockers Encountered
- **DuckDuckGo HTML search** is completely blocked with CAPTCHA on all queries — cannot use for searching
- **ManualsLib** search by manual ID is unreliable — returned wrong products for IDs 868756 and 410181
- **Cisco old datasheet URLs** have been widely retired/404'd for end-of-life products (3560, 4402 WLC)
- **IBM docs** for older RackSwitch products appear fully decommissioned

### What Remains
- Devices 1, 4, and 6 need more live links (only 1 or 0 found each)
- Could try: different archive.org snapshot dates, other third-party sites (NetworkWorld reviews, ServersPlus spec pages, etc.), Cisco community/forum pages, direct PDF links
- Final formatted output per device hasn't been delivered yet
- No files were modified — this is a pure research/URL-verification task
