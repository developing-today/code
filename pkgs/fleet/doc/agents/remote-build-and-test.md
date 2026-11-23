# Remote builds and tests through Fleet

`fleet queue dispatch` sends one command directly to the Fleet command workflow in an eligible runner repository. It does not wait for the target repository's pull-request or main-branch CI.

## The rule: dispatch first — do not compile on the sandbox or the laptop

Builds, test suites, and other CPU-heavy work run on dispatched GitHub Actions runners, not on the machine you are sitting on. The sandbox is small and shared; the laptop is the fleet's machine of record and is already loaded. The flow is always:

1. Push the code (branch or commit) to the target repo.
2. Dispatch the command against that ref: `fleet queue dispatch --repo <owner/name> --ref <ref> --command '<cmd>' --dispatch --json`.
3. Track it asynchronously with `fleet queue session status <HANDLE>` and collect the outcome with `fleet queue session result <HANDLE>`; the run's full log is on its Actions page (`gh run view <RUN_ID> --repo <RUNNER_REPO> --log`).

Placement chooses the runner pool by live load — the developing-today org (20 concurrent jobs) and the dezren39 account (40) — so the same command lands wherever there is capacity. GitHub-hosted runners are the default; Blacksmith is a budgeted fallback, never the default, and paid Blacksmith minutes require the explicit `--allow-paid-blacksmith` override. macOS is available with `--platform macos` when a job genuinely needs it. Dependency caches (Actions cache, the shared Nix cache, per-branch Rust/Go keys) are keyed so repeat dispatches of the same kind of job reuse them; `--followup` pins a repeat job to the previous run's host and outputs.

Run locally only when the job cannot be dispatched: a NixOS build or activation, work needing laptop state or secrets, or a quick single-binary check while iterating. "The sandbox was closer" is not a reason.

## Submit and track a command

Run from the current Fleet checkout or anywhere the Fleet binary and GitHub auth are configured:

```sh
fleet queue dispatch \
  --repo developing-today/code \
  --ref main \
  --command 'cargo test --workspace' \
  --dispatch --json
```

The command is a dry run unless `--dispatch` is present. Keep the returned local `handle`, `run_id`, `runner_repo`, `portable_id`, and `run_url`. The URL opens the Actions run and logs. Once a GitHub run ID is correlated, JSON includes `portable_id` in the form `OWNER/REPO#RUN_ID`; it works from another workspace with GitHub access.

```sh
fleet queue session status <HANDLE> --json
fleet queue session result <HANDLE> --json
fleet queue session status '<OWNER/REPO#RUN_ID>' --json
fleet queue session result '<OWNER/REPO#RUN_ID>' --json
fleet queue session list --mine --json
```

The `--repo` value is the repository to build or test. Fleet chooses the runner repository independently; `fleet queue surfaces` prints the configured placement registry. The runner identity must be able to fetch the target. The public fork pool cannot build private targets.

## Continue a run and reuse its outputs

Wait until the previous run reaches a terminal state, then pass its local handle or portable ID to the next command:

```sh
fleet queue dispatch \
  --repo developing-today/code \
  --ref main \
  --followup '<OWNER/REPO#RUN_ID>' \
  --command 'cargo test --workspace -- --test-threads=1' \
  --dispatch --json
```

Fleet prefers the previous runner host while it remains eligible, has a free slot, and its queue is below the configured threshold. If that host is overloaded or unavailable, Fleet selects another eligible location. The follow-up workflow can download the previous run's `fleet-output` artifact; place files to carry forward under `target/fleet-output/`. Rust and Go dependency caches use branch-specific keys and broader restore keys, including the default-branch cache, so a branch can reuse a compatible cache when it lacks a closer match. Same-host artifact download and dependency-cache restoration were verified in a follow-up run.

## Choose a runner

