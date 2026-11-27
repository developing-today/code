# serve and publications

Source: `pkgs/id/crates/id-cli/src/cli/mod.rs` (including `ServeArgs` and the flattened `WorldRuntimeArgs` in `pkgs/id/crates/id-cli/src/cli/world.rs`) and `pkgs/id/src/main.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/ — provenance only; this chapter is complete without following it.

## `serve`

```
id serve [FLAGS]
```

`serve` is the only hosting command. It runs indefinitely, hosting stored blobs, and optionally a world and the web UI. Every other command — including all `id world …` subcommands — is a client that connects out to a host; the client side is covered in [World commands: joining and playing](cli-worlds.md) and [World commands: administering and verifying](cli-worlds-admin.md). The global flags from [The id command line](cli.md) apply as usual; `--data-dir` selects the store the server hosts.

### Core and discovery flags

| Flag                     | Argument      | Default                    | Meaning                                                                                                                                                                                                              |
| ------------------------ | ------------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--ephemeral`            | none (switch) | off                        | Use in-memory storage instead of persistent disk storage. Content is lost when the server stops.                                                                                                                     |
| `--server-config`        | `FILE`        | none                       | Operator server-config TOML. A malformed file fails startup, naming the parse error. Loading the config requires a build with the `world` feature (see Feature gates).                                               |
| `--no-relay`             | none (switch) | off                        | Direct connections only; no relay servers. Connections through NATs or firewalls can fail in this mode.                                                                                                              |
| `--no-gossip`            | none (switch) | off                        | Disable gossip discovery entirely: the server accepts connections but does not join a gossip topic, does not announce itself, and does not discover peers. Conflicts with `--gossip`.                                |
| `--gossip`               | none (switch) | off                        | Force gossip discovery on, overriding a build-time disabled default. Conflicts with `--no-gossip`.                                                                                                                   |
| `--bootstrap`            | `IDS`         | none                       | Comma-separated node IDs for manual peer bootstrapping; the flag is repeatable and each occurrence is split on commas. Supplementary to automatic discovery.                                                         |
| `--topic`                | `TOPIC`       | see the topic caveat below | Gossip topic name for peer discovery. All servers on the same topic discover each other.                                                                                                                             |
| `--topic-secret`         | `SECRET`      | see the topic caveat below | Shared secret for topic access control, used to encrypt DHT bootstrap records. Only nodes with the same secret discover each other via DHT. The value is a secret: pass it by flag or configuration, never print it. |
| `--no-default-bootstrap` | none (switch) | off                        | Ignore the bootstrap node IDs from `defaults.json`; only `--bootstrap` nodes are used.                                                                                                                               |
| `--no-default-topic`     | none (switch) | off                        | Ignore the topic and secret from `defaults.json` and fall back to the hardcoded constants. Explicit `--topic`/`--topic-secret` values still take precedence.                                                         |
| `--no-mdns`              | none (switch) | off (mDNS on)              | Disable mDNS local peer discovery. By default the server uses mDNS on the local network and announces its node ID and addresses to every host there.                                                                 |
| `--dht`                  | none (switch) | off                        | Enable mainline DHT address lookup and publishing for the server's endpoint. The flag exists in the parser regardless of the `dht` cargo feature.                                                                    |
| `--iroh-port`            | `PORT`        | `0`                        | Port for the Iroh QUIC endpoint. `0` lets the OS assign a random available port.                                                                                                                                     |

Topic and secret default caveat: the strings `id-peer-discovery-v1` (topic) and `id-public-discovery-v1` (secret) are the hardcoded constants, and the constants are the fallback, not the operative default. On a stock serve, the topic and secret are read first from `defaults.json`; the constants apply only when `defaults.json` supplies neither, or when `--no-default-topic` is passed. The `--version` provenance line (see [The id command line](cli.md)) reports which defaults source is in force. The `--topic`/`--topic-secret` help text once presented the constants as "the default" (discrepancy CLI-6, resolved — the help now describes them as the fallback).

### Web UI flags

