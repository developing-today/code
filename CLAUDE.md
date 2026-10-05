# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

This is Drewry Pope's personal "everything" monorepo. The core is a NixOS flake that builds
his machines, but the repo also carries several largely-independent subprojects (a Rust P2P
CLI, Kubernetes/Talos manifests, Terraform-managed DNS, personal docs/notes). Treat each area
as its own context — changes to `pkgs/id` have nothing to do with the NixOS modules, etc.

Note: `nixos_bootstrap_unattended-installer_offline_nixos-24.11.20241014...iso` (~15GB) and
`amd.qcow2`/`nixos.qcow2` VM disk images live in the repo root — never read, copy, or diff these.

## Commands

### Formatting / linting
- `just fmt` (or `just treefmt`) — runs `treefmt` using the root `treefmt.toml`, which dispatches
  per-language formatters: `nixfmt` + `statix fix` for `*.nix`, `biome` for JS/TS/CSS/JSON,
  `prettier` for md/html/yaml/scss, `shfmt` for shell, `rustfmt`, `taplo` for TOML, `ruff` for
  Python, `gofmt`, etc. (see `treefmt.toml` for the full formatter list and exclusions).
- `just fix` — strips trailing whitespace, regenerates `just-recipes.json`, then runs `just fmt`.
- `lefthook` runs `treefmt --fail-on-change` on staged files pre-commit, and `conform` on the
  commit message (commit-msg must follow Conventional Commits; `WIP:`/`wip:`/`fixup!`/`squash!`
  are exempt — see `.conform.yaml`).
- `deadnix` (checking only — intentionally not auto-fixed; it risks deleting deliberately unused
  bindings used for documentation/future work).

### Flake input management (all via `just`, see `root.just`)
- `just update-inputs-all` — `nix flake update` (all inputs).
- `just update-input <name...>` — update specific flake inputs by name only.
- `just update-nixpkgs` / `update-nixpkgs-master` / `update-nixpkgs-unstable` — update
  nixpkgs-derived inputs by branch, discovered from `flake.lock` via `nix/nixpkgs-inputs.nix`.

### Rebuilding an actual NixOS host
`./rebuild`, `./rebuild-simple`, `./rebuild-offline` (symlinks into `lib/auth-rebuild-*.sh`) run
`nixos-rebuild switch --flake .` with sudo, optionally updating every flake under `pkgs/*` first
(`rebuild-full`) and committing a `<hostname> <generation>` marker commit afterward. **These only
make sense on one of the actual target machines** (`nixos`, `amd` — see `nixos/hosts/default.nix`)
and require root/hardware access; do not run them in a sandboxed dev container, and don't run them
without being asked — they mutate the live system and create commits.

### `pkgs/id` (standalone Rust subproject)
Has its own flake, justfile, CI, and `AGENTS.md`/`ARCHITECTURE.md`. Read `pkgs/id/AGENTS.md`
before working there — it has its own rules (e.g. never delete `rust-toolchain.toml`, keep
justfile recipes and `nix run .#<cmd>` apps in sync, add deps only via `nix-common.nix`). Use
`nix develop` inside `pkgs/id` for the Rust toolchain.

## Architecture (NixOS flake core)

`flake.nix` doesn't hand-assemble the usual `nixosConfigurations` by itself. Instead:

```
outputs = inputs: lib.merge [
  { hosts = import ./nixos/hosts inputs; configurations = lib.make-nixos-configurations hosts; ... }
  lib.make-vim
  lib.make-clan
  lib.make-root-apps
  lib.make-id
]
```

`lib = import ./lib inputs` (`lib/default.nix`) is a custom DSL layered on `nixpkgs.lib` +
`home-manager.lib`. The important pieces:

- **Hosts are declared declaratively** in `nixos/hosts/default.nix` as calls to
  `host { profiles = ...; hardware = ...; disks = ...; users = ...; wireless = ...; }`.
  Currently two real machines: `nixos` and `amd`.
- Each of those attributes is a **string or list of strings that resolves to a directory under a
  fixed base path**, via helpers in `lib/default.nix`:
  - `profiles` → `nixos/<name>` (e.g. `profiles = "desktop"` imports `nixos/desktop`)
  - `hardware` → `nixos/hardware-configuration/<name>`
  - `users` → `nixos/users/<name>`
  - `disks` → `nixos/disks/<name>` (disko-based disk layout)
  - `wireless` → `nixos/networking/wireless/<name>`
  So adding a machine capability is usually "add a directory under the matching `nixos/<category>/`
  tree and reference its name from `nixos/hosts/default.nix`," not writing new plumbing.
- `keys/` holds `ssh-host-*.pub` / `ssh-group-*.pub` / `ssh-user-*.pub` files, read via
  `lib.host-key` / `lib.group-key` / `lib.user-key` and wired into host/user configs (e.g. for
  sops-nix). Secrets themselves live under `sops/` and `secrets/` (see `.sops.yaml`).
- Most flake inputs pin to `nixpkgs-unstable` (aliased as the main `nixpkgs` input) because of a
  `neededForBoot` patch (`patches/nixpkgs/neededforboot-nixos-unstable.patch`) that only applies
  cleanly to the unstable tree — don't re-follow individual inputs back to `nixpkgs-stable`/`-25`
  without checking that patch still applies.
- Root-level single-letter/word symlinks (`rebuild`, `auth`, `nvim`, `n`, `v`, `format-nix`, ...)
  are convenience aliases into `lib/*.sh`; `lib/` here is shell scripts, distinct from the Nix
  `lib/default.nix` DSL described above.

## Other areas

- `apps/` and `infra/` (`infra/talos`, `infra/dns`) are Kubernetes/Talos manifests and
  Terraform-managed DNS — independent of the Nix flake, operated by their own scripts
  (`infra/dns/*.sh` wrap `terraform plan/apply`).
- `doc/` holds dated design docs named `doc/<UTC_RFC_DATETIME>_<kind>_<name>/`
  (`<kind>` ∈ feature/architecture/refactor/design/component/reference, etc.). Follow
  `doc/DOCUMENTATION_PROTOCOL.md` when a change is significant enough to warrant one: write the
  doc first with intent + plan, then append dated `## <datetime> <Type>:` sections as the work
  evolves. `just stage-docs` / `just stage-docs-dry` (wrapping `scripts/stage-docs.sh`) stage
  new/modified files under `doc/` without ever staging deletions.
- `AGENTS.md` at the repo root defines a **web-retrieval fallback protocol**: on a 403/429/bot-block
  when fetching public web content, retry with alternate User-Agent strings in a defined order
  before giving up, record findings (successes and failures) in `ai-crawler-site-access-table.md`,
  and never use UA spoofing to debug real HTTP/auth behavior or to bypass access controls. Follow
  it whenever using `curl`/`wget`/HTTP clients against public sites.
