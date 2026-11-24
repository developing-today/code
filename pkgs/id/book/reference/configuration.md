# Configuration and environment

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-config/src/lib.rs, https://github.com/developing-today/code/blob/main/pkgs/id/config/id-config-v1.schema.json, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/server_config/mod.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-core/src/defaults.json

ID has five distinct configuration surfaces, and they are not interchangeable:

| Surface                          | Form                                    | Who reads it                                       |
| -------------------------------- | --------------------------------------- | -------------------------------------------------- |
| Environment variables            | Process environment, 66 named variables | The `id` binary, its crates, benches and tests     |
| The `id-config` v1 document      | JSON, parsed in memory                  | The `id-config` crate; no file location, no loader |
| The server-config file           | TOML, operator-chosen path              | `id serve --server-config <path>`                  |
| Embedded `defaults.json`         | JSON, compiled into the binary          | Serve/peers discovery and web bindings             |
| `[interfaces.web]` in `app.toml` | TOML, per app                           | The web layer, for one app's route and socket      |

## Environment variables: the complete table

66 named variables are read across the tree: **29 production/build**, **20 bench**, **17 test**. The production set is given in full below, with read site and the behaviour when the variable is unset. Bench and test variables are summarised by family. A further six hosted-Worker bindings, one dynamic credential scheme, and three variables the process _writes_ follow the tables; none of those are counted in the 66.

### Production and build variables (29)

