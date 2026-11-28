# Mode orchestration (end of S5)

After the children exist, the fan-out agent writes one more issue: the handoff whose purpose is to orchestrate subagents against every child issue. `design-orchestrate` runs it. You write it; you do not run it.

## Title and marker

- Title: `Design: <concept title> (<package>·C001)`.
- Marker: `<!-- workflow:issue {"concept":"C001","kind":"orchestration","package":"<package>","stage":"S5"} -->`.

## Body sections, in order

1. **Opening line**, as in `parent.md`.
2. **Goal.** The end state in the present tense: every child issue has been researched by a worker, its PR is merged, and its findings are folded into the concept's design records; decisions are recorded with steelmanned options and their source; the challenge pass is done; the minimal human-review list exists; and the plan issue for `feature-plan` is open.
3. **Current state (verified `<UTC>` against `<SHA>`).** The parent, the children by wave with their state, the estimate from the parent comment, and anything already in flight.
4. **Your role: orchestrator, not implementer.** Quote the user's instruction verbatim:

   > "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

   Then link the orchestrator rule in `stages.md` and the merge gate and restart rules in `collaboration.md`, without restating them.

5. **Read first, in order.** The parent issue, the concept document, the concept's sub-section of the initial research stages in `design.md`, `collaboration.md`, `decisions.md`, and the reviewer, critic and investigator prompts in `planning/workflow/templates/prompts/`. One line each on why.
6. **Child issues by wave.** A snapshot of the `design.md` sub-section: links, ids, one-line descriptions, priorities, blocked-by and parallel-safe-with. Say that `design.md` on `main` is the live version.
7. **Where to start.** Dispatch wave 0 first. Compute the frontier with `ready --package <pkg>` and from what has merged, not from GitHub's blocked-by state. Take a lease per child with `lease acquire`. Keep at most `max_parallel_writers` workers at once. Give each worker the child issue URL and its owned output path; pass URLs and paths, not pasted summaries.
8. **Decisions and human actions to raise at the start.** The D and H items from the parent that are still open, with defaults. Conflicting worker findings go to the decision protocol in `decisions.md`; work continues on the preliminary choice.
9. **Traps.** From `fanout.md`, plus: a worker that reports done without a pushed SHA has not finished; a child that finds new work reports it here and does not fan out; late notes are picked up by notes-reconcile, not waited for.
10. **Keeping track.** Comment on this issue when a worker PR opens, merges or blocks. Update `main.md` and `design.md` only through PRs, inside this concept's sections, marking each stage and what it found, and never removing earlier findings.
11. **Done when.** Checkboxes matching the goal, plus: every child is closed with a link to its merged PR, and the plan issue URL is commented here.
12. **Related.** The parent, sibling concepts' orchestration issues, and anything sharing files.
13. **Not verified.**
14. **Marker.**

## After publishing

- Comment on the parent with links to every child and to this issue.
- Comment on each child with this issue's link, so its worker knows where to report.
- Add the issue to the concept state block's `issues` with stage `S5`, and fill the orchestration link into the `design.md` sub-section.
- If the user keeps a personal handoff list (cairn's handoff goal uses `~/git/handoffs/TODO.md`), append one line for this issue, as in `parent.md`.
