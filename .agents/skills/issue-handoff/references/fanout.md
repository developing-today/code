# Mode fanout (S5)

You start with only the parent issue URL and no other context. You turn the parent into one child issue per facet, each a complete handoff, record them in the design's initial research stages and in the main reference, and then write the orchestration issue (`orchestration.md`). You run none of the child issues.

## F1. Verify the parent

- Read the parent issue, its comments and everything it links, at a trunk SHA you record.
- Parse its marker. Confirm the S4 preconditions in `parent.md` still hold on `origin/main`.
- Compare the parent's "Current state" with the concept document now. Record every discrepancy; the document on `main` wins.

Done when the parent's claims are checked and the discrepancies are listed.

## F2. Gather every related issue and PR

The initial research stages list every GitHub issue that was made for, or is related in any way to, this concept. Find them with:

- the package's markers: `gh issue list --repo <repo> --state all --search '"<package>" in:body' --json number,url,title,state,body --limit 200`, then keep the bodies that contain this concept's id;
- the concept id and title keywords in titles and bodies;
- every issue and PR linked from the concept document, its `main.md` section and the parent;
- PRs that touched the concept file: `gh pr list --repo <repo> --state all --search '<concept file name>' --json number,url,title,state`.

Record each search and its result count, so the negative claim "no other related issue" names where you looked.

Done when you have the list, each entry with number, state and one line on how it relates.

## F3. Settle the facet list

- Start from the parent's facets, including its proposed ones.
- Add any facet the concept document implies but the parent missed, with the reason.
- Size each against one fresh session. If a worker would run out of context before finishing, split the facet. If setting it up would cost more than doing it, merge it with a neighbour. Splits get new item ids from `item add`; never invent ids.

Done when every facet has an id, a one-line scope and a size.

## F4. Estimate size and cost before creating anything

Write the estimate down before drafting a single child:

- the number of child issues, and the waves and critical path from F5;
- the peak number of parallel workers, capped by `max_parallel_writers` in `.agents/research-workflow.json`;
- worker sessions: one fresh session per child, or the number you expect;
- tokens: sessions times a per-session figure you state (for example 150k tokens, about the size of one session's useful context), plus the orchestrator's overhead, labelled an estimate;
- human attention: the decisions and human actions the children will raise.

Post the estimate as a comment on the parent issue. If it exceeds a budget the parent or your brief sets, merge facets until it fits and say what you merged. Never create more child issues than the estimate states.

Done when the estimate comment is on the parent.

## F5. Maximize parallelism

For each pair of facets, decide the edge, and prefer the arrangement with the fewest hard edges:

- `requires` only when one facet's output is a real input to the other.
- `recommended-after` when starting early is possible but costs rework; say what rework.
- `shares-files-with` when both would write the same file: assign the file to one facet, or serialize the two.
- An undecided shared interface (names, paths, formats, a shared module) gets its own wave-0 child that pins it before the facets that use it start. Parallel workers otherwise invent incompatible answers.
- `parallel-safe-with` lists every sibling with no edge and no shared file or interface.

Then group the facets into waves: wave 0 for pinned interfaces and foundation files, wave 1 for every independent facet, wave 2 for facets that need wave 1's results. Priority order within a wave follows the concept's priority tiers, then the reversal cost of being wrong.

Done when every child has its wave, priority, blocked-by and parallel-safe-with lists.

## F6. Draft each child

- Title: `Research: <facet title> (<package>·C001.N)`.
- Marker: `<!-- workflow:issue {"concept":"C001","facet":"C001.N","kind":"child","package":"<package>","stage":"S5"} -->`.
- Body sections, in order:
  1. Opening line, as in `parent.md`.
  2. **Goal**: the facet's end state in the present tense.
  3. **Questions**: the facet's questions, copied exactly from the concept document, with a link to its heading.
  4. **Links**: full GitHub URLs to the facet heading, the concept document, its `main.md` section, the transcript segments, the parent, the orchestration issue (filled in later) and related issues.
  5. **Current state (verified `<UTC>` against `<SHA>`)**: what S2 already found for this facet and what is still unknown, with evidence.
  6. **Owned outputs**: the findings file at the path the orchestration issue assigns (`design-orchestrate` places it in the concept's design record folder), notes to other concepts, and nothing else. It never writes the concept document, `main.md` or `design.md`.
  7. **Acceptance evidence**: what proves it is done: the questions answered with typed, cited sources; the open questions ranked with defaults; the PR merged.
  8. **Prerequisites**: `Blocked by:` issue links with reasons; `Recommended after:` with the rework cost.
  9. **Parallel-safe with**: sibling issue links.
  10. **What to do first**: three concrete actions.
  11. **Done contract**: the outcome, how it is verified, the boundaries (files it may write and must not touch), and the stop-as-blocked rule: if a blocker is not merged or a decision is missing, comment "Blocked: <what>; default: <x>" on the issue, then continue on the default where `decisions.md` allows it, otherwise stop.
  12. **Leaf rule**: "Do not create issues or fan out further. Report new work in your final message and as a comment on the orchestration issue."
  13. **Report**: branch, PR URL, last pushed SHA, what is verified, what is not.
  14. **Marker.**

Done when each body passes zero-context validation (`publishing.md`).

## F7. Create, then cross-link

- Create the children in wave order, blockers first, each with `--parent <parent number>`, recording each URL in your live log as soon as it exists.
- Then edit each child once with `gh issue edit <url> --body-file <file>` to replace facet ids with issue links in `Blocked by` and `Parallel-safe with`.
- The body text is the contract. Where the repository has native blocked-by links, add them too, but the orchestrator computes the frontier from what has merged, not from GitHub's blocked-by state.

Done when every child is open, cross-linked and recorded.

## F8. Write the initial research stages in `design.md`

One sub-section per concept, so fan-outs for different concepts merge cleanly. Bullets only, no tables:

```markdown
### C001 <concept title>: initial research stages

S5 · 2026-10-21T12:00:00Z · parent [#501](url) · orchestration [#510](url) · estimate: 6 children, 3 waves, about 1M tokens

- **Wave 0**
  - [#502](url) `C001.4` Pin shared names for note files — fixes the names the other facets use. Priority 1. Blocked by: none. Parallel-safe with: none.
- **Wave 1**
  - [#503](url) `C001.5` Remote-ref scanning — how to find notes on unmerged branches. Priority 2. Blocked by: #502. Parallel-safe with: #504, #505.
- **Related issues**
  - [#381](url) S1 research issue, closed; context only.
- **What this stage found**: <new details, discrepancies, facets added or merged>.
```

A rerun never removes an earlier sub-section. It adds a new dated block below it, and earlier entries that it replaces get a status trail.

## F9. Update the main reference

In the concept's `main.md` section: add an item that marks S5 and denotes what was found; set `clarified` or `superseded` on earlier items the fan-out changed; add every child and the orchestration issue to the concept state block's `issues` so the generated "Issues and PRs" views list them. Remove nothing.

## Traps

- Relative links in an issue body resolve against the issue URL and break. Use full GitHub URLs (`publishing.md`).
- GitHub search splits the marker into words, and a new issue takes a while to appear in search. Check your own record first, then search, then match the marker exactly.
- Tracker blocked-by state lags behind merges. The frontier comes from what has merged.
- Parallel children that share an undecided name or file collide. Pin it in wave 0.
- A fan-out without an estimate can burn far more tokens than the work is worth. Estimate first.
