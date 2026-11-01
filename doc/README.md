# Documentation map

This page says where each kind of document lives in this repository and how it changes. [`planning/DOCUMENTATION_ARCHITECTURE.md`](../planning/DOCUMENTATION_ARCHITECTURE.md) gives the placement rules and their reasons. [`DOCUMENTATION_PROTOCOL.md`](DOCUMENTATION_PROTOCOL.md) says when to write a dated record and how to add to it. These are defaults: an exception is fine when the PR that makes it says why.

## Where things live

- **Repository root.** Entry points, kept short:
  - [`README.md`](../README.md): licence and contribution terms;
  - [`AGENTS.md`](../AGENTS.md): the rulebook every agent session loads;
  - [`CLAUDE.md`](../CLAUDE.md): Claude Code's guide to the flake and the commands;
  - [`ROADMAP.md`](../ROADMAP.md): priorities across projects.
- **`doc/<UTC>_<kind>_<name>/`.** Dated records: one folder per design, architecture, feature, fix or phase. A record says what was known when it was written. While the work is active, new sections are appended to it. It is not rewritten later.
- **`doc/<UTC>_research_<slug>/`.** Research packages. Each holds an exact request transcript, a main reference, a review order, a change log and one file per concept. `request-intake` creates them. [`planning/workflow/records.md`](../planning/workflow/records.md) defines the format.
- **[`doc/references/`](references/README.md).** Shared standards and reference material that more than one record cites.
- **[`planning/`](../planning/README.md).** Maintained control documents:
  - roadmaps;
  - implementation plans ([`planning/plans/`](../planning/plans/README.md));
  - templates;
  - the [dependency vocabulary](../planning/DEPENDENCIES.md);
  - the [workflow library](../planning/workflow/README.md).

  They change in place, through PRs.

- **Package docs.** A component's living docs stay with its code and change with it. Examples: [`pkgs/id/ARCHITECTURE.md`](../pkgs/id/ARCHITECTURE.md), [`pkgs/id/AGENTS.md`](../pkgs/id/AGENTS.md), [`pkgs/id/TODO.md`](../pkgs/id/TODO.md), [`pkgs/mcpx/docs/`](../pkgs/mcpx/docs/) and [`pkgs/mcpx/TODO.md`](../pkgs/mcpx/TODO.md).
- **Agent skills.** `.agents/skills/` holds the workflow lifecycle skills and the shared skills. `.opencode/skills/` holds the OpenCode skill set.

## Research packages

This list is the workflow's request catalog (`request_catalog` in [`.agents/research-workflow.json`](../.agents/research-workflow.json)). Add one line per new package. For a package's current state, run `python3 -I scripts/research_workflow.py status --package doc/<folder>`.

- [Research intake to implementation](2026-10-19T06-09-02Z_research_research-intake-to-implementation/main.md). Created 2026-10-19 from the request that defined this workflow. It has 14 concepts. The design record is [research workflow](2026-10-19T06-09-02Z_design_research_workflow/2026-10-19T06-09-02Z_design_research_workflow.md).

## Dated records

Folders are named `<UTC datetime>_<kind>_<name>`. The kinds in use are `feature`, `design`, `architecture`, `phase`, `fix`, `reference` and `research`. Run `ls doc/` to list them all. Some starting points:

- [Next stage platform architecture](2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [Portable world surfaces](2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md)
- [Roc worlds architecture](2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- [Research workflow design](2026-10-19T06-09-02Z_design_research_workflow/2026-10-19T06-09-02Z_design_research_workflow.md)
- [Project-management stack design](2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md)
- [PM stack forecasting: velocity, quota walls, and stall risk](2026-11-01T21-07-00Z_design_pm-stack_forecasting/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md)
- [T3 Code V2 integration](2026-10-26T21-18-21Z_feature_t3code-v2-integration/main.md)

Two names break the pattern. They stay as they are because other documents link to them: `2026-03-23T00:00:00Z_feature_file_list_ui/` has colons in its name, and `2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md` is a file, not a folder.

## Legacy places

These stay where they are, because scripts and other documents use their paths:

- `docs/` at the root is a symlink to `doc/`. The protocol's older examples say `docs/`, and both names reach the same files. The root `documents` is also a link to `doc/`.
- `doc/hardware` is a symlink to a `hardware-doc` checkout outside this repository. `doc/archive` and `doc/scratch` point into it, and so do the root `archive` and `scratch`. Where that checkout is missing, the links are broken.
- `.opencode/plans/` holds the plans written before `planning/plans/` existed. [`planning/plans/README.md`](../planning/plans/README.md) lists them.
- Undated notes in `doc/` are personal notes and hardware references. Examples: `openwrt.md`, `ventoy.md`, `xcp-ng.md`, `network-topology.md`, `talos-addresses.md`, `todo.md`, `TODO.md`, `doc/notes/`, `doc/inventory/`, the `database_structure*` diagrams and the PDFs. A new note that records a decision gets a dated folder instead. A source that several records cite goes in `doc/references/`.
- Notes at the root: `TODO.md`, `TODO`, `stuff.todo`, `thoughts/`, `inspiration/` and `ANDROID-CONNECTIVITY.md`. [`ai-crawler-site-access-table.md`](../ai-crawler-site-access-table.md) is maintained, and `AGENTS.md` routes agents to it.

## Legacy notes

These are the unsorted notes that were in this file before it became the map. They are kept as they were:

flake.nix
-> hosts
hosts/<host>/(<user>)
-> home
-> secrets/sops/<host>/(<user>)
home/<user>/<host>
-> secrets/sops/<user>/(<host>)
lib/.sops.yaml
-> secrets/sops/
-> secrets/sops/<host>/(<user>)
-> secrets/sops/<user>/(<host>)
secrets/sops/<host>/(<user>)
secrets/sops/<user>/(<host>)

- take apart configuration.nix
  - first make all the parts into files and imports in configuration.nix
  - then move the files into common,global,hosts,home,etc.

difference between lib and module?
difference between created module and configured module?
difference between lib and pkgs?
difference between pkgs and modules?
do overlays need a root folder?
how to best handle many-to-many relationships?
if hostname is unique, how to handle template? shell script to make <name>\_<random>?
if hostname is not unique, how to handle discovery/dns/networking/vpn/ssh?
setup persistence, disko, iso-installer by default
setup vpn (tailscale for now, later also wireguard)
setup some kind of monitoring
setup ci/cd/build-farm
setup service discovery (vpn names and gokrazy/caddy??)
finish network setup, flash routers(update firmware, install os (r7-router or openwrt?)), build rails, rack up switches, plug it all in, ensure latest firmware/onie for switches
