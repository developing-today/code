# Source-occurrence preservation ledger

Generated rows: **3572**. These are mechanically segmented occurrences, explicitly **not atomic requirements**. Repetitions are retained, not coalesced.

Targets below mean planned design destinations, not implementation/issue coverage.

| Source | Units | SHA-256 (repo bytes) |
|---|---:|---|
| `01_handoff_pm_stack_fleet_orchestration.md` | 166 | `e293349ec31e997ac3330d8df947f121e8aba537477b1bfd6e6f8290b85675c0` |
| `02_charter_v1_1.md` | 253 | `293925e58f5b19f319ec494665ec7646d36f9bdb1f823501ed8211ad8dfd9136` |
| `03_full_orchestration_proposal.md` | 1776 | `7d24690d14409e4dc32876a50d1a7ac19fe3599b0ecaed7ae909009baec695d1` |
| `04_recovered_original_combined_design.md` | 1311 | `6607fba35aee81e7e4e521e8ff7cb6ee31280823c667b9c22f3766fd30edff86` |
| `05_supplemental_raw_note.md` | 66 | `d5855c278a7f7276adb7bf32ea25da1b41ab978fb3d9e207154d1803d4cf1121` |

## Coverage by domain

| Domain | Row occurrences |
|---|---:|
| priority | 478 |
| distribution | 76 |
| pm-mobile | 248 |
| dispatch-events | 400 |
| queue-selection | 508 |
| lifecycle-cli | 407 |
| hydration-activity | 308 |
| dependencies-authority | 536 |
| leases-outbox-completion | 272 |
| crosscutting | 1584 |

## Validation boundary

`check_inventory.py --check` compares sources against the saved manifest and verifies the entire deterministic occurrence ledger (IDs, byte/character spans, quotes, order, and count). It cannot establish semantic atomicity, implementation status, or named-issue coverage. It excludes whitespace and sentence-ending separator punctuation; it retains punctuation attached to identifiers/paths such as `.fleet`.

The ledger is a mechanical preservation seed, not the curated requirement/crosswalk deliverable. `source-section-map.md` maps source sections to candidate starter domains. `conflicts.md` records known conflicts and precedence decisions. Do not infer complete requirement coverage from the ledger or from a design-target suggestion.
