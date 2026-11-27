# Build manifest schema v1 — from a composition manifest to a built binary

#

# S1 of developing-today/code#14056. Implements plan §5.1 as amended

# (amendment 2: alias resolution + open profile sets; amendment 3: roles

# compose per node). The build tool (`id-compose`, S7) validates manifests

# against this schema and the catalogue; the hash spec

# (`composition-hash-spec.md`) defines what happens after validation.

## Surface syntax

TOML, operator-written, diffable. Schema version is carried inside the
manifest (`schema = 1`) and inside the hash (hash-spec §"canonical
manifest").

## Fields

| Field          | Type     | Required | Meaning                                                                                                                                                   |
| -------------- | -------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `schema`       | uint     | yes      | Manifest schema version. Today: `1`. Unknown versions are refused.                                                                                        |
| `target`       | string   | yes      | Rust target triple, e.g. `x86_64-unknown-linux-musl`.                                                                                                     |
| `name`         | string   | no       | Human name for a served environment's build. Absent for arbitrary (unlisted) compositions. Never hashed — identity is the composition hash, not the name. |
| `profile`      | string   | no       | Base profile: a canonical name or an alias (`basement` resolves to `headless` before anything else).                                                      |
| `features`     | string[] | no       | Additions to the base profile, or the full selection when no profile is named. Every entry must exist in the catalogue.                                   |
| `group_select` | table    | no       | Per-group member overrides, `{ group = "member" }`. Group and member must exist in the catalogue.                                                         |

Omitted `features` means "no additions". Omitted `profile` with `features`
present means "compose from core plus exactly these features".

## Rules (all refusals name the offending entries)

1. **One base.** A manifest names a base profile _or_ an explicit feature
   list composition — additions to a profile are stated as additions, and
   the _resolved set_ is what is hashed (plan §5.1). Stating the same
   feature both in the profile and in `features` is allowed and means
   nothing (sets, not lists).
2. **Alias resolution first.** Aliases (`basement` → `headless`, and any
   user-profile aliases) resolve to the canonical profile before
   resolution and hashing. An alias never changes a composition hash:
   `profile = "basement"` and `profile = "headless"` hash identically.
3. **Closed catalogue.** Every `features` entry and every `group_select`
   member must exist in `catalogue.toml`. Unknown names refuse.
4. **Exclusivity.** Resolution computes the profile closure (base +
   additions + transitive `requires`), applies `group_select`, then
   checks: at most one explicitly selected member per exclusivity group.
   Two explicit members refuse — refusal, never silent weakening
   (the composer's own stance, plan §1.2.3). Core defaults are implicit
   until S6 wires the switches; overlap with a core default warns
   today and refuses from S6.
5. **Unrepresentable.** `currency-poc-tb`, `test-utils`, `test-provider`,
   and any `test-*` name refuse in a manifest — they are harness
   artefacts, never shippable composition members (§0.3, §9).
6. **Requires closure.** Transitive `requires` are added automatically;
   the resolved set (sorted) is the hash input. A manifest need not list
   what its selections imply (`roc-composer` implies `rust`), but may.
7. **Grants, config, data are never manifest content.** A composition
   decides what a binary _can_ do; what it _does_ is runtime
   configuration, a different layer (plan §5.2, composer §3.1 rule).
8. **Roles are descriptive, not selective.** A feature's `roles` say
   which role-acts need it (amendments 1, 3); they do not filter
   resolution. A node's feature set is the union of what its roles
   need — compile, host, and consume features coexist in one build.

## Open profile sets (amendment 2)

Defining a new profile is a config act, not a code act. A user profile
is a TOML table in exactly the built-in format — `name`, optional
`extends`, `features`, optional `group_select`, optional `aliases` —
validated by the same path (unknown features, group violations, and
unrepresentable names refuse identically), hashed identically
(`profile` name is not a hash input; only the resolved set is), and
registered the same way. Built-ins are expressed in the same format so
there is one definition mechanism and one validation path.

## Examples

`examples/` holds three valid manifests under this schema:

- `basement-store-node.toml` — the plan §5.1 example verbatim.
- `headless-alias.toml` — the same composition via the `basement`
  alias; resolves and hashes identically to the `headless` form.
- `user-profile.toml` — a user-defined profile extending `headless`,
  showing the open-set mechanism (a served team profile, not built-in).

## What S7 builds from this

The `id-compose` tool (S7): validate manifest → resolve + compute
closure → compute composition hash (hash-spec) → invoke cargo with the
resolved `--no-default-features --features …` set against the profile's
composition crate → stamp the artefact (`--version` gains composition
hash + profile name) → write the registry record. Bespoke builds run
through the fleet compile coordinator / Actions, never as unrecorded
local acts for served environments.