| #   | Name                               | Read site                                                                                                          | Default behaviour when unset                                                                                                                   |
| --- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `ID_WEB_TOKEN`                     | clap env for `--web-token`, `crates/id-cli/src/cli/mod.rs`                                                         | No web gate token; the web UI is ungated (and `serve` warns). See [The web UI: pages and HTTP API](web-ui.md).                                 |
| 2   | `ID_WEB_COOKIE_SECURE`             | clap env for `--web-cookie-secure`, `cli/mod.rs`                                                                   | `false`.                                                                                                                                       |
| 3   | `ID_WEB_MAX_IDENTITIES`            | `crates/id-web/src/identity.rs` (`MAX_IDENTITIES_ENV`)                                                             | `10000`. The value `none` means unbounded. `0` or an unparsable value warns and falls back to `10000`.                                         |
| 4   | `ID_WORLD_ADMIN_TOKEN`             | clap env in `cli/mod.rs` and throughout `cli/world.rs` (values hidden)                                             | World admin frames and invites are disabled unless the flag is given.                                                                          |
| 5   | `ID_WORLD`                         | clap env in `cli/world.rs`, on many subcommands                                                                    | Unset. One subcommand (`world replica`) defaults it to `lobby`.                                                                                |
| 6   | `ID_WORLD_CAPABILITY`              | clap env in `cli/world.rs` (values hidden)                                                                         | The capability must come from the `--capability` flag.                                                                                         |
| 7   | `ID_WORLD_SOURCE_ONLY`             | clap env for `--world-source-only`, `cli/mod.rs`                                                                   | `false`.                                                                                                                                       |
| 8   | `ID_GALLERY_DIR`                   | clap env in `cli/mod.rs` and `cli/gallery.rs`                                                                      | No gallery is served.                                                                                                                          |
| 9   | `ID_GALLERY_ACCEPT_PUBLISH`        | clap env, `cli/mod.rs`                                                                                             | `false`. Requires a gallery directory.                                                                                                         |
| 10  | `ID_ROC_BIN`                       | clap env in `cli/mod.rs` and `cli/world.rs`; `env::var` in `id-web` world bridge, `id-worlds` compile, and `serve` | Falls back to a `roc` binary found on `PATH` or in roc-home candidates.                                                                        |
| 11  | `ID_ROC_PLATFORM`                  | clap env in `cli/mod.rs` and `cli/world.rs`; `env::var` in world commands and `serve`                              | Falls back to `./examples/roc-world`.                                                                                                          |
| 12  | `ID_WORLD_COMPILE_CACHE_DIR`       | `crates/id-cli/src/commands/serve.rs`                                                                              | Unset or empty: the shared compiled-world cache is disabled. The cache is opt-in.                                                              |
| 13  | `ID_WORLD_COMPILE_CACHE_LRU_BYTES` | `commands/serve.rs`                                                                                                | The `id-worlds` cache default, 2 GiB. An unparsable value falls back to the default.                                                           |
| 14  | `ID_APPS_DIR`                      | `commands/serve.rs` (`var_os`)                                                                                     | Falls back to the built-in apps root.                                                                                                          |
| 15  | `ID_PUBLICATIONS_FILE`             | `crates/id-cli/src/publications.rs` (`var_os`)                                                                     | Falls back to the default publications file path.                                                                                              |
| 16  | `ID_SHELL_FILE_READ_ROOT`          | `crates/id-native-shell/src/main.rs`                                                                               | The native shell file-read root is not additionally restricted or configured by environment. See [id-native-shell](native-shell.md).           |
| 17  | `RUST_LOG`                         | `src/main.rs`                                                                                                      | When no CLI log level is set, the filter is `debug` plus noisy-module filters. Log-level resolution order is in [The id command line](cli.md). |
| 18  | `LOG_LEVEL`                        | `src/main.rs`                                                                                                      | Participates in log-level resolution; unset means ignored.                                                                                     |
| 19  | `DEBUG`                            | `src/main.rs`                                                                                                      | A truthy value raises the log level in resolution; unset means ignored.                                                                        |
| 20  | `ID_LOG_FORMAT`                    | `src/main.rs` (structured logging)                                                                                 | Historical plain-text stderr logging. Recognised values: `text`, `json`.                                                                       |
| 21  | `ID_LOG_FILE`                      | `src/main.rs`                                                                                                      | No log file.                                                                                                                                   |
| 22  | `ID_LOG_FILE_LEVEL`                | `src/main.rs`                                                                                                      | The log file uses the main level.                                                                                                              |
| 23  | `ID_LOG_TZ`                        | `src/main.rs`                                                                                                      | The default timestamp timezone.                                                                                                                |
| 24  | `HOME`                             | `id-worlds` compile, world commands, CLI integration test helper                                                   | Used for roc-home candidates and home resolution; the test falls back to `/home/user`.                                                         |
| 25  | `PATH`                             | `id-worlds` compile and world doctor (`split_paths`)                                                               | An empty path list.                                                                                                                            |
| 26  | `TIGERBEETLE_POC_BINARY`           | `src/currency_poc/harness.rs` (`BINARY_ENV`, `var_os`)                                                             | Proof-of-concept harness only: no external TigerBeetle binary.                                                                                 |
| 27  | `CARGO_MANIFEST_DIR`               | `build.rs`, plus compile-time `env!` in tests and helpers                                                          | `"."`.                                                                                                                                         |
| 28  | `CARGO_FEATURE_WEB`                | `build.rs`                                                                                                         | Unset means the web feature is off and the build script skips the web asset step.                                                              |
| 29  | `PROFILE`                          | `build.rs`                                                                                                         | `""`.                                                                                                                                          |

Notes on the logging set: an unrecognised `ID_LOG_FORMAT` or `ID_LOG_TZ` value, or a log file that cannot be opened, warns on stderr and logging continues without that piece. Blank flag and environment values count as unset in log-level resolution.

### Bench variables (20), by family

All are read only by `benches/world_bench.rs`.

