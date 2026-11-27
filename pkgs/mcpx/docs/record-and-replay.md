# Record and replay tool calls

mcpx can capture a client session at its transport boundary and replay that
session against a fresh client. This turns a field failure or flaky end-to-end
scenario into a deterministic regression input. The implementation is the
family-2 slice of [the in-house simulation evaluation](https://github.com/developing-today/code/issues/8242).

## What a trace contains

A trace uses the versioned `mcpx-call-trace/v1` format. It contains an ordered
list of JSON-RPC frames. Each frame has a direction (`send` from client to
server or `recv` from server to client) and the raw JSON message:

```json
{
  "format": "mcpx-call-trace/v1",
  "frames": [
    { "dir": "send", "msg": { "jsonrpc": "2.0", "id": 1, "method": "initialize" } },
    { "dir": "recv", "msg": { "jsonrpc": "2.0", "id": 1, "result": {} } }
  ]
}
```

The recorder sits below the client, so it can capture initialization,
notifications, server-initiated requests and tool calls without changing the
code that sends them. It records a frame only after the inner transport accepts
it. A failed send or receive is not part of the captured scenario.

Loading is strict: the reader requires a supported format version, at least
one frame, a recognized direction on every frame, and valid JSON in every
message. It rejects the whole trace on an error instead of replaying a partial
session. See [`CallTrace` and `RecordTransport` in the client](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/mcpclient/record.go).

## Opt in to live capture

Live capture is off by default. Set `mcpclient.Options.TraceCapture` when
dialing a client to wrap its transport before the handshake. The resulting
client exposes `RedactedTrace()`, which returns a redacted copy of the frames
recorded so far. Capture continues after that copy is returned.

Trace messages can contain tool arguments, results, and authorization
material. `CallTrace.Marshal()` preserves the messages verbatim and is for
fixtures and replay tests. Before persisting live capture, use
`Client.RedactedTrace()` or `CallTrace.MarshalRedacted()`; these apply the same
credential matcher used by structured logging. Capture is opt in because even
a redacted trace is an artifact that should be reviewed before sharing.
See [`TraceCapture`](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/mcpclient/modern.go)
and [`RedactedTrace`](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/mcpclient/capture.go).

## Replay a scenario

Tests load a trace with `mcpclient.UnmarshalCallTrace` and pass it to
`testsupport.NewReplayTransport`. The replay transport gives the client the
recorded server frames and compares each client frame with the next recorded
one. It treats JSON object key order and whitespace as irrelevant, but keeps
the order of frames in both directions. A different request or interleave
fails with a divergence error; `Finish()` also reports frames the client never
consumed. See [`ReplayTransport`](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/testsupport/replay.go).

The recorder and replay tests are in
[`internal/mcpclient/record_test.go`](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/mcpclient/record_test.go)
and
[`internal/testsupport/replay_test.go`](https://github.com/developing-today/code/blob/main/pkgs/mcpx/internal/testsupport/replay_test.go).

## Limits

Replay reproduces one captured conversation; it does not explore new schedules
or inject new network faults. The recorded server behavior is frozen in the
trace, and replay succeeds only while the client produces the same requests in
the same cross-direction order. A trace also does not capture server state or
other external dependencies. Use the seeded scheduler and deterministic test
seams described in [the simulation evaluation](https://github.com/developing-today/code/issues/8242)
when the goal is to explore behavior beyond a recorded run.
