# fleet proxy

The session proxy is the broker every sandbox seat talks through to reach the laptop's services. The wire is 8-byte big-endian length-prefixed JSON frames. The broker persists nothing: the routing table comes from config, tunnels live in memory, and token files are read per request and never stored or logged.

The broker's services map (port → service) is built in: 4799 laptop-agent, 4800 secrets-broker, 4801 queue-broker, 21118 rustdesk. The session broker socket defaults to `~/.cache/laptop-agent/broker.sock` (`FLEET_BROKER_SOCKET` / `[proxy] broker_socket`).

The domain's argv0 alias is `laptop-agent-broker`. Capabilities: `proxy-serve` (serve), `proxy-connect` (connect/init/get/put/ssh — present in every build), `proxy-raw-splice` (splice).

See also [fleet serve](serve.md) and [Proxy, sessions and recovery](../concepts/proxy.md).

## fleet proxy serve

```
fleet proxy serve
```

| Flag | Argument | Default | Meaning                                                                                                                |
| ---- | -------- | ------- | ---------------------------------------------------------------------------------------------------------------------- |
| —    | —        | —       | Takes no arguments. `--help` prints the usage, exit 0; any other trailing argument is a usage error naming it, exit 2. |

### Behaviour

Binds the session broker Unix socket and serves in the foreground. If a live broker already owns the socket, it exits quietly; if the socket file is stale, it unlinks and binds.

- **Warm tunnel pools:** per-port pools of warm CONNECT tunnels to the laptop's services; pool size from `LAPTOP_AGENT_POOL`, compiled default **50**. A request takes a pooled connection and on any failure retries **once on a fresh one**. Pool overflow policy: idle → grow (below the live ceiling) → wait briefly for a returned tunnel → grow anyway; a request never fails because of the pool.
- **Two reaches to the laptop:** the **Cloudflare leg is primary** for the laptop-agent daemon port _when its Access file exists_ (`~/tools/laptop-agent-cf-access.json`, a JSON `{client_id, client_secret}` file, mode 600, read per request, sent only as `CF-Access-*` headers, never stored or logged; no file, no leg) — TLS through the platform egress proxy to the tunnel hostname `sandbox-link.security.cab`, whose ingress lands on the daemon's own listener laptop-side, token auth preserved end to end. The **tailnet leg** (CONNECT through the host egress proxy to the tailnet target host `100.99.113.103`) is the automatic fallback, on connection failure _and_ on a Cloudflare edge error status (502/503/530) alike. Pools and recovery gates key on `(port, transport)` so the legs never share a bucket.
- **Hello negotiation:** sessions speak the legacy dialect (**proto 0**: request frames from the first frame) or negotiate a hello first (**proto 1**, fleet clients), decided per connection by peeking at the first frame, never by configuration. The hello response carries the services map (port → `{name, version}`) and the broker's capability list. A too-old peer gets the proto-too-old frame and exit 69 on the client side.
- **Keepalive and idle exit:** a keepalive thread pings the laptop-agent's `/health` every **180 s**. The broker exits once _client requests_ have been idle for `idle_exit_secs` (`[proxy] idle_exit_secs`, default **3600** s; 0 is rejected at config load) — keepalive traffic does **not** count as use. In a multi-role serve list the idle exit is disabled (co-hosted mode).
- **Colocated brokers:** for the queue/secrets broker ports, the broker first tries the role's own Unix socket with a bounded dial (the dial _is_ the probe; a socket that answers within the deadline is used, any other outcome selects TCP within the same attempt) before tunnelling.

Port 3130 note: port **3130** is the host **egress proxy** port — the CONNECT proxy named by the session's `HTTPS_PROXY` environment. It is not a fleet listener. The tailnet leg's shape is: fleet → HTTP CONNECT to the egress proxy at `<egress host>:3130` → tailnet target `100.99.113.103:<service port>`. Fleet contains no 3130 listener to restart; 3130 belongs to the session/host layer.

### Exit codes

- `0` — clean exit, including "a live broker already owns the socket".
- `2` — usage (trailing argument).
- `69` — capability unavailable in this invocation.

### Example

```console
$ fleet proxy serve
```

Binds the broker socket and serves until client requests have been idle for 3600 s.

## fleet proxy connect

```
fleet proxy connect [--print-path] [--stdio [--host H] [--port N]] [--request METHOD PATH [--service-port N] [--token-file F] [--body JSON]]
```

