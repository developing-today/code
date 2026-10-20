---
name: research-prepare
description: Map codebase, doc, and architecture context for an individual concept and draft research questions and trigger issues. Use before launching deep factual concept research.
---

# Research Prepare (Stage 1)

Prepare the contextual foundation and exploration brief for a single concept before launching deep research. Detailed rules live in [/planning/workflow/stages.md](/planning/workflow/stages.md).

## Procedure

1. **Verify Concept Assignment**:
   Read the concept document in `concepts/CXXX-<slug>.md` and its associated transcript excerpts.

2. **Explore Codebase & Prior Art**:
   Explore related modules under `pkgs/`, existing documentation under `doc/`, and historical architecture decisions to position the concept.

3. **Formulate Research Questions**:
   Draft specific, checkable research questions covering feasibility, prior art, failure modes, and integration seams. Update the concept file under `## Research Angles`.

4. **Draft Trigger Issue**:
   Use template [/planning/workflow/templates/issue-parent.md](/planning/workflow/templates/issue-parent.md) to draft the research trigger issue.

5. **Commit & Report**:
   Commit changes to the concept file on the worker branch and report readiness for deep research.

## Turn Report

End every turn with:

- Done: <concept prepare summary>
- Verified: <codebase search evidence>
- Remaining: <handoff to concept-research>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
