# Laptop state backup set — inventory (developing-today/code#10298)

The set definition is `laptop-state-backup.sh` (restic, daily ~05:40,
`laptop-backups.nix`). Verified 2026-11-17 by names, paths and sizes only.
No secret contents were read, printed or archived to produce this note.

## Backed-up sources (in the set)

| Path                               | Disposition                                                                                                                                                                  |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `~/.local/state/opencode`          | backed up directly                                                                                                                                                           |
| `~/.local/state/opencode-claimant` | backed up directly (added #10298). Kept: `mode`, config, `fires.jsonl`, probe/poll/shadow JSON history. Excluded: per-run `*.out` / `*.err` logs (disposable, most of ~11M). |
| `~/secrets/*.sops.yaml`            | backed up directly (ciphertext only)                                                                                                                                         |
| `~/.ssh`                           | backed up directly                                                                                                                                                           |

## Rendered and local files in `~/pm-stack` and `~/secrets`

Disposition: **direct** = in the set above; **sops** = rebuildable by
`deploy.sh` / `install.sh` from `~/secrets/pm-stack.sops.yaml`; **vault** =
recoverable from the Bitwarden entry named; **repo** = in git; **gap** = no
recovery source found.

| File                                                                              | Disposition                          | Recovery source (name only)                                                                          |
| --------------------------------------------------------------------------------- | ------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `pm-stack/tunnel.token`                                                           | vault                                | `cloudflare.tunnel.amd-6-pm-stack-token` (verified present; also sops key `cloudflare_tunnel_token`) |
| `pm-stack/tunnel.env`                                                             | sops                                 | rendered from `secrets/pm-stack.sops.yaml` (`cloudflare_tunnel_token`)                               |
| `pm-stack/.env`                                                                   | sops                                 | rendered by `deploy.sh` from `secrets/pm-stack.sops.yaml`                                            |
| `pm-stack/grist/.env`                                                             | sops                                 | `secrets/pm-stack.sops.yaml` (`grist` keys); Bitwarden `grist.*`                                     |
| `pm-stack/pad/.env`                                                               | sops                                 | `secrets/pm-stack.sops.yaml`; Bitwarden `pad.*`                                                      |
| `pm-stack/dispatch/.env`                                                          | vault + sops                         | `DISPATCH_API_TOKEN` from `pm-dispatch.api-token`; GitHub App key from `secrets/pm-stack.sops.yaml`  |
| `secrets/dispatch.env` (+ two `.bak-*`)                                           | vault                                | `pm-dispatch.api-token` (README: restore from vault before `install.sh`)                             |
| `pm-stack/retired-2026-10-27/aggregator-deployed-copy/.env`                       | retired                              | not restored; no recovery needed (retired tree)                                                      |
| `pm-stack/dispatch/` source, `compose.yml`, `deploy.sh`, `pad/`, `grist/` scripts | repo                                 | in git (`developing-today/code`)                                                                     |
| `pm-stack/ready-for-agent/repos/code.git` (bare mirror)                           | rebuildable (**untested inference**) | `git clone --mirror https://github.com/developing-today/code.git`; not yet tested per audit          |
| `pm-stack/aggregator-backup-*.tar.gz`, `grist/*.grist.bak-*`                      | not in set                           | one-off archives; explicitly unbacked, low value; recorded as known gap                              |
| `pm-stack/__pycache__`, `dispatch/node_modules`, `deploy.log`                     | regenerable                          | none needed                                                                                          |

Docker data stores (Kaneo Postgres, Grist sqlite, Pad sqlite + attachments

- keys) are covered by `pm-stack-backup.sh` (`~/backups/pm-stack/`, not the
  restic set).

## Gaps (flagged, not fixed)

1. **sops age key** `~/.config/sops/age/keys.txt` (75 bytes) is in **no backup
   set**. The `*.sops.yaml` ciphertext is backed up, but without this key it
   cannot be decrypted. Adding it is a decision (it is a private key); Drew
   to decide. Vault copies of most values mitigate this.
2. **Bare mirror rebuild** is an untested inference, per the 2026-11-14 audit.
3. **Snapshot evidence** (acceptance 1): not yet produced. The installed unit
   runs the nix-store copy of the script, so the new source appears only
   after the flake is rebuilt and the next daily run completes.

## Projector boundary (unchanged)

`projector.db` is mostly unbacked by design; `pmp_links` is exported daily.
This note restates that boundary and does not change it. (Not located by name
under `~` at this check; the boundary is taken from issue #10298.)

## Scope discipline

No vault entries were created, no secrets rotated, and no values read.
Names above were checked with `bws secret list`, printing key names only.