| Flag             | Argument      | Default                            | Meaning                                                                                                                                    |
| ---------------- | ------------- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `--print-path`   | —             | off                                | Print the PATH shim directory and exit. Mutually exclusive with `--request` and `--stdio`.                                                 |
| `--stdio`        | —             | off                                | Raw relay spelling (SSH's ProxyCommand shape): relay stdin/stdout over one tunnel. Mutually exclusive with `--request` and `--print-path`. |
| `--host`         | `H`           | config target host                 | Tunnel destination host. Requires `--stdio`.                                                                                               |
| `--port`         | `N`           | `22`                               | Tunnel destination port. Requires `--stdio`.                                                                                               |
| `--request`      | `METHOD PATH` | —                                  | Issue exactly one framed request on the negotiated stream and print `{"status": …, "body": …}` as JSON.                                    |
| `--service-port` | `N`           | the request's implied route (4799) | Select the laptop service port. Must follow `--request`.                                                                                   |
| `--token-file`   | `F`           | —                                  | Name the token file the broker should read for this request. Must follow `--request`.                                                      |
| `--body`         | `JSON`        | —                                  | Request body; must be valid JSON. Must follow `--request`.                                                                                 |

Each flag may be given at most once; a repeat is a usage error.

### Behaviour

Connect to the session proxy: **ensure a broker answers (starting one when none does)**, sync the PATH shims from its services map, and optionally issue one request over the negotiated stream. The ensure step spawns `fleet serve proxy` detached when the socket does not answer, then waits for the hello. A legacy (proto 0) answer still counts as running; its services map is the built-in table. The negotiated services map regenerates the PATH shim directory; `--print-path` prints that directory and exits (the shell-integration spelling).

### Exit codes

- `0` — ok.
- `2` — usage (repeated flag, bad combination, invalid `--body` JSON).
- `3` — transport failure.
- `69` — protocol / capability unavailable (including a proto-too-old peer).

### Example

```console
$ fleet proxy connect --request GET /health --service-port 4799
{"status": 200, "body": {"ok": true}}
```

Ensures a broker, then issues one health request to the laptop-agent service.

## fleet proxy init

```
fleet proxy init
```

| Flag | Argument | Default | Meaning             |
| ---- | -------- | ------- | ------------------- |
| —    | —        | —       | Takes no arguments. |

### Behaviour

Byte-identical to the `fleet init` built-in — one implementation lives in the proxy client and both spellings resolve identically. It autostarts this host's fleet roles per the `[host]` manifest: on the sandbox, probe the broker socket and sync PATH shims when a broker answers (a strict no-op when one is already running — init never fights an incumbent broker), start one when none does, and probe/start the standing loopback secrets broker. An unknown location reports state, starts nothing, and exits 69.

### Exit codes

- `0` — roles running or correctly reported.
- `2` — session initialisation error, before any role work.
- `69` — unknown location, or this build cannot run `init`.

### Example

```console
$ fleet proxy init
```

Autostarts the sandbox roles (proxy + secrets-broker) if they are not running.

## fleet proxy ssh

```
fleet proxy ssh -- [ssh args...]
```

| Flag | Argument | Default         | Meaning                                                                                                                                                                 |
| ---- | -------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--` | —        | required, first | Literal separator. Everything after it is SSH arguments. A second `--` inside the SSH arguments is an error: "the SSH destination is fixed; do not pass a second '--'". |

### Behaviour

Runs system OpenSSH through the fleet proxy **as a recovery path** — the route you use when the normal laptop channel is what is broken. Everything that decides where the connection lands is pinned by fleet; the caller supplies only extra SSH options and an optional remote command. Option parsing walks clusters character-by-character so clustered forms like `-vL` split correctly.

The pinned argv shape:

- Destination `user@<target host>` with `HostName=<target host>` (default `100.99.113.103`), `User=user`.
- Identity `~/.ssh/id_ed25519` with `IdentitiesOnly=yes`.
- `UserKnownHostsFile=~/.ssh/known_hosts_laptop`, `StrictHostKeyChecking=yes`.
- `ConnectTimeout=60`, `ConnectionAttempts=3`, `ServerAliveInterval=15`, `ServerAliveCountMax=8`.
- `ControlMaster=auto` with `ControlPath=~/.ssh/cm-%r@%h:%p` and `ControlPersist=3600`.
- `ProxyCommand=<this fleet binary> proxy connect --stdio --host <target host> --port %p`.

The fleet proxy credential stays in the environment and is sent only by the CONNECT relay, never in the SSH argv. The command `exec`s the system `ssh` (Unix only; elsewhere exit 69). Its exit code is ssh's own.

### Exit codes

- ssh's own exit code on success of the exec.
- `2` — usage (missing `--`, or a second `--` in the SSH arguments).
- `69` — non-Unix platform.

### Example

```console
$ fleet proxy ssh -- hostname
amd
```

Opens the pinned recovery SSH connection and runs `hostname` on the laptop.

## fleet proxy get

```
fleet proxy get [--mode OCTAL] [--no-resume] <laptop-path> <local-path | ->
```

| Flag                 | Argument | Default             | Meaning                                                        |
| -------------------- | -------- | ------------------- | -------------------------------------------------------------- |
| `--mode`             | `OCTAL`  | destination default | File mode for the downloaded file.                             |
| `--no-resume`        | —        | off (resume on)     | Do not resume from an existing deterministic temp; start over. |
| `-` (as destination) | —        | —                   | Stream to stdout instead of a local path.                      |

### Behaviour

Downloads a file from the laptop over the agent channel, through the broker to the laptop-agent daemon's file routes. Chunks are **2 MiB raw chunks, base64-encoded**, carried in JSON envelopes (`stat`, `begin`, `commit`, `abort`, `read`). Chunks land in a deterministic local temp — `.<name>.fleet-part-<sha256[..12]>` beside the destination — so an interrupted transfer resumes by re-issuing `begin` and continuing from the temp's size. The whole-file **SHA-256** from the daemon's `stat` must match before the temp is renamed over the destination. `-` streams to stdout (the hash is still verified at the end; a mismatch fails the command _after_ the bytes were emitted — check stderr). **Recursive get is deferred in v1**: the daemon protocol has no directory-listing route.

### Exit codes

- `0` — ok, hash verified.
- `2` — usage.
- `3` — transport.
- `69` — capability unavailable.

### Example

```console
$ fleet proxy get /home/user/build/result.tar ./result.tar
```

Downloads the file with resume support and verifies its SHA-256 before renaming it into place.

## fleet proxy put

```
fleet proxy put [--mode OCTAL] [--no-resume] <local-path | -> <laptop-path>
fleet proxy put -r [--mode OCTAL] [--no-resume] <local-dir> <laptop-dir>
```

| Flag            | Argument | Default                                           | Meaning                                                        |
| --------------- | -------- | ------------------------------------------------- | -------------------------------------------------------------- |
| `--mode`        | `OCTAL`  | `0644` for stdin; otherwise the local file's mode | File mode on the laptop.                                       |
| `--no-resume`   | —        | off (resume on)                                   | Do not resume from an existing deterministic temp; start over. |
| `-r`            | —        | off                                               | Recursive: upload a directory tree.                            |
| `-` (as source) | —        | —                                                 | Read stdin (spooled locally first).                            |

### Behaviour

Uploads a file to the laptop over the agent channel. The file is hashed (SHA-256), uploaded in **2 MiB chunks** to a deterministic temp on the laptop — `.<name>.fleet-part-<sha256[..12]>` beside the destination — and **atomically renamed over the destination only after the daemon verifies the hash**. An interrupted put resumes from the temp unless `--no-resume` is given. Progress and the final sha256 go to stderr.

### Exit codes

- `0` — ok, hash verified by the daemon and rename committed.
- `2` — usage.
- `3` — transport.
- `69` — capability unavailable.

### Example

```console
$ fleet proxy put ./result.tar /home/user/build/result.tar
```

Uploads the file in 2 MiB chunks; the destination only appears once its hash verifies.

## fleet proxy splice

```
fleet proxy splice
```

| Flag | Argument | Default | Meaning             |
| ---- | -------- | ------- | ------------------- |
| —    | —        | —       | Takes no arguments. |

### Behaviour

Listens on **`127.0.0.1:21118`** (loopback only, a pinned constant) and relays each accepted TCP stream through the fleet broker to the laptop's RustDesk direct-access service (port 21118 laptop-side). Requires the broker to advertise raw-splice support in its hello (the `proxy-raw-splice` capability plus the service flag); a legacy or non-advertising broker is a transport error, not a silent fallback. This is the RustDesk direct-access path that avoids the relayed session.

### Exit codes

- `0` — clean exit.
- `2` — usage (trailing argument).
- `3` — transport (broker does not advertise raw-splice).
- `69` — capability unavailable.

### Example

```console
$ fleet proxy splice
```

Starts the loopback splice listener on 127.0.0.1:21118 until stopped.
