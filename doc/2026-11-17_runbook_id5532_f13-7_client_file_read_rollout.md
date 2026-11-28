# F13.7 rollout runbook — `client.file.read`, the first client-side capability provider

Issue: [#5532](https://github.com/developing-today/code/issues/5532) ·
Design: `doc/2026-11-01_design_id789.md` · Final implementation step:
step 6 (end-to-end hardening + this runbook). Step 5 (the provider
itself) landed in PR #10925.

`client.file.read` lets a world program ask a _joined shell_ to read
one file under a root directory the local user chose, and return the
bytes as the ordinary journaled capability result. It is the first
provider with `side: client` in the provider registry: the host
never executes it, and no other world, session, or shell is affected
by one shell opting in.

**Default posture: off everywhere.** A read can only happen when
BOTH gates below are open at the same time. If either is closed, the
request is refused and the refusal is audited. Worlds and shells
that never opt in see no behavior change: the provider is absent
from the legacy host `CATALOG` view and from every default ceiling.

## The two gates

| Gate               | Who controls it                                                                                                 | What it decides                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------------ | --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host gate          | Host operator (server ceiling, at `serve` time) and world operator (world ceiling + grant, via `id world caps`) | The provider is named in the **server ceiling**, in the **world ceiling**, AND **granted** to the world — ceilings compose by intersection, so a world may allow less than the server, never more. The host broker checks, in order: ceiling → actor scope → explicit grant → session attribution → argument shape → the _originating session's_ offer. Grant-on-use never applies to a client provider. |
| Local gate (shell) | The local user at the shell                                                                                     | The shell has opted the provider in for that world with the disclosure acknowledged. The shell's adapter re-checks the grant on every request; a host "allow" cannot bypass it, and revoking locally takes effect on the next request.                                                                                                                                                                   |

A request is routed only to the session whose input caused it and
which offered the provider. Another session's offer never makes the
provider available to a request (no cross-session fallback), and a
request that cannot be attributed to a joined session is refused
`actor_denied` / `unattributed`. Requests already listed when a
world reopens are refused `actor_denied` / `restarted`, never
re-executed — replaying a journaled result never re-runs local I/O.

## Enabling a world (operator)

Two host-side steps, in order. Skipping the first is the most
common setup mistake: the provider's manifest default is
`Excluded`, so it is **not** in the default server ceiling, and no
world setting can reach it until the server allows it.

**1. Allow the provider on the server** (restart the host):

```sh
id serve … --world-allowed-caps time.now,chat.say,client.file.read,…
```

- `--world-allowed-caps` **replaces** the server ceiling (the
  default is the default ceiling: the whole catalog minus the
  sensor caps and the `Excluded` providers). List every capability
  any world on this host must be able to use, not just the new
  one. Only known capability names are accepted; `*` is refused.
- This is runtime configuration, not journaled state: it bounds
  every world on the host and takes effect at startup.

**2. Open the world gate** (live, no restart):

```sh
id world caps <node-id> \
  --admin-token <token> \
  --world <world-name> \
  --ceiling chat.say,client.file.read \
  --grant client.file.read
```

- `--ceiling` **replaces** the world's ceiling. List every
  capability that must remain allowed, not just the new one. Only
  known capability names are accepted; there is no `*` ceiling.
- `--grant` is repeatable. Both flags together open the world gate;
  either one alone changes nothing a guest can reach.
- The change is live: the next request is decided against the new
  ceiling and grants.

## Enabling a shell (local user)

```sh
ID_SHELL_FILE_READ_ROOT=/path/to/shared-dir id-native-shell …
```

Setting the variable to an existing directory is the shell's entire
opt-in for the session's world, under the design's default local
bounds (32 KiB results, 10 reads/minute, 2 in flight). On startup
the shell prints one line to stderr confirming the provider is
enabled; the line deliberately never prints the root path:

```text
id-native-shell: client.file.read enabled for world <name> under ID_SHELL_FILE_READ_ROOT; returned bytes become journaled world data
```

Unset (the default), nothing is registered, offered, or executed:
the shell sends no `provider_offer`, and the host broker reports
the provider unavailable for that session.

## Disclosure semantics (read this before opting in)

Bytes this provider returns **become journaled world data**. The
host journals the result as the ordinary capability `System` event;
the world operator, the host, and anyone with journal access can
read the file's contents back later, and world replay replays the
journaled bytes — it never re-reads your disk.

That is why the acknowledgment is mandatory, not a formality:

- Setting `ID_SHELL_FILE_READ_ROOT` _is_ the acknowledgment in the
  shipped shell.
- An executor built without the acknowledgment refuses every read
  with `content_disclosure_not_acknowledged`, and the adapter's
  combined opt-in refuses to grant at all. There is no route that
  serves reads whose disclosure was not acknowledged.

Only share a root whose contents you are willing to see copied into
the world's journal.

## Bounds and path rules

Bounds compose by minimum across manifest, world, and local grant;
an over-bound result is refused whole, never silently truncated.

- Arguments: at most 8 KiB serialized; exactly one required string
  argument, `path`.
- Result: at most 32 KiB. A larger file refuses `result_too_large`.
- Rate: 10 executions/minute and 2 in flight locally; the host
  broker also caps the provider at 2 in flight with a 30 s reply
  deadline (`provider_timeout` after it).
- Paths must be relative, contain no parent traversal, and resolve
  (after symlinks) to a regular file **beneath** the root. Absolute
  paths, `..` traversal, and symlink escapes refuse
  `file_read_invalid_path`; a missing file refuses
  `file_read_not_found`; a directory refuses `file_read_not_a_file`.

## Revoking

- **World side:** `id world caps <node-id> --admin-token <token>
--revoke client.file.read` — the next request refuses
  `cap_denied` (host audit reason `not_granted`). Removing the name
  from the ceiling instead refuses `outside_ceiling`.
- **Shell side:** unset `ID_SHELL_FILE_READ_ROOT` and restart the
  shell — the offer is withdrawn and the provider is unavailable to
  that session. A shell that revokes in-process refuses routed
  requests with `provider_revoked` from the next request on.

## Verification (operator checklist)

1. Start the shell with the root set; confirm the stderr enablement
   line appears and does **not** contain the root path.
2. Confirm the host side: the host was started with
   `client.file.read` in `--world-allowed-caps`, and
   `id world caps <node-id> --admin-token <token>` shows
   `client.file.read` in the world's ceiling and grants (a read
   refused `outside_ceiling` despite a world grant almost always
   means the server ceiling step was skipped).
