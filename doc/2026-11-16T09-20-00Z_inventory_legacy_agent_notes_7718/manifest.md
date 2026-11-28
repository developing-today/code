# Inventory: legacy agent notes under the #6022 placement policy (code#7718)

- Generated: 2026-11-16 by repo-worker-code lane code-a (task 14510)
- Policy applied: `doc/2026-11-10T20-00-00Z_design_agent_note_placement.md` (#6022) — the §1 orphan test, the §2 decision table, and §6 migration expectations (grandfathered in place, move on next touch, no bulk move).
- Scope: `~/workspace/research_notes/` per folder (27) and completed/active `~/workspace/goals/**` workspaces per workspace (53). Excluded: every document enumerated by #5981 (none of these folders is a fleet/architecture repo doc) and all sandbox-environment material classified STAY by #5983 (these are research outputs, not host/tooling notes). `~/memory/` is reference-only per the issue scope: not read, copied, or inventoried — pointer only (policy §2: memory is a home, Tier 3).
- No migration was performed. Recommended homes are promotion targets for next touch, via the single `note_submission` envelope for zettel-bound items (staging → human promotion; nothing auto-commits).
- Sensitivity: automated pattern scan only; hits are topical word mentions in usage/pricing research, no credential values were extracted or are recorded here. A human sensitivity check is still required before any individual promotion.
- Content hash: SHA-256 of the folder's primary report file (full per-folder file lists and the machine-readable manifest are in `manifest.json` beside this file).

## research_notes — classified candidates and non-candidates

| Folder                                                   | Class                               | Files | Primary SHA-256 (short) | Recommended home                                                |
| -------------------------------------------------------- | ----------------------------------- | ----- | ----------------------- | --------------------------------------------------------------- |
| `ai-plans-comparison-retry-20261002-1624`                | research synthesis                  | 2     | `4b2235e74313`          | zettel via note_submission (Tier 1)                             |
| `ainventory-aa-rerun-20261031`                           | research synthesis + extracted data | 7     | `5250a08fad55`          | zettel via note_submission (Tier 1); extracted data stays local |
| `aster-utilization-20261029`                             | durable finding                     | 1     | `2b156953086e`          | zettel via note_submission (Tier 1)                             |
| `claude-code-cli-multisession-daemon-chec-20261025-1131` | durable finding                     | 3     | `65aaf0610ad4`          | zettel via note_submission (Tier 1)                             |
| `cloud-db-free-paid-comparison-20261030-1659`            | research synthesis                  | 3     | `eece51184a57`          | zettel via note_submission (Tier 1)                             |
| `cloudflare-workers-versions-deployments-20261029-0053`  | durable finding                     | 3     | `3b0878848b1b`          | zettel via note_submission (Tier 1)                             |
| `codex-review-value-20261029`                            | scratch / raw data                  | 43    | `ffc0947abd87`          | retain locally (not a candidate)                                |
| `codex-usage-investigation-20261029`                     | durable finding (fleet operations)  | 1     | `819a84805960`          | zettel via note_submission (Tier 1)                             |
| `free-llm-models-inferx-opencode-alternat-20261031-1712` | research synthesis                  | 2     | `00a664b9ee05`          | zettel via note_submission (Tier 1)                             |
| `go-fallthrough-20261029`                                | durable finding (repo-specific)     | 2     | `7243f34dbd99`          | repo doc, developing-today/code doc/ (Tier 2)                   |
| `google-ai-pro-drive-gcp-limits-20261025-1857`           | superseded draft                    | 2     | `0a60708cad12`          | retain locally (superseded by v2)                               |
| `google-ai-pro-drive-gcp-limits-v2-20261025-1928`        | durable finding                     | 2     | `7d30c893aff9`          | zettel via note_submission (Tier 1)                             |
| `local-model-features-20261029`                          | durable finding                     | 1     | `f18cd1155b58`          | zettel via note_submission (Tier 1)                             |
| `merge-queue-stall-20261029`                             | durable finding (repo-specific)     | 1     | `7ff0ee05282c`          | repo doc, developing-today/code doc/ (Tier 2)                   |
| `model-effort-size-personality-research-20261021-1056`   | research synthesis                  | 3     | `62d9d406b8d7`          | zettel via note_submission (Tier 1)                             |
| `muse-meta-product-roadmap-20261001-0030`                | research synthesis                  | 4     | `571e0d8b7d0b`          | zettel via note_submission (Tier 1)                             |
| `nix-deployment-patterns-research-20261028-2131`         | research synthesis                  | 3     | `1b410cef0128`          | zettel via note_submission (Tier 1)                             |
| `openclaw-opencode-single-server-research-20261027-0359` | research synthesis                  | 3     | `a2c1bccb99eb`          | zettel via note_submission (Tier 1)                             |
| `openclaw-scheduled-tasks-20261029`                      | durable finding                     | 1     | `19047e0f4ef2`          | zettel via note_submission (Tier 1)                             |
| `openclaw-vs-opencode-20261029`                          | scratch / raw data                  | 44    | `d60729e1cb71`          | retain locally (not a candidate)                                |
| `opencode-scheduled-tasks-20261029`                      | durable finding                     | 4     | `ecf1bef30f8d`          | zettel via note_submission (Tier 1)                             |
| `plus-vs-pro-value-research-20261001-1253`               | research synthesis                  | 3     | `c6266efea866`          | zettel via note_submission (Tier 1)                             |
| `quota-burn-plan-20261029`                               | working state / plan iterations     | 74    | `5b4a119d061c`          | retain locally (scratch class)                                  |
| `quota-model-review-20261031`                            | durable finding (fleet operations)  | 2     | `e810356e4180`          | zettel via note_submission (Tier 1)                             |
| `student-promos-2026-scan-20261022-2111`                 | research synthesis                  | 2     | `128c8cbfc6c0`          | zettel via note_submission (Tier 1)                             |
| `unconfigured-jewels-20261029`                           | durable finding                     | 1     | `9918e1a1c747`          | zettel via note_submission (Tier 1)                             |
| `workhorse-aa-review-20261031`                           | durable finding (fleet operations)  | 1     | `f4061f40ada9`          | zettel via note_submission (Tier 1)                             |

Non-candidates, explicitly: `codex-review-value-20261029` and `openclaw-vs-opencode-20261029` (raw data, no distilled report), `quota-burn-plan-20261029` (plan-iteration working state), `google-ai-pro-drive-gcp-limits-20261025-1857` (superseded by its v2 folder the same day). Repo-specific candidates: `go-fallthrough-20261029`, `merge-queue-stall-20261029` (about developing-today/code itself → repo doc, Tier 2). All other folders are zettel candidates (durable findings / research syntheses, Tier 1).

## Goal workspaces — left as-is (policy §6)

All 53 workspaces under `~/workspace/goals/` are inventoried in `manifest.json` with file counts. Treatment per §6: deliverables in `files/` stay user-facing records, state in `hidden_files/` stays machine state, and knowledge produced inside a goal follows §4 promotion on next touch. The largest, `keep-the-four-repos-healthy` (91,918 files at inventory time), is the active fleet-operations workspace, not a completed-goal remainder. No goal workspace is a bulk-migration candidate.

## Memory boundary

`~/memory/` (daily logs, topics, people/group pages) is a placement home in the policy table, not legacy sprawl: Tier 3, provenance only, never published to the ID graph. It was not inventoried in this pass; this section is the pointer that makes the boundary explicit, per the issue scope.
