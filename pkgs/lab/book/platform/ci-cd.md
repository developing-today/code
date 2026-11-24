# CI/CD: Nix and GitHub Actions

Two systems meet in this estate's delivery pipeline: Nix builds
the artefacts, and GitHub Actions runs the builds. Status:
**in use and proven**. This chapter records the pinned pattern
that joins them, what CI does with Nix, and both CD legs — as
counted by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).
The Actions estate itself is in
[GitHub Actions](github-actions.md); the flake is in
[Nix and the NixOS flake](nix-and-the-flake.md).

## Actions using Nix — the house pattern

Actions-using-Nix is a pinned, repeated pattern (uses counted
2026-11-23):

- `DeterminateSystems/nix-installer-action` at a pinned commit
  (v23) — **32 uses**.
- `DeterminateSystems/magic-nix-cache-action` at a pinned commit
  (v15) — **24 uses**, typically configured
  `use-flakehub: false, use-gha-cache: true` (the
  `container-images.yml` instance is the type example) and often
  `continue-on-error: true`.
- `actions/checkout@v7` — 124 uses. Language setup actions
  (node, go, bun, python, deno) fill out the rest.

The rationale is recorded in the workflows themselves:
`id-check.yml`'s comments give the magic-nix-cache reason as "so
unchanged derivations are not rebuilt from cold", and record
Determinate Nix quirks the estate has hit (a shallow-repo
revCount refusal; a lazy-trees note, #705). The cache leg of this
pattern is the GitHub Actions cache — not the estate's own cache
worker; that tension is
[The Nix cache worker and its alternatives](nix-caches.md).

## What CI does with Nix

- `nix-ci.yml` — a nightly full `nix flake check` plus an
  advisory `host-eval`, explicitly **not** a merge gate (per
  `.github/ci-policy.md` and #3039).
- `deploy-flake-checks.yml` and `flake-inputs-update.yml`.
- `mcpx-nixbuild.yml`.
- `container-images.yml` — `nix build` of the container packages,
  publishing to GHCR on push to main; see
  [Containers and Docker images](containers-and-images.md).
- `id-check.yml` — the project check suite on the pattern above.

The governing policy is nightly-on-main: CI is post-merge
evidence, and merges never wait for it.

## CD from Nix

`lib.make-deploy` builds `pkgs/deploy` — a deploy-targets
registry with schema validation (the validation fixtures in
`lib/default.nix` exist to reject the exact malformed-registry
form of #6383) — targeting laptop and systemd services.
`pkgs/deploy` is the flake-native deploy leg.

## CD to Cloudflare

The 2026-11-04 deployment-architecture record is the baseline
audit, and this book presents it as a baseline, not as current
state. It found a split estate: the good pattern
(`just deploy-<name>` → flake app → `deploy.mjs`, the 2026-10-31
standard) covered exactly three workers (pkarr,
developing-today-backup, id-mail); other workers used raw
`wrangler deploy` or a manual REST PUT; **no GitHub Action
deployed any Worker** at that date; and id-mail had no rollback
path.

The current tree shows the follow-through in progress: many
worker publish, promote and deploy workflows now exist —
desktop-commander, id-mail, mcpx, message-bridge, pkarr,
pm-console-edge, queue-transport-alert, voice-line. The two
records are not the same statement, and the book does not merge
them.

## Releases

`goreleaser.yml` (goreleaser-action v7) and `mcpx-release.yml`
cover the binary-release leg; docs sites deploy via
`actions/deploy-pages@v5`.

## Gaps

- The Worker rollback and coverage gaps from the 2026-11-04
  record have no follow-up re-audit in any later record the
  census found. Whether every worker now follows the standard is
  **not established**.

## Sources

Platform census §5 and supplements S1 and S8, 2026-11-23, landed
as PR #13959.
