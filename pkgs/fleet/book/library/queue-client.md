# The queue client

`fleet-queue` is three surfaces in one crate — a client, a queue broker
and a secrets broker — and Cargo features are the walls between them.
This chapter is about embedding the **client** surface: what the
`client` feature turns on, how a verb call travels to the broker (the
`call()` path), the argument-parsing helpers every verb is built on, the
fallback module's feature gate, the two transport routes, and how token
files are handled on each side.

For the crate's place in the workspace, see [The crates](crates.md).
For the verbs themselves, the tutorials
[Tutorial: claim and complete a queue task](../tutorials/claim-and-complete.md)
and [Tutorial: run the brokers](../tutorials/run-the-brokers.md) cover
the operator's view.

## The feature gates

`fleet-queue`'s features, and the modules each one gates in `lib.rs`:

| Feature | Dependencies it adds | Modules it gates |
|---|---|---|
| `client` | `fleet-config`, `ureq` | `client`, `fallback`, `needs_drew`, `placement` |
| `queue-broker` | `client`, plus `tiny_http`, `ureq`, `fleet-config`, `libc`, `rusqlite` | `queue_broker`, `turso_queue`, `validate` |
| `secrets-broker` | `tiny_http`, `fleet-config`, `libc` | `secrets_broker` |

Either broker also gates `uds_http`; `disposition` is gated on
`client` **or** `queue-broker` (the completion gate is needed on both
sides). Unconditional modules include `backoff`, `bin_args`,
`creation_outbox`, `log`, `merge_intent`, `token_file`, `work` and
`person_wait`.

Two consequences an embedder must internalise:

- **`queue-broker` includes `client`.** A broker build carries the whole
  client surface with it; there is no broker-only build of this crate
  without the client.
- **The fallback module sits behind the `client` feature** —
  `#[cfg(feature = "client")] pub mod fallback;`. The GitHub-issues
  fallback transport is client surface, not broker surface. A test run
  under `--features queue-broker` alone would not even compile the
  fallback's tests; they run only under a client-carrying feature set
  (the Nix `fleet-brokers` build and the full-variant check phase both
  carry it).

The bin crate maps capabilities onto these features exactly:
`queue = […, "fleet-queue/client"]`, `queue-broker = […,
"fleet-queue/queue-broker"]`, and the standalone `fleet-brokers`
package builds `-p fleet-queue --features
client,queue-broker,secrets-broker --bins`, producing `repo-queue`,
`queue-broker` and `secrets-broker` as separate artifacts.

## The entry point

The client's single public entry is the same function the multicall
binary and the standalone `repo-queue` binary both call:

```rust
pub fn run(args: &[String]) -> i32   // fleet_queue::client::run
```

`args` is the argument vector *after* the domain name (`fleet queue
<args…>` / `repo-queue <args…>`), and the return is the process exit
code. Inside, `run` is one match over 32 verb names — the same 32 names
the registry lists as the queue domain's subcommands — dispatching to a
`cmd_<verb>` handler in the client modules (`commands_lifecycle.rs` for
lifecycle verbs, `commands_query.rs` for query verbs, `fallback_stack.rs`
for the outbox/stack groups), with `fallback` routed to
`crate::fallback::run` and `dispatch` / `session` / `surfaces` routed to
the placement module (see [Placement and surfaces as data](placement.md)).

Bare invocation and help follow the client's own contract: no args
prints the top usage and returns `2`; a single `--help` / `-h` / `help`
returns `0`; an unknown verb prints `repo-queue: unknown command …` plus
the top usage and returns `2`.

## The `call()` path

Every broker-backed verb handler is the same five moves:

1. Declare its flags and parse (next section).
2. Validate positionals and build a JSON request body.
3. POST the body to the verb's broker route via the transport's `call`
   helper — `call("/unblock", &body)` for the `unblock` verb, and so on.
   Every client verb is one POST route on the broker (`/validate`,
   `/enqueue`, `/list`, `/claim`, `/claim-flow`, `/heartbeat`,
   `/checkpoint`, `/complete`, `/unblock`, … plus the event, outbox and
   stack families).
4. Unwrap the broker envelope: success is `{ok: true, …}`; failure is
   `{ok: false, exit, error}` — the client prints `error` and exits with
   `exit` (defaulting to `4` when the envelope carries no `exit` of its
   own).
5. Print the returned row (most verbs) as JSON and return `0`.

