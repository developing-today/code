# Design mill pass 1: fleet subcommands for the four tools — pass-through, endpoint + credential resolution (#10390)

- **Date:** 2026-11-24; publication audit: 2026-11-27.
- **Status:** design for pass 1; no implementation is authorized by this record.
- **Issue:** [#10390](https://github.com/developing-today/code/issues/10390), under the fleet-surface design mill [#10389](https://github.com/developing-today/code/issues/10389).
- **Related implementation foundation:** [PR #14083](https://github.com/developing-today/code/pull/14083) adds the four endpoint settings and the tested resolver. It does not add command families or per-service standard endpoints.
- **Verified when:** the design is reviewable in the repository, maps each requirement to a decision or explicit deferred question, and gives each build slice a command-level acceptance oracle.

## 1. Problem and context

Fleet has no owned subcommand surface for its four tools; callers hand-resolve endpoints and credentials per service. Pass 1 makes Fleet own invocation and resolution while initially passing through to the same local binaries or service APIs. The source issue names the subcommand tree, resolution order, credential sources, raw mode, per-subprogram customization, and each tool's surface.

Current code evidence:

- `pkgs/fleet/crates/fleet-config/src/lib.rs` defines optional `[opencode]`, `[openchamber]`, `[id]`, and `[mcpx]` endpoint sections. `resolve_tool_endpoint` implements **flag → setting → `FLEET_<TOOL>_ENDPOINT` environment variable → standard endpoint**; blank higher-priority values fall through. The standard endpoint is still supplied by each adapter.
- Fleet's OpenCode session probe uses `http://127.0.0.1:13420` when no URL is configured and reads the Basic-auth password from `OPENCODE_AUTH_FILE` or `~/.local/state/opencode/service-prod.json` (`pkgs/fleet/crates/fleet/src/session_state.rs:243-315`).
- ID world commands distinguish guest authority (`--capability` / `ID_WORLD_CAPABILITY`) from admin authority (`--admin-token` / `ID_WORLD_ADMIN_TOKEN`) (`pkgs/id/book/reference/cli-worlds.md`, `cli-worlds-admin.md`).
- MCPX already owns daemon selection and its configured bearer handoff: `transport.bearerToken` is passed to exec children as `MCPX_TRANSPORT_BEARER_TOKEN` only when configured (`pkgs/mcpx/docs/environment.md`, `docs/configuration.md`). The separate per-user key store described in `docs/daemon-auth.md` is not assumed to be landed by pass 1.
- No independent OpenChamber standard API endpoint or credential source is established in this repository. The default therefore remains OpenChamber's native CLI/config path, not an invented port or reused OpenCode credential.

## 2. Goals and explicit non-goals

**Goals**

- Provide `fleet opencode`, `fleet openchamber`, `fleet id`, and `fleet mcpx` command families.
- Centralize per-tool endpoint resolution and credential handoff while preserving the underlying tools' existing semantics.
- Keep an exact raw endpoint/path/body escape hatch for surfaces not yet represented by typed subcommands.
- Define a named subprogram endpoint-customization format and a small, testable resolver contract.

**Non-goals for pass 1**

- Iroh-native ID transport (pass 2), ID REST session/presentation (pass 3), or MCP aggregation and graph routing (pass 4).
- Replacing any underlying tool CLI or API, inventing new authentication, or granting authority by selecting a Fleet subcommand.
- Automatically discovering or probing non-local endpoints, opening network listeners, or making hosted calls without an explicit endpoint and remote opt-in.
- Implementing every model/media endpoint. Those can use raw mode until a later typed slice adds them.

## 3. Alternatives

- **Do nothing:** no Fleet code, but every caller keeps reimplementing endpoint and credential handling. Rejected because it preserves the fragmentation this pass addresses.
- **Fleet as a full normalized façade:** uniform flag shapes, but requires mirroring four evolving upstream APIs and exceeds pass 1. Rejected; the first pass stays thin.
- **One generic HTTP proxy for all four tools:** simplifies transport, but does not cover local binaries, ID's capability-bearing CLI, or MCPX's own daemon resolver. Rejected as the universal interface.
- **Chosen — thin command families with target-native adapters:** Fleet owns stable dispatch, endpoint precedence, explicit credentials, and raw mode; each adapter preserves the target's native request/response and auth behavior.

## 4. Chosen approach: interface, endpoints, credentials, and security

### Command and resolution contract

Canonical command names are `fleet opencode`, `fleet openchamber`, `fleet id`, and `fleet mcpx`. These are the names shown by help; shorter aliases are additive only and do not change the canonical tree.

For an endpoint-capable invocation, resolve in this order:

1. explicit per-invocation `--endpoint`;
2. the tool's `[tool].endpoint` setting;
3. `FLEET_OPENCODE_ENDPOINT`, `FLEET_OPENCHAMBER_ENDPOINT`, `FLEET_ID_ENDPOINT`, or `FLEET_MCPX_ENDPOINT`;
4. the tool adapter's documented standard-local target.

This order matches the resolver already landed in #14083. Empty and whitespace-only levels count as unset. The winning value is passed through unchanged. Do not silently trim, normalize, or rewrite a caller's explicit endpoint.

The standard-local target is adapter-owned, not guessed by the generic resolver:

| Tool        | Standard-local default for pass 1                                                                           | Evidence / boundary                                                                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenCode    | `http://127.0.0.1:13420`                                                                                    | Existing Fleet session probe fallback.                                                                                                                     |
| OpenChamber | Invoke the installed `openchamber` CLI and let its current config resolve its backend.                      | No standalone standard HTTP endpoint is verified in this repository; `--endpoint`/settings are available when a service endpoint is explicitly configured. |
| ID          | Invoke the installed `id` CLI. Remote world operations continue to use ID's explicit node/world addressing. | ID has no single global HTTP endpoint in the current contract.                                                                                             |
| MCPX        | Invoke the installed `mcpx` CLI; daemon operations use MCPX's own socket/`daemon.endpoint` resolution.      | The daemon client already owns local socket and remote `ssh://` resolution; Fleet must not duplicate its socket rules.                                     |

### Credential source per service

Credential resolution is target-specific. Fleet stores only a credential _name/reference_ in config, never a secret value. When a named secret is needed, the adapter resolves it through `fleet-secrets`/the secrets broker and hands it only to the target process or request. Missing credentials fail before the operation with the required name; values are never logged.

| Tool        | Source and handoff                                                                                                                                                                                                                           | Refusal/default                                                                                                                                          |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| OpenCode    | Existing Basic-auth password in `OPENCODE_AUTH_FILE`, default `~/.local/state/opencode/service-prod.json`; username is `opencode`.                                                                                                           | Do not print or log the password. If the file/field is unavailable, report the missing source; do not reuse another service credential.                  |
| OpenChamber | Delegate to OpenChamber's existing CLI/config and its own OpenCode client when it proxies to OpenCode. A future dedicated auth source is a named per-tool setting resolved by Fleet's secret chain.                                          | No separate source is verified on main, so default to no Fleet-injected credential. Never copy OpenCode's password into a distinct OpenChamber audience. |
| ID          | Preserve per-command authority: guest capability from `--capability` / `ID_WORLD_CAPABILITY`; admin token from `--admin-token` / `ID_WORLD_ADMIN_TOKEN`. The caller supplies the intended node/world and level of authority.                 | Missing capability/admin authority is an explicit refusal. Fleet never upgrades guest authority to admin or forwards an admin token to guest operations. |
| MCPX        | Use MCPX's existing config/environment resolver. When `transport.bearerToken` is configured, MCPX passes it to an exec child as `MCPX_TRANSPORT_BEARER_TOKEN`; this slice does not assume the separate per-user key-store design has landed. | Do not put bearer values in Fleet config, raw bodies, logs, or issue text. Let MCPX refuse a missing/invalid credential.                                 |

### Raw mode and per-subprogram customization

Every command family has an explicit raw route, initially `--endpoint <target> --raw-path <exact-path> --raw-body <bytes>`. It sends the path/body as supplied; it is not a shell command, does not perform path discovery, and does not bypass the target's authorization. Raw bodies are treated as sensitive and redacted from logs.

The tool-level endpoint is a base target. A named subprogram can add a path without changing that base. Proposed config shape:

```toml
[opencode]
endpoint = "http://127.0.0.1:13420"

[opencode.subprograms.sessions]
path = "/api/session"

[openchamber.subprograms.home]
homepage = "/"

[openchamber.subprograms.custom]
unique_id_path = "/api/<service-issued-id>"
```

`path`, `homepage`, and `unique_id_path` are opaque strings from config and are used raw; the resolver does not infer a path from a display name. A command-specific `--path` overrides the corresponding configured subprogram path. `ToolEndpointConfig` in #14083 currently lands only the tool-level endpoint mechanism; the subprogram map is a later slice, not claimed as implemented.

### OpenCode and MCP surface boundaries

ACP is treated as a transport for the OpenCode session family (`--via rest|acp`), not a fifth tool tree. The current endpoint and credential defaults remain REST-based. Exposing Fleet commands through MCP or REST is a separate, feature-gated surface (pass 4); the pass-1 command tree is the source interface, not a claim that those façades already exist.

Remote targets require an explicit `--allow-remote` alongside the endpoint override. This is the default safety behavior: the four adapters remain local-first, and a pasted URL cannot silently send credentials or raw bodies to a non-local host.

## 5. Dependency-ordered build slices

**S1 — command tree, adapter interface, endpoint/raw resolver.** Add the four help trees, a shared typed endpoint input, raw path/body route, and tests for flag → setting → environment → standard precedence. Acceptance: `fleet <tool> --help` lists the canonical tree; unit fixtures prove precedence and blank fall-through; raw requests preserve the exact path/body; no live service is required for the resolver test.

**S2 — per-tool credential adapters.** Add target-specific credential-source descriptors and handoff. Acceptance: OpenCode resolves only its Basic password source; ID guest/admin inputs remain distinct; MCPX calls its existing client resolver; OpenChamber does not receive a guessed/reused secret; missing sources produce named, value-free refusals; log canaries prove no credential value leaks.

**S3 — per-subprogram settings.** Add the named path/homepage/unique-id map and command path override. Acceptance: fixtures show each subprogram path is read exactly as configured and does not mutate the base endpoint; unknown names fail clearly; no path is inferred.

**S4 — typed coverage for OpenCode model/media and ACP.** Add stable typed commands for the explicitly listed session/model/media surface, leaving unsupported upstream routes reachable through raw mode. Acceptance: each typed command has a fixture or mock-server oracle; `--via rest|acp` is covered; unsupported operations retain raw-mode access.

**Cross-surface MCP/REST exposure** is deferred to pass 4; no pass-1 slice depends on it.

## 6. Rollout and rollback

Roll out additively behind the existing `fleet` binary. Config sections remain optional and preserve the native command/endpoint behavior when absent. Each slice can be reverted independently by removing its command registration/adapter; no service-side state or migration is introduced. Keep remote access opt-in. On any endpoint or credential error, stop before sending the request and show the selected source name without its value.

## 7. Verification plan

- Unit tests: endpoint precedence, blank levels, per-subprogram path selection, raw-byte/path preservation, credential-source selection, and log redaction.
- Adapter tests: mock each local process/API and assert the exact endpoint, auth scheme, body, and refusal behavior; never require a live OpenCode/ID/MCPX server in the unit suite.
- Manual local smoke per slice: no-flag invocation uses the standard-local target where established; settings/env/flag overrides win in order; credential-omitted calls succeed only when the target's native local auth already supplies authority; missing auth fails visibly.
- Security oracle: no default remote dialing, no cross-service credential reuse, no guest-to-admin escalation, and no credential or raw-body values in logs.

## 8. Open questions (named decider; all non-blocking)

1. **OpenChamber standalone HTTP base and auth.** Default now: use the installed OpenChamber CLI/config and its own backend client; no guessed port and no copied OpenCode password. Decider: the OpenChamber adapter implementer, using the service's checked-in config when that adapter is built. This does not block S1 or OpenCode/ID/MCPX work.
2. **Per-subprogram names and endpoint paths for surfaces not yet in Fleet.** Default now: require an explicit config path or use raw mode; add a typed name only when an upstream endpoint is evidenced. Decider: the adapter implementer for that tool. This does not block the generic map/resolver in S3.
3. **Additional OpenCode model/media operations beyond the current typed list.** Default now: raw mode. Decider: the OpenCode adapter implementer when each upstream route is verified. This does not block the command tree or session commands.

The canonical command names, endpoint precedence, local/remote boundary, credential isolation, and ACP-as-transport are decisions in this record; none requires a person to answer before implementation.

## 9. Record

This dated design is the repository publication of the pass-1 artifact referenced in #10390 and queue task 21683. It incorporates the endpoint resolver landed by #14083, while keeping the remaining design-only slices reviewable. PR #25249 publishes the document and index link; #10390 receives the PR link. Passes 2–4 and implementation are out of scope; the protected parent #10389 is unchanged.
