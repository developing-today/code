# Embedding research: ID and mcpx inside fleet (first slice)

Issue: [#10394](https://github.com/developing-today/code/issues/10394), pass 5 of the
fleet-surface design mill [#10389](https://github.com/developing-today/code/issues/10389).
Status: **research and recommendation, not a build.** First slice: the
measurement method with first results (Q1), the current-state inventory that
every verdict rests on, and first verdicts for Q2 to Q4. What is still open
is listed in §6.

Read against `developing-today/code` main `cab55d98`.

## 1. Where things stand today

| Fact                                                                                                                                                                                                                                                                                                                          | Evidence (path at the read SHA)                                                                                                                                                                                  |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| fleet does not call ID or mcpx at all today. The only `mcpx` strings in fleet are test fixtures and titles.                                                                                                                                                                                                                   | `grep` of `pkgs/fleet/crates` for `Command::new("id"`, `id::`, `mcpx`: only `fleet-queue/src/placement.rs` (a `nix build .#mcpx` fixture) and `validate.rs` (a title string)                                     |
| fleet already embeds another Rust service by path, behind a cargo feature: the Atlas search service.                                                                                                                                                                                                                          | `pkgs/fleet/crates/fleet-search/Cargo.toml` (`serve = ["dep:atlas-core", ...]`, `atlas-core = { path = "../../../atlas-search/..." }`); `pkgs/fleet/crates/fleet/Cargo.toml` (`search-serve` feature, in `full`) |
| fleet ships three builds, with a size gate on the smallest one: `fleet-connect` (`--no-default-features`) has a target of 1.5 MiB and a hard budget of 2 MiB, stripped. The release profile optimises for size (`opt-level = "z"`, fat LTO).                                                                                  | `pkgs/fleet/Cargo.toml` `[profile.release]`; `pkgs/fleet/crates/fleet/Cargo.toml` feature comments ("D7 2 MiB gate")                                                                                             |
| ID is already a library plus a binary. `pkgs/id` has `src/lib.rs` and `src/main.rs`, and is a workspace of 19 sub-crates from the #3447 split.                                                                                                                                                                                | `pkgs/id/src/{lib,main}.rs`; `pkgs/id/crates/`                                                                                                                                                                   |
| ID already has a client-only facade with an enforced dependency boundary. `id-client` re-exports the client-safe leaves (`id-core`, `id-caps`, `id-config`, `id-store`, `id-events`, `id-groups`, `id-sync`, `id-directory`) and never pulls `id-worlds` (wasmtime, russh) or `id-web` (axum). A test enforces that boundary. | `pkgs/id/crates/id-client/src/lib.rs`; `tests/android_dependency_boundary.rs`                                                                                                                                    |
| ID's release profile optimises for speed (`opt-level = 3`, thin LTO), and its dependency graph is about 3.5 times fleet's: 1,045 `Cargo.lock` packages against 300.                                                                                                                                                           | `pkgs/id/Cargo.toml` `[profile.release]`; `grep -c '^name = '` on both lock files                                                                                                                                |
| Both crates pin the same toolchain, Rust 1.99.1, and edition 2024.                                                                                                                                                                                                                                                            | `pkgs/{fleet,id}/rust-toolchain.toml`                                                                                                                                                                            |
| mcpx is pure Go by decision. It builds with `CGO_ENABLED=0`, and "iroh via cgo was rejected" in its identity-transport decision.                                                                                                                                                                                              | `pkgs/mcpx/package.nix` (`env.CGO_ENABLED = "0"`); `pkgs/mcpx/internal/daemon/transport_tsnet.go` header                                                                                                         |
| mcpx already serves its API over a unix socket (filesystem permissions are the access control), TCP, and a tailnet identity (tsnet). There is deliberately no fallback between transports.                                                                                                                                    | `pkgs/mcpx/internal/daemon/transport.go`                                                                                                                                                                         |
| mcpx size: 137,923 lines of non-test Go and 132,626 lines of tests, with 3,490 `func Test` across 628 test files.                                                                                                                                                                                                             | `wc -l` and `grep -c '^func Test'` over `pkgs/mcpx`                                                                                                                                                              |

## 2. Q1: what one request costs at each boundary

**Method.** `supplements/ipc_bench.rs` is a dependency-free Rust program,
built with a bare `rustc -O` (`supplements/run.sh`). Every leg does the same
work: it sends N bytes and gets the same N bytes back, with N set to 64 B
(a small command) and 16 KiB (a document-sized reply). It measures seven
shapes, each timed per round trip after a 10 % warm-up:

1. an in-process call, which is what a static embed costs
2. the one-time `dlopen` + `dlsym` of a cdylib, and then a call through
   the resolved pointer, which is the dynamic-library option
3. a persistent child process over stdin/stdout pipes: a helper exec'd
   once and kept alive, which is the MCP stdio shape
4. a unix domain socket to a separate server process, with length-prefixed
   frames
5. HTTP/1.1 keep-alive over a unix socket, which is the mcpx daemon API's
   shape
6. TCP loopback to a separate server process, which is the floor for any
   remote transport
7. spawn per call: fork+exec of a fresh process for every request, which
   is `fleet` shelling out to `id`

The run was dispatched remotely through `fleet queue dispatch` to a
GitHub-hosted Linux runner, per the remote-builds-only rule.

**First results.**

Run 2026-11-25 02:04 UTC on a GitHub-hosted Linux runner (kernel
7.0.0-azure, x86_64, 4 CPUs, rustc 1.102.0-nightly), fleet dispatch
handle `fp_9e899db360e5889d28fda105cc2654df` (run `dezren39/fleet#45529689370`).
Times are microseconds per round trip.

| boundary                                | payload bytes | samples | min     | p50     | p99     | mean    |
| --------------------------------------- | ------------- | ------- | ------- | ------- | ------- | ------- |
| in-process call (static embed)          | 64            | 200000  | 0.020   | 0.020   | 0.031   | 0.023   |
| dlopen + dlsym (one-time load)          | 0             | 1       | 104.538 | 104.538 | 104.538 | 104.538 |
| dylib call via dlsym pointer            | 64            | 200000  | 0.020   | 0.020   | 0.031   | 0.023   |
| persistent child, stdio pipes           | 64            | 20000   | 5.619   | 12.899  | 17.717  | 13.030  |
| unix socket, framed                     | 64            | 20000   | 7.231   | 12.148  | 18.037  | 11.894  |
| unix socket, HTTP/1.1 keep-alive        | 64            | 20000   | 6.519   | 11.117  | 18.648  | 11.414  |
| tcp loopback, framed                    | 64            | 20000   | 14.952  | 20.981  | 32.098  | 22.473  |
| spawn per call (fork+exec, this binary) | 64            | 300     | 535.951 | 563.913 | 912.540 | 581.766 |
| in-process call (static embed)          | 16384         | 200000  | 0.120   | 0.140   | 0.141   | 0.138   |
| dylib call via dlsym pointer            | 16384         | 200000  | 0.120   | 0.131   | 0.141   | 0.135   |
| persistent child, stdio pipes           | 16384         | 20000   | 14.552  | 17.457  | 25.128  | 17.922  |
| unix socket, framed                     | 16384         | 20000   | 10.166  | 18.308  | 31.348  | 17.903  |
| unix socket, HTTP/1.1 keep-alive        | 16384         | 20000   | 9.936   | 16.685  | 28.623  | 16.638  |
| tcp loopback, framed                    | 16384         | 20000   | 25.329  | 30.346  | 38.969  | 30.561  |
| spawn per call (fork+exec, this binary) | 16384         | 300     | 524.263 | 545.885 | 626.988 | 553.914 |

**Reading.**

Four things follow from one run on one runner. They are first results,
not a benchmark suite.

- **Sockets against a separate process: a tie, not a win.** A kept-alive
  child over pipes (12.9 µs p50) and a unix socket (12.1 µs, or 11.1 µs
  with HTTP framing) are within run-to-run noise of each other at 64 B,
  and again at 16 KiB (17.5 µs against 18.3 and 16.7 µs). The issue's
  expectation that "sockets are slightly less overhead" is not shown by
  this run. The cost that matters is the context switch to another
  process, and both shapes pay it once per round trip. HTTP framing
  costs nothing measurable at these sizes.
- **TCP loopback costs about twice a unix socket**: 21 µs against 12 µs
  at 64 B. A remote transport (web or iroh) starts above that floor
  before any TLS or QUIC work.
- **Spawning per call costs about 45 times a kept-alive process.** It is
  roughly 0.55 ms even for a tiny static binary, and a real `id` or
  `mcpx` start-up (config, store, runtime) can only add to it. That is
  fine for a human command and wrong for a loop.
- **A dylib call costs the same as a static call** (0.02 µs at 64 B).
  The only dylib cost is the one-time load: 105 µs for a tiny library,
  and more for a real one. So performance does not decide between
  options (a) and (b). Coupling and safety do (§3).

Measured against these numbers, an in-process call saves about 11 µs per
request over the best cross-process shape. That matters only above tens
of thousands of calls per second, and no fleet path makes calls to ID or
mcpx at that rate, or at all (§1).

## 3. Q2: ID as a library in fleet

| Option                               | Verdict                                                        | Why                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------ | -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| (a) Static embed behind a feature    | **Feasible, and the recommended direction for `full` only.**   | The Atlas precedent shows the pattern already works in this workspace. Same toolchain, same edition. Embed the `id-client` facade, not the whole `id` crate: its tested boundary keeps wasmtime, russh and axum out. The cost lands on build time and binary size, so it belongs in `full`, never in `connect`, which has a 2 MiB gate. Two traps need handling: fleet's `opt-level = "z"` would also apply to ID code unless the workspace sets `[profile.release.package.id-*] opt-level = 3`; and the two lock files must agree on shared crate versions such as tokio, rustls and iroh, which is real merge work. |
| (b) Dynamic library loaded on demand | **Not recommended as a default.**                              | Rust has no stable ABI, so a Rust-to-Rust dylib must be built by the same compiler with the same flags, or go through a C ABI and lose typed calls. The per-call cost is close to a static call (§2), but the gain over (a) is only a smaller fleet binary. The price is a version-skew failure mode that a static build cannot have, plus a search path (standard location, next to `id` on PATH, next to `fleet`, next to PWD) that is itself an injection surface. Keep it open only for the "`id` as a tiny loader of `libid`" idea, which helps no fleet path.                                                   |
| (c) Exec the `id` binary             | **Keep as the zero-coupling fallback.**                        | It works with any installed `id` and any version. Its cost is the spawn-per-call row in §2, which is fine for a human command and wrong for a hot loop. If fleet calls ID often, keep one child alive (row 3) instead of spawning per call.                                                                                                                                                                                                                                                                                                                                                                           |
| (d) Remote over web or iroh          | **Required anyway for cross-device**, and not a local default. | ID's own peers already speak iroh. For a same-machine call it costs at least the TCP row in §2 plus TLS and QUIC framing.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

## 4. Q3: mcpx

| Option                                                             | Verdict                                                            | Why                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ------------------------------------------------------------------ | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Reach mcpx as it is: unix socket (local), its HTTP API, or MCP     | **Recommended default.**                                           | It exists today, its access control is the socket's file permissions, and it needs no change to mcpx. §2's HTTP-over-unix-socket row is its cost per call.                                                                                                                                                                                                                                                                                 |
| (a) cgo build of mcpx (`c-shared` or `c-archive`) linked into Rust | **Not recommended.**                                               | It reverses a recorded decision (pure Go, `CGO_ENABLED=0`). A Go `c-shared` library also brings its own runtime, garbage collector and scheduler into the host process, and its signal handling has to be coordinated with the Rust side. In return it saves only the difference between rows 1 and 5 of §2. If someone wants it anyway, it should be its own issue, as #10394 already says.                                               |
| (b) Port mcpx to Rust                                              | **Feasible only as a long programme, and not justified by speed.** | 138k lines of Go with 3,490 tests is the specification. A port could keep the Go test suite as the oracle, by driving both implementations through the same black-box tests over the daemon's HTTP API and CLI, and then retiring Go package by package. That is a multi-quarter rewrite, and §2 shows the IPC it would remove costs microseconds per call. The only strong reason left would be one-binary distribution, which §5 weighs. |

## 5. Q4: the smush question

If both were embeddable, a `full` fleet build could include ID and mcpx as
features, and every call would be direct. What that buys, plainly:

- **One file to install and upgrade**, with versions that cannot skew.
- **No daemon to keep alive** for local calls.
- **Microseconds per call**, which matters only for chatty, high-rate
  paths. No current fleet path calls ID or mcpx at all (§1).

What it costs:

- the 2 MiB `connect` gate, so these could never be in that build
- a merged dependency graph of up to 1,345 packages before deduplication, against fleet's 300 today
- slower builds for every fleet change
- the mcpx port or cgo problem in §4

**Recommended default topology**, with the alternatives kept open:

1. fleet reaches **mcpx over its existing unix socket API**, and over
   tsnet or HTTP when remote.
2. fleet reaches **ID by exec of `id`** for occasional commands, and
   through **one kept-alive child or ID's local socket** for anything
   chatty.
3. A **`full` build feature that embeds `id-client`** is the first
   embedding worth building, following the Atlas pattern, with
   per-package `opt-level = 3`. It needs a real caller first.
4. Roc apps stay outside, through Wasm, as the issue says.

## 6. Still open on #10394

- Q1 against the real binaries: the startup time of `id` and `mcpx` (exec
  per call) and the mcpx daemon's real per-call latency over its unix
  socket. This needs the Nix builds, which means a remote run on a builder
  with the Nix store, not this bare-rustc harness.
- Q2's dylib-loader shape, measured: an `id` that `dlopen`s `libid` at
  boot, compared with the static binary. This needs a cdylib build of the
  `id` crate.
- The size and build-time numbers for (a): a `full` fleet build with
  `id-client` embedded, compared with today's, measured on a remote
  builder.
