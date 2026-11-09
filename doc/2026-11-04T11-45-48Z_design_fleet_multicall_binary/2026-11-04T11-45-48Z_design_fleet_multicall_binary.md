# One `fleet` multicall binary: search, queue, dispatch, proxy

Design record, 2026-11-04. Step 0 of the epic
developing-today/code#5412 — Drew's 2026-11-04 ruling to
consolidate the fleet's tools into ONE multicall binary, `fleet`,
as the single distribution unit, including the proxy (serve and
connect) as domains of the binary itself, with compile-time
capability-selected build variants. This note is the design
only — no implementation. It ends with the go/no-go decision
list (§8) Drew rules on before any implementation step is filed.

Facts are from a census taken 2026-11-04 against the live
systems (binary sizes, source trees, protocol docstrings, the
open packaging PR), not from memory. Where the census
contradicts the epic's framing, §1 says so plainly. Two census
limits, stated once: the laptop was unreachable over SSH for
part of the census window (the tailnet tunnel was dropping
CONNECTs), so laptop _runtime_ state is quoted from the unit
files and from states verified earlier the same evening; every
repo-side and sandbox-side fact below was read directly.
Measurements quoted are the ones the live artifacts carry;
proposals are labelled proposals.

## The current shape (census)

The fleet's tooling is not a set of programs with a distribution
problem. It is a set of programs with **three different source
homes, two languages, and one program family that lives in no
published repo at all**:

