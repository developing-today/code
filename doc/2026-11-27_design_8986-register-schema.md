# Design register schema and exception ledger — #8986 Slice 1

- **Date:** 2026-11-27
- **Status:** Implementation Slice 1; read-only baseline, no mutations
- **Tracking home:** [developing-today/code#8986](https://github.com/developing-today/code/issues/8986)
- **Design record:** [doc/2026-11-25_design_8986-design-register.md](2026-11-25_design_8986-design-register.md) (Sections 4–5; Slice 1 acceptance)
- **Queue task:** repo-health-code-pilot 25784 · lane oc-code-06
- **Lineage:** [PR #8980](https://github.com/developing-today/code/pull/8980), `pm-stack/lineage-manifest-2026-11-12.md`

This document defines the register schema and the exception ledger without
bulk issue mutations or reparenting. Later audit/monitor slices consume this
format; they do not replace it.

## 1. Schema

One row per register member. All fields are read from the live issue and the
committed tree; nothing here authorizes a write.

| Field            | Meaning                                                                                                                                                                                                           |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tracking_issue` | Member issue number (link). The durable tracking home.                                                                                                                                                            |
| `state`          | OPEN / CLOSED at ledger time.                                                                                                                                                                                     |
| `canonical_doc`  | Committed `doc/` or `pm-stack/` design record linked from the issue body and verified present on `origin/main`.                                                                                                   |
| `doc_gap`        | `true` when no member-specific canonical doc was found in the body (register-entry reference only). The monitor must treat these as doc-findings, not as clean.                                                   |
| `labels`         | `design` + `epic/design-register` + area epic(s) as observed.                                                                                                                                                     |
| `native_parent`  | `#8986`, or the cross-epic parent for an exception row.                                                                                                                                                           |
| `exception`      | Empty for ordinary members; `cross-epic` + reason for #8833.                                                                                                                                                      |
| `board_eligible` | `true` when the issue lives in a repository that can be placed on Project #4. All current members are in `developing-today/code` (org repo), so all are eligible; no personal-repository exemption applies today. |
| `board_expected` | `true` when the issue carries the `design` label, which is the selector for Project #4's `Design Register` view. Members without it are mirror gaps to verify, not exemptions.                                    |

### Exception format (one-parent rule)

A cross-epic member records: native parent (the existing one-parent relation),
the second epic whose register also lists it, and the reason. #8833 is the
known exception: natively parented under #8988 (`epic/storage`), listed on
#8986 for register completeness. The exception is recorded on the #8986 body
("Linking exceptions") and in the ledger below; neither parent is overwritten.

## 2. Exception ledger

### 2.1 Ordinary members natively parented under #8986 (23)

| Issue  | State  | Canonical doc (verified on origin/main)            | Labels (design / register / area)                                                | Board                    |
| ------ | ------ | -------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------ |
| #1817  | OPEN   | doc-gap — no doc link in body                      | design · epic/design-register · epic/lab-cluster                                 | eligible, expected       |
| #5983  | CLOSED | doc-gap — no doc link in body                      | design · epic/design-register · design/docs-migration · epic/pm-stack            | eligible, expected       |
| #8806  | CLOSED | doc-gap — register entry only                      | design · epic/design-register · epic/fleet-orchestration                         | eligible, expected       |
| #8807  | OPEN   | doc-gap — register entry only                      | design · epic/design-register · epic/turso-cloudflare                            | eligible, expected       |
| #8808  | OPEN   | doc-gap — register entry only                      | design · epic/design-register · epic/turso-cloudflare                            | eligible, expected       |
| #8809  | OPEN   | doc-gap — register entry only                      | design · epic/design-register · epic/mcpx · epic/mcpx-platform                   | eligible, expected       |
| #8810  | OPEN   | doc-gap — no doc link in body                      | design · epic/design-register · epic/mcpx-platform                               | eligible, expected       |
| #8811  | CLOSED | doc-gap — register entry only                      | design · epic/design-register · epic/storage                                     | eligible, expected       |
| #8820  | OPEN   | doc/living/cloudflare-credential-architecture.md   | design · epic/design-register · epic/turso-cloudflare                            | eligible, expected       |
| #8821  | OPEN   | doc/living/ai-gateway-opencode-gateway.md          | design · epic/design-register · epic/turso-cloudflare                            | eligible, expected       |
| #8829  | OPEN   | pm-stack/project-tracking-evaluation-2026-10-23.md | design · epic/design-register · epic/pm-stack                                    | eligible, expected       |
| #8830  | OPEN   | pm-stack/needs-user-actions-design-2026-10-26.md   | design · epic/design-register · epic/pm-stack                                    | eligible, expected       |
| #8832  | OPEN   | doc/living/turso-cloudflare-idea-engine.md         | design · epic/design-register · epic/turso-cloudflare                            | eligible, expected       |
| #8835  | OPEN   | doc/living/globalping-outside-in-watch.md          | design · epic/design-register · epic/fleet-orchestration                         | eligible, expected       |
| #8836  | OPEN   | doc/living/account-connection-campaign.md          | design · epic/design-register · epic/fleet-orchestration                         | eligible, expected       |
| #8837  | OPEN   | doc/living/late-night-shift-silence-rule.md        | design · epic/design-register · epic/fleet-orchestration                         | eligible, expected       |
| #8864  | CLOSED | doc-gap — no doc link in body                      | design · epic/design-register · epic/pm-stack                                    | eligible, expected       |
| #10389 | OPEN   | doc-gap — no doc link in body                      | epic/design-register · primary-parent/design-register · stray-work (no `design`) | eligible, **mirror gap** |
| #10390 | OPEN   | doc-gap — no doc link in body                      | epic/design-register · primary-parent/design-register · stray-work (no `design`) | eligible, **mirror gap** |
| #10392 | OPEN   | doc-gap — no doc link in body                      | epic/design-register · primary-parent/design-register · stray-work (no `design`) | eligible, **mirror gap** |
| #10393 | OPEN   | doc-gap — no doc link in body                      | epic/design-register · primary-parent/design-register · stray-work (no `design`) | eligible, **mirror gap** |
| #12950 | OPEN   | doc-gap — register entry only                      | epic · epic/design-register · epic/fleet-orchestration (no `design`)             | eligible, **mirror gap** |
| #12951 | OPEN   | doc-gap — register entry only                      | epic · epic/design-register · epic/pm-stack (no `design`)                        | eligible, **mirror gap** |

### 2.2 Body-listed members not natively linked (9)

Listed on the #8986 body checklist but not native sub-issues at ledger time.
No reparenting is performed in this slice; the monitor (Slice 4) reports them
as parent-link findings through the repair protocol (Slice 2).

| Issue | State  | Canonical doc (verified on origin/main)           | Labels                                                                                      |
| ----- | ------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| #8822 | OPEN   | doc/living/mcpx-dedup-canonical-source.md         | kind/design · design · epic/design-register · epic/mcpx · epic/mcpx-platform                |
| #8823 | OPEN   | doc/living/fleet-sweep-review-job-family.md       | kind/design · design · epic/design-register · epic/fleet-orchestration                      |
| #8824 | OPEN   | doc/living/routine-task-architecture.md           | kind/design · design · epic/design-register · epic/fleet-orchestration                      |
| #8825 | OPEN   | doc/living/quota-full-utilization-burn-plan.md    | kind/design · design · epic/design-register · design/quota-plane · epic/fleet-orchestration |
| #8826 | OPEN   | doc/living/fleet-roles-minder-catcher-advocate.md | kind/design · design · epic/design-register · epic/fleet-orchestration                      |
| #8827 | OPEN   | doc/living/one-queue-model.md                     | kind/design · design · epic/design-register · epic/fleet-orchestration                      |
| #8828 | CLOSED | doc/living/lane-naming-law.md                     | kind/design · design · epic/design-register · epic/fleet-orchestration                      |
| #8831 | OPEN   | pm-stack/decisions-ledger-system.md               | app/pm-stack · kind/design · design · epic/design-register · epic/pm-stack                  |
| #8838 | CLOSED | doc-gap — register entry only                     | kind/feature · design · epic/design-register · epic/fleet-orchestration                     |

### 2.3 Cross-epic exception (1)

| Issue | State | Canonical doc                                                                            | Native parent          | Exception                                                                                                                                                             |
| ----- | ----- | ---------------------------------------------------------------------------------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #8833 | OPEN  | doc/living/cold-store-backup.md (+ doc/living/cold-store-r2-layout-design-2026-10-25.md) | #8988 (`epic/storage`) | cross-epic: also carries `epic/design-register`; listed on #8986 for register completeness per the body note. Recorded here and on #8986; neither parent overwritten. |

## 3. Count reconciliation

- Ledger rows: **33** = 23 native (§2.1) + 9 body-only (§2.2) + 1 cross-epic exception #8833 (§2.3).
- #8986 body checklist: **27** children = 17 native + 9 body-only + #8833 (exception). Reconciles: 17 + 9 + 1 = 27.
- Native sub-issues at ledger time: **23** = 17 from the original checklist + 6 added later (#10389, #10390, #10392, #10393, #12950, #12951, design-mill/fleet family).
- Lineage manifest (`pm-stack/lineage-manifest-2026-11-12.md`): 27 zero-carrier `epic/design-register` issues at creation — matches the 27-item body checklist.
- Doc-gaps: **16** rows (1817, 5983, 8806–8811 except 8806's siblings with docs, 8864, 8838, 10389, 10390, 10392, 10393, 12950, 12951). Canonical docs verified present: **17** rows.
- Mirror gaps (eligible but no `design` label): **6** rows (10389, 10390, 10392, 10393, 12950, 12951). No personal-repository exemptions apply.
- No parent or issue state was mutated to produce this ledger.

## 4. Consumer notes for later slices

- Slice 2 (intake/repair protocol): dry-run against one ordinary member (e.g. #8820) and #8833 for the exception path.
- Slice 3 (audit oracle): fixtures — correctly linked (#8820), missing doc (#1817), missing label / wrong parent (body-only §2.2 rows), documented exception (#8833), not-eligible personal-repo (none today; synthesize), absent/stale snapshot.
- Slice 4 (weekly reconciliation): candidate set = native sub-issues ∪ body checklist ∪ #8833; dedupe findings by issue + invariant; report §2.2 rows as parent-link findings via the Slice 2 repair protocol.

## Change log

- 2026-11-27: Slice 1 — schema + exception ledger from #8986 (lane oc-code-06, task 25784). Read-only; no mutations.
