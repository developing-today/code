# Child facet issue template

S5 (`issue-handoff` mode `fanout`) opens one per facet of a parent issue; an S6 worker dispatched by the design orchestration issue works it. Title: `Design: <facet> (<package>·C001.3)`, where `C001.3` is the item `item add` allocated for the facet. Run the marker search in the [README](README.md) first.

````markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

Facet **<facet>** of <concept title> is settled: [`<design record path>`](https://github.com/<issue_repository>/blob/<trunk>/<design record path>) answers <the question>, every decision it needed is a decision record, and item C001.3 in the main reference shows the result.

## Read first

1. [Parent issue #<n>](https://github.com/<issue_repository>/issues/<n>) and [orchestration issue #<n>](https://github.com/<issue_repository>/issues/<n>).
2. [The concept file](https://github.com/<issue_repository>/blob/<trunk>/<package path>/concepts/C001-<slug>.md), sections <Research, Sources>.
3. [Main reference, item C001.3](https://github.com/<issue_repository>/blob/<trunk>/<package path>/main.md#c001-<title-anchor>) and [transcript](https://github.com/<issue_repository>/blob/<trunk>/<package path>/transcript.txt) segments `<T012>`.
4. [Decision protocol](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/decisions.md) and [design record template](https://github.com/<issue_repository>/blob/<trunk>/planning/templates/design.md).

## Current state (verified <UTC> against `<trunk SHA>`)

- <what is known and decided about this facet, with evidence>
- Open: <questions this issue must answer>

## Your role

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- You are the worker for this facet and the only writer of its outputs. Use parallel read-only subagents for separable research.
- When the evidence settles a choice, decide it and record it. When it does not, record your preliminary choice as `agent-preliminary` and keep going; a conflict with another facet goes to the orchestrator as a note, not into the other facet's files.

## Owned outputs

- Writes: `<design record path>`, `decisions/D<nnn>-<slug>.md` (IDs from `ids`), item C001.3 and its children (`item set-status`, `item add`), one `changes.md` line per change.
- Branch `<branch>` from `<trunk>`, one PR into `<trunk>`.
- Must not touch: other facets' design records and decisions.

## Prerequisites

- Blocked by: <#n, or "none">.
- Parallel-safe with: <#n, #n>.
- Shares files with: <"none", or the issue to finish first>.

## First steps

1. Boot a fresh worktree and push the empty branch:
   ```sh
   source <git_env>
   git -C <checkout> fetch -q origin
   git -C <checkout> worktree add -b <branch> <worktree_root>/<branch-slug> origin/<trunk>
   cd <worktree_root>/<branch-slug> && git push -u origin <branch>
   ```
2. Take the facet's lease (`lease acquire`), make a first commit and open the PR.
3. Answer the open questions, then write the design record and decisions.

## Done when

- Outcome: the goal above is true on `<trunk>`.
- Verified by: the design record covers intent, design, decisions, alternatives, consequences and open questions; each decision record lists options steelmanned, choice, rationale, source and reversal cost; `check --package <package path>` passes; the PR is merged.
- Boundaries: design only, for this facet. No code, no plan, no changes to other facets.
- Stop as blocked: if the facet cannot be settled without another facet or a human, record the preliminary choice and the question with its default, comment here, set the run status to `blocked`, and stop.
- Report here and in your final message: branch, PR URL, last pushed SHA, what is verified, what is not.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S5","kind":"design","item":"C001.3"} -->
````