| Family              | Variables                                                                                                                                                                                                                                                               | Default behaviour                                      |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Guest burn / frames | `ID_BENCH_GUEST_BURN_CONCURRENCY`, `ID_BENCH_FRAME_EVENTS`                                                                                                                                                                                                              | Helper-parsed; unset means the committed bench value.  |
| Provenance          | `ID_BENCH_MACHINE_CLASS` (default `fleet-laptop`), `ID_BENCH_GIT_SHA` (falls back to `GITHUB_SHA`, then a computed value), `GITHUB_SHA`, `ID_BENCH_TOOLCHAIN` (default `unknown`), `ID_BENCH_RESULTS` (default `results.jsonl`)                                         | As stated.                                             |
| Many-worlds         | `ID_BENCH_MW_IDLE_WORLDS` (`1000`), `ID_BENCH_MW_ACTIVE_WORLDS` (`100`), `ID_BENCH_MW_EVENTS_PER_SECOND` (`20`), `ID_BENCH_MW_DURATION_SECS` (`10`)                                                                                                                     | Parsed by one helper with the stated literal defaults. |
| Stress              | `ID_BENCH_STRESS_TIER`, `ID_BENCH_STRESS_SEED`, `ID_BENCH_STRESS_OUT`, `ID_BENCH_STRESS_MEM_PART` (`all`), `ID_BENCH_STRESS_IDLE_WORLDS`, `ID_BENCH_STRESS_ACTIVE_WORLDS`, `ID_BENCH_STRESS_ACTIVE_EVENTS`, `ID_BENCH_STRESS_HOT_EVENTS`, `ID_BENCH_STRESS_HOT_MEMBERS` | Parsed by one helper with a per-tier default.          |

### Test variables (17), by family

Read only in tests, test helpers, or test configuration. None affects a production run.

| Variables                                                                                                                               | Where                                                                                               |
| --------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `ID_FED_NODE_B`                                                                                                                         | `id-sync` substrate transport test; unset means the test peer B is absent.                          |
| `BAKEOFF_RESULTS`                                                                                                                       | `id-store` graph-encoding bakeoff; unset means no results file is written.                          |
| `CARGO`                                                                                                                                 | `id-store` store-traits test; default `cargo`.                                                      |
| `SCREEN_PARITY_GENERATE`                                                                                                                | `id-core` screen parity test; unset means the parity corpus is not regenerated.                     |
| `ID_TEST_FAKE_SH`                                                                                                                       | `id-worlds` compile test helper, also set by `flake.nix`; unset means the real shell.               |
| `TERRITORY_WASM`                                                                                                                        | `id-worlds` sandbox test; unset means the test module is not loaded.                                |
| `ID_IDENTITY_PROBE_PATH`                                                                                                                | `id-web` identity tests; unset means the probe is not run.                                          |
| `WORKER_POC_REFERENCE_OUT`                                                                                                              | Worker PoC reference test; unset means the reference JSONL is not written.                          |
| `PROPTEST_CASES`, `PROPTEST_RNG_SEED`                                                                                                   | Property tests; unset means the proptest default case count and a random seed.                      |
| `ID_BINARY`, `ID_TEST_EXAMPLES_DIR`, `ID_TEST_APPS_DIR`                                                                                 | CLI integration tests; unset means the built binary path and the in-tree examples/apps directories. |
| `ID_TUPLE_FUZZ_CASES`                                                                                                                   | Tuple property tests; unset means the default case count.                                           |
| `ID_E2E_WORLD_ADMIN_TOKEN`                                                                                                              | Playwright config (TypeScript); unset means the config-file fallback token.                         |
| `UPDATE_EXPORT_FIXTURE`                                                                                                                 | Hosted-worker export test (TypeScript); unset means the fixture is not regenerated.                 |
| Playwright runner variables: `TEST_PORT` (`4173`), `NIX_BUILD_TOP`, `PLAYWRIGHT_VM_TEST`, `CHROMIUM_BASE_URL`, `FIREFOX_BASE_URL`, `CI` | Read by `e2e/playwright.config.ts`, as coded there.                                                 |

### Hosted-Worker bindings (not process environment)

The hosted Worker reads Cloudflare `Env` bindings, not process environment variables: `USERS`, `BLOBS`, `USER_CEILING_BYTES`, `BRIDGE_WAITING_PER_AUTHOR`, `BRIDGE_WAITING_TOTAL`, `BRIDGE_WAITING_MAX_AGE_MS`, plus test-only `TEST_USERS`. They are specified in [The hosted Worker API](hosted-worker.md).

### The dynamic `env:<VAR>` credential scheme

