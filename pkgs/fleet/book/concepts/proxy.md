# Proxy, sessions and recovery

The _session proxy_ is how a sandbox seat reaches the laptop's services. The
proxy domain's verbs are `serve`, `connect`, `init`, `ssh`, `splice`,
`get`, `put`, and `copy`, gated per verb on the capabilities `proxy-serve` (serve),
`proxy-connect` (connect/init/get/put/copy/ssh), and `proxy-raw-splice`
(splice). The proxy succeeds an earlier Python trio of broker/connect
scripts, behaviour for behaviour. The wire is the `fleet-proxy-proto`
protocol alone: **8-byte big-endian length-prefixed JSON** frames.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-proxy/src/serve.rs

## The session broker socket

`fleet proxy serve` runs the broker. It takes no arguments (a trailing
argument is a usage error naming it, exit 2; `--help` prints the usage,
exit 0). It binds the **session broker Unix socket** from fleet config —
`FLEET_BROKER_SOCKET` or `[proxy] broker_socket`, default
`~/.cache/laptop-agent/broker.sock`. If a live broker already owns the
socket, it exits quietly; if the socket file is stale, it unlinks and
binds.

The broker persists nothing: its routing table comes from config, its
tunnels live in memory, and token files are read per request and never
stored or logged. The routing table — the _services map_ — is port →
service name: **4799 laptop-agent, 4800 secrets-broker, 4801 queue-broker,
21118 rustdesk**. The hello response carries that map and the broker's
capability list.

**Hello negotiation.** Sessions speak the legacy dialect (**proto 0**:
request frames from the first frame — the old Python clients) or negotiate
a hello first (**proto 1**, fleet clients), decided per connection by
peeking at the first frame, never by configuration. A too-old peer gets the
proto-too-old frame and exit 69 on the client side.

**Keepalive and idle exit.** A keepalive thread pings the laptop-agent's
`/health` every **180 s**. The broker exits once _client requests_ have
been idle for `idle_exit_secs` (`[proxy] idle_exit_secs`, default **3600 s**;
0 is rejected at config load) — keepalive traffic does not count as use. A
warmth cache that exits when cold is a feature on a shared sandbox. Inside
a multi-role `fleet serve` list, the idle exit is disabled, so an idle
proxy cannot exit the shared process and kill its sibling roles.

**Colocated brokers.** For the queue/secrets broker ports, the broker first
tries the role's own Unix socket with a bounded dial — the dial _is_ the
probe; a socket that answers within the deadline is used, any other outcome
selects TCP within the same attempt — before tunnelling to the laptop.

## Warm pools

The broker keeps per-port pools of warm CONNECT tunnels to the laptop's
services. Pool size comes from `LAPTOP_AGENT_POOL`, compiled default
**50** (sized from a concurrency benchmark: the old default of 12 cliffed at
roughly 12 concurrent consumers; 32 already restored direct-class latency).
A request takes a pooled connection and on any failure retries **once on a
fresh one**. Overflow policy: idle → grow (below the live ceiling) → wait
briefly for a returned tunnel → grow anyway. A request never fails because
of the pool. Pools key on `(port, transport)`, so the two laptop reaches
(below) never share warmth or failure state.

## Two reaches to the laptop: the CF leg and the tailnet fallback

Sandbox↔laptop transport lives in this binary. There are two legs:

1. **The Cloudflare leg is primary** for the laptop-agent daemon port
   _when its Access file exists_: `~/tools/laptop-agent-cf-access.json` — a
   JSON `{client_id, client_secret}` file, mode 600, read per request, sent
   only as `CF-Access-*` headers, never stored or logged. No file, no leg.
   The leg is TLS through the platform egress proxy to the tunnel hostname
   `sandbox-link.security.cab`, whose ingress lands on the daemon's own
   listener laptop-side; the daemon's token auth is preserved end to end.
2. **The tailnet leg is the automatic fallback**: a CONNECT through the
   host egress proxy to the tailnet target host `100.99.113.103`. It is
   selected on connection failure _and_ on a Cloudflare edge error status
   (502/503/530) alike.

Port **3130** is the host egress proxy port — the CONNECT proxy named by
the session's `HTTPS_PROXY` environment. It is **not a fleet listener**.
The tailnet leg's shape is: fleet → HTTP CONNECT to the egress proxy at
`<egress host>:3130` → tailnet target `100.99.113.103:<service port>`. (The
URL's own port in `HTTPS_PROXY` is not used; 3130 is fixed by the tunnel
setup.) Fleet contains no 3130 listener to restart — 3130 belongs to the
session/host layer, and the recovery story below is layered accordingly.

## `connect` and `init`

`fleet proxy connect` ensures a broker answers — spawning `fleet serve
proxy` detached when the socket does not answer, then waiting for the hello
(a legacy proto-0 answer still counts as running; its services map is the
built-in table) — syncs the PATH shims from the broker's services map, and
optionally issues one request over the negotiated stream:

```
fleet proxy connect [--print-path] [--stdio [--host H] [--port N]]
      [--request METHOD PATH [--service-port N] [--token-file F] [--body JSON]]
```