| Component                      | Source home today                                                                  | Shape                                                                                                   | Launched / distributed today                                                                                                                              |
| ------------------------------ | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repo-queue` CLI               | sandbox `~/tools/fleet-brokers/crates/repo-queue` (1,155 LOC)                      | Rust, 956,800 B unstripped dynamic ELF                                                                  | built in `~/tools/fleet-brokers/target`, hardlinked to `~/tools/repo-queue`, symlinked from `~/tools/bin/repo-queue`                                      |
| `queue-broker`                 | same workspace (610 + 467 LOC)                                                     | Rust (tiny_http/ureq), 4,080,672 B                                                                      | laptop systemd **user** unit, `ExecStart=~/fleet-brokers/bin/queue-broker`, binds tailnet IP `100.99.113.103:4801`                                        |
| `secrets-broker`               | same workspace (276 LOC)                                                           | Rust, 1,255,464 B                                                                                       | laptop systemd user unit, binds `100.99.113.103:4800`; resident Bitwarden cache, TTL 900 s                                                                |
| `laptop-agent` daemon          | Python stdlib; vendored copy in dezren39/nix PR #1459 `pkgs/laptop-agent/src/`     | Python                                                                                                  | laptop systemd user unit, binds `100.99.113.103:4799`; client is sandbox `~/tools/laptop-agent.py` (223 lines)                                            |
| Sandbox session proxy          | sandbox `~/tools/` — inside the sandbox home tree, **not a published source repo** | Python trio: `laptop-agent-broker.py` (168 lines), `laptop-proxy-connect.py` (88), `laptop-ssh.sh`      | auto-started by clients when its Unix socket is missing; exits after 1 h idle                                                                             |
| pm-stack incl. **pm-dispatch** | developing-today/code `pm-stack/`                                                  | TypeScript/Bun service suite (aggregator, dispatch, projector, opencode-claimant, …)                    | its own systemd units (`pm-stack/systemd`); state rendered from SOPS into `~/.local/state/pm-dispatch/<project>/`                                         |
| `atlas-search`                 | developing-today/code `pkgs/atlas-search/`                                         | Rust workspace, 4 crates (atlas-core, atlas-embed, atlas-proto, atlas-search)                           | packaging (default.nix, user service on :18760) landing in epic #5350 step 5, in flight at census time; sandbox also carries a `~/tools/bin/atlas-search` |
| `pkgs/id`                      | developing-today/code `pkgs/id/`                                                   | full product workspace (P2P file-sharing CLI on Iroh; crate-split in progress; vendored p2panda crates) | its own `default.nix`; a product with its own users and cadence                                                                                           |

How the pieces connect today, because the proxy design (§4)
inherits it: sandbox clients talk **length-prefixed JSON**
(4-byte length + JSON) to the session broker's Unix socket
(`~/.cache/laptop-agent/broker.sock`). A request is
`{method, path, body, port?, token_file?}`; `port` selects the
laptop service — 4799 laptop-agent (default), 4800
secrets-broker, 4801 queue-broker. The broker holds a pool of
keep-alive HTTP CONNECT tunnels per port (pool 12, keepalive
180 s) through the tailnet egress proxy (:3130) to the laptop's
tailnet IP, and sends the per-service `X-Agent-Token` (read
from the named token file per request, never stored). The cold
cost it exists to hide is the CONNECT handshake itself
(~1.8 s through the DERP hairpin vs ~0.25 s warm). SSH to the
laptop rides the same CONNECT construction via
`laptop-proxy-connect.py` as ProxyCommand.

The queue path end-to-end: `repo-queue` (client) → session
proxy _or_ direct HTTP on the laptop (`REPO_QUEUE_BROKER_URL`
/ `~/.config/repo-queue/broker-url`) → `queue-broker`
(`POST /<command>`, JSON envelope `{ok, …}` / `{ok:false,
exit, error}`) → Postgres RPCs in Supabase. All queue semantics
live in the RPCs; the service-role key lives only in the
broker, fetched from the secrets-broker. The client's surface
is `validate|list|enqueue|claim|lane|bind|heartbeat|
checkpoint|park|block|cancel|fail|complete|health|signal`, and
its exit codes mirror the retired Python: 2 validation,
3 broker-path/config unavailable, 4 HTTP, 5 no row.

### Where the census contradicts the epic

1. **There is no single source tree to consolidate _from_.**
   The broker family's canonical workspace sits in the
   sandbox's `~/tools/fleet-brokers` — files inside the sandbox
   home tree, not a published repo. A second, vendored copy of
   the same sources sits in **dezren39/nix PR #1459, which is
   OPEN, not merged** (it also vendors the Python laptop-agent
   and adds a home-manager module launching all three services;
   no machine enables that module yet — the live units are the
   hand-installed ones with the same names and ports). The
   laptop runs a third copy from `~/fleet-brokers/bin`. Drew's
   standing rules decide the destination — _all services and
   binaries come from the code repo; no laptop services from
   dezren39/nix_ — so the fleet workspace's home is code (§2),
   and PR #1459's fate is a go/no-go decision (D1), not a
   detail.
2. **`repo-queue` has no `--help` and no `--version`** — both
   return `unknown command`. The epic's unified help surface
   and the release mechanics both assume a version surface
   that does not exist yet; it is new work in step 1, and
   tonight's "which build is this?" questions had no command
   to ask.
3. **"Dispatch" is not CLI-shaped today.** It is the pm-stack
   TypeScript service suite plus an LLM steward cron
   (hatch-personal) that gates pm-dispatch launches. Absorbing
   it wholesale is a rewrite of a working stack, not a
   repackaging — hence the scope call in §6.
4. **The laptop-agent daemon is Python.** It is the one fleet
   daemon not in Rust, and everything an agent does on the
   laptop rides its exec path. The epic's "broker roles as
   designed" glosses over a rewrite-or-wrap decision (§6).
5. **The session proxy is three Python programs, not one** —
   broker, CONNECT helper, and the ssh wrapper — all
   sandbox-resident. `fleet proxy` replaces all three, and
   `~/tools` copies are entangled with the sandbox home tree,
   so retiring them is an archive operation, not a delete.

None of this contradicts the epic's _conclusion_ — one
distribution unit is worth more, not less, when the starting
point is three copies in three homes. It changes what step 1
is: giving these programs their first real source home, then
consolidating.

## Decision summary

| #   | Question              | Recommendation                                                                                                                                                                                           |
| --- | --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Workspace home        | `pkgs/fleet/` in developing-today/code, self-contained Cargo workspace (atlas pattern). Close dezren39/nix PR #1459; port its home-module pattern into code's home tree against the code flake. **(D1)** |
| 2   | Variant matrix        | Three shipped variants: `fleet` (full), `fleet-serve`, `fleet-connect` (tiny). Config domain-disable shapes roles inside `full`. **(D2)**                                                                |
| 3   | Stub exit code        | **69** (`EX_UNAVAILABLE`, sysexits.h). **(D3)**                                                                                                                                                          |
| 4   | pkgs/id               | Stays its own binary; not a fleet domain. **(D4)**                                                                                                                                                       |
| 5   | Broker daemons        | `fleet serve <role>` subcommands, one systemd user unit per role.                                                                                                                                        |
| 6   | laptop-agent daemon   | Keep the Python daemon in v1; fleet manages lifecycle/health. Rust rewrite is a named post-v1 candidate. **(D5)**                                                                                        |
| 7   | Dispatch              | v1 is a thin client domain over pm-stack's existing interfaces; pm-stack is not rewritten. **(D6)**                                                                                                      |
| 8   | Proxy protocol        | Keep length-prefixed JSON framing; add a versioned hello/capability frame as the first frame per pooled connection. Broker supports protocol majors N and N−1.                                           |
| 9   | fleet-connect linkage | Static musl, decided by measurement in step 1 (size + runs-anywhere smoke). **(D7)**                                                                                                                     |
| 10  | Migration             | Additive-first: skeleton → queue pilot → proxy → dispatch (thin) → search last (after #5350 step 6). Per-domain rollback = re-point one symlink/unit to the kept standalone artifact.                    |

## 1. Workspace and crate layout

**Home: `pkgs/fleet/` in developing-today/code** — the
pkgs/atlas-search pattern exactly: a self-contained Cargo
workspace with its own `Cargo.toml`, `Cargo.lock`, and
`rust-toolchain.toml`, its own `default.nix`, no workspace
relationship to the repo root. Drew's rule ("all services and
binaries should be in code repo") names the repo; the atlas
precedent names the shape. The dezren39/nix flake keeps Darwin
concerns only; the home-manager module that launches fleet
services is ported from PR #1459's pattern into code's `home/`
tree, consuming code's own flake packages. What has to move:

- `~/tools/fleet-brokers` (sandbox) — the three broker crates,
  source of the live builds. Moves into `pkgs/fleet/crates/`;
  the sandbox tree is then archived in place (it lives inside
  the sandbox home git tree — archive, never bulk-delete).
- dezren39/nix PR #1459's vendored `pkgs/fleet-brokers/src` —
  dies with the PR (D1). Its `home-module.nix` is the one
  artifact worth porting before closing.
- Laptop `~/fleet-brokers/bin` — build output, replaced by
  unit re-points during migration (§7); the directory is
  retired once the last unit flips.

**Crates** (domain crates consume platform crates; the bin
crate consumes domains; nothing consumes the bin):

| Crate               | Contents                                                                                                                                                                                                       |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet-registry`    | The full domain registry: names, one-line descriptions, argv0 aliases, per-domain help pointers. std-only, no I/O. Compiled into **every** variant (§5).                                                       |
| `fleet-config`      | Config file (`~/.config/fleet/fleet.toml`) + env resolution; the domain enable/disable table (§7); state-dir paths.                                                                                            |
| `fleet-secrets`     | Secrets-by-name client: secrets-broker protocol first, `bws` CLI fallback; token-file handling identical to today's (read per request, never stored, never logged).                                            |
| `fleet-proxy-proto` | Proxy framing (length-prefixed JSON) + the hello/capability frame (§4). The fleet analogue of atlas-proto: the wire contract lives here and nowhere else.                                                      |
| `fleet-proxy`       | `proxy serve` (the broker: socket, per-port pools, idle exit) and `proxy connect`/`init` (client, shim materialization, autostart).                                                                            |
| `fleet-queue`       | The repo-queue client surface and the queue-broker + secrets-broker server roles (the three moved crates, refactored onto the platform crates). Client and broker stay separable features of one domain crate. |
| `fleet-dispatch`    | Thin client over pm-stack dispatch interfaces (§6). Lands after the pilot.                                                                                                                                     |
| `fleet-search`      | The atlas-search bin crate rehomed as a domain; atlas-core/atlas-embed/atlas-proto are untouched and consumed as-is. Lands last (§7).                                                                          |
| `fleet` (bin)       | Multicall dispatch (argv0 + subcommand), the stub machinery, variant feature selection, `version` / `capabilities` / `help`.                                                                                   |

