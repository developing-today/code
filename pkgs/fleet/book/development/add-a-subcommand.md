# Add a subcommand

There is no clap anywhere in the fleet workspace — no `clap` dependency in any fleet `Cargo.toml`, no derive macros. Every layer is hand-rolled by design: the top-level dispatch in `crates/fleet/src/main.rs` must behave identically in every variant, including the tiny connect build where the registry and the stub machinery are the product, and multicall routing happens before any per-domain parsing exists. Domains adopt their own parsers as they land.

This chapter traces one real verb end to end — `fleet queue unblock` — and then gives the step-by-step for adding a new one. For adding a flag to an existing verb instead, see [Add a flag](add-a-flag.md).

## The worked trace: `queue unblock`

`unblock` is a good trace because it arrived later (#8276) and then grew a flag (#12777), so every seam shows.

### 1. The registry entry

In `crates/fleet-registry/src/lib.rs`, the `queue` entry of `pub const DOMAINS` lists `"unblock"` as one string in its `subcommands` array. That alone puts the verb in `fleet --help`, `fleet queue --help`, and `fleet capabilities` on *every* variant — including builds where the queue client is not compiled, which then answer the verb with the capability stub at exit 69.

### 2. The client dispatch arm

In `crates/fleet-queue/src/client.rs`, `pub fn run(args: &[String]) -> i32` is one match over the verb name. The arm is:

```rust
"unblock" => cmd_unblock(rest)
```

where `rest = &args[1..]`. Bare invocation and `--help` are handled above the match (bare prints `TOP_USAGE` and exits 2; a lone help argument exits 0). An unknown verb prints `repo-queue: unknown command {:?}` plus `TOP_USAGE` and returns 2. The registry's 32 queue subcommands match the 32 arms of this match.

### 3. The handler: `cmd_unblock`

The handler is `pub(super) fn cmd_unblock(rest: &[String]) -> i32` in `crates/fleet-queue/src/client/commands_lifecycle.rs` — the module for lifecycle verbs (claim, lane, bind, heartbeat, checkpoint, park, wake, block/unblock, the dependency and quarantine rechecks, cancel, revive, fail, complete, health, signal, and the small outbox verbs). Query verbs live in the sibling module `commands_query.rs`.

Its shape is the canonical shape for a broker-backed verb:

```rust
let args = parse_verb_args("unblock", rest, &["worker", "reason", "drop-dep"], &[]);
reject_extra_pos("unblock", &args, 1);
let mut body = json!({
    "task_id": args.pos_i64(0, "task_id"),
    "worker":  opt_worker(&args),
    "reason":  args.get("reason").unwrap_or_default(),
});
let drops = drop_dep_flags(&args);
if !drops.is_empty() { body["drop_deps"] = json!(drops); }
let resp = unwrap_ok(call("/unblock", &body));
print_json(&require_row(&resp, "unblock_task"));
0
```

Five moves: declare the flags, validate the positionals, build the JSON body, call the broker, print the returned row. The helpers live in `crates/fleet-queue/src/client/support.rs`:

- `parse_verb_args(verb, rest, flags, bools)` — `flags` are value flags, `bools` are boolean flags. Its pure core is `parse_args_inner`: duplicate value flags are errors (never silent last-wins), a boolean given `--flag=value` is an error, a value flag followed by another `--flag` token is an error, and an unknown flag is `unrecognized argument: --<name>`. A help token that is not a flag *value* prints the verb's usage and exits 0; any shape rejection exits 2 with the message plus usage.
- `Args` — the parsed result, with `get`, `has`, `get_i64`, and `pos_i64` accessors.
- `call("/unblock", &body)` — the transport in `src/client/transport.rs`: session-proxy Unix socket from the sandbox, direct HTTP on the laptop, probe-selected. Exit 3 means the broker path was unavailable.
- `unwrap_ok` / `require_row(resp, "unblock_task")` / `print_json` — envelope handling: a broker envelope is `{ok: true, …}` or `{ok: false, exit, error}`; the client prints `error` and exits with `exit`.

Help text is two hand-maintained strings in `support.rs`, both keyed by verb name: the `TOP_USAGE` synopsis line (`unblock <task-id> [--worker <id>] [--reason <text>] [--drop-dep <list>]`) and the `usage(verb)` match arm (synopsis plus the long explanation). The parity test `every_verb_usage_matches_its_parser` fails the suite if they drift from the parser — with one known gap for `TOP_USAGE`, documented in [Testing](testing.md).

### 4. The broker verb

The client is not the end of the verb. Queue-broker routing in `crates/fleet-queue/src/queue_broker.rs` maps `POST /unblock` to `pub(crate) fn verb_unblock(t: &TursoBackend, req: &Value) -> Value` in `crates/fleet-queue/src/turso_queue/admin.rs`. It refuses a missing task and any task whose status is not `blocked` (`unblock_task: task <id> is <status>, not blocked`); otherwise it flips the task to `queued`, writes the audit events (`unblocked`, and `dep_unblock_override` with `via: "unblock_task_turso"`, worker and reason recorded), and returns the row the client prints as `unblock_task`. The surgical dependency-dropping path is `fn unblock_drop(...)` in the same file.

### 5. The fallback twin

A queue verb that must work while the broker is down also lands in `crates/fleet-queue/src/fallback.rs` (compiled only under the `client` feature — see [Testing](testing.md)). The fallback is the GitHub-issues transport of the same queue. For `unblock` it has:

- its own dispatch arm, `"unblock" => cmd_unblock(rest)`;
- its own `fn cmd_unblock`, whose grammar is narrower — flags `&["worker", "reason"]` only; `--drop-dep` was deliberately not added to the fallback grammar;
- its own usage arm in `fallback_help`;
- the guard `fn unblock_refusal(open, labels, &ClaimState)`, which the comments require to *mirror* the broker's `verb_unblock` refusal rules;
- a marker discipline: the GitHub side records the override as an audit-only marker comment (`<!-- fleet-claim v1 kind=unblock worker=<id> ts=<rfc3339-utc> -->`, `MarkerKind::Unblock`) that does not fold claim state.

Fallback switching itself: a work verb that hits a primary-store failure switches to the GitHub fallback with exit 6 and one JSON object naming the switch.

## Step by step: add a new verb

The queue domain is written out in full; other domains follow the same shape minus the broker and fallback steps.

1. **Registry entry.** Add the verb name to the domain's `subcommands` array in `crates/fleet-registry/src/lib.rs`. Do this only for a *top-level* verb of the domain — nested leaves such as `outbox claim` live only in the handler and the usage strings, not in the registry.
2. **Usage strings, both of them.** In `crates/fleet-queue/src/client/support.rs`, add the one-line synopsis to `TOP_USAGE` and the long-text arm to `usage(verb)`. The parity test checks the second; nothing checks the first, so review it by eye (see the discrepancy note in [Testing](testing.md)).
3. **Parser and handler.** Add the match arm in the domain's `run` (queue: `crates/fleet-queue/src/client.rs`) and write `cmd_<verb>` in the domain's commands module — lifecycle verbs in `client/commands_lifecycle.rs`, query verbs in `client/commands_query.rs`. Build it on `parse_verb_args` and, for a broker verb, on `call("/<verb>", body)` followed by `unwrap_ok` / `require_row` / `print_json`.
4. **Broker verb, if it mutates state.** Implement `verb_<verb>` in the appropriate `crates/fleet-queue/src/turso_queue/*.rs` module and route `POST /<verb>` to it from `src/queue_broker.rs`. The broker re-validates everything the client validated; the client-side checks are for fast, legible failure at the caller's terminal, not for safety.
5. **Fallback twin, if it must work broker-down.** Mirror the verb in `src/fallback.rs`: dispatch arm, handler, `fallback_help` arm, and a guard that mirrors the broker's refusal rules. Decide deliberately which flags the fallback grammar carries — parity with the primary grammar is not automatic, as `--drop-dep` shows.
6. **Tests.** Add oracle-named tests next to the code (see [Testing](testing.md)): parser invariants in the client tests, guard-mirror tests in `fallback.rs` where a twin exists, broker behaviour where the verb mutates state. Every test names its oracle and its issue.
7. **Docs in the same change (DOC-01).** Update `pkgs/fleet/docs/verbs.md` and the matching chapter of this book in the same change — the same pull request, not a follow-up. The full contract is in [Keeping this book current](keeping-this-book-current.md).
8. **Ship it.** A merged verb is not shipped until both live installs report it. Follow [Release and rollout](release-and-rollout.md) and verify by verb surface (`fleet queue --help`, the new verb's own `--help`), not by version string alone.
