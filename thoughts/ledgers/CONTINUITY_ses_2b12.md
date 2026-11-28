---
session: ses_2b12
updated: 2026-04-02T15:38:57.907Z
---

## Summary

### Task
The user asked to copy relevant links from a Google search for "TRENDnet TEG-30284" into the doc/ inventory file.

### What Was Done
- Read the existing file `~/code/doc/inventory/routing-and-switching.md` (900 lines) — a comprehensive routing & switching hardware inventory with detailed specs and curated reference links for each device.
- Found the existing TRENDnet TEG-30284 references section at lines 732-741, which already had 7 links (product page, Reddit networking thread, ModuleTek teardown, SECOMP datasheet PDF, Amazon, Newegg, eBay).
- Compared Google search results against existing links. Identified 3 already present (Amazon, ModuleTek, eBay) and 5 new relevant links.
- Edited the file to add 5 new links (#8-#12) to the TEG-30284 references section:
  1. **TRENDnet TEG-30284 v2.5R Product Page** — current revision with Hive cloud management
  2. **TRENDnet RB-TEG-30284 Refurbished Product Page** — factory refurb listing
  3. **TRENDnet RB-TEG-30284 Support Page** — support downloads for refurb unit
  4. **ServeTheHome Forums — TEG-30284 LAG discussion** — community thread on Link Aggregation configuration
  5. **Reddit r/homelab — 10G Switch TEG-30284 Questions** — community Q&A on homelab 10G use
- Skipped the PCNation marketplace link as it had no unique technical content beyond existing store listings.

### File Modified
- `~/code/doc/inventory/routing-and-switching.md` — Added 5 new reference links to the TRENDnet TEG-30284 section (after line 741, before the TP-Link section).

### No Remaining Work
The task is complete.