One reader names no fixed variable. The `EnvCredentialResolver` in `src/foreign_libsql.rs` resolves any credential reference string of the form `env:<VAR>` from configuration, looking up `<VAR>` in the process environment. The variable name is data, not code, so no name can be listed for this reader. (Its own test uses `TEST_LIBSQL_TOKEN`.) A configuration that carries `env:MY_TOKEN` therefore reads the variable `MY_TOKEN` at resolve time; the secret value itself never appears in the configuration.

### Variables the process writes

Three variables are set by ID, not read by it. When `id serve --world-mail-command <PROG>` runs a confirmation-mail command, the child process receives:

| Name              | Content                                      |
| ----------------- | -------------------------------------------- |
| `ID_MAIL_TO`      | The recipient address for that confirmation. |
| `ID_MAIL_SUBJECT` | The confirmation subject.                    |
| `ID_MAIL_BODY`    | The confirmation body.                       |

These are set in `crates/id-directory/src/directory_mail.rs` during construction of the child's environment. That construction is the only production `set_var` path in the tree.

## The `id-config` v1 document

The `id-config` crate defines the canonical versioned configuration contract: a `ConfigDocument` in JSON. **The crate has no file location and no loader.** `ConfigDocument::parse(&str)` parses a JSON string in memory, `validate()` checks the hard invariants, and `resolve()` produces a deterministic `ResolvedConfig`, serialised with `canonical_json()`. A deployment that uses this document supplies the string itself; there is no path the crate opens.

The JSON Schema twin lives at `pkgs/id/config/id-config-v1.schema.json` and is kept in lockstep with the Rust types by `pkgs/id/tests/config_invariants.rs`. Companion modules cover layered resolution (`LayerOverride`, `ResolutionInput`, `ResolvedLayers`) and protocol negotiation, whose transcript domain is `id-config/negotiation-transcript/v1`.

The document has **10 top-level keys and 29 leaf keys**. Every struct is `deny_unknown_fields`: an unknown key rejects the document. Validation is fail-closed throughout.

| Key                            | Type / values                                                     | Default / rule                                                                                                                                                                                      |
| ------------------------------ | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `schema_version`               | `u32`                                                             | **Required.** Must equal `1`, else the document is rejected with `UnsupportedSchemaVersion`.                                                                                                        |
| `config_id`                    | string                                                            | **Required**, non-empty.                                                                                                                                                                            |
| `revision`                     | `u64`                                                             | **Required.**                                                                                                                                                                                       |
| `mode`                         | `single-node` \| `hosted-tenant` \| `federated-relay`             | **Required.** Seeds the behaviour defaults below.                                                                                                                                                   |
| `protocol_versions`            | string array                                                      | **Required**, non-empty, duplicate-free. The resolved form is sorted.                                                                                                                               |
| `ceilings`                     | object                                                            | **Required.** Every field must be ≥ 1, else `ZeroCeiling`.                                                                                                                                          |
| `ceilings.max_hops`            | `u32`                                                             | Hard cap on relay hops.                                                                                                                                                                             |
| `ceilings.max_bytes`           | `u64`                                                             | Hard cap on a single-operation payload.                                                                                                                                                             |
| `ceilings.max_time_ms`         | `u64`                                                             | Hard cap on single-operation wall-clock time.                                                                                                                                                       |
| `ceilings.max_fan_out`         | `u32`                                                             | Hard cap on delivery targets.                                                                                                                                                                       |
| `gates`                        | object                                                            | Default: all `true`. Setting any gate to `false` **rejects** the document with `FixedGateDisabled`.                                                                                                 |
| `gates.signature_verification` | bool, const `true`                                                | One of the five fixed gate IDs: `signature-verification`, `validation`, `replay-protection`, `authorization`, `tenant-scope`. Gates are mechanism, not behaviour.                                   |
| `gates.validation`             | bool, const `true`                                                | As above.                                                                                                                                                                                           |
| `gates.replay_protection`      | bool, const `true`                                                | As above.                                                                                                                                                                                           |
| `gates.authorization`          | bool, const `true`                                                | As above.                                                                                                                                                                                           |
| `gates.tenant_scope`           | bool, const `true`                                                | As above.                                                                                                                                                                                           |
| `behavior`                     | object                                                            | Default: empty. Every field `None` means "inherit the previous plane".                                                                                                                              |
| `behavior.max_hops`            | `u32`, optional                                                   | Must fit the applicable (possibly narrowed) ceiling.                                                                                                                                                |
| `behavior.sync_scope`          | `metadata` \| `full`, optional                                    | —                                                                                                                                                                                                   |
| `behavior.tier`                | `cold` \| `warm` \| `hot`, optional                               | Tier labels never authorise access.                                                                                                                                                                 |
| `overrides`                    | array of scoped overrides                                         | Default `[]`. Applied in scope-name order. A duplicate scope rejects the document.                                                                                                                  |
| `overrides[].scope`            | string                                                            | Required, non-empty.                                                                                                                                                                                |
| `overrides[].ceilings`         | object: `max_hops?`, `max_bytes?`, `max_time_ms?`, `max_fan_out?` | May only **narrow** a ceiling. Widening is rejected with `CeilingWidened`; it is never clamped.                                                                                                     |
| `overrides[].behavior`         | object                                                            | Same shape and rules as `behavior`.                                                                                                                                                                 |
| `extensions`                   | map of string to JSON                                             | Default `{}`. Keys must be namespaced, matching `^[^./\s]+([./][^./\s]+)+$` (for example `example.com/experiment`). Values are retained verbatim in the resolved output and are **never executed**. |

