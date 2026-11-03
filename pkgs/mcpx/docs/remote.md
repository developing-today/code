# Reaching a daemon on another machine

The daemon speaks HTTP on a unix socket, and every client surface — the
CLI, scripts, the TUI, MCP — goes through the same client. Which means a
daemon on another machine is a transport question, not a feature: point
`daemon.endpoint` at it and everything works there.

Three routes exist, in order of how much setup they need.

## 1. `ssh://` — nothing to set up

```sh
export MCPX_DAEMON_ENDPOINT=ssh://buildbox
mcpx ls
mcpx tui
```

The client runs your own `ssh` binary (`ssh -T -o BatchMode=yes buildbox
mcpx stdio-proxy`) and speaks the daemon's HTTP API through it. Your ssh
agent, `ssh_config`, `known_hosts`, jump hosts and `Match` rules are the
access control, because it is literally your ssh doing the connecting.
Nothing listens on the remote host beyond the daemon's own unix socket, and
no mcpx credential exists to leak.

Details worth knowing:

- The remote socket path is resolved **by the remote process**, from mcpx's
  own paths and configuration file there. The client makes no assumption
  about the remote filesystem layout.
- `?bin=/path/to/mcpx` names a remote mcpx that is not on `PATH`:
  `ssh://buildbox?bin=/opt/mcpx/bin/mcpx`.
- `?sock=/run/user/1000/mcpx/daemon.sock` picks a specific remote daemon,
  for a host running more than one.
- `user@` and `:port` work as in any ssh target:
  `ssh://drew@buildbox:2222`.
- `BatchMode=yes` is set, so ssh never stops to ask for a password or a
  passphrase: an HTTP request has no terminal to answer on. Load the key in
  your agent, or use a key with no passphrase.
- A remote endpoint is **never** auto-started. If nothing answers, the error
  names the host and the exact remote command that was tried; start the
  daemon there with `ssh buildbox mcpx daemon` (or `mcpx daemon
--detached`).
- Server-sent events (`/v1/events`, `mcpx log --follow`) stream over the
  same tunnel; there is no separate event path.

`mcpx stdio-proxy` is the remote half and is a normal command: it dials the
local daemon socket and copies stdin/stdout to it. Running it by hand is the
quickest way to check a host:

```sh
ssh buildbox mcpx stdio-proxy --sock /run/user/1000/mcpx/daemon.sock
```

## 2. `ssh -L` — forward the socket itself

For tools that cannot run `ssh` themselves, or when you want one tunnel for
many shells, forward the unix socket as a local socket:

```sh
ssh -N -L /tmp/buildbox-mcpx.sock:/home/you/.local/state/mcpx/daemon.sock buildbox &
export MCPX_DAEMON_ENDPOINT=unix:///tmp/buildbox-mcpx.sock
mcpx status
```

The forward is a plain byte pipe, so it is the same thing the `ssh://`
transport does, just started by you instead of by the client. The remote
path must be the socket the daemon there is actually using — `mcpx status
--json` on the remote host prints it (`socket`).

`ssh -L` also works in the other direction of convenience: `-L
127.0.0.1:8065:127.0.0.1:8065` forwards the daemon's loopback TCP port, for
a remote daemon started with `daemon.address` widened or a container that
serves TCP. Prefer the socket: it is the same filesystem permission check
as being there.

## 3. `ssh -R` — a daemon behind NAT

The two routes above dial out from the client, which fails when the daemon's
host cannot be reached at all. Reverse the direction instead: the daemon's
host connects out to a host that is reachable, and publishes the socket
there.

On the daemon's host (behind NAT), forward its socket to a socket on the
public host:

```sh
ssh -N -R /tmp/team-mcpx.sock:/home/you/.local/state/mcpx/daemon.sock public-host
```

Then, anywhere that can reach `public-host`:

```sh
ssh -N -L /tmp/mcpx.sock:/tmp/team-mcpx.sock public-host &
MCPX_DAEMON_ENDPOINT=unix:///tmp/mcpx.sock mcpx ls
```

Or, with no local forward at all, use `ssh://` from the middle host, since
the socket is already there:

```sh
ssh public-host MCPX_DAEMON_ENDPOINT=unix:///tmp/team-mcpx.sock mcpx ls
```

`ssh -R` needs `AllowStreamLocalForwarding` (the default) on the server, and
`StreamLocalBindUnlink=yes` in the forwarding client's `ssh_config` to
replace a stale socket after a dropped connection.

## 4. `mcpx serve` over ssh — MCP to a remote daemon

An MCP host that cannot set an endpoint can spawn ssh instead of mcpx: see
the MCP-over-ssh form in [`protocol.md`](./protocol.md#24-transport-and-identity).
The short version:

```json
{ "mcpServers": { "mcpx": { "command": "ssh", "args": ["buildbox", "mcpx", "serve", "--transport", "stdio"] } } }
```

That runs `mcpx serve` on the remote host, so the daemon there is the one
answering — again with no port and no credential beyond ssh.

## What a remote endpoint is not

- It is not a second authentication layer. **The daemon API is
  unauthenticated**; who may reach it is who may reach the host and the
  socket. `ssh://` inherits exactly the ssh access you already grant.
- It is not a shared filesystem. Scripts and artifacts sent to a remote
  daemon travel as bytes; a caller's `{path}` is uploaded, not made visible to
  the daemon, because an ssh tunnel never proves shared-disk access. See
  [`exec.md`](./exec.md#the-store).
- It is never started from here. Local autostart does not apply, by design:
  answering from a local daemon when the remote one is unreachable would be
  worse than failing.
