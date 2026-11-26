# Cargo target broker

A bounded, leased pool of warm cargo target dirs for fleet lanes on
the laptop. Fixes the half of code#6797 that sccache (code#3448) does
not: sccache caches compiler outputs, but every worktree still
materializes a full target dir from them, and hundreds of worktrees
fill `/home`. Sharing one `CARGO_TARGET_DIR` across worktrees was
tried and rejected (code#3448): alternating/concurrent use thrashes
cargo fingerprints and workspace crates recompile on every switch.

The broker keeps the sharing and removes the thrash with lease
discipline — the pattern the fleet already uses for secrets and
dispatch: a bounded pool, exclusive leases, no concurrent writers.

## How it works

- Pool dirs live under `~/build-cache/broker-pool/`, keyed by
  `(repo, profile label, rustc version + host triple)`. Dependency
  artifacts are checkout-independent, so a pooled dir stays warm
  across _different worktrees of the same repo_; only workspace
  member crates are path-sensitive (measured split is on code#6797).
- `checkout` grants an **exclusive lease** on a warm pooled dir (or
  creates one). A concurrent request for the same key gets a second
  pooled dir — up to the per-key cap (default 3) — or waits; it never
  shares a live dir. Before returning the lease, the broker cleans the
  current workspace members' outputs from that target dir, preventing
  checkout-sensitive artifacts from leaking between worktrees while
  retaining dependency artifacts.
- `checkin` releases the lease and refreshes the dir's recorded
  size. The dir stays warm for the next lane of the same repo.
- Bounds: total pool cap (default 300 GiB) with LRU eviction of
  **idle** dirs only; a leased dir is never evicted. Leases expire
  after a TTL (default 6 h) so a dead lane's dir becomes
  reclaimable. Dirs already present in the pool root are adopted on
  first sight rather than orphaned.
- State is SQLite at `~/.local/state/cargo-target-broker/broker.db`;
  every mutation runs under an flock. No daemon.
- `exec` also watches sccache compiler clients. A client older than the
  configured stall bound with negligible CPU is treated as wedged (the
  observed failure shape in code#13367): the broker reports its pid/crate
  and sends it SIGTERM so Cargo fails that compile instead of parking the
  build indefinitely. Healthy compiles accrue CPU; young idle clients are
  ignored. Default bound: 15 minutes old and at most 5 CPU seconds, polled
  every 30 seconds. This is a local client-side guard; it does not diagnose
  or alter the upstream sccache daemon.

## Use

```sh
# one-shot: checkout, run with CARGO_TARGET_DIR set, checkin
~/tools/cargo-target-broker exec --repo code --profile test -- cargo test -p id

# or explicitly
DIR=$(~/tools/cargo-target-broker checkout --repo code --profile test)
CARGO_TARGET_DIR=$DIR cargo test -p id
~/tools/cargo-target-broker checkin --path "$DIR"

# pool health: sizes by key, leases out, warm/cold hit rate, evictions
~/tools/cargo-target-broker status
```

`--profile` is a caller-chosen label for the build flavor (`dev`,
`test`, `release`, …); keep it consistent per lane family so keys
stay shared. `--repo` takes a repo name or a path (its basename is
used). Holders default to `user@host:pid`; set
`CARGO_TARGET_BROKER_HOLDER` to name the lane/session.

Useful flags: `--wait SECONDS` on checkout/exec (wait for a dir
instead of failing when the per-key cap is fully leased),
`--workspace-cwd DIR` on checkout/exec (clean the selected Cargo workspace
while keeping the command's original working directory),
`--cap-gib`, `--per-key`, `--ttl-secs`, `--pool-root`, `--state-dir`.

The `exec` watchdog can be tuned with `CARGO_BROKER_SCCACHE_STALL_SECS`,
`CARGO_BROKER_SCCACHE_MAX_CPU_SECS`, and
`CARGO_BROKER_SCCACHE_POLL_SECS`. Set
`CARGO_BROKER_NO_SCCACHE_WATCHDOG=1` to opt out. An explicitly empty
`RUSTC_WRAPPER` or `CARGO_BUILD_RUSTC_WRAPPER` also disables it when Cargo
is intentionally run without sccache.

## Integration status

Fully wired (2026-11-11, code#6797):

- `~/tools/fleet-build-lock` routes direct Cargo compile commands
  through a broker lease keyed by workspace and profile. A command with
  `--manifest-path` supplies the selected workspace for checkout cleanup,
  while Cargo itself keeps running from the caller's original directory.
  If the broker is absent, the lock uses the static per-workspace fallback.
- Repo-queue session launches: the opencode claimant's session
  prompts (`pm-stack/opencode-claimant/launch_one.py` and
  `claimant.py`) instruct launched sessions to run heavy cargo
  builds through `target-broker exec`, and the repo `AGENTS.md`
  build-discipline paragraph describes both routes.

Known remaining bypasses: launch surfaces whose stored prompts
predate the claimant change (e.g. OpenChamber's own saved
scheduled-task prompts) inherit only the repo `AGENTS.md`
discipline, and ad-hoc interactive cargo runs are unrouted by
definition.
