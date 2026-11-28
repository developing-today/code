# Secrets and the secrets broker

Fleet resolves secrets _by name, through one crate_ (`fleet-secrets`). No
domain shells out to `bws` (the Bitwarden Secrets Manager command-line tool)
or reads a token file on its own. A caller names a secret; the chain finds
the value; the value goes only where the caller's contract says it goes.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs and https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-secrets/src/lib.rs

## The broker: a resident cache of the vault

The _secrets broker_ is a resident cache of the Bitwarden Secrets Manager
vault. It exists because the per-call alternative — `bws secret list` plus
`bws secret get` on every invocation — made every queue call pay 10–40
seconds. The broker binds the laptop tailnet address `100.99.113.103:4800`
plus a Unix-socket dual-bind at `~/.config/secrets-broker/broker.sock`, with
identical routes and auth on both transports; a socket bind failure degrades
to TCP-only with a log line. Sandbox consumers reach it through the session
proxy — except on the sandbox itself, where `fleet init` runs a local
loopback copy: when `FLEET_LOCATION=sandbox`, the bind is **forced to
`127.0.0.1:4800`** at the bind itself, whatever the environment says, so a
stale or widened bind can never expose the vault off-host. (The domain
summary's phrase "the laptop secrets-broker" is therefore
location-relative.)

Endpoints (the `X-Agent-Token` header is required on everything except
`/health`; an empty expected token never authenticates, not even against an
empty header):

| Route                | Answer                                                                                                               |
| -------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `GET /health`        | `{ok, serving, secrets, last_refresh_unix, refreshing, oldest_secret_age_seconds}`; **503 while `serving` is false** |
| `GET /secrets`       | `{names: [...]}` — names only, sorted                                                                                |
| `GET /secret/<name>` | `{key, value, fetched_at_unix, age_seconds, source}`, or 404 / 502 / 503 (below)                                     |
| `POST /refresh`      | 202 `{ok: true, refresh: "started"}` — kicks a refresh on a new thread                                               |

Auth failures are 401. Startup exit codes: 2 for a bad home or a refused
TTL, 3 for a token-file or bind failure. The broker's own token file is
`~/.config/secrets-broker/token` (`SECRETS_BROKER_TOKEN_FILE`).

## The serving gate: 503 until loaded

`serving` is true only once at least one refresh has _succeeded_. Before
that, public `/health` answers **503** with `ok:false, serving:false`.
An authenticated `GET /secret/<name>` answers 503 with
`error: "secrets not loaded yet"` and includes `refresh_error` when a
refresh has failed. A live process whose
`bws` cannot run is _not_ serving, and health says so: not-loaded is a
distinct, diagnosable state, not a 404. Clients key on this gate.

## Cache semantics: age and source

A cache hit answers 200 immediately with `source: "cache"`, the value's
`fetched_at_unix`, and its computed `age_seconds`. Every cached value
carries the time the broker last fetched it from Bitwarden, precisely so a
rotation runbook can tell "the broker is serving a value N seconds old"
from "Bitwarden itself holds this value".

**Read-through on miss.** A miss on a _loaded_ cache is not final: the
broker reads through to Bitwarden for that one name and serves what it
finds with `source: "bitwarden-read-through"`, `age_seconds: 0` — a secret
minted moments ago is visible immediately instead of at the next TTL tick.
Read-through is single-flighted per name: concurrent misses for the same
name wait on one gate instead of stampeding `bws`, and the cache is
re-checked after acquiring the gate in case a refresh landed the value
meanwhile. Outcomes: found → 200; definitely absent → 404
`{error: "no such secret"}`; source failure → a loud **502**
`{error: "bitwarden read-through failed"}` — never a silent
404-as-if-absent.

## Refresh: startup, TTL, and the one-call delta install

