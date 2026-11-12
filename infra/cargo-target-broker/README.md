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
  shares a live dir.
- `checkin` releases the lease and refreshes the dir's recorded
  size. The dir stays warm for the next lane of the same repo.
- Bounds: total pool cap (default 300 GiB) with LRU eviction of
  **idle** dirs only; a leased dir is never evicted. Leases expire
  after a TTL (default 6 h) so a dead lane's dir becomes
  reclaimable. Dirs already present in the pool root are adopted on
  first sight rather than orphaned.
- State is SQLite at `~/.local/state/cargo-target-broker/broker.db`;
  every mutation runs under an flock. No daemon.

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
`--cap-gib`, `--per-key`, `--ttl-secs`, `--pool-root`, `--state-dir`.

## Integration status

Fully wired (2026-11-11, code#6797):

- `~/tools/fleet-build-lock` routes its `CARGO_TARGET_DIR` default
  through `checkout`/`checkin`: a cargo build that reaches the lock
  without `CARGO_TARGET_DIR` set leases a pooled dir — keyed by the
  lock's workspace key plus the invocation's profile label — for
  the build's duration. If the broker cannot lease, the lock falls
  back to the pre-broker static per-workspace dir.
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
