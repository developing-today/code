# Laptop route forms — the codified matrix (T0–T7)

Design record for developing-today/code#13717 (companion: #13654).
Direction: Drew, 2026-11-23 14:56 + 14:59 + 15:07 CST.

## Why a matrix, not a ladder

The platform execution-risk behaviour observed on 2026-11-23 keys on
the **literal command shape** an agent runs: the same laptop
operation returned via one route comes back clean, via another
trips the check. A single opaque command that papers over every
route would hide which form actually works for a given operation,
and would remove the raw forms an agent sometimes needs when the
wrapper itself is the obstacle. So the raw commands are first-class
tiers, not implementation details — and tier 0, the one generic
command, still reports exactly which tier served the call.

## The codified matrix

| Tier | Method      | Shape            | Literal command                                                                                       |
| ---- | ----------- | ---------------- | ----------------------------------------------------------------------------------------------------- |
| T0   | tiered      | fleet subcommand | `fleet laptop exec [--timeout S] -- cmd...` — walks T1 → T2 → T3 internally, reports the serving tier |
| T1   | commander   | fleet subcommand | `fleet commander exec [--cwd D] --timeout S -- cmd...`                                                |
| T2   | http-direct | fleet subcommand | `fleet http exec [--cwd D] --timeout S -- cmd...`                                                     |
| T3   | ssh         | fleet subcommand | `fleet ssh exec [--cwd D] --timeout S -- cmd...`                                                      |
| T4   | commander   | raw command      | the documented curl recipe against `https://commander.security.cab/mcp`                               |
| T5   | http-direct | raw command      | literal curl to the daemon API through the session proxy                                              |
| T6   | ssh         | raw command      | the literal `ssh` invocation written out in `TOOLS.d/laptop.md`                                       |
| T7   | wrapper     | simplified tier  | `laptop-ssh 'cmd...'` — final fallback, and the one-command form for an agent without the full matrix |

**Composition.** Tier 0 is the first option. It attempts T1 → T2 →
T3 in order, each attempt a literal invocation of that tier's own
implementation, advancing only when a tier fails to _serve_
(transport / auth / connect failure or a tier-level timeout) —
never on a served result, whatever its exit code — and it names
the serving tier. Tier 0 does **not** fall through to the wrapper:
when tier 0 fails, the individual forms come into play as their
own literal commands — the fleet subcommands (T1–T3) again,
because an attempt under tier 0's process shape is a different
literal invocation than the standalone subcommand — then the raw
commands (T4–T6), then the wrapper (T7).

One recorded casualty of the walk rule: ssh's own failure exit is
255, so tier 0 reads a remote command that itself exits 255 over
ssh as a tier failure. Standalone `fleet ssh` propagates 255
untouched; only the walker applies the convention.

## Endpoint-grade contracts (the commander pattern, all three methods)

- **Commander (T1)** — `crates/fleet/src/commander.rs`. Bearer
  resolved in-process from the secrets broker by name
  (`desktop-commander.machine-bearer`), never stored; browser UA
  (the edge 1010-blocks non-browser agents, #13404); one MCP
  session per invocation; `timeout_ms` always sent; spawned
  processes polled to completion or force-terminated on the
  deadline — no orphaned processes on the laptop; the remote exit
  code is read from the endpoint's own completion line.
- **HTTP (T2)** — `crates/fleet/src/http.rs` over the
  fleet-laptop-agent client. The daemon is the endpoint; this is
  its contract surface: token auth from the token file, the
  primary-checkout guard applied before anything is sent (a guard
  refusal is a _served_ result — tier 0 must not route around it),
  daemon `timeout_s` plus the transport grace as the local
  deadline, exit codes from the daemon reply. The ladder is built
  HTTP-only (`legs::http_ladder`: broker socket → Cloudflare
  tunnel → Tailscale CONNECT) — the SSH leg is excluded by
  construction, so T2 can never silently become T3.
- **SSH (T3)** — `crates/fleet/src/ssh.rs`. The endpoint-grade
  shape for SSH is a **binary-owned client contract**: the binary
  constructs the ssh invocation itself (pinned host key in
  `~/.ssh/known_hosts_laptop`, `StrictHostKeyChecking=yes`, proxy
  CONNECT helper, `IdentitiesOnly`, `ConnectTimeout`) instead of
  delegating to a shell script; a mandatory overall deadline
  enforced by the binary (child kill → exit 124). Honest limit:
  sshd owns the remote process, so a client-side kill ends the
  session but cannot cancel the remote command protocol-side.
  Session hygiene: a fleet-owned ControlMaster path
  (`~/.ssh/cm-fleet-…`, persist 600 s — separate from laptop-ssh's
  master) and `fleet ssh close` to tear it down. A native
  in-process SSH protocol implementation is the recorded horizon,
  not this build.

Timeouts are required by construction on every fleet-shape form:
there is no invocation path without one (T0 sets a per-attempt
timeout, default 120 s; T1 default 120 s; T2 and T3 default 300 s).

## Selection doctrine

- Circumstance decides which form is the right way through. There
  is no single blessed invocation.
- An agent tries each form as **its own literal command** — never
  a blended invocation.
- **Form visibility**: every result and log line names the exact
  form used, as method × shape (`form T2 http/fleet`,
  `form T4 commander/raw`). The fleet forms announce themselves on
  stderr; tier 0 narrates each attempt and the serving tier.
- Raw forms must exist, stay documented, and stay exercised —
  they are the route around a wrapper that has itself become the
  obstacle. Their literal recipes live in `TOOLS.d/laptop.md`.

## The demotion of `laptop-ssh` (T7)

Documentation-and-defaults, not removal. `laptop-ssh` keeps working
for every existing caller — it is load-bearing in the heal/init
stack (the channel-heal and init-ensure scripts delegate through
it), in crons, and in lane habits. What changes: the mechanics doc
and the shim's own header teach the matrix and the tier-0-first
order, and name `laptop-ssh` as tier T7 — the fallback when the
fleet tiers fail, and the simplified tier for an agent that
doesn't carry the full list: one command that does something sane.

## Recorded horizon (Drew, 2026-11-23)

Longer-term design threads named alongside this codification, not
part of this build: host selection and access without Tailscale; a
direct iroh transport for non-Muse agents; a generic pooling
system.

## Verification (2026-11-23)

Built in the `pkgs/fleet` workspace (features `commander`, `http`,
`ssh`, `laptop`, all in `full`); `cargo test` green across
fleet-registry, fleet-laptop-agent and the fleet binary (including
the DOC-01 inventory test, which now lists the four new domains).
Smoke from the sandbox with the built binary: every fleet form ran
`hostname` → `amd`, read `/etc/hostname`, and propagated a remote
`exit 3` as exit 3, naming its form; the raw forms ran as written
in `TOOLS.d/laptop.md`. The seat-wide install rides the next
fleet-static publish; nothing was installed over the seat binary.
