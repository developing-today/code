# Facet research subagents (S2)

The concept owner fans out read-only subagents, one per facet or per cluster of small facets, and stays the only writer of the concept document. This file is the brief and the merge procedure. It adapts cairn's `research-capture/references/brief-template.md` and `agents/researcher.md`.

## Shape of the fan-out

- **Scopes are disjoint.** No facet belongs to two subagents. Each brief lists the facets to skip because a sibling owns them.
- **Read-only.** Subagents need no worktree, branch or lease. They never write in any repository checkout. They return evidence in their final message, or write exactly one evidence file in the scratch directory when the brief names one.
- **Leaves.** Subagents do not launch subagents of their own.
- **Parallel.** Launch them all at once. If the harness runs fewer at a time than there are facets, run them in rounds, largest facets first.
- **Sized.** Each subagent's scope fits one fresh session. A facet too large for one session is split before launch.

## Scratch directory

Create one scratch directory per run outside the repository (for example `mktemp -d`). It holds the brief, the evidence files and any fetched-page cache. Nothing in it is committed. The research log in the concept document names each subagent, its scope and what it returned.

## Brief

Write the brief once and point every subagent at it. Give each its scope line in the launch message.

```markdown
# <package>·C001 <concept title>: facet evidence

The concept owner writes the concept document; you collect evidence only.
Concept document: <full GitHub URL or repository path>. Transcript segments: <ids>.
Main reference section: <link>. Research questions from S1: <ids and text>.

## Your scope

- Facets you own: <C001.N title, its questions>.
- Facets you skip (a sibling owns them): <list>.
- Preferred sources: official docs and specs first, then builders' write-ups and papers, then practitioners' repositories, issues and forum threads, then anecdotes.
- Angles: how at least two projects or harnesses handle it; best practice; advanced techniques; failure reports and critiques; example projects; user anecdotes.

## Boundaries

- Read-only. Allowed: reading files, `git log/show/grep`, read-only `gh api` GETs, web search and fetch.
- Never write in the repository, push, open issues or PRs, or read secret values. Do not launch subagents.
- Write nothing, or exactly <scratch>/<prefix>.md if named here. No helper or temporary files.

## Return

1. One line per source: `S | key | title | publisher | date or undated | exact returned URL | type`
   (type: docs, spec, paper, repo, issue, blog, forum, talk, anecdote). Keys start with `<prefix>-`.
2. At least one line per source: `C | key | "exact quote of at most 25 words" | claim it supports`.
3. Repository facts: `R | path:line | sha | command | what it shows`.
4. Per facet: findings, pitfalls, open questions, and a verdict with confidence.
5. Conflicts between sources.
6. What you searched for and did not find, with the queries.
7. Each unsourced idea in scope: what it is, `For: Inference:`, `Against: Inference:`, `Apply/test:`, `Nearest:`.

Every count gives n of N and its scope. Copy URLs exactly as a tool returned them; never build one.
```

## Known failures to guard against

From cairn's research logs; each happened.

1. Subagents told in prose not to write files wrote helper files anyway. Grant only read tools where the harness allows, and list the scratch directory and `git status` afterwards.
2. A sample was reported as the whole population. Require "n of N" on every count.
3. A quote was applied to the wrong claim. Require the heading check, and re-check it yourself.
4. An error was relayed from a subagent's notes. Re-measure before repeating.

## Merging the results

1. Collect every `S`, `C` and `R` line. Merge keys that share an exact URL.
2. Re-fetch each cited page and match each quote exactly (step 5 of the skill). Drop or re-source any quote that does not match.
3. Re-run the repository commands at your SHA.
4. Assign `[n]` numbers by first use in the document.
5. Write the facet sections yourself. Copy no subagent prose verbatim without checking it.
6. Record each subagent, its scope and its result in the research log, including what it could not find.
