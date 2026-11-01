# Planning

This folder holds the plans for the code repository at three levels of detail.

| Level   | Question it answers                     | Where                                                      | How it changes                                  |
| ------- | --------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------- |
| Roadmap | Where is this going, and in what order? | [`ROADMAP.md`](../ROADMAP.md) and [`roadmaps/`](roadmaps/) | Edited in place, kept short                     |
| Design  | What will it be, and why this way?      | [`doc/`](../doc/)                                          | Dated records, appended to while work is active |
| Plan    | How do we build it, one step at a time? | [`plans/`](plans/)                                         | Updated as each step lands                      |

The roadmap is the top level. An item can sit there as one line with no details, long before anyone knows how to build it. A design is written when the shape of the thing has to be decided. A plan is written when the work is ready to start.

## Roadmap

The repo holds several projects that move on their own schedules, so each has its own roadmap file. [`ROADMAP.md`](../ROADMAP.md) at the repo root is the overall view and links to them.

| Project                                        | File                                                           | Code                       |
| ---------------------------------------------- | -------------------------------------------------------------- | -------------------------- |
| `id`, the peer-to-peer CLI and worlds platform | [`roadmaps/id.md`](roadmaps/id.md)                             | `pkgs/id`                  |
| `mcpx`, the MCP gateway and Code Mode runner   | [`roadmaps/mcpx.md`](roadmaps/mcpx.md)                         | `pkgs/mcpx`                |
| The NixOS flake and the infra manifests        | [`roadmaps/nixos-and-infra.md`](roadmaps/nixos-and-infra.md)   | `nixos/`, `lib/`, `infra/` |
| Repo-wide CI and automation                    | [`roadmaps/repo-and-ci.md`](roadmaps/repo-and-ci.md)           | `.github/`                 |
| The basement home cluster (Talos, Cozystack)   | [`roadmaps/basement-cluster.md`](roadmaps/basement-cluster.md) | `infra/talos/`, `doc/`     |

Each roadmap file groups its items by horizon.

| Horizon | Meaning                                                                       |
| ------- | ----------------------------------------------------------------------------- |
| Now     | Being worked on, or first in line. Has an issue.                              |
| Next    | The direction is chosen and the shape is mostly known. Starts after Now.      |
| Later   | The direction is chosen, the shape is not. Details are allowed to be missing. |
| Ideas   | Written down so it is not lost. No commitment, and it may be dropped.         |

### Milestones and iterations

For roadmap work, the fleet does not use GitHub milestones or Project iterations.
Horizon (the Project field and the [#899 priority ladder](https://github.com/developing-today/code/issues/899)), together with Project start and target dates, is the planning mechanism; do not create milestones for roadmap work. Revisit this decision if a release train needs a named cut.

The existing open milestone in `dezren39/nix` is an exception, not a precedent for roadmap tracking; this decision does not change it.

Each item has a status.

| Status        | Meaning                                                               |
| ------------- | --------------------------------------------------------------------- |
| `idea`        | Someone wrote it down. Details may be missing.                        |
| `proposed`    | Described with a reason. Waits for a decision to go ahead.            |
| `shaped`      | The direction is decided. A design record exists or is being written. |
| `planned`     | A plan with steps exists and the issues are filed.                    |
| `in-progress` | At least one PR for it is open or merged.                             |
| `shipped`     | Done. The row moves to the "Done" table with the date and the PR.     |
| `dropped`     | Decided against. The row moves to the "Done" table with the reason.   |

A roadmap row records intent. It is not a go-ahead to build the thing, and it does not settle a product question. Wherever this repo says a maintainer decides, a row at `proposed` or earlier still waits for that decision.

## How to use it

1. Add a new idea as one row under Ideas. One sentence of intent is enough.
2. Change a row's horizon or status in the same PR as the work that causes the change.
3. Link every row to its issues. When a design or plan exists, link that too.
4. When a row ships or is dropped, move it to the "Done" table. Do not delete rows, since the history is useful.
5. Open an issue for every item in Now, so the work can be picked up and tracked there. The roadmap links to issues and does not copy their content.

## Templates

- [Roadmap item](templates/roadmap-item.md)
- [Design record](templates/design.md)
- [Implementation plan](templates/plan.md)

## Where existing material lives

- **Design records** stay in [`doc/`](../doc/), one folder per record, named `<UTC datetime>_<kind>_<name>`. [`doc/DOCUMENTATION_PROTOCOL.md`](../doc/DOCUMENTATION_PROTOCOL.md) says when to write one and how to append to it. [`designs/`](designs/) only explains this and points there, so there is a single home for designs.
- **Implementation plans** are written in [`plans/`](plans/) from now on. The plans written before this folder existed stay in [`.opencode/plans/`](../.opencode/plans/), and [`plans/README.md`](plans/README.md) lists them. They are not moved, because other documents link to them by path.
- **Project notes** that already exist keep their place: `pkgs/id/TODO.md`, `pkgs/id/ARCHITECTURE.md`, `pkgs/mcpx/TODO.md` and `pkgs/mcpx/docs/`. A roadmap row links to them when it draws on them.

## Related repositories

`mcpx` is consumed by `dezren39/nix` (the macOS flake), `developing-today/opencode` and `developing-today/openchamber`. This repo is the primary source for `mcpx`. Changes to it land here first.
