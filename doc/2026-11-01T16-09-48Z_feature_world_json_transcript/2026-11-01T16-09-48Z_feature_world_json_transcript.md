# `id world join --command-mode --json` transcript

## Request

Add an opt-in machine-readable transcript for the line-oriented world join
client. Keep the existing default output and command-mode server-frame output
unchanged when `--json` is absent.

## Contract

`--json` is available with `--command-mode`. Stdout remains JSON Lines. Server
frames retain their existing wire JSON shape (`type` and frame-specific
fields), including asynchronous events and server errors. Local command
responses use this stable record shape:

```json
{"type":"local","level":"info","message":"..."}
{"type":"local","level":"error","message":"..."}
```

`level` is `info` for help and informational local responses, and `error` for
invalid commands or commands unavailable on this client. Each output record is
one line. Server frames remain in receive order; records are written in the
order they are emitted, so asynchronous server frames may appear between local
responses. The client does not echo sent commands or join credentials into the
transcript.

Without `--json`, local messages continue to go to stderr and server frames
continue to print as before.