Two platform rules carry over from tonight's lessons.
**Secrets are resolved by name through one crate** — no
domain shells out to `bws` or reads a token file on its own;
the three `X-Agent-Token` files stay files, but exactly one
code path reads them. **Logging is one facility** — the
structured-log shape the brokers already emit, with the
domain as a field, so one journal query follows a request
across proxy → broker → client.

## 2. Variant matrix

Compile-time capability selection via Cargo features on the
bin crate. Unselected domains are **absent from the artifact**
— the epic's mechanical-incapability rule. Exactly three
shipped variants (the testing rule: the shipped matrix is
tested, the feature power set is not):

| Variant         | Flake attr               | Features (domains compiled in)                                                         | For                                                             |
| --------------- | ------------------------ | -------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `fleet`         | `packages.fleet`         | registry, config, secrets, proxy serve+connect, queue client+brokers, dispatch, search | the laptop and the sandbox — every full client and every daemon |
| `fleet-serve`   | `packages.fleet-serve`   | registry, config, secrets, proxy serve+connect, queue brokers, search **serve only**   | a host that runs daemons but is not a working seat              |
| `fleet-connect` | `packages.fleet-connect` | registry, config (minimal), proxy connect/init — **nothing else**                      | drop on any box: codespaces, CI runners, a borrowed shell       |