Field classes: mechanism fields are `schema_version`, `config_id`, `revision`, `protocol_versions`, `ceilings`, `gates`. Behaviour fields are `mode`, `behavior`, `overrides`, `extensions`. The `ResolvedConfig` adds `provenance`: for each behaviour field, one of `mode:<mode>`, `document`, or `override:<scope>`.

### Mode behaviour defaults

| Mode              | `max_hops` | `sync_scope` | `tier` |
| ----------------- | ---------- | ------------ | ------ |
| `single-node`     | `1`        | `metadata`   | `cold` |
| `hosted-tenant`   | `3`        | `full`       | `warm` |
| `federated-relay` | `5`        | `full`       | `hot`  |

### Worked example

A toy document, and what resolution does to it:

```json
{
  "schema_version": 1,
  "config_id": "toy-node",
  "revision": 4,
  "mode": "single-node",
  "protocol_versions": ["v1"],
  "ceilings": { "max_hops": 4, "max_bytes": 1048576, "max_time_ms": 5000, "max_fan_out": 8 },
  "behavior": { "tier": "warm" },
  "overrides": [{ "scope": "relay", "ceilings": { "max_hops": 2 } }]
}
```

Resolution: behaviour starts from the `single-node` defaults (`max_hops 1`, `metadata`, `cold`), the document's `behavior.tier` replaces `tier` with `warm` (provenance `document`), and the `relay` override narrows that scope's `max_hops` ceiling to `2`. Changing the override to `"max_hops": 9` would reject the whole document with `CeilingWidened`, because `9` widens the document ceiling of `4`. Adding `"gates": {"validation": false}` would reject it with `FixedGateDisabled`.

## On-disk configuration: the server-config TOML

`id serve --server-config <FILE>` loads an operator server-config file (`ServerConfig::load`, in `crates/id-cli/src/server_config/`). Every table is optional; an empty file is valid. A malformed file fails startup, naming the parse error. Loading the file needs a build with the `world` feature; see [serve and publications](cli-serve.md).

The file has **23 named keys**, including 5 `[budgets]` keys.