Two transport behaviours sit inside step 3 and are part of the client's
contract, not accidents:

- **Retry on transient store errors.** A broker envelope whose error
  carries PostgREST code `57014` (statement timeout) or `PGRST002`
  (schema cache) is retried with exponential backoff and jitter — base
  500 ms, doubling, capped at 20 s, at most 6 attempts. This is safe for
  writes because a `57014` means the statement rolled back.
- **The fallback switch.** For the work verbs — `claim`, `heartbeat`,
  `checkpoint`, `complete`, `health` — *any* primary-store failure
  (broker unreachable, or an `{ok: false}` envelope) switches the caller
  to the GitHub fallback for that work instead of dying: a diagnostic on
  stderr, one JSON object on stdout naming the switch
  (`{"ok": false, "fallback": true, "switch_to": "github-fallback", …}`
  with a `mode_line` of mode `db-degraded`, route `github-markers`), and
  exit **6**. The environment variable `REPO_QUEUE_NO_FALLBACK_SWITCH=1`
  is the escape hatch that restores plain die-on-error.

The client's exit-code vocabulary follows from this: `0` success, `1`
gate/not-found failures on specific verbs, `2` usage, `3` transport or
broker-path failure (unreachable, HTTP 401, HTTP ≥ 400, malformed reply),
`4` a broker envelope failure with no exit of its own, `5` "the verb ran
but there is no matching claimed row / no claimable task", `6` the
fallback switch.

## Verb argument parsing helpers

All queue verbs parse through one hand-rolled parser in
`client/support.rs` — there is no `clap` anywhere in the workspace, by
design. The pure core is `parse_args_inner`; `parse_verb_args` is the
wrapper handlers call:

```rust
// Shape of a handler's first moves (from cmd_unblock):
let args = parse_verb_args("unblock", rest, &["worker", "reason", "drop-dep"], &[]);
reject_extra_pos("unblock", &args, 1);
```

The signature pattern is `parse_verb_args(verb, rest, value_flags,
bool_flags)`: the handler declares exactly the flags it accepts, value
flags in the first slice and booleans in the second. The result is an
`Args` value — `{ pos: Vec<String>, opts: Map<String, Value> }` — with
accessors `get`, `has`, `get_i64` and `pos_i64`.

The parser's rules are the client's usage contract, and they are strict:

- A help token anywhere in a verb's args prints that verb's exact usage
  and exits `0` — help wins over every other argument, *except* when the
  help-looking token is the value of a value-flag (`fail 5 --error help`
  records the error text "help").
- Unknown flags, duplicate value flags (never silent last-wins), a
  boolean given an inline value (`--dry-run=false`), a value flag with
  no value, a value flag followed by another `--flag` token, extra
  positionals past the verb's arity, and non-numeric values for numeric
  flags are **all** usage errors: message plus the verb's usage on
  stderr, exit `2` — and for the arity and flag-shape errors, all of
  this happens before any broker contact.
- Durations and limits are **rejected, never clamped**, identically at
  client and broker: claim lease `1..=86400` (default 2700), heartbeat
  extend `1..=86400` (default 900), park delay `1..=604800`, list limit
  `1..=500` (default 20).

Two hand-maintained usage strings per verb live beside the parser (the
one-line synopsis in `TOP_USAGE` and the long form in `usage(verb)`),
and a machine test — `every_verb_usage_matches_its_parser` — asserts
every verb's usage names *exactly* the flags its parser accepts. An
embedder adding a verb gets that test as a free oracle; see the worked
example in the crates' contributor documentation.

Shared resolution helpers sit on top of `Args`: `opt_worker` (flag, else
the default worker `repo-worker-code`), `opt_queue` (flag, else
`REPO_QUEUE_DEFAULT_QUEUE`, else the shadow default
`repo-health-code-shadow` — with `claim` and `list` deliberately
*unscoped* when neither flag nor env is present, and `enqueue` having no
default at all), and the lane resolution for holder verbs (flag, else
`REPO_QUEUE_LANE`, else null, in which case the broker checks the worker
alone). These helpers are crate-internal (`pub(super)` / `pub(crate)`):
an embedder outside the crate uses `client::run`, not the helpers.

## Transport routes

The client never touches the database and never fetches from Bitwarden.
It POSTs JSON to the queue broker, which is the only process that talks
to the Turso store. Route selection, in order:

### 1. The direct route (on-laptop)

