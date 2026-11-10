---
name: concept-research
description: "Write the definitive research document for one concept of a workflow research package (doc/<UTC>_research_<slug>/concepts/C001-<slug>.md) at stage S2: restate the request, then cover every facet, feasibility, state of the art, prior art and example projects, labelled anecdotes, steelmanned alternatives, risks, open questions and preliminary recommendations, with typed sources cited only from tool results, a research log and a change log. Runs read-only research subagents per facet while the concept owner stays the only writer, sends cross-concept notes, and records refinements as items directly below what they refine. Use when a concept's S1 preparation or research issue is done and its full research should start, resume or go deeper. For mapping the repo and writing research questions first use research-prepare; for the final note sweep use notes-reconcile; for a research question outside a package use research."
---

# Concept research (S2)

You own one concept file, `concepts/C001-<slug>.md`, and turn it into the definitive document for that concept: everything a later design, plan or implementation agent needs, every claim traceable to a source, and nothing from earlier stages lost. There is no length limit; later runs add more detail. The stage contract is in [stages.md](../../../planning/workflow/stages.md), file formats in [records.md](../../../planning/workflow/records.md), and the concept template in [templates](../../../planning/workflow/templates) (`concept.md`).

Commands below are `python3 -I scripts/research_workflow.py <command>`, run from the repository root; `<pkg>` is the package folder `doc/<UTC>_research_<slug>`. Run `<command> --help` for argument names.

## Load or not

Load for:

- running or resuming a package concept's research issue (`Research: <concept> (<package>·C001)`);
- deepening an existing concept document: more facets, sources, alternatives or risks;
- answering "research everything about C003 so a designer needs nothing else".

Do not load for:

- mapping the repository and writing research questions before research: `research-prepare`;
- the final sweep of notes and receipts after research: `notes-reconcile`;
- writing the GitHub handoff or fan-out issues: `issue-handoff`;
- choosing between options or writing decision records: `design-orchestrate`;
- a research question that belongs to no package concept: `research`.

## Ownership and parallelism

Many concept owners run at once, one per concept, so nothing here waits on another concept. You are the only writer of your concept file, your outgoing notes and your receipts. Facet subagents are read-only: they return evidence and never write in the repository. In `main.md` you touch only your concept's section, and only through your own pull request. Anything for another concept goes in a note; anything else wide goes through a PR. Boot, branch, lease, heartbeat and final-report rules are in [collaboration.md](../../../planning/workflow/collaboration.md); follow them, do not restate them.

## Steps

1. **Claim and boot.** Follow the worker boot in collaboration.md: git env, fresh worktree from `origin/main`, your own branch (the brief's name, else `research/<slug>/s2-c001`), push it empty. Take the concept lease with `lease acquire` and renew it before it expires. Append a `start` line to your live log.
   Done when `lease list` shows your lease, `git ls-remote origin <branch>` shows the branch, and the log has `start`.
2. **Read the inputs.** Read the concept file (S0 excerpts and thoughts, the S1 repo map and research questions), its research issue, the concept's section of `main.md`, the "Reading of garbled input" register, and every transcript segment the state block lists (`ids` resolves allocated ids; never invent one). Run `check --package <pkg> --remote-notes` and read every note already addressed to you.
   Done when you can list each segment and each S1 question for this concept, and each note addressed to it.
3. **Restate the request and fix the facets.** Write "Request restated" (segment ids, assumptions, inference labelled). List the facets: every S1 question, every distinct sub-topic in the segments, and every idea that was hinted, implied or missed but belongs in the record. Give each facet a tool-allocated item id with `item add` under the concept in `main.md` (for example `C001.4`), and use that id as the facet's heading in the concept file.
   Done when every segment and every S1 question maps to at least one facet, and every facet has an id.
4. **Research the facets in parallel.** Write one brief in a scratch directory outside the repository and launch one read-only subagent per facet, or per cluster of small facets, with disjoint scopes. Research the repository yourself where the facet needs measured facts (`path:line` at a recorded SHA). Read `references/facet-subagents.md` first.
   Done when every facet has evidence back, or an explicit "one source only" or "no source found" with the queries tried.
5. **Verify before writing.** Re-fetch every page you will cite, match each quote exactly and check the heading above it, re-measure every number and every subagent claim you will repeat, and re-run repository commands at your SHA. Read `references/citations.md`.
   Done when every `[n]` traces to a page fetched in this run and every number states its sample and scope.
6. **Write the document.** Add the sections in `references/document-outline.md`, in order, below what S0 and S1 wrote: facets, feasibility, state of the art, prior art and example projects, use cases and labelled anecdotes, steelmanned alternatives, risks, open questions, preliminary recommendations, cross-concept notes, unsourced ideas (each argued for and against), sources, research log, change log. Commit and push after each large section so a restart loses little.
   Done when every section exists or says in one line why it does not apply, and every fact carries `[n]`, `[unsourced]` or "Inference:".
7. **Refine, never overwrite.** Earlier text stays. When research clarifies, supersedes or retracts an earlier item, add the refinement as a child directly below it with `item add`, then prepend the new status to the earlier item with `item set-status` (`clarified`, `superseded` or `retracted`, with one sentence such as "More nuanced than S0 thought; see C001.2.1"). In the concept file, write the refinement directly below the text it refines, carrying the same item id.
   Done when `check --package <pkg>` passes and `git diff origin/main -- <pkg>` deletes no earlier line except where a status prefix was added.
8. **Exchange notes as you go.** When a finding affects another concept, send `note --package <pkg> --from C001 --to C00N --input <file>` (one topic per note; say what you found, the evidence, and what you suggest). Answer notes addressed to you while you work with `resolve-note` (see the notes-reconcile skill's `references/receipts.md` for dispositions) instead of leaving them all for S3. Re-run `check --package <pkg> --remote-notes` before you finish.
   Done when "Cross-concept notes" lists every note you sent and every note received so far, each with its receipt or "pending S3".
9. **Update state and logs.** In the state block set `stage` to `S2`, update `relations` from the feasibility section, and append a `history` entry. Add a change-log line at the bottom of the concept file and one line to the package `changes.md`. Run `render` and commit only the hunks inside your concept's own sections; the router refreshes shared views such as the run table.
   Done when `check --package <pkg>` and `check --changed-since origin/main` pass.
10. **Publish and report.** Format your files (`prettier --write`), merge `origin/main` into your branch, push, open a ready PR into `main` whose body says what, why, evidence and what is not verified, record the PR in the state block, `lease release`, and append `end` to your log.
    Done when `git ls-remote origin <branch>` equals the PR head and the SHA in your report.

## References

- `references/document-outline.md`: the section list and what each section must hold. Read at steps 3 and 6.
- `references/citations.md`: source, citation, quote and count rules, and the unsourced-ideas format. Read before step 4 and again at step 5.
- `references/facet-subagents.md`: the facet brief, the return format and how to merge results. Read at step 4.

## Turn report

End every turn with:

```text
Done: <what this turn finished>
Verified: <command -> result>, ...
Remaining: <work still to do; when complete, "notes-reconcile for C001 after PR <url> merges">
Questions (non-blocking): <question -> default being used>, or "none"
Blocked on user: <only what nothing else can proceed without>, or "none"
```

The final turn also gives the branch, the PR URL, the last pushed SHA and the counts of facets, sources and `[unsourced]` claims.