`--print-path` prints the shim directory and exits (the shell-integration
spelling). `--request` prints `{"status": …, "body": …}` as JSON;
`--service-port N` selects the laptop service port (default: the request's
implied route, 4799), `--token-file F` names the token file the broker
should read for this request, and `--body JSON` must be valid JSON.
`--stdio` is the raw relay spelling (SSH's ProxyCommand shape): `--host H`
(default: the config target host) and `--port N` (default 22) relay
stdin/stdout over one tunnel. Flag discipline: each flag at most once;
`--stdio` and `--print-path` are mutually exclusive with `--request`;
`--host`/`--port` require `--stdio`; `--service-port`/`--token-file`/
`--body` must follow `--request`. Exit codes: 2 usage, 3 transport, 69
protocol/capability unavailable.

`fleet init` (byte-identical to `fleet proxy init` — one implementation)
autostarts this host's roles per the `[host]` manifest. Manifest resolution:
`FLEET_HOST_CONFIG` (a complete inline manifest; wins outright) →
`FLEET_LOCATION` (roles from the config file's `[host]` only if its location
matches, else the built-in map) → the config file's `[host]` section. Only
two built-in locations exist: `sandbox` (init mode `spawn`; roles `proxy`,
`secrets-broker`) and `laptop` (init mode `systemd`; roles `laptop-agent`,
`queue-broker`, `search`, `secrets-broker`). The full role vocabulary is
five names — those four plus `proxy`.

- **Unknown location:** init reports state (every source consulted, plus
  detected hostname/OS/systemd for the report only — detection never selects
  roles), starts nothing, and exits **69**. It never guess-starts a role.
- **Spawn mode (sandbox):** for the proxy, init probes the broker socket and
  says hello; a live broker means status plus PATH-shim sync and out — a
  strict no-op, never a spawn, never a restart; init never fights an
  incumbent broker. Only an unanswered socket spawns `fleet serve proxy`
  detached. For the secrets broker, a live socket answer means status and
  out; otherwise init spawns `fleet serve secrets-broker` detached with
  `SECRETS_BROKER_BIND=127.0.0.1:4800` set explicitly, then waits (50 ×
  100 ms probes) for the socket to answer. Any other role in a spawn
  manifest is reported and skipped, never spawned.
- **Systemd mode (laptop):** init never detached-spawns a daemon systemd
  owns. For each role it reports `fleet-<role>.service` state via
  `systemctl --user is-active`; an active unit is a no-op; an inactive but
  existing unit is started; a missing unit is reported and skipped, not an
  error.
- Roles the manifest itself disables are reported as skipped with their
  reason, and the `[domains]` config gate is applied after the manifest and
  wins over it.

## Recovery: connect and ssh pinning

The recovery story is layered: the broker's Cloudflare leg is independent
of the tailnet CONNECT path, so it carries the laptop-agent port when the
tailnet leg is what is broken; `proxy connect` re-ensures a dead broker;
stale sockets are unlinked and rebound. The operator's recovery shell is
`fleet proxy ssh`:

```
fleet proxy ssh -- [ssh args...]
```

It runs system OpenSSH through the fleet proxy, as the route you use when
the normal laptop channel is what is broken. Everything that decides where
the connection lands is **pinned by fleet**; the caller supplies only extra
SSH options and an optional remote command. A literal `--` is required
first; a second `--` inside the SSH arguments is an error ("the SSH
destination is fixed; do not pass a second '--'").

The pinned argv: destination `user@<target host>` with
`HostName=<target host>` (default `100.99.113.103`), `User=user`, identity
`~/.ssh/id_ed25519` with `IdentitiesOnly=yes`,
`UserKnownHostsFile=~/.ssh/known_hosts_laptop`,
`StrictHostKeyChecking=yes`, `ConnectTimeout=60`, `ConnectionAttempts=3`,
`ServerAliveInterval=15`, `ServerAliveCountMax=8`, `ControlMaster=auto`
with `ControlPath=~/.ssh/cm-%r@%h:%p` and `ControlPersist=3600`, and
`ProxyCommand=<this fleet binary> proxy connect --stdio --host <target
host> --port %p`. The fleet proxy credential stays in the environment and is
sent only by the CONNECT relay — never in the SSH argv. The command `exec`s
the system `ssh` (Unix only; elsewhere exit 69).

## `get`/`put`: verified transfers

Both verbs talk to the laptop-agent daemon's file routes (port 4799)
through the broker, in JSON envelopes carrying **2 MiB raw chunks,
base64-encoded** (about 2.67 MiB on the wire, far under the 64 MiB frame
ceiling). The whole file's SHA-256 is verified at commit (put) and after
the last chunk (get). Transfer temps are deterministic —
`.<name>.fleet-part-<sha256[..12]>` beside the destination — so an
interrupted transfer resumes by re-issuing `begin` and continuing from the
temp's size.

- `fleet proxy put [--mode OCTAL] [--no-resume] <local-path | -> <laptop-path>`
  hashes the file, uploads it in chunks to the deterministic temp on the
  laptop, and **atomically renames it over the destination only after the
  daemon verifies the hash**. The recursive form is `put -r … <local-dir>
<laptop-dir>`. `-` reads stdin (spooled locally first; mode defaults to
  0644). Progress and the final sha256 go to stderr.
- `fleet proxy get [--mode OCTAL] [--no-resume] <laptop-path> <local-path | ->`
  lands chunks in a deterministic local temp; the SHA-256 from the daemon's
  `stat` must match before the temp is renamed over the destination. `-`
  streams to stdout — the hash is still verified at the end, and a mismatch
  fails the command _after_ the bytes were emitted, so check stderr.
  **Recursive get is deferred in v1**: the daemon protocol has no
  directory-listing route.

Exit codes for both: 2 usage, 3 transport, 69 capability-unavailable.

## Splice: raw TCP for RustDesk

`fleet proxy splice` takes no arguments. It listens on **`127.0.0.1:21118`**
(loopback only — a pinned constant with a test asserting both properties)
and relays each accepted TCP stream through the fleet broker to the
laptop's RustDesk direct-access service (port 21118 laptop-side). It
requires the broker to advertise raw-splice support in its hello (the
`proxy-raw-splice` capability plus the service flag); a legacy or
non-advertising broker is a transport error, not a silent fallback. This is
the RustDesk direct-access path that avoids the relayed session.