Why a serve variant at all, when the census shows the laptop
runs everything and the sandbox runs clients: it is the
cheapest possible proof that role separation is real — if
`fleet-serve` cannot accidentally grow a client-only surface,
the feature gates are honest — and it is the build a second
host (or a rebuilt laptop, mid-recovery) wants. The honest
counter-option is two variants (full + connect) with role
shaping done entirely by config domain-disable inside `full`;
it halves nothing (the smoke matrix is per-variant, and both
options test the same domains) but it does delete the
serve-role proof. **Recommendation: three**, with the
two-variant option named for D2.

`fleet-connect` size expectations, stated as a budget with a
calibration point: today's `repo-queue` — serde_json, std
networking, no async runtime — is 0.91 MiB unstripped,
dynamically linked. The connect client needs the framing,
the hello, the shim writer, and the registry (strings):
**target ≤ 1.5 MiB, hard budget ≤ 2 MiB**, stripped, and the
budget is a _build-time contract_ — the nix check for
`packages.fleet-connect` fails if the artifact exceeds it,
so size regressions block the release instead of surprising a
codespace. Linkage (D7): a glibc-dynamic binary still assumes
the host's loader and libc match its build host's — exactly
what "drop on any box" denies. **Proposal: `fleet-connect`
builds static musl**; the step-1 spike measures the size
delta (expect modest growth, still inside budget) and runs
the binary under `env -i` and in a foreign container image.
If musl costs more than the budget allows, the fallback is
glibc-dynamic with the constraint documented on the variant —
Drew rules on the measurement, not on the guess.

Per-variant smoke tests (each is a nix check or a scripted
post-build run, all offline except where noted):

- **all variants**: `version` prints the workspace version +
  enabled capability list + source rev; `--help` renders the
  full registry with `[not in this build]` marks; invoking a
  known-but-absent domain exits **69** with the stub text (§5).
- **fleet**: queue `health` against a stub broker; search
  `get-chunk` against a fixture DB; `proxy serve` + `connect`
  round-trip on a scratch socket; each `serve <role>` binds
  its scratch port and answers `/health`.
- **fleet-serve**: every serve role answers `/health`; every
  client-only domain (`search` standalone mode, `dispatch`)
  stubs at 69.
- **fleet-connect**: size ≤ budget (build gate); `env -i`
  run; `init` against a scratch broker materializes the shim
  dir (§4); every domain except `proxy` stubs at 69.

## 3. Multicall and stubs

**argv0 map** — every name that must keep working the day the
symlinks flip, because call sites (45 cron bodies, hooks,
worker instruction files, units) invoke these names, not
`fleet <domain>`:

| argv0                 | Routes to                   | Notes                                                                                                |
| --------------------- | --------------------------- | ---------------------------------------------------------------------------------------------------- |
| `fleet`               | subcommand dispatch         | `fleet <domain> …`                                                                                   |
| `repo-queue`          | `queue` client              | the pilot's compat promise                                                                           |
| `queue-broker`        | `serve queue-broker`        | unit ExecStart name today                                                                            |
| `secrets-broker`      | `serve secrets-broker`      | unit ExecStart name today                                                                            |
| `laptop-agent`        | the laptop-agent **client** | today's `~/tools/laptop-agent.py` CLI shape; the daemon keeps its unit and is not argv0-invoked (§6) |
| `atlas-search`        | `search`                    | after the search domain lands (§7)                                                                   |
| `laptop-agent-broker` | `proxy serve`               | the sandbox broker's current process name                                                            |

**The registry is in every build.** `fleet-registry` is
strings and a table — kilobytes — so even `fleet-connect`
can name the whole fleet. Dispatch order in the bin crate:
argv0 alias → compiled-in domain? run it : known domain? stub
: unknown name → clap's normal error. `--help` on any variant
lists **all** domains; absent ones are suffixed
`[not in this build]`, and compiled-in ones carry their
one-liners. `fleet capabilities` prints the same table as
data (JSON) for agents.

**Stub format** (stderr, one block):

```
fleet: this build does not include 'queue'.
Enabled capabilities: proxy-connect.
This is the fleet-connect build (0.1.0). The full build is 'fleet'.
```

**Exit code: 69 — `EX_UNAVAILABLE` from sysexits.h** — "a
service is unavailable," which is precisely the semantics: the
capability is not in this artifact. The justification is the
collision table; every nearby code already means something an
agent must not confuse with "wrong build":

| Code      | Already means                                                                                                    |
| --------- | ---------------------------------------------------------------------------------------------------------------- |
| 0 / 1     | success / generic failure                                                                                        |
| 2         | clap usage error; repo-queue validation failure                                                                  |
| 3         | repo-queue: broker path / config unavailable — **transport down, retry or repair**                               |
| 4 / 5     | queue-broker HTTP failure / no row                                                                               |
| 126 / 127 | shell: not executable / command not found; the omnibin dispatcher uses 127 for an unresolvable `program@version` |

69 is distinct from all of them, is a documented standard
constant rather than an invented number, and gives agents a
clean branch: **69 → reroute** (proxy through, or find a
fuller build); **3 → repair the transport**. A config-disabled
domain (§7) uses the same code with different wording —
`disabled by config on this host` — so agent branching stays
binary: 69 means "not here," and the text says why.

## 4. The proxy protocol

The proxy is the distribution mechanism, so its protocol is
the one contract every variant — of every age — must keep.
Design rule, taken from atlas-proto's discipline: **the wire
contract lives in `fleet-proxy-proto`, is versioned from the
first frame, and outlives any variant's feature set.**

**Topology (unchanged in v1).** Two hops, because they solve
two different problems: the sandbox broker's Unix socket is
the _warmth_ layer (it holds the pooled CONNECT tunnels so
clients skip the ~1.8 s handshake), and the CONNECT tunnel
through the egress proxy to the laptop's tailnet IP is the
_reach_ layer. `fleet proxy serve` is the broker; `fleet
proxy connect` is the client half; `fleet proxy serve` on
the laptop side is a no-op role in v1 (the laptop's services
bind their tailnet ports directly, as today) — the role
exists in the registry so a future relay host is a config
change, not a protocol change.

**Framing (unchanged).** Length-prefixed JSON — 4-byte
big-endian length + one JSON object — both directions, the
exact framing the Python broker runs today. What changes is
the first frame on every pooled connection:

```
client → { "hello": { "proto": 1, "client": "fleet-connect 0.1.0",
                      "capabilities": ["proxy-connect"] } }
broker → { "hello": { "proto": 1,
                      "services": { "4799": {"name":"laptop-agent","version":"…"},
                                    "4800": {"name":"secrets-broker","version":"…"},
                                    "4801": {"name":"queue-broker","version":"…"} },
                      "capabilities": ["proxy-serve"] } }
```

