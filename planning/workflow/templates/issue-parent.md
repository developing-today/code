# Parent handoff issue template

S4 (`issue-handoff` mode `parent`) opens one per concept once its research and reconcile are merged. A zero-context agent takes the URL and runs S5: one child issue per facet, the "Initial research stages" section of `design.md`, and the orchestration issue. Title: `Handoff: Investigate and plan <concept title> (<package>·C001)`. Run the marker search in the [README](README.md) first.

````markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

Every facet of **<concept title>** that still needs research, review, confirmation or design has its own self-contained child issue; [`design.md`](https://github.com/<issue_repository>/blob/<trunk>/<package path>/design.md) lists them under "Initial research stages" in priority order with parallel groups; and one orchestration issue runs them.

## Read first

1. [The concept file](https://github.com/<issue_repository>/blob/<trunk>/<package path>/concepts/C001-<slug>.md): the research this builds on.
2. [Main reference, section C001](https://github.com/<issue_repository>/blob/<trunk>/<package path>/main.md#c001-<title-anchor>): the items each facet maps to.
3. [Transcript](https://github.com/<issue_repository>/blob/<trunk>/<package path>/transcript.txt), segments `<T007-T009>`.
4. [issue-handoff skill](https://github.com/<issue_repository>/blob/<trunk>/.agents/skills/issue-handoff/SKILL.md), [child](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/templates/issue-child.md) and [orchestration](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/templates/issue-orchestration.md) templates.

## Current state (verified <UTC> against `<trunk SHA>`)

- Research: [#<n>](https://github.com/<issue_repository>/issues/<n>), merged in [#<n>](https://github.com/<issue_repository>/pull/<n>).
- Notes: all resolved (`check --require-resolved` passes).
- Open questions and preliminary decisions: <list, with links>.

## Facets found so far

- <facet>: <the question it answers>. Covers <C001.2>. Blocked by: <none, or facet>.
- <facet>: <...>.

## Your role

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- Draft child issues in parallel subagents, one facet each; give every draft to a zero-context reader before publishing. You publish, link and record.
- Size each facet for one fresh session. Mark hard blockers (`requires`) apart from soft order (`recommended-after`, with the rework it costs).

## Owned outputs

- Child issues (one per facet), each with an item allocated by `item add` (`C001.<n>`).
- The C001 part of "Initial research stages" in `design.md`, and the concept's `issues`, through one PR from `<branch>`.
- The orchestration issue.
- Must not touch: other concepts' files and sections.

## Prerequisites

- Blocked by: the research issue [#<n>](https://github.com/<issue_repository>/issues/<n>) (merged).
- Parallel-safe with: the parent issues of other concepts.

## First steps

1. Boot a fresh worktree and push the empty branch:
   ```sh
   source <git_env>
   git -C <checkout> fetch -q origin
   git -C <checkout> worktree add -b <branch> <worktree_root>/<branch-slug> origin/<trunk>
   cd <worktree_root>/<branch-slug> && git push -u origin <branch>
   ```
2. Confirm or extend "Facets found so far" from the concept file; allocate an item per facet with `item add`.
3. Fan out the child-issue drafts, search each marker, publish, then write the orchestration issue.

## Done when

- Outcome: the goal above is true.
- Verified by: each facet has an open child issue with its marker; `design.md` lists every issue related to C001 with a one-line description, priority and parallel group; `render` shows them in `main.md`; the PR is merged.
- Boundaries: issues and planning only. No research answers, design records or code.
- Stop as blocked: if a facet cannot be scoped without a human answer, publish the rest, list that facet with its question and default in a comment here, and stop.
- Report here and in your final message: branch, PR URL, last pushed SHA, issue URLs, what is verified, what is not.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S4","kind":"handoff"} -->
````
