# Roadmap: NixOS flake and infra

This file covers the NixOS flake (`flake.nix`, `lib/`, `nixos/`, `keys/`, `sops/`) and the manifests that run beside it (`infra/talos`, `infra/dns`, `apps/`).

It is the thinnest roadmap in the repo. The open issues I read (the 30 most recently updated) hold no NixOS or infra work, and the daily Jules flake sync keeps inputs current. Items below come from the repo's own notes, so most of them are ideas that still need an owner's decision. How to read this file: [planning README](../README.md).

## Constraints that shape the work

- Hosts are declared in `nixos/hosts/default.nix` and each attribute names a directory under `nixos/`. Adding a capability usually means adding a directory and naming it there.
- Most inputs follow `nixpkgs-unstable`, because the `neededForBoot` patch in `patches/nixpkgs/` applies only to that tree. Do not re-follow inputs to a stable channel without checking the patch.
- The two real hosts are `nixos` and `amd`. `rebuild` scripts change a live machine and run only on those.

## Now

| Title                              | Status      | Intent                                                    | Links                                                |
| ---------------------------------- | ----------- | --------------------------------------------------------- | ---------------------------------------------------- |
| Keep flake inputs current          | in-progress | A daily Jules job updates inputs and audits dependencies. | `.github/workflows/jules-daily-flake-sync.yml`, #287 |
| mcpx as a package for other flakes | in-progress | The macOS flake needs a way to build mcpx from this repo. | #308, [mcpx roadmap](mcpx.md)                        |

## Next

| Title                                  | Status   | Intent                                                                                                    | Links    |
| -------------------------------------- | -------- | --------------------------------------------------------------------------------------------------------- | -------- |
| Name an owner for NixOS and infra work | proposed | The roadmap has no items of its own. Decide what the next host, service or cluster change is and file it. | none yet |

## Later and ideas

These come from scratch notes in `TODO.md` and `doc/todo.md`. They are listed so they are not lost, and each needs a decision first.

| Title                                    | Status | Intent                                                                                                                                                               | Links                            |
| ---------------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- |
| Impermanence review                      | idea   | The root `TODO.md` holds a commented impermanence setup (persisted machine-id, ssh host keys, rollback and sudo lectures). Decide what applies to `nixos` and `amd`. | [TODO.md](../../TODO.md)         |
| Automatic upgrades                       | idea   | A note points at the NixOS manual section on automatic upgrades.                                                                                                     | [TODO.md](../../TODO.md)         |
| Wireless backend                         | idea   | Compare iwd and wpa_supplicant.                                                                                                                                      | [TODO.md](../../TODO.md)         |
| Content-addressed derivations by default | idea   | A note links the NixOS call for testers.                                                                                                                             | [doc/TODO.md](../../doc/TODO.md) |
| Mail service on Cloudflare Workers       | idea   | `nixos/services/id-mail` deploys a Worker through `idMailDeploy`. Its direction is not written down.                                                                 | `nixos/services/id-mail`         |
| Infra scripts to a documented workflow   | idea   | `infra/dns` is driven by many small shell wrappers around Terraform. A short runbook would help whoever runs them next.                                              | `infra/dns`                      |

## Done

Completed work is recorded in GitHub — see closed issues and merged PRs.