- Omit `--runner` to use automatic placement. GitHub-hosted Actions is preferred; eligible Blacksmith capacity is a fallback within its configured free-minute budget.
- Use `--runner blacksmith` only for an intentional Blacksmith run. Paid Blacksmith overage needs the explicit `--allow-paid-blacksmith <minutes>` override.
- Linux is the default. Add `--platform macos` only when the task requires macOS.
- Add `--container <image>` for a specific environment on a compatible surface.
- Use `--actions-cache <key>:<path>` and repeat `--actions-cache-restore-key <prefix>` to request extra dependency-cache paths. Use `--push-nix-cache` when a Nix build should publish its cache output.

## Target laptop, Codespaces, or local

`fleet shell` runs commands on the laptop, a Codespace, or the local environment. Inspect current reachability and automatic selection first:

```sh
fleet shell targets
```

Pin a target when the command must run there. Use the path that exists on that target:

```sh
fleet shell --target laptop --cwd <LAPTOP_REPO_PATH> --build -- cargo test -p fleet-queue --lib
fleet shell --target codespace --codespace <CODESPACE_NAME> --cwd <CODESPACE_REPO_PATH> -- cargo test -p fleet-queue --lib
fleet shell --target local --cwd "$PWD" -- cargo test -p fleet-queue --lib
```

The `actions` target is the same dispatch through the shell verb — it dispatches the command through placement and waits for the session result (add `--detach` to get the session handle and return immediately):

```sh
fleet shell --target actions --repo developing-today/code -- cargo test -p fleet-queue --lib
fleet shell --target actions --detach --repo developing-today/code -- cargo test -p fleet-queue --lib
```

`auto` never picks `actions`; name it explicitly when a build or test should leave the local machines.

`auto` honors `FLEET_SHELL_TARGET`; otherwise it prefers this seat if it is the laptop, then a reachable laptop, a Codespace, and finally the local environment. An explicit target does not fall back. `--build` takes a laptop `fleet-build-lock` slot and allows a longer build timeout.

For a detached laptop command, add `--detach` and track the returned job ID with `fleet laptop-agent poll JOB --wait SECONDS`; use `fleet laptop-agent fetch REMOTE_PATH LOCAL_PATH` to retrieve an output file. This is separate from GitHub Actions dispatch. `fleet queue dispatch --surface laptop` currently creates a handoff record; it does not execute arbitrary commands on the laptop. `fleet dispatch launch` submits an agent session to pm-dispatch.

## Portable run IDs

Random `fp_...` handles and their records remain in `~/.local/state/fleet-place/sessions.json`. They work where that local store is available. For a follow-up from another agent or workspace, use the portable `OWNER/REPO#RUN_ID` identifier; Fleet resolves it through GitHub without the local session record. If dispatch has not correlated a run ID yet, keep using the local handle.

## Other dispatch routes

- **`fleet dispatch-test scan|submit|status`** — the explicit-dispatch route for work inside a fleet-umbrella submodule. `scan` lists the execution homes (dezren39/fleet and its fork network) with their live Actions load and picks the first with free capacity; `submit --submodule <name> --ref <ref> --commands '<cmds>'` stores the job in the fleet-dispatch Turso store and dispatches `fleet-dispatch-test.yml` there; `status <job-id>` reads the stored result. Linux, GitHub-hosted only; use it when the job is naturally "run inside submodule X at ref Y".
- **Raw workflow dispatch** — when no current `fleet` binary is at hand, dispatch the workflows directly: `gh workflow run fleet-command.yml --repo developing-today/fleet -f repo=<owner/name> -f ref=<ref> -f command='<cmd>'` (the same runner the placement route uses; `-f platform=macos` and `-f runner=blacksmith` exist — Blacksmith stays explicit-only), or `gh workflow run fleet-run.yml --repo dezren39/fleet -f repo=<owner/name> -f command='<cmd>'` for the matrix-capable generic runner. Track with `gh run list` / `gh run view --log` on the hosting repo. Prefer the `fleet` verbs whenever they are available: they add placement, budgets, and session tracking.
