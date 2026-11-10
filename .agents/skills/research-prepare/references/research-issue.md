# Research trigger issue

Read this in step 9. One open issue starts the full research (S2) for one concept. An agent with no other context must be able to take its URL and do the work. The issue templates are in [templates/](../../../../planning/workflow/templates/issue-parent.md). If a dedicated research template exists there, start from it. Either way, the body must have every section below.

## Find before you create (idempotent)

Retries must never create a second issue. Search by the title's package id, then match the body marker:

```sh
REPO=<issue_repository>; PID=${PKG##*/}
gh issue list --repo "$REPO" --state all --limit 200 --search "$PID in:title,body" \
  --json number,url,state,body \
  --jq ".[] | select(.body | test(\"workflow:issue\")) | select(.body | contains(\"$PID\")) | select(.body | test(\"\\\"concept\\\": ?\\\"$C\\\"\")) | select(.body | test(\"\\\"stage\\\": ?\\\"S1\\\"\")) | {number,url,state}"
```

- One open match: reuse it. Edit its body only if yours adds something, using `gh issue edit <n> --body-file`.
- One closed match: reopen it only if this is a deliberate redo, and say why in a comment.
- No match: create it. Use existing labels only; `gh label list --repo "$REPO"` shows them.

```sh
gh issue create --repo "$REPO" --title "Research: <concept title> ($PID·$C)" --body-file "$D/issue-$C.md"
```

Then repeat the search, which must return exactly one issue.

## Body

```
## Goal
Full research for <C> <title>: the definitive concept document, covering every facet below.

## Links
- Package main reference: <repo URL>/blob/main/<PKG>/main.md#<anchor>
- Concept file (owned by the S2 worker): <repo URL>/blob/main/<PKG>/concepts/<file>
- Transcript segments: <T ids>, exact text at the top of the concept file
- S1 preparation PR: <PR URL>

## Current state (evidence)
<3-6 lines: what exists, from the repo map, with path:line; overlapping issues/PRs>

## Research questions and facets
<copy the question ids and one-line facet list; the full text is in the concept file>

## Owned outputs
The concept file (sole writer); notes to other concepts via `rw note`; receipts for notes sent to <C>.

## Acceptance evidence
Every question answered or marked open with a reason; every source with its URL; `rw check --package <PKG>` passes; PR open into main.

## Prerequisites
Blocked by: <requires ids/issues, or none>. Parallel-safe with: <concepts with no shared targets>.

## First step
Load `.agents/skills/concept-research/SKILL.md` and follow it, starting from the concept file's Research questions.

## Done contract
Outcome: <the definitive document>. Verified by: <the checks above>. Boundaries: no design or plan; other concepts change only by notes. Stop as blocked if a prerequisite is unmerged or a needed file has another owner; push and say what blocks.

<!-- workflow:issue {"package":"<PID>","concept":"<C>","stage":"S1"} -->
```
