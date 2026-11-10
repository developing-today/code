# Concept file template

`init` writes one of these per concept at S0, as `concepts/C001-<slug>.md`. From then on only the concept's current owner writes it: S1 adds the repository map and research questions, S2 the research, S3 the outcome of each incoming note, and every stage appends to `history` in the state block and to the change log at the bottom. Anyone else who wants a change here sends a note (`note`) instead.

````markdown
# C001: <title>

```workflow-state
{
  "disposition": "proposed",
  "enables": [],
  "goal": "<end goal this concept serves>",
  "history": [
    {
      "at": "<UTC>",
      "note": "Created at intake.",
      "ref": "transcript.txt",
      "stage": "S0",
      "status": "created"
    }
  ],
  "horizon": "<now, next, later, ideas or unspecified>",
  "id": "C001",
  "issues": [],
  "kind": "<new or edit>",
  "priority": "<crucial, recommended, contingent or unspecified>",
  "prs": [],
  "relations": [],
  "segments": [
    "T007-T009",
    "T031"
  ],
  "slug": "<slug>",
  "stage": "S0",
  "status": "created",
  "targets": [
    "<repository path this concept creates or edits>"
  ],
  "title": "<title>",
  "type": "<topic, hint, constraint or directive>"
}
```

[Main reference section](../main.md#c001-<title-anchor>) · [Transcript](../transcript.txt) · [Meta design](../design.md)

## Transcript excerpts

The exact text of every segment assigned to this concept.

[T007-T009 in transcript.txt](../transcript.txt)

```workflow-excerpt T007-T009
<exact text of T007 to T009>
```

## Purpose

Inference: <the concept restated as a goal, in plain words>.

## Initial thoughts

- <preliminary thought>

## Hinted or missed ideas

- Inference: <an idea the transcript only hints at, or that seems missing, kept for consideration>.

## Current repository state

S1: what exists today, each with a path or link (code, docs, designs, plans, earlier requests).

## Research questions

S1: what S2 must answer, most consequential first.

1. <question>

## Research

S2: the definitive record for this concept; no length limit. Feasibility, prior art and example projects, use cases, user anecdotes, alternatives (each steelmanned), risks with mitigations and fallbacks.

## Sources

- <title>, <publisher>, <date or undated> — <exact URL the tool returned> — <what it supports>

## Notes

S3: every note sent and received, with its receipt.

- `notes/C002__to__C001__<id>.md` — `<disposition>`: <where it landed or why not>.

## Change log

- <UTC> · S0 — Created by `init`.
````

- The state block is JSON with 2-space indent, sorted keys and a trailing newline, in a `workflow-state` fence so prettier leaves it alone. Only the owner edits it; `history` is append-only, newest last.
- `relations` entries: `{"kind": "requires", "reason": "<why it blocks>", "to": "C002"}`. `recommended-after` adds `"rework_if_early"`, `optional-if` adds `"trigger"`. Kinds: `requires`, `recommended-after`, `shares-files-with`, `conflicts-with`, `enables`, `optional-if`, `supersedes`, `contradicts`, `part-of` ([ordering](../ordering.md)).
- `issues` and `prs` entries: `{"number": <n>, "stage": "S1", "state": "open", "url": "https://github.com/<issue_repository>/issues/<n>"}`.
- A `hint` concept is never ready for research until a note or a human answer changes its `type`. A `deferred` or `rejected` disposition records its reason, and for `deferred` the revisit trigger, in the history entry's `note`.
- The excerpt fence must be longer than any backtick run inside the excerpt.
