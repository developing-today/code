# Tutorial: resolve a secret

This tutorial resolves a secret **by name** through `fleet secrets`:
first through the secrets broker, then by observing the Bitwarden
fallback, then listing what the broker holds — and finally reading the
three failure shapes (`502`, `404`, exit `3`) correctly and checking the
broker's health when resolution fails.

It assumes the `fleet` binary is installed (see
[Installation and bootstrap](../getting-started/installation.md)).

The standing rule for this whole surface: **secrets are resolved by name
through `fleet secrets` only.** No fleet domain shells out to `bws`
itself or reads a token file on its own, and no command in this tutorial
ever prints a secret value anywhere except the one place `get` is
specified to put it — its own stdout. The examples below use the
placeholder name `example.api-token`; substitute a real secret _name_
(never a value) from your own vault.

## Step 1: Resolve a name through the broker path

```bash
fleet secrets get example.api-token
```

What the output means: the secret's value, on **stdout and nowhere
else**, and exit `0`. Nothing else is printed — no source label, no age,
no log line. That silence is deliberate: the command is built to sit in
a pipeline, and a broken pipe on stdout (for example piping into `head`)
is a quiet success, not an error.

The resolution chain behind that one line (`Resolver::default_chain` in
the `fleet-secrets` crate):

1. **Broker leg first.** Where the secrets broker's own socket answers
   (the laptop), the client speaks `GET /secret/<name>` over that Unix
   socket directly. Elsewhere (a sandbox seat), it makes a framed
   exchange through the session broker to port `4800`. On that leg the
   request names the token _file_ (config `[secrets] token_file`,
   default `~/tools/secrets-broker-token`); the session broker reads the
   file per request and sends its contents as the auth header — the
   client only ever names the file, and the value never transits the
   client's own memory on this leg.
2. **`bws` CLI fallback** (Step 3), only when the broker leg cannot
   answer.
3. At library level there is also a 1Password source for seats that keep
   secrets there; the `fleet secrets` default chain's headline shape is
   broker, then `bws`.

What the broker does with the request: a cache hit answers immediately
with the cached value. A miss on a _loaded_ cache is not final — the
broker reads through to Bitwarden for that one name and serves what it
finds, so a secret minted moments ago is visible immediately instead of
at the next refresh tick. The broker refreshes its whole cache from
Bitwarden at startup and then on a TTL (default 900 seconds), serving
stale values while a refresh runs.

What can go wrong, and the fix:

- Exit `2`: a usage error — `get` takes exactly one name, and the name
  must not start with `-`.
- Exit `3`: unresolved — no source in the chain could produce the value.
  Work through Steps 4 and 5 before concluding the name is wrong.
- The value looks stale after a rotation: the broker serves cached
  values with their fetch time, and a rotation lands within one TTL of
  the edit. Wait out the TTL, or ask for a broker refresh (Step 5); do
  not "fix" it by reading Bitwarden directly in your own scripts — the
  by-name chain is the platform rule.

## Step 2: Know which names exist — `list`

```bash
fleet secrets list
```

What the output means: the names in the **broker's cache**, one per
line, names only, never values. Exit `0`.

What can go wrong, and the fix — one deliberate asymmetry to memorise:
**`list` has no `bws` fallback.** A broker failure here is loud (an
error on stderr, exit `3`), because a `bws` listing has different
semantics — it enumerates the vault, not the broker's cache — so it is
never silently substituted. If `list` fails but `get` works, you are
seeing exactly that asymmetry, not an inconsistency: `get` fell back,
`list` refused to. Fix the broker (Step 5) rather than working around
the listing.

## Step 3: Force and observe the `bws` fallback

There is no `--force-bws` flag. The fallback is a property of the chain:
when the broker leg reports itself _unavailable_, the leg is retried
first — the retry budget is sized to real restart windows (a full switch
restarts broker and tunnel together, and the broker is back in about
16 seconds), with pauses doubling per attempt up to a cap — and only
then does the chain move on to `bws`. One exception: if the
session-broker socket file does not exist at all, the leg does not burn
its retry budget — no broker on this box is not a broker mid-restart.

To force the fallback in a test, make the broker leg genuinely
unavailable on a seat where that is safe — for example on a sandbox seat,
before `fleet init` has started the local loopback secrets broker (see
[Tutorial: run the brokers](run-the-brokers.md) for what `init` starts),
or while the laptop broker is stopped for maintenance — then run Step 1
again:

```bash
fleet secrets get example.api-token
```

What the output means: the same contract as the broker path — the value
on stdout, exit `0`, nothing else. The fallback runs `bws secret list`
plus `bws secret get`, the same shape the broker itself uses to fill its
cache, with the `bws` credential resolved the way the broker resolves
its own (the `BWS_ACCESS_TOKEN` environment variable when non-empty,
else the on-disk machine-account token file, handed to the `bws` child
through its environment).

