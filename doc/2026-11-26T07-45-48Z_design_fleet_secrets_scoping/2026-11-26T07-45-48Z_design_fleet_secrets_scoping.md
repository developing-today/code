# Fleet secrets scoping — prefix, store and vault selection, per-device profiles

Issue: #10358. Status: design record. It decides how a configured
prefix rewrites and filters secret names, how a call picks a store and
vault, where a device's profile lives, and how the broker and the
direct Bitwarden leg stay consistent under scoping. It writes no code;
the build slices in §9 carry the implementation.

Two rules hold throughout:

- **A prefix is a namespace view, not a security layer.** It prevents
  accidents and keeps an agent's visible context clean. What a device
  can actually read is bounded by the keys it holds (a Bitwarden
  machine account scoped to a project, a 1Password service account
  scoped to a vault), never by the prefix.
- **Nothing configured means nothing changes.** With no prefix, store
  or vault set anywhere, every command's stdout, stderr and exit code
  are byte-identical to main today.

## 0. Governing decisions consumed

| Source                               | What this record takes from it                                                                                                                             |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #10358, Drew's rules of 2026-11-14   | The four prefix rules (§4 D1 to D4), `--store` and `--vault` per call, per-device profiles set at provisioning, a scoped caller sees no unscoped names.    |
| #5554 (closed, the verb as shipped)  | `get` and `list` are the whole surface; broker first, Bitwarden fallback inside `get`; `list` reads the broker only.                                       |
| #9158 (canonical secrets model)      | Which store and namespace a name canonically lives in. This record does not decide that; it only selects among stores that already exist.                  |
| #13948 (secrets plane sync)          | Delta refresh, peer sync and dual-store puts. Scoping must not change the broker's cache contents or refresh, only what a scoped request is answered with. |
| #14370 (closed)                      | `[secrets] bws_project_id` and `FLEET_BWS_PROJECT_ID` as the write project for `BwsStore`.                                                                 |
| Drew's ruling, 2026-11-24 (on #9196) | Design-only gates without a hard blocker are void. The gates in §9 are real ordering edges only.                                                           |

## 1. Verified state on main `2b086382`

- `pkgs/fleet/crates/fleet/src/secrets.rs:73` `run` dispatches `get`
  and `list` only. `cmd_get` (:110) takes exactly one name and calls
  `Resolver::default_chain(config).resolve(name)`. `cmd_list` (:132)
  reads `DirectBrokerSource::list_names` on the laptop, else
  `BrokerSource::list_names`, with no Bitwarden fallback. Exit codes:
  0 ok, 2 usage (:25), 3 unresolved (:26). A broken pipe on `list` is
  a quiet exit 0.
- `pkgs/fleet/crates/fleet-secrets/src/lib.rs:142` `validate_name`
  refuses empty names, `.`, `..`, and any `/`, `?`, `#`, `%`,
  whitespace or control character. A dot elsewhere is legal, so
  `42.x` and `42.42.x` are valid names today. A name with a space,
  such as Drew's example `cloudflare api key`, is refused today.
- `Resolver::default_chain` (:324) builds: `DirectBrokerSource`
  (laptop only, owns the retry budget), `BrokerSource` (session
  broker), `BwsSource`, then `OpSource` only when `FLEET_OP_VAULT` is
  set. `Resolver::resolve` (:350) validates the name once and returns
  the first non-empty value, else `SecretError::Chain`.
- `BwsSource` runs `bws secret list -o json` over every secret the
  access token can see, matches on `key`, then `bws secret get <id>`.
  It does not filter by project.
- `BwsStore` (:1234) writes into `FLEET_BWS_PROJECT_ID` or
  `[secrets] bws_project_id`. `OpSource` (:1347) reads
  `op://{vault}/{name}/{field}` with `FLEET_OP_VAULT` and
  `FLEET_OP_FIELD` (default `credential`).
- `pkgs/fleet/crates/fleet-config/src/lib.rs:471` `SecretsConfig` is
  `#[serde(default, deny_unknown_fields)]` with `token_file` and
  `bws_project_id` only. The config file is `$FLEET_CONFIG`, else
  `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`.
