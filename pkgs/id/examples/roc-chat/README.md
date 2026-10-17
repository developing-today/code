# Roc Chat Example

A minimal chat client built on `pf.RocCap` and the `chat.history` capability.

- Type a line and press enter to send it with `chat.say`.
- Type `hist` and press enter to ask for the newest eight lines.
- The view reports `seen` (how many lines the last history answer carried) and
  the newest line's text.

## Build

```bash
./build.sh
```

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/examples/roc-chat/roc-chat.wasm" \
  --world-cap chat.say --world-cap chat.history \
  --world-ssh-port 2223 --web --port 8787
```

Grants for `chat.say` and `chat.history` are needed; `chat.history` requires
the `chat` scope, so spectators cannot read logs.
