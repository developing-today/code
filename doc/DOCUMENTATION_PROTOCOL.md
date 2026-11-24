# Documentation Protocol for Design & Architecture Decisions

When making a significant design or architecture decision—whether modifying an existing component or introducing a new feature that changes how things operate—**document first, then implement**.

## When to Document

- New features or components that affect system behavior
- Architectural changes or refactors
- Design decisions with trade-offs worth recording
- Changes to existing components that alter their interface or semantics

## Initial Documentation

1. **Create a docs folder** for the change:

   ```
   docs/<UTC_RFC_DATETIME>_<kind>_<name>/
   ```

   - `<UTC_RFC_DATETIME>`: e.g., `2026-03-16T14-30-00Z`
   - `<kind>`: `feature`, `architecture`, `refactor`, `design`, `component`, etc.
   - `<name>`: descriptive snake_case name

2. **Create the initial document** with the same naming:

   ```
   docs/2026-03-16T14-30-00Z_feature_blob_streaming/2026-03-16T14-30-00Z_feature_blob_streaming.md
   ```

3. **Document the request/intent and initial plan** before implementing:
   - What was requested or identified as needed
   - Initial design approach
   - Key decisions and their rationale
   - **Verified when:** For every new design record, state the observable result that would satisfy the design and the test, comparison, or evidence that will establish it.

4. **Append updates during rollout** as new sections:
   - Add a page break (`---`) before each appended section
   - Start with a header: `## <UTC_RFC_DATETIME> <type>: <brief description>`
   - Types: `Update`, `Clarification`, `Deviation`, `Discovery`, `Decision`
   - Example: `## 2026-03-17T10-00-00Z Update: Switched to async streaming`
   - Content: modifications, clarifications, edge cases, deviations from plan

## Post-Rollout Updates

After initial rollout is complete:

- **If significantly different or many updates**: Create a new file in the same folder with a new datetime and clarifying suffix:

  ```
  2026-03-18T09-00-00Z_feature_blob_streaming_final_design.md
  ```

- **Returning in a new session with major changes planned**: Create a new datetime document with a suffix explaining the revision type:

  ```
  2026-03-25T11-00-00Z_feature_blob_streaming_v2_proposal.md
  ```

- **Short updates**: Files can be brief notes, updates to specific parts, or complete re-summarization of current/proposed design

## Dated Records and Maintained Control Documents

Documents follow one of two sets of rules. A dated record says what was known and decided at one time. A maintained control document says how things stand now, and other work steers by it. [`planning/DOCUMENTATION_ARCHITECTURE.md`](../planning/DOCUMENTATION_ARCHITECTURE.md) has the placement rules for every kind of document in the repository. [`README.md`](README.md) maps where each kind lives.

### Dated historical records

Dated historical records are:

- the files in `doc/<UTC>_<kind>_<name>/`;
- in a research package (`doc/<UTC>_research_<slug>/`):
  - `transcript.txt`, the exact request;
  - `package.json`, written once at intake;
  - directed notes in `notes/`;
  - receipts in `receipts/`;
  - decision records in `decisions/`.

The rules for them:

- Do not modify files after initial creation, except appending during active rollout (see above). A later finding goes in a new dated section or file that links back.
- `transcript.txt` is never edited, not even its whitespace. Notes are immutable once written. Corrections are new files: a new note, receipt or decision.

### Lane work product

Work is split into lanes, and a lane's work product is a dated record like any other. Reports and one-offs — and everything gathered or generated while producing them — land in this repository as a normal, expected class of record, not as an exception and not only in a summary handed back to whoever asked.

