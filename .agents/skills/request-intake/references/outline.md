# Outline schema

Read this in step 5. `init --outline` reads this JSON. The field meanings are in [records.md](../../../../planning/workflow/records.md), and the vocabulary for `type`, relation kinds, `priority` and `horizon` is in [DEPENDENCIES.md](../../../../planning/DEPENDENCIES.md). If `init` rejects a key, follow its one-line error. The tool's `--help` has the final say.

## Example (illustrative ids)

```json
{
  "title": "Research-to-delivery workflow",
  "introduction": "What the user asked for, in two to five plain sentences.",
  "global": {
    "segments": ["T001", "T060-T066"],
    "summary": "How to work: parallel agents, PRs for wide files, keep everything at early stages."
  },
  "garbled": [
    {
      "segment": "T071",
      "token": "**§**",
      "reading": "features",
      "why": "The sentence lists things to implement, defer or reject.",
      "confidence": "medium",
      "alternatives": ["sections"]
    }
  ],
  "concepts": [
    {
      "title": "Exact transcript storage",
      "slug": "transcript-storage",
      "type": "topic",
      "kind": "new",
      "targets": ["planning/workflow/records.md"],
      "segments": ["T002-T003"],
      "summary": "Store the request byte for byte and link everything to it.",
      "thoughts": ["Speech-to-text input arrives as very long lines; segment by sentence."],
      "suggestions": ["Inference: record where the bytes came from, so a second copy can be told apart."],
      "priority": "crucial",
      "horizon": "now",
      "goal": "No idea from the request is ever lost.",
      "enables": ["main-reference"],
      "relations": [{ "kind": "enables", "to": "main-reference", "reason": "Excerpts link to the transcript." }]
    }
  ]
}
```

## Field rules

- **`segments`** take ids or ranges (`"T010"`, `"T010-T014"`) from `segments.txt`.
  - Every non-blank segment must be in at least one concept or in `global`; `init` fails otherwise.
  - `global` is the reserved bucket `G`: directives and constraints about how to work.
- **`type`** is one of:
  - `topic`;
  - `hint`: the user gestured at it, so it is never ready for research until a note or a human answer changes its type;
  - `constraint`;
  - `directive`.
- **`kind` and `targets`**:
  - `new` concepts list the paths they will create.
  - `edit` concepts list the existing files they will change, which the explorers found.
  - Don't create target files at intake. Stubs live only in `concepts/`.
- **Relations** have the form `{kind, to, reason}`:
  - `to` is a concept `slug`, because the tool allocates `C001` ids and agents never pick ids.
  - `recommended-after` adds `rework_if_early` (what breaks if it runs first).
  - `optional-if` adds `trigger`.
  - Use `contradicts` or `supersedes` for contradictions and changes of mind between concepts.
  - Inside one concept, keep both statements as thoughts that cite their segments, for example "T040 supersedes T012 (change of mind; later wins by default)".
- **Thoughts** become the concept's preliminary items (`C001.1`, `C001.2`, ...). Write one statement each, and cite segments where they apply.
- **Suggestions** start with `Inference:`. They hold ideas the user missed or only implied that the corpus should consider, and they are never presented as the user's words.
- **Priority and horizon** come from the user's words (`crucial`, `recommended`, `contingent`; `now`, `next`, `later`, `ideas`). Otherwise use `unspecified`. A `contingent` concept states its trigger in `relations` (`optional-if`).
- **Titles and slugs** are a single line. Slugs are lowercase kebab case.

## Idea-level check (before step 6)

`init` checks coverage per segment. You check it per idea:

1. List every segment that holds more than one idea. Long speech-to-text sentences usually do.
2. For each idea in such a segment, name the concept, `G` entry or suggestion that claims it.
3. Then read the outline's titles against `reading.md`. Every topic, directive, hint, contradiction and repeat in `reading.md` has a home.

Write the result as a short list at the end of `$WORK/reading.md`. Step 5 is done when no idea lacks a home.
