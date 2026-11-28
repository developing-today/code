# Capabilities, ceilings and providers

## The model in one paragraph

A guest program holds no ambient authority. Every effect it can cause is a named capability, resolved by a provider on the host side (or, for `client.` capabilities, on the client side), and allowed only when three tests pass: the capability exists in the catalog, the world holds a grant for it, and the grant fits inside the ceilings that bind the world. A request that fails any test returns a refusal event to the guest. The guest learns of the refusal through the same `update` path as a success; see [Worlds and the Roc guest runtime](worlds.md).

The catalog, limits and manifest live in the crate `id-caps` (capability catalog, execution limits and world manifest, split step 1, #4403). Its 22 public modules include `world_caps` (the catalog), `world_limits`, `world_manifest`, `world_scopes`, `cap_baseline`, `cap_class`, `cap_diff`, `used_caps`, `declaration`, `agent_delegation`, the four Roc library gate modules (`library_manifest`, `library_resolution`, `library_gate`, `library_attach`) and the MCP pair (`mcp_bridge`, `mcp_policy`), plus the November additions `account_quota`, `agent_quota_ledger`, `cap_notice` and `role_config`. Disagreement LIB-14: the `id-caps` lib.rs doc still describes only the original three moved modules. The module list in this chapter follows the code.

The manifest's role table (#13237, PR #16428): a world manifest may carry an optional versioned `[role_config]` table, parsed by `world_manifest` against the canonical `role_config` types. Its `schema_version` must be 1; an unknown schema version, an invalid definition (bad name, ceiling-bits or lifetime bounds, checked by `RoleDefinition::validate`), a duplicate name, or an unknown section key refuses the manifest at parse time. Plan, diff and model-apply thread the table as a reported-only `SetRoles` step: an omitted table leaves the live roles alone, a present table reconciles by name, and reordering identical definitions is not drift. The wire executor refuses `SetRoles` loudly in apply until the admin role path lands — it is never silently half-applied and never takes false applied credit.

## The catalog

`id_caps::world_caps` registers the provider catalog. The entries named in the library survey are: `time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `kv.get`, `kv.set`, `kv.delete`, `blob.put`, `blob.get`, `blob.delete`, `http.fetch`, `notify`, `schedule.at`, `schedule.cancel`, `world.info`, `device.info`, `form.present`, `form.result`, `form.cancel`, `loc.position` and `loc.watch`. The survey's enumeration ends with an ellipsis, so this list is the attested subset, not a proven-complete catalog; the full argument schemas and bounds are in `world_caps` itself.

Two namespaces exist. Host providers carry bare names such as `time.now`. Client providers carry the prefix in `CLIENT_PROVIDER_NAMESPACE = "client."` (`world_caps.rs:312`): a capability such as `client.file.read` is executed by the connected shell, not by the host. The shell contract's conformance suite records the native shell declaring exactly one client provider, `client.file.read`. The rollout record for that provider is design record F13.7, carried with the rest of the client-provider design in [Design: capabilities and providers](../design/capabilities-providers.md).

Subscriptions are capabilities too. A guest subscribes through its `wants` document: `players` subscribes to the participant list, and `time.tick:<ms>` subscribes to a tick at that interval, with at most 4 distinct tick intervals per document.

## Grants and revokes

Grants are set in two places.

At host startup, `id serve` takes `--world-cap <NAME>`, repeatable, naming capabilities for the default world's program. The value `*` grants the whole catalog.

At runtime, `id world caps <NODE>` reads and changes a world's grants. Reading needs only a guest capability. Any change needs the admin token (`--admin-token` or env `ID_WORLD_ADMIN_TOKEN`). The change flags are `--grant <CAP>`, repeatable, with `*` meaning the catalog; `--revoke <CAP>`, repeatable; and `--ceiling <CAPS>`, comma-separated and repeatable, naming known capabilities only, with no `*` allowed.

Revocation is per capability name. The surveys do not describe a grant expiry or a grant object format for world caps; the grant state is the world's current grant set as reported and changed by `id world caps`. That absence is stated here as a survey limit, not as a design claim: the grant record format is **unverified** against the survey sources.

## Ceilings

A ceiling is a set of capability names that bounds what a world may hold. Two ceilings apply to a hosted world.

The server-wide ceiling is set by `id serve --world-allowed-caps <NAMES>`, a comma-separated list. The value `*` is refused on this flag: a server ceiling must name its capabilities. A grant outside the server ceiling does not take effect; a guest request that passes its grant but falls outside the applicable ceiling is answered with the error code `outside_ceiling`.

The per-world ceiling is set by `id world caps --ceiling`. It narrows the world further, inside the server ceiling.

The configuration contract in `id-config` states the general rule for ceilings in this system: ceilings only narrow. A scoped override that widens a ceiling is rejected with `CeilingWidened`, never clamped (web-config survey §4.1). The `id-config` ceilings are operation ceilings (`max_hops`, `max_bytes`, `max_time_ms`, `max_fan_out`), distinct from capability ceilings, but the narrowing rule is the same doctrine in both places.

## Cap policy: `deny` and `grant-on-use`

`id serve --world-cap-policy <POLICY>` decides what happens when a guest requests a capability it has not been granted. The default is `deny`. Under `deny`, the request returns a refusal event with code `cap_denied`. The other documented value is `grant-on-use`: the first use of a requestable capability grants it to the world, and the request proceeds. `grant-on-use` does not lift a ceiling: a capability outside the server-wide or per-world ceiling stays refused under either policy, with code `outside_ceiling`. The remaining refusal codes on the capability path are `unknown_cap` (no such catalog entry), `bad_args`, `failed`, `actor_denied` and `result_too_large` (full list in [Worlds and the Roc guest runtime](worlds.md)).

## Attenuation and delegation of world access

World access itself is a capability string. A participant joins with `--capability <C>` (or env `ID_WORLD_CAPABILITY`), and the session protocol carries an `attenuate` client frame: a holder can pass on a narrower access than the one it holds. The library side of delegation for agents is `id_caps::agent_delegation`, with types `AgentId`, `Role`, `RoleBinding`, `Delegation`, `JoinGrant` and `DelegationRefusal`, and the function `verify_join`. World invite roles are a separate, coarser ladder — `spectator`, `observer`, `player`, `guest`, `delegator`, `builder`, `moderator`, with invite default `guest` — covered in [Identity, the directory and groups](identity-directory.md). The attenuated-capabilities design record of 2026-10-09 is carried in [Design: capabilities and providers](../design/capabilities-providers.md).

## The Roc library import gate

Capabilities bind at compile time as well as at run time. A Roc library that uses a capability declares it, and the import pipeline in `id-caps` checks the declaration before the host loads the program: `library_manifest` states the requirement, `library_resolution` resolves it, `library_gate` admits or refuses the import, and `library_attach` attaches an admitted library. The worked example is `examples/roc-cap-demo/`: a library `lib/Greeter.roc` over `records.read`, plus two compile-must-fail negative tests — a missing capability against `platform-no-caps.roc`, and a wrong `RocCap` signature. The integration test `crates/id-caps/tests/roc_import_policy.rs` runs the same pipeline against `examples/roc-import-policy/fixtures.json`.

## Policy enforcement next to the session: `PolicySession`

The MCP bridge holds the clearest shipped statement of the enforcement order, in `id_caps::mcp_policy`. A `PolicySession` wraps a `BridgeSession` and applies, per tool call: (1) a capability-ceiling check against the session's `CapabilityCeiling` (sets of tools, worlds, object classes and a program allowlist; anything unnamed is outside the ceiling); (2) a quota charge taken before execution, on three axes — `frames`, `tool_calls`, `program_invocations`; (3) exactly one audit event per action, refusals included, hash-chained and signed with the journal key. It also enforces a stale-decision rule (a call's `screen_revision` must name the latest observed revision) and per-tool sliding-window rate limits. Quota exhaustion parks the agent with the world event `agent parked, quota exhausted`. `world.leave` is never refused by ceiling or quota and is charged nothing, but it is audited. The bridge itself is covered in [Agents in worlds: the MCP bridge and policy](mcp-and-agents.md) and [The MCP bridge](../reference/mcp.md).

The same pattern — ceiling first, charge before execution, audit every outcome — is the enforcement shape the world host applies to guest capability requests, with the grant set and ceilings of the world in place of the agent's `RoleGrant`. The surveys name the host-side pieces (catalog, grants, ceilings, refusal codes) and the bridge-side `PolicySession` in full; a single named host-side policy type equivalent to `PolicySession` is **unverified** against the survey sources.

## What is design, not shipped

The surveys attest the catalog, grants, ceilings, the two cap policies, the import gate and the bridge policy point as shipped code. The following are design records only, and are carried in [Design: capabilities and providers](../design/capabilities-providers.md): the capability taxonomy across web, ESP32/MQTT and text/audio interfaces; `speech.*` conversational audio capabilities (a feature record dated 2026-11-20; the shipped catalog enumeration in the library survey does not list a `speech.*` entry, so its presence in the shipped catalog is **unverified**); the MQTT device bridge `id.device.v1`; capability certification and its exception policy; and the used-capability derivation tooling that scans a program's call graph to produce its used set (#5998). The `id-caps` modules `used_caps`, `cap_diff` and `cap_baseline` ship as library code; the derivation tool as a user-facing surface is the design record's subject.
