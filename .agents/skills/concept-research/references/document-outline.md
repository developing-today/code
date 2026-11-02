# Concept document outline (S2)

The concept file is the definitive document for one concept. Keep everything S0 and S1 wrote at the top, unchanged: the title, the `workflow-state` block, the links to `main.md` and the transcript, the transcript excerpts, the normalized purpose, the initial thoughts, the inferred suggestions, the S1 repository map and the S1 research questions. S2 adds the sections below, in this order, after that material.

Headings are fixed so that later stages, `check` and readers find them. A section that does not apply stays, with one line saying why. There is no length limit. Later stages and later S2 runs add to these sections; they never delete from them.

## Writing rules for every section

- Every factual sentence carries `[n]`, `[unsourced]` or starts with "Inference:" (`citations.md`).
- Prefer `-` bullets, `**bold**`, backticks and links. Avoid tables: a one-cell edit reflows the whole table and makes merges hard.
- Refinements go directly below what they refine and carry the item id and status trail used in `main.md`:

  ```markdown
  - **C001.2** `clarified` · S2 · 2026-10-21T10:00:00Z — More nuanced than S0 thought; see C001.2.1. ← `created` · S0 · 2026-10-19T06:09:02Z
    - **C001.2.1** `created` · S2 · 2026-10-21T10:00:00Z — The nuance, with its evidence [4].
  ```

- Repository facts give `path:line` and the SHA they were read at. State that SHA once at the top of the S2 material ("Repository facts verified at `<sha>`, `<UTC>`").

## Sections

### `## Request restated`

- What the user asked for in this concept, in clean words, with the segment ids it comes from (`T012-T014`).
- Ideas that were only hinted at, implied or garbled, each labelled "Inference:", with the reading taken from the garbled-input register in `main.md`.
- Assumptions made, each with the default used.
- What is out of scope for this concept and which concept holds it instead.

### `## Summary of findings`

Five to ten bullets with the findings a designer must not miss, each cited. Point to the facet or section that holds the detail.

### `## Facets`

One `### C001.N <facet title>` sub-section per facet, using the tool-allocated item id. Each holds:

- **Question:** the facet's research questions, with their S1 ids where they exist.
- **In this repository now:** what exists, with `path:line`, the command run and its output. Say "nothing found" with the searches run when nothing exists.
- **Findings:** what the sources say, cited.
- **How others do it:** at least two projects, tools or harnesses when two exist, or a line saying why only one applies.
- **Pitfalls:** failure modes, with evidence.
- **Open questions:** what this facet still does not know (also listed in "Open questions").
- **Verdict:** one or two sentences and a confidence (`high`, `medium`, `low`) with what drives it.

### `## Feasibility`

- Technical feasibility: what exists, what is missing, and the constraints of this repository (for example stdlib-only Python for workflow tooling, the Nix flake, formatters in `treefmt.toml`).
- Effort: a size (`S`, `M`, `L`, `XL`) and what drives it.
- Dependencies on other concepts, each as a relation kind (`requires`, `recommended-after`, `shares-files-with`, `conflicts-with`, `enables`, `optional-if`) with its reason. The same relations go into the state block.
- Verdict: `feasible`, `feasible with conditions` (list them) or `not feasible` (say what would change that).

### `## State of the art`

Current best practice and the most advanced techniques in use, each dated, because these move fast. Name the version or date each source describes.

### `## Prior art and example projects`

One bullet per project or system: name and link `[n]`, what it does like this concept, what it does differently, and the lesson for us. Include projects in this repository and in its other packages.

### `## Use cases and anecdotes`

- Use cases the concept must serve: stated ones with segment ids, inferred ones labelled.
- Anecdotes, presented as anecdotes ("one user reports [12]"), with the source type `anecdote` or `forum` in Sources. Never generalise from one report; state how many reports you found and where you looked.

### `## Alternatives`

At least two realistic alternatives, one of which is the smallest thing that could work (or doing nothing). For each:

- what it is;
- **For:** the strongest honest case for it (steelman), cited;
- **Against:** the strongest case against it, cited;
- **Wins when:** the conditions under which it is the best choice;
- **Switching cost later:** what it costs to move off it.

Do not pick here. The pick is a preliminary recommendation, and the decision record comes at S6.

### `## Risks`

One bullet per risk: what can go wrong, likelihood, impact, the early signal, the mitigation, the fallback if it happens anyway, and the evidence.

### `## Open questions`

One bullet per question, ranked by how much the answer changes what we do:

- the question and why it matters (what changes with each answer);
- the default assumed until it is answered;
- who or what can answer it: the human, more research, a prototype;
- what it blocks.

Never list a question the transcript already answers; cite the segment instead.

### `## Preliminary recommendations`

One bullet per recommendation, each marked `agent-preliminary`:

- the recommendation and the alternative it prefers;
- the rationale, cited;
- the confidence and the evidence that would change it;
- the reversal cost and the items it affects.

These are not decisions. Only the decision protocol in [decisions.md](../../../../planning/workflow/decisions.md) records decisions, and nothing downstream may promote an `agent-preliminary` item to confirmed.

### `## Cross-concept notes`

- **Sent:** link to each note file, the receiver, and a one-line subject.
- **Received:** link to each note, the sender, and its receipt with the disposition, or "pending S3".
- **Related concepts:** each related concept with the relation kind and a one-line reason.

### `## Unsourced ideas`

Every idea no source supports: ideas from the transcript that no source confirms, and recommendations that rest on `[unsourced]` claims. Number them. Each is marked `[unsourced]` and carries these labels:

- what it is;
- `For: Inference:` the strongest reason it may be right;
- `Against: Inference:` the strongest reason it may be wrong, cost too much or not apply;
- `Apply/test:` how to apply it, or how to test it cheaply;
- `Nearest:` the nearest source, or "none".

An idea is never dropped because it has no source.

### `## Sources`

Numbered by first use: `1. <Title>. <Author or publisher>, <date or "undated">. <URL>. <type>`. Repository sources give the path and SHA with type `repo`. Rules are in `citations.md`.

### `## Research log`

- Date, session and agent name for each run.
- Subagents launched, each with its scope and the evidence file it returned.
- Queries run, per facet; pages fetched; sources rejected and why.
- Retrieval fallbacks used (the repository's web-retrieval fallback protocol in `AGENTS.md`), and what failed.
- What was searched for and not found.

### `## Change log`

Add-only, oldest first, one line per major change: `- <UTC> · S2 · <agent> — <what changed and why>`. Minor fixes need no line. Each major change also gets one line in the package `changes.md`.
