# Research issue template

S1 (`research-prepare`) opens one per concept, after writing the concept's repository map and research questions, to trigger S2 (`concept-research`). Title: `Research: <concept title> (<package>·C001)`. Run the marker search in the [README](README.md) first; afterwards add the issue to the concept's `issues` and run `render`. `<package path>` is `doc/<UTC>_research_<slug>`.

````markdown
Picking this up with no context? Everything you need is here or linked. Read "Your role" first, then "Current state" and "Read first".

## Goal

[`concepts/C001-<slug>.md`](https://github.com/<issue_repository>/blob/<trunk>/<package path>/concepts/C001-<slug>.md) is the definitive research record for **<concept title>**: feasibility, prior art and example projects, use cases, anecdotes, alternatives steelmanned, risks with mitigations, every source cited, and every note to or from other concepts resolved by a receipt.

## Read first

1. [The concept file](https://github.com/<issue_repository>/blob/<trunk>/<package path>/concepts/C001-<slug>.md): transcript excerpts, repository map and research questions are at the top.
2. [Main reference, section C001](https://github.com/<issue_repository>/blob/<trunk>/<package path>/main.md#c001-<title-anchor>): how the concept fits the request.
3. [Transcript](https://github.com/<issue_repository>/blob/<trunk>/<package path>/transcript.txt), segments `<T007-T009>`, `<T031>`: the exact request.
4. [Meta design](https://github.com/<issue_repository>/blob/<trunk>/<package path>/design.md): order, waves and who else is running.
5. [concept-research skill](https://github.com/<issue_repository>/blob/<trunk>/.agents/skills/concept-research/SKILL.md) and [collaboration rules](https://github.com/<issue_repository>/blob/<trunk>/planning/workflow/collaboration.md).

## Current state (verified <UTC> against `<trunk SHA>`)

- <what S1 found, each with evidence: a path, commit, or command and its output>
- <n> research questions open in the concept file.

## Your role

In the user's words: "it's orchestrating/planning/managing and it should use subagents to implement/review/validate and only handle itself when it can't use a subagent somehow. it should make parallel subagents for any portions that can be broken apart."

- You lead this concept and are the only writer of its concept file. Split separable questions across parallel read-only researcher subagents and merge their evidence yourself.
- A change for another concept goes to it as a note (`note`), never into its file. `main.md` and `design.md` change only through your PR.

## Owned outputs

- Writes: the concept file, outgoing `notes/C001__to__*`, receipts for notes addressed to C001, one `changes.md` line per change.
- Branch `<branch>` from `<trunk>`, one PR into `<trunk>`.
- Must not touch: other concept files, other writers' notes and receipts, anything outside the package.

## Prerequisites

- Blocked by: <#n, or "none">. Confirm with `python3 scripts/research_workflow.py ready --package <package path>`.
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
2. Take the concept's lease (`lease acquire`), make a first commit and open the PR.
3. Answer the research questions, most consequential first; check `check --remote-notes` for notes addressed to C001.

## Done when

- Outcome: the goal above is true on `<trunk>`.
- Verified by: `python3 scripts/research_workflow.py check --package <package path> --require-resolved` passes; `check --remote-notes` lists nothing pending for C001; every claim under "Research" cites a source; the PR is merged with a merge commit.
- Boundaries: research and record only. No code, no design beyond `agent-preliminary` decisions written down as such.
- Stop as blocked: if something only a person or another unfinished concept can provide stops you, comment here with what and why, set the run status to `blocked`, and stop. Do not work around it.
- Report here and in your final message: branch, PR URL, last pushed SHA, what is verified, what is not.

## Not verified

- <what the author of this issue could not check>

<!-- workflow:issue {"package":"<package>","concept":"C001","stage":"S1","kind":"research"} -->
````
