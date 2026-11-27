# Release and rollout

Fleet has three distinct release motions, and confusing them is the classic rollout mistake:

1. **Publish** — build the static binary and upload it as a GitHub release asset (`scripts/muse/publish-fleet.sh`).
2. **Bootstrap pin** — point fresh-seat provisioning at that asset (`scripts/muse/bootstrap.sh`).
3. **Deploy** — roll broker services on the laptop through generation-based profiles (`pkgs/deploy/deploy.py`), and update the two live installs of the binary itself.

The governing rule, from the fleet README's Deploy section: **a verb is not shipped when it merges — it is shipped when both live installs report it.** That rule exists because of a real failure (#6728): `fleet queue fallback` was merged as source and docs while the installed binaries answered `repo-queue: unknown command "fallback"` for a full day.

## 1. Publish: `scripts/muse/publish-fleet.sh`

The script builds the static fleet binary and uploads it as a GitHub release asset on `developing-today/code`, then prints the tag and sha256 to pin in `bootstrap.sh`.

Contract: at most one positional argument, `REV` (default `origin/main`); no flags; `-h` / `--help` prints the header; anything else exits 2 before any subprocess runs (the operator-script argument contract: a script declares its whole command line and parses strictly against that declaration before any subprocess, network, file write, lock, or deploy step).

Steps:

1. Fetch `origin main` and resolve the revision. The tag is derived from it: `short` is the 7-character revision prefix and `tag = "fleet-static-$short"`.
2. Create a throwaway detached git worktree at that revision (removed by an exit trap) and build inside it:

   ```sh
   nix build "$work/src#fleet-static"
   ```

   Building in a detached worktree, not the primary clone, keeps the primary on `main` and clean.

3. Gate the artefact, twice:
   - **Static-linkage gate** — `file -L` on the binary must report `statically linked` or `static-pie`. A non-static build is refused.
   - **Bridge-capability gate** — the binary's `capabilities` output, checked programmatically, must show a domain named `bridge` with `available` true. The Muse seat's bootstrap requires the bridge-capable full profile; a build without it is fatal.
4. Copy the binary to the asset name `fleet-x86_64-linux-musl` and write its checksum file `fleet-x86_64-linux-musl.sha256` (`<sum>  <asset>`).
5. Publish idempotently: if the release `fleet-static-<short>` already exists, download its existing `.sha256` and re-upload with `--clobber` only when the checksum differs; otherwise create the release as a prerelease targeted at the revision, with notes naming the build command and the bootstrap consumer.
6. Read the two output lines — they are the pin values for the next step:

   ```text
   tag=fleet-static-<short>
   sha256=<sum>
   ```

