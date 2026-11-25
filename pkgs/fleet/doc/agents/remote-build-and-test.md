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

Random `fp_...` handles and their records live in `~/.local/state/fleet-place/sessions.json` and mirror best-effort to the queue backend's `placement_sessions` table, so a handle resolves from any seat whose client can reach the broker; the local store remains the offline fallback (and the only input to placement decisions). For a follow-up from another agent or workspace, the portable `OWNER/REPO#RUN_ID` identifier also works; Fleet resolves it through GitHub without the local session record. If dispatch has not correlated a run ID yet, keep using the local handle.

## Placement coordinator (central seat)

Since #13765 a dispatch consults the **central placement coordinator** before it submits anything: the broker holds the fleet-wide true load view, the shared Blacksmith budget, and the waitlist, so two seats cannot both take the last slot or spend the same Blacksmith minutes.

- **Claim before dispatch.** After a surface is chosen the client claims it (`/placement-claim`). `claimed` holds the seat, the run is dispatched, and the slot is confirmed with the run id. `queued` means the owner is full at the fleet level: the session is recorded `queued` with its `claim_handle` and **nothing is dispatched**. `refused` means the Blacksmith budget is spent: the client re-decides once without Blacksmith and claims the replacement.
- **Queued sessions finish themselves.** `fleet queue session status <handle>` polls the claim; when the coordinator grants it, the stored job is dispatched, the slot confirmed, and the recorded handle becomes the run handle as usual. Blacksmith slots settle with the run's reconciled minutes when the session turns terminal.
- **Every record names `placement_path`.** `coordinator` for granted/queued/refused-then-placed, `singular-fallback` (plus `coord_error`) when the coordinator is unreachable — in that case dispatch proceeds exactly as before the coordinator existed, so a broker outage never blocks a build.
- **Operator escape hatch.** `FLEET_PLACE_COORD=off` disables the client (`/placement-*` calls all fail), restoring the pre-coordinator singular path for that process.
- **Broker side.** The five verbs (`/placement-claim`, `/placement-confirm`, `/placement-release`, `/placement-claim-status`, `/placement-load`) live in `turso_queue::placement_coord`; budgets and caps come from the embedded registry only. A background sweep in `queue_broker::run()` refreshes GitHub counts, expires stale holds, grants queued claims FIFO, and recomputes Blacksmith spend.

## Sandbox seats

Sandbox seats run the static `fleet` build (installed by `scripts/muse/bootstrap.sh`). Dispatch and `dispatch-test` call the GitHub API directly from the seat, and the binary validates TLS against the platform root store, so both work under the sandbox's TLS-intercepting egress proxy with the ambient environment — no proxy overrides needed.

On a binary old enough to predate the platform-verifier fix (code#13122), the same calls fail with `invalid peer certificate: UnknownIssuer`: that build trusted webpki roots only, and the egress proxy's CA is not among them. Do not conclude dispatch is unavailable from a sandbox seat on that error — update the binary (re-run `scripts/muse/bootstrap.sh` on a current checkout), or route every proxy variable (`HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`) through the seat's plain-tunnel relay, `http://broker:broker@127.0.0.1:3130`, which presents the real GitHub certificate.

## Other dispatch routes

- **`fleet dispatch-test scan|submit|status`** — the explicit-dispatch route for work inside a fleet-umbrella submodule. `scan` lists the execution homes (dezren39/fleet and its fork network) with their live Actions load and picks the first with free capacity; `submit --submodule <name> --ref <ref> --commands '<cmds>'` stores the job in the fleet-dispatch Turso store and dispatches `fleet-dispatch-test.yml` there; `status <job-id>` reads the stored result. Linux, GitHub-hosted only; use it when the job is naturally "run inside submodule X at ref Y".
- **Raw workflow dispatch** — when no current `fleet` binary is at hand, dispatch the workflows directly: `gh workflow run fleet-command.yml --repo developing-today/fleet -f repo=<owner/name> -f ref=<ref> -f command='<cmd>'` (the same runner the placement route uses; `-f platform=macos` and `-f runner=blacksmith` exist — Blacksmith stays explicit-only), or `gh workflow run fleet-run.yml --repo dezren39/fleet -f repo=<owner/name> -f command='<cmd>'` for the matrix-capable generic runner. Track with `gh run list` / `gh run view --log` on the hosting repo. Prefer the `fleet` verbs whenever they are available: they add placement, budgets, and session tracking.
- No `queue dispatch` verb? Your binary is stale. Get a current one: in a developing-today/code checkout, run scripts/muse/bootstrap.sh (it downloads the pinned static fleet release and installs ~/tools/fleet), or ask the fleet operator for the current published build. Verify with: fleet queue dispatch --help — it must list --dispatch and --followup. On a machine without the Muse bootstrap: read the pinned release tag from the `FLEET_URL` / `FLEET_SHA256` pin block at the top of scripts/muse/bootstrap.sh on main (the tag is the `fleet-static-<short-sha>` segment of `FLEET_URL`), then `gh release download <TAG> -R developing-today/code -p fleet-x86_64-linux-musl` and download the matching `fleet-x86_64-linux-musl.sha256` asset the same way; verify the binary against it (`sha256sum -c`) before installing. Verify after install with: fleet queue dispatch --help and fleet queue surfaces.