- `pkgs/fleet/crates/fleet-bridge/src/lib.rs:486` `ChainSecrets`
  resolves through `default_chain` and stores through `BwsStore`. It
  is the only write path; there is no `fleet secrets set` verb.
- `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs` caches every
  name the broker's token can see. `GET /secrets` (:828) matches the
  path exactly and returns `{names:[...]}`. `GET /secret/<name>`
  (:838) answers one value, reading through to Bitwarden on a miss.
- No prefix, store, vault or profile selection exists anywhere.

## 2. Intent

A device or agent instance carries a profile, for example "Bitwarden,
project `sandbox`, prefix `42`". Every `fleet secrets` call and every
`fleet bridge` secret read or write on that instance then works inside
that view with no change at the call site. A person can still name a
full secret, and can override the store or vault for one call.

## 3. Ownership

| Part                       | Owner file                                                               |
| -------------------------- | ------------------------------------------------------------------------ |
| Name mapping and filtering | `fleet-secrets/src/lib.rs`, new `Scope` type beside `validate_name`      |
| Profile fields             | `fleet-config/src/lib.rs`, `SecretsConfig`                               |
| Verb flags and output      | `fleet/src/secrets.rs`                                                   |
| Broker-side filter         | `fleet-queue/src/secrets_broker.rs`, `GET /secrets` query handling       |
| Bridge reads and writes    | `fleet-bridge/src/lib.rs`, `ChainSecrets`                                |
| Provisioning of profiles   | whoever writes the instance's `fleet.toml`; the verb never writes config |

## 4. Decisions

### D1. Unconfigured is identity

`Scope::none()` maps every name to itself on the way in and every
stored name to itself on the way out, and filters nothing. With no
prefix, store or vault configured, the verb builds `Scope::none()`
and `default_chain` exactly as today. This is the regression oracle
in §6, not a best effort.

### D2. A prefix rewrites a short name once

With prefix `P`, the separator is always `.`. A caller name `n` maps
to a stored name as follows:

- If `n` starts with `P.`, it is already a full name and is used
  as is.
- Otherwise the stored name is `P.n`.

The mapping runs before `Resolver::resolve`, so every source in the
chain (both broker legs, `bws`, `op`) receives the same full name.
`validate_name` then runs on the full name, so the refused
characters stay refused. A full name whose part after `P.` is empty
(`42.`) is refused as an invalid name.

### D3. Full names still work

Because of D2's first branch, `get 42.cloudflare-api-key` under
prefix `42` resolves the same secret as `get cloudflare-api-key`.

### D4. The prefix is applied exactly once

A secret stored as `42.42.x` inside namespace `42` is reached only by
asking for `42.42.x`. Asking for `42.x` is a full name and reaches the
stored `42.x`. This is Drew's rule that such an item "would only be
able to be set and gotten by saying 42.42".

### D5. `list` filters, strips, and round-trips

Under prefix `P`, `list` shows only stored names that start with
`P.`. Each is displayed by removing `P.`, except where the remainder
itself starts with `P.`: that name is displayed in full. So stored
`42.alpha` shows as `alpha`, and stored `42.42.x` shows as `42.42.x`.
Every displayed name, passed back to `get`, resolves the stored name
it came from. Sort order is the broker's, after filtering.

### D6. The broker filters on its side

A scoped `list` sends `GET /secrets?prefix=P`. The broker answers with
only the cached names that start with `P.`, so unscoped names never
cross the socket to a scoped caller, even from cache. The broker's
cache, its refresh and #13948's watermark are unchanged: scoping
narrows answers, never what is cached. The client applies D5's filter
again on what it receives, so output is correct against any broker
that honours the parameter.

A broker that predates the parameter answers 404 for the query form,
because it matches `/secrets` exactly. A scoped client treats that
404 as unresolved (exit 3, "broker does not support scoped list") and
does not retry unscoped. An unscoped client never sends the
parameter, so it keeps working against old and new brokers alike.

### D7. `get` needs no broker change

`GET /secret/<full name>` already returns exactly one secret, so
scoped `get` sends the full name from D2 and nothing else.

### D8. Store and vault selection

A profile names a `store` and an optional `vault`:

