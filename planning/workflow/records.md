# Workflow records

The files a research package holds, who writes each one, and their exact formats. Values come from [vocabulary.md](vocabulary.md). Who may write what, and how, is ruled by [collaboration.md](collaboration.md). This page describes the target format. Where the tool on `main` writes something else, the difference is listed under [Tool status](#tool-status). The [templates](templates/README.md) show each file filled in.

## Package layout

A package is a folder `doc/<UTC>_research_<slug>/` under the profile's `requests_root`. The tool treats a folder as a package only if its name matches `^\d{4}-\d\d-\d\dT\d\d-\d\d-\d\dZ_research_[a-z0-9]+(-[a-z0-9]+)*$` and it contains `package.json`.

```text
doc/<UTC>_research_<slug>/
  transcript.txt              exact request bytes; never edited
  package.json                manifest; written once at intake
  main.md                     main reference
  design.md                   meta design
  changes.md                  add-only change log
  human-review.md             generated at S6 from decisions/
  concepts/C001-<slug>.md     one per concept
  notes/C001__to__C002__<uuid>.md
  receipts/<note-id>.md
  decisions/D001-<slug>.md
```

| Path                   | Writer                                              | Changes how                                  |
| ---------------------- | --------------------------------------------------- | -------------------------------------------- |
| `transcript.txt`       | intake, once                                        | never                                        |
| `package.json`         | intake, once                                        | never                                        |
| `main.md`, `design.md` | anyone, through a PR                                | authored parts by PR; views only by `render` |
| `changes.md`           | anyone                                              | one appended line, `merge=union`             |
| `human-review.md`      | `render`                                            | regenerated, never edited                    |
| `concepts/C001-*.md`   | the concept's current stage owner                   | edited by its owner only                     |
| `notes/*.md`           | the sender, once                                    | never; a reply is a new note                 |
| `receipts/*.md`        | the receiver, or the reconciler on its behalf, once | never                                        |
| `decisions/D001-*.md`  | an S6 worker, once; later the holder of its lease   | status changes and dated lines only          |

`notes/`, `receipts/` and `decisions/` are created when the first file needs them. A single concept gets the same layout with one concept file. There is no special case.

Files outside the package that the workflow changes (`AGENTS.md`, `doc/README.md`, `planning/plans/README.md`, roadmaps) are wide files: they change only through a PR.

## Transcript

`transcript.txt` holds the request exactly as received: no prefix added by a harness, no trimming, no newline added at the end. `.gitattributes` marks it `-text`, `.editorconfig` turns off whitespace and newline fixes for it, `just strip-whitespace` skips it, and the CI path filter watches it.

`package.json` records the transcript as:

```json
{
  "bytes": 16566,
  "encoding": "utf-8",
  "expected_sha256": "e250cfa2…",
  "path": "transcript.txt",
  "sha256": "e250cfa2…",
  "source": "free text: where the bytes came from",
  "visibility": "public"
}
```

- `sha256` is computed over the stored bytes. `expected_sha256` is optional; when given, intake refuses a mismatch. `verify-transcript --against <file>` compares the stored bytes with another copy.
- `encoding` defaults to `utf-8`. Input that does not decode is refused unless `--encoding` names the encoding, which is then recorded.
- `visibility` is `public` or `private`. This repo is public. Private material is not committed here: store it where it may live (for example the private notebook) and keep only its hash and a pointer, with `visibility: private`.
- `init` refuses a second package with the same `sha256` unless `--allow-duplicate` is given.

## Segments and coverage

Segmentation is deterministic and by sentence. Split after `.`, `?` or `!` when whitespace follows, and at every newline. A segment starts at its first non-whitespace code point and ends after its last one. Whitespace between segments belongs to none, and whitespace-only pieces get no ID. IDs are `T001`, `T002`, ... in order.

`package.json` lists them as `"segments": [{"end": 120, "id": "T001", "start": 0}, ...]`, with `start` and `end` as code-point offsets into the decoded text (end exclusive). `init --preview-segments` prints them so the intake agent can write the outline.

Coverage: every segment is assigned to at least one concept (the concept's `segments`) or to the global bucket `G` (`package.json` `global_segments`). Overlap between concepts is allowed. The intake outline lists, per concept, `title`, `slug`, `type`, `segments`, `summary`, `thoughts`, `suggestions` (each labelled inference), `priority`, `horizon`, `goal`, `enables` and `relations`. A `hint` concept may cite no segment, but its summary must say what it was inferred from.

Titles and slugs reject control characters and newlines.

## Concept file

`concepts/C001-<slug>.md` starts with the title, then the state block directly after it:

````markdown
# C001: Exact transcript storage

```workflow-state
{
  "disposition": "proposed",
  "enables": ["C004"],
  "goal": "No request is ever lost or altered",
  "history": [
    {
      "at": "2026-10-19T06:09:02Z",
      "note": "Created at intake.",
      "ref": "transcript.txt",
      "stage": "S0",
      "status": "created"
    }
  ],
  "horizon": "now",
  "id": "C001",
  "issues": [],
  "kind": "new",
  "priority": "crucial",
  "prs": [],
  "relations": [],
  "segments": ["T001-T004", "T010"],
  "slug": "exact-transcript-storage",
  "stage": "S0",
  "status": "created",
  "targets": ["scripts/research_workflow.py"],
  "title": "Exact transcript storage",
  "type": "topic"
}
```
````

- The block is JSON with 2-space indentation, sorted keys and a final newline. Its fence info string is `workflow-state`, which prettier leaves alone. Never wrap state in an HTML comment.
- `stage` is the latest stage that has started on the concept. The worker of a stage sets it in its first commit and appends a history entry.
- `issues` and `prs` entries are `{"number", "url", "stage", "state"}`, plus optional `title`, `item` (the item the issue or PR is for) and `reason` (why it was implemented, declined or changed). `state` is the last recorded GitHub state (`open`, `closed`, `merged`); `status --live` reads the real one.
- `relations` entries are `{"kind", "to", "reason"}`, plus `rework_if_early` for `recommended-after`, `trigger` for `optional-if`, and an optional `stage` for `requires` ([DEPENDENCIES.md](../DEPENDENCIES.md#1-dependency-relations)). `to` is a concept or item ID in this package, or a repo path or issue URL outside it.
- `history` is append-only, newest last. Each entry has `at`, `stage`, `status`, `note` (why) and `ref` (commit, PR, issue or path), plus `disposition` when the disposition changes and `trigger` when it becomes `deferred`. An entry that sets `deferred` or `rejected` must have a non-empty `note`.
- Only the concept's current owner writes the file. The owner may keep supporting material in a sibling folder `concepts/C001-<slug>/`, under the same rule.

The rest of the file is authored Markdown. The sections each stage must fill are listed in [stages.md](stages.md).

## Excerpts

An excerpt quotes transcript segments exactly. It is a link line, a blank line, then a fenced block whose info string is `workflow-excerpt` and the segment range:

````markdown
[T007-T009](../transcript.txt#L12-L14)

```workflow-excerpt T007-T009
The exact text from the start of T007 to the end of T009.
```
````

- The block holds the decoded text from the start of the first segment to the end of the last, with one normalisation: spaces and tabs at the end of each line are removed and `\r\n` becomes `\n`. The repo's whitespace stripper would remove them anyway; the transcript itself keeps them.
- The fence is exactly one backtick longer than the longest backtick run inside the text, and at least three. Prettier rewrites any other length.
- Line anchors count `\n` only, as GitHub and editors do.
- `check` recomputes each excerpt from `package.json` and the transcript, and fails if it differs.

## Generated views

`main.md`, `design.md` and `human-review.md` contain generated sections. Each sits between two markers, with a blank line inside each:

```markdown
<!-- workflow:view run-status -->

- **C001** [Exact transcript storage](main.md#c001): S0 `validated` · S1 `running` · S2–S8 `not-started`

<!-- /workflow:view run-status -->
```

- `render --package <dir>` rewrites everything between the markers, and inserts a missing view under its heading. Never edit a view by hand. If two PRs conflict inside a view, take either side and run `render` again.
- Once a package has views, `check` fails when one is stale, missing, or has a name `render` does not know.
- Generated Markdown uses only headings, `-` bullets, `**bold**`, backticks, links and blank lines. No tables, no `*emphasis*`. Output must pass `prettier --check` unchanged.

Views `render` writes:

- `main.md`, `concept-C001`: one per concept, in its section. State (status, stage, disposition, priority, horizon), goal, item counts by status, relations in and out, and its issues and PRs with stage and state.
- `main.md`, `issues-and-prs`: every issue and PR in the package, grouped under the concept it belongs to, naming the item when there is one.
- `design.md`, `waves`: the output of `ready` (done, running, wave 1, wave 2, ...), with each `recommended-after` cost shown under the wave that pays it.
- `design.md`, `run-status`: the run status of each concept at each stage.

Views planned but not rendered yet (do not add their markers until `render` writes them, because `check` rejects unknown names): `files` and `global` in `main.md`, `review-order` and `research-stages` in `design.md`, and `human-review` in `human-review.md`.

## Main reference

`main.md` holds, in order: the title and an introduction; the list of package files and of every repo file the concepts create or edit; links to the transcript, `design.md` and `changes.md`; the section "Reading of garbled input"; the segments in `G`; one section per concept; the section "Issues and PRs" with its view.

"Reading of garbled input" lists each suspicious token with its segment, the reading adopted and why, labelled as inference. Example: `- § (T145): read as "features". Inference: the sentence lists things to implement.` The transcript stays exact; readings live only here and in concept summaries.

Each concept section is a heading `## C001: <title>` (with an `<a id="c001"></a>` anchor line before it), the `concept-C001` view, authored preliminary thoughts, and the items list.

## Items and status trails

Items are hierarchical statements under a concept section in `main.md`. Each is one list line; a child sits directly below its parent, indented by two spaces.

```markdown
- **C001.2** `clarified` · S2 · 2026-10-21T10:00:00Z — More nuanced than first thought; see C001.2.1. ← `created` · S0 · 2026-10-19T06:09:02Z — Agents write cross-file changes as to/from note files.
  - **C001.2.1** `created` · S2 · 2026-10-21T10:00:00Z — The nuance, written directly below the item it refines.
```

Grammar of one line:

```text
line  = indent "- **" id "** " entry *( " ← " entry )
entry = "`" status "` · " stage " · " timestamp [ " — " text ]
```

- The newest entry is first, and timestamps never increase from left to right. The newest entry must have text. The oldest entry keeps the original text, which is never deleted or edited.
- `status` is a statement status and `stage` is `S0` to `S8`.
- `item add --package <dir> --parent <C001 or C001.2> --stage S2 --text <text>` allocates the next free ID under the parent and inserts the line: a first top-level item goes directly below the concept view, any other item goes after its last sibling's subtree. `--status` (default `created`) and `--at` are optional.
- `item set-status --package <dir> --id C001.2 --status clarified --stage S2 --note <text> [--see C001.2.1]` prepends an entry whose text is the note followed by `see <ids>`. `superseded` requires `--see`. An item never goes back to `created`.
- Both commands refresh the concept's view and append a line to `changes.md`.
- `check` validates the grammar, that each parent exists, that IDs are unique, the vocabulary and the timestamp order.

## Change log

`changes.md` starts with `# Changes`, a blank line, a link to `main.md` and a blank line. After that, each line is one change:

```text
- 2026-10-21T10:00:00Z | S2 | clarified | C001.2 created → clarified: More nuanced than first thought; see C001.2.1.
```

The fields are timestamp, stage, the status the change set (a statement status, disposition or run status) and the text, which starts with the IDs it concerns. A line written by hand ends with `, by <writer> (<path, PR or commit>)`. `item add` and `item set-status` write their own lines; add one by hand only for a change no command records.

`.gitattributes` sets `merge=union` for `changes.md`, so two branches that each add a line merge cleanly. Lines are never edited or deleted, because a union merge would keep both versions. Their order follows merges, not time; sort by timestamp to read them in order. GitHub's merge button may not apply the union driver, which is one reason to merge the base locally first ([collaboration.md](collaboration.md#the-rules)).

## Notes

A note asks another concept's owner for a change or tells them something. It is a new file `notes/C001__to__C002__<uuid>.md`, written once by the sender with `note --package <dir> --from C001 --to C002 --input <file>`:

````markdown
# Note C001 to C002

```workflow-note
{
  "at": "2026-10-21T10:00:00Z",
  "body_sha256": "…",
  "from": "C001",
  "id": "C001__to__C002__4f1c…",
  "to": "C002"
}
```

The body: what to change and why, with links.
````

- The body is everything after the blank line that follows the closing fence. `body_sha256` is the hash of its UTF-8 bytes. `note` formats the body with prettier (when present) before hashing, so the pre-commit formatter cannot break the hash.
- A note is immutable. A reply is a new note in the other direction. `note` validates only what it writes, so a problem elsewhere in the package cannot block a sender.
- Merge note-only PRs quickly: they add one new file and cannot conflict.

## Receipts

A receipt records how the receiver handled one note. It is `receipts/<note-id>.md`, created once with `resolve-note --package <dir> --note notes/<note-id>.md --disposition <d> --target <path#anchor> --reason <text>`:

````markdown
# Receipt for C001__to__C002__4f1c…

```workflow-receipt
{
  "at": "2026-10-21T11:00:00Z",
  "by": "c002-reconcile",
  "disposition": "incorporated",
  "note": "notes/C001__to__C002__4f1c….md",
  "reason": "Added the transport comparison the note asked for.",
  "scanned": [
    { "ref": "origin/main", "sha": "cf6c4f90…" },
    { "ref": "origin/wf/research-pipeline-c001-s2", "sha": "9a1b…" }
  ],
  "target": "concepts/C002-transport.md#research"
}
```
````

- `by` is the receiver's agent name, or the reconciler's name followed by `for C002` when it acts on the receiver's behalf.
- `target` is a link inside the package to where the note's content went, or where the answer is.
- `scanned` lists every ref and SHA that `check --remote-notes` looked at before the receipt was written. A concept that received no notes records the same scan in the `note` of its S3 history entry, with the trunk SHA as `ref`. A note dated after the scan is provably late and reopens S3 for its receiver.
- The file is created with `O_EXCL`, so two writers cannot both resolve one note. `resolve-note` accepts only `notes/*.md` files with a valid note block.

## Decisions

`decisions/D001-<slug>.md` holds one decision. Its header is a field list directly after the title, so `git grep` and a simple parser both read it:

```markdown
# D001: Where live leases are stored

- Status: preliminary
- Source: agent-preliminary
- Date: 2026-10-21T12:00:00Z
- Affects: C009, C010.2
- Reversal cost: low — move the directory and update the profile key
- Decided by: s6-reviewer-2 (https://github.com/developing-today/code/pull/530)
```

- `Source: stated` and `Source: human-confirmed` carry their evidence in parentheses: segment IDs (`stated (T051)`) or the URL of the human's message.
- The sections are Question, Options (each with a steelman and the case against), Choice, Rationale and Human review. Status changes, the challenge pass and reversals each add a dated line under Choice; nothing is deleted.
- Field meanings, who may set which value, and the review flow are in [decisions.md](decisions.md).

## Issues

Every issue the workflow creates has a title of the form `<Prefix>: <subject> (<slug>·C001)`. Prefixes: `Research:` (S1), `Handoff:` (S4), `Design:` (S5 facet and orchestration issues), `Plan:` (S6), `Implement:` (S7 step and orchestration issues). Facet issues name the item instead, `(<slug>·C001.3)`.

The body ends with a marker line:

```markdown
<!-- workflow:issue {"concept":"C001","item":"","package":"2026-10-19T06-09-02Z_research_research-intake-to-implementation","stage":"S1"} -->
```

- Before creating an issue, search the repository for its marker so a retry never makes a duplicate.
- Use existing labels only.
- The body is self-contained: goal; links to the package, the concept's section in `main.md`, the concept file and its transcript segments; current state with evidence; owned outputs; acceptance evidence; prerequisites (blocked-by); parallel-safe-with; what to do first; and a done contract (outcome, how it is verified, boundaries, and when to stop and report blocked). The [issue templates](templates/README.md) lay these out.
- Record each issue in the concept's `issues` list with the stage that created it.

## Tool status

What `scripts/research_workflow.py` and `scripts/research_workflow_items.py` on `main` at `cf6c4f90` (2026-10-21) do differently from this page. Work package `tool-live` is building the live commands; check `--help` before relying on one.

- State: all state is one package-level JSON block in `main.md`, wrapped in `<!-- workflow:state -->` comments, with `concepts`, `dependencies`, `issues`, `prs`, `tasks` and `events` arrays. Prettier de-indents it (see the existing package's `main.md`). There is no `package.json` and no per-concept `workflow-state` block; relations are `dependencies` entries `{from, to, kind, reason}`.
- Run status: recorded as `tasks` entries `{concept, stage, status}` in that block; `check` rejects `pushed`, `pr-open` and `merged` there. `render` shows S0 as `validated`.
- Excerpts: line-based. The outline uses `source_lines`, the marker is `<!-- workflow:excerpt C001 1 5 -->` with a `text` fence, and line numbers come from Python `splitlines()` (which also splits at U+2028 and form feeds). There are no `T` segments and no `--preview-segments`; `ids` lists segments only when a `package.json` exists.
- Coverage: by non-blank line, not by segment. There is no `G` bucket, no `G.1` items and no `hint` type; every concept needs source lines.
- Single concept: the concept is written into `main.md` with an inline change log, instead of a concept file and `changes.md`.
- Notes: the marker is `<!-- workflow:note -->`, the body is hashed without a formatter pass, and `note` runs a full package check first.
- Receipts: `resolve-note` appends a `## Note disposition` section to the receiver's concept file (a second writer) and guards with an `O_EXCL` lock file `notes/.<id>.resolved`. `deferred` is accepted with a mandatory `Revisit when:` trigger in the reason (#492), and nothing records `scanned` in a concept-file resolution block.
- Vocabulary: item statuses and stages are validated. Concept `status`, `priority`, `criticality` and `horizon` are free text, and completion uses a synonym set (`done`, `completed`, `researched`, ...). The relation kinds `supersedes`, `contradicts` and `part-of` are rejected.
- Scheduling: `ready` and `status` read the working tree, not `origin/<trunk>`. The `waves` view honours `requires` and `shares-files-with`; there are no leases and no live join.
- IDs: `ids` lists concept, item, segment, note and decision IDs but does not allocate decision IDs.
- Commands missing: `lease acquire|release|list`, `verify-transcript`, `status --live`, `check --changed-since`, `check --remote-notes`, `check --run-validation`, `check --profile-only`, and the `init` flags `--preview-segments`, `--source`, `--encoding` and `--visibility`.
- Encoding: non-UTF-8 input is always refused.
- Discovery: besides the name pattern, any folder whose `main.md` has a state block counts as a package.
- `init` writes its change-log line as `- <UTC> | intake | prepared | <text>`, with a stage name instead of `S0`.
- Decisions and `human-review.md` have no tool support. Skills create issues and search markers with `gh`.
- CI runs `check` over the whole tree on every PR, not only the changed packages.
