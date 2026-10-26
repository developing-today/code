# 0004 — Scope-resolved roots: spawn cwd, `${mcpx.root}` substitution, initialize workspace

```
number:   0004
status:   accepted
date:     2026-10-26
issues:   dezren39/nix#817 #818 #819 (design source, consolidated here) ·
          implementation tracked in this repo, see "Slices" below ·
          touches #308 #309 #864
code:     pkgs/mcpx at the merge commit of this record's PR
```

> mcpx scopes already decide _which_ process serves a caller, resolving the
> caller's cwd to a git root to do it. But nothing tells that process _where
> the caller is_: the child spawns in the daemon's cwd, argv and env are the
> config's static strings, and the initialize handshake carries no workspace.
> Three issues in dezren39/nix (#817, #818, #819) proposed the three halves
> of one fix from a single design note (2026-10-25). This record consolidates
> them into one design, decides the questions they left open, and is the
> design's home from now on: the source lives in this repo, so the design
> lives here too. The nix issues stay open until the #309 re-homing pass
> closes them with pointers; their acceptance criteria carry over verbatim
> into the implementation issues filed from this record.

---

## Context

### What the code does today (verified at the branch point of this record)

- `internal/config/scope.go` — `Scope.Key(ctx CallContext)` resolves the
  caller's cwd via `locateGit` (`internal/config/gitlocate.go`, native
  discovery with a `git rev-parse` fallback; `gitWhere` carries `commonDir`,
  `worktree`, `noWorktree`) and returns an **opaque key only**
  (`repo:<common-dir>`, `worktree:<top-level>`, `cwd:<dir>`, `global`,
  `session:…`, `psession:…`, `pid:…`, `call:…`). The resolved root is
  computed and then thrown away.
- `internal/daemon/registry.go` — `keyFor(p, cc)` computes the key from the
  `CallContext`; `internal/pool/pool.go` — `Acquire(ctx, key)` receives only
  the key. `startLane` → `dial()` builds
  `StdioOptions{Cwd: p.cfg.Cwd}` verbatim (`internal/mcpclient/stdio.go`
  sets `cmd.Dir = opts.Cwd`). A `repo`- or `worktree`-scoped server
  therefore still _runs_ in the daemon's cwd.
- `internal/config/config.go` — the `Server` struct holds
  `Command`/`Args`/`Env`/`Cwd` with no placeholders; there is no templating
  anywhere in `internal/config` or `internal/pool`.
- `internal/mcpclient` sends no `rootUri` and no `workspaceFolders` at
  initialize (grep: none).
- `internal/pool/eras.go` — `Identity(c *config.Resolved)` hashes the
  **static** spawn definition (command, args, env, cwd, transport, URL,
  headers) for the era cache.
- Instance lifecycle is keyed sharing: instances are keyed by
  (server, scope key), capped (`pool.max`, default 4), LRU-evicted at
  capacity, stopped after `IdleTimeout` (default 5 min) by `ReapIdle`, and
  stopped eagerly for caller-owned keys (`CallContext.CallerOwned`).
  None of that changes here.

### What each issue proposed

- **nix#817 (step 1)** — when a server's scope is `repo`/`worktree` and its
  config `cwd` is empty, spawn the instance with `Dir` = the resolved scope
  root. The scope resolution must hand the pool the root, not just the key.
  Explicit config `cwd` always wins.
