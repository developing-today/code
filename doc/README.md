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

- [Architecture index](../planning/architecture/README.md) — every architecture and design record in one place, plus the ADR index and template.
- [Next stage platform architecture](2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [Portable world surfaces](2026-10-14T19-39-30Z_architecture_portable_world_surfaces/2026-10-14T19-39-30Z_architecture_portable_world_surfaces.md)
- [Roc worlds architecture](2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
- [AGPL/GPL hosting: inspectable worlds, uploads, and self-hosting](2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting/2026-11-05T15-08-16Z_research_id5986_agpl_gpl_hosting.md)
- [Research workflow design](2026-10-19T06-09-02Z_design_research_workflow/2026-10-19T06-09-02Z_design_research_workflow.md)
- [Project-management stack design](2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md)
- [T3 Code V2 integration](2026-10-26T21-18-21Z_feature_t3code-v2-integration/main.md)
- [PM stack forecasting: velocity, quota walls, and stall risk](2026-11-01T21-07-00Z_design_pm-stack_forecasting/2026-11-01T21-07-00Z_design_pm-stack_forecasting.md)
- [Authenticated peer announcements and bounded discovery](2026-11-02T08-13-35Z_design_peer-announcement-authentication/2026-11-02T08-13-35Z_design_peer-announcement-authentication.md)
- [Graph-native currency event contracts](2026-11-07T03-55-49Z_feature_graph_currency_contracts/2026-11-07T03-55-49Z_feature_graph_currency_contracts.md) (first implementation slice for #5522; later v1 layers remain)
- [Self-hosting and interop stance](2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance/2026-11-08T09-12-27Z_design_id-self-hosting-interop-stance.md) (decision record for #5975: the interop contract, supported self-hosting shapes, and what is not required of a self-hoster)
- [Conversation archaeology: Second Life and programmable worlds, tied into the worlds program](2026-11-08T09-10-00Z_research_worlds_conversation_archaeology/2026-11-08T09-10-00Z_research_worlds_conversation_archaeology.md) (the #5955 tie-in record and C1–C4 rulings, landed for #5990)
- [Per-user function mapping](2026-11-08T10-26-00Z_design_id_per_user_function_mapping/2026-11-08T10-26-00Z_design_id_per_user_function_mapping.md) (design for #5967: the per-user function interface, the per-tick mapping algorithm with its failure table, boundary-only switching and triggers, human+AI-partner composition, and fuel-style fairness)
- [Server-side invariants: budgets, observability, exploit resistance, containment](2026-11-09T05-15-00Z_design_id_server_invariants/2026-11-09T05-15-00Z_design_id_server_invariants.md) (design for #5962: the budget set B1–B7 with enforcement classes, the trip-observability surface per audience, the platform attack-surface checklist over all 20 registered providers, and co-resident-world failure containment)
- [Two-level metering: burst bucket + hard per-tick ceiling, per function](2026-11-10T03-10-00Z_design_id_two_level_metering/2026-11-10T03-10-00Z_design_id_two_level_metering.md) (design for #6001: Screeps two-level metering mapped term-by-term to per-function fuel accounting — refill/bucket/ceiling as world-author configuration under server caps, the getUsed self-observation API, and the bucket-exhausted throttle outcome)
- [Atlas storage growth model](2026-11-08T11-18-02Z_design_atlas_storage_growth_model/2026-11-08T11-18-02Z_design_atlas_storage_growth_model.md) (design for #6061: per-source churn x retention growth model against the #6019 plan envelope, the ledger-fed calibration procedure, conservative v1 retention defaults for the #6026 rule engine, and staging modelled separately from the retained corpus)
- [Writer sets v2: view-as-object, set algebra, federation, discovery v2](2026-11-08T11-10-00Z_design_id_writer_sets_v2/2026-11-08T11-10-00Z_design_id_writer_sets_v2.md) (design for #5789: pinned/tracking view references, derived sets with provenance and exclusions winning, federation claim discipline via #5675/#5672, inherited uncounted-side honesty, and the three v1 open questions answered)
- [Reserved admin authority (LambdaMOO governance arc)](2026-11-08T12-50-00Z_design_id6004_lambdamoo_governance_arc/2026-11-08T12-50-00Z_design_id6004_lambdamoo_governance_arc.md) (design note for #6004: the explicit reserved-vs-delegable authority split for world governance, the Mr. Bungle capability-abuse lesson mapped to id's delegation and per-user-function surface, and quota as a governance tool)
- [Personalized search ranking for notes](2026-11-09T01-10-00Z_design_id_search_ranking/2026-11-09T01-10-00Z_design_id_search_ranking.md) (decision record for #5836: per-searcher tier ranking over #5675 filtered results - own-edit-first absolute by tier, explainability via reason codes, graffiti demotion without erasure, boundaries with #5732 recommendations and #5677 sovereignty)
- [A native Android client for ID](2026-11-08T22-06-00Z_design_id_android_client/2026-11-08T22-06-00Z_design_id_android_client.md) (design for #5780: thick local-first client, Keystore/StrongBox custody under #5684, offline journal + sync through the #3447 seams, shell contract with #1831 remote drive, phone as a two-gate #5686 provider, and honest presence with no guaranteed background wake)

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
