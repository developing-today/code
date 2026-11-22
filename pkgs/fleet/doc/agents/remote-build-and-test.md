# Remote builds and tests through Fleet

`fleet queue dispatch` sends one command to a registered Fleet GitHub Actions host. It dispatches the dedicated `fleet-command.yml` workflow directly; it does not wait for the target repository's pull request or main-branch CI.

## Submit and track a command

Run from the current Fleet checkout or anywhere the Fleet binary and auth are configured:

```sh
fleet queue dispatch \
  --repo developing-today/code \
  --ref main \
  --command 'cargo test --workspace' \
  --dispatch --json
```

The command is a dry run unless `--dispatch` is present. Keep the returned local `handle`, `run_id`, `runner_repo`, `portable_id`, and `run_url`. The URL opens the Actions run and its logs. Once a GitHub run ID is correlated, the JSON output includes `portable_id` in the form `OWNER/REPO#RUN_ID`; it works from another workspace with GitHub access.

```sh
fleet queue session status <HANDLE> --json
fleet queue session result <HANDLE> --json
fleet queue session status '<OWNER/REPO#RUN_ID>' --json
fleet queue session result '<OWNER/REPO#RUN_ID>' --json
fleet queue session list --mine --json
```

Use the target repository in `--repo`; Fleet chooses the runner repository. The default is `--runner auto --platform linux`. The selector balances eligible GitHub-hosted pools across `dezren39`, `developing-today`, and `developing-today-forks`. The public fork pool cannot build private targets.

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

Fleet prefers the previous runner host while it remains eligible, has a free slot, and its queue is below the configured threshold. If that host is overloaded or unavailable, Fleet selects another eligible location. The follow-up workflow can download the previous run's `fleet-output` artifact; place files to carry forward under `target/fleet-output/`. Rust and Go dependency caches use branch-specific keys and broader restore keys, including the default-branch cache. Same-host artifact download and dependency-cache restoration were verified in a follow-up run.

## Choose a runner or build environment

- Omit `--runner` for automatic placement; GitHub-hosted Actions is preferred, with eligible Blacksmith capacity as a fallback within its configured free-minute budget.
- Use `--runner blacksmith` only for an intentional Blacksmith run. Paid Blacksmith overage requires the explicit `--allow-paid-blacksmith <minutes>` override.
- Linux is the default. Add `--platform macos` only for a task that needs macOS.
- Add `--container <image>` to run in a specific container on a compatible surface.
- Use `--actions-cache <key>:<path>` and repeat `--actions-cache-restore-key <prefix>` to request additional dependency paths.

## Laptop and Codespaces

`fleet shell targets` reports which local, laptop, and Codespace execution targets are reachable. `fleet shell <command> [args...]` is the unified shell front door and selects from reachable targets. This is separate from remote GitHub Actions dispatch. `fleet queue dispatch --surface laptop` is currently a handoff record; it does not execute arbitrary commands on the laptop. `fleet dispatch launch` sends an agent session to the laptop through pm-dispatch.

## Portable run IDs

Random `fp_...` handles and their records remain in `~/.local/state/fleet-place/sessions.json`. They work wherever that local store is available. For a follow-up from another agent or workspace, use the portable `OWNER/REPO#RUN_ID` identifier; Fleet resolves it directly through GitHub and does not need the local session record. If dispatch has not correlated a run ID yet, keep using the local handle until it does.
