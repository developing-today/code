# OpenCode and OpenChamber: our forks programme

This part of the book covers the two forks the fleet runs its agent work on: `developing-today-forks/opencode`, the coding agent, and `developing-today-forks/openchamber`, the agentic development environment built on it. It records what the forks are, why the fleet carries them, what the fleet has changed in them, what the fleet intends to change next, and how the open work on each fork is ordered. It is a programme record, not a user guide: the unit throughout is the fork, the change, and the issue.

The scale of the programme, counted in the survey this part rests on (survey date 2026-11-23; GitHub API read 2026-11-24T00:35–01:05Z, fully paginated): the opencode fork carries 1,258 merged pull requests and 30 open issues; the openchamber fork carries 1,186 merged pull requests and 30 open issues; the programme's open set in the `developing-today/code` repository, defined by title match in the survey, is 91 issues. The counts are stated in full in [Our changes, shipped and in flight](our-changes.md), and the open issues are listed in full in the two issue maps.

## Why this part exists

The forks are load-bearing. The fleet's sessions run in the fork builds, the fleet's model routes and quota machinery live in the fork code, and the control surface the fleet operates from is the second fork. A change to either fork changes what every session does. The rest of this book treats the forks as part of the ground: the Project Management stack's claimant drives the fork server, and the plan and route chapters describe pools that the fork implements. This part is where the forks themselves are the subject.

This part is not the plan record. Which plans the fleet holds, which pools and routes the deployed configuration names, and what the walls and resets are sit in the LLM plans part of this book — start at [Pools, routes and the switchboard](../llm-plans/pools-and-routes.md) and [The plans we use](../llm-plans/the-plans.md). Where a fork change exists to serve a plan or a route, this part records the fork change and links across; it does not restate the plan.

## How to read this part

- [The forks: what they are and why we carry them](the-forks.md) — the two repositories, the two-step fork chain, the tracking policy for each fork, the purpose the programme's own epics state, and the divergences from upstream: the ones the fork READMEs claim, and the ones the repository record shows on its own.
- [Our changes, shipped and in flight](our-changes.md) — the merged pull requests of both forks, grouped by theme under the survey's grouping rule, with shipped work and in-flight work kept in separate sections. Shipped means merged. Intention is never counted as shipped.
- [Goals and intentions](goals-and-intentions.md) — what the programme is trying to make true, each goal with its issue link, including the closed landmark items the survey marks as closed.
- [OpenCode: prioritised issue map](opencode-issue-map.md) — every open issue on the opencode fork (30), in the order the map's stated ordering rule produces.
- [OpenChamber: prioritised issue map](openchamber-issue-map.md) — every open issue on the openchamber fork (30), under the same rule.
- [The forks programme dependency map](dependency-map.md) — the dependency edges the survey states, each with its source, and the non-edges the survey records, recorded as non-edges.

## The lift-out note

This part is written to be lifted out. It has its own framing, its own opening chapter, and its own sources, and it depends on the rest of this book only through explicit cross-links of the kind in the paragraph above. When the forks programme gets its own book, this part moves as a unit: the seven chapters, their tables, and their snapshot dates move together, and the cross-links back into this book become cross-book links. Until that move, the part lives here because the programme and the Project Management stack share one repository, one queue, and one operator.

## Conventions used in this part

- **Counts are counted.** Every count in this part comes from the survey's fully paginated API read of 2026-11-24T00:35–01:05Z. No count is estimated, and no count from a search-result page is used where the survey gives a counted figure.
- **The maps are dated.** Issue states rot. Each issue map states the date its map was drawn (2026-11-23) in its opening lines, and an issue's presence in a map is a statement about that read, not about today.
- **Claims are labelled as claims.** What a fork README asserts is recorded as the README's claim. What the repository record shows is recorded as shown. Where the two differ, the chapter states both and cites the disagreement by number (D50–D58 in this part).
- **Shipped and intention stay separate.** Merged pull requests are shipped. Open issues and open pull requests are in flight. A goal is an intention until its issues close.
- **Placeholders are escaped.** A value you supply is written as &lt;name&gt;.

Sources. Survey file `~/workspace/pm-book/survey/oc-programme.md` (OC-SURVEY, survey date 2026-11-23; API data read 2026-11-24T00:35–01:05Z via `fleet gh api`); raw material deposit `~/workspace/pm-book/sideline-material/oc-survey/`; book shape from `~/workspace/pm-book/skeleton.md` (the liftable section, own SUMMARY part).
