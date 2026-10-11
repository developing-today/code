# Takeover passes idle keep-alive connections to the successor

## Intent

`mcpx daemon --takeover` replaces the running daemon. The old daemon drained
its HTTP server with `Shutdown`, which closes idle keep-alive connections. A
client holding one of them could send its next request into a connection
already closed, and get a reset. Go's transport does not retry a POST whose
bytes were written, so the call failed.

The goal is to remove that window: idle connections are passed to the
successor, which answers the client's next request on the same TCP connection.

## Design

- **Tracking.** Every accepted connection is a `handoffConn`, a wrapper over the
  socket. The HTTP server's `ConnState` hook records each connection as New,
  Active, Idle or Closed. Reads and writes go through the wrapper, so the daemon
  knows whether it has taken bytes from a connection.
- **Stop accepting.** The daemon's listeners are `connListener`s. Stopping one
  makes `Accept` return `http.ErrServerClosed`, so the serve loop ends without
  reporting an error. The kernel socket stays open: the successor holds a
  duplicate.
- **Freeze.** A connection not in a request is frozen. A read blocked on the
  socket is woken with a past deadline and freeze waits for it to return. A
  connection that goes idle during the handover is frozen by the hook itself.
  Once frozen, nothing in this process reads the socket again. Its serve loop
  exits, and `Close` on it does not close the socket.
- **Done.** The old daemon waits until it holds no open connection. Every
  connection is then either frozen or closed, so no request can be answered by
  two daemons.
- **Pass.** Each frozen socket is duplicated close-on-exec and sent with
  `SCM_RIGHTS`, one descriptor per frame (`kind: conn`), before the pools. The
  old side closes its copies only after the successor commits.
- **Successor.** Each descriptor becomes a connection with `net.FileConn`. The
  connections are queued on the listener of their network, and the listener
  yields them before any new accept. They are tracked as Idle.
- **Rollback.** If anything before commit fails, the frozen connections are
  thawed once their serve loops have returned, and queued on the restored
  listeners. Keep-alives were never turned off, so they keep working.

## The read rule

A connection is passed only if no byte has been read from it since the server
last wrote to it.

This was not the first rule. The first rule reset the "taken" flag when the
server went idle. A real-process stress run then failed once in twenty runs
with `405 Method Not Allowed` on a POST: the request reached the successor one
byte short.

The cause is in `net/http`. While a handler runs, the server reads one byte from
the socket in the background, to notice a client that has gone. `finishRequest`
flushes the response before it stops that read (`w.conn.bufw.Flush()` comes
before `abortPendingRead()`). A quick client can send its next request in that
window, and the byte goes into the server's `hasByte` buffer. The server serves
that byte without touching the socket again, so the wrapper never sees it being
consumed.

A byte read after the last write is therefore treated as held. The client cannot
send anything in reply before it has the answer, so a byte that arrives after
the answer is the background byte or a pipelined request. A byte read before the
last write has been consumed: body bytes are read before the response is
written, and the server drains anything left before it goes idle.

`TestAByteReadAfterTheLastAnswerIsNotPassed` covers this. It fails against the
old reset (checked by reintroducing it temporarily).

## Decisions and deviations from the brief

- **Active connections.** The brief said a connection that became Active during
  the handover is not passed, and is closed after its answer. Here it is
  finished by the old daemon and then passed when it goes idle, since its answer
  has already gone out. Keep-alives stay on, so the old daemon never closes a
  connection a client is holding, and the reuse race after a `Connection: close`
  does not arise.
- **Poll interval.** `limits.takeoverPoll` (5ms) in `defaults.json`, per the
  defaults guard.
- **Takeover request.** The hijacked takeover connection is unwrapped before the
  handover, since the handler asserts `*net.UnixConn`.

## Residual gaps

- **Pipelining.** A client that sends a second request before it reads the first
  answer can have that second request in the old daemon's parser at handover.
  The daemon cannot see it, and it is not reproduced on the successor. Go's
  client and curl do not pipeline by default.
- **Long requests.** A request still running when the handover starts is waited
  for up to `http.shutdownGrace`. If it runs longer, the handover rolls back, as
  it did before.
- **Not verified here.** No live systemd user session, and no real MCP server:
  the tests use the fake MCP server from `testsupport`.
- **Unix socket.** The CLI client turns keep-alives off on the unix socket, so
  the window this removes is for TCP keep-alive clients. The unix listener gets
  the same handling.

## Verification

Run on the branch rebased onto `origin/main` at `9168c105`, from `pkgs/mcpx`:

- `go vet ./...`: clean.
- `go test ./... -count=1`: 38 packages pass. `internal/e2e` has two failures,
  `TestPromptSaysPlainlyThatThereIsNoModel` and
  `TestEveryDeclaredRouteAnswersOverTheSocket`. Both also fail on the base
  commit `7455b51d` and do not touch the takeover path.
- `go test ./internal/daemon/ -count=1 -v`: 70 passed, 0 failed.
- `go test ./internal/e2e/ -count=1 -v` (run before the final rebase, same
  takeover code): 315 top-level tests passed, 2 failed (the two above).
- `nix build .#mcpx --no-link`: succeeded.

Stress and race, on the rebased tree:

- Real processes (`daemon --takeover` as a child process), 8 keep-alive callers
  with idle gaps, `-count=20`: 20 runs, 298,591 calls, 0 failed.
- In-process, the same shape, `-count=20`: 20 runs, 118,463 calls, 0 failed.
- `-race`, takeover tests in `internal/daemon`, `-count=5`: 75 passed, 0 failed,
  no data race reported. The stress test in this run carried 7,947 calls, 0
  failed.

Before the held-byte rule, the real-process stress run failed once in 20 runs
(one `405 Method Not Allowed`, out of 223,318 calls). After the rule, 40 further
real-process runs had no failures (586,345 calls), before the final rebase.

Negative control (handover disabled by making the old daemon close idle
connections, as before):

- `TestAnIdleKeepAliveConnectionSurvivesATakeover` fails:
  `the call did not use the connection it held: port 40784 -> 40792, reused false`.
- The in-process idle test fails the same way.
- The real-process stress test failed in 1 of 2 runs with
  `read tcp ...: read: connection reset by peer` (1 of 2,509 calls).
  The in-process stress did not reproduce the reset in 2 runs, so the
  real-process test is the stronger detector.
