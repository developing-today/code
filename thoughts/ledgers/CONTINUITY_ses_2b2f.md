---
session: ses_2b2f
updated: 2026-04-02T15:34:03.441Z
---



## Summary of Work Done This Session

### Task
Add community/forum/video links to ALL 21 devices in `~/code/docs/inventory/routing-and-switching.md`. No cap on links — add all useful links found. Dead links use web.archive.org or `#### Dead/Unresolvable Links` subsections. Skip reseller listings.

### Completed Link Additions (12 devices updated)

| Device        | Before → After      | Change       |
| ------------- | ------------------- | ------------ |
| 4402 WLC      | 2 → 9               | +7           |
| 2960          | 4 → 9+1dead         | +5           |
| 3560          | 5+1dead → 8+1dead   | +3           |
| ASA 5505      | 7 → 13              | +6           |
| G8316         | 7 → 7               | no new found |
| 881           | 3+1dead → 11+1dead  | +8           |
| GS116E        | 5 → 9               | +4           |
| SG3210XHP-M2  | 4 → 7               | +3           |
| G8264e        | 4 → 6+1dead         | +2, +1dead   |
| SG300-52      | 10+1dead → 11+1dead | +1           |
| 2811          | 9 → 11              | +2           |
| Calix GP1101X | 9 → 10              | +1           |

### Current State
- User just pasted Google search results for **SG3210XHP-M2** (second search). I checked the current section — it already has 7 links. The search results show: Amazon (reseller, skip), ServeTheHome non-PoE variant (already link #6), Omada downloads (already link #7), TP-Link AE regional page (new candidate), ServeTheHome deal thread page 2 (already link #5), Newegg/CDW/ADI (all resellers, skip).
- The TP-Link AE regional page (`tp-link.com/ae/business-networking/managed-switch/tl-sg3210xhp-m2/v1.6/`) is a new candidate but is a regional duplicate of existing link #1. **Likely no new links from this search.**

### Remaining Devices Needing Google Search Pastes
- **HIGH**: Cisco 1841 (currently 4 links)
- **MEDIUM**: Dell PowerConnect 5448 (7 links), TRENDnet TEG-30284 (7 links)
- **LOW**: Most others already well-covered (9+ links)

### Key Technical Details
- **File**: `~/code/docs/inventory/routing-and-switching.md` (~900 lines)
- **DNS workaround**: Environment DNS (100.100.100.100) sometimes returns SERVFAIL; use `dig @8.8.8.8` + `curl --resolve` 
- **Cisco Community**: Always returns 403 (Cloudflare bot block) but pages are confirmed real
- **Google CAPTCHA**: Blocks browser automation, so user pastes search results manually
