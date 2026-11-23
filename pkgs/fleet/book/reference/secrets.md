# fleet secrets

`fleet secrets` resolves secrets **by name**: the secrets broker first, Bitwarden (`bws`) as fallback. Secret values are never printed anywhere except `get`'s stdout, never logged, and never written to a file by this domain. For the broker itself — its cache, refresh, and serving gate — see [Secrets and the secrets broker](../concepts/secrets.md) and the `secrets-broker` role in [fleet serve](serve.md).

The platform rule is that secrets are resolved by name through the fleet-secrets chain only; no domain shells out to `bws` or reads a token file on its own.

## The resolution chain (shared context)

`get` walks this chain in order:

1. **Broker leg(s) first.** Where the secrets broker's own socket answers (the laptop), a direct source speaks `GET /secret/<name>` over that Unix socket directly. Elsewhere (the sandbox), a broker source makes a **framed exchange through the session broker** to port 4800: the request names the token _file_ (`[secrets] token_file`, default `~/tools/secrets-broker-token`); the session broker reads the file per request and sends its contents as `X-Agent-Token` — fleet only ever names the file; the value never transits fleet's own memory on this leg.
2. **`bws` CLI fallback** — `bws secret list` + `get`, the same shape the broker itself uses to fill its cache. The fallback carries its own credential: the `BWS_ACCESS_TOKEN` env var when non-empty, else the on-disk machine-account token file, handed to the `bws` child via its environment.
3. (Library-level, not in this domain's headline chain) a 1Password source for seats that keep secrets there — the same names resolve without a broker or a Bitwarden token.

Retry semantics: a broker leg that cannot answer is retried before the chain moves on — the budget is sized to real restart windows (a full switch restarts broker + tunnel together; the broker is back in ~16 s) without making a genuinely dead broker a hang: the first pause doubles per attempt up to a cap, with a per-attempt ceiling on a single exchange. Exactly one broker leg owns the retry budget; when a direct leg exists, the session-broker leg behind it gets no retries. A broker **404 is definitive** about the name and is not retried. If the session-broker socket file does not exist at all, the leg does not burn the retry budget. Resolved values live in a type whose `Debug` is redacted; chain errors count failed sources without ever carrying a value.

Known discrepancy: "the laptop secrets-broker" in the domain summary is **location-relative**. On the sandbox the first leg is a _local_ loopback broker spawned by `fleet init`; on the laptop it is a direct Unix-socket leg; only from the sandbox-through-proxy path is it the laptop's broker.

The three broker service tokens are published in Bitwarden Secrets Manager under the names `fleet.secrets-broker-token`, `fleet.queue-broker-token`, and `fleet.laptop-agent-token`, and live on a seat as mode-0600 files `~/tools/secrets-broker-token`, `~/tools/queue-broker-token`, and `~/tools/laptop-agent-token`. Do not conflate these client-side files with the daemon-side token files in each broker's own config dir (`~/.config/secrets-broker/token`, `~/.config/queue-broker/token`).

## fleet secrets get

```
fleet secrets get <name>
```

| Flag                  | Argument    | Default  | Meaning                                                                                                                                                          |
| --------------------- | ----------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<name>` (positional) | secret name | required | Exactly one name, not starting with `-`. This verb takes no flags. Help flags are answered before arity checks, so `--help` is never mistaken for a secret name. |

### Behaviour

Resolves one secret by name through the chain above. The value goes **to stdout and nowhere else**; errors are value-free shapes on stderr. A broken pipe on stdout is a quiet success (the Unix-filter rule), so `fleet secrets get NAME | head` does not fail.

Broker response shapes this verb consumes: a cache hit answers `{"key", "value", "fetched_at_unix", "age_seconds", "source": "cache"}`; a read-through hit answers the same shape with `"source": "bitwarden-read-through"` and `"age_seconds": 0`; a not-loaded broker answers 503 `{"error": "secrets not loaded yet"}`; an absent name answers 404 `{"error": "no such secret"}`; a Bitwarden read-through failure answers a loud 502, never a silent 404-as-if-absent.

### Exit codes

- `0` — ok, value on stdout (including the broken-pipe case).
- `2` — usage (missing/extra name, or a name starting with `-`).
- `3` — unresolved / broker unavailable and the fallback could not resolve the name either.

### Example

```console
$ fleet secrets get fleet.secrets-broker-token > /dev/null && echo resolved
resolved
```

(The value itself is deliberately not shown; in real use it is piped or captured, never displayed.)

## fleet secrets list

```
fleet secrets list
```

| Flag | Argument | Default | Meaning                          |
| ---- | -------- | ------- | -------------------------------- |
| —    | —        | —       | Takes no arguments and no flags. |

### Behaviour

Lists the names in the broker's cache, one per line — names only, never values. Source selection mirrors `get`'s first leg only: the direct broker source when the broker's own socket is configured/live, else the session-broker source over the session broker socket plus the configured secrets token file.

Known discrepancy: `list` has **no `bws` fallback**, contradicting the symmetry the domain summary ("broker first, Bitwarden as fallback") suggests. A broker failure is loud — stderr plus exit 3 — because a `bws` listing has different semantics: it enumerates the vault, not the broker's cache, so it is never silently substituted.

### Exit codes

- `0` — ok, names on stdout.
- `2` — usage (trailing argument).
- `3` — broker unavailable. There is no fallback for this verb.

### Example

```console
$ fleet secrets list
fleet.laptop-agent-token
fleet.queue-broker-token
fleet.secrets-broker-token
```

Names only; the listing is the broker's cache, not the vault.
