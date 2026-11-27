# Alignment request handler

This handler reads #alignment message events from the existing Archer
Socket Mode collector's SQLite store (`collected_events`). It watches
only channel `C0CBYQR65LL`, and treats an owner message mentioning the
configured Archer bot or a reply in a thread where the handler already
participated as a request.

Each request is persisted with an ID, owner, scope, evidence, thread,
and state. Missing scope or evidence produces `BLOCKED`; a complete
request produces `ACK`. A later completion records `RESULT` in the
originating thread. Processed message timestamps and a durable outbox
make restarts and Slack event replays idempotent. The first collector
poll establishes a cursor at the current store tail, so old channel
history is never replayed as new work.

## Staged verification

Run the offline end-to-end fixture:

```sh
python3 -I pm-stack/alignment-handler/handler.py stage
python3 -I -m unittest discover -s pm-stack/alignment-handler -p 'test_handler.py'
```

`stage` uses an in-memory store and a fake sender. It performs one full
owner-mention → ACK → RESULT request; it makes no Slack API call and
does not require credentials. The test suite also covers missing-field
blocking, a participating-thread reply, event dedupe, collector cursor
baselining, and retry idempotency.

## Fleet runtime

`watch` polls the deployed collector's SQLite store and uses the Archer
bot token to deliver pending replies. The send adapter is pinned to the
written #alignment channel allowlist; it refuses other channel IDs. The
NixOS laptop module runs it as a systemd user service beside the collector,
using the same database path. Its launcher resolves `slack.archer-bot-token`
by name through Fleet secrets and never writes or prints the token.

```sh
python3 -I pm-stack/alignment-handler/handler.py result \
  --db "$HOME/workspace/archer-slack/puller/state/archer-slack.db" \
  --request-id <id> --text "<result>" --evidence <url>
```

The staged verifier intentionally does not start `watch` or send a live
Slack message. The service is installed by the laptop's next NixOS switch;
live reliance begins only after the staged run is recorded on issue #6651.
