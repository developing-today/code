# oc-attach — attach-first OpenCode client for fleet workers

`~/tools/oc-attach.py` (symlinked as `~/tools/oc-attach`) is the single
documented entry point for programmatic OpenCode work against the
laptop's background service (developing-today/code#1357). Stewardship
dispatch and sandbox tooling use this helper instead of re-improvising
server discovery, credentials, and forwards per workstream.

Programmatic work defaults to ATTACH: sessions live on the laptop
service and survive SSH/transport death; the helper re-discovers them.

## Commands

```bash
oc-attach discover   # refresh the 0600 secret file from the laptop's
                     # ~/.local/state/opencode/service-prod.json
oc-attach forward    # open/reuse the multiplexed SSH forward (idempotent)
oc-attach health     # GET /api/info through the forward
oc-attach list       # list sessions on the laptop service
oc-attach prompt --directory /home/user/git/code \
                 --model opencode-route/workhorse \
                 --text "…" [--session <id>] [--title "…"]
oc-attach run    --directory /home/user/git/code \
                 --model opencode-route/workhorse \
                 "…" [--session <id>] [--title "…"]
```

`prompt` and `run` auto-provision the secret file and forward, so the
one-command form is all a dispatcher needs.

## Safety defaults (baked in, do not bypass)

- The service password is provisioned into
  `~/tools/secrets.d/opencode-laptop-service.json` (dir 700, file 600).
  It is never printed, never logged, never passed in argv (`run` passes
  it via the `OPENCODE_SERVER_PASSWORD` env var only).
- Session location travels in the session-create **POST body**
  (`location: {directory}`) — the `?directory=` query param is silently
  ignored by the server.
- `run` closes stdin for every `opencode run` (an open non-tty stdin
  hangs the CLI, code#1359). The v2 CLI has no `--directory` flag, so
  `run` creates the session through the API first and attaches with
  `--session`.
- The forward is a dedicated SSH ControlMaster
  (`~/.ssh/cm-ocattach@100.99.113.103:22`) with the same pinned host key
  and proxy handling as `~/tools/laptop-ssh.sh`. If it dies, the next
  helper invocation reopens it; sessions keep running on the laptop in
  the meantime. Local port defaults to 13421 (the sandbox's 13420 is
  held by an unrelated wrapper); override with `OC_ATTACH_LOCAL_PORT`.
- Always pass a real project `--directory`: a session defaulted to a
  huge tree stalls on snapshot/git work.
- The laptop's default model route can pick a provider outside Drew's
  plan (observed 2026-10-28: commandcode/gpt-5.6-terra → 403
  MODEL_NOT_IN_PLAN). Pass `--model opencode-route/workhorse` (or
  another known-good route) when creating sessions until the laptop
  default is fixed.

## Verified 2026-10-28 (queue task 1918)

- discover/forward/health/list live against the laptop service
  (v2.0.30+56590c5, 127.0.0.1:13420).
- `prompt` turn end-to-end: exact-reply smoke passed (~7s).
- `run` turn end-to-end: exact-reply smoke passed.
- Forward killed mid-turn (`ssh -O exit` during a sleep-40 turn):
  local port dead, next `oc-attach list` reopened the forward, the
  session was visible, and the turn completed successfully.
- Password grep over helper source and test outputs: absent everywhere.
