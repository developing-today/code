# Project-management stack: layered tools, thin glue, one aggregator

Canonical record: `doc/2026-10-24T11-05-38Z_design_project-management-stack/`
in the `developing-today/code` repository. This Pad item is the
planning-layer copy; the repo document wins on any disagreement.

## Intent

Project management at two levels across the four working
repositories (`developing-today/code`, `developing-today/opencode`,
`developing-today/openchamber`, `dezren39/nix`): (1) a curated
work-in-progress board — features and fixes actually in flight, not
every open issue; and (2) a higher-level roadmap. The decision is a
composition of single-purpose tools plus a thin layer of our own
glue and UI, rather than any one all-in-one product.

Repository topology rule: **everything for this stack lives in the
code repository** — design, deployment, glue, UI. The only work
that belongs elsewhere is a fork of an upstream project. Secrets
are SOPS-encrypted in the repo and mirrored to Bitwarden +
1Password.

## Layers and ownership

Each layer has exactly one source of truth; nothing competes.

| Layer           | Tool             | Owns                                          |
| --------------- | ---------------- | --------------------------------------------- |
| Code work       | GitHub           | Issues, PRs, comments, reviews across repos   |
| Execution board | Kaneo (MIT)      | Per-repo task boards, two-way GitHub sync     |
| Dispatch        | ready-for-agent  | Turning a labelled issue into an agent run+PR |
| Planning        | Pad (Apache-2.0) | Plans, conventions, playbooks, ideas          |
| Tables          | Grist or Baserow | Roadmap, idea register, inventories, trackers |
| Glue            | ours (code repo) | Link registry + projections (never a SoT)     |
| Aggregator UI   | ours (code repo) | Cross-repo WIP board + roadmap timeline       |

Licenses verified from source 2026-10-24: Kaneo MIT;
ready-for-agent MIT; Pad Apache-2.0; Grist Apache-2.0; Baserow
core MIT (the tables-layer choice between them is Phase 2's).
Rejected on license alone: Plane,
Focalboard, Leantime (AGPL), Planka, NocoDB (non-OSS licenses).

## Data flow

GitHub (issues/PRs/comments) ↔ Kaneo projects (one per repo, via
our GitHub App) → projector (ours) joins by ID into a link registry
(issue URL ↔ Kaneo task ↔ Pad note ↔ table row) → aggregator UI
renders the two views. The glue links and projects; it never
becomes a fifth source of truth. If the UI rots, every layer
underneath still works standalone.

## The two views (aggregator UI, final phase)

1. **Cross-repo WIP board** — the curated in-flight set across all
   four repos, with an execution strip (implementing / awaiting
   review / merged).
2. **Roadmap view** — the table layer's roadmap as a timeline,
   each row drilling down to its GitHub issues and live state.

## A day in the life (target state)

An idea matures in Pad → becomes a roadmap row → spawns issues in
the right repo → they appear on that repo's Kaneo board
automatically → an issue is labelled ready → ready-for-agent (or
the sweep fleet) implements it → the PR walks the Kaneo card to
in-review → merge closes the issue and ticks the roadmap row
forward.
