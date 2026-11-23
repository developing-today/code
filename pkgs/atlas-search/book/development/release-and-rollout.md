# Release and rollout

Source: `home/common/default.nix` and `pkgs/atlas-search/worker/README.md`, `pkgs/atlas-search/worker/package.json` and `pkgs/atlas-search/worker/cloudflare.config.ts` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/README.md — provenance only; this chapter is complete without following it.

Atlas ships as three artefacts on three different paths: the Rust
package (built by Nix, run on the laptop), the service (that package,
run as a systemd user service — through the fleet binary, as explained
below), and the Cloudflare Worker (deployed through the `cf` versions
flow). A change is shipped when every artefact it touches has moved and
been verified in place — not when its pull request merges. The build
itself is [Building and testing](building.md); running the result day to
day is [Running the service and the Worker](../operations/services.md).

## The merge rule

- **CI runs on `main` only.** The Atlas workflows are nightly plus
  dispatch (their actual triggers are stated in
  [Building and testing](building.md)); they are the main-tier safety
  net, not a merge gate, and no pull request waits on them.
- **Merge when verified.** A change merges when its verification —
  local tests, the laptop build, the smoke checks its surface demands —
  has actually run and passed. Finished pull requests do not sit open:
  not merged is not done. A red main after a merge is fixed forward or
  reverted, in the same working session that discovers it.

## The laptop package and the service restart

1. **Build the package** from current `main`:

   ```sh
   nix build path:./pkgs/atlas-search
   ```

   The crane split means a source-only change rebuilds the workspace
   crates, not the dependency tree; the build's check phase runs the
   test suite and its install check runs the smoke script, so a green
   build is already a tested artefact.
2. **Deploy through home configuration.** The service package is
   `pkgs.callPackage ../../pkgs/atlas-search { … }` in
   `home/common/default.nix`, with the toolchain read from
   `pkgs/atlas-search/rust-toolchain.toml`. A home-manager switch is
   what puts the new package in place; nothing is copied into position
   by hand.
3. **Restart the service.** The unit is the systemd **user** service
   `atlas-search.service`: `Restart = on-failure`, `RestartSec = 3`,
   wanted by `default.target`, configuration from
   `~/.local/state/atlas-search/service.json` (mode 0600, written by
   `atlas-search init`). The restart shape is therefore:

   ```sh
   systemctl --user restart atlas-search.service
   ```

4. **Know which binary you just restarted.** The unit's `ExecStart` is
   selected by the home-manager flag `atlasSearchServeViaFleet`, which
   is `true` today: the live service runs **`fleet serve search`** —
   the serve half rehomed into the fleet-search crate (#5604) — not
   `atlas-search serve`. Setting the flag `false` re-points the unit
   at the standalone binary; the unit name, the configuration, the
   port (loopback `127.0.0.1:18760`) and the wire are unchanged, so
   rollback of the serving binary is a single re-point and a restart.
   `init` and `fetch-model` stay on the standalone `atlas-search` in
   both shapes. Debugging "the service" against the standalone binary's
   serve path is debugging the rollback, not the live service.
5. **Verify in place, by surface.** `GET /health` on the loopback port
   answers without auth — `{status: "ok", embedder_state,
   queue_depth, …}` — and the embedder state must reach ready; then one
   real search through the surface the change touched (the CLI, the
   service, or `fleet search`), checked against what the change was
   supposed to do. A version string alone is not verification.

Exposure does not change at rollout time: the service binds loopback
only, and its tailnet exposure is a `tailscale serve` mapping created
imperatively in front of it. A rollout that needs a new listener, a new
port or a public route is not a rollout — it is a design change, and it
goes through a design record first.

## The Worker deploy

First, a correction this book states once and applies everywhere: the
Worker does **not** deploy via wrangler. `wrangler.toml` drives local
development and the test suite; the deploy source of truth is
`cloudflare.config.ts`, and the deploy command (the Worker's
`package.json` `deploy` script) is the sanctioned `cf` binary's
versions flow:

1. **Create a version** with `cf workers versions create`, uploading a
   throwaway secrets file assembled at deploy time from fleet secrets,
   **by name** — the gate token, the service token and the Access
   client pair (`atlas.search-worker-gate-token`,
   `atlas.search-cf-worker-token`,
   `cloudflare.access-atlas-search-client-id`,
   `cloudflare.access-atlas-search-client-secret`). No version deploys
   without its secrets, and no secret value ever lands in a config file
   or the Nix store.
2. **Verify the bindings** on the created version: the `QUERY_CACHE`
   Durable Object, `SERVICE_BASE`, the `HOT_CACHE` D1 binding and its
   flag (below), and the four secret names present.
3. **Create the deployment at 100%.** There is no percentage rollout
   machinery in this Worker; the versions flow's deployment step is
   the rollout.
4. **Verify**: `GET /health` on the workers.dev hostname answers
   `{status: "ok", service: "atlas-search-worker"}` unauthenticated;
   then a gated search and, if the change touched it, an MCP
   `tools/call` — remembering that the Worker degrades to 502 with
   nothing cached while the laptop service is unreachable, so a Worker
   verification is also a service verification.
5. **Rollback** is a new deployment at the previous version id. No
   upload is needed and no code changes hands; the previous version is
   still there.

One deploy-time substitution to check every time: the D1 `database_id`
committed in both config files is a local placeholder
(`00000000-0000-0000-0000-000000000001`), replaced with the real id when
the database is created at deploy time. A deploy that ships the
placeholder has not connected the hot cache to anything.

## The flag posture

New paths land **flag-off**, and stay off until their own change turns
them on. The standing examples:

- **The D1 hot cache's master flag**, `HOT_CACHE_ENABLED`, is committed
  as `"false"` in both Worker config files. Only the literal string
  `"true"` enables it, and the store also requires the `HOT_CACHE`
  binding to be present. State the current truth precisely when you
  touch it: in the code as it stands, nothing in the request path calls
  the hot-cache store at all, so the flag today changes no behaviour
  either way — its read-path integration is follow-on work (#13105),
  and landing that integration flag-off is the pattern to copy.
- **The service itself landed inert**: in its step 5 the systemd unit
  was declared with no `Install` section, so it could not auto-start;
  the `Install` section arrived at the step-6 cutover. A new service
  role follows the same shape — declared, inert, enabled by a separate
  deliberate change.
- **Experimental search paths are off by default**: the shard
  fan-out behind the EXPERIMENTAL `--shards` flag is the CLI-side
  example. Off by default is the correct posture for a path whose
  evidence is still being gathered.

The rule for a new flag: name it, state its default as a literal value
in [Configuration and environment](../reference/configuration.md),
default it to off, and make the enabling change a separate, verifiable
step from the landing change.

## Rollout checklist

1. Merged to `main` when verified; main-tier CI watched after the merge.
2. Package built from `main`; service restarted; `/health` ok and the
   embedder ready; one real search per touched surface.
3. Worker, if touched: version created with its secrets file, bindings
   verified (placeholder D1 id checked), deployment at 100%, `/health`
   and one gated call verified.
4. Flags: every new path landed off; every enabling is its own step.
5. This book updated in the same change — including this chapter, if
   the rollout shape itself moved.
