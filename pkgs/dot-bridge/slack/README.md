# Private Slack bridge for Aster / Fleet

Local reference implementation, **not installed or activated**. Python 3 standard library only. No real Slack messages have been sent by this build.

## What works

A distinct Slack bot sends explicit requests to the verified Dot member in a dedicated **private** channel. Signed Slack Events API messages become a durable local inbox; the local CLI reads, replies and acknowledges them. Requests use stable UUIDs and Slack thread timestamps. Replies correlate only by the authenticated event's channel/thread, never an ID asserted in message text. Unknown senders, other workspaces/channels, edits and self messages are ignored. Unmatched Dot messages are retained as uncorrelated data, never executed; an early reply is correlated when the send receipt is durably saved. There is no automatic echo or model execution.

Every send verifies bot identity and private-channel membership first, refusing Slack Connect/shared channels. User text has Slack control markup escaped; only the deliberate Dot mention is inserted by the bridge. Only `xoxb-` bot tokens are accepted. No user token, `chat:write.customize`, custom username, impersonation or public-channel write capability is requested. The server binds loopback. The public reverse proxy must expose **only `/slack/events`**, with TLS, body size/time limits and rate limits. Do not expose the CLI/database as an unauthenticated API.

## Setup, requiring owner action

1. Create a dedicated private channel, such as `aster-fleet-private`. Invite only the user, the intended Dot integration and the new Fleet Inbox bot. Do not reuse a public coordination channel. Channel membership is the confidentiality boundary; review it periodically.
2. Create/install a Slack app from `slack-app-manifest.json`, replacing its HTTPS URL. This requires workspace permission and acceptance of Slack/app terms. No installation is performed by these files. Invite the bot manually to the private channel. Do not add broader scopes to fix an unexplained error.
3. Obtain the bot token and signing secret via the approved secret manager. Supply `SLACK_BOT_TOKEN` and `SLACK_SIGNING_SECRET` in the process environment; never put them in the config, command arguments, repository or logs. Store state on an owner-only encrypted disk. The CLI applies umask077 to newly created files; secure any pre-existing files/directories yourself.
4. Copy `config.example.json` to a private config path. Record the exact Slack team, installed app, private channel, bridge bot user, Dot user and the user's IDs from verified Slack metadata. Do not infer identity from display names. `allowed_sender_ids` is an exact allowlist, not a list of names. Keep Dot and bridge bot IDs distinct.
5. Run read-only checks: `python3 bridge.py --config CONFIG --db STATE check`. It verifies token workspace/bot identity and channel privacy/membership. Then run `serve --port 8788` behind your approved TLS proxy. Configure the Events API URL verification; the listener verifies Slack HMAC and the five-minute timestamp window before processing. Durable DB commit precedes HTTP200. No webhook processing calls Slack or a model.
6. **Verify Dot delivery separately:** connect this Dot through the product's supported Slack connection/setup, and use its supported invite/mention flow in this private channel. A connector app or a Slack channel by itself is not a Dot wake API. Have the user first send a normal mention and verify this exact Dot replies in the expected thread. Only then test an explicit bot request with the user's approval. Verify the real replying sender ID and thread correlation. Do not impersonate the user if bot-originated messages are ignored.
7. Do not enable unattended requests until the bot-originated round trip is proven. If the product filters bot messages or cannot join private channels, immediate wake is **blocked**. Supported alternatives: owner sends/forwards the request, or an owner-approved scheduled Dot check reads the private inbox through an available supported connector. This implementation does not secretly use ChatGPT sessions, browser endpoints, or claim arbitrary MCP notifications can wake a Dot.

## CLI

All commands take `--config CONFIG --db STATE` before the subcommand.

```sh
python3 bridge.py --config CONFIG --db STATE check
python3 bridge.py --config CONFIG --db STATE serve --port 8788
# Generate and retain one UUID per deliberate send; text is read from a file.
python3 bridge.py --config CONFIG --db STATE request --id UUID --text-file request.txt
python3 bridge.py --config CONFIG --db STATE inbox
python3 bridge.py --config CONFIG --db STATE get-reply UUID
python3 bridge.py --config CONFIG --db STATE reply --id ANOTHER_UUID --event-id Ev123 --text-file reply.txt
# ACK only after the consumer has durably processed/saved the message.
python3 bridge.py --config CONFIG --db STATE ack Ev123
python3 bridge.py --config CONFIG --db STATE outbox
```

Only `request` and `reply` post messages. Receipt state `sent` means Slack accepted the message, **not** that the Dot woke or processed it. Completion requires an authenticated correlated reply and the consumer's own task-verification rules. One event may be read repeatedly until explicit ACK; consumers should dedupe downstream actions by `event_id`.

On network/receipt failure an outbound record stays `uncertain` (or `pending` if the process died). Reusing that UUID refuses to resend. Inspect the private Slack thread/history and reconcile manually before any new send. `client_msg_id` is an aid, not a claimed exactly-once guarantee. A process crash between Slack acceptance and local receipt can orphan correlation; do not claim delivery/processing in that case. No automatic retry/polling schedule is installed. Archive/delete events are not applied; this is an operational inbox, not a complete Slack archive. Events missed during prolonged downtime need explicit history reconciliation; no completeness claim is made.

## Authority and privacy

A valid Slack signature establishes Slack transport provenance, not approval to obey message instructions. Sender allowlisting establishes which participant may enter the inbox, not unlimited delegated authority. Apply the user's actual saved authorization, destination-specific sharing limits, confirmation rules and prompt-injection defenses before executing tasks or replying with private data. No arbitrary URLs or commands in messages are fetched or executed. A Dot's broader private chat history must not be copied into this channel automatically. Any agent needs its own identity and explicit scoped permission, not a shared “Drew” identity.

## Verification

`python3 -m unittest -v test_bridge.py` runs offline with fake Slack API responses. Tests cover HMAC/replay, tenant/channel/sender isolation, dedupe, explicit ACK, self-loop refusal, authenticated thread correlation, uncertain delivery, send idempotence and public/shared-channel refusal, markup escaping and early-reply reconciliation. Run from this directory. Network deployment, actual Slack scopes and Dot bot-event delivery remain unverified until the setup acceptance test.

## Sources and reuse

- [Slack signing requests](https://api.slack.com/authentication/verifying-requests-from-slack): raw-body HMAC and timestamp verification.
- [Slack Events API](https://docs.slack.dev/apis/events-api/): event delivery and retries.
- [Slack message events](https://docs.slack.dev/reference/events/message/): message subtypes.
- [Bot messages](https://docs.slack.dev/reference/events/message/bot_message/): bot messages may carry a user plus bot_id.

Fleet's prior Archer puller informed channel/thread correlation, but its fetch-time watermark advance is deliberately replaced by a persisted inbox plus explicit consumer ACK. No Fleet queue tasks, OpenCode sessions, application installations, external notifications or deployments are created.