| Key                                            | Default / rule                                                                                         |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `[create] users[]`                             | With `[create]` absent, anyone may create a group.                                                     |
| `[create] groups[]`                            | As above.                                                                                              |
| `[create] min_role`                            | Unset means any member may create.                                                                     |
| `[[users]] key`                                | A provisioned user entry.                                                                              |
| `[[users]] id`                                 | The user's 64-hex Ed25519 public key.                                                                  |
| `[[users]] alias`                              | Optional. Users are provisioned at startup, idempotently.                                              |
| `[[groups]] key`                               | A provisioned group entry.                                                                             |
| `[[groups]] name`                              | The group name.                                                                                        |
| `[[groups]] creator`                           | A user key; that user becomes Admin.                                                                   |
| `[[groups]] alias`                             | Optional.                                                                                              |
| `[[groups.members]] user`                      | A member entry for the group above it.                                                                 |
| `[[groups.members]] role`                      | One of `pull`, `read`, `write`, `manage`, `admin`.                                                     |
| `[policy] freshness_window_seconds`            | `300` (5 minutes).                                                                                     |
| `[policy] pull_policy`                         | `open`. The other value is `members`.                                                                  |
| `[budgets] app_inbound_bytes_per_hour`         | 20 GiB/h: `21474836480`.                                                                               |
| `[budgets] app_inbound_bytes_per_second`       | 32 MiB/s: `33554432` (the 1-second window of `BudgetPolicy::default` in `id-core`); see WEB-14 below.  |
| `[budgets] per_peer_bytes_per_hour`            | 100 MiB/h: `104857600`.                                                                                |
| `[budgets] per_peer_burst_bytes_per_minute`    | 8 MiB/min: `8388608` (the 60-second window of `BudgetPolicy::default` in `id-core`); see WEB-14 below. |
| `[budgets] per_ip_bytes_per_hour`              | 250 MiB/h: `262144000`.                                                                                |
| `[[budgets.counterparty]] peer`                | A per-peer override entry.                                                                             |
| `[[budgets.counterparty]] bytes_per_window`    | Replaces — does not stack with — the per-peer limits for that peer.                                    |
| `[[budgets.counterparty]] window_seconds`      | The override window.                                                                                   |
| `[[budgets.counterparty]] messages_per_window` | Optional.                                                                                              |
| `[residency]`, `[[boundaries]]`                | Optional tables; their shapes are owned by `server_config/boundaries.rs`.                              |

The operator block list is explicitly **not** configuration: it lives in the event store's `blocks` table (see [Data stores and schemas](data-stores.md)), not in this file.

### Two budget keys the module documentation omits (WEB-14)

The `server_config/mod.rs` header documents three `[budgets]` keys. The parser implements five, adding `app_inbound_bytes_per_second` and `per_peer_burst_bytes_per_minute`. An operator reading only the module documentation cannot discover those two live knobs. That is disagreement WEB-14; both keys are listed in the table above because the parser accepts them.

### Worked example

A toy server-config file:

```toml
[create]
min_role = "write"

[[users]]
key = "alice"
id = "0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd0123abcd"
alias = "Ada (toy)"

[[groups]]
key = "toy-club"
name = "Toy Club"
creator = "alice"

[[groups.members]]
user = "alice"
role = "admin"

[policy]
freshness_window_seconds = 300
pull_policy = "members"

[budgets]
per_peer_bytes_per_hour = 104857600
```

At startup, user `alice` and group `toy-club` are provisioned idempotently, group creation requires role `write` or above, pulls are restricted to members, and each peer is budgeted at 100 MiB per hour.

## Embedded `defaults.json`

`crates/id-core/src/defaults.json` is embedded at compile time with `include_str!`. Its shipped content is:

```json
{ "mode": "append", "topic": "id-peer-discovery-v1", "topic_secret": "id-public-discovery-v1" }
```

The schema is the `Defaults` struct in `crates/id-core/src/discovery.rs`, deserialised with `deny_unknown_fields`, so an unknown key fails the parse. Verified against the checkout, it has 7 keys — the survey's count of 6 is one short; the code is the side documented here:

