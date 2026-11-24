# Code Book

This book is the whole-project book for `developing-today/code`: the one place
that describes the project as a single system, ranks everything the project
intends to do next in one merged list, and tells the project's story so far
with its evidence attached. Every other book in the project documents one
system. This book documents the project those systems add up to.

The project, at the highest level, is a personal fleet platform built and run
by one engineer, Drew, with a large number of AI agents working through a
shared queue. Its parts, each with its own book, are:

- **fleet** — one multicall binary that carries the work queue, the secrets
  broker, remote build placement, agent enrolment and messaging.
- **Atlas** — the unified search system over the corpora Drew and his agents
  produce, with provenance on every result.
- **ID** — the identity and graph platform: content-addressed objects,
  peer-to-peer file sharing over Iroh, worlds running Roc guests under
  capabilities, and a hosted edition on Cloudflare.
- **mcpx** — the MCP gateway: every Model Context Protocol server behind one
  local daemon, driven by scripts instead of tool schemas.
- **Project Management (the PM stack)** — the aggregator, console, dispatch
  and notification services that make the project's own state visible and
  actionable.
- **The Lab** — the physical side: a basement fleet of about a dozen
  Threadripper machines in Osseo, Wisconsin, and the Talos/Cozystack cluster
  programme that will run on them.
- **The forks** — the project's maintained forks of OpenCode and OpenChamber,
  the agent harnesses the fleet's sessions run in.

## Why this book exists

Until this programme, the project's knowledge lived in the per-system books,
in 619 dated records under `doc/`, and in programme trackers on GitHub. A
reader who wanted the whole picture — what exists, what matters next, and how
it got this way — had to assemble it from those parts, and the parts disagreed
in places about dates and counts. This book is the assembly, done once, with
the disagreements stated rather than smoothed over:

- [The systems chapters](systems/fleet.md) condense each system's own book
  into a short, dated statement of what it is and where it stands.
- [The merged priority list](priorities/the-merged-list.md) is the single
  ranked list of the project's next work: 30 items in 7 tiers, merged from
  Drew's seed list on the programme tracker and the four programme
  investigations of 2026-11-23.
- [The retrospective](retrospective/the-story-so-far.md) tells the story the
  records support — and states plainly the long stretch, 2026-04-02 to
  2026-09-27, that no mined source covers. That gap is recorded as a gap,
  never padded with reconstruction.

The programme that commissioned this book is tracked at
[#13905](https://github.com/developing-today/code/issues/13905). Its censuses
are the book's sources: Census A (the retrospective), Census B (the merged
priority list) and Census C (the lab census), each landed in the repository as
a dated record under `doc/` and indexed from `doc/README.md`.

## How to read this book

- **New reader:** this chapter, then the systems chapters in order, then
  [The story so far](retrospective/the-story-so-far.md). You will know what
  the project is, what its parts do, and how it reached its current state.
- **Reader who wants what happens next:** [The merged priority
  list](priorities/the-merged-list.md), with [How priorities
  work](priorities/how-priorities-work.md) for the ordering doctrine behind
  it.
- **Reader checking a claim:** every status in this book is dated
  2026-11-23/24 and cites its tracker or record. Where this book and a living
  tracker disagree, the tracker is right and this book is stale — the sync
  rule in [Keeping this book current](development/keeping-this-book-current.md)
  says how the book is brought back into line.
- **Reader of one system only:** start from that system's chapter here for
  the short version, then follow its book link for the full treatment.

## The book map

One repository has one GitHub Pages site, and every book is staged into it
under its own subpath by the combined deploy (see [Deploying this
book](operations/deploying-this-book.md)).

| Book                    | Published at                                                                                 | Sources in this repo                        | Programme tracker                                               |
| ----------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------- |
| mcpx book               | [developing-today.github.io/code/](https://developing-today.github.io/code/) (the site root) | `pkgs/mcpx/docs/`                           | [#13264](https://github.com/developing-today/code/issues/13264) |
| Fleet book              | [developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/)     | `pkgs/fleet/book/`                          | [#13272](https://github.com/developing-today/code/issues/13272) |
| Atlas book              | [developing-today.github.io/code/atlas/](https://developing-today.github.io/code/atlas/)     | `pkgs/atlas-search/book/`                   | [#13241](https://github.com/developing-today/code/issues/13241) |
| ID book                 | [developing-today.github.io/code/id/](https://developing-today.github.io/code/id/)           | `pkgs/id/book/`                             | [#13240](https://github.com/developing-today/code/issues/13240) |
| This book (Code Book)   | [developing-today.github.io/code/code/](https://developing-today.github.io/code/code/)       | `pkgs/code-book/book/`                      | [#13905](https://github.com/developing-today/code/issues/13905) |
| Lab Book                | [developing-today.github.io/code/lab/](https://developing-today.github.io/code/lab/)         | built under the same programme as this book | [#13905](https://github.com/developing-today/code/issues/13905) |
| Project Management book | [developing-today.github.io/code/pm/](https://developing-today.github.io/code/pm/)           | `pm-stack/book/`                            | [#13900](https://github.com/developing-today/code/issues/13900) |

## Conventions used in this book

- **Statuses are dated.** The status vocabulary is the censuses': _done_,
  _in flight_, _designed_, _not started_ — each with its evidence. A status
  without a date in this book is a status as of the 2026-11-23/24 censuses.
- **Item numbers are stable.** The merged list's numbers (1–30) are used
  identically in every book's high-level roadmap chapter, so a reader can
  move between books without renumbering.
- **Secrets are named, never valued.** This book names systems, issues and
  records; it prints no credential values.
- **Source references are provenance.** Chapters cite the tracker, record or
  book each claim came from. The chapter text is complete without following
  them.
