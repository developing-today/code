# Phase 2 — plan (dispatch, planning, roadmap layers)

Status: **in progress**, started 2026-10-24. Scope per the
coordinator brief; all definitions land in `pm-stack/` in the
`developing-today/code` repo, same patterns as Phase 1
(containers, SOPS secrets mirrored to Bitwarden + 1Password,
laptop host, loopback/tailnet-only unless a webhook needs a
tunnel hostname).

## Deliverables

1. **ready-for-agent** (MIT) deployed on the laptop in the Phase 1
   style, configured for `developing-today/code` with **manual
   merge policy only** (auto-merge off). Trial: one deliberately
   trivial throwaway issue labelled `ready-for-agent`, run the
   Implement flow, verify a PR appears with exactly the trivial
   diff and no auto-merge. PR left open for review.
2. **Pad** (Apache-2.0) — this instance. Container on the laptop,
   SQLite data in a named volume, loopback + tailnet only.
   Workspace `developing-today` with the startup-template
   collections: Tasks, Ideas, Plans, Docs, Conventions,
   Playbooks. Seeded with the stack design and the Phase 1/2
   plans (this item).
3. **Roadmap table layer** — Grist vs Baserow evaluated in the
   design record (license, self-host footprint, API quality,
   exportability); the winner deployed as a container and seeded
   with real rows: PM-stack phases, basement-cluster register
   headlines, the four-repo fleet.
4. **Docs**: design record refreshed (Phase 1 as-built corrected,
   Phase 2 as-built appended), runbook updated, and the
   aggregator UI scoped as the next phase — not built here.

## Remaining after Phase 2

- Connect the other repos (opencode, openchamber, nix) as their
  own Kaneo projects with the app installed per repo.
- The projector + link registry (Turso) and the aggregator UI
  with the two views (cross-repo WIP board, roadmap timeline).
