---
name: notes-reconcile
description: Process cross-agent directed notes, record atomic resolution receipts, and update concept documents. Use during Stage 3 reconciliation before handoff.
---

# Notes Reconcile (Stage 3)

Incorporate, acknowledge, or decline all directed notes received by a concept, recording immutable receipts. Full rules live in [/planning/workflow/stages.md](/planning/workflow/stages.md) and [/planning/workflow/records.md](/planning/workflow/records.md).

## Procedure

1. **Discover Unresolved Notes**:
   Scan local and remote incoming notes:

   ```sh
   python3 scripts/research_workflow.py check --package <dir>
   ```

2. **Evaluate & Incorporate**:
   For each note addressed to your concept in `notes/`:
   - Evaluate the notification or change request against your concept findings.
   - If accepted, update the concept document with the change.
   - If declined or deferred, document the technical rationale.

3. **Record Resolution Receipt**:

   ```sh
   python3 scripts/research_workflow.py resolve-note --package <dir> --note <path> --disposition <incorporated|acknowledged|declined|superseded> --target <target-anchor> --reason "<rationale>"
   ```

4. **Verify Full Reconciliation**:
   ```sh
   python3 scripts/research_workflow.py check --package <dir> --require-resolved
   ```

## Turn Report

End every turn with:

- Done: <notes processed and dispositions recorded>
- Verified: <check --require-resolved output>
- Remaining: <handoff to issue-handoff>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
