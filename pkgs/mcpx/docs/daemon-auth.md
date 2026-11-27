# Daemon authentication: per-user keys and capability ceilings

This is the design record for #13659 slice 1. It decides the identity model, credential storage and provisioning, the PolicySession and CapabilityCeiling binding, the loopback migration story, and the #13657 door-selector mapping. Implementation is follow-on slices (section 2.7). This record changes no daemon behavior.

## Current ground

- The daemon HTTP surface is unauthenticated by deliberate design (`http-api.md`): one listener, one access-control decision. Listeners are a unix socket (0600, state directory 0700) and loopback TCP.
- One gate exists: the setting `transport.bearerToken`. When set, every `/v1` and `/mcp` request must carry it as a Bearer credential. It is a single shared token. It identifies nobody and grants everything to whoever holds it. Clients already send it (`WithToken` in `public-client.md`; the exec service hands `MCPX_TRANSPORT_BEARER_TOKEN` to scripts, `environment.md`).
- The public door (#13657, `worker.md`) is the daemon's security boundary today. The Worker checks the caller's OAuth access token or machine bearer and removes it. The bridge leg carries the Worker's own Access service token. Door state at rest is stored only under SHA-256 hashes (`code:<hash>`, `at:<hash>`, `rt:<hash>`). The single scope is `mcpx`, and the `ns` selector is the only per-client exposure control. Its profiles: absent is `conservative` (discovery only), `a,b` is `selected` (discovery plus calls into exactly those namespaces), and `*` is `full`.
- Decision 0007 (accepted 2026-11-17): mcpx has no authenticated user. Elicitation state binds to the request and the daemon's own principal. No client-supplied value is ever an identity input.
- The id-program model (`pkgs/id/book/concepts/capabilities.md`, `concepts/mcp-and-agents.md`): `PolicySession` in `id_caps::mcp_policy` applies, per tool call and in order, (1) a capability-ceiling check, where anything unnamed is outside the ceiling; (2) a quota charge before execution on `frames`, `tool_calls` and `program_invocations`; (3) exactly one audit event per action, refusals included, hash-chained and signed. `RoleGrant` is set by the owner at grant time and never widened by the session.

## 2. Design

### 2.1 Identity model

- Two principal kinds. A **user** is a person or a standing machine role (the owner, `door`, `lane-worker`). A **key** is one credential belonging to exactly one user, with a label (`laptop-cli`, `door-machine`, `door-client-<id>`). Authentication resolves a presented key to the caller identity (user, key). Authorization attaches to the key, so one key can be narrowed or revoked without touching the user's other keys. This mirrors RoleGrant's `agent_key` and `agent_label` split.
- A key is an opaque bearer secret: 32 random bytes, base64url, displayed with the prefix `mcpx_key_`. The prefix aids recognition in logs and is not part of the entropy claim.
- Identity comes only from a daemon-verified credential or from the transport, never from a client-asserted value. Headers, `_meta` fields and session IDs are never identity inputs. Decision 0007 stays intact: elicitation state still binds to the request and the daemon principal, and the authenticated (user, key) is the daemon principal for that call, recorded in audit. 0007 needs no amendment.

### 2.2 Credential storage and provisioning

- The daemon stores only the SHA-256 hash of each key, applying the door's at-rest rule locally. Key records live in the daemon state directory (0700) in `auth.json` (0600). Each record holds a public short ID derived from the hash, label, user, ceiling name, `capability_ref` (the grant record ID stamped on audit events), created-at, last-used-at and a revoked flag.
- Provisioning is local only: `mcpx auth key create --user <u> --label <l> --ceiling <name>` prints the secret once; `mcpx auth key list` and `mcpx auth key revoke <id>` manage keys. Creating a key requires unix-socket access (OS-owner trust) or an existing key whose ceiling carries the admin scope. No key is provisioned or listed over unauthenticated HTTP.
- Rotation is create-new then revoke-old. A key's secret never changes in place. Revocation takes effect on the next request, because the store is consulted per request; no restart.
- Compatibility: when `transport.bearerToken` is set, the daemon imports it at start as a key labelled `legacy-transport-token`, user = daemon owner, ceiling `full`, with a logged deprecation. It is removed in the final migration slice (2.5), not before.

### 2.3 Identity to PolicySession and CapabilityCeiling

- Each key names a **ceiling**: a named `CapabilityCeiling` declared in configuration under a new settings family `auth.*`. Ceilings are data in config, per the house rule that every value is a setting. A ceiling is a set of namespaces (mcpx's worlds), a tool allowlist (gateway tools and upstream tools named separately), a program allowlist for `mcpx_exec`, and quotas on the two axes mcpx has: `tool_calls` and `program_invocations`.
- The id model's third axis, `frames`, has no mcpx analogue and is dropped. This is the one deliberate divergence.
- Per request the daemon runs the PolicySession order, which is the alignment contract with `id_caps::mcp_policy`: ceiling check first, quota charge before execution, and exactly one audit event per action, refusals included, stamped with the key ID and `capability_ref`.
- Ceilings are set by the owner at provisioning and are never widened by the session or the caller (the RoleGrant rule).
- A key with no ceiling named gets the `discovery` ceiling, never the full surface. Starter ceilings: `full` (the owner's clients; everything, today's loopback behavior), `discovery` (namespaces, catalog, types and search; no calls), and `admin` (key management; no tool calls).

### 2.4 The unix socket

The unix socket stays filesystem-permission authentication: 0600, owner-only. A socket caller is the daemon principal (the OS user who owns the daemon) with ceiling `full`, and needs no key. The socket is not an unauthenticated hole; it is the owner identity by construction. stdio `mcpx serve` shims reach the daemon over the socket and are unaffected.

### 2.5 Loopback migration

- New setting `auth.mode` with three values. `off` is today's behavior exactly. `permissive` honors keys when presented, still serves unauthenticated loopback HTTP, and audits every unauthenticated call as the daemon principal. `required` makes HTTP callers without a valid key receive 401 with a `WWW-Authenticate` header, the same refusal shape as the door's.
- Sequence: ship `off` as the default; flip the default to `permissive` in the client-migration slice; flip to `required` and drop the `legacy-transport-token` import only in the final slice, after shipped clients carry keys.
- CLI and serve shims over loopback TCP: the client auto-provisions a per-user key on first use (user = OS username, label `local-cli`, ceiling `full`), stored 0600 under `~/.config/mcpx/credentials`. The existing paths carry it: `pkg/client`'s `WithToken`, and the exec service's `MCPX_TRANSPORT_BEARER_TOKEN` handoff to scripts, which rides the path #1975 already built.
- Migration is per machine and reversible: `auth.mode=off` restores today's behavior exactly.

### 2.6 The #13657 door-selector mapping

- Today the door is the daemon's only boundary. The Worker strips the caller's bearer, the bridge leg carries the Worker's Access token, and the `ns` selector is the sole exposure control, enforced at the door on requests and responses.
- With daemon auth, the bridge maps each door credential to a daemon key and presents that key on the loopback leg. The machine bearer maps to a dedicated key (user `door`, label `door-machine`). Each registered OAuth client maps to its own key (user `door`, label `door-client-<client_id>`), provisioned by the bridge at registration. The caller's own bearer stays stripped. No credential crosses a boundary it was not issued for; the daemon key is issued for exactly the bridge-to-daemon leg.
- The `ns` selector becomes a per-request narrowing on top of the key's ceiling: effective exposure = ceiling intersected with selector. Selector semantics are unchanged (absent = discovery, list = selected, `*` = no narrowing), with one change: `*` no longer widens past the key's ceiling. Under a `discovery` ceiling key, `ns=*` still yields discovery only. A client can narrow, or ask up to its ceiling, but never widen its own exposure.
- The door keeps filtering responses in the first mapping slice, as defense in depth, since daemon-side list filtering is unproven. Retiring door-side filtering is deferred to a later decision made on measurements. `/v1` stays unproxied at the door.

### 2.7 Follow-on implementation slices

1. Slice 2: key store and `mcpx auth` provisioning CLI. Hash-at-rest `auth.json`, create, list, revoke, and the `legacy-transport-token` import.
2. Slice 3: daemon auth middleware. The `auth.mode` setting, key resolution on `/mcp` and `/v1`, the 401 refusal shape, and audit stamping with key ID and `capability_ref`.
3. Slice 4: ceilings. Config-declared `CapabilityCeiling` sets and the PolicySession order (ceiling check, quota charge, audit) on both surfaces.
4. Slice 5: client migration. Auto-provisioned `local-cli` key, client library and exec forwarding, and the `auth.mode` default flip to `permissive`.
5. Slice 6: door mapping. The bridge presents per-client and per-machine daemon keys, selector-as-narrowing at the door, the `auth.mode` default flip to `required`, and removal of the legacy token import.

### 2.7.1 Implementation status

- The key store and the HTTP gate have landed (the gate half of slice 3 together with the store half of slice 2). `auth.json` holds only SHA-256 hashes, mode 0600, in the state directory; a key's public ID is the first 12 hex digits of its hash. The store is re-read whenever the file changes, so a revocation takes effect on the next request.
- `auth.mode` is registered with `off` as the default. `off` is the #1975 gate unchanged and does not consult keys. `permissive` and `required` resolve `Bearer mcpx_key_...` against the store and refuse an unknown or revoked key with 401 in either mode. `required` refuses every HTTP caller with no valid credential. The unix socket needs no key in either mode (section 2.4). `transport.bearerToken`, when set, is honoured in every mode as the virtual key `legacy-transport-token` (owner, ceiling `full`), resolved in memory rather than written into the store.
- An authenticated request carries its (key ID, user, label, ceiling, `capability_ref`) on the request context. The chat-completions proxy never forwards a keyed request's Authorization header upstream.
- The local CLI supports `mcpx auth key create|list|revoke`. Create requires a user and label, defaults to the `discovery` ceiling, and prints the raw key only once; `auth.json` stores only the hash. List exposes public metadata without the secret or hash. Keys are managed through the local state directory, never over unauthenticated HTTP. A lost key cannot be recovered: create a replacement, update its caller, then revoke the old key by its public ID.
- Still open: audit stamping with the key ID, ceilings (slice 4), client migration (slice 5) and the door mapping (slice 6).

### 2.8 Non-goals

No daemon code changes in slice 1. No OAuth inside the daemon; the door remains the OAuth terminus. No change to decision 0007's elicitation binding. No third quota axis. Door-side response filtering is not retired by this record.
