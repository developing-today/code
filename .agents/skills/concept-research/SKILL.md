---
name: concept-research
description: Conduct deep factual research for a single concept, steelmanning alternatives, documenting use cases, and analyzing risks. Use during Stage 2 concept investigation.
---

# Concept Research (Stage 2)

Execute deep factual research and architectural analysis for a single concept. Full contracts live in [/planning/workflow/stages.md](/planning/workflow/stages.md) and [/planning/DEPENDENCIES.md](/planning/DEPENDENCIES.md).

## Procedure

1. **Verify Ownership**:
   Confirm lease and own branch/worktree. Read the concept document and prepare angles.

2. **Conduct Factual Research**:
   Investigate official specs, source repositories, practitioner experiences, and local codebase patterns. Cite only URLs returned by tools. Label inference and unconfirmed statements explicitly.

3. **Steelman Alternatives**:
   Document at least two realistic architectural alternatives, providing strong cases for and against each.

4. **Risk & Failure Mode Analysis**:
   Construct a risk matrix covering likelihood, severity, preventive mitigation, and contingent fallback.

5. **Cross-Concept Communication**:
   If research uncovers dependencies or impacts on another concept, write an immutable note:

   ```sh
   python3 scripts/research_workflow.py note --package <dir> --from CXXX --to CYYY --input /tmp/note.md
   ```

6. **Update Concept Record**:
   Update `concepts/CXXX-<slug>.md` with detailed findings, cite all sources, update the concept state block, and commit.

## Turn Report

End every turn with:

- Done: <research findings summary>
- Verified: <citations and check command output>
- Remaining: <handoff to notes-reconcile>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