Requests then keep today's shape (`{method, path, body,
port?, token_file?}`). **Versioning policy:** the integer is
the protocol _major_; a broker must accept majors N and N−1
(the window is one major — a client older than that gets a
clean `proto_too_old` error naming the minimum, not a hung
socket); fields are additive within a major (unknown fields
ignored, the JSON rule the queue envelope already follows).
**Capability negotiation is the services map**: it is the
broker's actual routing table, not a promise — a serve build
without the queue broker advertises a map without 4801, and
clients must treat an unadvertised port as _not here_ (the
proxy-level analogue of the 69 stub). Per-service version
strings ride the map so a client can feature-detect a broker
capability instead of guessing from the fleet version.

**PATH exposure on connect.** The expectation Drew set —
"the moment you're connected to the proxy you can expect
repo-queue on your PATH" — is implemented as a **materialized
shim dir**, regenerated at every `connect`/`init` from the
negotiated services map: `~/.local/state/fleet/shims/` gets
one argv0 symlink per advertised tool (`repo-queue`,
`laptop-agent`, `atlas-search`, …) pointing at the local
fleet binary, and `connect` prints (and with `--print-path`,
emits) the PATH prefix to use. Regeneration-from-negotiation
is the point: tonight's drift came from hand-maintained stubs
in `~/tools/bin` that silently aged; a shim dir rebuilt from
what the broker _actually advertises_ cannot advertise a
service that is not there. Shims for domains the local build
lacks still materialize — invoking one produces the 69 stub,
which is the correct, branchable answer.

**`fleet init`.** Idempotent autostart, Drew's standing
directive in command form: if the broker socket answers a
hello, `init` is a no-op (exit 0, one status line). Otherwise
it spawns `fleet proxy serve` detached (setsid, the
run-switch lesson: never a child of the caller's session),
waits for the hello, materializes shims, prints status. The
Python clients' "auto-start when the socket is missing"
behavior becomes this one explicit verb plus the same
auto-start inside `connect`.

**Serve-side state and restart semantics.** The routing table
is **config, not state** (ports/services declared in
`fleet.toml`, §1) — a broker restart loses nothing but its
pooled tunnels, which clients re-dial transparently (today's
behavior, kept). The broker persists **nothing** to disk:
no job table (jobs live in the laptop-agent daemon), no
request log beyond the journal, no token material (token
files are read per request by path, as today). Idle exit
after 3600 s without a client request is retained as a
config value — the broker is a cache of warmth, and a cache
that exits when cold is a feature on a shared sandbox. One
deliberate change from today: keepalive traffic must not
reset the idle timer (the Python broker already encodes this
— "client requests only" — and the port keeps the rule
explicitly, because it is the kind of rule that gets lost in
a rewrite).

## 5. What serve mode holds (daemon roles)

The daemon roles are **subcommands of the one binary** —
`fleet serve queue-broker`, `fleet serve secrets-broker`,
`fleet serve search` — with **one systemd user unit per
role**, ExecStart pointing at the same artifact. One artifact,
several units: a secrets-broker crash restarts alone; a bad
fleet deploy rolls all units back with one package flip
(§7); journal and version are uniform. The alternative —
one combined daemon hosting all roles — was rejected on the
census's own evidence: the roles have different blast radii
(the secrets broker holds the vault cache; the queue broker
holds the Supabase service-role key it fetched; the search
service holds 1.6 GB of embedder) and different restart
reasons, and the current three-unit shape has never been the
problem. The problem was three _source homes_; the units
were fine.

## 6. Domain scope calls

**pkgs/id — stays its own binary (D4).** id is a product: a
P2P file-sharing application with its own users, its own
in-flight crate-split, vendored third-party crates, and a
release cadence that answers to its roadmap, not to fleet
operations. Fleet is infrastructure Drew's agents run the
fleet with. Merging them couples a product's releases to the
ops toolchain's — the exact coupling the epic accepts _among
ops tools_ because they share distribution pain; id does not
share that pain (its `default.nix` ships fine). The transfer
from id to fleet is the _pattern_ (workspace factoring,
one optimized graph), already taken. Fleet's shim layer may
advertise `id` on PATH like any external tool; it does not
own it.

**laptop-agent daemon — Python stays in v1 (D5).** The daemon
(remote exec: jobs, polling, kill, batching) is 100% of how
agents act on the laptop, and its semantics are delicate in
ways the fleet has already paid to learn (no client-visible
exec timeout — code#5405; batch/job edge cases). A Rust
rewrite inside this epic would make the _pilot's_ critical
path depend on the riskiest single port. v1: the daemon keeps
running byte-identical (the copy vendored in PR #1459 was
sha256-verified against the live daemon — that verification
habit carries over), its unit is ported with the others, and
`fleet` manages it as an external role: health through the
proxy, version reported in the services map. The rewrite is
a named post-v1 candidate, gated on an exec-semantics parity
spec written from the Python's behavior — its own design
note, not a step of this epic.

**Dispatch — thin domain in v1 (D6).** pm-dispatch and its
siblings are a working TypeScript/Bun service suite in code
(`pm-stack/`), launched as their own stack, gated by the
steward cron. `fleet dispatch` in v1 is a **client**: status
and launch verbs over pm-stack's existing interfaces (its
rendered state under `~/.local/state/pm-dispatch/`, its
service endpoints), so agents get one binary to hold while
the stack they talk to stays untouched. Absorbing pm-stack
into Rust is a rewrite decision about a different stack,
taken (if ever) on its own evidence — the epic's mistake to
avoid is letting "dispatch" silently balloon this program.
What fleet _does_ absorb is the queue-side meaning of
dispatch: enqueue/claim/park are already the queue domain.

**Search — joins last, unchanged.** The atlas workspace's
crates move as-is; only its bin crate rehomes as
`fleet-search`, and only after epic #5350 step 6 has cut the
standalone service over. Atlas's own gates (bit-identity,
golden parity) remain its release criteria inside the fleet
workspace — a fleet release that changes search results must
re-pass them (§7).

## 7. Migration plan and release mechanics

Additive-first throughout: at every step the previous path
keeps working until its replacement is verified, and every
flip is a single symlink or unit re-point that flips back.

0. **This note; Drew rules §8.** No step issues are filed
   before the go.
1. **Skeleton.** `pkgs/fleet/` workspace: registry, config,
   secrets and proxy-proto crates; the bin with multicall
   dispatch, stub machinery, `version`/`capabilities`; the
   three variants building empty-but-correct; every §2 smoke
   test live, including the connect size budget and the
   musl measurement (D7 evidence).
2. **Pilot: queue.** The three broker crates move from
   `~/tools/fleet-brokers` into the workspace as the queue
   domain (the sandbox tree is archived, not deleted).
   Standalone `repo-queue`/`queue-broker`/`secrets-broker`
   artifacts keep building from the workspace during the
   whole migration. Flips, one at a time, each verified by
   `health` plus a real enqueue→claim→complete round-trip:
   secrets-broker unit → queue-broker unit → sandbox
   `~/tools/bin/repo-queue` symlink. Rollback per flip:
   re-point at the kept artifact.
3. **Proxy.** `fleet proxy serve` takes over the sandbox
   socket path (the Python broker keeps its socket until the
   flip; the flip is a restart with the same path), then
   `connect`/`init` replace the client autostart habits, then
   `laptop-proxy-connect.py` retires (SSH's ProxyCommand
   re-points at `fleet proxy connect`'s stdio mode — the last
   Python piece, retired only after SSH-through-fleet is
   proven over days, not minutes). `fleet-connect` ships to
   codespaces/CI here.
4. **Dispatch (thin).** Client verbs over pm-stack; the
   steward cron's call sites move only if they want the
   verbs — no cron is rewritten by this epic.
5. **Search (last).** After #5350 step 6: atlas bin crate
   rehomes; the atlas user unit re-points at `fleet serve
