# Main reference template

Use at S0 (`request-intake`): `init` writes a package's first `main.md` in this shape, and from then on it changes only through PRs ([collaboration](../collaboration.md)). It is the document everything links to: the restated request, the file list, the reading of garbled input, one section per concept with its hierarchical items, the process directives, and the issues and PRs grouped by concept. Only `render` writes inside the view markers; `item add` and `item set-status` write the item lines.

```markdown
# <Request title>

<Introduction: one paragraph restating what was asked and why, in plain words. The exact words are in the transcript; anything that interprets them starts with "Inference:".>

- Transcript: [transcript.txt](transcript.txt) (sha256 `<sha256>`, <bytes> bytes, <visibility>; source: <source>)
- Meta design: [design.md](design.md) (review order, waves, run status, research stages)
- Change log: [changes.md](changes.md)
- Human review: [human-review.md](human-review.md) (from S6)
- Workflow: [stages](/planning/workflow/stages.md), [records](/planning/workflow/records.md), [collaboration](/planning/workflow/collaboration.md)

## Files

Every file this request creates or edits, one line per concept.

- [C001 <title>](concepts/C001-<slug>.md) — `new` `<target path>`: <what it covers, one line>.
- [C002 <title>](concepts/C002-<slug>.md) — `edit` `<target path>`: <what it covers, one line>.

## Reading of garbled input

Tokens in the transcript that look mistyped or mistranscribed, and the reading adopted. The transcript stays exact; every reading here is an inference.

- `<token>` (`T014`) — Inference: read as "<reading>" because <reason>. Confidence: <high, medium or low>.

## C001: <title>

<!-- workflow:view concept-C001 -->

- **State:** `proposed` · stage S0 · priority `<priority>` · horizon `<horizon>`
- **Goal:** <end goal this concept serves>
- **Items:** none yet
- **Relations:** none
- **Issues and PRs:** none yet

<!-- /workflow:view concept-C001 -->

<Restated purpose and preliminary thoughts. Mark anything beyond the transcript "Inference:".>

- **C001.1** `created` · S0 · <UTC> — <a statement from the transcript, restated>.
  - **C001.1.1** `created` · S0 · <UTC> — <a detail of C001.1, directly below it>.
- **C001.2** `clarified` · S2 · <UTC> — The situation is more nuanced than first read; see C001.2.1. ← `created` · S0 · <UTC> — <original statement>
  - **C001.2.1** `created` · S2 · <UTC> — <the nuance, directly below the item it refines>

## G: process directives

Segments in the global bucket `G`: instructions about how to work, not what to build. They bind every stage and are not asked again.

- `<T061-T063>` — <the directive, restated>. Applies to: <stages, or "all">.

## Issues and PRs

<!-- workflow:view issues-and-prs -->

- [C001](#c001) <title>
  - Issue [#<n>](https://github.com/<issue_repository>/issues/<n>) · S1 · `open` — <title> (for C001.2); <reason>
- [C002](#c002) <title>
  - PR [#<n>](https://github.com/<issue_repository>/pull/<n>) · S5 · `merged` — <title>; <reason>

<!-- /workflow:view issues-and-prs -->
```

- Concept sections follow the order in `design.md`. Write prose above the item list; never inside a view.
- Items: new status in front, the earlier trail after `←`, nothing deleted. A refinement goes directly below the item it refines, with the next child ID. Statuses: `created`, `clarified`, `confirmed`, `superseded`, `retracted` ([records](../records.md)).
- `render` lists each issue or PR once in its concept's view and again under "Issues and PRs". The grouped view is flat and concept-linked. An issue reference has this shape: Issue [#<n>](https://github.com/<issue_repository>/issues/<n>) · S<n> · `state` — title (for C001.2); reason. A PR uses the same shape with `PR` and a pull URL. With no references, the generated line is `No issues or PRs are recorded yet. Each one will be listed here under its concept.`