3. Have a participant input trigger a read of a known small file
   under the root. The guest program receives a result event with
   `ok: true` and the file's bytes as `result_hex`.
4. Check the shell's stderr: one local audit line per request —
   world, provider, nonce, outcome, reason — e.g.
   `… outcome=executed reason=-`. It never prints paths or content.
5. Check the host audit trail (F12.6): `client_provider_use`
   records show `allowed` then `completed` for the read, with
   provider, version, actor, and an `args_digest` (BLAKE3 of the
   canonical args). Argument values, local paths, and file content
   never appear in either audit trail.
6. Negative checks: ask for `../something` (refused
   `file_read_invalid_path`), a file over 32 KiB
   (`result_too_large`), and — after `--revoke client.file.read` —
   the original file (`cap_denied`). Each refusal appears in both
   audit trails with its stable code.

## What a guest sees on refusal

Result events carry a stable `error` and, where one applies, a
stable `reason`. The codes a program may observe:

| error              | reason                                                                                         | Meaning                                                                                                                                                                  |
| ------------------ | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `outside_ceiling`  | —                                                                                              | The world ceiling does not name the provider.                                                                                                                            |
| `cap_denied`       | —                                                                                              | No explicit world grant (host audit records `not_granted`).                                                                                                              |
| `actor_denied`     | `unattributed`                                                                                 | No originating joined session; never routed.                                                                                                                             |
| `actor_denied`     | `restarted`                                                                                    | The request predates a world reopen; never re-executed.                                                                                                                  |
| `failed`           | `provider_unavailable`                                                                         | The originating session did not offer the provider (no local opt-in).                                                                                                    |
| `failed`           | `provider_busy` / `provider_timeout`                                                           | In-flight cap reached / no reply before the deadline.                                                                                                                    |
| `cap_denied`       | `content_disclosure_not_acknowledged`                                                          | Local disclosure not acknowledged; nothing was read.                                                                                                                     |
| `cap_denied`       | `file_read_invalid_path` / `file_read_not_found` / `file_read_not_a_file` / `file_read_io`     | The local path rules refused the read.                                                                                                                                   |
| `cap_denied`       | `provider_not_granted` / `provider_revoked` / `provider_unavailable` / `provider_rate_limited` | The shell's local grant state refused the request.                                                                                                                       |
| `cap_denied`       | `provider_refused`                                                                             | The client replied with a refusal code outside the stable shape; the host substitutes this generic code rather than journaling arbitrary client text (step-6 hardening). |
| `result_too_large` | —                                                                                              | The reply exceeded the 32 KiB bound at the host.                                                                                                                         |
| `bad_args`         | e.g. `missing_required_arg`                                                                    | Arguments fail the manifest schema or the 8 KiB bound.                                                                                                                   |

## Step-6 verification coverage

The assembled path — registry, broker, session routing, adapter,
executor — is exercised end to end against a live host in
`crates/id-native-shell/tests/provider_file_read_e2e.rs` (happy
path, the refusal matrix, both world gates, cross-session offer
isolation, the disclosure gate, and revocation at either gate),
with the two hardening fixes from that pass covered in
`id-worlds`: client refusal codes are sanitized at the broker
before they can reach the journal, and restarted refusals of
client providers are recorded in the host audit trail.
