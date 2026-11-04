# Dynamic publication runtime apply and operator CLI

**Status:** initial design; settle the open decisions below against the existing serve and protocol architecture before source implementation.

## Request and intent

Issue [#2147](https://github.com/developing-today/code/issues/2147) requires operators to publish and unpublish programs while `id serve` is running, without restarting it. A publication may use a dedicated listener (port/interface) or a path on an Explorer listener. The set must persist across restarts and be inspectable through the CLI. An acknowledged path mount may override an Explorer route; an unacknowledged shadow must be rejected or warned about as specified by the issue.

The acceptance walk is: publish a program on a new port and reach it; publish a path mount that shadows an Explorer route and observe the warning and override; unpublish both and verify the listener closes and the Explorer route returns, all while the same serve process remains alive.

## Baseline and scope

PR [#5114](https://github.com/developing-today/code/pull/5114) merged slice 1: the persisted `Registry`, validation, boot restore, and shadow warnings. The remaining work is runtime application of that registry and CLI publish/unpublish/list operations. Do not redesign the slice-1 registry model beyond fields needed for runtime application; do not add unrelated refactors.

`Registry` is intentionally socket-free. `cmd_serve` currently owns configured listener tasks and routers, while CLI clients use the meta protocol and `MetaClient::open()` can fall back to local stores when no serve process is running. A publication command must not report success via that fallback without changing the running server.

## Initial design direction

1. Keep the persisted registry as the desired publication set. A serve-owned runtime component should own live listener handles and the dynamic path-routing state for the lifetime of `cmd_serve`.
2. Apply the persisted set during startup and apply mutations through the same runtime path. Preserve each listener's existing `WebSecurity` boundary. A failed validation, bind, or persistence operation must not leave the CLI reporting success with live and durable state disagreeing.
3. Make publish, unpublish, and list CLI commands address a running serve control surface. They should use the same registry and apply operations as startup, rather than editing the JSON file independently. If no running server is available, fail with an actionable error rather than falling back to a local-store operation.
4. Keep the existing shadow acknowledgement semantics: reject unacknowledged shadowing, surface the warning for acknowledged shadowing, and restore the Explorer route when the path mount is removed.

These are design constraints, not yet a settled transport or transaction protocol. The next step is to choose those details based on the repository's listener, router, and meta-protocol lifecycle.

## Decisions to settle before source changes

- **Control channel and authorization:** The existing meta protocol has authenticated peer identities and `AccessPolicy`, but `--open-writes` can authorize every peer to mutate the store. Publication control changes network listeners and routes, so decide whether it needs a narrower operator-only authorization or a local-only control channel. Do not silently treat ordinary store write access as sufficient without recording the security rationale.
- **Explorer target for path mounts:** `PathMount` stores only a path and calls the target the primary Explorer listener, but serve can have multiple Explorer listeners. Define which listener receives a mount and how ambiguity is reported.
- **Configured/dynamic listener conflicts:** Define duplicate-address behavior and how port-0 publications report the actual OS-assigned address in `list` and after restart.
- **Apply/persistence ordering:** Define how publish/unpublish stage runtime changes and atomic registry writes so a bind or save failure leaves a recoverable, non-success state.
- **Target semantics:** Confirm that “programs and pages” in #2147 means the world-backed program surface already represented by `Publication::world`, not arbitrary static pages.

## Verification plan

- Unit tests for desired-to-live apply behavior, duplicate/conflicting placements, rollback on failed bind/save, route shadowing and restoration, and actual listener address reporting.
- Protocol/CLI tests for request parsing, authorization, no-server behavior, and any append-only wire-format compatibility requirement.
- Integration acceptance using the existing serve harness: add a listener and fetch the program; mount an acknowledged shadow and observe the warning/override; unpublish both and verify route restoration and listener closure without restarting the process; restart and verify persisted publications restore.
- Run focused `pkgs/id` tests locally and inspect the PR's actual basic-tier checks before deciding merge readiness.

## 2026-11-04T21-56-00Z Decision: control and apply boundaries

The implementation will use the existing meta-protocol connection for CLI-to-serve control, but publication requests are **operator-only**, not ordinary store writes. The server will authorize these requests only when the authenticated caller identity is the serving node's own identity; this remains true with `--open-writes` and `--allow-node`. Local publication commands will explicitly connect to the running serve process using its local identity and will fail if no serve process is available. They will not use `MetaClient::open()`'s local-store fallback. This keeps remotely authorized file writers from opening network listeners or changing routes.

The serve process will serialize publication mutations through one manager that owns the registry and active dynamic resources. A listener publish will validate and bind a staged listener, atomically save the candidate registry, then activate the staged listener. A save failure closes the staged listener without changing the active set. Unpublish will atomically save the candidate set before removing the live route or stopping the listener; those in-process removals are non-failing, and a process crash after the save naturally restarts from the new set. The CLI responds success only after persistence and live application both finish.

Because `PathMount` has no listener identity, it targets the first effective Explorer binding (the primary Explorer listener); it fails if no Explorer listener exists. With port `0`, the registry retains the requested ephemeral address so restart may choose a new port; `list` reports the manager's actual bound address while the process is running. “Programs and pages” means the world-backed program surface already represented by `Publication::world`, not arbitrary static content.

These choices fit the current `MetaProtocol`/`AccessPolicy`, `cmd_serve` listener ownership, and the slice-1 `Registry` model. The key trade-off is that commands require local access to the serving identity key; this is preferred over widening the existing remote writer policy. The remaining source changes must preserve each listener's `WebSecurity` and use the same shadow-warning validation for runtime and boot application.

## References

- [Issue #2147](https://github.com/developing-today/code/issues/2147)
- [Slice 1 PR #5114](https://github.com/developing-today/code/pull/5114)
- `pkgs/id/src/publications.rs`
- `pkgs/id/src/commands/serve.rs`
- `pkgs/id/src/protocol.rs` and `pkgs/id/src/meta_client.rs`
