# Fleet command execution and handoff

**Current handoff reference:** 2026-11-22. The command dispatcher is implemented in `fleet-queue` and is available from the multicall binary as `fleet queue dispatch`; `repo-queue dispatch` is the standalone alias. The selected runner workflow is `.github/workflows/fleet-command.yml` in each Fleet host repo.

## Submit builds, tests, and commands

`dispatch` is a dry run unless `--dispatch` is present. It prints the placement decision and why a surface is eligible or skipped.

```bash
# Show the selected surface without submitting
fleet queue dispatch --repo developing-today/code \
  --command 'nix flake check'

# Submit the same check
fleet queue dispatch --repo developing-today/code \
  --command 'nix flake check' --dispatch --json

# Rust in the standard full build image
fleet queue dispatch --repo dezren39/fleet \
  --command 'nix develop -c cargo test --workspace' --dispatch

# Go test in the standard full build image
fleet queue dispatch --repo developing-today/code \
  --command 'go test ./...' --dispatch
```

The command runs from the target repository checkout. The standard Linux image is the public `ghcr.io/dezren39/fleet-build:full` image: it includes Nix and Go; enter a repository's Nix dev shell for Rust commands. A custom Linux image can be selected with `--container IMAGE`. Custom images must include `bash`, `git`, and `python3` when the shared cache is enabled.

Only `workflow_dispatch` and `workflow_call` trigger the runner. No pull-request or main-branch CI check is a prerequisite.

## Placement and runner preference

- Automatic placement prefers GitHub-hosted runners and balances eligible account owners by live load and recent dispatches. The owner-wide target rules cover valid repositories under `dezren39`, `developing-today`, and `developing-today-forks`.
- Blacksmith is a Linux fallback hosted by `developing-today/fleet`. It is not selected before available GitHub capacity. The free-minute budget is enforced; paid usage requires `--allow-paid-blacksmith MINUTES`.
- macOS is explicit: use `--macos` or `--platform macos`. The macOS pool has a separate conservative cap.
- A surface can be pinned with `--surface fleet-personal`, `--surface fleet-org`, `--surface fleet-forks`, or `--surface fleet-org-blacksmith`. Pins still have to pass the surface's eligibility and budget checks.

## Caches and followups

The workflow enables shared Cargo and Go caches by default. It caches Cargo registry/git data, Rust toolchains and target artifacts, Go modules, and Go build output. The cache key includes host OS, dependency/toolchain fingerprints, and the target ref. Restore prefixes first reuse a cache with the same dependency fingerprint from another target ref, then allow a broader same-OS restore. Cache storage is scoped to the Fleet host repository, so a followup should remain on the same surface while it has capacity.

Add a custom cache namespace and additional paths with `--actions-cache KEY:PATH`; `PATH` may contain newline-separated workspace-relative paths. Add one or more restore prefixes with `--actions-cache-restore-key PREFIX`.

Commands that want to pass outputs to a followup should write them under `$FLEET_OUTPUT_DIR` (`target/fleet-output` in the runner checkout). The workflow publishes that directory as the `fleet-output` artifact for 14 days. A followup restores it at `$FLEET_PREVIOUS_OUTPUT_DIR` and receives the previous run id in `$FLEET_PREVIOUS_RUN_ID`.

```bash
fleet queue dispatch --repo developing-today/code \
  --command 'mkdir -p "$FLEET_OUTPUT_DIR" && printf output > "$FLEET_OUTPUT_DIR/result.txt"' --dispatch

fleet queue dispatch --followup fp_PREVIOUS_HANDLE \
  --repo developing-today/code \
  --command 'test -e "$FLEET_PREVIOUS_OUTPUT_DIR/result.txt"' --dispatch
```

Followups must wait for the prior run to finish before downloading its artifact. The prior surface remains preferred until its queue reaches the configured sticky threshold or capacity is exhausted; then placement falls back to another eligible surface.

## Run identifiers and logs

A submitted run returns an `fp_...` session handle and the chosen Fleet surface. Use the handle to reconnect:

```bash
fleet queue session status fp_HANDLE
fleet queue session result fp_HANDLE
fleet queue session list
```

The status output includes the provider run URL; open it for job logs. `--json` returns the full session record, including the handle, placement decision, and run id.

## Codespaces and laptop

- Codespaces are reuse-only. Pin `--surface codespace-overflow` to use an existing space, or allow it as a last resort. The authenticated GitHub CLI must have the `codespace` OAuth scope; no workflow creates a new space.
- The `laptop` placement surface is still a handoff record and does not execute a shell command. Actual laptop agent work uses the existing `fleet dispatch launch --repo OWNER/REPO --prompt TEXT` or `fleet dispatch enqueue ...` lane through pm-dispatch. The laptop runner is not yet integrated into the same `fp_...` run/status protocol.

## State, auth, and current limits

Placement sessions, owner rotation history, and Blacksmith usage are currently stored in the dispatching laptop's local placement state file. Followup handles are not yet portable across isolated machines; share `FLEET_PLACE_STATE` only when operators intentionally need a shared local filesystem. The Supabase/Cloudflare Worker shared session backend is not connected by this implementation.

`GHCR_ORG_PAT` is used only by runner workflows for private target checkouts and reading artifacts from another Fleet host repo. The public-only `fleet-forks` host has no such secret; a cross-host artifact download there may be unavailable, but the build proceeds and logs that miss. Nix cache publishing is opt-in with `--push-nix-cache` and requires the host repo secrets `NIX_CACHE_SIGNING_KEY` and `NIX_CACHE_UPLOAD_URL`.

The Codespaces OAuth scope must be approved interactively by the GitHub account owner. Do not paste an OAuth token into repository files or command examples.