How you observe which path answered, since the output deliberately does
not say: by the broker's state, not by the value. Check the broker's
health (Step 5) at the same moment — if it was not serving, the value
came from `bws`; if it was serving, it came from the broker. Latency is
a secondary hint only: the per-call `bws` fetch the broker exists to
replace cost 10–40 seconds per invocation, against an immediate cache
hit. Never treat latency alone as proof.

What can go wrong, and the fix:

- The fallback also fails, exit `3`: `bws` is missing, its credential
  does not resolve, or the name is absent from the vault as well as
  unreachable through the broker. Distinguish "absent" from
  "unreachable" with Step 4.
- A broker **404 is definitive and is not retried, and does not fall
  back**: the broker has a loaded cache and read-through, and the name
  is not in Bitwarden. That is Step 4's `404`, not a fallback trigger.

## Step 4: Read the failure shapes — `502` vs `404` vs exit `3`

These three answers mean three different things. Confusing them sends
you fixing the wrong layer.

**`404` — the name does not exist.** From the broker:
`{"error": "no such secret"}`. The broker's cache was loaded, the
read-through to Bitwarden ran, and Bitwarden definitely does not hold
that name. Fix the _name_: check spelling against Step 2's list. Do not
retry, and do not restart anything — nothing is down.

**`502` — the source behind the broker failed.** From the broker:
`{"error": "bitwarden read-through failed"}`. The name was a cache miss,
the broker tried to read through, and Bitwarden itself failed (the `bws`
call behind the broker errored). This is loud precisely so it is never
mistaken for a `404`: the secret may well exist; the broker could not
check. Fix the broker's Bitwarden leg (Step 5), then retry.

**`503` — the broker is up but not serving yet.** Before its first
successful refresh, the broker answers `GET /secret/<name>` with
`{"error": "secrets not loaded yet"}`, and its health endpoint answers
503 too. A live process whose `bws` cannot run is _not_ serving, and
health says so. This is a wait-or-fix-the-refresh state, not a missing
secret.

**Exit `3` — the client could not resolve the name at all.** The
`fleet secrets` client-side shape: the broker leg was unavailable (after
its retries), and the fallback also failed or the name was unresolved
through it. Exit `3` is about the _path_, where `404`/`502`/`503` are
answers _from_ a broker that was reached. (For completeness: the
client's other exits are `0` for success and `2` for usage.)

## Step 5: Check the broker's health when it fails

The broker's health endpoint is unauthenticated and reports the serving
gate directly. On the laptop, where the broker binds its tailnet address
on port `4800`:

```bash
curl -s http://100.99.113.103:4800/health
```

Read the answer field by field: `{ok, serving, secrets,
last_refresh_unix, refreshing, oldest_secret_age_seconds}`.

- `serving: true` with a recent `last_refresh_unix`: the broker is
  healthy; `secrets` is the cached name count and
  `oldest_secret_age_seconds` tells you the stalest value it can serve.
  A resolution failure against a serving broker is a `404` problem
  (Step 4), not a health problem.
- HTTP `503` with `serving: false`: no refresh has succeeded yet.
  Check `refreshing` and `last_refresh_unix`. A failed authenticated
  secret read during startup can include `refresh_error` in its 503
  response. Use that diagnostic when available.
- No answer at all: the broker process is not running, or this seat
  cannot reach it. On a sandbox seat, run `fleet init` (it starts the
  local loopback broker copy the sandbox uses) or see
  [Tutorial: run the brokers](run-the-brokers.md) to run one in the
  foreground and watch it start. On the laptop, check the service that
  runs `fleet serve secrets-broker`.

To kick a refresh on a running broker without restarting it, POST to
`/refresh` on the same address. Unlike `/health`, that endpoint requires
the broker's `X-Agent-Token` header — supply the token through your
normal operator tooling for the daemon-side token file
(`~/.config/secrets-broker/token` on the laptop), without printing its
contents or placing them in a command line. It answers
`202 {"ok": true, "refresh": "started"}`. This acknowledges the request.
If a refresh is already running, the worker skips the overlapping sweep.
Then re-read `/health`: a successful refresh sets `last_refresh_unix`
and clears the recorded refresh error.

Note the sandbox shape: on a sandbox seat (`FLEET_LOCATION=sandbox`) the
local broker's bind is forced to `127.0.0.1:4800` at the bind itself,
whatever its environment says — the same health checks apply against
loopback there.

## You are done when

- `fleet secrets get example.api-token` (with a real name substituted)
  exits `0` and prints the value on stdout only.
- `fleet secrets list` exits `0` and lists names — proving the broker
  leg, since `list` never falls back.
- You can state, for a failure you induced or observed, which of `404`
  (name absent), `502` (Bitwarden read-through failed), `503` (broker
  not serving yet) or exit `3` (client could not resolve at all) it was,
  and which layer you fixed.
- The broker's `/health` reports `serving: true`.