| Flag                  | Argument      | Default                                          | Meaning                                                                                                                                                                                                                                       |
| --------------------- | ------------- | ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--web`               | none (switch) | off                                              | Start the HTTP web UI. On a build without the `web` feature this fails at startup (see Feature gates). The UI itself is covered in [The web UI: pages and HTTP API](web-ui.md).                                                               |
| `--bind`              | `IP`          | `127.0.0.1`                                      | Address the web interface binds to. The loopback default makes the UI reachable only from this machine. Binding `0.0.0.0` exposes it on the network; set `--web-token` when doing so.                                                         |
| `--port`              | `PORT`        | `3000`                                           | Port for the web interface. `0` lets the OS assign a random available port.                                                                                                                                                                   |
| `--web-listen`        | `IP:PORT`     | none                                             | Add one web listener, in `IP:PORT` form (IPv6 in brackets, for example `[::1]:3001`). Repeatable. The set of `--web-listen` values replaces the `web_bindings` entries from `defaults.json` entirely. Subject to the explicitness rule below. |
| `--web-token`         | `TOKEN`       | none; environment fallback `ID_WEB_TOKEN`        | Require this token to use the web interface. A browser opens `http://host:port/?token=<TOKEN>` once and keeps a cookie; scripts send `Authorization: Bearer <TOKEN>`. The value is hidden in clap and is a secret: name it, never print it.   |
| `--web-cookie-secure` | none (switch) | off; environment fallback `ID_WEB_COOKIE_SECURE` | Mark the web session cookies `Secure`. Set it only when HTTPS is terminated in front of the server; browsers drop `Secure` cookies over plain HTTP except on localhost.                                                                       |