| `store` value  | Chain used                                    | What `vault` means                                                   |
| -------------- | --------------------------------------------- | -------------------------------------------------------------------- |
| unset (`auto`) | today's chain exactly                         | must be unset; setting it without a store is a config error          |
| `bitwarden`    | direct broker, session broker, `bws`; no `op` | Bitwarden project id: reads match only that project, writes go there |
| `1password`    | `op` only                                     | 1Password vault; overrides `FLEET_OP_VAULT`                          |

For `bitwarden` with a vault, `bws` matches only entries whose
`projectId` equals the vault, and the broker filter gains
`project=<id>` beside `prefix`. Writes through `BwsStore` use the
vault as the project, overriding `bws_project_id`.

For `1password`, `list` is a new `op item list --vault <v>` reader in
`OpSource`. Until that slice lands, a `list` with store `1password`
exits 3 with "list is not supported for 1password yet", never an
empty success.

### D9. Precedence

Per call: flag, then environment, then `[secrets]` in `fleet.toml`,
then the D1 default.

| Setting | Flag      | Environment            | Config key         |
| ------- | --------- | ---------------------- | ------------------ |
| prefix  | none      | `FLEET_SECRETS_PREFIX` | `[secrets] prefix` |
| store   | `--store` | `FLEET_SECRETS_STORE`  | `[secrets] store`  |
| vault   | `--vault` | `FLEET_SECRETS_VAULT`  | `[secrets] vault`  |

Prefix has no flag: Drew asked for flags on store and vault only, and
D3 already lets a caller speak in full names. An empty string in any
source counts as unset. A prefix is validated with `validate_name`
and may not end with `.`; a dotted prefix such as `team.42` is legal
and matched as a whole.

### D10. Profiles live in the instance's config file

A profile is the `[secrets]` table of the `fleet.toml` that the
instance runs with. Different agent instances on one host get
different profiles by running with different `FLEET_CONFIG` paths.
The instance's keys (`token_file`, the `bws` access token, the `op`
service account) sit in the same profile, which is what bounds what
the instance can read. Only provisioning writes the file; no verb
writes config.

### D11. `set` is symmetric

Any write maps its name through D2 and writes into the store and
vault of D8. Today that is `ChainSecrets::store` in `fleet-bridge`;
a future `fleet secrets set` verb uses the same `Scope`.
`ChainSecrets::resolve` maps its names the same way, so a scoped
bridge reads what it wrote.

### D12. Broker and direct Bitwarden stay consistent by construction

Scoping is a pure name transform applied before the chain (`get`,
`set`) and a pure filter after it (`list`). The broker and `bws`
legs see identical full names, so they cannot disagree about which
secret a scoped name means. `list` keeps its broker-only behaviour
from #5554; scoping adds no Bitwarden fallback to it.

## 5. Negative cases

- A scoped `list` never prints a name outside `P.`, even when the
  broker returns one (old or buggy broker): the client filter drops
  it.
- A scoped `list` against a broker without the parameter fails with
  exit 3; it never falls back to an unscoped request.
- `vault` without `store` is a config parse error at load time, not
  a silent default.
- An unknown `store` value is a config parse error, and as a flag a
  usage error (exit 2).
- A prefix containing a refused character, or ending in `.`, is a
  config error.
- `get 42.` under prefix `42` is refused as an invalid name (exit 3,
  the same `InvalidName` path as today).
- Names with whitespace stay refused (§10).

## 6. Oracles and tests

Name-mapping matrix, unit tests on `Scope` in `fleet-secrets`, with
prefix `42`:

| Case | Caller name | Configured prefix | Stored name used | Rule |
| ---- | ----------- | ----------------- | ---------------- | ---- |
| 1    | `alpha`     | none              | `alpha`          | D1   |
| 2    | `42.alpha`  | none              | `42.alpha`       | D1   |
| 3    | `alpha`     | `42`              | `42.alpha`       | D2   |
| 4    | `42.alpha`  | `42`              | `42.alpha`       | D3   |
| 5    | `42.x`      | `42`              | `42.x`           | D4   |
| 6    | `42.42.x`   | `42`              | `42.42.x`        | D4   |
| 7    | `42`        | `42`              | `42.42`          | D2   |
| 8    | `42.`       | `42`              | refused          | D2   |
| 9    | `420.x`     | `42`              | `42.420.x`       | D2   |

