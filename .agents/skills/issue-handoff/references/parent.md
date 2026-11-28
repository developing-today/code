# Mode parent (S4)

One issue per concept. You read the whole concept document and write the handoff that a zero-context agent will later fan out into child issues. The issue covers investigating the concept and implementing some or all of it.

## Preconditions

- The concept's S3 notes-reconcile PR is merged into `main`.
- On `origin/main`, `check --package <pkg> --require-resolved` exits 0 and `check --package <pkg> --remote-notes` lists nothing pending for the concept.
- No open issue already carries this concept's parent marker (`publishing.md`).

If one fails, stop as blocked and name it.

## Review the whole document

Before drafting, build these lists from the concept document. Each entry links the section it came from.

- **Scope.** What is in scope to implement (all of the concept by default), what is only to investigate, what is `deferred` (with reason and revisit trigger) and what is `contingent` (with its trigger). Follow the concept's priority tier.
- **Facets.** Every facet item (`C001.N`) with a one-line scope. Add the gaps you find: an open question or risk that needs research of its own becomes a proposed facet, marked as proposed by S4.
- **Known edges.** Relations to other concepts, shared files and undecided shared interfaces between facets.
- **Open questions and preliminary recommendations**, each with its default.
- **Existing issues and PRs** for this concept: the S1 research issue, the S2 and S3 PRs, and any issue that links the concept.

## Title and marker

- Title: `Handoff: <concept title> (<package>·C001)`.
- Marker, the last line of the body: `<!-- workflow:issue {"concept":"C001","kind":"parent","package":"<package>","stage":"S4"} -->`.

## Body sections, in order

1. **Opening line.** "Picking this up with no context? Everything you need is here or linked. Read 'Your role' first, then 'Current state'."
2. **Goal.** The end state in the present tense: "Every facet of C001 has a research child issue; the concept's initial research stages in `design.md` list them, with every related issue, in priority order; an orchestration issue runs them." Then the longer-term goal: the concept designed, planned and implemented (in full, or the parts in scope).
3. **Links.** Full GitHub URLs: the concept document, its `main.md` section, the transcript with segment ids, `design.md`, `changes.md`, the S1 research issue, the S2 and S3 PRs.
4. **Current state (verified `<UTC>` against `<trunk SHA>`).** What the research established, in bullets that link the document's sections; note status (receipt count, last scan time); what is not known yet. Every bullet has evidence.
5. **Your role.** "You create issues; you do not research, design or implement. Load the `issue-handoff` skill and run mode `fanout`, then mode `orchestration`, on this issue." Link the skill file on `main`.
6. **Facets to fan out.** One bullet per facet: id, title, one-line scope, the document sections it draws on, a suggested wave, known blockers, and shared files or interfaces. Proposed facets are marked as such.
7. **Basic design plan.** The high-level stages below, adapted to the concept.
8. **Decisions and human actions to raise at the start.** `D1`, `D2`, ... and `H1`, `H2`, ..., each with the question or need, the recommended option, the default used if there is no answer, and what it blocks. Leave out anything the transcript already answers.
9. **Traps.** The concept's top risks, plus the workflow traps in `fanout.md` ("Traps").
10. **Repository mechanics.** Links to `collaboration.md`, `.agents/research-workflow.json` and `AGENTS.md`; the git environment line; the tool commands the next agent uses.
11. **Done when.** Checkboxes: child issues exist, one per facet, each validated with zero context; the `design.md` initial research stages sub-section is merged; the concept's `main.md` section marks S5 and what it found; the orchestration issue exists; this issue has a comment linking all of them.
12. **Related.** Sibling concepts' parent issues and related work, each with what it shares.
13. **Not verified.** What you could not check and why, plus the searches behind any negative claim.
14. **Marker.**

## Basic design plan (section 7)

The stages mostly cover research and design. Planning and implementation get one line each, because their own skills set them up later.

- **Stage R, initial research.** One child issue per facet, in waves: wave 0 pins shared names, interfaces and foundation files; wave 1 runs every independent facet in parallel; wave 2 is anything that needs wave 1's results. Entry: the child issues exist. Exit: every child's findings are merged. Skill: `design-orchestrate`, driven by the orchestration issue.
- **Stage D, design.** Design records, decision records with steelmanned options, the challenge pass and the minimal human-review list. Entry: stage R exits. Exit: the design is merged and the plan issue exists. Skill: `design-orchestrate`; rules in `decisions.md`.
- **Stage P, feature plan.** Skill `feature-plan`.
- **Stage I, implementation.** Skill `implementation-orchestrate`.

## After publishing

- Add the issue to the concept state block's `issues` with stage `S4`, and mark S4 in the concept's `main.md` section with an item.
- If the user keeps a personal handoff list (cairn's handoff goal uses `~/git/handoffs/TODO.md`), append one line `- [<short goal>](<issue URL>)`. Never edit existing lines; skip the append if the URL is already there.
