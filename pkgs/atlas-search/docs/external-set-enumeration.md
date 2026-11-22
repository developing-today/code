# External-set enumeration — Atlas GitHub corpus (#6023)

Enumeration artifact for the external half of the Atlas GitHub
corpus. **Enumeration only: zero external comment text has been
extracted or ingested.** External extraction is gated on the
licensing research (#6024); this list is the set Drew can strike
items from before any external build.

Method (three sources, per #6023):

1. **Fork upstreams** — the `parent` of every fork in the
   `developing-today-forks` org, via the GitHub GraphQL API.
2. **Account activity** — repositories where `dezren39` authored
   or commented on issues/PRs, via the GitHub search API
   (`author:` / `commenter:` qualifiers, full 1,000-item result
   window per qualifier). Finding: within that window the
   account's activity is entirely inside the estate, so this
   source contributes no external candidates today; it is a
   candidate source, not an exhaustive history.
3. **Dependency manifests** — `github.com/<owner>/<repo>`
   references in `flake.lock` / `Cargo.lock` files across the
   local `developing-today/code` checkout.

Repos under our own accounts/orgs (`developing-today`,
`developing-today-forks`, `dezren39`) are excluded throughout —
those are the our-repos estate, extracted by the
`atlas-github-comments` feed.

**Union: 31 candidate external repos.**

## Fork upstreams (30)

- `BrunoKrugel/dotfiles`
- `Gako358/neovim`
- `Mic92/sops-nix`
- `NixOS/nixpkgs`
- `NvChad/NvChad`
- `alex35mil/dotfiles`
- `benwis/leptos-heavy-metal-stack`
- `charmbracelet/charm`
- `charmbracelet/log`
- `charmbracelet/promwish`
- `charmbracelet/ssh`
- `charmbracelet/wish`
- `cullenmcdermott/terraform-provider-porkbun`
- `jim3692/microvm.nix`
- `jim3692/server.nix`
- `jmacdonald/amp`
- `knadh/koanf`
- `libsql/sqlite-antlr4-parser`
- `m15a/flake-awesome-neovim-plugins`
- `nix-community/home-manager`
- `nix-community/nixvim`
- `nrdcg/porkbun`
- `openclaw/nix-openclaw`
- `rohoswagger/ez-stack`
- `shritesh/raytrace.roc`
- `srid/emanote`
- `tursodatabase/libsql-client-go`
- `usekaneo/kaneo`
- `vtemian/octto`
- `whitfin/runiq`

## Account activity (0)

- (none found)

## Dependency manifests (1)

- `rust-lang/crates.io-index`
