# Bringing up a fresh Muse sandbox

`bootstrap.sh` rebuilds a Muse sandbox from nothing: the old one's tools,
config and laptop link come back, and the fleet binary is the current
static build with the `bridge` verb (see `doc/agents/fleet-command-channel.md`).

## Credentials for a new lane

For a new Muse lane, use the [one-time handoff](handoff/README.md). The
generator prompts privately for both the BWS access token and the 1Password
service-account token, then prints a 15-minute single-use URL. The URL carries
only an opaque claim code; the credentials are encrypted in the Worker and
returned once over HTTPS. Never paste the credential values into the agent
prompt.

The standalone legacy bootstrap still accepts a BWS token by hidden prompt or
stdin. It also accepts `OP_SERVICE_ACCOUNT_TOKEN`, stores it in
`~/.config/op/service-account-token` with mode 0600, and restores
`op` from the existing tool backup.

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

Publishing is also automated (#14999):
`.github/workflows/fleet-static-release.yml` runs the same script from
GitHub Actions on a daily schedule (05:41 UTC) and on
`workflow_dispatch`, skipping the build when the target revision's
release already exists — so the newest fetchable release tracks
current main without an operator remembering the manual step.

## Keeping a seat's fleet binary current

```bash
scripts/muse/fleet-binary-keeper.sh              # restore on drift
scripts/muse/fleet-binary-keeper.sh --check-only # report drift only
```

The keeper compares the installed binary's sha256 (default target
`~/tools/fleet`) against the `.sha256` asset of the newest
`fleet-static-*` release. On drift it downloads the release asset,
verifies it against the published sum, and only then replaces the
target; a failed verification never touches the installed binary.
Every run appends one line to
`~/.local/state/fleet/fleet-binary-keeper.log` (matches included), so
a keeper that silently stopped running is visible in the log's
timestamps. Run it every 15 minutes from cron or a systemd timer —
that is the alignment-standard cadence, and it is what makes a
bootstrap pin reinstalling a stale binary at boot a 15-minute
condition instead of a permanent one.

## Enrolling another Muse lane

Use [handoff/README.md](handoff/README.md) to deploy the single-use handoff
Worker and mint a link for the new lane. The copy-ready agent prompt is
[handoff/PROMPT.md](handoff/PROMPT.md). Report failures and script repairs in
[feedback/](feedback/README.md), then send the resulting branch or PR and
non-secret diagnostics back through the fleet bridge.
