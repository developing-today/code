# clan.lol and machine fleets

clan.lol is the estate's candidate tool for configuring fleets
of NixOS machines. What exists today is a scaffold: clan is
consumed as a flake input with exactly one toy machine defined.
Status: **in use as a scaffold; planned as a fleet system. No
clan-managed fleet exists.** This chapter is short because the
record is. Read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **The input is stale by the repo's own comment.**
  `clan-core.url = "https://git.clan.lol/clan/clan-core/archive/main.tar.gz"`,
  annotated in `flake.nix`: "TODO: update! way out of date even
  as of 2026-03".
- **`lib.make-clan`** (in `lib/default.nix`) calls the clan
  library with meta name "developing-today" and
  `machines = { user = { }; }` — **one machine, named `user`** (a
  second, `sara`, is commented out). Its inventory instances:
  sshd (server, all tags; host certificates disabled — the repo
  has no shared openssh-ca vars), user-root (the users module),
  zerotier (this machine as controller _and_ peer), and
  user-user. It exports `nixosConfigurations`, `clanInternals`
  and the clan config. A migration is already recorded in its
  comments: the instances are clanServices replacements for the
  clanModules imports that `modules/shared.nix` used to carry
  (clanModules was removed upstream; see #159).
- **That is the whole fleet.** There is no `machines/` directory
  in the repository — the census searched, and only `lib/clan.sh`
  and `lib/clan-installer-flash.sh` match the clan name outside
  docs. Those helper scripts cover the installer flow, and
  clan-cli ships in the dev shell (see
  [Dev shells as a default](dev-shells.md) — the root dev shells
  are defined inside this same make-clan output).

## The research record

`doc/nix-ecosystem-scan-2026-08.md` §6 records clan's current
form: the version scheme now tracks NixOS releases (26.05
stable), the inventory instances plus roles/tags model, `vars`
secrets generators (sops by default), the
`clan machines install|update|build` commands, and the layered
fallback transports — p2p-ssh over iroh, internet, wireguard,
zerotier, mycelium, yggdrasil, tor. The scan's relevance note:
this flake already consumes clan-core, and the iroh transport
pairs with the estate's iroh findings (see
[The pipes into Cloudflare and externally](pipes-to-cloudflare.md)).

## Where clan would bite — and the gap

The lab machines will run Talos, not NixOS (part 1's
[Talos and Cozystack](../cluster/talos-and-cozystack.md)), so
clan's plausible estate role is the NixOS machines — laptop-class
hosts and future NixOS boxes — not the cluster. **No record found
states that decision explicitly.** The census names the gaps
plainly: the clan-core pin is stale; there are no real machine
definitions; and no document explains what clan is _for_ in this
estate against the hand-rolled hosts builder of
[Nix and the NixOS flake](nix-and-the-flake.md).

## Sources

Platform census §8 and supplements S4, 2026-11-23, landed as
PR #13959.