- **nix#818 (step 2)** — two placeholders, substituted at instance-spawn
  time only: `${mcpx.root}` (the scope-resolved root) and `${mcpx.cwd}`
  (the caller's cwd), in `Args`, `Env` and `Cwd`. Stored config stays
  static; substituted values must feed the era-hash inputs; unknown or
  malformed placeholders are a loud error, never passed through.
- **nix#819 (step 3)** — when an instance has a scope-resolved root, send
  it at initialize as `rootUri` + `workspaceFolders`, so workspace servers
  (gopls) start a real workspace session. `global` instances send nothing —
  no fabricated daemon-cwd root.

### The casualty that motivates all three

gopls is a workspace server whose workspace is its process cwd and its
initialize root. Behind mcpx today it loads files ad hoc
("command-line-arguments" package) and `go_package_api` on a module path
returned **empty after 156 s** (live fleet, 2026-10-25). Flipping gopls to
`"scope": "worktree"` before these land only multiplies processes; the
config flip is deliberately the last slice.

---

## Decision

### 1. One resolution object: `ResolvedScope`

`Scope.Key` gains a sibling in `internal/config`:

```go
type ResolvedScope struct {
    Key      string // exactly what Key() returns today
    Root     string // the scope-resolved directory, "" when the scope has none
    Degraded string // as Key() reports today
}

func (s Scope) Resolve(ctx CallContext) ResolvedScope
```

`Key()` becomes `Resolve(ctx).Key` so there is one resolution path, not
two. Root semantics per scope:

| scope                                                                        | Root                                                                                                                                                                                                                                     |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `worktree`                                                                   | the caller's checkout top-level (`gitWhere.worktree`)                                                                                                                                                                                    |
| `repo`                                                                       | the main worktree: the parent directory of the common dir, **only** when the common dir's base name is `.git`; otherwise `""` (bare repos and submodule-style common dirs whose parent is not a worktree root get no root — never guess) |
| `cwd`                                                                        | the caller's cwd, canonicalized exactly as the `cwd:` key canonicalizes it                                                                                                                                                               |
| `worktree`/`repo` degraded to a `cwd:` key (outside a repo, unreadable repo) | that same cwd — the instance is effectively per-cwd, so rooting it there is consistent, and the degradation is still logged as today                                                                                                     |
| `global`, `session`, `psession`, `pid`, `call`, and any fallback key         | `""` — these scopes have no meaningful root, and an empty root is what keeps `global` instances from fabricating one                                                                                                                     |

Root is a pure function of the key for `repo`/`worktree`/`cwd` scopes,
which is what makes the plumbing in §2 safe: two callers who share a key
always resolve the same root.

### 2. Plumbing: the pool learns the root at acquire time

`Registry.keyFor` already holds the `CallContext`; it switches to
`Resolve` and passes the `ResolvedScope` alongside the key into
`Pool.Acquire`. The pool stores `root` on the `Instance` at creation;
`startLane`/`dial()` read it from the instance being started (including
restarts and era-lane redials — the root cannot change for a given key, so
re-resolution at redial is unnecessary and must not happen).

### 3. Spawn cwd (from #817)

Stdio only. The effective spawn cwd is, in order:

1. the config's `cwd`, after §4 substitution, when non-empty — explicit
   config always wins;
2. the instance's resolved root, when non-empty;
3. unset (today's behavior: the child inherits the daemon's cwd).

HTTP/SSE servers have no process cwd and are unaffected.

### 4. Substitution (from #818)

At spawn time only, on the _effective_ spawn definition, in `Args`, every
`Env` value, and `Cwd`:

- `${mcpx.root}` → the instance's resolved root;
- `${mcpx.cwd}` → the caller's cwd from the `CallContext` that created the
  instance (canonicalized as in §1).

Rules:

- Substitution never touches the stored or merged config; `mcpx` config
  output keeps showing the placeholders.
- Placeholder **names** are validated at config load: any token matching
  `${mcpx.<name>}` with an unknown `<name>`, and any malformed `${mcpx.`
  sequence, is a config error naming the server and the field. Only the
  `mcpx.` namespace is mcpx's — every other `${…}` form passes through
  verbatim, exactly as today.
- If a server's spawn definition references `${mcpx.root}` and the
  instance's root is `""` (e.g. the server is `global`-scoped), the spawn
  fails loudly naming the server and field. Silently substituting an
  empty or daemon-cwd root is the failure mode this design exists to
  remove.
- The substituted definition is what `pool.Identity` hashes for that
  instance's era record: two instances of one server differing only by
  substituted root must not share an era entry. (Today `Identity` reads
  the static `Resolved` config; the slices must move the hash input to
  the effective per-instance definition and verify this holds, per #818.)

### 5. Initialize workspace (from #819)

`mcpclient.Options` gains the instance root. When it is non-empty, the
initialize mcpx sends for that instance includes:

- `rootUri`: the root as a `file://` URI;
- `workspaceFolders`: `[{uri: <same>, name: <base name of the root>}]`;
- the `workspace.workspaceFolders` client capability declared true.

Both fields are sent for every era mcpx speaks (legacy and modern):
initialize params are extensible, servers that do not consume workspace
fields ignore them, and era-specific withholding would need per-server
quirks this design refuses to grow. When the root is empty, initialize is
byte-for-byte what it is today. Servers that connect over HTTP get the
same treatment — the root describes the caller's workspace, not the
transport.

### 6. Observability

Every instance spawn logs one line — server, key, root, and the cwd
source (`config` / `scope` / `daemon`) — at the same level the daemon
already logs scope degradations. `mcpx status` instance JSON gains a
`root` field. The spawn-cwd decision must be testable without a live
server (a pure function of scope + CallContext + config cwd).

### 7. codedb and codebase-memory: deliberately global

Decided, closing the open question from the 2026-10-25 design note:
**one shared codedb, `global` scope, is the fleet design and stays.**
codedb and codebase-memory are multi-project _inside_ one process —
their indexes are keyed by project path and the project is a per-call
argument — so per-project instances would duplicate indexes and buy
nothing. This decision is about the default, not a limitation: §4 is
what lets an argv-rooted server (codedb's index root is baked into its
argv in the fleet config today) express a per-scope root if a future
config ever wants one.

The shared codedb's index root stays a per-machine config concern (the
machine's git tree: `/home/user/git` on the laptop, `~/git` on the
sandbox), living in the machine-side config, not in the shared
`.mcpx.json` — the absolute-path leftovers in that file are #864's to
clean up. No new shared index location is created by this design.

### 8. Config surface

No new config fields, no new scope values. The entire config-visible
change is the two placeholders and, in the final slice, flipping
`gopls-mcp-server` to `"scope": "worktree"` in the fleet config. The
user-level config stays what it is: a symlink to the repo `.mcpx.json`,
with identical server lists on both machines — unchanged by this design.

### 9. Failure modes

- **Root vanished before spawn** (checkout deleted between resolution
  and start): the spawn fails with the OS error surfaced as the instance
  start error. No silent fallback to the daemon cwd — a server running
  in the wrong workspace returns wrong answers, which is worse than a
  failed start.
- **Unreadable or partial git discovery**: unchanged from today —
  `locateGit` degrades the key, the degradation is logged once, and the
  root follows the degraded key per §1.
- **Substitution errors**: load-time for names (§4), spawn-time for an
  empty `${mcpx.root}`; both name the server and the field.
- **Era-cache collisions**: covered by §4's identity rule; a regression
  here shows up as a server speaking the wrong era after a root change,
  which the slice-2 tests must pin down.

### Non-goals

No new scopes; no daemon, service or deployment changes (how mcpx is
installed or started on either machine is untouched); no per-project
codedb or codebase-memory; no changes to sharing, leasing, idle or
eviction policy.

---

## What every affected issue has to change

- **dezren39/nix#817, #818, #819** — design source only from this
  record's merge. Their acceptance criteria carry over verbatim into the
  implementation issues below. They stay open in nix until the #309
  re-homing pass closes them with pointers to this record and to the
  implementation issues (closing them is #309's acceptance criterion,
  not this record's).
- **#309** — the re-homing list gains this record as the design home for
  the per-project trio; the trio's implementation issues live here, so
  #309 should not re-file duplicates of them in this repo.
- **#308** — untouched: how downstream flakes consume mcpx is orthogonal
  to what a scope resolves to.
- **#864** — untouched, but slice 4 below depends on the fleet config
  being in its post-#864 shape (no `/home/user` absolute paths) or must
  say why it proceeded anyway.

## Slices (implementation issues filed from this record)

1. **ResolvedScope plumbing + spawn cwd** — §1, §2, §3, §6. (from #817)
2. **`${mcpx.root}` / `${mcpx.cwd}` substitution + era identity on the
   effective definition** — §4. Depends on slice 1's plumbing. (from #818)
3. **Initialize `rootUri` / `workspaceFolders`** — §5. Depends on slice 1.
   (from #819)
4. **Fleet config flip + live proof** — `gopls-mcp-server` to
   `"scope": "worktree"` in the fleet config; acceptance is the #819
   live test: `go_package_api` on a module path returns the module's API
   promptly through the daemon, from a caller in a linked worktree as
   well as the main checkout. Depends on slices 1 and 3; slice 2 not
   required.

Slices 1–3 each carry the acceptance criteria of their source issue,
plus: tests for the spawn-cwd decision (scope root vs explicit cwd vs
daemon fallback), for degraded roots, and for `global` sending nothing.
