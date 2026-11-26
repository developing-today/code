# fleet laptop route forms

The four Fleet domains that reach the laptop, codified as one
matrix by #13717: **tier 0** (`fleet laptop`), the generic tiered
command; **T1** (`fleet commander`), the Desktop Commander MCP
endpoint; **T2** (`fleet http`), the laptop-agent daemon's HTTP
API; **T3** (`fleet ssh`), real SSH. Outside the binary, the same
methods exist as raw commands (T4–T6, literal recipes in the
laptop mechanics doc) and the `laptop-ssh` script is T7 — the
fallback / simplified tier.

Why forms at all: the platform's execution-risk behaviour keys on
the literal command shape an agent runs, so each form stays a
first-class, separately invocable command, and every result names
the form that served it. Circumstance decides the form; there is
no single blessed invocation.

## fleet laptop — tier T0

`fleet laptop tiers` prints the matrix. `fleet laptop exec
[--form T1|T2|T3] [--timeout SECS] [--cwd DIR] -- CMD...` walks
T1 → T2 → T3 in order (or attempts one pinned tier), each attempt
a literal invocation of that tier's own implementation. It
advances only when a tier fails to _serve_ — transport, auth or
connect failure, or a tier-level timeout — never on a served
result, whatever its exit code, and it reports the serving tier
on stderr (`fleet: served by form T1 commander/fleet`). The
per-attempt `--timeout` always applies (default 120 s). Tier 0
deliberately does not fall through to the `laptop-ssh` wrapper:
reaching T7 is the caller's own final step.

One recorded casualty of the walk rule: ssh's own failure exit is
255, so the walker reads a remote command that itself exits 255
over ssh as a tier failure. Standalone `fleet ssh` propagates 255
untouched.

## fleet commander — tier T1

A client for the self-hosted Desktop Commander MCP endpoint
(#13404, #13654). The bearer is resolved from the secrets broker
by name (`desktop-commander.machine-bearer`) at call time and
never stored; requests carry a browser user agent (the edge
1010-blocks non-browser agents). One MCP session per invocation.
`exec` starts the command via `start_process` (the schema's
`timeout_ms` is always sent), polls `read_process_output` to
completion, and propagates the remote exit code from the
endpoint's own completion line; on the deadline (default 120 s)
it force-terminates the process and exits 124 — an exec never
leaves a running process behind on the laptop. `health`
initializes a session, runs `hostname`, and reports
`{"ok":true,"form":"T1 commander/fleet",...}`.

## fleet http — tier T2

The laptop-agent daemon's HTTP API as a route-form surface: the
same verbs as [fleet laptop-agent](laptop-agent.md), over a
ladder built from the client's HTTP legs only (`http_ladder`:
broker socket → Cloudflare tunnel → Tailscale CONNECT). The SSH
leg is excluded by construction, so this tier can never silently
become the ssh tier. The primary-checkout guard is applied before
anything is sent; a guard refusal is a served result (exit 1),
never something tier 0 may route around. Timeouts always apply:
the daemon's `timeout_s` (default 300) plus the transport grace
as the local deadline; a daemon-side timeout exits 124.

The daemon has no stdin channel (the stdin contract of
developing-today/code#10356, section 4). `fleet http exec`
therefore refuses a stdin that carries data, such as a pipe or a
non-empty redirected file, with a usage error (exit 2) before
anything is dialed or sent, instead of silently dropping the
bytes. A terminal, `/dev/null`, an empty file or an empty pipe is
fine. To hand a command finite data, stage it first and read it
from the laptop path:

```
fleet proxy put ./input.json /tmp/input.json
fleet http exec -- sh -c 'jq . < /tmp/input.json'
```

For interactive input, use `fleet ssh exec` (tier T3).

## fleet ssh — tier T3

SSH's endpoint-grade shape is a binary-owned client contract: the
binary constructs the ssh invocation itself — pinned host key
(`~/.ssh/known_hosts_laptop`, `StrictHostKeyChecking=yes`),
proxy CONNECT helper, `IdentitiesOnly`, `ConnectTimeout` —
instead of delegating to a shell script, and enforces a mandatory
overall deadline (default 300 s; the ssh child is killed on
expiry, exit 124). Connections multiplex over a fleet-owned
ControlMaster socket (`~/.ssh/cm-fleet-…`, persist 600 s,
separate from laptop-ssh's master); `fleet ssh close` tears it
down. The honest limit: sshd owns the remote process, so a
client-side kill ends the session but cannot cancel the remote
command protocol-side.
