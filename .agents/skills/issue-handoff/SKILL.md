---
name: issue-handoff
description: Generate self-contained parent, facet child, or orchestration GitHub issues for concepts and stages. Use during Stages 4 and 5 to hand off research into design.
---

# Issue Handoff (Stages 4 & 5)

Publish self-contained GitHub issues with zero-context handoff contracts. Specifications live in [/planning/workflow/stages.md](/planning/workflow/stages.md) and [/planning/workflow/templates/issue-parent.md](/planning/workflow/templates/issue-parent.md).

## Modes

- **`parent` (Stage 4)**: Creates a single parent issue per concept defining overall research goals, scope, and deliverables using `templates/issue-parent.md`.
- **`fanout` (Stage 5)**: Breaks down a parent issue into granular child issues (one per research facet) using `templates/issue-child.md` and populates the initial research stages in `design.md`.
- **`orchestration` (Stage 5)**: Generates the top-level orchestration issue using `templates/issue-orchestration.md` to coordinate parallel subagents across the child issues.

## Procedure

1. **Verify Source Concept**:
   Read the concept document and ensure all notes are resolved (`check --require-resolved`).

2. **Draft Issue Body**:
   Format the issue body using the appropriate template. Ensure context pointers, acceptance criteria, and the trailing machine comment `<!-- workflow:issue ... -->` are present.

3. **Check for Duplicate Issues**:
   Search open repository issues for matching metadata markers before publishing to prevent duplicate creation on retry.

4. **Publish Issue**:

   ```sh
   gh issue create --repo <repo> --title "<title>" --body-file /tmp/issue-body.md
   ```

5. **Update Concept & Design Records**:
   Record the created issue number and URL in the concept state block and update `design.md`.

## Turn Report

End every turn with:

- Done: <issue created with URL and stage>
- Verified: <gh issue view output>
- Remaining: <handoff to design-orchestrate>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
