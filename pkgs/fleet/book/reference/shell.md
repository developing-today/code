# fleet shell

`fleet shell` runs a command, or opens a shell, where builds belong: the laptop, a codespace, or here. A limited sandbox is the wrong place to compile. It adds no transport; it builds the argv for the verb that already knows how to get there. The domain rides on the `agent` capability.

Known discrepancy: the top-level help for this domain lists only `targets` and hides every flag below. The flags are documented here from source (they appear only in the domain's `--help` text). A second discrepancy: the `--detach` usage error for local/codespace says "it needs --target laptop", but the actions target also accepts `--detach` with different semantics — the error string predates the actions target.

See also [fleet agent](agent.md) and [fleet laptop-agent](laptop-agent.md).

## fleet shell

```
fleet shell [options] [-- COMMAND...]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--target` | `auto\|laptop\|codespace\|actions\|local` | `auto` | Where to run. An explicit target is used as given and **never falls back** — a caller that asked for the laptop does not silently build in a sandbox. |
| `--codespace` | `NAME` | the first available | Which codespace (codespace target). |
| `--repo` | `OWNER/NAME` | this checkout's `origin` | The repository an actions run executes in. |
| `--cwd` | `DIR` | — | Directory to run in (the Cargo workspace for the shared target lane). A usage error on the actions target ("the command runs in the runner's checkout of the repository"). |
| `--build` | — | off | A heavy build: take a fleet-build-lock slot; Cargo workspaces use a shared target lane (see Behaviour). A usage error on the actions target ("dispatches through placement instead — drop --build"). Requires a command after `--`. |
| `--timeout` | `SECS` | laptop non-build `120`; laptop build `3600`; actions wait `3600` | How long a laptop command may run; on the actions target, how long to wait for the run. Must be a `u64`. Requires a command after `--`. |
| `--detach` | — | off | Laptop: start it as a laptop job and return its id (follow with `fleet laptop-agent poll JOB --wait N`). Actions: dispatch and return the session handle instead of waiting. A usage error on local/codespace. Requires a command after `--`. |
| `--print` | — | off | Print the plan as JSON instead of running it: `{target, codespace (name\|null), repo (actions repo\|null), program, args, because}`. |
| `-h`, `--help` | — | — | Print the full usage (the only place the flags are listed). |

Positionals before `--`: only the single word `targets` is allowed; anything else is a usage error "unexpected argument '{x}': put the command after `--`". `--build`, `--detach`, and `--timeout` all require a command after `--`; with no command they are a usage error.

### Behaviour

**Targets:** `auto` (default), `laptop`, `codespace`, `actions`, `local`.

**Auto resolution, in order:**

1. Env **`FLEET_SHELL_TARGET`** (parsed with the same validator; an invalid value is a usage error) — only when `--target` was left at `auto`; an explicit `--target X` beats the env.
2. **Here, if this seat is the laptop:** seat signal = env `FLEET_LOCATION` or fleet.toml host location, trimmed, equal to `"laptop"`.
3. **The laptop, if it answers:** probe = running `fleet laptop-agent health` successfully.
4. **A codespace, if one exists:** the first codespace from `gh codespace list --json name,repository,state`.
5. **Here** (`local`), with because-string "neither the laptop nor a codespace answered; running here".

**Auto never picks actions.** Actions is reachable only by explicit `--target actions` (or `FLEET_SHELL_TARGET=actions`).

**Per-target execution plan:**

- **local:** interactive (no command) → `sh -l`. With a command → `sh -lc <line>` where the line is the command, prefixed `cd {cwd} && ` when `--cwd` was given, or the build line when `--build`. `--detach` on local is a usage error.
- **laptop:** interactive → `fleet proxy ssh --` (the pinned ssh leaf). With a command → `fleet laptop-agent exec --shell -- <command>`, with `--cwd DIR` when given and `--timeout N`. With `--detach` → `fleet laptop-agent run --shell -- <command>` instead (no `--timeout` is passed in detach mode), returning the laptop job id.
- **codespace:** `gh codespace ssh -c NAME [-- <line>]`; NAME from `--codespace`, else the first available (preferring `state == "Available"`, else the first listed). Interactive = no `--` part. `--detach` on codespace is a usage error.
- **actions:** the plan *is* a placement dispatch: `fleet queue dispatch --repo {repo} --command {command} --dispatch --json`. `--repo` defaults to this checkout's `origin` remote parsed for `github.com/` / `github.com:` shapes; with neither a flag nor a GitHub origin it errors "the actions target needs a repository". Interactive use is a usage error ("it cannot open a shell").

Command quoting: the words after `--` are joined by shell-string rules — a single word is taken as shell text verbatim; multiple words are POSIX-quoted word by word (left bare only if `[A-Za-z0-9_\-./:=,+@%]`, else single-quoted).

**`--build`: the build lock and the Cargo target broker.** On laptop/local/codespace, a `--build` command is wrapped by the build lock: `~/tools/fleet-build-lock <command>` (one slot). If the host has no executable lock, local/codespace fall back with a stderr note "fleet shell: this host has no build lock; running without one". If the command may invoke Cargo (a `cargo` invocation followed — after skipping toolchain/flag words — by a compiling subcommand `build|check|test|bench|clippy|run|install|doc|rustc|nextest|fix|package|publish`, and conservatively **any** `just` or `make` invocation), the locked command is instead a broker-aware shell script that: locates the Cargo workspace (honouring `--manifest-path`/`--path`), derives a repo key (the path segment after `pkgs/`, else the workspace root basename, sanitised), and leases a shared target dir from `~/tools/cargo-target-broker checkout --repo {key} --profile {dev|test|release} --wait 900 --json` (profile: `test` if the command mentions test/bench, `release` for `--release` or a non-`--debug` `install`, else `dev`; `--workspace-cwd` added when the caller's cwd is outside the workspace). It exports the leased `CARGO_TARGET_DIR`, runs the command in its own process group with signal-forwarding traps, and checks the lease in on exit. If the broker/lease fails, it falls back to `CARGO_TARGET_DIR=$HOME/build-cache/{repo}` with the stderr note "fleet shell: broker unavailable; using shared fallback …". Non-Cargo builds under the lock run as `fleet-build-lock sh -c '<command>'`.

**Detach behaviour, per target:**

- **laptop + `--detach`:** runs `fleet laptop-agent run …` and returns its job id immediately; follow up with `fleet laptop-agent poll JOB --wait N`. No timeout is forwarded.
- **actions + `--detach`:** dispatches through placement (a "patient" call — dispatch is not cut short, "cutting it short would orphan the run"), extracts `handle` from the JSON (missing handle or non-JSON is a Down error), prints `{target: "actions", repo, handle, state, run_url, because, note: "dispatched, not waited on; reconnect with: fleet queue session result {handle}"}` and exits 0.
- **actions without `--detach`:** polls `fleet queue session result {handle} --json` every **15 s** until `terminal` is true or the budget (default 3600 s, or `--timeout`) expires. On terminal it prints `{target, repo, handle, state, run_url, because}` and exits **0 iff state == "succeeded", else 1** (failed *and* cancelled both give 1). On budget expiry it errors "run {handle} still {state} after {budget}s; reconnect with: fleet queue session result {handle}". The run's own output is **not** streamed: "the run's own output lives on that page (gh run view --log)".
- **local / codespace + `--detach`:** usage error — "--detach starts a laptop job; it needs --target laptop".

### Exit codes

- The executed command's / leaf verb's exit code, in general.
- Actions target: `0` iff the run succeeded, else `1` (and `0` for a successful `--detach` dispatch).
- `2` — usage (bad target, missing command for `--build`/`--detach`/`--timeout`, detach on local/codespace, interactive/`--build`/`--cwd` on actions, invalid `FLEET_SHELL_TARGET`).

### Example

```console
$ fleet shell --target laptop --build -- cargo test
```

Takes a build-lock slot on the laptop, leases a shared Cargo target dir, and runs `cargo test` there with the 3600 s build timeout.

## fleet shell targets

```
fleet shell targets
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| — | — | — | Takes no arguments; anything after `--` with `targets` is a usage error. |

### Behaviour

Prints a JSON reachability report:

```json
{"targets": [{"target": "laptop", "reachable": true, "how": "…"},
             {"target": "codespace", "reachable": false, "codespace": null},
             {"target": "actions", "reachable": true, "how": "…"},
             {"target": "local", "reachable": true, "how": "this machine"}],
 "auto_would_pick": {"target": "laptop", "because": "…"}}
```

The `how` strings: laptop = "this seat is the laptop" or "fleet laptop-agent (broker, Cloudflare tunnel, tailnet, ssh)"; codespace = "gh codespace ssh"; actions = "fleet queue dispatch (placement over the registered Actions surfaces)"; actions reachability = `fleet queue surfaces` parses and contains at least one `github-actions` or `blacksmith` surface (of any status).

Known discrepancy: because reachability ignores surface `status`, a registry whose only Actions surface were a standby surface would still report actions reachable even though placement could never pick it.

### Exit codes

- `0` — report printed.
- `2` — usage.

### Example

```console
$ fleet shell targets
```

Prints which targets answer from this seat and which one `auto` would pick, with the reason.