Chosen when a broker URL is configured: the environment variable
`REPO_QUEUE_BROKER_URL`, else the file `~/.config/repo-queue/broker-url`.
A `unix://<path>` value selects the broker's Unix socket; anything else
is read as `http://host:port` TCP. With no configured URL, fleet-config's
probe-based choice for the `queue-broker` role applies: a live role
socket selects the Unix socket, otherwise the client falls through to
route 2 — and a failed Unix-socket attempt falls back to the
session-proxy route *within the same call*.

On the wire this is raw HTTP/1.1: `POST <path>`, header
`X-Agent-Token: <token>`, `Content-Type: application/json`,
`Connection: close`. Read timeout 300 s, write timeout 60 s. Chunked and
Content-Length responses are both decoded strictly; HTTP 401 and
HTTP ≥ 400 die with exit `3` (a ≥ 400 surfaces the body's `error`
field).

### 2. The session-proxy route (sandbox default)

The client connects to the laptop-agent session proxy's Unix socket —
fleet-config's `broker_socket()` when it loads, else
`$HOME/.cache/laptop-agent/broker.sock` (environment beats config file
beats that compiled-in default). Framing is the proxy protocol: an
8-byte big-endian length prefix plus one JSON envelope:

```json
{"method": "POST", "path": "/claim", "body": { }, "port": 4801, "token_file": "<home>/tools/queue-broker-token"}
```

The proxy, on the laptop, reads the named token file *there* and
forwards to the broker on port `4801`. Read timeout 660 s. On a failed
first attempt the client auto-spawns the proxy/broker program — a
sibling `fleet` binary (`fleet proxy serve`) always wins; the Python
`~/tools/laptop-agent-broker.py` is the fallback for standalone rollback
builds — waits up to about 5 seconds for the socket, and retries once.

Broker-unavailable on either route is exit `3`, or the fallback switch
(exit `6`) for the work verbs.

## Token file handling

One loader serves every side: `token_file::load_token_file` reads the
file and trims it, and a missing, unreadable, empty or whitespace-only
file is a **hard error naming the file** — a broker refuses to start and
a client refuses to send. (Before this rule, an empty token file
authenticated *any* empty-token client, including to the secrets
broker.)

Which file, on which route (names and paths only — values are never
read by the client code that names them, and never printed):

| Side | Source |
|---|---|
| Session-proxy envelope | Names the file `<home>/tools/queue-broker-token`; the proxy reads it on the laptop, per request |
| Direct route, client token | `REPO_QUEUE_TOKEN_FILE`, else `~/.config/queue-broker/token` |
| Broker's own token | `QUEUE_BROKER_TOKEN_FILE`, default `~/.config/queue-broker/token` — compared against the request's `X-Agent-Token` header |
| Broker's Turso credential | `QUEUE_BROKER_TURSO_TOKEN_FILE`, else resolved through the secrets broker by the secret name `turso.fleet-queue-rw-token` |

The broker side mirrors the client's strictness: its pre-body gate
refuses from headers alone — wrong or missing token gets
`401 {"error":"unauthorized"}`, a `Content-Length` over the 16 MiB cap
gets `413` — before any body byte is read. The only unauthenticated
route is `GET /healthz`.

## Example: embedding the client

This example is illustrative: it uses the verified entry point
(`fleet_queue::client::run`, taking the post-domain argument vector and
returning the exit code) to run one verb from an embedding binary.

```toml
# Cargo.toml — the client surface only; no broker code is compiled.
[dependencies]
fleet-queue = { path = "../fleet-queue", features = ["client"] }
```

```rust
// Run `fleet queue health` from inside another binary and propagate
// its exit code (0 ok, 3 broker path down, 6 fallback switch, ...).
fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = fleet_queue::client::run(&args);
    std::process::ExitCode::from(code as u8)
}
```

With the `queue-broker` feature instead, the same crate additionally
exposes `fleet_queue::queue_broker::run()` and
`fleet_queue::secrets_broker::run()` — both argument-free serve loops
(the standalone broker binaries accept no arguments at all: `-h` /
`--help` prints usage with exit `0`, anything else is a usage error at
exit `2`, classified by the crate's `bin_args` helper).

There is no public transport-level helper for embedders: the `call()`
family in `client/transport.rs` (`call`, `call_result`, `call_fb`) is
`pub(super)` — visible only inside the client module. `client::run` is
the only public entry point, which is why the example above goes
through it.