**Automated cadence (#14999).** Publishing no longer waits for an operator to remember the script: `.github/workflows/fleet-static-release.yml` runs it from GitHub Actions on a daily schedule (05:41 UTC) plus `workflow_dispatch`, so the newest fetchable release tracks current main. A run first resolves its target revision (the dispatch input, else `origin/main`) and checks whether `fleet-static-<short>` already exists with both assets; if it does, the run ends without building. Otherwise it installs Nix the way `fleet-smoke` does and runs `publish-fleet.sh` for that revision, retrying once on failure — the script is idempotent, and the retry covers the transient cache rate limits of #14859. Publishing a release does not move the bootstrap pin; the pin still moves by hand (section 2).

## 2. The bootstrap pin: `scripts/muse/bootstrap.sh`

`bootstrap.sh` rebuilds a Muse sandbox seat from nothing — tools, config, laptop link — with every phase idempotent and skipping work already done. It runs in **8 phases**: `tools token restore tailnet fleet persist init check`, selectable with `--only <list>` / `--skip <list>`; `--all` also restores `~/git` and `~/agents`. Exit 0 means ok, exit 1 names the failed phase, exit 2 is a usage error.

The fleet pin is a pair of lines near the top of the script, under the comment "pinned artifacts (update together; sha256 of the downloaded file)":

```sh
FLEET_URL="${FLEET_URL:-https://github.com/developing-today/code/releases/download/fleet-static-<short>/fleet-x86_64-linux-musl}"
FLEET_SHA256="${FLEET_SHA256:-<sha256 of the downloaded file>}"
```

Both are environment-overridable. The URL is exactly the publish step's tag scheme plus the asset name. To move the pin:

1. Run `publish-fleet.sh` for the revision you intend to ship and note its `tag=` and `sha256=` lines.
2. Edit both `FLEET_URL` (the tag segment) and `FLEET_SHA256` in `bootstrap.sh`, together, in the same change. Updating one without the other makes every fresh bootstrap fail its checksum.
3. Commit and merge through the normal worktree-and-pull-request flow.

How the pin is consumed: in the `tools` phase, a sha256-verifying `fetch` downloads the pinned binary into `$HOME/.local/share/muse-bootstrap/fleet/fleet` — kept under the plain name `fleet` in its own off-PATH directory, because fleet picks its verb set from argv0 — and only when the existing copy is missing or fails the same `has_bridge` capability probe publish used. `fetch` dies on a sha256 mismatch. In the `fleet` phase, that download becomes the seat's install at `~/tools/fleet`: if the existing file is not writable it is moved aside to `~/tools/fleet.old`; the new binary is installed atomically (`install -m 755` to `~/tools/fleet.tmp`, then `mv -f` over `~/tools/fleet`) and only when the bytes differ; then `~/tools/bin/fleet` and `~/tools/bin/repo-queue` are symlinked to `~/tools/fleet` — the `repo-queue` link is the argv0 alias doing the work, not a second binary.

The other phases, briefly: `token` obtains the seat's Bitwarden Secrets Manager access token; `restore` restores the seat's files and provisions the shared broker token files by name from Bitwarden Secrets Manager; `tailnet` joins the tailnet and writes the `[host]` config into `~/.config/fleet/fleet.toml`; `persist` saves a boot hook that re-runs `--only tailnet,fleet` after a sandbox recycle; `init` brings up the seat's init stack (see [Running the brokers as services](../operations/services.md)); `check` verifies the install — `fleet version` and `capabilities` including the bridge capability, laptop-link reachability the way fleet dials it, secret resolution by name, and a bridge hello/inbox probe as the seat's agent.

## 3. Deploy the brokers: `deploy.py` profiles and generations

Broker services on the laptop are deployed with `pkgs/deploy/deploy.py` — a registry-validating deployment CLI (stdlib-only Python). Service definitions live in `pkgs/deploy/services.nix`; the registry the CLI consumes is baked into its package at build time. A valid definition carries exactly the fields `package, entryPoint, args, environment, secrets, user, ports, restart, healthCheck, targets` (plus optional `healthTimeoutSeconds` and `timer`), and a `class` — for the laptop brokers, `systemd`. Secrets in a definition are **names only**, resolved by the secrets broker at activation time; a definition never carries a value.

The three fleet profiles:

| Profile          | Package / entry point             | Args                   | Port                                             | Health check                                                                                                        |
| ---------------- | --------------------------------- | ---------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| `queue-broker`   | `fleet`, `bin/fleet`              | `serve queue-broker`   | 4801 (`PORT=4801`, `QUEUE_BROKER_BACKEND=turso`) | `GET /healthz` on the laptop tailnet address, port 4801                                                             |
| `secrets-broker` | `fleet`, `bin/fleet`              | `serve secrets-broker` | 4800 (`PORT=4800`)                               | `GET /health` on the laptop tailnet address, port 4800                                                              |
| `laptop-agent`   | `laptopAgent`, `bin/laptop-agent` | none                   | 4799 (`LAPTOP_AGENT_PORT=4799`)                  | none — `/health` requires the daemon token, so health is verified through the authenticated client after activation |

`queue-broker` names one secret by name (the Turso fleet-queue read-write token); `secrets-broker` names none, because it _is_ the resolver. `laptop-agent` restarts `always` rather than `on-failure`: a clean SIGTERM/success exit does not restart under `on-failure`, and a past daemon-death episode showed that gap is real.

### Profiles, generations, and the generation log

Each target owns a per-service Nix profile. The system-scope default base is `/nix/var/nix/profiles/per-service`; sandbox targets default to `~/.local/share/deploy/profiles` (home survives a sandbox recycle; `/etc` does not). `--profile-base` / `$DEPLOY_PROFILE_BASE` relocates the base.

The rendered systemd unit's `ExecStart` points at the profile's **`current`** link (`<base>/<name>/current/<entryPoint>`), never at a bare store path — so a generation flip is a profile repoint, not a unit rewrite, and the unit file itself does not change between generations. A flip is one atomic symlink repoint at the generation directory (`<base>/.generations/<name>/<id>/`).

Every deploy and rollback appends to an **append-only JSONL generation log**, one file per target at `<state-dir>/generations/<name>.jsonl` (state dir via `--state-dir` / `$DEPLOY_STATE_DIR`). Each event records the generation id, source revision, artefact, timestamp, and actor. The log is never truncated; the current and previous generations are derived by _folding_ it in order — a `deploy` event makes its generation current, a `rollback` event makes the generation it repointed to current, and previous is the last distinct generation before current.

### Activation and rollback

1. Preview with no side effects:

   ```sh
   deploy <target> --dry-run
   ```

   This prints the ordered activation plan, which is the oracle for what a real activation will do.

2. Activate. The real run, in plan order: build → install the next generation into the profile → write the unit → daemon-reload → restart **only that unit** → health-check (the definition's `healthCheck` URL, within `healthTimeoutSeconds`). On any failure, the profile flips back to the previous generation and the unit restarts on it.
3. Roll back when a generation is bad. **Rollback is a repoint, never a rebuild:** repoint the profile at the previous distinct generation → restart → health-check → on failure, flip back to the generation you came from → append a `rollback` event to the log. Rollback refuses when no generations are recorded or only one is ("rollback needs a previous generation"). `deploy status` folds the log and reports current and previous with their source revisions plus health.

## 4. The two live installs of the binary

Separate from `deploy.py`, the fleet binary itself has two live installs, and both must move for a change to count as shipped:

1. Build **one** artefact from current `main` and record its version:

   ```sh
   nix build .#fleet
   ./result/bin/fleet version
   ```

   Both installs must match that build's source revision.

2. **Laptop install** — a plain binary copy over the laptop's `~/.local/bin/fleet`. Verify in place.
3. **Sandbox install** — `~/tools/fleet`, reached through the `~/tools/bin/fleet` and `~/tools/bin/repo-queue` symlinks. This install is root-owned provisioning, read-only to lanes: a lane that cannot redeploy it files or updates the tracking issue instead of working around it. In practice it moves through the bootstrap path of section 2.

One warning from the same README section: a vendor-step failure naming an atlas crate means `pkgs/fleet/Cargo.lock` has drifted from the atlas-search path dependencies. `bash .github/scripts/check-fleet-lock.sh` names the drift in seconds; regenerate with `(cd pkgs/fleet && cargo update --workspace)` before building. The same check runs in CI — see [Testing](testing.md).

## 5. Seat rollout order and verification

Roll a new binary out to seats in this order:

1. Publish the static artefact and move the bootstrap pin (sections 1–2), so any seat rebuilt from scratch lands on the new build.
2. Deploy the laptop brokers through `deploy.py` if the change touches a broker role, and verify each profile's health check.
3. Update the laptop install (`~/.local/bin/fleet`), then the sandbox install (`~/tools/fleet`).
4. Verify **per seat**, by surface, not by version string alone:

   ```sh
   fleet version
   fleet capabilities
   fleet queue --help
   ```

   `fleet version` on each seat must report the same source revision as the artefact you built, and the verb surface you shipped must be present — for a new verb, run the verb's own `--help` on each seat. A seat still answering `unknown command` for a merged verb is not rolled out, whatever its version line says.
