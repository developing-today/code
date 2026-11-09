---
name: research-prepare
description: Prepare one concept of a research package for its full research (stage S1). Boots a worker branch, pulls the concept's exact transcript excerpts to the top of its file, maps related code, docs, issues and PRs with file:line evidence, does light web research, writes research questions and facets, sends notes to other concepts and opens an idempotent Research trigger issue and a PR. Use when a concept stub from intake needs preliminary research, a research brief, a repo map or its Research issue, or when research-workflow dispatches S1 for a concept. Not the full research, which is concept-research (S2).
---

# Research prepare (S1)

Prepares one concept so the agent who picks up its Research issue can do the full research with almost no other context. There is one agent per concept, and many run in parallel. This is not the full research: S1 makes no recommendation, takes no decision and does no design. That is `concept-research` (S2). The stage contract is in [stages.md](../../../planning/workflow/stages.md), and the concept file format is in [records.md](../../../planning/workflow/records.md).

## Inputs

These come from the worker brief, or from the user:

- `PKG`, the package folder;
- `C`, the concept id (for example `C003`), and its concept file;
- the branch, `research/<slug>/s1-<c003>`;
- the issue repository (the profile's `issue_repository`).

If you were not dispatched by `research-workflow`, check that nobody else is preparing this concept: run `rw status --package "$PKG" --live`, which shows no run, lease or PR for `C` at S1.

## Steps

1. **Boot.**

   ```sh
   source /home/user/.local/share/plaza-tools/git-env.sh
   git fetch -q origin && git worktree add -b "$BRANCH" "$WT" origin/main && cd "$WT" && git push -u origin "$BRANCH"
   rw() { python3 -I scripts/research_workflow.py "$@"; }
   D="$(git rev-parse --git-common-dir)/agent-work/${PKG##*/}-S1"; mkdir -p "$D"; L="$D/$NAME.md"
   echo "$NAME $(date -u +%FT%TZ) start: S1 $C" >> "$L"
   rw lease acquire --package "$PKG" --stage S1 --resource "$C" --owner "$NAME" --purpose "S1 research-prepare"
   ```

   Renew the lease with the same command before 20 minutes pass, and append a heartbeat line at least every 10 minutes. The rest of the boot rule is collaboration rule 3. If a command is missing, read [tool-fallbacks.md](../research-workflow/references/tool-fallbacks.md).
   Done when `git ls-remote origin "$BRANCH"` prints a SHA, the log has `start`, and `rw lease list --package "$PKG"` shows your lease.

2. **Read the assignment.** Read:
   - the concept file and its state block (`type`, `kind`, `targets`, `segments`, `relations`);
   - its section in `main.md`;
   - the `G` section;
   - the reading register;
   - the segments in `package.json`.

   For `type: hint`, follow the hint path in [references/prepare-sections.md](references/prepare-sections.md).
   Done when your log has a three-line statement of the concept's goal, its targets and its segments.

3. **Put the exact excerpts at the top.** The stub already has intake's excerpts. Add every other segment that bears on this concept: `G` directives that constrain it, and segments held by other concepts that mention it. Most relevant goes first. Generate each block with the snippet in `references/prepare-sections.md`, never by typing it. Add those segment ids to your own state block.
   Done when `rw check --package "$PKG"` passes (it compares every excerpt with the transcript) and every segment in your state block has an excerpt.

4. **Map the repository.** For each target and its neighbours, cover:
   - code and config;
   - docs and decisions in `doc/` and `planning/`;
   - roadmap rows;
   - skills;
   - issues and PRs (`git grep -n`, `git log --oneline -- <path>`, `gh issue list --state all --search`, `gh pr list --state all --search`).

   For a wide concept, launch up to three read-only explorers in one turn with the briefs in [explore-briefs.md](../request-intake/references/explore-briefs.md). Write `## Repo map`.
   Done when every target is mapped with `file:line`, or marked "new file; nearest existing: <path>", and each overlapping issue or PR is listed.

5. **Do light web research.** Find up to about five primary sources that frame the questions: official docs or specs, and one or two projects that do the same thing. For each, record the URL as the tool returned it, its date, and one line on what it bears on. Follow the web-retrieval fallback in `AGENTS.md`. No synthesis and no verdict.
   Done when `## Leads for research` lists each source, or says "none needed: <reason>".

6. **Write the research questions and facets.**
   - `## Research questions`: each question can be answered, names what its answer would decide, and says where to look first. Together they cover feasibility, prior art, alternatives, use cases, failure modes, integration seams, cost, and what the concept enables.
   - `## Facets`: the separable aspects that S2 researches and that S5 later fans out into child issues. Give each a scope line and the questions it covers.

   Record any statement you found to be more nuanced: `rw item add --package "$PKG" --parent C003.2 --text "..."` adds a child item, and `rw item set-status --package "$PKG" --id C003.2 --status clarified --stage S1 --text "..."` updates its status. Use these on your own items only.
   Done when every question names its decision and its first source, and every facet maps to at least one question.

7. **Send notes; never edit another concept.** Some findings affect another concept: an overlap, a shared target, a contradiction, or a dependency that wasn't declared. For each, write the body to `$D/note-<to>.md`, then run `rw note --package "$PKG" --from "$C" --to C005 --input "$D/note-C005.md"`. Relations that start from your concept go in your own state block.
   Done when every cross-concept finding is a note file, or a line in your file saying why it needs none.

8. **Commit and open the PR.** In your state block:
   - set `stage` to `S1`;
   - append `{at, stage, status, note, ref}` to `history`;
   - append one line to `changes.md`.

   Then commit with explicit paths (`git add -- <your files>`), run `git fetch -q origin && git merge --no-edit origin/main`, and validate:

   ```sh
   rw check --package "$PKG" --changed-since origin/main
   prettier --check <your .md files>
   ```

   Push, then run `gh pr create --base main --title "docs: prepare $C <title> for research"`.
   Done when the PR is open and `git diff --name-only origin/main...HEAD` lists only files you own.

9. **Open the Research trigger issue (idempotent).** Search for the marker first, using the commands in [references/research-issue.md](references/research-issue.md). Reuse a matching issue; otherwise create it from the body there. The body links:
   - the concept file and the `main.md` section on `main`;
   - the transcript segments;
   - this PR.

   Add `{number, url, stage: "S1", state: "open"}` to `issues` in your state block, commit, and push.
   Done when exactly one issue carries the `(package, C, S1)` marker and your state block lists it.

10. **Finish.** Then:
    - append `push <sha>` to the log;
    - check that the PR head = `git ls-remote origin "$BRANCH"` = `git rev-parse HEAD`;
    - run `rw lease release --package "$PKG" --stage S1 --resource "$C"`;
    - append `end` to the log.

    Done when all four are true and the final message has the turn report.

## Boundaries (this is not the full research)

- Don't write answers, comparisons, recommendations or decisions.
- Don't create target files.
- Don't edit another concept's file or items, or the generated views.

If a question is easy to answer, leave the answer as a lead for S2. If the concept is too large for one S2 session, say so in your file and add a `part-of` split proposal as a note to the router. Don't split it yourself.

## Parallelism and ownership

You own only:

- your concept file;
- your outgoing notes;
- receipts for notes sent to you;
- your own items in `main.md`;
- one `changes.md` line per change;
- your log.

Other prepare workers run at the same time on other concepts. Wide files change only through your PR. The shared run table is the router's (`rw render`). The full rules are in [collaboration.md](../../../planning/workflow/collaboration.md).

## Should load / should not load

- Load for: preparing one concept for research (excerpts, repo map, questions, facets); opening a concept's Research trigger issue; being dispatched as an S1 worker.
- Don't load for: the full research of a concept (`concept-research`); capturing new input (`request-intake`); finding what's ready or dispatching waves (`research-workflow`); processing received notes (`notes-reconcile`); parent or child handoff issues (`issue-handoff`); general research with no package.

## References

- [references/prepare-sections.md](references/prepare-sections.md): read in steps 2, 3 and 6 (section order, excerpt snippet, good questions and facets, the hint path).
- [references/research-issue.md](references/research-issue.md): read in step 9.

## Turn report

```
Done: <C, sections written, n excerpts, n questions, n facets, n notes sent, issue URL, PR URL>
Verified: <rw check ... -> ok; triple SHA match <sha>; issue marker search -> 1 result>
Remaining: <S2 concept-research for C via the issue; notes awaiting receipts>
Questions (non-blocking): <question -> default used>, or none
Blocked on user: <only for a hint that needs the user's missing context>, or none
```
