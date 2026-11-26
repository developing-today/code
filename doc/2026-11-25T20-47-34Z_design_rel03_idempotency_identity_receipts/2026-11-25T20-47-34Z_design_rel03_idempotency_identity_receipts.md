# REL-03 — Idempotency identity and stable structured receipts

Issue: [#9513](https://github.com/developing-today/code/issues/9513)
(slice `REL-03` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the REL-03 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the identity, lookup, receipt and compatibility
contract a separately scoped implementation issue can be filed
against. The component row says: "operation id + key/request digest
and stable effect receipt shared by commands", with first oracles
`pmo_rel_same_key_same_digest_replays_receipt` and
`pmo_rel_same_key_different_digest_conflicts`, and negatives
"canonical request ordering changes digest, principal scope leak,
timeout after side effect". This record _decides_ what the
idempotency scope is, in which order validation, digest checking,
authorization and lookup run, what a stored receipt holds and which
of its fields may never change, what a replay returns, what happens
to operations accepted before this contract, and exactly how the
landed DSP-01 acceptance ledger has to change. It authorizes no
code, schema, migration, deployment or live acceptance; those remain
separately gated stages (see §10).

Governing decisions consumed, not re-opened:

- **Component #9209 §4** (`components/9209-leases-fencing-outbox-direct-completion.md:65-67`):
  operation identity is a globally unique `operation_id` plus an
  "idempotency key scoped to actor/client and stable request
  digest"; "Same idempotency key + same digest returns existing
  operation/receipt; same key + different digest is deterministic
  conflict." The suggested operation states (`pending`,
  `delivering`, `confirmed`, `retryable_failure`, `conflict`,
  `terminal_failure`, `unknown`) belong to REL-04's state table; this
  record only holds the slot.
- **Component #9209 §5 step 7 and §6** (`:87`, `:91`): "A lost final
  response can be retried with same idempotency key and returns
  receipt", and "Never POST a second comment solely because first
  response timed out."
- **Component #9209 error envelope** (`:105`): the stable code
  `IDEMPOTENCY_KEY_REUSED` exists in the design vocabulary, with
  operation ID, retryability and recovery action.
- **Architecture shared Command/Result** and its digest contract, as
  landed in `contracts.rs` by DST-01 (#9451): `request_digest` is
  `sha256:` over `ASCII(domain) || 0x00 || JCS(intent DTO)` under the
  registered domain `fleet.command-intent/v1`; the intent DTO
  excludes operation/key ids, delivery ids, receipt times, output
  format, wait/timeout and dry-run.
- **CLI-02 record** (#9438,
  `doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`
  §6 items 4–5): semantically unordered argument sets are normalized
  by the command schema at construction, not by canonicalization;
  "Idempotency lookup is scoped by authenticated principal +
  authority, and authorization is re-checked at acceptance; a
  matching digest recovers the original operation, and never grants
  anything."
- **CEN-04 record** (#9434,
  `doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`
  §3.9): REL-03 owns idempotency identity and the stable receipt;
  REL-04 (#9514) owns outbox atomicity; **no persisted operation or
  receipt table exists on main**; `creation_outbox` is a scoped
  creation producer and is not generalized by implication; any
  receipt field that must persist enters through CEN-04 §5 first.
- **DSP-01** (#9445, closed): the delivery GUID is provenance only,
  never acceptance identity; a refusal records nothing.
- **DSP-05 record**
  (`doc/2026-11-25T19-35-00Z_design_dsp05_session_capability_delivery_receipts/`)
  and **REL-06 record**
  (`doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  both consume REL-03's `operation_id`, key and digest and add no
  second idempotency store. This record must not break either.
- The issue body's compatibility line: "old operations remain
  legacy/unknown; no fabricated receipts", and rollback "disable only
  the new capability; preserve pending intents, artifacts, existing
  state, and remote effects."

## Verified state at `origin/main` `6d423fba3`

Two command shapes exist, and neither carries the REL-03 contract
end to end.

| Surface                                                     | What exists                                                                                                                                                                                                                                                                                                     | What is missing for REL-03                                                                                                                                                                                                  |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `work/contracts.rs` `Command` (`:608-660`)                  | Wire `fleet.command/v1`: `operation_id`, `idempotency_key` (doc comment: "scoped by authenticated principal and authority at lookup time"), `request_digest`, optional `delivery_id` (excluded from the digest), `dry_run`; `intent_dto()`, `compute_request_digest()`, `verify_request_digest()` (`:728-768`). | No lookup, no store, no receipt. Nothing performs the scoped lookup the doc comment promises.                                                                                                                               |
| `work/contracts.rs` `AuthenticatedPrincipal` (`:773-780`)   | `principal_ref` + verified `capabilities`; `evaluate_authorization` never reads `actor_ref`.                                                                                                                                                                                                                    | Not yet the scope input of any ledger.                                                                                                                                                                                      |
| `work/contracts.rs` `Outcome` (`:513-534`)                  | `conflict` ("same key with a different digest") and `uncertain` ("timeout after a remote effect; the receipt is resumable") already exist.                                                                                                                                                                      | No error code for the conflict; `ContractError` has no idempotency variant.                                                                                                                                                 |
| `work/operations.rs` DSP-01 `AcceptanceLedger` (`:270-329`) | `receipts: BTreeMap<String, Receipt>` keyed by the bare key (`ghcmd:v1:<repo>#<issue>:comment:<id>:<verb>`); `accept()` authorizes, then returns the stored receipt if the key exists — **no digest comparison**; a refusal records nothing; one effect per key.                                                | Key namespace is global, not principal-scoped; no digest; `Receipt` has no `operation_id`, principal, digest or status; the DSP-01 `Command` carries no arguments, so a `/fleet priority` value cannot be part of identity. |
| Persistence                                                 | None for operations or receipts (CEN-04 §3.9). The DSP-01 ledger is an in-memory model; its broker adapter persistence is not on main.                                                                                                                                                                          | Storage is REL-04/REL-07 plus the #9198 store owner, admitted through CEN-04 §5.                                                                                                                                            |

The DSP-01 gap is concrete, not theoretical. Its key names the
comment and the verb but not the verb's argument. A `/fleet priority`
comment that is edited from one value to another keeps the same
comment id, so it keeps the same key, and today's ledger would hand
back the receipt for the _old_ value as a successful duplicate. That
is precisely the "same key, different intent" case that must be a
conflict.

## 1. Intent

Every Fleet command that can change state gets one identity that
survives retries, redeliveries, lost responses and restarts, and one
stored receipt that describes the accepted operation the same way
every time it is read back. Retrying with the same key and the same
intent is always safe and never applies anything twice. Reusing a key
for a different intent is always refused, visibly, and never resolved
by guessing which intent was meant. One principal's key can never
read, replay or block another principal's operation.

## 2. Ownership: who writes what

| Concern                                                                                                    | Owner                                                                                        |
| ---------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Idempotency scope, lookup order, replay/conflict decision, receipt identity and immutability (this record) | **REL-03** — `src/work/operations.rs` model and tests; `contracts.rs` serialization fixtures |
| Wire `Command`/`CommandResult`, intent DTO, digest function and domains                                    | DST-01 / CLI-01 (landed); CLI-02 for parser construction                                     |
| Atomic local state + journal + outbox intent; the status transition table                                  | REL-04 (#9514)                                                                               |
| Remote preconditions, effect-specific read-back, `unknown` resolution                                      | REL-06 (#9516)                                                                               |
| Persisted tables, columns, migration and rollback of storage                                               | REL-07 + the #9198 store owner, through CEN-04 §5                                            |
| Operator inspection of `unknown`/`conflict`/stuck operations                                               | REL-08                                                                                       |
| Webhook verb grammar, routing, delivery dedupe                                                             | DSP-01 (landed model), DSP-04, #9203                                                         |
| `main.rs`, the registry, CLI registration                                                                  | CLI-13 alone                                                                                 |

REL-03 adds no queue, database, table, service, scheduler, digest
domain, executable or GitHub client.

## 3. Identity

### 3.1 The idempotency scope

The lookup key is the **scope tuple**

```text
(principal_ref, authority_id, idempotency_key)
```

- `principal_ref` comes from `AuthenticatedPrincipal` as supplied by
  the auth layer. It is never `actor_ref`, never a login parsed from
  a payload and never the session id.
- `authority_id` is the command's `work.authority_id`. For a DSP-01
  webhook command, whose work pair is not resolved until DSP-04, it
  is the authority that owns the repository in the `ResourceRef`;
  DSP-04 supplies that mapping and REL-03 does not invent one.
- `idempotency_key` is opaque to REL-03 beyond the format rules in
  §3.3.

The same key under two principals is two independent operations.
Principal B presenting principal A's key gets B's own scope: it
neither sees A's receipt nor collides with A's digest. That is the
"principal scope leak" negative, and it is closed by construction
rather than by a check that could be forgotten.

### 3.2 The request digest

- The digest is `Command::compute_request_digest()`: domain
  `fleet.command-intent/v1` over `intent_dto()`. REL-03 uses it as
  landed and adds no domain.
- The ledger **recomputes** the digest and compares it with the
  carried `request_digest` before lookup. A mismatch is
  `invalid_schema`, outcome `rejected`, nothing recorded. A carried
  digest is a fingerprint the client claims; the ledger never stores
  a digest it did not compute.
- The DSP-01 webhook command gains a digest over the same intent
  shape: `kind` is `github.slash_command.<verb token>`, `arguments`
  holds the parsed verb arguments (for example the `/fleet priority`
  value), and the work pair is the DSP-04 resolution. Until DSP-04
  supplies the pair, the DSP-01 path keeps its current key-only
  behavior **and** may only accept verbs whose parsed arguments are
  empty; a verb that carries arguments is refused with
  `invalid_schema` rather than accepted without a digest. This is the
  only staged step, and it is fail-closed.

Canonical ordering, the issue's first negative, is fixed as follows:

| Change between two presentations                                                                     | Same digest?                                                                | Why                                                      |
| ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- | -------------------------------------------------------- |
| Object keys in a different order                                                                     | yes                                                                         | JCS sorts keys                                           |
| New `delivery_id`, `operation_id`, wait/timeout, output format, `dry_run`                            | yes                                                                         | excluded from `intent_dto()`                             |
| Array elements in a different order (for example `reason.evidence_refs`)                             | **no**                                                                      | JCS preserves array order; it performs no set reordering |
| A semantically unordered set (for example labels) in a different order                               | yes, only if the command schema normalized it at construction (CLI-02 §6.4) | canonicalization never does it                           |
| Different `expected_work_version`, `lease_generation`, `observed_snapshot_ref` or `owner_policy_ref` | **no**                                                                      | a changed precondition is a changed intent               |
| Unicode in a different normalization form                                                            | **no**                                                                      | JCS performs no normalization                            |

### 3.3 Key format

- 1 to 200 bytes of printable ASCII with no whitespace. Anything else
  is `invalid_schema` before lookup.
- The prefix `ghcmd:` is reserved for keys minted by the webhook
  forwarder. A CLI or API caller presenting a `ghcmd:` key is refused
  with `invalid_schema`, so no caller can forge the identity of a
  webhook acceptance.
- The key is never derived from the delivery GUID (DSP-01) and never
  from a timestamp chosen by the server.

### 3.4 `operation_id`

- On first acceptance the ledger stores the presented `operation_id`
  as the operation's identity. It must be unique across all scopes; a
  first acceptance whose `operation_id` already names another stored
  operation is a conflict (`OPERATION_ID_REUSED`, outcome `conflict`,
  nothing recorded).
- On replay the **stored** `operation_id` is returned. A replay that
  presents a different `operation_id` is still a replay, because
  `operation_id` is excluded from the digest; the result carries the
  stored id and the warning `operation_id_superseded`. Clients must
  read the id from the result, not assume their own.

## 4. Lookup order

Acceptance runs these steps in this order. Each failing step returns
a typed result and writes nothing.

1. **Parse and validate** (CLI-02 / `Command::validate`). A parse
   failure mints no operation.
2. **Key format** (§3.3).
3. **Digest verification** (§3.2): recompute, compare.
4. **Authorization** from the authenticated principal, re-checked on
   every presentation, including replays. A principal whose
   capability was revoked gets `AUTH_DENIED` even for a key it used
   before. The stored receipt stays readable through the REL-08
   operator surface, not through replay. This keeps DSP-01's present
   order (authorize, then look up) and means a replay never discloses
   anything authorization would not.
5. **Dry-run branch.** A dry-run performs the lookup read-only: an
   existing same-digest receipt is reported as `would_replay`, a
   different digest as `would_conflict`, an absent scope as
   `would_accept`. It creates no operation, receipt, journal or
   outbox row (architecture: dry-run is strictly read-only).
6. **Scoped lookup** on the scope tuple:

| Stored state for the scope | Presented digest | Result                                                                                                                                                                                                          |
| -------------------------- | ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| absent                     | any              | **accept**: store the receipt (§5), hand the operation to REL-04's producer; `duplicate: false`                                                                                                                 |
| present                    | equal            | **replay**: return the stored receipt as it stands now, `duplicate: true`; no effect, no new delivery record, no status change                                                                                  |
| present                    | different        | **conflict**: outcome `conflict`, code `IDEMPOTENCY_KEY_REUSED`, retryable `false`, the stored `operation_id`, next action "use a new idempotency key for a new intent"; the stored intent itself is not echoed |
| present, legacy (§8)       | any              | **legacy**: outcome `uncertain`, code `LEGACY_OPERATION`, the legacy record as stored; never accepted as new and never reported as a matching replay                                                            |

Two concurrent first presentations of one scope resolve to exactly
one stored operation. The store's uniqueness on the scope tuple is
the arbiter (REL-04/REL-07 choose the mechanism); the loser re-reads
and takes the replay or conflict row above. The model test
interleaves the two and asserts one operation and one effect.

## 5. The stored receipt

### 5.1 Fields

The receipt is the stored record from which every `CommandResult`
for the operation is rendered. Field names are snake_case; sequences
and versions cross JSON as canonical decimal strings.

| Field                                                                              | Set when                         | Mutable?                                               |
| ---------------------------------------------------------------------------------- | -------------------------------- | ------------------------------------------------------ |
| `operation_id`                                                                     | first acceptance                 | never                                                  |
| `scope.principal_ref`, `scope.authority_id`, `scope.idempotency_key`               | first acceptance                 | never                                                  |
| `request_digest` (as computed by the ledger)                                       | first acceptance                 | never                                                  |
| `kind`, `work`                                                                     | first acceptance                 | never                                                  |
| `accepted_sequence` (ledger-monotonic)                                             | first acceptance                 | never                                                  |
| `accepted_at` (authority DB clock, verbatim)                                       | first acceptance                 | never                                                  |
| `provenance`: `source_ref`, `source_event_ref`, `session_ref`, first `delivery_id` | first acceptance                 | never                                                  |
| `status`                                                                           | first acceptance                 | only by REL-04's transition table                      |
| `effects[]`: requested / observed / verified (`contracts::Effect`) and remote refs | as REL-04/REL-06 record evidence | append or fill only; a verified value is never cleared |
| `status_changed_at`                                                                | each transition                  | yes, with `status`                                     |

The immutable part is the **identity block**. Its serialized bytes
are identical on every read for the life of the operation; that is
what "stable" means in the issue title, and it is what the oracle
compares.

### 5.2 Rendering a result from a receipt

| Receipt `status` (REL-04)                      | `CommandResult.outcome`                                                      |
| ---------------------------------------------- | ---------------------------------------------------------------------------- |
| `pending` (accepted, not yet sent)             | `accepted`                                                                   |
| `delivering`, `retryable_failure`              | `pending_sync`                                                               |
| `confirmed`, all required effects verified     | `verified_complete`                                                          |
| `confirmed` for some required effects, not all | `partial`                                                                    |
| `unknown`                                      | `uncertain`                                                                  |
| `conflict`                                     | `conflict`                                                                   |
| `terminal_failure`                             | `partial` if any effect verified, otherwise `rejected` with the stored error |

A replay renders from the receipt as it stands at the replay, so a
caller retrying after a lost response sees the operation's current
truth, not the answer it would have received the first time.

### 5.3 What a replay never does

A replay writes nothing: no effect, no delivery record, no status
change, no `accepted_at` update and no new sequence number. It never
asks REL-04 to send anything. Delivery retries belong to REL-04 and
reconciliation of `unknown` belongs to REL-06; acceptance is not a
second retry loop.

## 6. Timeout after a side effect

The issue's third negative. A client sends a merge, the remote merge
happens, and the response is lost. The client retries with the same
key and intent.

- The retry is a replay (§4 row 2). It returns the receipt with
  `status: unknown` and outcome `uncertain`, plus the stored
  `operation_id` the client can poll or inspect.
- No new operation is accepted and nothing is resent. Whether the
  merge happened is resolved by REL-06's read-back against the
  original operation, never by acceptance.
- A client that instead mints a **new** key for the same intent gets
  a new operation. That is the client's explicit choice; REL-06's
  remote preconditions (for example the expected head SHA) are what
  stop that second operation from doing harm, and REL-03 does not
  claim otherwise.

## 7. The delta to DSP-01's ledger

Stated as the change an implementation makes to
`pkgs/fleet/crates/fleet-queue/src/work/operations.rs`, so that the
REL-03 rule lives in the one ledger that already exists and no
second idempotency store appears:

1. The receipt map key becomes the scope tuple (§3.1) instead of the
   bare key string.
2. `Receipt` gains `operation_id`, `principal_ref`, `authority_id`,
   `request_digest`, `status` and the provenance fields of §5.1.
3. `accept()` runs the §4 order: format, digest, authorize, lookup.
   It returns a new `AcceptError` that wraps the existing
   `AuthorizationError` and adds `IdempotencyConflict { operation_id }`,
   `OperationIdReused`, `LegacyOperation` and `InvalidKey`.
4. `ContractError`/`ResultError` gain the wire codes
   `IDEMPOTENCY_KEY_REUSED`, `OPERATION_ID_REUSED` and
   `LEGACY_OPERATION`, following the uppercase spelling the 9209
   envelope list and `OWNER_POLICY_*` already use. No reason code is
   added; these are error codes, not reasons.
5. `pmo_dsp_01_command_acceptance_is_durable_separate_from_projection_delivery`
   and `pmo_dsp_01_verb_and_resource_forms_parse` keep passing
   unchanged: a redelivery of the same comment is the same scope and
   the same digest, so it is still exactly one effect.

## 8. Legacy operations

Anything accepted before this contract has no ledger-computed digest
and no scope tuple. At this pin that population is empty on main
(§ Verified state), but the rule is fixed now so that whichever
store the broker adapter persists first is covered:

- Legacy records are marked `legacy_unknown` and keep every field
  they had. No digest, principal, status or effect is backfilled or
  guessed.
- A presentation that matches a legacy key returns outcome
  `uncertain`, code `LEGACY_OPERATION`, with the legacy record. It is
  not accepted as new, because the legacy operation may already have
  applied its effect, and it is not reported as a matching replay,
  because nothing proves the intent matches.
- `creation_outbox` rows stay inside their own scope (CEN-04 §3.9)
  and are not legacy operations of this ledger.

## 9. Acceptance oracle

All tests live in `fleet-queue` `src/work/operations.rs` (fixtures in
`contracts.rs`), run with
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`,
and must first be listed by exact name with `-- --list`; a filter
that matches zero tests is not acceptance. The issue's done command
filter `pmo_rel_same_key_` matches the first two.

| Test                                                          | Asserts                                                                                                                 |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `pmo_rel_same_key_same_digest_replays_receipt`                | second presentation returns the byte-identical identity block, `duplicate: true`, one effect total                      |
| `pmo_rel_same_key_different_digest_conflicts`                 | changed argument under the same scope → `conflict` / `IDEMPOTENCY_KEY_REUSED`, stored `operation_id`, ledger unchanged  |
| `pmo_rel_edited_slash_command_argument_conflicts`             | DSP-01 `/fleet priority` edited in place → conflict, not a replay of the old value                                      |
| `pmo_rel_key_scope_is_per_principal_and_authority`            | principal B with A's key gets an independent operation and never A's receipt; same for a second authority               |
| `pmo_rel_replay_after_revocation_is_denied_not_disclosed`     | revoked principal replaying its own key gets `AUTH_DENIED`, no receipt in the result                                    |
| `pmo_rel_digest_ignores_key_order_and_excluded_fields`        | reordered object keys, new `delivery_id`, new `operation_id`, `dry_run` flip → same digest, replay                      |
| `pmo_rel_digest_changes_with_array_order_and_preconditions`   | reordered `evidence_refs`, changed `expected_work_version` → different digest, conflict                                 |
| `pmo_rel_carried_digest_mismatch_rejected_before_lookup`      | forged `request_digest` → `invalid_schema`, nothing recorded, existing receipt untouched                                |
| `pmo_rel_timeout_after_side_effect_replays_uncertain_not_new` | receipt in `unknown` → retry returns `uncertain`, same `operation_id`, no new operation or send                         |
| `pmo_rel_replay_writes_nothing`                               | ledger bytes identical before and after any number of replays                                                           |
| `pmo_rel_dry_run_creates_no_operation`                        | dry-run reports `would_accept`/`would_replay`/`would_conflict` and leaves the ledger byte-identical                     |
| `pmo_rel_operation_id_collision_conflicts`                    | a new scope presenting an existing `operation_id` → `OPERATION_ID_REUSED`; a replay with a new id returns the stored id |
| `pmo_rel_forwarder_key_prefix_reserved`                       | a CLI-sourced `ghcmd:` key → `invalid_schema`                                                                           |
| `pmo_rel_legacy_operation_never_fabricates_receipt`           | legacy key → `uncertain` / `LEGACY_OPERATION`, record unchanged, no backfilled field                                    |
| `pmo_rel_receipt_identity_stable_across_status_changes`       | after `pending → delivering → unknown → confirmed`, the identity block serializes byte-identically                      |
| `pmo_rel_concurrent_first_acceptance_single_operation`        | interleaved first presentations of one scope → one operation, one effect, the other gets replay or conflict             |
| `pmo_rel_receipt_fixture_round_trips`                         | `contracts.rs` golden receipt JSON: snake_case, decimal-string sequence, unknown `status` rejected, not aliased         |

Oracle: an explicit reference model of the ledger (scope map plus
operation-id index), compared field by field with the implementation
after each generated sequence of accept / replay / conflict /
revoke / status-change steps. No mock result is evidence of deployed
behavior.

## 10. Gates and sequencing

- **Design prerequisites named by the issue:** CLI-02 (#9438, record
  merged in PR #13255) and CEN-04 (#9434, record merged in PR
  #13341). Both records exist; this record consumes them.
- **Implementation gates.** The issue states that implementation
  starts only after this slice's granular prerequisite edges are
  resolved. Earlier worker notes on #9513 also hold implementation
  on the trackers #9206 (Command/Result schema), #9198 (storage
  authority) and DSP-08 #9448, which shares `operations.rs` and sits
  in the DSP-04 file-collision chain. This record does not decide
  between those readings; it only records that the file collision on
  `operations.rs` is real and that REL-03's change must sequence
  after DSP-04/DSP-08 edits to that file land.
- **Storage:** any persisted receipt or scope column enters through
  CEN-04 §5 with the #9198 store owner, as REL-04/REL-07 work. REL-03
  ships the model and fixtures only.
- **Downstream consumers:** REL-04 (#9514) needs §4 and §5 before its
  outbox can key intents; REL-06 (#9516) needs §5.1's identity block
  for read-back evidence; DSP-05, DSP-09 (#9449) and HYD-07 consume
  `operation_id` as defined in §3.4.
- **Rollback:** disable the new acceptance path and fall back to the
  DSP-01 key-only ledger for argument-free verbs only (the §3.2
  fail-closed stage). Stored receipts, pending intents and remote
  effects are kept; nothing is deleted or rewritten.

## 11. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records which part of this design speaks to each group of
allocated IDs. It is design coverage, not implementation.

| IDs                                                                                                                                    | Theme                                                             | Where addressed                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| C-R138, C-R140, O-R032, P-R006, P-R021, P-R053, P-R068, H-R015                                                                         | stable operation ids, durable intent, nonduplicate effects        | §3, §4, §5, §7                                                                                                  |
| C-R139                                                                                                                                 | outbox states                                                     | §5.1 slot, §5.2 mapping (REL-04 owns)                                                                           |
| C-R086, C-R087, C-R091, C-R092, C-R093, O-R014, O-R062, O-R074, H-R012, O-R011                                                         | authenticated commands, replay protection, no double launch       | §3.1, §3.3, §4 step 4, §7                                                                                       |
| P-R023, P-R043, P-R044, P-R058, P-R059, H-R016                                                                                         | lost acknowledgment, GitHub unavailable, crash after mutation     | §5.3, §6                                                                                                        |
| C-R007, C-R145, P-R022, P-R033, P-R038, P-R045, P-R060                                                                                 | direct completion and partial outcomes                            | §5.2 (`partial`, `uncertain`)                                                                                   |
| P-R041, P-R042, P-R046, P-R047, P-R048, P-R056, P-R057, P-R061, P-R062, P-R063                                                         | claim races, fencing, managed edits, late comments, cycles        | §4 concurrency row; preconditions in the digest (§3.2); otherwise owned by REL-01/02/05/06 and not decided here |
| C-R005, C-R009, C-R107, C-R108, C-R133, H-R001, H-R017, H-R024, H-R031, O-R022, O-R023, O-R038, P-R009, P-R025, P-R027, P-R028, P-R029 | cohesive CLI, preserved inventory, structured errors and receipts | §5 receipt and error codes; `--idempotency-key` flag stays CLI-02/CLI-13's                                      |

## 12. Non-goals

- No storage, table, column, migration or retention policy; key
  expiry is not introduced, so a key is never silently reusable as a
  new operation.
- No status transition table, outbox, delivery, retry or relay
  (REL-04), and no remote read-back or `unknown` resolution (REL-06).
- No change to the digest function, the intent DTO or the digest
  domain registry.
- No CLI flag, registration or `main.rs` change (CLI-02, CLI-13).
- No work-pair resolution for webhook commands (DSP-04).
- No change to #6533 or its live acceptance.
