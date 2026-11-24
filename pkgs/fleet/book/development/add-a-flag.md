# Add a flag

Adding a flag to an existing verb is a small change with more touch points than it looks like. This chapter uses the one fully traced example — `--drop-dep` on `fleet queue unblock` (#12777) — which took exactly five touches. Treat that list as the checklist for any flag; the parity test at the end is the gate that catches most misses.

For adding a whole verb, see [Add a subcommand](add-a-subcommand.md). For the parser and usage machinery referenced throughout, see [Testing](testing.md) for the parity test and its known gap.

## The case study: `--drop-dep` on `unblock`

`unblock` clears a task's block. `--drop-dep <list>` additionally drops named dependencies in the same operation, surgically, instead of leaving the task blocked on a dependency that will never land. The list grammar is comma-separated `kind:ref` entries: `issue:`, `pr:`, or `ci:` followed by `owner/repo#N`, or `task:ID`.

### Touch 1 — the parser declaration slice

In the handler `cmd_unblock` (`crates/fleet-queue/src/client/commands_lifecycle.rs`), the flag is declared by adding one string to the flags slice of the existing `parse_verb_args` call:

```rust
parse_verb_args("unblock", rest, &["worker", "reason", "drop-dep"], &[])
```

The first slice is value flags; the second, empty here, is boolean flags — a boolean flag goes there instead. Nothing else about parsing changes: duplicates, missing values, and the `--drop-dep=<list>` inline form all inherit the rules of `parse_args_inner` in `crates/fleet-queue/src/client/support.rs` (duplicate value flags are errors, a value flag followed by another `--flag` token is an error, unknown flags are `unrecognized argument: --<name>`).

### Touch 2 — the value parser

A flag with a non-trivial value grammar gets a dedicated parser next to the handler:

```rust
pub(super) fn parse_drop_dep_list(list: &str) -> Result<Vec<Value>, String>
```

in `commands_lifecycle.rs`. It parses the comma-separated `kind:ref` grammar and returns an error string on a bad reference. The accessor `drop_dep_flags(&args)` calls it and dies at the caller's terminal with exit 2 (`unblock: --drop-dep expects …`) on error. The handler then wires the result into the broker body only when it is non-empty:

```rust
let drops = drop_dep_flags(&args);
if !drops.is_empty() { body["drop_deps"] = json!(drops); }
```

That conditional is deliberate: when the flag is absent, the body keeps exactly the old shape, so an older broker sees a request it already understands.

### Touch 3 — both usage strings

Both hand-maintained usage surfaces in `crates/fleet-queue/src/client/support.rs` gain the flag:

- the `TOP_USAGE` synopsis line becomes `unblock <task-id> [--worker <id>] [--reason <text>] [--drop-dep <list>]`;
- the `usage("unblock")` arm gains the synopsis plus the long explanation of `--drop-dep` semantics.

Updating only one is the classic miss. The parity test (touch 5) enforces the second; the first is on you — see the discrepancy note in [Testing](testing.md).

### Touch 4 — broker re-validation

The broker does not trust the client's parse. `verb_unblock` in `crates/fleet-queue/src/turso_queue/admin.rs` reads `drop_deps` from the request body and routes to `fn unblock_drop(...)`, which re-validates every spec. The client's own comment states the contract: the broker re-validates every spec; a bad reference dies there first, at the caller's terminal. Client-side validation exists for fast, legible errors; broker-side validation is the authority, because the broker is the component that mutates state and other clients exist.

### Touch 5 — docs and tests

- `pkgs/fleet/docs/verbs.md` gains the flag in the verb's prose where it is user-facing, and this book's matching chapter is updated in the same change (DOC-01 — see [Keeping this book current](keeping-this-book-current.md)).
- Oracle-named tests cover the new behaviour. For `--drop-dep`, the fallback-side oracles from #12777 assert that the unblock marker stays audit-only and does not fold claim state.

One deliberate non-touch: the flag was **not** added to the fallback grammar. Fallback `unblock` in `crates/fleet-queue/src/fallback.rs` still takes only `--worker` and `--reason`. A flag does not automatically exist in both transports; decide for each one, and record the decision in the change.

## Step by step: add a flag

1. Add the flag name to the correct slice of the handler's `parse_verb_args` call — value flags in the first slice, booleans in the second (`bools`).
2. If the value has a grammar, write a dedicated value parser next to the handler, returning `Result<_, String>`, and an accessor that dies with exit 2 and a message naming the expected shape. Wire the parsed value into the request body, conditionally if omitting it preserves the old body shape for older brokers.
3. Update **both** usage strings in `client/support.rs`: the `TOP_USAGE` synopsis and the `usage(verb)` long text.
4. Re-validate on the broker side if the flag reaches a broker verb. Never let a client parse be the only validation of a value that mutates state.
5. Decide the fallback grammar explicitly: add the flag to the `fallback.rs` twin, or record that it is primary-only.
6. Update `docs/verbs.md` and this book in the same change, and add oracle-named tests.

## Defaults discipline

- A new flag is optional unless the verb is meaningless without it. Adding a _required_ flag to a shipped verb is a breaking change to every script that calls it; that needs its own justification, not a ride-along.
- An omitted flag must reproduce the old behaviour exactly — including the old request-body shape where a broker is involved, as the `drop_deps` conditional above does.
- Defaults that already exist are part of the contract: for example `--worker` defaults to `repo-worker-code` (with named exceptions such as `cancel`, whose worker defaults to `fleet-monitor`), and `--queue` defaults are verb-specific (see the queue chapters). Do not quietly change a default while adding an unrelated flag.
- Numeric flags die with exit 2 naming the flag when given a non-numeric value. Follow that shape for any new validated value.

## Hidden flags: the `--reason-class` precedent

Not every flag belongs in the usage strings. `fail` carries `--reason-class`, a hidden flag: it is accepted by the parser, it is a closed enum, and it defaults to `transient` — but it is not advertised in the verb's help. It exists so automated callers can classify a failure for the broker's retry and reporting logic without turning an internal taxonomy into a user-facing promise.

Use the hidden-flag shape only when all three hold:

1. The value set is a closed enum with a safe default, so omitting the flag is always correct for a human caller.
2. The intended callers are programs, not people reading `--help`.
3. The flag is still declared in the parser slice like any other — hidden means unadvertised, not unparsed or unvalidated.

## The parity test is the gate

`every_verb_usage_matches_its_parser` (in the `client.rs` test module) asserts that every verb's `usage()` text names exactly the flags its parser accepts. Add a flag to the parser without the usage text — or to the usage text without the parser — and the workspace test suite fails. Run it before you push:

```sh
cargo test --workspace
```

Remember its one blind spot: it does not cross-check `TOP_USAGE`. Check that line yourself.