The `--web-listen` explicitness rule (#2145): combining `--web-listen` with an explicitly set `--bind` or `--port` is a startup error naming the conflict. Explicitness is read from the clap value sources in `src/main.rs`, so the clap defaults for `--bind` and `--port` do not count: `id serve --web-listen 127.0.0.1:8080` is valid, while `id serve --web-listen 127.0.0.1:8080 --port 3000` fails at startup even though `3000` is the default value, because the operator set it explicitly.

The `--web-token` warning: startup prints a warning when a web listener is bound without `--web-token`. The warning is the whole protection on a loopback-only listener; on a non-loopback bind the UI is reachable by anyone on the network who can reach the port.

Worked example (toy data; the token value is a placeholder name, not a real secret):

```
$ id serve --web --port 8080 --web-token $ID_WEB_TOKEN
```

### Gallery flags

| Flag                       | Argument      | Default                                               | Meaning                                                                                                                                                                                                      |
| -------------------------- | ------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--gallery-dir`            | `DIR`         | none; environment fallback `ID_GALLERY_DIR`           | Serve the program gallery stored in this directory: as a program registry over Iroh, and with `--web` also over the `/api/gallery/*` HTTP routes. Every bundle file in the directory is verified at startup. |
| `--gallery-accept-publish` | none (switch) | off; environment fallback `ID_GALLERY_ACCEPT_PUBLISH` | Accept bundle publishes from any peer over Iroh; each is verified and then goes through the gallery's admission. Requires `--gallery-dir`.                                                                   |

Gallery administration itself is covered in [Gallery, match-audit and walk](cli-gallery-walk.md).

### World hosting flags

Every flag in this section requires `--world`, except `--world` itself, which requires `--world-admin-token`.

| Flag                       | Argument      | Default                                                        | Meaning                                                                                                                                                                                                                                                                                      |
| -------------------------- | ------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--world`                  | none (switch) | off                                                            | Host a multiplayer world over Iroh (and over `/ws/world` with `--web`). Requires `--world-admin-token`.                                                                                                                                                                                      |
| `--world-admin-token`      | `TOKEN`       | none; environment fallback `ID_WORLD_ADMIN_TOKEN`              | Admin secret required to mint world guest capabilities. The value is hidden in clap and is a secret: name it, never print it.                                                                                                                                                                |
| `--world-module`           | `FILE`        | none                                                           | Start with this precompiled Wasm world module (Roc platform ABI). Requires `--world`. Loading a module requires a build with the `sandbox` feature (see Feature gates). Conflicts with `--world-source-only`.                                                                                |
| `--world-name`             | `NAME`        | `lobby`                                                        | Name of the world this server offers (one world per process). The world's files live in `.id-worlds/<NAME>/`. The name may contain `a-z`, `0-9`, `-` and `_`.                                                                                                                                |
| `--world-ssh-port`         | `PORT`        | none                                                           | Serve the world over SSH on this port, bound to `--bind`. Actual SSH serving exists only in `ssh`-feature builds (see Feature gates).                                                                                                                                                        |
| `--world-checkpoint-every` | `N`           | `1000`                                                         | Trim the world's journal behind a program snapshot every N events. `0` never trims.                                                                                                                                                                                                          |
| `--world-max-open`         | `N`           | `256`                                                          | Most worlds open at once; idle durable worlds close to make room.                                                                                                                                                                                                                            |
| `--world-max-sessions`     | `N`           | `1024`                                                         | Most world sessions at once, across every world.                                                                                                                                                                                                                                             |
| `--world-idle-secs`        | `N`           | `600` (`900` with `--world-host`)                              | Close durable worlds that no session used for this many seconds; they reopen from their journal on demand. `0` keeps worlds open, and is refused with `--world-host`, where `id host pin` keeps a world awake.                                                                               |
| `--world-host`             | none (switch) | off                                                            | Serve the worlds directory as a host-mode data root: wake asleep worlds on connect unless their wake mode is `operator`, never close pinned worlds, record idle closes as asleep, refuse archived worlds, and answer `id host` acts on `.id-worlds/host.sock`. Conflicts with `--ephemeral`. |
| `--world-boot-window-secs` | `N`           | `600`                                                          | Refuse signed world and directory requests for this many seconds after startup, so a request captured before a restart cannot be replayed into a server with empty replay memory. `0` disables the window.                                                                                   |
| `--world-native`           | none (switch) | off                                                            | Accept native (ELF) module installs. The guarantee for this tier is the OS sandbox (Landlock, seccomp, rlimits), so the tier is opt-in.                                                                                                                                                      |
| `--world-source-only`      | none (switch) | off; environment fallback `ID_WORLD_SOURCE_ONLY`               | Accept new world programs only as Roc source compiled by this server. Disables raw Wasm and native module uploads and `--world-module`. Conflicts with `--world-module`.                                                                                                                     |
| `--world-module-trust-key` | `HEX`         | none                                                           | Ed25519 module-signing public key trusted by this server, in hex. Repeatable. The flag name is singular (`…-key`); the parser field collects multiple keys.                                                                                                                                  |
| `--world-cap`              | `NAME`        | none                                                           | Capability the default world's program may use. Repeatable. `*` grants the whole catalog.                                                                                                                                                                                                    |
| `--world-cap-policy`       | `POLICY`      | `deny`                                                         | What an unlisted capability costs the program: `deny`, or `grant-on-use`, which grants a known capability the first time the program uses it.                                                                                                                                                |
| `--world-allowed-caps`     | `NAMES`       | the whole catalog                                              | Server-wide capability ceiling, comma-separated. A world may grant less, never more. `*` is refused here.                                                                                                                                                                                    |
| `--roc-bin`                | `BIN`         | `roc` from PATH; environment fallback `ID_ROC_BIN`             | The Roc binary used to compile sources sent to `id world compile`.                                                                                                                                                                                                                           |
| `--roc-platform`           | `DIR`         | `./examples/roc-world`; environment fallback `ID_ROC_PLATFORM` | The world platform directory used for on-the-fly compilation.                                                                                                                                                                                                                                |

### World runtime flags (flattened `WorldRuntimeArgs`)

These flags are defined by the flattened `WorldRuntimeArgs` struct and flattened into `ServeArgs`. Each requires `--world`.

| Flag                                         | Argument | Default    | Meaning                                                                                          |
| -------------------------------------------- | -------- | ---------- | ------------------------------------------------------------------------------------------------ |
| `--world-fuel`                               | `UNITS`  | `10000000` | Wasm fuel (instruction budget) for one world call.                                               |
| `--world-memory-mib`                         | `MIB`    | `16`       | Linear memory for a Wasm world program, in MiB (range 1..=2048).                                 |
| `--world-message-kib`                        | `KIB`    | `1024`     | Largest Wasm payload (input or output), in KiB (range 1..=1024).                                 |
| `--world-native-memory-mib`                  | `MIB`    | `256`      | Address-space cap for a native worker, in MiB (range 16..=65536).                                |
| `--world-native-cpu-secs`                    | `SECS`   | `10`       | CPU seconds a native worker may use in total.                                                    |
| `--world-native-deadline-ms`                 | `MS`     | `10000`    | Wall-clock budget for one native worker call, in milliseconds.                                   |
| `--world-total-memory-mib`                   | `MIB`    | `64`       | Total per-world logical memory budget, in MiB (range 1..=65536).                                 |
| `--world-guest-burn-concurrency`             | `COUNT`  | `0`        | Concurrent synchronous guest burns admitted hub-wide. `0` means auto (parallelism − 2).          |
| `--world-guest-burn-per-account-concurrency` | `COUNT`  | `0`        | Concurrent synchronous guest burns admitted per charged account. `0` means no per-account bound. |

### Confirmation-mail sinks

A world host can send confirmation mail through exactly one of three sinks. The three sink flags conflict with each other pairwise, and each requires `--world`.

| Flag                   | Argument  | Default | Meaning                                                                                                                                                     |
| ---------------------- | --------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--world-mail-outbox`  | `DIR`     | none    | Write confirmation mail into this directory, one private file per message.                                                                                  |
| `--world-mail-command` | `PROG`    | none    | Run this program for each confirmation mail. `serve` sets `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` in the program's environment for each message. |
| `--world-mail-smtp`    | `IP:PORT` | none    | Relay confirmation mail through this SMTP server. The server must be on loopback and must forward with TLS. Requires `--world-mail-from`.                   |
| `--world-mail-from`    | `ADDR`    | none    | Sender address for mail sent with `--world-mail-smtp`. Requires `--world-mail-smtp`.                                                                        |

Worked example (toy data):

```
$ id serve --world --world-admin-token $ID_WORLD_ADMIN_TOKEN --world-name toy-lobby --world-mail-outbox ./mail-outbox
```

### Write access flags

| Flag            | Argument      | Default | Meaning                                                                                                                                                                                                             |
| --------------- | ------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--allow-node`  | `IDS`         | none    | Allow this node to modify the store. Repeatable and comma-separated. The same list can be supplied by the `.iroh-allowed` file, one node ID per line. This machine's own server and client keys are always allowed. |
| `--open-writes` | none (switch) | off     | Let every peer modify the store. The parser documentation labels this flag "insecure" and scopes it to demos and trusted networks.                                                                                  |

Reading names, hashes and tags is public. Writing — `put`, delete, rename, copy, tag changes and blob pushes — requires the writer's node ID to be listed through `--allow-node` or `.iroh-allowed`, or requires `--open-writes`.

### Serve runtime environment

Three environment variables are read by the serve handler outside clap: `ID_WORLD_COMPILE_CACHE_DIR` and `ID_WORLD_COMPILE_CACHE_LRU_BYTES` (the world compile cache), and `ID_APPS_DIR`. The mail variables `ID_MAIL_TO`, `ID_MAIL_SUBJECT` and `ID_MAIL_BODY` are set by `serve` for `--world-mail-command`, not read from the operator's environment. The full environment list is covered in [Configuration and environment](configuration.md).

### Feature gates

| Requested surface                                   | Gate                    | Behaviour when the feature is off                                                                                                                                                            |
| --------------------------------------------------- | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--web`, `--web-listen`, explicit `--port`/`--bind` | `web` cargo feature     | Startup fails with: "the web interface was requested (--web/--web-listen/--port/--bind), but this id binary was built without the `web` feature; rebuild with `--features web` to serve it". |
| `--server-config`                                   | `world` cargo feature   | Startup fails with a message stating the binary was built without the `world` feature.                                                                                                       |
| `--world-module`                                    | `sandbox` cargo feature | Startup fails with: "--world-module requires a build with the `sandbox` feature".                                                                                                            |
| `--world-ssh-port`                                  | `ssh` cargo feature     | The flag parses in every build; SSH serving exists only in `ssh`-feature builds (the Nix `id-web` package has it; `id-lib` does not).                                                        |

The default Cargo build (`default = [sandbox, world]`) has no web UI and no world SSH serving. The Nix `id-web` package (the default package) is built with `web + sandbox + ssh + world`; the Nix `id-lib` package is built with `sandbox + world` only.

### Serve failure shapes and exit behaviour

`serve` runs indefinitely; a clean stop is by interruption. Startup failures are handler errors and exit 1, including: a malformed `--server-config` file (the error names the TOML parse error); `--web-listen` combined with an explicitly set `--bind` or `--port`; the feature-gate failures in the table above. Giving two of the three mail sinks, `--gossip` with `--no-gossip`, or `--world` without `--world-admin-token` is a clap error and exits 2. The process-wide rules are stated in [The id command line](cli.md).

## Publications: `publish`, `unpublish`, `publications`

The publications trio manages dynamic publications on a running serve (#2147). All three are operator-only: each connects with the serve node's own key, and each fails when no serve is running. None of the three edits local state directly; each drives the running serve's publication manager. A publication made this way goes live without restarting the node and persists across restarts.

### `publish`

```
id publish <WORLD> --listen <ADDR>
id publish <WORLD> --path <PATH> [--acknowledge-shadow]
```

| Flag                   | Argument      | Default | Meaning                                                                                                                                 |
| ---------------------- | ------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `--listen`             | `ADDR`        | none    | Publish on a dedicated listener at this address (for example `127.0.0.1:8080`; port `0` is OS-assigned). Conflicts with `--path`.       |
| `--path`               | `PATH`        | none    | Publish as a path mount on the primary Explorer listener (for example `/prog`). Conflicts with `--listen`.                              |
| `--acknowledge-shadow` | none (switch) | off     | Acknowledge that a path mount shadows an Explorer route. Required to publish a mount that shadows one; without it the mount is refused. |

Behaviour notes:

- Exactly one of `--listen` and `--path` is required. The requirement is enforced at runtime, not by clap: giving neither, or both, fails with `exactly one placement is required: --listen ADDR or --path PATH` (exit 1). Giving both is not a clap conflict, because `--path` carries no parser-level conflict in `publish`; the runtime check fires instead.

Worked example (toy data):

With a serve already running (started, for example, by `id serve --web --web-token $ID_WEB_TOKEN` in another terminal):

```
$ id publish toy-lobby --path /toy
$ id publish toy-lobby
Error: exactly one placement is required: --listen ADDR or --path PATH
```

The first command mounts the world `toy-lobby` at `/toy` on the running serve. The second fails: it supplies no placement, and the runtime check rejects the invocation with the placement error and exit 1.

Failure shapes: no serve running (handler error, exit 1); a protocol mismatch against the running serve fails with `unexpected response` or `unexpected response from serve: …` (exit 1); a shadowing path mount without `--acknowledge-shadow` is refused (exit 1).

### `unpublish`

```
id unpublish --listen <ADDR>
id unpublish --path <PATH>
```

| Flag       | Argument | Default | Meaning                                                                                            |
| ---------- | -------- | ------- | -------------------------------------------------------------------------------------------------- |
| `--listen` | `ADDR`   | none    | Unpublish the dedicated listener at this address. Conflicts with `--path` (parser-level conflict). |
| `--path`   | `PATH`   | none    | Unpublish the path mount at this path.                                                             |

Behaviour notes:

- Removes a dynamic publication from the running serve without restarting the node: the listener closes, or the Explorer route is restored.
- Unlike `publish`, `unpublish` declares the `--listen`/`--path` conflict in the parser, so giving both is a clap error (exit 2).

Worked example (toy data):

```
$ id unpublish --path /toy
```

Failure shapes: no serve running is a handler error (exit 1). Giving both `--listen` and `--path` exits 2.

### `publications`

```
id publications
```

| Flag   | Argument | Default | Meaning                                                                                    |
| ------ | -------- | ------- | ------------------------------------------------------------------------------------------ |
| (none) | —        | —       | `publications` takes no arguments and defines no flags of its own; the global flags apply. |

Behaviour notes:

- Lists the running serve's dynamic publications, with their live listener addresses.

Worked example (toy data):

```
$ id publications
/toy
127.0.0.1:8080
```

Failure shapes: no serve running is a handler error (exit 1).
