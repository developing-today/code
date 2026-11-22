---
name: fleet-operations
description: Use when operating or documenting the Fleet CLI, selecting where a command should run, dispatching builds/tests to Fleet GitHub Actions or Blacksmith, targeting laptop or Codespaces, tracking runs, reusing follow-up artifacts and caches, or adding Fleet command surfaces.
---

# Fleet operations

## Read the repo's command contracts

- Read `pkgs/fleet/docs/verbs.md` for the complete shipped CLI inventory, including nested queue and agent groups.
- Read `pkgs/fleet/doc/agents/remote-build-and-test.md` before submitting or following up on a build, test, or runner command.
- Read the relevant domain source and run `fleet --help`, `fleet capabilities`, and the exact leaf `--help` when flags, outputs, or availability are uncertain. Help inventory can be broader than the capabilities compiled into the current binary.
- Keep the registry help inventory, command reference, and any changed workflow/runbook examples in sync in the same change.

## Choose an execution location

- Run `fleet shell targets` first when laptop, Codespaces, or local placement matters. It reports reachability and the automatic choice.
- Use `fleet shell --target TARGET --cwd DIR -- COMMAND...` to pin an interactive command. Use `--codespace NAME` to choose a specific Codespace.
- Use `fleet shell --build` for a heavy laptop build so it takes the laptop build lock. Use `--detach` only with `--target laptop`, then track it with `fleet laptop-agent poll JOB --wait N` and retrieve files with `fleet laptop-agent fetch REMOTE_PATH LOCAL_PATH`.
- Do not treat `fleet queue dispatch --surface laptop` as laptop execution; the current surface records a handoff. For arbitrary laptop commands use `fleet shell` or `fleet laptop-agent` when reachable.

## Submit remote builds and tests

- Use `fleet queue dispatch --repo OWNER/REPO --command 'COMMAND'` for a Fleet runner command. Target the repository to build; let Fleet choose the runner repository and surface.
- Inspect the default dry-run placement first. Add `--dispatch` to submit the work. Keep Linux and `--runner auto` unless the task requires another platform or runner.
- Prefer GitHub Actions placement; allow automatic placement to use eligible Blacksmith capacity as fallback. Do not pin Blacksmith unless the work specifically needs it. Request macOS only when the command needs macOS.
- Use `--container IMAGE` only for a required build environment. Add `--actions-cache KEY:PATH` and restore-key prefixes for reusable dependencies. Put follow-up files in `target/fleet-output/`; the workflow publishes and restores the `fleet-output` artifact. Use `--push-nix-cache` when the build needs to publish Nix cache output.
- Do not wait for the target pull request or main CI: the Fleet command workflow is dispatched directly.

## Track and continue a run

- Preserve the returned local handle, run ID, runner repository, portable ID, and run URL. Use the URL for logs and `fleet queue session status|result` for structured status.
- Prefer the portable `OWNER/REPO#RUN_ID` ID for handoffs across agents or workspaces; local `fp_...` handles need the originating session store.
- Wait for the previous run to finish before scheduling a dependent follow-up. Pass its handle or portable ID with `--followup`; automatic placement prefers the prior surface when eligible and falls back if it is busy, unavailable, or above its queue threshold.
- Keep build products needed by a follow-up under `target/fleet-output/`. The follow-up can download them while reusing the same repository's dependency cache. Cargo and Go caches use scoped keys plus broader restore prefixes so a new branch can restore a compatible default-branch cache.
- Report the run ID and run URL, terminal status, selected surface, and any artifact/cache handoff result. Do not report a submitted command as successful until its run reaches a successful terminal result.

## Extend the Fleet surface

When adding a command, runner, container, or build target, inspect `pkgs/fleet/fleet-surfaces.json`, the workflow that implements the selected surface, and the registry adapters before changing placement. Keep Linux/macOS eligibility, owner/repository balance, capacity limits, artifact transfer, cache restore/save behavior, and failure fallback explicit. Update the registry help inventory, this CLI reference, the relevant runbook, and the skill only where the reusable procedure changes.