The same nine rows run for `get` (through a fake chain that records
the name it was asked for) and for `set` (through a fake store). Row 9
guards against a prefix match without the separator.

`list` display, prefix `42`, stored names `alpha`, `42.alpha`,
`42.42.x`, `420.y`, `43.z`: output is exactly `alpha` and `42.42.x`.
A property test asserts, for random stored names under the prefix,
that `to_full(display(s)) == s`.

No-config regression, the explicit oracle Drew asked for: with an
empty `[secrets]` table and no `FLEET_SECRETS_*` variables, the
existing `fleet secrets` tests pass unchanged, and a new test asserts
that `Scope::none()` maps and displays every row of the matrix to
itself and that `list` sends `GET /secrets` with no query.

Broker: `handle_request` tests for `GET /secrets?prefix=42` (filtered
names), `?prefix=` empty (refused, 400), and an unscoped `GET
/secrets` unchanged.

Config: parse tests for each new key, `vault` without `store`,
unknown `store`, a bad prefix, and an old-shape table.

All Rust tests run by remote dispatch (`cargo test -p fleet-secrets`,
`-p fleet-config`, `-p fleet-queue`, `-p fleet`), with `-- --list`
first to confirm the new test names exist.

## 7. Compatibility, migration and reversal

- `SecretsConfig` denies unknown fields, so an older `fleet` binary
  fails to parse a config that sets `prefix`, `store` or `vault`.
  Rollout is binaries first, then profiles. A host that must run an
  older binary sets the profile by environment instead.
- A scoped client needs the broker from slice S2. Profiles with a
  prefix are provisioned only after the broker on that host has S2.
- Reversal is removing the keys or variables: D1 then restores
  today's behaviour exactly, with no data migration, because scoping
  never renames or moves a stored secret.
- `fleet-connect` keeps its secrets stub (exit 69). Widening it stays
  Drew's call from #5554.

## 8. Requirement trace

| #10358 requirement                          | Where               |
| ------------------------------------------- | ------------------- |
| Unconfigured is today's behaviour exactly   | D1, §6 regression   |
| Prefix filters and rewrites                 | D2, D5              |
| Full names still work                       | D3, §6 row 4        |
| Prefix applied once (`42.42`)               | D4, §6 rows 5 and 6 |
| `--store` and `--vault` per call            | D8, D9              |
| Per-device profiles at provisioning         | D10                 |
| Broker cache and list respect scope         | D6                  |
| Where profiles live, who changes them       | D10                 |
| Scoping applies to `set`                    | D11                 |
| Broker and direct Bitwarden consistency     | D12                 |
| Testable list, get, set matrix              | §6                  |
| Build issues split, no-config test explicit | §9, §6              |

## 9. Gates and build slices

| Slice | Content                                                                                                         | Hard edge                   |
| ----- | --------------------------------------------------------------------------------------------------------------- | --------------------------- |
| S1    | `Scope` in `fleet-secrets`; `[secrets] prefix` and `FLEET_SECRETS_PREFIX`; scoped `get`; `ChainSecrets` mapping | none                        |
| S2    | Broker `GET /secrets?prefix=`; scoped `list` with the client filter and the 404 rule                            | S1 (`Scope`)                |
| S3    | `store` and `vault` in config, env and flags; `bws` project filter; broker `project=`; `op` list                | S1; broker part after S2    |
| S4    | `fleet secrets set` verb on `BwsStore`, scoped                                                                  | S1; vault handling after S3 |

Provisioning a prefixed profile on a host waits for S2 on that
host's broker (§7). No other gate applies.

## 10. Non-goals

- Deciding which store or namespace a secret canonically belongs in.
  That is #9158.
- Names with spaces. `validate_name` refuses whitespace because a
  name becomes a URL path segment. Allowing `cloudflare api key`
  needs an encoding decision for `/secret/<name>` and is separate
  from scoping; the matrix uses hyphenated names.
- Treating the prefix as access control. Access is bounded by the
  keys in the profile.
- Peer sync, delta refresh and dual-store puts (#13948).
- Widening the `fleet-connect` stub.
