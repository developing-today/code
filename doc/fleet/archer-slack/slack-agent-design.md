# Making Muse a Slack agent ("Archer") — design notes

Status: design only, nothing installed. Companion files in this folder:
`slack-app-manifest.json` (ready to create the app from),
`dot-archer-authority.md` (standing-authority text for the Dot).

## How a Slack agent is made (the general recipe)

A Slack "agent" is just a Slack **app** with a bot user, plus a backend
you operate. Five parts:

1. **App + bot identity.** Create the app at api.slack.com/apps (from the
   manifest in this folder). It gets a bot user — name, avatar — that is a
   distinct workspace participant people can invite, @mention, and DM.
2. **Scopes (bot token).** Least privilege per job, e.g.
   `app_mentions:read`, `channels:read/history`, `groups:read/history`,
   `im:read/history/write`, `chat:write`, `users:read`. Scope changes
   require reinstall/re-consent by the workspace owner.
3. **Install + tokens.** Owner installs the app once. You end up with:
   bot token `xoxb-…` (API calls as Archer), app-level token `xapp-…`
   (Socket Mode), signing secret (verifies inbound webhooks if used).
   Tokens live in Bitwarden / Secure Vault, never in files or chat.
4. **Inbound path — pick one:**
   - *Socket Mode:* a long-running process on Muse's VM holds an
     outbound WebSocket to Slack; events arrive with no public URL.
     Needs a supervised daemon that survives VM recycles (systemd +
     init.sh pattern we already have).
   - *HTTPS Events API:* Slack POSTs events to a public endpoint
     (a Cloudflare Worker fits our stack). Must verify the signature,
     ack within 3 s, dedupe retries.
   - *Polling:* no listener at all — a scheduled job pulls new messages
     with the bot token (see the puller below). Simplest and most
     robust; latency is the polling interval.
   Events worth subscribing to if used: `app_mention`, `message.im`,
   `message.channels`, `message.groups`, `message.mpim`.
5. **Brain loop + state.** Event/message → verify & dedupe → enqueue →
   an agent run reads it with context → reply in thread as Archer via
   `chat.postMessage`. Durable state (what has been seen, what is
   pending) lives outside the model: our Supabase `agent` schema or
   Turso, both already running.

Key contrast with the built-in Muse Slack connector: that connector is
Meta's app running on **Drew's user token** — request/response only,
posts as Drew, no events, no presence, one identity. The custom app is
**ours**, runs on a **bot token**, and can receive.

## The "since last review" puller (the tool Drew proposed)

Per watched channel, keep a watermark. One run:

1. `conversations.history(channel, oldest = watermark, inclusive = false)`
   — returns only messages after the watermark; page with cursors.
2. Sort ascending; drop pure noise subtypes as configured (joins/leaves),
   keep edits/deletes handling simple: re-pull a small overlap window
   and dedupe on `(channel, ts)`.
3. Threads: also call `conversations.replies` for parent messages that
   are still "active" (replied to since the last run, or younger than a
   cutoff), so replies to old threads aren't missed.
4. Hand the agent only the new messages (the delta) plus a one-line
   state summary — this is the token saving: state lives in the DB,
   not in re-read channel history.
5. Only after successful processing, advance the watermark to the
   newest processed `ts`. Failure = watermark stays = at-least-once
   processing, with dedupe making replays harmless.

Storage: table `slack_review(channel_id, watermark_ts, updated_at)` +
`slack_seen(channel_id, ts)` PK — Supabase muse-dev fits (agent schema
patterns already proven there).

Rate-limit note: Slack tightened `conversations.history` for
non-Marketplace apps in 2025 (reported: Tier 1, 1 req/min, 15 objects
for distributed apps), while internal workspace-built apps reportedly
keep the higher historical limits. Ours would be an internal app, but
design as if limits are tight: few watched channels, modest cadence
(5–15 min), cursor batching. Events (Socket Mode) are the escape hatch
if polling ever stops scaling.

## Built-in connector vs custom app — verdict

Keep both; they do different jobs.

| | Built-in Muse connector | Custom Archer app |
|---|---|---|
| Identity | Drew | Archer (own user) |
| Direction | Pull only, when asked in Muse chat | Push (events) or scheduled pull |
| Cost to run | Zero build, Meta maintains it | Build ~a day + ongoing care |
| Best at | Ad-hoc search/read across Drew's Slack, drafts, sending as Drew (incl. owner authority with the Dot) | Being addressable in Slack, reliable incremental review, agent-to-agent channels |
| Limits seen live | MCP search 429'd twice under light use (Oct 25); semantic search, no completeness guarantee | Slack API tiers; scope reinstalls; our uptime is our problem |

Phase 1 (recommended): create the app from the manifest, skip the
daemon — scheduled watermark puller over a few watched channels +
posting as Archer. Phase 2 only if latency matters: add Socket Mode
for mentions/DMs. The built-in connector stays for everything
"as Drew".
