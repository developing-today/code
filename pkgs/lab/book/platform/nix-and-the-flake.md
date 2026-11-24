# Nix and the NixOS flake

Everything in this estate is built on one Nix flake: the code
repository's `flake.nix` and the roughly 1,940-line
`lib/default.nix` beneath it. Status: **in use** — it is the
repo's foundation, the source of its packages, its NixOS hosts,
its dev shells and its deploys. This chapter records how the
flake is put together, as read by the platform census of
2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **The root flake is a hand-rolled composition, not a literal
  flake-parts call.** The header comment naming
  `flake-parts.lib.mkFlake` is aspirational. The actual shape is
  `outputs = inputs: let lib = import ./lib inputs; in lib.merge [ … ]`
  — a fold over builder functions: `make-clan`, `make-root-apps`,
  `make-id`, `make-mcpx-overlay`, `make-mcpx-modules`,
  `make-pm-stack`, `make-archer-slack`,
  `make-laptop-backups-checks`, `make-fleet`, `make-deploy` and
  `make-vim`. `flake-parts` is a real input and is used by
  sub-flakes and by inputs, but the root composition is
  `lib/default.nix`'s own `merge`, a deep recursive-update fold
  over merged nixpkgs and home-manager libraries.
- **Hosts and configurations.** `flake.nix` exposes
  `hosts = import ./nixos/hosts inputs`, a VM guest registry
  (`vmGuests = import ./nixos/vm-guests` — see
  [Minimal VMs and KubeVirt](vms-and-kubevirt.md)), and
  `nixosConfigurations` built from the host configurations, the
  VM configurations and the unattended-installer configurations.
- **One nixpkgs pin for the fleet.** `nixpkgs` follows
  `nixpkgs-unstable`, which since 2026-11-02 follows the
  fleet-wide pin in dezren39/fleet (`fleet.url = "github:dezren39/fleet"`;
  `nixpkgs-unstable.follows = "fleet/nixpkgs"`), so the code,
  opencode and openchamber repositories build against the same
  nixpkgs revision and share one store closure. There are no
  master or stable-channel inputs — a directive of 2026-10-31
  (#3327) records why: master revisions skip Hydra and force
  source builds.
- **The flake's own Nix configuration** (`nixConfig` in
  `flake.nix`): substituters are `cache.nixos.org`,
  `yazi.cachix.org` and `hyprland.cachix.org`, with several more
  cachix caches present but commented out; a long
  experimental-features list (flakes, nix-command, ca-derivations,
  dynamic-derivations, fetch-closure, recursive-nix,
  pipe-operators, and more); `accept-flake-config = true`; trusted
  users root and `@wheel`. On the NixOS side,
  `nixos/nix/settings/default.nix` simply applies the flake's
  `nixConfig` as `nix.settings`. Which caches those substituters
  are — and the proven cache this list omits — is the subject of
  [The Nix cache worker and its alternatives](nix-caches.md).
- **The standing ecosystem survey** is
  `doc/nix-ecosystem-scan-2026-08.md`: release cadence, nixpkgs
  structure, module changes, CLI and store, cache alternatives,
  and clan. It is a watch-brief, not a decision record — see
  [Nix apps V2 and next-gen tooling](next-gen-nix-tooling.md).

## What the record plans

The builder-function composition is the settled pattern; the
record plans no replacement for it. Sub-flakes (id, atlas-search)
carry their own outputs beside the root composition.

## Gaps

- The root flake's composition style is undocumented as a pattern:
  no "how this flake is put together" document was found by the
  census. This chapter is the first prose record of it.
- Several inputs carry "TODO: use this?" or "way out of date"
  comments — clan-core among them; see
  [clan.lol and machine fleets](clan.md).

## Sources

Platform census §2 and supplements S2–S3, 2026-11-23, landed as
PR #13959.