A background thread runs a full refresh immediately at startup, then sleeps
the TTL and repeats — stale-while-revalidate: the cache keeps serving
during a refresh, and a failed refresh keeps the old values. One upstream
refresh is a single `bws secret list` call carrying the whole vault as
id/key/value triples (S1, #13948) — there is no per-secret `bws secret get`
pool in the refresh path; the only `bws secret get` the broker runs is the
single-name read-through on a cache miss. The list is parsed strictly:
every entry must carry a valid id, key and value, and a malformed entry
fails the whole refresh loudly rather than silently dropping a secret.
The `bws` child has a 60-second deadline, so a hung child fails its fetch
instead of wedging the refresh — and the `refreshing` flag — forever. The
authenticated `POST /refresh` acknowledges the request with 202. If a sweep
is already running, the worker skips the overlapping sweep. Use
`last_refresh_unix` and `refreshing` on `/health` to follow progress.

Refresh cost is the list call, not a spawn per secret: PR #14021 measured a
cold refresh of 244 secrets from broker start to serving in 2.52 s (the
first-ever list call itself took 9.15 s), against 242 `bws` spawns and
160–220 s for the per-secret path it replaced.

**Install is atomic, then delta.** The complete list is validated before
any cache mutation, and the new set is swapped in under one write lock
together with the watermark advance: a failed or partial list preserves
the previous cache _and_ watermark untouched, so one broken list cannot
evict or half-rotate the vault (#5504).

The watermark is the start of the last successful refresh minus a
60-second skew overlap (`WATERMARK_SKEW_S`), persisted atomically
(write-temp-then-rename) to `~/.config/secrets-broker/watermark`
(`SECRETS_BROKER_WATERMARK_FILE`) and reloaded at startup;
`last_refresh_unix` _is_ the watermark — there is no second, un-skewed
timestamp. On a refresh that has a watermark (S2, #13948 §5.1), only
entries whose `revisionDate` is after the watermark are installed; an
entry with a missing or unparseable stamp is applied conservatively, and
every other entry is untouched — its cached value _and_ fetch time
survive, so its age keeps telling the truth. Ids absent from the
authoritative list are deleted; a cached entry whose id is unknown (a
read-through install) is retained, because an unknown id can never prove
absence. With no watermark yet, the refresh is a full apply: every entry
is installed, all stamped with the one refresh time.

Net effect: a rotation lands within one TTL of the edit, bounded and
observable via the exposed age, and a failed refresh leaves the last good
set serving.

**TTL.** `SECRETS_BROKER_TTL_S`: default **900 s (15 minutes)**, minimum
**30 s**. Unset → 900, no warning. Unparseable → 900 with a warning naming
the offending value. Parsed but below 30 (including 0) → startup refuses to
run, exit 2, with an error naming the variable, the value, and the minimum
— because one interval costs a full vault refresh, and 0 would spin that
loop with no sleep at all (measured at roughly 110 full refreshes per
second).

## The client chain: broker-first resolution

`fleet secrets get <name>` takes exactly one name. The value goes to stdout
and nowhere else; errors are value-free shapes on stderr; a broken pipe on
stdout is a quiet success (the Unix-filter rule). `fleet secrets list`
prints the names in the broker's cache, one per line, never values. Exit
codes for both: 0 ok, 2 usage, 3 unresolved / broker unavailable.

The resolution chain (`Resolver::default_chain`):

1. **Broker leg(s) first.** Where the secrets broker's own socket answers
   (the laptop), a direct source speaks `GET /secret/<name>` over that Unix
   socket. Elsewhere (the sandbox), the broker leg is a framed exchange
   through the session broker to port 4800: the request names the token
   _file_ (`[secrets] token_file`, default `~/tools/secrets-broker-token`);
   the session broker reads the file per request and sends its contents as
   `X-Agent-Token` — fleet only ever names the file, and the value never
   transits fleet's own memory on this leg. Exactly one broker leg owns the
   retry budget; when a direct leg exists, the session-broker leg behind it
   gets no retries.
2. **`bws` fallback.** `bws secret list` + `get` for the one name (the
   broker's own refresh, by contrast, fills its cache from the list call
   alone, because the list carries the values). The fallback carries its
   own credential: the `BWS_ACCESS_TOKEN` environment variable when
   non-empty, else the on-disk machine-account token file, handed to the
   `bws` child via its environment. Every wait on a fallback child is
   bounded: the runners spawn through `run_command_bounded`, which drains
   the child's pipes on threads, polls it, and kills it at the deadline —
   `SUBPROCESS_ATTEMPT_TIMEOUT`, 10 s per invocation for both the `bws`
   and `op` tiers, plus a 5 s probe bound (`PROGRAM_PROBE_TIMEOUT`) on the
   `bws --version` check that gates program resolution (PR #14362). A
   wedged child therefore fails its tier instead of hanging the client.
3. At library level, a 1Password source exists for seats that keep secrets
   there; the same names resolve without a broker or a Bitwarden token. It
   is not part of the `fleet secrets` command's headline chain.

Retry semantics: a broker leg that cannot answer is retried before the
chain moves on — the budget is sized to real restart windows (a full switch
restarts broker and tunnel together; the broker is back in about 16 s)
without making a genuinely dead broker a hang: the first pause doubles per
attempt up to a cap, with a per-attempt ceiling on a single exchange. A
broker **404 is definitive** about the name and is not retried. One broker
answer is fast-failed instead of retried: the cold-broker 503, `secrets not
loaded yet`, while the broker's first refresh is still in flight —
`resolve_with_retry` returns it at once and the chain falls through to its
next source, rather than wedging the client behind a
healthy-but-not-yet-serving broker for the whole restart budget (S1's
client bound; the broker answers that 503 immediately — 0.0005 s in PR
#14021's live measurement). If the
session-broker socket file does not exist at all, the leg does not burn the
retry budget — no broker on this box is not a broker mid-restart.

**`list` has no `bws` fallback.** A broker failure is loud (stderr, exit 3),
because a `bws` listing enumerates the vault, not the broker's cache, and is
never silently substituted. For `list`, the source selection mirrors `get`'s
first leg only.

## The three token files and bootstrap provisioning

Names and paths only — never values.

In Bitwarden Secrets Manager, the three broker service tokens are published
under the names `fleet.secrets-broker-token`, `fleet.queue-broker-token`,
and `fleet.laptop-agent-token`. On a seat they live as mode-0600 files:
`~/tools/secrets-broker-token`, `~/tools/queue-broker-token`, and
`~/tools/laptop-agent-token`. Two of the paths are compiled defaults in
`fleet-config`: the secrets token file is the one the session broker reads
per request for secrets-broker calls (overridable via `[secrets]
token_file`), and the laptop-agent token file is the one the session broker
reads when a request names none. Do not conflate these client-side
`~/tools/*-token` files with the daemon-side files in each broker's own
config directory (`~/.config/secrets-broker/token`,
`~/.config/queue-broker/token`).

A newly enrolled seat does not arrive with these files. The bootstrap token
phase (in `scripts/muse/bootstrap.sh`'s restore phase — the seat-bootstrap
layer, **not** the fleet crates; no code under `pkgs/fleet/crates/` reads
those three Bitwarden names to create the files) fetches the three names
into the `~/tools/*-token` paths with 0600 permissions, so a seat can speak
to all three brokers from its first command. The known failure shape this
prevents: a seat kind whose bootstrap skips the token phase can enrol — its
bridge leg answers, because that leg is HTTPS plus a by-name secret — yet
fail its queue, secrets, and shell legs with synthesised transport errors,
because the three shared files were never provisioned for it. Note that the
older Bitwarden name `secrets-broker.token` is a _different_ value and is
never used as the client token.

What the binary itself provisions at runtime is narrower: `fleet init`
_spawns_ the sandbox secrets broker (which then reads its own daemon token
file), and the brokers read their token files per startup or per request.
Token files stay files — read per request, never stored in memory
long-term, never logged.
