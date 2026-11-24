# Bridge source survey — pkgs/message-bridge + fleet book chapters, 2026-11-23

## pkgs/message-bridge/worker (at main 0ecb31eef33)

src/: auth.js events.js fleet.js index.js mcp.js messages.js oauth.js
page.js relay.js routes.js wake.js — NO notes/signal module (#13895 not landed).
wrangler.jsonc: name message-bridge, main src/index.js,
PUBLIC_URL=https://bridge.security.cab,
WORKERS_DEV_URL=https://message-bridge.developing-today.workers.dev,
Durable Object binding MESSAGES (class MessageStore, sqlite, migration v1).
Worker secrets (per README/wrangler comments): ACCESS_TOKEN (owner path
token), CHATGPT_TOKEN (ChatGPT /mcp only). Fleet secret names:
message-bridge.access-token, message-bridge.chatgpt-token,
message-bridge.fleet-token.

## Existing bridge (mailbox system) — verified facts

- Channels claude/chatgpt for connectors; fleet mailboxes in src/fleet.js
  in their own tables in the same Durable Object.
- Fleet routes: POST /fleet/<fleet token>/hello {agent, groups?};
  GET /fleet/<fleet token>/inbox (agent, groups, kind, since_id,
  unread_only default true, mark_read, limit);
  POST /fleet/<fleet token>/ack {agent, up_to_id|ids};
  POST /fleet/<fleet token>/send {agent, body, to?, kind?, reply_to?};
  owner: GET /api/<token>/fleet/agents; GET /api/<token>/fleet/messages;
  POST /api/<token>/fleet/send {body, to, kind?, sender?} (kind message
  default | command).
- Fleet token: derived base64url(HMAC-SHA256(key=ACCESS_TOKEN, msg=
  "message-bridge/fleet-token/v1")); Worker secret FLEET_TOKEN (>=32
  chars, distinct) overrides. Any fleet-token holder can read any mailbox
  / send as any agent (README states this trust level plainly).
- fleet.js send case (read directly): targets parsed agent:/bare name,
  group:<g> expanded via registered agents' groups, all/agents; group
  with no members -> error "no registered agent is in group <g>"; one
  INSERT per recipient agent sharing a batch id; unregistered names are
  collected and returned. inbox case: SELECT WHERE agent = ? AND
  direction='inbound' AND id > since_id [AND read_at IS NULL] [AND
  kind = ?] ORDER BY id ASC LIMIT ?; mark_read updates returned rows;
  hello/inbox call touch() to refresh agent+groups.
- Relay grammar (src/relay.js, README "Any to any"): drew/owner/central,
  claude, chatgpt, connectors, agent:<name>/bare name, group:<name>,
  agents, all; sender never receives its own message; kind message |
  instruction (command == instruction for connectors; stored as command
  in agent mailboxes); instructions arrive sender-less (no sender, no
  source, from_addr null) but reply_to reaches the real sender.
- Wake: ChatGPT via MCP Events (message.created, signed webhooks,
  events/list|subscribe|unsubscribe, 7-day subs, max 20); Claude/any
  address via wake hooks (src/wake.js; WAKE_MAX_PER_HOUR default 20);
  fleet agents have no callback — they poll `fleet bridge inbox --wait`.
- OAuth on the bare address for connectors (separate from fleet tokens);
  /healthz reports ok, configured, chatgpt_configured, version,
  token_fingerprint (16-hex SHA-256 fingerprint).
- Deploys: cf versions flow, nix run .#message-bridge-deploy;
  .github/workflows/message-bridge-deploy.yml on pushes to main touching
  pkgs/message-bridge/ (code-only; full deploy local for token rotation).

## #13895 design as recorded (filed 2026-11-24T00:18:27Z, open)

Title: "Bridge: class/name notes, public wake signal, and Archer hook".
Design (verbatim substance): identities class/name (e.g. muse/archer);
authenticated senders address one identity or wildcard either segment
(muse/*, */archer, _/_); notes stored server-side in the bridge Durable
Object, delivered to every registered identity matching the address;
GET /<class>/<name> is a public content-free signal returning the same
bare UUID shape for registered and unregistered paths; registered
signals change on delivery; all paths rotate on deterministic per-path
jittered schedules so a change does not prove delivery or reveal
registration; authenticated inbox + ack endpoints belong to the identity
token; token values kept in the vault by name, represented server-side
only by hashes; a small bridge helper provides send/inbox/ack; Archer
gets a Muse hook polling only the public muse/archer signal, comparing
one UUID to last-seen, waking on change, then fetching + acking inbox.
Acceptance proof list: worker deployed; muse/archer registered; direct

- wildcard notes delivered; signal changed after delivery; hook detects
  change in manual run, dry-run before enabling; inbox fetch+ack verified,
  inbox empty after ack; unregistered path rotates synthetic UUID.
  Privacy properties (issue): no note content, no identity existence
  proof, no differing response shape, decoy rotation for every path;
  tokens never committed/printed/logged.
  Follow-up comment on #13895 by dezren39, 2026-11-24T00:33:27Z, recording
  Drew's direction (~18:29 CST): fleet subcommand family (get/send/
  receive/watch) eventually replaces/reimplements the standalone helper;
  bridge coverage lands in the fleet book; the helper stays minimal;
  "This item is a follow-up for the fleet book / PM-book programme to
  pick up, not work for this issue." -> filed as #13907 by this lane.

## Fleet book bridge chapters — audit record

- concepts/bridge.md + reference/bridge.md: match live help + source on
  verbs, routes, defaults ($FLEET_AGENT / [bridge] agent;
  $FLEET_AGENT_GROUPS / [bridge] groups; $FLEET_BRIDGE_URL / [bridge] url
  / https://bridge.security.cab), secret names
  (message-bridge.access-token, message-bridge.fleet-token,
  [bridge] owner_token_name), HMAC derivation + label, exit codes
  0/2/3/4, token-in-path scrubbing, http-only-localhost rule.
- tutorials/bridge-messaging.md: two stale items (see 00 notes D4),
  fixed in PR #13915. All other steps verified consistent.
- Book build: pkgs/fleet/book.toml (title fleet, src book,
  create-missing=false); docs.nix link-escape rewrites ../ links to repo
  URLs at two depths; siteUrl default "/", deploy "/code/fleet/".
