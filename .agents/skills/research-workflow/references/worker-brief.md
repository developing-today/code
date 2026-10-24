# Worker brief

Read this before every dispatch. A worker starts with no context, so the brief has to carry everything it needs. Fill in every `<...>`. Pass paths, URLs and SHAs. Never paste summaries of documents the worker can read itself (collaboration rule 9).

Names: `<stage-skill short>-<concept or unit>-<4 hex>`, for example `prepare-c003-7f2a`. Branch: `research/<slug>/<stage>-<unit>`, for example `research/docs-pipeline/s1-c003`. Worktree: `<worktree_root>/research-<slug>-<stage>-<unit>`.

```
Name: <name>            Role: worker for <S1> (<research-prepare>)
Objective: <one end state, e.g. "C003 is prepared for full research: excerpts at the top, repo map,
  research questions and facets, Research trigger issue open, PR open into main">
Load the skill: .agents/skills/<skill>/SKILL.md, and follow its steps in order.

Repository: <absolute path of a checkout>   Base: origin/<trunk>
Branch: <branch>   Worktree: <worktree path>   (create it, push the empty branch first)
Package: <PKG>   Concept file: <PKG>/concepts/<C003-slug>.md
Main reference section: <PKG>/main.md#<anchor>   Transcript segments: <T007-T009, T041>
Issues and PRs so far: <URLs, or none>

Owns (writes): <concept file>; notes it sends (notes/<C003>__to__*); receipts for notes addressed to
  <C003>; its own items in main.md via `rw item`; one changes.md line per change; its log file.
Must not touch: other concept files, other concepts' items, generated views (`rw render` is the
  router's), AGENTS.md, doc/README.md, anything outside <PKG> unless listed above.
Rules: planning/workflow/collaboration.md (all ten rules). No stash, no rebase, no force push,
  no reset, no sudo; stage explicit paths; merge origin/<trunk> into the branch before opening the PR.
Lease: held for you as <resource> by the router. Renew it with the same `rw lease acquire` command
  before 20 minutes pass, and release it with `rw lease release` when you finish.
Coordination log: <git common dir>/agent-work/<package>-<stage>/<name>.md
  Append lines "<name> <UTC> <kind>: <text>". Write "start" first, a heartbeat at least every 10 minutes,
  "push <sha>" after every push, and "end" last.

Done when: <the skill's last step criterion, e.g. "PR open; its head = git ls-remote = git rev-parse HEAD;
  rw check --package <PKG> --changed-since origin/<trunk> exits 0">
Stop as blocked when: a prerequisite is missing, a file you must change is owned by someone else, or a
  check fails for a reason outside your files. Push what you have, say exactly what blocks you, stop.
Return: branch, PR URL, last pushed SHA, verified commands with their real output, what you did not
  verify, questions with the default you used, changes needed in files you don't own (as `rw note`
  files or handoff text).
```

Before relying on a worker's report, check:

- the files it wrote, with `git diff --name-only origin/main...<branch>`;
- the claims you will repeat, by re-running one or two of its commands.

A report is a secondary source.
