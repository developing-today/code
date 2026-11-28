---
name: issue-handoff
description: "Publish the GitHub issues that carry one concept of a workflow research package from research into design. Mode parent (S4) reviews the whole reconciled concept document and writes one self-contained issue to investigate and implement some or all of it, with a basic design plan in high-level stages. Mode fanout (S5) lets a zero-context agent turn that parent issue into one child issue per facet, each a full handoff, after estimating size and cost, and fills the design's initial research stages with every related issue in priority order with blocked-by and parallel-safe-with. Mode orchestration writes the issue that runs subagents against the child issues. Use when a concept has passed notes-reconcile, or when given a parent handoff issue URL to fan out. For a handoff file of the current conversation use handoff; for running the child issues use design-orchestrate."
---

# Issue handoff (S4 and S5)

You turn a reconciled concept into GitHub issues that a fresh agent with no context can pick up. One skill, three modes, run in this order:

- **`parent` (S4)**: one issue per concept, written after reading the whole concept document. It hands the concept on for investigation and implementation, in whole or in part, and sets out the basic design plan in high-level stages.
- **`fanout` (S5)**: a zero-context agent given only the parent issue URL creates one child issue per facet, each a complete handoff, then records them all in the design's initial research stages and in the main reference.
- **`orchestration` (end of S5)**: the same agent writes one issue whose job is to run subagents against the child issues. `design-orchestrate` executes it.

The stage contract is in [stages.md](../../../planning/workflow/stages.md), issue rules in [records.md](../../../planning/workflow/records.md), body templates in [templates](../../../planning/workflow/templates). Commands below are `python3 -I scripts/research_workflow.py <command>` from the repository root; `<pkg>` is `doc/<UTC>_research_<slug>`.

## Load or not

Load for:

- "create the handoff issue for C003" after its notes are reconciled;
- "here is issue #501, fan it out into child issues" (mode `fanout`);
- writing the orchestration issue for a set of child issues;
- filling the initial research stages of `design.md` for a concept.

Do not load for:

- a markdown handoff of the current conversation: `handoff`;
- the S1 research trigger issue: `research-prepare`;
- running the child issues, merging their PRs or deciding between options: `design-orchestrate`;
- plan or implementation issues: `feature-plan`;
- triaging incoming bugs or feature requests.

## Ownership and parallelism

One agent per concept runs `parent`, and one per parent issue runs `fanout` then `orchestration`, so concepts move through S4 and S5 in parallel. You are the only writer of the issues you create and, while you hold the concept lease, of the concept's state block. `main.md` and `design.md` change only inside this concept's own sections and only through your PR. You never edit another concept's issues or sections. Boot, lease, heartbeat and final-report rules are in [collaboration.md](../../../planning/workflow/collaboration.md).

**No recursion.** `parent` does not fan out. `fanout` creates child issues and one orchestration issue, and runs none of them. Child issues are leaves: their workers never fan out further or run this skill, and they report new work to the orchestrator instead. Subagents you launch (validators, searchers) are read-only leaves.

## Steps

1. **Boot and claim.** Boot per collaboration.md on your own branch (the brief's name, else `research/<slug>/s4-c001` or `research/<slug>/s5-c001`), push it empty, take the concept lease with `lease acquire`, and append `start` to your live log.
   Done when the branch is pushed and `lease list` shows your lease.
2. **Verify the entry state.** Record the trunk SHA and UTC time. For `parent`: the S3 PR is merged, and on `origin/main` both `check --package <pkg> --require-resolved` and `check --package <pkg> --remote-notes` show nothing pending. For `fanout`: the parent issue is open and its preconditions still hold. Read `status --live --package <pkg>` for the concept's issues and PRs. If a condition fails, stop as blocked with the reason.
   Done when every condition is checked against the recorded SHA, with the command output kept for the "Current state" section.
3. **Read everything, three times.** Read the whole concept document, its `main.md` section, `design.md`, the transcript segments and every linked issue and PR: at the start, before drafting, and before publishing. Log each re-read.
   Done when your live log has three `reread` lines.
4. **Draft the bodies for your mode.** `parent`: read `references/parent.md`. `fanout`: read `references/fanout.md`, and estimate size and cost before drafting any child. `orchestration`: read `references/orchestration.md`. Every body is self-contained and follows `references/publishing.md` for links, length and the marker.
   Done when each draft has every section its reference lists, and every claim has evidence.
5. **Check for duplicates.** For each draft, look for an issue that already carries its marker: first in the concept state block and your live log, then by search with an exact match on the body (`references/publishing.md`). Reuse what exists; never create a second issue with the same marker.
   Done when each draft is marked `new` or `exists: <url>`.
6. **Validate with zero context.** Give each draft body, and nothing else, to a fresh read-only subagent using the zero-context reader prompt in `references/publishing.md`. Fix every gap it reports and re-run until it has no blocking question.
   Done when each body's last validation lists no blocking question, no broken link and no unlocatable claim.
7. **Publish.** Create the issues with `gh issue create --repo <repo> --title <title> --body-file <file>`, adding only labels that already exist and, for child issues, `--parent <parent-number>`. Record each URL in your live log at once, then verify it with `gh issue view <url> --json url,state,title,body`.
   Done when every issue is open, its rendered body ends with its marker, and its URL is recorded.
8. **Record and mark.** Add each issue to the concept state block's `issues` (`{number, url, stage, state}`) and append a `history` entry. Add an item with `item add` in the concept's `main.md` section that marks this stage and says what was found ("S5 · <UTC> — 7 child issues, 2 related issues, orchestration #510"), and use `item set-status` on any earlier item this stage clarified or superseded. `fanout` also writes the concept's "Initial research stages" sub-section in `design.md`. Never remove an earlier finding. Run `render` and commit only your concept's hunks; add one line to `changes.md`.
   Done when `check --package <pkg>` and `check --changed-since origin/main` pass.
9. **Publish the record and report.** Format your files (`prettier --write`), merge `origin/main`, push, open a ready PR into `main`, `lease release`, append `end`, and comment the PR URL on the parent issue.
   Done when `git ls-remote origin <branch>` equals the PR head and the reported SHA.

## References

- `references/parent.md`: mode `parent`, its preconditions, the body sections and the basic design plan. Read at step 4 in that mode.
- `references/fanout.md`: mode `fanout`, the size and cost estimate, related-issue search, dependency analysis, child bodies, and the `design.md` and `main.md` updates. Read at step 4 in that mode.
- `references/orchestration.md`: mode `orchestration` and its body. Read at step 4 in that mode.
- `references/publishing.md`: titles, markers, duplicate search, labels, links, length, zero-context validation and publishing. Read before steps 4 to 7 in every mode.

## Turn report

End every turn with:

```text
Done: <issues created or reused, with URLs; sections written>
Verified: <gh issue view -> open, marker present>, <check -> ok>, <zero-context validation -> no blocking gaps>, ...
Remaining: <"fanout of <parent url>", "design-orchestrate on <orchestration url>", or what is left>
Questions (non-blocking): <question -> default being used>, or "none"
Blocked on user: <only: no repository, gh cannot create issues, or two readings that give different issues>, or "none"
```

The final turn also gives the branch, the PR URL, the last pushed SHA, and the size and cost estimate in `fanout` mode.
