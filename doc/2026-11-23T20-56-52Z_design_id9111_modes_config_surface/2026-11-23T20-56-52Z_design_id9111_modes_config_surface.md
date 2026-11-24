# #9111 — Modes and configuration surface: modes, link kinds, storages, tiering as config

Issue: [#9111](https://github.com/developing-today/code/issues/9111) (section §7 of program
[#9106](https://github.com/developing-today/code/issues/9106)).
Date: 2026-11-23. Author: fleet lane code-j (subagent of session ses_e5df63851a5ffet6CP9N7AyzCy).
Status: **design record — not an authorization to implement.** It fixes the configuration model,
its file format, its merge rules and the defaults for the fan-out keys. It does not write a schema
file, code or migration. Implementation issues are named in §10 and are **not filed** by this record.

Governing inputs, consumed and not re-decided:

- **Summit crosscut** `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`:
  "Mechanism is primitive; behavior is config"; the `ConfigDocument` shape; the resolution order;
  "mode is a coherent behaviour bundle and never a code fork"; tiering as signed, versioned
  identifiers; fail-closed validation; two-phase activation with rollback to the prior revision.
- **Part B** `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md`, "Modes/config
  seam (#9111)": required knobs (peer class, sync scope, relay permission, fan-out priority, budgets,
  retention, local rating weights); configuration may not disable validation, bounds, provenance,
  finite dedup retention, emergency revocation/peer disable, or rejection observability.
- **#9141 peer trust classes** `doc/2026-11-17T05-15-00Z_design_id9141_peer_trust_classes.md`:
  `fully_trusted | send_only | accept_signed_from | unknown`, `PeerPolicy`, versioned per peer, the
  four-check acceptance gate. Class is a directional label, never a gate bypass.
- **#9144 fan-out** `doc/2026-11-17T15-40-00Z_design_id_federated_query_fanout/…md` §3, §5, §8, §11.1:
  the fan-out keys this record must give a schema and validation to. Its §8 states that the file
  format and merge rules belong here.
- **#9110 primitives/adapters/hooks** (design landed via PR #9238): the extensibility boundary. The
  summit note `doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`
  S0.1–S0.17 is the source for "primitive vs config".
- **#9106 Part A** namespace registry (#9116, server-local, `namespace_id`, `shorthand_policy`,
  `defaultSlot` unset by default), in `part-a-addressing/versioned-design.md`.

## 1. Problem

The ID program has five axes that people will want to vary: how a deployment behaves (mode), which
peers it trusts and how (peer policy), how it talks to each peer (link kind), where bytes live and
how long they stay (storage), and how records are classified for placement and retention (tiering).
The program also has per-server namespace registries (Part A) and a fan-out query surface (#9144)
with its own bounds. Without one model, each lane invents its own keys, and the four failure modes the
summit set out to prevent appear:

1. A mode becomes a code fork, so two deployments with "the same" mode behave differently.
2. A config key disables a check (validation, gate, bounds, provenance, revocation), and nothing
   refuses it.
3. A peer's trust is spelled in one file and its fan-out limits in another, so a relay can be
   allowed by one and bounded by neither.
4. A config file that a newer or hostile writer extended silently changes trust behaviour.

This record gives one schema story for all five axes and the fan-out keys, one split rule between
config and primitives, and one set of load, merge and validation rules that refuse rather than
guess.

## 2. The split rule: primitives are mechanism, config is behaviour

A thing is **config** when an operator may legitimately choose a different value and the system stays
correct for every value in the allowed range. It is a **primitive** when changing it changes what the
protocol means, or when it is a guarantee a peer relies on.

| Put it in config (behaviour)                                                                                        | Keep it a primitive (mechanism or invariant)                                                                  |
| ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Which mode bundle applies; per-mode defaults                                                                        | The acceptance gate (signature, signer, semantics, scope) — not disableable                                   |
| Peer class and operation set per peer (ceiling and grant)                                                           | Class semantics (what `send_only` means); class is a ceiling, not a grant                                     |
| Link kind selection per peer and per operation (sync, query/fan-out, send-only, control)                            | The wire contract of each link kind; a link kind cannot add an unauthenticated path                           |
| Storage binding: which adapter family backs which store, its durability and retention class                         | The port contract each adapter must meet (atomic scope, unknown-outcome readback, conformance)                |
| Data-tier and storage-tier placement and retention mapped from tier identifiers                                     | Tier identifiers are signed, versioned data; reclassification creates a new version, never mutates an old one |
| Fan-out `priority`, `timeout_ms`, `deadline_ms`, `per_peer_timeout_ms`, `peer_breadth`, `max_hops` (within ceiling) | The hop ceiling (8), budgets that are finite by construction, provenance, duplicate and loop rejection        |
| Local rating weights and thresholds (Part B V4)                                                                     | Rejection observability; emergency peer disable; key revocation                                               |
| Namespace registry reference and `shorthand_policy` / `defaultSlot`                                                 | Core parsing of `id://` and the two address cores                                                             |
| Sync scope and retention within a declared maximum                                                                  | Signed-and-validated acceptance; finite dedup retention with a bounded hard maximum                           |

**Rule R-1 (split test).** A key is config only if (a) it has a default, (b) its every allowed value
is safe under the gate, and (c) it does not appear in the list of invariants in §5.3. Otherwise it is
a primitive, and config may only select among primitive-defined alternatives. Anything a config key
cannot express is a code-level primitive with an adapter, never a free-form hook in the file.

Extensibility (#9110) enters at adapter selection only: a config key names an adapter family
(for example `storage.adapter = "redb" | "turso" | "indexeddb"`); the adapter's conformance suite
decides what it may do. Config never supplies code.

## 3. The config schema

### 3.1 Top-level document

One `ConfigDocument` per deployment, in a single file format (§6). It has these top-level keys. Each
key is mandatory or optional as shown; unknown top-level keys are a load error (§6.4).

| Key                 | Required | Meaning                                                                                                  |
| ------------------- | -------- | -------------------------------------------------------------------------------------------------------- |
| `schema_version`    | yes      | Integer. This record defines `1`. A newer major rejects the file; a newer minor adds optional keys only. |
| `config_id`         | yes      | Stable string identifying the deployment's config lineage.                                               |
| `revision`          | yes      | Monotonic integer. Activation rejects a revision lower than the active one (§6.6).                       |
| `mode`              | yes      | Name of a mode bundle (§4). Must resolve to a built-in or an allowlisted bundle.                         |
| `protocol_versions` | no       | Allowlist of protocol versions this node will negotiate. Default: the versions the binary implements.    |
| `ceilings`          | no       | Hard resource ceilings (§5.2). Non-overridable. Defaults in §5.2.                                        |
| `peers`             | no       | Array of peer entries (§3.2). Default: empty (no peers; local-only).                                     |
| `links`             | no       | Link-kind bindings (§3.3). Default: sync and query disabled.                                             |
| `storage`           | no       | Storage bindings (§3.4). Default: the local persistent store only.                                       |
| `tiers`             | no       | Tier identifiers → placement and retention (§3.5). Default: none; all records in tier `default`.         |
| `namespaces`        | no       | Reference to the server-local namespace registry (#9116). Default: built-in registry only.               |
| `extensions`        | no       | Map keyed by namespaced name `vendor.name`. Retained; never executed unless understood and authorized.   |
| `fanout`            | no       | The #9144 fan-out keys (§7). Default: empty remote list, local-only behaviour.                           |

Secrets are never inline. A key that needs a secret takes a reference `{ "secret_ref": "<name>" }`
resolved by the secrets store at activation. Resolved output shows the reference, not the value.

### 3.2 Peer entries

One entry per peer relationship. A peer is directional per relationship: the same key may be
`fully_trusted` toward one peer and `send_only` toward another.

```
peers: [
  {
    peer_id: "<peer-key-id>",           // required; the peer's signing identity
    class: "fully_trusted" | "send_only" | "accept_signed_from" | "unknown",   // default "unknown"
    operations: ["sync.accept", "sync.send", "query.out", "query.answer"],   // default []
    relay_granted: false,               // default false; effective only with class fully_trusted (§7.3)
    scope: "<scope-label>",             // default: the deployment's default scope; cannot widen class
    policy_version: <int>,              // required; monotonic per peer (#9141 §2)
    valid_from: "<RFC3339>" | null,     // default null
    expires_at: "<RFC3339>" | null,     // default null; past expiry evaluates as "unknown"
    timeout_ms: <int> | null,           // per-peer timeout (§7.2); default null
    priority: <int> | null              // fan-out ask order (§7.1); default null (list position)
  }
]
```

The `peer` entry is the #9141 `PeerPolicy` (`peer_id`, `class`, `operations`, `relay_granted`, `scope`,
`policy_version`, `valid_from`, `expires_at`) with fan-out fields added. #9141 says `relay_granted` is a
separate flag and that no class grants it. This schema keeps that: `relay_granted: true` on a
non-`fully_trusted` peer is a **load error** (§6.5), not a silent drop.

Operation names are the #9141 matrix operations: `sync.accept` (inbound), `sync.send` (outbound read),
`query.out` (ask this peer), `query.answer` (answer its queries), `relay` (governed only by
`relay_granted`, listed here for completeness). Each is allowed only if listed here **and** permitted
by the class. An operation absent from `operations` is denied even for `fully_trusted` (#9141 §3).

### 3.3 Link kinds

A link kind is a transport capability choice, bound per peer and per operation. It is config because
the same peer may be reached by different kinds for different work; it is a primitive because each
kind's wire contract is fixed by its protocol.

| Link kind   | Carries                                               | Allowed classes                                                                              | Config keys                                |
| ----------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------ |
| `sync`      | Signed events and objects, both directions per ops    | `fully_trusted`, `accept_signed_from`, `send_only` (send side only)                          | `sync_scope`, `retention_class`, `adapter` |
| `query`     | Read-only queries and signed answers (fan-out, #9144) | `fully_trusted`, `accept_signed_from` (query-out); `fully_trusted` + `relay_granted` (relay) | `timeout_ms`, `per_peer_timeout_ms`        |
| `send_only` | Outbound-only delivery, no inbound acceptance         | `send_only`                                                                                  | `adapter`, `retention_class`               |
| `control`   | Emergency peer disable, key revocation messages       | any; always available                                                                        | none (cannot be disabled, §5.3)            |

`links` in the document holds **defaults per kind**: `links.sync.adapter`, `links.query.timeout_ms`,
and so on. Per-peer values in `peers[]` override them within the ceilings. Per-operation overrides
(summit crosscut: "config supports per-operation connection selection") are an entry under
`peers[].overrides` keyed by operation name. An override may choose an adapter or a timeout. It may
**not** change the class, the gate, the signature check, replay checks or the tenant boundary (§5.3).

### 3.4 Storage bindings

A storage binding names the adapter family that backs one store and its declared durability,
retention and sync participation. Adapter families are independently selectable (summit crosscut:
NATS, Kafka and Postgres are separate families, implementations may be deferred while contracts and
test doubles exist).

```
storage: {
  local_store:  { adapter: "redb", durability: "fsync", sync_participation: "full" },
  event_log:    { adapter: "redb", durability: "fsync", sync_participation: "none" },
  object_store: { adapter: "fs" | "s3" | "r2" | null, retention_class: "default" }
}
```

Defaults: `local_store.adapter = "redb"`, `durability = "fsync"`; `event_log` on the same adapter;
`object_store` absent (no object store). A value an adapter does not advertise in its capability
record is a load error (§6.5), not a runtime fallback. Summit crosscut: "capability mismatch and invalid
config are rejected before operation side effects."

### 3.5 Tiers

Tiering is native data, not a config-only label. A record references a signed, versioned
`data_tier` and/or `storage_tier` identifier. The `tiers` block maps identifiers to placement and
retention:

```
tiers: {
  "default": { placement: "local", retention: "indefinite" },
  "archive": { placement: "object_store", retention: "365d" }
}
```

Rules: tier identifiers are part of the signed record; policy maps them to placement; a tier label
**never authorizes access** (summit crosscut); reclassification creates a new signed version rather
than mutating the old record. An unknown tier identifier on an incoming record is held in `default`
placement and rejected for any placement-dependent operation, not silently reclassified.

## 4. Modes

A mode is a named, coherent bundle of behavioural defaults. It is never a code fork and it confers no
identity or privilege (summit crosscut).

Built-in modes at this record's date:

| Mode              | What it sets (defaults only)                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------------------------- |
| `single-node`     | No peers; `fanout.remotes = []`; `links` all off; local store only. Behaves as v1 local-only.                   |
| `hosted-tenant`   | Peers allowed only from the tenant's configured list; relay off; hosted service is an ordinary configured peer. |
| `federated-relay` | Peers allowed; `relay` allowed only for `fully_trusted` peers with `relay_granted`; `max_hops = 3`.             |

A mode may set any **behavioural** default in §3 and §7. It may not set a ceiling (§5.2), an invariant
(§5.3), a class ceiling, or an identity. A custom mode is a file in the `modes/` directory of the config
root and is allowlisted by name in `ConfigDocument.mode`; an unlisted name is a load error (§6.5).

Mode composition with per-peer settings: deployment default → mode default → peer or link override →
per-operation override (§6.2). Per-peer values can replace mode values in either direction, but only
within the ceilings.

## 5. Ceilings, invariants and what config may never do

### 5.1 Why there are two lists

Behaviour can be replaced by any later authorized scope. Ceilings bound how far behaviour may go.
Invariants cannot be configured at all. Keeping them separate prevents an operator from "relaxing" a
ceiling by setting a mode default above it.

### 5.2 Hard ceilings (non-overridable)

| Key                           | Default | Ceiling (reject above) | Source                  |
| ----------------------------- | ------- | ---------------------- | ----------------------- |
| `fanout.max_hops`             | 3       | **8**                  | #9144 §5.1, Q1          |
| `fanout.peer_breadth`         | 4       | 16                     | this record; #9144 §5.3 |
| `fanout.deadline_ms`          | 5000    | 60000                  | #9144 Q5                |
| `fanout.per_peer_timeout_ms`  | 2000    | 30000                  | #9144 Q5                |
| `ceilings.max_response_bytes` | 4 MiB   | 64 MiB                 | Part B budgets          |
| `ceilings.max_work_units`     | 10000   | 1000000                | Part B budgets          |
| `ceilings.dedup_retention_s`  | 86400   | 604800                 | Part B finite dedup     |

The ceilings in the second and later rows are proposed starting values, changeable by amending this
record. The `fanout` values are fixed by #9144. The table's "ceiling" column is the reject boundary.

### 5.3 Invariants: config may not disable these

Config fails to load if any key or mode attempts to:

1. disable or bypass the acceptance gate (#9141 §1, four checks);
2. widen a peer beyond its class ceiling, or grant relay, completeness or cross-tenant access through the
   class field (#9141 §4);
3. make an unknown peer acceptable (`class` unknown means deny everything);
4. turn off provenance, the per-hop chain, or rejection observability (Part B);
5. set dedup retention to unbounded or disable duplicate and loop rejection (#9144 §5.2);
6. disable emergency peer disable or key revocation (the `control` link is always on);
7. set `max_hops` above 8, or any ceiling in §5.2 above its ceiling;
8. let an operation override skip authentication, signature, replay or tenant-boundary checks (summit
   crosscut).

Each attempt is a load error with the key path and the invariant number, so an operator sees which
rule refused the value.

## 6. File format, layering, merge and validation

### 6.1 File format and location

- One UTF-8 JSON file per deployment, with a `modes/` directory of JSON files (Q1 records why JSON and
  not TOML).
- Canonical encoding for hashing: sorted keys, no insignificant whitespace, UTF-8, integers without
  fractions. The **resolved-config digest** is SHA-256 of the canonical encoding of the effective
  document (summit crosscut: "resolved-config digest").

### 6.2 Layers and precedence

Resolution runs in this order. Later layers replace earlier behavioural values only within ceilings.

1. Built-in invariants and §5.2 ceilings (fixed by the binary).
2. Deployment defaults (the top-level `links`, `storage`, `fanout` values in the active document).
3. Mode behavioural defaults (the named mode).
4. Tenant, peer, link and storage overrides (`peers[]`, `peers[].overrides`).
5. Permitted per-operation overrides (`peers[].overrides.<op>`).

Layer 4 and 5 may only narrow an authority or resource ceiling. They may replace a behavioural value
either way. Example (summit crosscut, applied to this schema): deployment `sync_scope = "metadata"`,
mode `sync_scope = "full"`, tenant allows only `metadata`: resolves to `metadata`. A
hard `max_hops` ceiling of 8 with mode default 3 and peer override 1 resolves to 1; a peer override of 9
is rejected at load.

### 6.3 Merge rules

- Scalars: the highest layer that sets the key wins, subject to §5.2 and §5.3.
- Arrays of peers: merged by `peer_id`. A peer appears once per deployment; a duplicate `peer_id` in
  one file is a load error. Peers from a mode bundle are not added; a mode cannot introduce a peer.
- Objects (`links`, `storage`, `fanout`): merged key by key. An absent key inherits; an explicit `null`
  clears to the built-in default for that key, never to "no limit".
- `extensions`: merged by namespaced key. Retained verbatim, not executed.
- A key set to a value outside its declared type or range is a load error (no coercion).

### 6.4 Unknown keys

- **Trust-affecting settings fail closed.** Any unknown key inside `peers[]`, `peers[].overrides`,
  `fanout`, `ceilings`, `storage` or the `control` link is a load error. These sections change who is
  trusted or how much work a peer may cause.
- **Other unknown keys** are a load error at the top level in schema version 1. Only keys under
  `extensions` (namespaced `vendor.name`) may be unknown and retained. Default: strict everywhere, and
  forward compatibility is by minor version adding optional keys (§6.7).

### 6.5 Validation at load

Validation runs in this order and stops at the first failure with a path and an invariant number:

1. Parse as canonical JSON; reject duplicate keys and non-canonical numbers.
2. `schema_version` supported; `mode` allowlisted; `revision` not lower than active.
3. Ceilings and invariants (§5.2, §5.3); peer-class consistency: `relay_granted` only with
   `fully_trusted`; `send_only` has no `sync.accept`; `accept_signed_from` has no relay; `unknown` has
   no operations.
4. Each adapter named in `links` or `storage` is advertised by the binary with the needed capability.
5. Each `secret_ref` resolves (or is marked pending and blocks activation).
6. The resolved document is computed and its digest recorded.

A failure at any step rejects the revision. Activation never partially applies (§6.6).

### 6.6 Activation and rollback

Two-phase, per summit crosscut. Validate (§6.5) against all selected adapter capabilities and peer
protocol constraints, then commit a revision. On failure the previous revision stays active. Rollback
restores the prior configuration, **not** already-completed external effects; the record must say so
when a rollback is requested. Config changes take effect at the next evaluation against the new
snapshot (#9141 §2: class changes are per peer, versioned, no in-place edit).

### 6.7 Versioning

- A new **minor** `schema_version` adds optional keys only; old readers reject the file only if it sets
  a key they do not know inside a trust-affecting section (§6.4), so a minor bump never silently
  changes trust.
- A new **major** changes meaning and needs a migration step and an explicit reader cut-over.

### 6.8 Explain output

The binary can print the resolved document with the source of each value (which layer set it) and the
digest. Secrets appear as references. This output is what an operator reviews before activation.

## 7. Fan-out keys (#9144) and peer and namespace knobs in one schema

### 7.1 The fan-out keys

These are the keys #9144 §8 and §11.1 name. Each has the default and ceiling given there or here.

| Key                          | Type                                                      | Default           | Validation                                                                          |
| ---------------------------- | --------------------------------------------------------- | ----------------- | ----------------------------------------------------------------------------------- |
| `fanout.remotes[]`           | list of `{peer_id, priority, relay, timeout_ms?, scope?}` | `[]` (local-only) | `peer_id` must exist in `peers[]`; `priority` integer; ties broken by list position |
| `fanout.remotes[].relay`     | bool                                                      | `false`           | `true` only if the peer is `fully_trusted` with `relay_granted` (§7.3)              |
| `fanout.max_hops`            | integer                                                   | `3`               | `1 ≤ n ≤ 8`; **a value above 8 is rejected** at load                                |
| `fanout.deadline_ms`         | integer                                                   | `5000`            | `1 ≤ n ≤ 60000`                                                                     |
| `fanout.per_peer_timeout_ms` | integer                                                   | `2000`            | `1 ≤ n ≤ 30000`; must be ≤ `deadline_ms` or it is clamped and logged                |
| `fanout.peer_breadth`        | integer                                                   | `4`               | `1 ≤ n ≤ 16`                                                                        |

Notes that come from #9144:

- `priority` is ordering only. It is not authority and it does not change class (#9144 §3.2). Trust
  class is a gate, not a score.
- Eligibility: a peer is asked only if its class allows `query.out`. A `send_only` peer is never asked.
- `scope` in `remotes[]` declares the allowed query scope and retention (#9144 Q6). Default: the peer's
  `scope` in `peers[]`.
- `max_hops` is set by the origin and signed inside the origin query (#9144 §5.1). A value above the
  ceiling is rejected at **load**, not clamped, so an operator sees the mistake.

### 7.2 Timeouts

`timeout_ms` on a remote entry overrides `per_peer_timeout_ms` for that peer, within the ceiling in
§5.2. Each remote's effective timeout is `min(its timeout_ms or per_peer_timeout_ms, deadline_ms)`.

### 7.3 Relay permission (summit S1.55, #9141 §3)

Relay is effective only when all of these hold, and config validation rejects the rest:

- the relaying peer's class is `fully_trusted`;
- `relay_granted = true` on the peer entry;
- `fanout.remotes[].relay = true`;
- the current authorization and budgets allow it.

`accept_signed_from` and `send_only` peers can never relay, even if a file asks for it (load error).

### 7.4 One schema for peer, trust and namespace knobs

The Part B knobs (peer class, operations, relay grant, sync scope, budgets, retention, rating weights)
and the Part A knobs (namespace registry reference, `shorthand_policy`, `defaultSlot`, resolver hint) sit
in the **same** document, in separate sections so the planes stay separate (summit crosscut: "peer,
link, storage, data-tier, and namespace axes remain distinct").

| Plane        | Section                      | Owner of meaning                               |
| ------------ | ---------------------------- | ---------------------------------------------- |
| Mode         | `mode`, `modes/`             | #9111                                          |
| Peer / trust | `peers[]`                    | #9141                                          |
| Link         | `links`, `peers[].overrides` | #9111 (kinds fixed by their protocols)         |
| Storage      | `storage`                    | storage owners; #9082 for ID backends          |
| Tier         | `tiers`                      | #9084 for content lifecycle; #9111 for the map |
| Fan-out      | `fanout`                     | #9144 meaning; #9111 format                    |
| Namespace    | `namespaces`                 | #9116                                          |

Defaults for Part A namespace knobs: `shorthand_policy` is absent on every namespace, so a bare-string
request returns `missing_shorthand_policy` (Part A, #9116). No global namespace registry is
consulted (#9116).

## 8. Requirement traceability

| Requirement (source)                                                                                               | Where answered            |
| ------------------------------------------------------------------------------------------------------------------ | ------------------------- |
| #9111 §7: modes, link kinds, storages, tiering as config; primitives for mechanism                                 | §2, §3.3–3.5, §4          |
| #9111 §7: peer/trust knobs (Part B) and namespace knobs (Part A) in one schema                                     | §7.4, §3.2                |
| #9111 acceptance: open questions with chosen defaults                                                              | §9                        |
| #9111 acceptance: implementation issues named for implementable parts                                              | §10                       |
| #9106 Part A: server-local namespace registry, no global lookup                                                    | §3.1, §7.4                |
| #9144 §8: `remotes[]`, `relay`, `max_hops`, `deadline_ms`, `per_peer_timeout_ms`, `peer_breadth` keys and defaults | §7.1, §5.2                |
| #9144 §11.1: validation rejects `max_hops` above 8; a config test per default and ceiling                          | §7.1, §10 (issue 9111-F1) |
| #9141 §1–§4: classes, the gate, operations, relay grant, `policy_version`, expiry                                  | §3.2, §5.3, §6.6          |
| #9141 §5 defaults (relay not implied; class change without grandfathering; expiry = unknown)                       | §3.2, §6.6                |
| #9110 / S0.1–S0.17: primitives, adapters, hooks; adding a backend is additive                                      | §2, §3.4, §6.7            |
| Summit crosscut: resolution order, ceilings narrow, fail-closed, two-phase activation                              | §6.2–6.6                  |
| Summit Part B: config may not disable validation, bounds, provenance, dedup, revocation                            | §5.3                      |

## 9. Open questions with chosen defaults

Each question below is recorded with the default this record chooses. A detached session does not call
`question`; each default holds until the owner changes this record by amendment, and silence is not a
decision.

- **Q1. Which file format?** Default: **JSON** canonical document, plus `modes/*.json`. Reason: one
  canonical encoding makes the resolved digest and cross-runtime fixtures (Rust and TypeScript,
  summit crosscut gate 4) byte-stable; TOML's comments and whitespace do not survive hashing. Operators
  may write a human-friendly source that compiles to JSON; the compiled JSON is what activates.
- **Q2. Where does the file live?** Default: `<config-root>/config.json` with `<config-root>` from the
  deployment's existing config directory. The file is never read from the working directory, to keep
  activation explicit.
- **Q3. Should unknown keys outside trust sections be a load error?** Default: **yes, in schema v1**, so
  typos fail loudly; forward compatibility comes from `extensions` and minor versions (§6.4, §6.7).
- **Q4. Is `max_hops` clamped or rejected above 8?** Default: **rejected at load**, with the key path.
  Reason: a clamp hides a misconfiguration. #9144 Q1 states the ceiling; this record makes it a load
  error.
- **Q5. Is the fan-out default `max_hops` 3 the right default?** Default: 3, ceiling 8 (#9144 Q1, owner
  B16 benchmark). The default is configurable per mode and per deployment; the ceiling does not move
  without an amendment here.
- **Q6. Should `priority` also accept routing observations?** Default: **no**, as #9144 Q2. Routing
  observations may reorder only when the operator opts in, and then as advisory signed observations
  (summit Part B, B23). Not in schema v1.
- **Q7. Do modes ship as built-ins only, or may operators add modes?** Default: **both**. Built-in
  modes are the three in §4. An operator mode is a file in `modes/` and must be named in
  `ConfigDocument.mode`. A mode cannot set a ceiling, an invariant or a class (§4).
- **Q8. Does a mode change a peer's class?** Default: **no**. A mode is a behavioural default. Class is a
  per-peer, versioned policy (#9141 §2). A mode that appears to change class is a load error.
- **Q9. Are per-operation overrides allowed to pick a different adapter?** Default: **yes**, from the
  adapters advertised by the binary, with the same gate and tenant boundary (summit crosscut). An
  override cannot skip checks (§5.3 item 8).
- **Q10. Does a config change take effect immediately?** Default: **at the next evaluation against the
  new revision**, never mid-operation. In-flight operations finish against the revision they started
  with (#9141 §5.3, no session grandfathering for class changes).
- **Q11. Does rollback undo external effects?** Default: **no.** Rollback restores the prior configuration
  only. Completed sends, delivered answers and written records are not reversed by a config rollback;
  the record must say so (summit crosscut).
- **Q12. Are the non-fan-out ceilings in §5.2 (response bytes, work units, dedup retention) fixed now?**
  Default: **these are proposed starting values** and are recorded as such. They can be amended here
  without changing the schema. The fan-out values come from #9144 and are fixed.
- **Q13. Does `control` ever become a config-disabled link?** Default: **no.** Emergency peer disable and
  key revocation are always available (§5.3 item 6; Part B).
- **Q14. Is tier placement a config key or a signed record field?** Default: **both**, as the summit
  crosscut says. The record carries the signed tier identifier; config maps the identifier to placement
  (§3.5). Changing the map does not rewrite records.

## 10. Implementation issues to file later (not filed by this record)

These are named, not filed. Each is a granular, implementable part. The summit crosscut names 9111-01
to 9111-05 in `crosscut/implementation-slices.json`; the titles below follow that list and add the
fan-out and validation parts this record defines.

| Name    | Title (proposed)                                                                    | Owns                    |
| ------- | ----------------------------------------------------------------------------------- | ----------------------- |
| 9111-01 | Define versioned configuration schema and hard invariants                           | §3.1, §5.2, §5.3, §6.1  |
| 9111-02 | Implement mode bundles and safe override precedence                                 | §4, §6.2, §6.3          |
| 9111-03 | Model peer, link, storage, tier, and namespace planes independently                 | §3.2–3.5, §7.4          |
| 9111-04 | Add capability negotiation and downgrade rejection                                  | §6.5 step 4, §3.4       |
| 9111-05 | Stage config activation and rollback by revision                                    | §6.6, Q10, Q11          |
| 9111-F1 | Validate fan-out config keys with a default and ceiling test for each (#9144 §11.1) | §7.1, §5.2              |
| 9111-F2 | Print resolved config with per-value provenance and digest (explain output)         | §6.8                    |
| 9111-F3 | Canonical JSON loader with duplicate-key and unknown-key rejection                  | §6.1, §6.4, §6.5 step 1 |

F1 and F3 are the first implementable parts that do not depend on a new adapter. F1 is the #9144 §11.1
item that was to be recorded as a comment on #9111; this record takes it into the schema instead, and
the comment should point here.

## 11. What this record does NOT decide

- The peer class matrix, the gate and the signer binding: #9141 and its slices.
- Partial answers, timeouts as response states and completeness reporting: #11414 (child of #9144).
- Receipt schema: #10250.
- Namespace registry and URI syntax: #9116 and #9117.
- Storage backend semantics and IndexedDB verdict: #9082.
- Content lifecycle, copy-vs-reference and retention of content: #9084.
- Tenancy, SSO: #9083 and #9077.
- Reputation as an input to priority: Part B V4.
- Any code, any schema file, any migration. This record writes none.

## 12. Acceptance for this record

- Dated folder with this file, under `doc/`.
- Config schema, split rule, peer and namespace knobs in one schema (§2–§7).
- Fan-out keys with defaults and the `max_hops` ceiling of 8 rejected above (§7.1, §5.2).
- File format, layering, merge and validation rules, including fail-closed unknown keys for trust
  sections (§6).
- Open questions with chosen defaults (§9).
- Implementation issues named, not filed (§10).
- Traceability to #9111, #9106, #9144, #9141 and #9110 (§8).