search`; standalone `atlas-search` argv0 keeps working
   from the same workspace.

**Config-level domain disable.** `fleet.toml` carries a
`[domains]` table (`queue = false`, …). A disabled domain
dispatches to the stub path with exit 69 and the wording
`disabled by config on this host`. This is the per-host
shaping mechanism (a seat that must never run brokers
disables `serve` roles) and the incident lever: a domain
misbehaving fleet-wide can be darkened by config push
without a rebuild or a rollback.

**Domain-scoped rollback, in practice.** Three depths,
cheapest first: (a) config-disable the domain (seconds, no
deploy); (b) re-point that domain's argv0 symlink or unit
ExecStart at the kept standalone artifact from the same
workspace (the migration's standing inventory — standalones
are built and published alongside the variants until the
epic closes); (c) roll the whole fleet package back one
version (one derivation flip in the home config). (b) is
why the standalones stay buildable _during_ migration; when
they retire is an epic-close decision, not a step decision.

**Release mechanics.** One workspace, one version
(`workspace.package.version`, semver), one `Cargo.lock`. All
variants of a release carry the same version and report it
plus their capability list and source rev via `fleet
version` — the surface §1's census found missing. A release
is one commit + one changelog entry in `pkgs/fleet/`;
variants never version independently (the proxy protocol's
own integer, §4, is the only separately-versioned contract,
deliberately). Any release touching the search domain
re-runs the atlas gates before the tag; any release touching
the queue domain re-runs the round-trip verification against
a scratch queue before the tag.

## 8. Risks and the go/no-go list

**Risks, named plainly.**

- **The pilot is load-bearing.** The queue is how the fleet
  assigns all repo work; a botched pilot flip stalls every
  lane. Mitigated by one-unit-at-a-time flips and depth-(b)
  rollback — but the pilot step's verification bar is the
  highest in the epic for a reason.
- **Three copies drift during migration.** Until the move,
  fixes can land in the wrong copy (tonight's broker fixes
  went to the sandbox tree; PR #1459's vendored copy is
  already a snapshot). Mitigation: the move is step 2, first
  after the skeleton, and the other copies are marked
  archived/superseded the day it lands.
- **Protocol freeze.** The hello frame makes the proxy
  protocol a public contract the day `fleet-connect` ships
  to codespaces. The N/N−1 window and additive-fields rule
  are the mitigation; the risk is freezing a _bad_ shape —
  which is why the framing is today's proven framing, not a
  redesign.
- **The Python island.** laptop-agent stays Python inside a
  Rust program (D5) — accepted, bounded, and revisited only
  with a parity spec. The inverse risk (rewriting it inside
  this epic) is the one being avoided.
- **Scope gravity.** "One binary for everything" will attract
  every tool in `~/tools/bin` (there are dozens, most of them
  omnibin-resolved third-party programs that are _not_ fleet
  tools). The registry is for fleet domains; the shim layer
  advertising external tools (id, omnibin programs) must not
  become ownership of them.

**Decisions Drew rules at go/no-go:**

- **D1 — Source home + PR #1459.** `pkgs/fleet/` in
  developing-today/code as the one source home (recommended),
  and dezren39/nix PR #1459 closed with its home-module
  pattern ported into code's home tree — vs. letting the nix
  repo keep the broker packaging.
- **D2 — Variant matrix.** Three shipped variants
  (full / serve / connect, recommended) vs. two
  (full / connect) with config shaping inside full.
- **D3 — Stub exit code 69** (`EX_UNAVAILABLE`), with 3 kept
  to mean transport-down (recommended) — or another pinned
  code with the same distinctness property.
- **D4 — pkgs/id** stays its own binary, not a fleet domain
  (recommended).
- **D5 — laptop-agent daemon** stays Python in v1; fleet
  manages lifecycle only (recommended) — vs. a Rust rewrite
  inside this epic.
- **D6 — Dispatch** is a thin client domain over pm-stack in
  v1; pm-stack is not rewritten (recommended).
- **D7 — fleet-connect linkage:** static musl if the step-1
  measurement holds the ≤2 MiB budget (recommended path);
  Drew rules on the measurement at step 1's report, or
  pre-commits to glibc-dynamic.
- **D8 — Proxy protocol:** keep today's length-prefixed JSON
  framing + the new hello/capability frame and the N/N−1
  major window (recommended) — vs. redesigning the framing
  as part of this epic.

A "go" on the epic with rulings on D1–D6 lets step issues be
filed immediately (D7/D8 resolve inside step 1–3 without
blocking them). A "no-go" on any single D is a scope change,
not a veto of the whole — the note is written so each D can
flip independently and the rest of the design stands.
