# Running a node: serve, worlds and the hosted edition

`id serve` is the only command that hosts anything. Every other command in [serve and publications](../reference/cli-serve.md) and in the world family is a client of a running serve, or works on a local store. This chapter is the operator version: what to pass, what it binds, what it writes to disk, and what stops working when the process stops. The full flag list is in [serve and publications](../reference/cli-serve.md); the data layout behind it is in [Backup, restore and data directories](backup-and-restore.md).

## The basic node

```text
id serve
```

With no flags, serve:

- opens the blob and name store in the data directory (the current directory by default, or the directory given to the global `--data-dir <DIR>` flag, or the directory created by the global `--new [<NAME>]` flag);
- takes the serve lock `.iroh-serve.lock` in that data directory (see "The serve lock" below);
- starts an Iroh QUIC endpoint on port `0`. Port `0` is the literal default of `--iroh-port <PORT>`: the operating system assigns the port. There is no fixed Iroh port to open in a firewall;
- announces and discovers peers with gossip, mDNS and the relay, all on by default;
- serves no web UI and hosts no world. Both are opt-in flags, below.

Serve runs until it is interrupted. There is no daemon mode, no service unit in the repository, and no stop verb: the operator runs the process in the foreground, in a terminal multiplexer, or under a supervisor of their own choosing. **Unverified:** this book found no systemd unit, launchd plist or other supervisor definition for `id serve` in the source tree.

Two local nodes that the user runs are recorded in the standing record, not in the source: a demo node on laptop loopback port `3210` and a production node on laptop port `3005`. **Unverified against source:** neither `3210` nor `3005` appears in `pkgs/id/` or in the home configuration in the checkout; both are web ports chosen by the operator at launch, presumably with `--port <PORT>`, and a fresh `id serve` does not use either number.

## Data directory selection

The global flags decide where a node lives, and they apply to every command, not only to `serve`:

