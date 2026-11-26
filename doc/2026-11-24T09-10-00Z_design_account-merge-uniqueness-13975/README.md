# Account merge: uniqueness audit (slice 1 of #13975)

Refs #13975, #13415, #9077. Report-only: no schema or code writes.
Source checked: `pkgs/idp-provider/src` on `main` at 2026-11-24.

## Method

Every persistent object in the provider was read from its `CREATE TABLE`
or its `Account`/`Identity`/`UpstreamDoor` struct. Each is classified by
per-account cardinality:

- **unique** — at most one per account, enforced or assumed.
- **limited** — a bounded set per account.
- **unlimited** — many per account by construction.

## Inventory: current provider state

| Object                                                              | Store                           | Cardinality per account                                          | Merge treatment                                                                                                                              |
| ------------------------------------------------------------------- | ------------------------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Account row (`account_id`, `display_name`, `local_credential_hash`) | registry config                 | unique (one row per `account_id`)                                | Secondary becomes a `merged_into` shell; primary wins for `display_name` and `local_credential_hash`, with per-attribute override (slice 3). |
| Role set (`roles`)                                                  | registry config                 | limited (owner / orchestrator / agent / read-only)               | Merge-critical. Primary's roles stand; secondary-only roles are surfaced for explicit keep or drop. Never silently granted or dropped.       |
| Identity link `(provider, subject)` → `account_id`                  | `identities` table              | unlimited per account; globally unique per `(provider, subject)` | Repointed to primary, one event per row. Primary key is unchanged.                                                                           |
| Identity history `identity_events`                                  | `identities` table, append-only | unlimited                                                        | Rows are never rewritten. Reads resolve the old `account_id` through the `merged_into` pointer.                                              |
| Audit rows `audit`                                                  | `audit` table, append-only      | unlimited; `identity` column names a subject                     | Not rewritten. Reads resolve through the pointer, as above.                                                                                  |
| Token `sub`                                                         | issued tokens                   | equals `account_id`                                              | Survival decision below.                                                                                                                     |
| Config seed list (`identities` bound to a seeded account)           | config                          | limited (seeded set)                                             | Merge-critical. The seed binding must point at the primary after merge.                                                                      |
| Local-door credential (`local_credential_hash`)                     | registry config                 | unique (empty or one hash)                                       | Merge-critical: at most one credential may survive. Primary's credential wins unless overridden.                                             |
| Upstream door config (`UpstreamDoor`, `ProviderConfig`)             | registry config                 | not per account (global)                                         | Out of merge scope.                                                                                                                          |
| Client registry (`Client`)                                          | registry config                 | not per account (global)                                         | Out of merge scope.                                                                                                                          |

**Merge-critical set** (unique or limited, must have a stated treatment):
`account_id`, `display_name`, `local_credential_hash`, `roles`, and the
seed-list binding.

## Planned objects (sibling workstreams)

| Object                 | Cardinality                          | Constraint from this audit                                                                                          |
| ---------------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| Passkeys (#13973)      | unlimited                            | Must be keyed by credential id, not account. Stored per account and repointed by merge like identities. Not unique. |
| Factor policy (#13974) | limited (one policy per factor type) | Must name its merge treatment before its schema lands; default primary-wins with per-attribute override.            |

Rule going forward (from the issue): no new object type may introduce
per-account uniqueness without a row in the merge-critical set with a
stated treatment.

## History-read rules

1. Stored rows keep their original `account_id`. Merge writes new
   events; it does not update `identity_events` or `audit`.
2. Reads that name an `account_id` follow the `merged_into` pointer
   chain to the primary for display.
3. An unmerge is not designed (open question, Drew, #13975 §8).

## Token `sub` survival (decision)

Tokens issued before merge keep `sub` = the secondary `account_id`.
Tokens issued after merge carry the primary's `account_id`. The
`merged_into` pointer keeps old `sub` values resolvable, so consumers
do not break. This is a decision recorded here for slice 2 to implement;
it is not a schema change.

## Gaps found

- The local-door credential has no explicit uniqueness rule; slice 2
  must pick one credential when both accounts have one.
- The seed list is config, not a table, so merge must write the primary
  id into config or a shim. Recorded for slice 2.
