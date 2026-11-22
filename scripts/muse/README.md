# Bringing up a fresh Muse sandbox

`bootstrap.sh` rebuilds a Muse sandbox from nothing: the old one's tools,
config and laptop link come back, and the fleet binary is the current
static build with the `bridge` verb (see `doc/agents/fleet-command-channel.md`).

## What a person hands over

One secret only: the Bitwarden Secrets Manager access token. Everything
else (R2 keys, restic passwords, the tailnet auth key, GitHub login, the
bridge fleet token) is fetched by name from Bitwarden with it.

## Run it on the fresh box

```bash
curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/scripts/muse/bootstrap.sh -o bootstrap.sh
bash bootstrap.sh
```

It asks once for the token (hidden). To pipe it instead:
`printf '%s' "$TOKEN" | bash bootstrap.sh`. Re-running is safe; each phase
skips what is done. `bash bootstrap.sh --help` lists the phases;
`--only tools,token` or `--skip restore` pick a subset, `--all` also
restores `~/git` and `~/agents` (large, normally re-cloned on demand).

## After a sandbox restart

Only `/home/hatch` survives a recycle; `/`, `/etc` and every process are
gone. Run the script with `HOME=/home/hatch` (it warns otherwise) so the
tools, token, tailnet state and fleet all land there. The `persist` phase
saves a copy of the script as `~/tools/muse-bootstrap.sh`, writes
`~/tools/muse-boot.sh`, and hooks it into the top of `~/tools/init.sh`,
which already runs on every boot. Each boot then restarts `tailscaled`
from its saved state (no new auth key, same "muse" node) and keeps fleet
linked; it takes about a second when everything is already up. Log:
`~/logs/muse-boot.log`.

To add this to a box set up before the `persist` phase existed:

```bash
curl -fsSL https://raw.githubusercontent.com/developing-today/code/main/scripts/muse/bootstrap.sh -o bootstrap.sh
HOME=/home/hatch bash bootstrap.sh --only tools,tailnet,fleet,persist
```

Needs `curl`, `tar`, `python3` and `sha256sum` on an x86_64 Linux box.
Root or passwordless sudo is used when present (kernel tailnet, `init.sh`)
and not required otherwise (userspace tailnet).

## Where things come from

| Piece                                 | Source                                                                                                         |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `fleet`                               | GitHub release `fleet-static-<rev>` on this repo, asset `fleet-x86_64-linux-musl` (`nix build .#fleet-static`) |
| `bws`, `restic`, `tailscale`          | upstream releases, pinned by version and sha256 in the script                                                  |
| `~/.config`                           | restic repo `restic-sandbox-config` in R2                                                                      |
| `~/tools`, dotfiles, the rest of home | restic repo `restic-sandbox-home` (one snapshot per top-level entry; latest of each)                           |
| tailnet                               | `tailscale.auth-key`, joins as `muse`                                                                          |

## Publishing a new fleet build for Muse

On a host with Nix and `gh` logged in to this repo:

```bash
scripts/muse/publish-fleet.sh            # builds .#fleet-static at origin/main and uploads it
```

It prints the tag and sha256; update `FLEET_URL`/`FLEET_SHA256` in
`bootstrap.sh` in the same PR (or pass both as environment variables for
a one-off run).