- **One dated folder per piece of work.** `doc/<UTC>_<kind>_<name>/`, named for the issue, topic or lane that produced it. The usual kind is `report`; a more specific kind (`audit`, `research`, `design`) is fine when it fits better.
- **The folder carries everything.** The main report sits in the folder alongside all of its supplements: the data gathered, the files downloaded and the documents generated along the way, in whatever form they were gathered. A summary is never the only record — a lane's work is not thrown away into a summary.
- **The index link lives outside the folder.** Add a line for the record to [`README.md`](README.md), or to a per-area index that `README.md` points to, in the same PR. The index is how the record is found; a folder with no index link outside it is not landed.
- **Landing is automatic.** A lane lands its work product as a pull request as part of finishing the work. No separate request or reminder is needed, and the work is not done until its record has landed, or been handed off with the branch named.
- **Size.** Smaller files are always fine, even hundreds of them. Large files are kept too, for now; moving large material to a separate repository with an R2 packing bucket is possible future work, not a reason to drop material today.
- **Work in the open.** Anything that is not literally a live credential value belongs in the repository. Doubtful material that is not a live credential value is included. Revoked or never-authorised test certificates and keys may be included, with their status labelled.
- **The one carve-out.** Live credential values — API keys, passwords, tokens and in-use private keys — are never committed. Refer to a credential by name only.
- **Goal bookkeeping.** A goal's `hidden_files/` — cycle logs, watermarks, working state — are tracked in the hatch-personal repo's git (Drew, 2026-11-23): "hidden" is a presentation boundary (the files are not shown as a goal's documents), not a preservation boundary. The carve-out above applies there exactly as everywhere else.

### Maintained control documents

Maintained control documents are:

- the root entry points: `README.md`, `AGENTS.md`, `CLAUDE.md` and `ROADMAP.md`;
- indexes, such as `doc/README.md` and `planning/plans/README.md`;
- everything in `planning/`, including roadmaps, plans, templates and the workflow library;
- in a research package:
  - the main reference `main.md` and the design `design.md`;
  - each concept file, which only its current owner edits.

The rules for them:

- Edit them in place, so they stay current. Do not pile history at the bottom.
- Change them only through a PR from your own branch. Many agents read and write them, and a PR is where conflicts are resolved before anything is lost. [`planning/workflow/collaboration.md`](../planning/workflow/collaboration.md) has the rules for parallel writers.
- Every change gets a change-log line:
  - In a research package, a change to `main.md` or `design.md` adds one line to the package's `changes.md`. That file is union-merged, so parallel PRs do not conflict.
  - A document with its own `## Change log` section gets one line there per PR.
  - Elsewhere, the PR and its commit message are the record.
- Do not delete what a statement originally said. Mark it `clarified`, `superseded` or `retracted`: the new status goes in front with a UTC time and a short reason, and the earlier trail follows. [`planning/workflow/records.md`](../planning/workflow/records.md) defines the item trail format.

## Books stay live

The repository carries seven documentation books, built with mdBook and published together on the one GitHub Pages site: mcpx (`pkgs/mcpx/docs/`), fleet (`pkgs/fleet/book/`), atlas (`pkgs/atlas-search/book/`), id (`pkgs/id/book/`), Project Management (`pm-stack/book/`), the Code Book (`pkgs/code-book/book/`) and the Lab Book (`pkgs/lab/book/`). A book is a maintained control document: it states how its subject stands now, and it goes stale the moment the subject changes underneath it. Books are therefore updated live, as part of the change that moves them — not batched into a later pass.

- **The trigger.** A change touches book-documented ground when it alters anything a book states: a command, flag, config key or setting; a behaviour or failure shape; a count a book prints (settings, chapters, workers, databases, seats — any number); a status (live, parked, held, landed, closed); or a design decision a book records. The test is mechanical: if the change, once merged, would make a sentence in a book false, the book is in scope for that change.
- **The update rides with the change.** When the change is direct — its author knows exactly which statement it moves — the same PR updates the book source: the chapter that owns the topic, in the book's own voice and structure, with every count re-derived from the subject on the day, never carried forward from the book's previous text. The change is not done until the book matches it.
- **When it cannot ride, it follows immediately.** If the book update cannot land in the same PR — the change is already merged, the owning chapter needs a wider rewrite, or a figure cannot be re-derived from inside the change — open the book-update PR immediately after, in the same working session, and link it from the change PR. A book statement known to be false is a defect: if the update cannot be written either, file the issue in the same pass, naming the stale statement.
- **Which book owns the ground.** mcpx's ground is `pkgs/mcpx/` — and its book carries a machine-checked sync contract in the package README plus the build gate; where that contract speaks, it governs. Fleet's ground is `pkgs/fleet/` and the fleet surfaces it operates (the queue broker, placement, seats, bridges and doors). Atlas's ground is `pkgs/atlas-search/` and the Atlas stores. The id book's ground is `pkgs/id/`. The Project Management book's ground is `pm-stack/` plus the estate-wide censuses it carries (Cloudflare Workers, Turso databases, provider quotas and costs). The Code Book's ground is this repository itself and the book programme. The Lab Book's ground is the lab: Nix and NixOS, the laptop and cluster work, CI/CD. A change spanning subjects updates every book whose statements it moves.
- **Generated text is fixed at the source.** Where a book carries generated regions — parity tables, settings inventories, protocol regions — fix the source string or registry and regenerate. Never hand-edit a generated region.
- **The scheduled passes are the net, not the mechanism.** A weekly review aligns every book against the week's changes, and a daily spot-check covers the fast-moving books (id, fleet, atlas, mcpx) over the last 36 hours. They exist to catch what slipped. A change whose author already knows a book statement moved is updated live under the rules above, never left for a scheduled pass to find.

## Handling Superseded Features

If a new feature replaces or subsumes an old documented feature:

1. Create a new folder based on the new feature's creation date
2. Reference the old feature's folder in the new documentation
3. Add a note file in the old feature's folder backlinking to the new one
   - This may not be a 1:1 replacement—could indicate a shift in direction
   - Old feature may be deprioritized over time, or not

## Noticed Discrepancies

If the latest summary + subsequent notes are out of date with the actual implementation:

- Create a TODO to provide an update datetime file
- Cover at minimum: noticed differences, understanding of intent/implications, timeline if known

## Example Structure

```
docs/
├── 2026-03-10T08-00-00Z_feature_meta_protocol/
│   ├── 2026-03-10T08-00-00Z_feature_meta_protocol.md          # Initial design
│   ├── 2026-03-12T14-00-00Z_feature_meta_protocol_revised.md  # Post-rollout summary
│   └── 2026-03-20T10-00-00Z_note_superseded_by_v2.md          # Backlink to replacement
├── 2026-03-20T09-00-00Z_feature_meta_protocol_v2/
│   └── 2026-03-20T09-00-00Z_feature_meta_protocol_v2.md       # References old folder
```

## Change log

- 2026-11-24: Require new design records to state the observable acceptance result and the evidence that will establish it.
- 2026-11-24: Lane work product: recorded that goal `hidden_files/` are tracked in the hatch-personal repo's git (Drew, 2026-11-23) — presentation-hidden is not untracked, and the live-credential carve-out applies there too.
- 2026-11-24: Added "Books stay live": a change that moves anything a book states carries its book update in the same PR, or as an immediate linked follow-up PR in the same session; the weekly review and daily spot-check are the catch-net, not the mechanism.
- 2026-11-24: Added lane work product as a dated-record class: reports and one-offs land in the repository by pull request, with their gathered material in the folder, an index link outside it, the size and openness rules, and the live-credential carve-out.
- 2026-10-21: Split dated historical records from maintained control documents. The older paths here say `docs/`, which is a symlink to `doc/`; both reach the same folders. Rules from earlier versions are unchanged.
