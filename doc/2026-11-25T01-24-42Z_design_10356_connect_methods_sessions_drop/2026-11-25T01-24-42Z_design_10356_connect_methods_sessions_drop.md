# Laptop connect: method selection, sessions, stdin and drop semantics

Decision record for [#10356](https://github.com/developing-today/code/issues/10356)
(epic #5412). This covers the issue's **design phase**: items 1–4,
the measured numbers in one table, the build split, and the
reconciliation with #5972. Items 5 and 6 stay recorded horizon.
This record changes no code.

Status: **decision proposal for the design phase.**

The record builds on what has landed since the issue was filed on
2026-11-14. It does not re-decide any of these:

- **#10577.** Cloudflare is the primary path and Tailscale the
  fallback at the proxy layer (`fleet-proxy/src/tunnel.rs`).
- **#10552.** The durable `sandbox-link` tunnel connector.
- **#10796.** The fleet binary's `laptop-agent` domain. It has the
  ladder broker socket → Cloudflare → Tailscale CONNECT → SSH, and
  `exec / run / poll / jobs / kill / fetch`.
- **#5759.** `fleet proxy get|put`: hashed, resumable 2 MiB chunked
  transfer over the agent channel.
- **#13717** (`doc/2026-11-23T21-18-54Z_design_laptop_route_forms/`).
  The route-form matrix T0–T7:
  - `fleet laptop exec` (T0) walks T1 commander → T2 http → T3 ssh
    and names the tier that served the call.
  - Each method is its own literal command.
  - The selection doctrine says "never a blended invocation" and
    "every result names its form".
- **#13774** (open). Live exec sessions inside Actions runners. This
  is a sibling with a different host class, not this record's
  scope.

## 1. Measured numbers, one table

All figures were recorded in #10356 on 2026-11-14 and 2026-11-15,
under normal fleet load. Each row gives its source date. Nothing
here was re-measured for this record.

| Path                               | Leg                                      | Warm round trip    | Cold             | Throughput (down / up)                                   | Notes                                               |
| ---------------------------------- | ---------------------------------------- | ------------------ | ---------------- | -------------------------------------------------------- | --------------------------------------------------- |
| Agent exec (T2)                    | Tailscale CONNECT via DERP NYC           | 0.39–0.45 s        | ~1.8 s (CONNECT) | n/m                                                      | 11-14; every byte hairpins DERP                     |
| Agent exec/health (T2)             | Cloudflare tunnel                        | 0.26–0.43 s health | n/m              | 0.80 / 0.65 MB/s (16 MiB via real exec calls, sha256 ok) | 11-15; per-call exec overhead bounds this           |
| Agent health (T2)                  | Tailscale, same day                      | 0.43–1.21 s        | n/m              | n/m                                                      | 11-15; auto mode lands on `cf` every trip           |
| Raw tunnel                         | Cloudflare `sandbox-link`                | n/a                | n/a              | 2.50 and 2.27 / 1.22 (96 MiB) and 1.06 (2×64 MiB) MB/s   | 11-15; one request over 100 MB gets 413 at the edge |
| Real SSH (T3)                      | Tailscale DERP (SSH has not moved to CF) | ControlMaster warm | n/m              | 0.66 (pull) / 0.37 (push) MB/s                           | 11-14; 128 MiB, content verified                    |
| Laptop hairpin via quick tunnel    | Cloudflare, laptop link crossed twice    | n/a                | n/a              | 1.55 MB/s                                                | 11-15; shows the tunnel side is not the cap         |
| R2 meeting point                   | sandbox → R2                             | n/a                | n/a              | 2.50 MB/s loaded; 10–14 MB/s low load                    | 11-14 and 11-15; sandbox load dominates             |
| Commander (T1)                     | `commander.security.cab` MCP             | n/m                | n/m              | n/m                                                      | never measured                                      |
| Batch behaviour (many short calls) | any                                      | n/m                | n/m              | n/m                                                      | never measured                                      |

`n/m` means not measured. The two unmeasured rows are the first
acceptance step of the build issues (§6). They must not be filled in
from intuition.

What the numbers decide:

1. HTTP over Cloudflare is now both the fastest and the
   lowest-latency control path.
2. SSH is the slowest bulk path, because it still rides DERP.
3. Bulk bytes belong on `fleet proxy put|get` or R2, not on stdin or
   stdout of a command.

## 2. Item 1: method selection. No `fleet connect --method`

**Decision.** #13717's matrix answers the question "can I flag and
choose the SSH method versus the other?". Each method is its own
subcommand:

- `fleet commander exec`
- `fleet http exec` (plus `run / poll / jobs / kill / fetch`)
- `fleet ssh exec`

`fleet laptop exec` is the `auto` form. A `fleet connect --method
exec|ssh|job|auto` wrapper is **not built**. It would be the blended
invocation the #13717 doctrine forbids. It would also hide the literal
form that served a call, which is exactly what the route-form
evidence depends on. `fleet proxy connect` keeps its existing meaning,
the session-proxy CONNECT client. A second `connect` verb with a
different meaning would collide with it.

The issue's `--method` values map onto existing forms:

| Asked for | Answer                                                  |
| --------- | ------------------------------------------------------- |
| `exec`    | `fleet http exec` (T2), or `fleet commander exec` (T1)  |
| `ssh`     | `fleet ssh exec` (T3); interactive use is §4            |
| `job`     | `fleet http run` then `poll`; the session surface is §3 |
| `auto`    | `fleet laptop exec` (T0)                                |

**One residual fix.** The `laptop-agent` domain (#10796) ladders into
an **SSH leg** that imitates the daemon's job API over
`laptop-ssh-real.sh` with `sshfb-` jobs. Under the matrix that is a
method switch hidden inside a leg ladder. `fleet http` builds its
ladder HTTP-only (`legs::http_ladder`), so T2 can never silently
become T3. `fleet laptop-agent` can.

Decision: `fleet laptop-agent` stays for parity with the Python
client. When its SSH leg serves a call, it must announce
`form T3 ssh` on stderr, as the other forms announce themselves. Its
docs then point new callers at `fleet http`, which has the same verbs,
and at T0. This is build issue B1.

## 3. Item 2: the session model is the daemon job, extended. No new store

A "connect session" is a job in the laptop-agent daemon's existing
spool (`~/.local/state/laptop-agent/jobs/`). The SSH-fallback leg's
`sshfb-` jobs sit in a second namespace. The design makes jobs fit to
be sessions. It does not add another session store.

### 3.1 Listing

`fleet http sessions` is the `jobs` verb with metadata added:

- job id;
- owning context id, when the caller exports one (the
  context-propagation design's id; absent until that lands, never
  guessed);
- argv;
- started at;
- state (`running | grace | done | reaped`);
- stdout and stderr bytes spooled;
- bytes dropped (§3.3).

It lists daemon jobs and `sshfb-` jobs together, labelled by origin.
The local offsets in `~/.cache/laptop-agent/` show which jobs this
client has drained.

### 3.2 Attach

`fleet http attach <id>` is `poll` with two corrections:

- It drains until **both** `state == done` and the reply reports no
  more spooled bytes. Today's `poll` returns at the first reply that
  says `done`. Because the daemon returns at most `FILE_CHUNK_MAX`
  (1 MiB) per stream per reply, a finished job with more than 1 MiB of
  output is cut off. This is the quirk observed on 2026-11-15. The
  daemon reply must carry `out_size` and `err_size`, or `more: true`.
  The client loops while `out_next < out_size`.
- It is idempotent. It replays from the stored offset, so the ladder
  may retry it on any leg.

### 3.3 Buffering limits

| Path                         | Buffer                                                                                       | At the cap                                                                                                                                                                                                                                      |
| ---------------------------- | -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Daemon job and exec-in-grace | Spool on the laptop's disk. Proposed default cap is 256 MiB per stream, set by daemon config | The process keeps running. The spool stops appending and records `bytes_dropped`. `attach` prints an explicit `[spool cap reached: N bytes dropped]` marker. It never silently truncates, and it never kills a working job because its log grew |
| `sshfb-` job                 | Same rule, same cap                                                                          | Same                                                                                                                                                                                                                                            |
| Synchronous exec (T2)        | Daemon capture of 1 MiB per stream (`FILE_CHUNK_MAX`)                                        | Unchanged. Use `run` and `attach` for large output                                                                                                                                                                                              |
| Commander (T1)               | None                                                                                         | The process is polled to completion or killed at the deadline (#13717). No session                                                                                                                                                              |
| SSH (T3)                     | None. sshd owns the process; output printed during a drop is lost                            | Use tmux (§5) when a session must survive a drop                                                                                                                                                                                                |

### 3.4 Drop semantics for exec

The exec path gets a bounded grace window. Abandoned runs still never
orphan.

Today a client disconnect during `/exec` reaps the command at once
(own process group, TERM then KILL, #5343, 2026-11-10). The proposed
behaviour:

- **The client names the job.** `/exec` already carries a client
  trip id. The daemon adopts it as the job id and spools exec output
  to the job spool from the start. No extra round trip is needed, and
  the client knows the id before any drop.
- **A drop starts a grace timer.** On disconnect the daemon moves the
  command to `grace` for G seconds. The proposed default is G = 60,
  per request via `exec --grace S` with `0..=600`. `--grace 0`
  restores today's immediate reap exactly.
- **Inside G.** `fleet http attach <trip-id>` converts the command
  into an ordinary job and cancels the reap. The client's
  drop-and-timeout message prints that exact command.
- **After G.** TERM then KILL as today. The job record stays as
  `reaped` with its spool, so the output up to the reap can still be
  read.
- **No replay.** The ladder still never replays a non-idempotent exec
  on another leg (`ladder.rs`). Recovery is attach, which is
  idempotent.

## 4. Item 3: stdin. Interactive stays on SSH, data is staged

**Decision: the daemon does not get a stdin channel.**

- **Interactive input or a PTY ⇒ SSH (T3), permanently.** This is now
  the documented contract, not an accident of implementation. The
  daemon is a request/response HTTP API through an edge that caps one
  request at 100 MB and adds per-call overhead (§1). Live interactive
  stdin through it would need a streaming upgrade the edge path has
  not been shown to carry. SSH already gives real PTY, stdin and
  forwards.
- **Finite input data ⇒ stage, then run.** Upload with `fleet proxy
put <local|-> <laptop-path>`, which is hashed, resumable and
  atomic, and accepts `-` for a pipe. Then run `fleet http exec --
sh -c 'cmd < path'`. This is faster than SSH stdin push (Cloudflare
  up 0.65–1.22 MB/s versus SSH push 0.37 MB/s). It survives a drop
  through put's resume. It needs no daemon protocol change.
- **No automatic composition.** T0 does not stage and run for the
  caller. Composing two forms invisibly would break form visibility.
  The `laptop-ssh` shim's routing ("stdin-carrying ⇒ real SSH") stays
  as it is for existing callers.

**Evidence that would reopen this decision:** a measured interactive
need that SSH cannot serve, or SSH moving off DERP with no gain. Until
one of those exists, the simplest contract stands.

## 5. Item 4: tmux, settled in writing

- **The SSH path's attach and detach answer is tmux on the laptop.**
  It is not an HTTP attach method.
- **It is documented and lightly tooled.**
  - Fleet-started tmux sessions are named
    `fleet-<context-id or trip-id>`.
  - `fleet ssh sessions` lists only `fleet-*` sessions, read-only
    through `fleet ssh exec -- tmux ls -F …`.
  - `fleet ssh sweep --idle-for DURATION` kills only `fleet-*`
    sessions that have no attached client and have been idle past the
    bound.
  - Neither command ever touches a session without the `fleet-`
    prefix. A person's own tmux sessions are out of reach by
    construction.
- **Cleanup duty.** Anything a fleet caller starts under tmux uses
  the `fleet-` name, so the sweep can find it. A caller that starts an
  unnamed session owns its cleanup.
- **On the sandbox.** Muse's own exec sessions are the equivalent. No
  second session system is built there.

## 6. Build split (to file after this record merges)

- **B1.** `fleet laptop-agent` SSH leg announces `form T3 ssh`. The
  docs point new callers at `fleet http` and T0 (§2).
- **B2.** The session surface:
  - `fleet http sessions` and `attach`, with the drain-until-empty
    fix;
  - daemon `out_size`/`err_size` in replies;
  - spool cap and `bytes_dropped`;
  - exec grace window with the trip id as job id (§3).
  - The daemon half is owned wherever `laptop-agent.py` lives. This
    repo holds the client half and the fleet-side tests.
- **B3.** The stdin contract:
  - document "interactive ⇒ T3, data ⇒ put + exec";
  - make `fleet http exec` refuse a non-tty stdin with a usage error
    that names `fleet proxy put`, instead of silently ignoring it
    (§4).
- **B4.** `fleet ssh sessions` and `sweep` with the `fleet-` prefix
  rule (§5).
- **B5.** Measure the two `n/m` rows in §1: T1 latency and batch
  behaviour across T1, T2 and T3. Do this with remote dispatch only,
  as one dispatched job that calls the laptop. Do not compile on a
  laptop or sandbox.

Each build issue starts with a named failing test and runs its tests
remotely (`fleet queue dispatch … --dispatch` or the
`fleet-command.yml` workflow).

## 7. #5972 reconciled: one answer

#5972 asked for a built-in fallback when the proxy path is
unavailable. It is closed, and there is now one answer with two
levels:

1. **Method fallback happens only at T0.** `fleet laptop exec` walks
   commander, http and ssh, and names the serving tier.
2. **Leg fallback happens inside a method.** T2 uses broker, then
   Cloudflare, then Tailscale. T3 uses its own proxy CONNECT.

The one place that mixed the two levels is the `laptop-agent` SSH leg.
B1 makes it visible, so there are no longer two answers that disagree
silently.

## 8. Recorded horizon (unchanged; not designed here)

- Item 5, inbound-less hosts by reverse connection. #13774's
  runner-side sessions are the nearest live instance. Spin it out if
  it becomes real.
- Item 6, direct node-to-node SSH with per-node certificates under a
  shared CA, with the proxy pool as the compatibility floor.
- #13717's horizon: host selection without Tailscale, direct iroh,
  and generic pooling.
