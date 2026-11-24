# Making Nix packages and apps

The estate's packages live in `pkgs/` — **90 entries** counted by
the platform census of 2026-11-23, including `default.nix` and
`shell.nix` — and the patterns they follow are consistent enough
to state, even though no single how-to document states them.
Status: **in use**. Census source:
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`
(PR [#13959](https://github.com/developing-today/code/pull/13959)).
How packages become runnable apps is
[NixOS flake apps](flake-apps.md); how they become images is
[Containers and Docker images](containers-and-images.md).

## What exists today — the patterns of record

- **Composition by `callPackage`.** `lib/default.nix` wires
  packages into the flake with `callPackage` — the type example
  is mcpx: `pkgs.callPackage ../pkgs/mcpx/package.nix { }`, with
  variants beside it (a container build calls the same package
  with `wrapWithRuntimes = false, doCheck = false`).
- **Per-package folders** holding `package.nix` or `default.nix`.
  Some packages carry their own sub-flake (`pkgs/id/flake.nix`,
  `pkgs/atlas-search/flake.nix`).
- **`buildNpmPackage` for npm-distributed CLIs**, with vendored
  lockfiles. `pkgs/cf` and `pkgs/wrangler` mirror each other,
  including the update ritual documented in the package comments:
  fix the hash from the build failure. `pkgs/cf` matters beyond
  packaging — it is the Cloudflare CLI direction of record in
  [DNS management](dns-management.md).
- **Rust via the `crane` flake input**, with dependency-layer
  splitting (#8392) and the id rust overlay; **Go** via the
  standard builders; **shell tools** via `writeShellScriptBin` —
  the same builder the generated flake apps use.
- **Test and guard packages are first-class citizens** of the
  tree: sigterm-witness, lane-overlap-check,
  opencode-config-normalize, and their kin sit in `pkgs/` beside
  the services.
- **Whole systems are package families here**: fleet itself
  (`pkgs/fleet` — a Rust workspace plus its book), the brokers'
  clients, and the PM stack.

## What the record plans

No packaging rework is recorded. New packages follow the
neighbouring package's pattern; new apps follow the justfile
contract in [NixOS flake apps](flake-apps.md).

## Gaps

- No single "how to add a package" document was found by the
  census; the patterns are consistent but learned from
  neighbours. This chapter is the first collected statement of
  them.
- The census flagged one unread file honestly rather than
  asserting over it: the exact role of `pkgs/default.nix` against
  the lib composition was not fully read.

## Sources

Platform census §6 and supplements S9, 2026-11-23, landed as
PR #13959.
