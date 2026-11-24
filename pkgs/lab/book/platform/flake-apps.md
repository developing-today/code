# NixOS flake apps

The flake's app surface — the commands a user runs as
`nix run .#<recipe-name>` — is mostly not written by hand. It is
generated from the repository's justfile. Status: **in use**.
This chapter records the mechanism as read by the platform census
of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **Root apps are generated from the justfile.**
  `lib.make-root-apps` in `lib/default.nix` reads
  `just-recipes.json` (produced by `just just-recipes`), filters
  out private recipes and the `default` recipe, and emits one
  flake app per public recipe. The comment of record states the
  whole contract: to add a new Nix app, add a recipe to the root
  justfile, run `just lockfiles`, and the app appears
  automatically as `nix run .#<recipe-name>`. Each app is a
  `writeShellScriptBin` wrapper that executes `just <name>`, with
  shared metadata (homepage, MIT/ASL20 licences).
- **Hand-written apps exist alongside the generated set** —
  further `apps` blocks in `lib/default.nix` serve the project
  outputs (mcpx, deploy tooling, and the per-system maps at the
  make-id and make-deploy layers), and sub-flakes carry their own
  apps (`pkgs/id/flake.nix`).
- **Deploy apps are the Cloudflare Worker deploy standard.** The
  chain `just deploy-<name>` → flake app → `deploy.mjs` was the
  recorded standard for exactly three workers as of the
  2026-11-04 deployment-architecture record (pkarr,
  developing-today-backup, id-mail). How that standard has since
  spread through workflows is covered in
  [CI/CD: Nix and GitHub Actions](ci-cd.md).

## What the record plans

No change to the generation mechanism is recorded. New apps
arrive by adding justfile recipes, per the contract above; the
next generation of Nix app tooling is a research question, not a
plan — see
[Nix apps V2 and next-gen tooling](next-gen-nix-tooling.md).

## Gaps

- No catalogue document lists the current app surface. The
  generated-from-justfile mechanism is discoverable only from the
  comment in `lib/default.nix`; this chapter is its first prose
  record outside the source.

## Sources

Platform census §3 and supplements S3, 2026-11-23, landed as
PR #13959.