| Flag | Effect |
|---|---|
| `-d, --data-dir <DIR>` | Use `<DIR>` as the base directory for `.iroh-store`, `.iroh-key` and `.iroh-serve.lock`. The directory is created if missing. The process changes directory into it at startup. Conflicts with `--new`. |
| `--new [<NAME>]` | Create `.iroh/<NAME>/` under the current directory and use it as the data directory. With no name, a random 8-hex name is used. Startup fails if the directory already exists. The name must not contain `/`, `\` or `..`. |

Running two nodes on one machine means two data directories. Two serves in one data directory do not work; the second one meets the serve lock and the single-opener store lock (see [Troubleshooting](troubleshooting.md)).

## The web UI

| Flag | Default | Effect |
|---|---|---|
| `--web` | off | Start the HTTP web UI. Startup fails on a build without the `web` feature; see [Troubleshooting](troubleshooting.md). |
| `--port <PORT>` | `3000` | Web port. `0` lets the operating system assign the port. |
| `--bind <IP>` | `127.0.0.1` | Web bind address. The default binds loopback only. |
| `--web-listen <IP:PORT>` | none | Repeatable. Replaces the `web_bindings` list from `defaults.json`. Startup fails if `--web-listen` is combined with an explicitly set `--bind` or `--port`. |
| `--web-token <TOKEN>` | unset | Gate token for the web UI, accepted as the `id_token` cookie, as `Authorization: Bearer`, or as a `?token=` query parameter that sets the cookie and redirects. The environment variable `ID_WEB_TOKEN` sets the same value. The value is never printed in this book. |
| `--web-cookie-secure` | off | Mark web cookies `Secure`. The environment variable `ID_WEB_COOKIE_SECURE` sets the same flag. |

Startup prints a warning when a web listener is bound without `--web-token`. On a loopback bind the web layer also enforces a Host allow-list (`localhost`, `127.0.0.1`, `::1`, plus operator extras) and a same-origin check on requests that carry an `Origin` header. Binding to a non-loopback address without a token removes the loopback protection and leaves the token as the only gate; the book states the mechanism and makes no recommendation beyond that fact. The pages and routes themselves are in [The web UI: pages and HTTP API](../reference/web-ui.md).

## Hosting a world

| Flag | Default | Effect |
|---|---|---|
| `--world` | off | Host a multiplayer world. Requires `--world-admin-token <TOKEN>`. |
| `--world-admin-token <TOKEN>` | unset | The admin token for the hosted world. The environment variable `ID_WORLD_ADMIN_TOKEN` sets the same value. The value is never printed in this book. |
| `--world-name <NAME>` | `lobby` | Name of the default world. Its files live in `.id-worlds/<NAME>/` inside the data directory. The name uses the characters `a-z`, `0-9`, `-` and `_`. |
| `--world-module <FILE>` | none | A precompiled Wasm module (Roc ABI) to load into the world. Requires `--world` and a build with the `sandbox` feature. |
| `--world-checkpoint-every <N>` | `1000` | Journal events between checkpoints. `0` never trims the journal. |
| `--world-max-open <N>` | `256` | Maximum open worlds. |
| `--world-max-sessions <N>` | `1024` | Maximum world sessions. |
| `--world-idle-secs <N>` | `600` | Seconds before an idle world is closed. `0` keeps worlds open. |
| `--world-boot-window-secs <N>` | `600` | Replay-refusal window after startup, in seconds. `0` disables it. |
| `--world-cap-policy <POLICY>` | `deny` | Capability policy for the default world's program. The other documented value is `grant-on-use`. |
| `--world-allowed-caps <NAMES>` | none | Comma-separated server-wide capability ceiling. The value `*` is refused here. |
| `--world-cap <NAME>` | none | Repeatable. One capability for the default world's program. `*` means the whole catalog. |
| `--world-ssh-port <PORT>` | none | Serve the world over SSH, bound to the `--bind` address. Requires `--world`. Actual SSH serving exists only in a build with the `ssh` feature. |
| `--world-native` | off | Accept native (ELF) module installs, in the OS-sandbox tier. |
| `--world-source-only` | off | Accept Roc-source installs only. Conflicts with `--world-module`. The environment variable `ID_WORLD_SOURCE_ONLY` sets the same flag. |

The runtime limits for guest execution are flags in the same family: `--world-fuel <UNITS>` (default `10000000`), `--world-memory-mib <MIB>` (default `16`), `--world-message-kib <KIB>` (default `1024`), `--world-total-memory-mib <MIB>` (default `64`), and the native-tier limits `--world-native-memory-mib <MIB>` (default `256`), `--world-native-cpu-secs <SECS>` (default `10`) and `--world-native-deadline-ms <MS>` (default `10000`). Confirmation mail for world sign-up has three mutually conflicting sinks, all requiring `--world`: `--world-mail-outbox <DIR>`, `--world-mail-command <PROG>` and `--world-mail-smtp <IP:PORT>` (SMTP also requires `--world-mail-from <ADDR>` and must be loopback).

A toy world host, with invented values:

```text
id --data-dir toy-node serve --web --world --world-admin-token toy-admin-token --world-name lobby
```

Clients then join with the commands in [World commands: joining and playing](../reference/cli-worlds.md). Administration is in [World commands: administering and verifying](../reference/cli-worlds-admin.md).

## Discovery: relay, gossip, mDNS and DHT

| Flag | Default | Effect |
|---|---|---|
| `--no-relay` | relay on | Direct connections only; no relay is used. |
| `--no-gossip` | gossip on | No gossip discovery or announcement. |
| `--gossip` | — | Force gossip on. Conflicts with `--no-gossip`. |
| `--no-mdns` | mDNS on | Disable mDNS local discovery. |
| `--dht` | off | Enable mainline DHT address lookup and publishing. The flag parses in every build; the `dht` cargo feature controls the underlying behaviour in `id-sync`. |
| `--bootstrap <IDS>` | from `defaults.json` | Comma-separated bootstrap node IDs. Repeatable through the comma delimiter. |
| `--no-default-bootstrap` | — | Ignore bootstrap IDs from `defaults.json`. |
| `--topic <TOPIC>` | see below | Gossip topic. |
| `--topic-secret <SECRET>` | see below | Gossip topic secret. The value is never printed in this book. |
| `--no-default-topic` | — | Ignore topic and secret from `defaults.json`; fall back to the hardcoded constants. |

The topic defaults need care, because `--help` states them in a way that misleads. The help text gives `--topic` a default of `id-peer-discovery-v1` and `--topic-secret` a default of `id-public-discovery-v1`. Those two strings are the hardcoded fallback constants. The operative default on a stock build comes first from the embedded `defaults.json`, whose shipped content sets `topic` to `id-peer-discovery-v1` and `topic_secret` to `id-public-discovery-v1` — the same strings today, by coincidence of the shipped file, not by the help text's logic. If `defaults.json` changes, serve follows the file, not the constants, unless `--no-default-topic` is passed. The `--version` output reports which defaults source is in force, as a second line naming the file hash and mode (`append` or the built-in equivalent). This is disagreement CLI-6 in the survey record. The full configuration precedence is in [Configuration and environment](../reference/configuration.md).

## Who may write

| Flag | Default | Effect |
|---|---|---|
| `--allow-node <IDS>` | none | Repeatable, comma-separated. Node IDs allowed to write the store. The file `.iroh-allowed` in the data directory (one node ID per line) adds more, and this machine's own keys are always allowed. |
| `--open-writes` | off | Any peer may write the store. The help text documents this as insecure. |

With neither flag, writes from other nodes are not accepted beyond this machine's own keys and the `.iroh-allowed` list.

## The gallery

| Flag | Default | Effect |
|---|---|---|
| `--gallery-dir <DIR>` | unset | Serve a program gallery from `<DIR>`: the Iroh registry plus the `/api/gallery/*` routes when `--web` is on. Every bundle file in the directory is verified at startup. The environment variable `ID_GALLERY_DIR` sets the same value. |
| `--gallery-accept-publish` | off | Accept gallery publishes. Requires `--gallery-dir`. The environment variable `ID_GALLERY_ACCEPT_PUBLISH` sets the same flag. |

Without a gallery directory, the gallery routes answer 404. Operating the gallery (approval, removal, reinstatement) is in [Gallery, match-audit and walk](../reference/cli-gallery-walk.md).

## Server config and ephemeral mode

| Flag | Default | Effect |
|---|---|---|
| `--server-config <FILE>` | none | Load an operator server-config TOML file: provisioned users and groups, create policy, pull policy, budgets, residency and boundaries. A malformed file fails startup and names the parse error. Loading requires a build with the `world` feature. The key list is in [Configuration and environment](../reference/configuration.md). |
| `--ephemeral` | off | Use an in-memory store. All content is lost when the process exits. No `.iroh-store` on disk is the record for an ephemeral node. |

An ephemeral node is the right shape for a throwaway test peer and the wrong shape for anything whose files must survive the terminal closing. Nothing about `--ephemeral` is written to the data directory's store, because there is no store.

## Operator server-config and compile-cache environment

Serve reads three environment variables outside the flag parser:

| Variable | Effect when set |
|---|---|
| `ID_WORLD_COMPILE_CACHE_DIR` | Directory for the shared compiled-world cache. Unset or empty disables that cache. |
| `ID_WORLD_COMPILE_CACHE_LRU_BYTES` | Size limit for that cache. An unparsable value falls back to the default of 2 GiB. |
| `ID_APPS_DIR` | Apps root for host app discovery. Unset falls back to the built-in apps root. |

For `--world-mail-command`, serve sets `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` in the child process environment. Those are outputs of serve, not settings an operator provides.

## The serve lock

While serve runs, the data directory holds `.iroh-serve.lock`. Two readers depend on it:

- a second `id serve` in the same data directory cannot take the store while the first holds it;
- the REPL detects the lock at startup and switches to its Remote mode, proxying operations to the running serve instead of opening the store directly. See [The interactive REPL](../reference/repl.md).

The lock file name is `.iroh-serve.lock`, from the constant `SERVE_LOCK` in `id-core`. A module comment in `serve.rs` spells a different name, `.id-serve-lock`; that spelling is wrong and no file with that name is created. This is disagreement CLI-5 in the survey record. Deleting a lock file by hand is not a procedure this book documents: stop the serve process that owns the directory. What to do when a lock outlives its process is in [Troubleshooting](troubleshooting.md), marked there as unverified because the surveys record no stale-lock recovery path in the code.

## Publications on a running serve

Three client commands operate on a live serve and on nothing else: `id publish <WORLD>`, `id unpublish` and `id publications`. They connect with the serve node's own key, fail when no serve is running, and never edit local state. `publish` requires exactly one placement, `--listen <ADDR>` or `--path <PATH>`; giving neither or both is the error `exactly one placement is required: --listen ADDR or --path PATH`. The full treatment is in [serve and publications](../reference/cli-serve.md).

## The hosted edition: what an operator can run today

The hosted edition is a Cloudflare Worker package, `pkgs/id/hosted-worker`, with a per-user Durable Object class named `UserDO`. Its status is exact:

- The package is a **local-only scaffold**. Its `wrangler.toml` header states that the config is for `wrangler dev` and tests, and that no account-level deployment, production binding or billable resource is authorised by the package. Deployment and the real-account jurisdiction canary are gated on issues #2285 and #3403.
- An operator today can run it locally under `wrangler dev` and run its test harness (`wrangler.test.toml`). An operator today cannot deploy it as a production service from this package, point real users at it, or treat it as a running hosted node. There is no hosted URL in the source.
- Locally, the Worker **fails closed**: local workerd does not implement jurisdiction restrictions, so the application Worker returns 503 with the error `jurisdiction_unavailable` for every fully valid request, by design. That response locally is the correct behaviour, not an outage. See [Troubleshooting](troubleshooting.md).
- The Worker's front door does not authenticate end users itself. It consumes context headers (`X-Binding-Id`, `X-Jurisdiction`, and `X-Iroh-Id`) set by an authenticated internal caller; the binding ceremony that produces them is native code, in `pkgs/id/src/hosted_binding.rs`. The Worker strips every client-supplied internal header and re-sets them internally before forwarding to the `UserDO` through the jurisdiction-restricted namespace only.
- Its bindings, in the local config, are the Durable Object namespace `USERS` (class `UserDO`), the R2 bucket binding `BLOBS` (bucket name `id-hosted-blobs`, local emulation only), and the optional string variables `USER_CEILING_BYTES`, `BRIDGE_WAITING_PER_AUTHOR`, `BRIDGE_WAITING_TOTAL` and `BRIDGE_WAITING_MAX_AGE_MS`.

What the hosted edition would hold for a bound user, once deployed, is one `UserDO` per binding ID holding that user's key-value data, blob references and a signed-object bridge store. The routes are in [The hosted Worker API](../reference/hosted-worker.md) and the design is in [The hosted edition](../concepts/hosted.md). Neither chapter changes the status above: scaffold, local only.

A second package, `pkgs/id/worker-poc`, is a proof of concept that runs one example world in a Worker isolate and proves journal equality against the native wasmtime host. It is not a production path and an operator has no reason to run it outside its own tests.

## Exit behaviour for operators

Serve has no special exit codes of its own. The `id` binary's contract, from [The id command line](../reference/cli.md), is:

| Exit | Meaning |
|---|---|
| `0` | Success, including `--help` and `--version`. |
| `1` | A handler error. The error chain is printed to stderr as `Error: <message>`. Startup failures (malformed `--server-config`, the `--web-listen` conflict, a build without a required feature) are in this class. |
| `2` | A command-line parse error: unknown flag, missing required argument, conflicting flags, bad value. Usage is printed to stderr. |

A script that supervises serve should treat exit `1` at startup as a configuration fault to fix, not a transient fault to retry unchanged.
