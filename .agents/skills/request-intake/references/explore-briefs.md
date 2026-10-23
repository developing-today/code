# Explore briefs (read-only context subagents)

Read this in step 4 of `request-intake` and in step 4 of `research-prepare`. Launch all briefs in one turn, so the explorers run in parallel. Explorers write no files. Their final message is the report.

## Common brief

```
Name: explore-<area>-<4 hex>     Role: read-only explorer
Objective: for each topic below, find what already exists in this repository and its issue tracker.
Repository: <checkout path> at origin/main (<sha>). Issue repository: <owner/repo from the profile>.
Topics: <one line per candidate concept: slug, title, the segment ids, 1-sentence summary>
Area: <code and config | docs and architecture | issues and PRs | <named area>>
Writes: nothing. No files anywhere; use pipes and shell variables.
Allowed commands: git grep -n, git log --oneline -- <path>, git show, ls, rg,
  gh issue list --state all --search, gh pr list --state all --search, gh issue view, gh pr view.
Done when: every topic has evidence or "nothing found" with the searches you ran.
Return, per topic:
  Topic: <slug>
  Evidence: - <path>:<line> — what it shows (exact quote of at most 25 words if quoting)
  Overlap:  - <path | #issue | #PR> — covers | contradicts | would be edited by | duplicates — one line
  Targets:  - <existing file this topic would edit>, if any
  Searched: <the searches>, <n of N results read>
```

## Areas

- **Code and config:** source trees, scripts, CI workflows, flake and module layout touched by the topics.
- **Docs and architecture:**
  - `doc/` (dated records), `planning/` (workflow, roadmaps, plans, designs);
  - `AGENTS.md`, `CLAUDE.md`, READMEs;
  - `.agents/skills/` and `.opencode/skills/` (a topic may already be a skill).
- **Issues and PRs:** open and closed issues and PRs whose title or body covers a topic. Include earlier research packages (`doc/*_research_*`) with overlapping concepts.
- **Named area:** one more explorer for each large subproject the topics touch (for example `pkgs/id`, `infra/`). That explorer reads the subproject's own `AGENTS.md` first.

## Using the reports

Before you rely on a report, re-run two of its claims. Then merge the overlaps into `reading.md`:

- An existing file a concept would change makes that concept `kind: edit`, with the file as a target.
- An existing issue or PR that covers a concept goes in that concept's thoughts ("Overlap: #123 already tracks ..."). The concept is kept, never dropped. Whether to merge them is decided at S2.
- A contradiction between the request and an existing doc becomes a thought on the concept, citing both the segment and the `path:line`.