| Key            | Type / default                          | Meaning                                                                                                                                                                |
| -------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bootstrap`    | string array, default `[]`              | Bootstrap node IDs.                                                                                                                                                    |
| `topic`        | string, optional                        | The gossip discovery topic. Must be supplied together with `topic_secret`.                                                                                             |
| `topic_secret` | string, optional                        | The gossip discovery topic secret. Must be supplied together with `topic`.                                                                                             |
| `web_bindings` | string array, default `[]`              | Web listener bindings, replaced wholesale by `--web-listen`.                                                                                                           |
| `relay`        | relay policy selector, default `N0`     | The relay policy used for connections.                                                                                                                                 |
| `gossip`       | bool, default `true`                    | Whether gossip discovery is enabled by default.                                                                                                                        |
| `mode`         | `append` or `replace`, default `append` | How these defaults combine with the hardcoded constants: `append` keeps the built-in list defaults and appends the file values; `replace` uses the file lists instead. |

Precedence, lowest to highest: hardcoded constants, then `defaults.json`, then CLI flags, then the runtime skip flags `--no-default-bootstrap` and `--no-default-topic`, which discard the file's bootstrap IDs or topic values and fall back to the constants. The `id --version` long form prints which defaults source is in force, as a provenance line naming the file's SHA-256 and mode.

## `[interfaces.web]`: per-app web binding

Each app's `app.toml` may carry an `[interfaces.web]` table, read by `web_interface_from_app_toml` in `crates/id-web/src/app_interface.rs`. It has 3 keys:

| Key      | Default                                 | Rule                                                                                                                             |
| -------- | --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `mode`   | —                                       | `adapter`: host surfaces render the app. `direct`: the app ships its own frontend; accepted by the parser but not adapter-bound. |
| `route`  | `/world` (for the default world app)    | Must start with `/`, be a valid axum pattern, differ from `socket`, and not collide with the host-reserved route list.           |
| `socket` | `/ws/world` (for the default world app) | Same rules as `route`.                                                                                                           |

The host-reserved route list is mirrored against the main router and the world bridge routes in [The web UI: pages and HTTP API](web-ui.md), so an app cannot claim a host path.

## Data directory layout (paths, not a config file)

The data directory is the current directory by default. `--data-dir <path>` selects one; `--new [NAME]` creates `.iroh/<NAME>/` under the current directory and uses it. All data paths are relative to it: `.iroh-store/`, `.iroh-key`, `.iroh-serve.lock`, `.iroh-meta/`, `.id-worlds/<NAME>/`. `--ephemeral` uses in-memory stores instead. The formats at those paths are specified in [Data stores and schemas](data-stores.md).

Other on-disk configuration locations:

| File              | Location                                                                                                                                                      |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| App manifests     | `app.toml` per app, under `ID_APPS_DIR` or the built-in apps root.                                                                                            |
| Gallery directory | `--gallery-dir` / `ID_GALLERY_DIR`: verified `<content-hash>.idpb` bundle files plus an `operator/` folder holding collections, tags and admission decisions. |
| Directory journal | `directory.jsonl` in the server-config directory.                                                                                                             |
| Keysets           | `.id-keyset.jsonl` and `.id-keyset-client.jsonl`.                                                                                                             |

## Serve, web and world defaults that act as configuration

These CLI defaults are configuration in effect; the flags themselves are specified in [serve and publications](cli-serve.md) and [World commands: administering and verifying](cli-worlds-admin.md).

| Setting                                  | Literal default              |
| ---------------------------------------- | ---------------------------- |
| `--web`                                  | off                          |
| `--port`                                 | `3000`                       |
| `--bind`                                 | `127.0.0.1`                  |
| `--web-token`                            | unset (no gate)              |
| `--web-cookie-secure`                    | `false`                      |
| `--world-name`                           | `lobby`                      |
| `--world-checkpoint-every`               | `1000`; `0` means never trim |
| `--world-max-open`                       | `256`                        |
| `--world-max-sessions`                   | `1024`                       |
| `--world-idle-secs`                      | `600`                        |
| `--world-boot-window-secs`               | `600`                        |
| `--world-cap-policy`                     | `deny`                       |
| Gossip topic (hardcoded fallback)        | `id-peer-discovery-v1`       |
| Gossip topic secret (hardcoded fallback) | `id-public-discovery-v1`     |
