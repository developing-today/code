# CLI-12 — Typed work-resource link command semantics

Issue: [#9441](https://github.com/developing-today/code/issues/9441)
(slice `CLI-12` of [#9206](https://github.com/developing-today/code/issues/9206),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the CLI-12 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "`work.link` only, exact source spelling … `work/contracts.rs`
resource-ref types + `work/authority.rs` authorization +
`work/journal.rs` provenance; no resolver, schema, add/remove
subcommands or alias invented here. Link mutation does not belong to
read-only inspect/status." Its oracle: "ambiguous provider/repo/resource
refs fail without mutation and accepted link is journaled." Main now
has a links store and a `link` command, so this record puts `work.link`
on top of them and decides the reference forms, version check,
authorization, idempotency and result. It authorizes no code, schema
or deployment (§7).

Governing decisions consumed, not re-opened:

- **CLI-02** (`doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`):
  the `fleet.command/v1` envelope, strict parsing and the common flags
  (`--work`, `--repo`, `--json`, `--dry-run`, `--expected-version`,
  `--idempotency-key`). Registration stays with CLI-13.
- **DSP-04** (`doc/2026-11-25T21-50-00Z_design_dsp04_item_resolution_event_router/`):
  locator forms; a bare number is never resolved across repositories.
- **REL-03** (`doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  operation id, principal-scoped idempotency key, digest and stored
  receipt; same key with a different digest is a conflict.
- **DSP-08** (`doc/2026-11-25T21-52-00Z_design_dsp08_directives_owner_authority/`):
  grant is necessary, floors are not sufficient; no invented role names.
- **Component #9208 §2 and §4:** typed relations are Fleet data;
  "Native GitHub relationship ingestion retains provider provenance",
  and Project hierarchy and native blockers stay distinct (DEP-04).

## Verified state on main

Read at `origin/main` on 2026-11-25:

- **Store.** `queue_links` (#14170 Q6): `task_id`, `other_kind`
  (`issue`, `pr`, `task`), `other_repo`, `other_number`,
  `other_task_id`, `link_kind`, `link_key`, `created_by`, `created_at`,
  `removed_at`, `removed_by`, with a unique active index on
  `(task_id, link_kind, link_key)`. Removal is soft.
- **Kinds.** `turso_queue/links.rs` `LINK_KINDS`: `implements`,
  `pr_for`, `parent`, `child`, `split_from`, `supersedes`,
  `duplicate_of`, `related`. `pr_for` needs a pull-request target and
  `implements` an issue target.
- **Verbs.** `/link-add` and `/link-remove` (routed in
  `stacks_outbox.rs`). Each writes the row, a `linked`/`unlinked` event
  and a state-neutral history row in one `BEGIN IMMEDIATE` batch. A
  duplicate active add returns `changed: false`.
- **Targets** are checked for shape only: `owner/repo` with exactly one
  slash and a positive number. Nothing confirms the resource exists or
  that an `--issue` number is not a pull request.
- **Actor.** The row's `created_by` is the request's `worker`, which
  defaults to `"fleet queue link"` when absent.
- **No version, key or receipt.** The verbs take no expected version
  and no idempotency key, check no grant, and return the row, not a
  `fleet.result/v1`.
- **Client.** `repo-queue link add|remove|list <task-id> [--kind]
[--issue owner/repo#n | --pr owner/repo#n | --task id] [--worker id]`
  (`client/support.rs`, `client/commands_query.rs`).
- **Contracts.** `work/contracts.rs` has `WorkRef` and
  `fleet.result/v1` (`Outcome`, `Effect`) but no resource-reference
  type.

## 1. Intent

An agent or person says "this work implements that issue" or "this PR
is for this work" once, through the shared command, and gets back a
receipt that says exactly what was linked, by whom, at which work
version. A reference that could mean two things is refused before
anything changes.

## 2. Ownership

| Concern                                     | Owner                              | This slice                               |
| ------------------------------------------- | ---------------------------------- | ---------------------------------------- |
| `work.link` semantics and its contract test | `tests/work_link_contract.rs`      | **owns**                                 |
| `ResourceRef` type                          | `work/contracts.rs`                | read-only consumer; names the shape      |
| Authorization row                           | `work/authority.rs` (AUTH-01)      | read-only consumer; proposes the row     |
| Journal provenance on `linked`              | `work/journal.rs` (JRN-01)         | read-only consumer                       |
| Store and broker verbs                      | `turso_queue/links.rs` (#14170 Q6) | unchanged; `work.link` calls `/link-add` |
| Resource existence and kind lookup          | #9207 source-ref read adapter      | consumer                                 |
| Native GitHub relation ingestion            | DEP-04                             | not here                                 |
| Command registration and aliases            | CLI-13                             | not here                                 |

No new table, resolver, executable or GitHub client.

## 3. The command

`fleet work link` (`work.link`, exact source spelling) creates one
link. It does not remove links and adds no `add`/`remove` subcommands
or aliases; the existing `repo-queue link remove` stays the removal
path until CLI-13 decides the inventory.

```
fleet work link --work <authority>/<work_id> --kind <link_kind>
                (--issue <owner>/<repo>#<n> | --pr <owner>/<repo>#<n> | --target-work <authority>/<work_id>)
                [--expected-version <v>] [--idempotency-key <k>] [--dry-run] [--json]
```

- `--kind` takes the eight existing `LINK_KINDS` spellings, unchanged.
- Exactly one target flag. The kind/target rules on main stay
  (`pr_for` → PR, `implements` → issue).
- Inspect and status commands read links and never write them.

## 4. Resource references

A `ResourceRef` (proposed for `contracts.rs`) is one of:

- `github` + `owner/repo` + number + kind (`issue` or `pull_request`),
- a Fleet `WorkRef` (`authority_id`, `work_id`).

Refused with no mutation, each with its own code:

| Input                                              | Code                        |
| -------------------------------------------------- | --------------------------- |
| bare number, `#123`, or a repo with no owner       | `link_target_ambiguous`     |
| a URL that does not parse to exactly one resource  | `link_target_ambiguous`     |
| more than one target flag, or none                 | `link_target_ambiguous`     |
| a provider other than `github`                     | `link_provider_unsupported` |
| `--issue` naming a pull request, or the reverse    | `link_target_kind_mismatch` |
| `--target-work` in another authority with no grant | `link_unauthorized`         |
| a work linking to itself                           | `link_self_reference`       |

**Existence.** When the #9207 read adapter answers, the target's
existence and kind are checked before the write. When it cannot answer,
a syntactically complete reference is accepted with the warning
`link_target_unverified` and the receipt records
`target_state: unverified`; Fleet never guesses a kind it could not
read. A later read that finds a kind mismatch reports it; it does not
silently rewrite the link.

## 5. Version, authorization, idempotency and journal

Order of checks (all before any write):

1. **Parse** strictly (CLI-02).
2. **Authenticate.** The principal comes from the broker's
   authenticated channel. A `--worker` or `worker` value is never the
   actor for `work.link`; `created_by` records the principal.
3. **Authorize.** Proposed policy row over existing names: `work.link`,
   severity medium, PM role `agent` when it holds the work, or
   `orchestrator`/`owner`; GitHub permission floor `read` on the
   target repository (linking changes nothing on GitHub). A grant is
   still necessary (DSP-08 §3). Refusal: `link_unauthorized`.
4. **Version.** With `--expected-version`, a mismatch against the
   work's current version returns `conflict`, code `VERSION_CONFLICT`,
   the current version, and next action "refresh and retry". Without
   it, the write proceeds and the receipt records the version it saw.
5. **Idempotency.** With `--idempotency-key`, REL-03 lookup: same key
   and digest returns the stored receipt; same key, different digest
   is `IDEMPOTENCY_KEY_REUSED`. Without a key, an existing active link
   with the same `(kind, link_key)` returns `verified_complete` with
   warning `already_linked` and no new row (today's behavior).
6. **Write** through `/link-add` in its existing batch, with the
   operation id and principal added to the `linked` event detail, in
   JRN-01's provenance shape.

`--dry-run` runs steps 1 to 5 and writes nothing, not even an event.

**Result.** `fleet.result/v1` with one effect
`{field: "links", requested: <kind, link_key>, observed: <row>,
verified: true|false}`, the work version before and after, the
operation id, and any warnings. Human output is one line naming the
link; JSON is the result object.

## 6. Tests

The owned file is an integration test, so the slice's done command
must name it. The issue's command uses `--lib`, which never runs
`tests/work_link_contract.rs`; the correct form is
`--test work_link_contract`.

| Test                                                             | Proves                                                                                             |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `work_link_requires_version_auth_and_idempotent_receipt` (first) | stale version, missing grant and reused key each refuse with no row; a good call returns a receipt |
| `work_link_ambiguous_refs_fail_without_mutation`                 | every §4 refusal leaves rows, events and history unchanged                                         |
| `work_link_kind_target_rules_preserved`                          | `pr_for` and `implements` rules match `LINK_KINDS` on main                                         |
| `work_link_kind_mismatch_refused_when_verified`                  | an `--issue` that is a PR refuses                                                                  |
| `work_link_unverified_target_is_warned_not_guessed`              | adapter down: accepted with `link_target_unverified`                                               |
| `work_link_actor_is_principal_not_worker_flag`                   | a `--worker` value never becomes `created_by`                                                      |
| `work_link_accepted_link_is_journaled`                           | one `linked` event with operation id, principal and versions                                       |
| `work_link_duplicate_add_is_noop`                                | same link twice: one row, `already_linked`                                                         |
| `work_link_dry_run_writes_nothing`                               | no row, event or history                                                                           |
| `work_link_inspect_never_mutates`                                | inspect/status leave links unchanged                                                               |

Command: confirm `work_link_requires_version_auth_and_idempotent_receipt`
is listed exactly once by
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --features client,queue-broker --test work_link_contract -- --list`,
then run the same target with that filter. A zero-test filter is not a
pass.

## 7. Gates

- **Hard blockers:** CLI-02 (record on main), AUTH-01 (#9429) for the
  policy row, the #9207 source-ref adapter, and the canonical identity
  contract.
- **Persisted change:** none in `queue_links`; the operation id and
  principal ride in the existing event detail. A `ResourceRef` type in
  `contracts.rs` is an additive change under its owner.
- **Owner decisions this record does not make:** the §5 policy row
  floors, and whether `work.link` later grows a removal form (CLI-13).
- **Stages each needing their own evidence and approval:** broker
  wiring, the client command, deployment.
- **Compatibility:** `repo-queue link add|remove|list` and the two
  verbs keep working as today.
- **Rollback:** stop routing `work.link`; links already written stay.

## 8. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                    | Where      |
| -------------------------------------- | ---------------------------------------------------------------- | ---------- |
| C-R005, C-R107, O-R023, P-R009, H-R017 | Cohesive Fleet CLI; names reconciled; no parallel mechanism      | §2, §3     |
| C-R108                                 | `--work`, `--expected-version`, `--idempotency-key`, `--dry-run` | §3, §5     |
| C-R009                                 | Agents understand dependencies and links and act safely          | §4, §5     |
| H-R024, H-R031                         | Original commands, errors and failures stay in coverage          | §3, §6, §7 |

## 9. Non-goals

- No link removal command, no subcommands, no aliases.
- No resolver and no guessing of a bare number's repository.
- No GitHub-side relation write; native relations are DEP-04's.
- No change to `queue_links` or the existing `link` client.
