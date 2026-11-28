# Session lifecycle: identity, release, and termination

Contract for [#1931](https://github.com/developing-today/code/issues/1931):
what a stable session identity denotes, which lifecycle transitions
`release` performs, and the safe semantics and API shape for terminating
a managed session. Slice 1 (`sessions_read`) and slice 2 (`session_kill`)
are implemented; `resume` is a later slice. This page changes no runtime
behavior.

## Three sessions that share one name

`session` names three different things. The contract keeps them apart:

| Kind | Who mints it | What it identifies | Where it lives |
|---|---|---|---|
| Leasing session (caller handle) | The caller's host (`MCPX_SESSION_ID`, `--session`, or `_meta` key in `transport.sessionMetaKeys`) | The caller on whose behalf pool keys were created | `Registry.leases[CallID]`, `config.CallContext.SessionID` |
| Plugin session | The daemon (`POST /v1/plugin-sessions`) | Everything one plugin connection declares or owns (capabilities, hooks, registrations) | `routes_pluginsessions.go`; dropped atomically on close |
| Upstream MCP session | The upstream server / protocol era probe | One protocol conversation with one upstream process | `internal/pool/upstream_sessions.go`, `internal/mcpclient` |

A fourth identifier rides below all three: the pool instance key, the
scope-resolved value one live process serves (`internal/config/scope.go`,
`internal/pool/pool.go`). Examples: `session:<id>`, `psession:<id>`,
`pid:<n>`, `call:<id>`, `global`, `cwd:<path>`, `repo:<dir>`,
`worktree:<dir>`.

A leasing session is **not** a plugin session (`internal/api/ops.go`:
"a plugin session is not the leasing session a caller names with
`MCPX_SESSION_ID`"). A leasing session is **not** an upstream MCP
session either: the pool opens and tracks upstream sessions per
instance, and closing a caller never renegotiates an upstream
handshake for another caller sharing that instance.

## What stable session identity denotes

A stable session identity is the caller handle the daemon records in
`Registry.leases`: the `CallID` in `config.CallContext`, resolved from
`SessionID` (host-assigned) or a caller-minted ephemeral value.

Concretely (`internal/daemon/registry.go:resolvedFor`,
`internal/config/scope.go`):

- Every call with a non-empty `CallID` creates or touches
  `leases[CallID]` (`lastSeen = now`) and records, per server, the
  scope-resolved key **only when `CallerOwned(key)`**.
- `CallerOwned(key)` is narrow: every `call:<id>` key, plus — only for
  ephemeral callers — the exact `session:<own-session-id>` or
  `pid:<own-pid>` key (and never a daemon-shared PID, `PIDShared`).
  Shared and long-lived scopes (`global`, `repo:`, `worktree:`,
  `cwd:`) are never caller-owned: another caller may legitimately
  still want them.
- Identity is therefore **the lease key, not the instance key**: one
  lease maps to one owned key per server (`owned map[server]key`).
  Two callers sharing a `repo:` instance hold two leases pointing at
  the same key, and neither owns it.
- An absent session degrades to per-call isolation (`call:<id>` via
  `fallback(ctx)`), never to accidental sharing. A degraded scope
  keeps the degraded key and records the reason (`warnDegraded`).

## Lifecycle transitions

```
acquire(scope) -> lease[CallID].owned[server] = key, holders++
lease return   -> holders-- (instance stays live; idle timer owns it)
release(handle)-> forget lease; ReleaseKey(owned key) per server
kill(handle)   -> forget lease; KillKey(owned key) per server
reap (TTL)     -> ReleaseCaller(stale handle) (never kill)
shutdown       -> every pooled process stops; every lease is void
```

Observed today:

- `sessions_read` (`GET /v1/sessions`) is read-only: one
  `pool.InstanceStatus` per live instance (`id`, `pid`, `holders`,
  `key`, `calls`, uptime/idle seconds), flattened across pools
  (`internal/daemon/routes_sessions.go:sessionInventory`). An empty
  daemon answers `[]`, never null. Nothing ends or detaches here.
- `session_release` (`POST /v1/session/release`,
  `Registry.ReleaseCaller`, `Pool.ReleaseKey`) detaches the caller:
  the lease record is deleted and each owned key is stopped **only if
  idle** (`holders == 0`). A held instance survives release; a later
  call under the same key reuses it. Unknown and empty handles answer
  `{"released": 0}` — detach is idempotent.
- `session_kill` (`POST /v1/session/kill`, `Registry.KillCaller`,
  `Pool.KillKey`, #1931 slice 2) is the force path for the same
  handle: the lease is deleted and every owned instance is ended even
  while held (`holders > 0`), its client closed with its streams and
  subscriptions. A later call starts a fresh instance. An unknown
  handle is a typed refusal (`404 unknown session %q`), so a
  misspelled session never reads as an empty success; an empty handle
  is `400 session is required`.
- Returning a stale lease after its instance was killed is a no-op,
  never a resurrection (`pool/session_kill_test.go`).
- End-of-run release happens after hooks, not before: the daemon wires
  `Release` to stopping the session's caller-private instances, and an
  `onSuccess`/`onError`/suffix hook may still need the servers the
  script used — or start ones the same release must also stop
  (`internal/execsvc/execsvc.go:release`).

## Release versus termination

| | Release (`session_release`) | Termination (`session_kill` shape) |
|---|---|---|
| Addresses | Caller session handle (lease key) | Same caller session handle, never an instance id |
| Lease record | Deleted | Deleted |
| Idle owned instance | Stopped | Stopped |
| Held (busy) owned instance | Left alone | Ended, client closed (streams/subs end with it) |
| Shared (not caller-owned) instance | Untouched | Untouched |
| Unknown handle | `200 {"released": 0}` | `404 unknown session` |
| Privilege | Admin (another caller's servers stop) | Admin + destructive (cannot be undone) |

Rule: release detaches; kill ends. Neither ever touches a key the
caller does not own, and neither addresses an instance by id — both
address the session and end what that session created.

## Safe API shape for terminating a managed session

The implemented `session_kill` is the shape future termination keeps:

- Address the **caller session handle** (`{session}` in the body,
  `X-Mcpx-Session` header as fallback), never an instance id and
  never an empty "all sessions" form.
- Require the handle (`400` when absent); refuse unknown handles
  (`404`) instead of answering an empty success.
- Report what ended (`{"killed": n}` / `{"released": n}`), one count
  across the caller's owned keys; per-server detail stays in
  `sessions_read` / `status`.
- Keep the privilege split: ending somebody else's session stays
  privileged, and the force path stays marked destructive.
- Keep termination out of the read path: `sessions_read` never ends,
  detaches, or resumes; `resume` — when it lands — is its own
  operation on the same handle.

A future `terminate managed daemon session` (as opposed to `kill this
caller's leases`) must additionally name the daemon-side owner it
ends (plugin session id vs leasing handle vs upstream session), fence
the old epoch before accepting writes from a replacement, and leave
the shared/global instances of other callers running. That operation
does not exist yet; nothing here adds it.

## Next implementation slice

**Slice 2: `session_resume` (read + re-acquire on the same handle).**

- Goal: a caller that released (or reaped) its handle can resume it:
  `GET /v1/sessions?session=<handle>` scoping of the existing
  inventory plus re-acquire of the same scope-resolved keys, without
  resurrecting killed instances.
- Acceptance: (1) release handle H, resume H → caller-private keys
  restart on next use, shared keys are untouched, `sessions_read`
  shows the same keys with `holders == 0` until reuse; (2) kill handle
  K, resume K → fresh instances, never the pre-kill PIDs; (3) resume of
  an unknown handle is the same typed `404` as kill, not an empty
  success.
- Oracle: `TestResumeReacquiresReleasedHandleWithoutResurrectingKilled`
  in `internal/pool` + `internal/daemon`: acquire s1, release, resume,
  assert live-set equality for shared scopes and PID inequality for
  caller-private scopes; kill s2, resume, assert new PID; unknown
  handle asserts `404`.
- Out of scope: process termination beyond `KillKey`, force-detach,
  install/uninstall, intent routing.
