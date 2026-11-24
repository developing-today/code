# Dev shells as a default

Dev shells are the default way into this repository's tooling —
and the root shell lives in a surprising place. Status: **in
use**, on a thin record; this chapter is short because the record
is. Read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **The root flake's `devShells` are defined inside the
  `make-clan` output** in `lib/default.nix` — an historical
  accident of composition, visible in the source. "The clan
  output" and "the dev environment" are the same flake branch;
  [clan.lol and machine fleets](clan.md) tells the clan side.
- The shells are generated for four systems (x86_64-linux,
  aarch64-linux, aarch64-darwin, x86_64-darwin) from
  `nixpkgs-unstable` plus the id rust overlay, with unfree
  packages allowed and the Android SDK licence accepted — the
  shell carries the Android SDK through `nix-common.nix`, shared
  with `shell.nix` so `nix develop` and `nix-shell` resolve
  identical versions.
- The shell pins a **pristine official Bun 1.4.5 binary** instead
  of the nixpkgs patchelf repack, with the crash evidence
  recorded on #13337. clan-cli is among the shell's shared
  packages.
- **Sub-flakes carry their own shells**:
  `devShells.default = pkgs.mkShell { … }` in `pkgs/id`,
  `pkgs/atlas-search`, and the roc platform templates.

## What the record plans

Nothing specific: shells arrive with the projects that need them.

## Gaps

- There is no estate-wide devshell standard document, and no
  per-language shell catalogue.
- The census did not search for a `direnv` record, and this book
  asserts nothing either way about direnv.

## Sources

Platform census §7 and supplements S3, 2026-11-23, landed as
PR #13959.
