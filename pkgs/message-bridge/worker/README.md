# Message bridge Worker (`message-bridge`)

A remote MCP server that Claude and ChatGPT both use as a **custom
connector**, plus a page where the user sends a message to Claude, ChatGPT
or both and sees everything sent and received. It lives at
`https://bridge.security.cab` (a Workers custom domain; the
`workers.dev` address answers too). Every message is stored, with its
channel, direction, source, sender and time, in one SQLite-backed
Durable Object.

- **Channels:** `claude` or `chatgpt`. Both connectors run the same
  tools over the same store, each on its own channel with its own token,
  so neither sees or acknowledges the other's messages. ChatGPT reaches
  it either directly (`/mcp/<chatgpt token>`) or through its private
  Site, `pkgs/dot-bridge/site`, which keeps the Site's own address and
  sign-in and passes each request on.
- **Outbound** messages are pushed by an assistant through its connector
  (`send_message`).
- **Inbound** messages are sent to an assistant from the page (or the
  JSON API), one stored row per recipient. They start unread; the
  assistant reads them with `read_messages` and marks them read with
  `mark_read` or `ack_messages`.
- **Instructions** are inbound messages with `kind` `"instruction"`
  (plain ones are `"message"`). They are sent and targeted the same way,
  to one agent or several, but whoever receives one gets it without its
  sender (no `sender`, no `source`, `from_addr` `null`), whoever passed it
  on, and treats it as the user's; the page and the JSON API still show who
  sent it, and `reply_to` still answers the real sender. `read_messages(kind="instruction")` returns only instructions,
  and the counts include `unread_instructions`.
- **Any to any:** an assistant or a fleet agent can send a message or an
  instruction to any other, not only to the user (see "Any to any" below).
  Each such row carries `from_addr`, the sender's address
  (`channel:claude`, `channel:chatgpt`, `agent:<name>`); it is `null` for
  everything the user sends, and recipients see it on messages only. The
  recipient is woken where it can be.

## Routes

`<token>` is the owner's access token and `<chatgpt token>` the ChatGPT
connector's (below). A wrong or missing token gets the same plain 404 as
an unknown path; the ChatGPT token opens only its `/mcp`. The bare
address (`/` or `/mcp`, no token) is the same MCP server behind OAuth
instead: see "Adding it to Claude".

| Route                                  | What                                                                                                                                                                                    |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /mcp/<token>`                    | MCP server on Claude's channel, Streamable HTTP, stateless JSON answers (`GET` is 405: no server stream)                                                                                |
| `POST /mcp/<chatgpt token>`            | The same MCP server on ChatGPT's channel                                                                                                                                                |
| `GET /p/<token>`                       | History page: send form + every message, newest first; refreshes every 10 s                                                                                                             |
| `POST /p/<token>`                      | Form post (`body`, optional `sender`, one `to` per recipient, `kind`): stores the inbound messages, redirects to the page                                                               |
| `GET /api/<token>/messages`            | JSON history: `direction`, `channel` (`claude`/`chatgpt`/`all`), `kind`, `since_id`, `before_id`, `unread_only`, `limit`, `order`                                                       |
| `POST /api/<token>/messages`           | JSON `{"body", "sender", "to", "kind"}`; `to` is `"claude"` (default), `"chatgpt"` or both as a list: one inbound row per recipient; `kind` is `"message"` (default) or `"instruction"` |
| `POST /api/<token>/ack`                | JSON `{"up_to_id": N}` or `{"ids": [...]}`, with `channel` (default `claude`): marks inbound messages read                                                                              |
| `GET /api/<token>/pipeline`            | One normalized feed over connector messages, fleet messages and notes, real senders shown: `kind`, `sender`, `receiver`, `surface`, `batch`, `unread_only`, `work_state`, `since_id`, `before_id`, `limit`, `order`; rows carry the instruction work-item lifecycle (`work_state`, `work_task_id`) |
| `GET /api/<token>/notes`               | Owner read over the notes table (the same filters as the pipeline); rows carry recipient, sender, body and ack state                                                                     |
| `GET /api/<token>/agents`              | Unified connected-agents list: fleet registry (groups, first/last seen, unread counts) and note identities (scopes, signal counter), each row marked by surface; notes have no presence |
| `GET /api/<token>/oauth-events`        | The last 100 connector sign-in events, newest first (registrations with the client's requested metadata, approvals, refusals with the reason); never a secret                           |
| `GET /api/<token>/event-subscriptions` | ChatGPT's live event subscriptions: id, channel, callback host, created, expires; never the callback path or secret                                                                     |
| `GET /api/<token>/wake-hooks`          | Wake hooks: address, kind, host, whether a token is set, when added; never the token or the URL's path                                                                                  |
| `POST /api/<token>/wake-hooks`         | JSON `{"address", "url", "token"}` sets the hook for an address; `{"address", "url": null}` removes it                                                                                  |
| `POST /notes/register`                 | Owner Bearer token: JSON `{"identity", "token", "can_send", "can_inbox"}` registers a class/name note identity; only the token's SHA-256 hash is stored                                 |
| `POST /notes/send`                     | Identity Bearer token: JSON `{"to", "note"}`; `to` is one identity or a wildcard in either segment (`muse/*`, `*/archer`, `*/*`)                                                        |
| `GET /notes/inbox`                     | Identity Bearer token: that identity's unacknowledged notes                                                                                                                             |
| `POST /notes/ack`                      | Identity Bearer token: JSON `{"id"}`, `{"ids": [...]}` or `{"up_to_id": N}` acknowledges its own notes                                                                                  |
| `POST /notes/work`                     | Identity Bearer token: JSON `{"id", "state", "task_id"?}` records the work-item lifecycle of one of its own notes: `acted` \| `queued` (needs `task_id`) \| `done`, forward only from `received` |
| `GET /<class>/<name>`                  | Public wake signal: one bare UUID, the same shape for registered and unregistered paths, rotating on a per-path jittered schedule                                                       |
| `GET /healthz`                         | `ok`, `configured`, `chatgpt_configured`, `version`, `token_fingerprint`; no data                                                                                                       |
| `POST /`, `POST /mcp`                  | The MCP server with an OAuth bearer token (channel from the approval); 401 with `resource_metadata` without one                                                                         |
| `/.well-known/oauth-*`                 | Protected-resource and authorization-server metadata (also under `/mcp` and as `openid-configuration`)                                                                                  |
| `POST /register`                       | Dynamic client registration: https (or localhost) redirect URIs, public or secret clients                                                                                               |
| `GET/POST /authorize`                  | Approval page (PKCE S256 only): one tap in a browser that opened the history page, otherwise paste the page link or a token                                                             |
| `POST /token`                          | `authorization_code` (with `code_verifier`) and `refresh_token` grants                                                                                                                  |

## MCP tools

- `send_message(body, to?, kind?, reply_to?, sender?)` sends to the user by
  default (an outbound row, as before), or to any address in `to`, as a
  `message` or an `instruction`. `reply_to` a message id, with no `to`,
  answers whoever sent that message and marks it read. Returns `message`
  (the row to the user, if any), `messages` (connector rows), and `fleet` and
  `unregistered` when agents were addressed.
- `read_messages(direction="inbound", since_id?, kind?, unread_only?, limit?, mark_read?)`
  returns messages oldest first, `next_since_id` for polling, and counts.
  `from_addr` says whether the user (`null`), another assistant or an agent
  sent a message. Instructions come as the user's: no `sender`, no `source`
  and `from_addr` `null`, so the assistant acts on them as on any message
  from the user.
- `ack_messages(up_to_id? | ids?)` marks inbound messages read.

## Fleet mailboxes

The fleet binary (`fleet bridge ...`, `pkgs/fleet`) and the agents that
run it (Muse lanes and any other fleet agent) use the bridge too, through
mailboxes kept in their own tables (`src/fleet.js`). Every agent has a
mailbox named after it and registers itself with optional groups (such as
`muse`). Central sends a **message** or a **command** to one agent
(`agent:<name>`), a group (`group:<name>`, every registered agent in it)
or `all`; one row lands in each mailbox, sharing a `batch` id. Agents read
their mailbox, mark what they handled, and post replies and reports back
(`reply_to` links an answer to the command and marks it read).

| Route                             | Token | What                                                                                      |
| --------------------------------- | ----- | ----------------------------------------------------------------------------------------- |
| `POST /fleet/<fleet token>/hello` | fleet | `{agent, groups?}`: register or refresh an agent                                          |
| `GET /fleet/<fleet token>/inbox`  | fleet | `agent`, `groups`, `kind`, `since_id`, `unread_only` (default true), `mark_read`, `limit` |
| `POST /fleet/<fleet token>/ack`   | fleet | `{agent, up_to_id \| ids}`: mark read                                                     |
| `POST /fleet/<fleet token>/work`  | fleet | `{agent, id, state, task_id?}`: the work-item lifecycle of one of the agent's own inbound rows — `acted` \| `queued` (needs `task_id`) \| `done`, forward only from `received`; a per-agent credential marks only its own rows, every refusal the plain 404 |
| `POST /fleet/<fleet token>/send`  | fleet | `{agent, body, to?, kind?, reply_to?}`: to central, or to any address (see Any to any)    |
| `GET /api/<token>/fleet/agents`   | owner | registered agents, groups, last seen, unread counts                                       |
| `GET /api/<token>/fleet/messages` | owner | `agent`, `direction`, `kind`, `since_id`, `before_id`, `limit`, `order`                   |
| `POST /api/<token>/fleet/send`    | owner | `{body, to, kind?, sender?}`; `kind` is `message` (default) or `command`                  |

The **fleet token** opens only `/fleet/...`. It is derived from the access
token (HMAC-SHA256 of `ACCESS_TOKEN` over the label
`message-bridge/fleet-token/v1`, base64url), so it needs no Worker secret
and rotating the access token rotates it. A Worker secret `FLEET_TOKEN`
(at least 32 characters, distinct from the others) replaces it if ever
needed. It lives in Bitwarden as `message-bridge.fleet-token`; `fleet
bridge` derives it from the owner token when that secret is missing. Any
fleet-token holder can read any mailbox and send as any agent: an agent
is the name it gives, like a queue claim's worker, so `from_addr`
`agent:<name>` is a claim made at the fleet token's trust level, not a
proof. Because receivers treat every instruction as the user's, anything
holding the fleet token (or the bridge's own tokens) can send what reads
as the user's word to any connector or agent; per-agent tokens would narrow
that. An agent's inbox shows a command with `sender` and `from_addr`
`null`, like one from central.

Commands reach agents by mailbox rather than by webhook. The event
wake-ups below push to a callback URL; fleet agents have none (Muse lanes
are cron-started runs in a sandbox with no inbound address), so they wait
with `fleet bridge inbox --wait <secs>`, which polls the mailbox until
something arrives. Unread rows and `ack` are the replay that webhook
delivery lacks: a command sent while an agent was asleep is still there on
its next run. Rows carry the same fields as the `message.created` payload
(`id`, `kind`, `sender`, `body`, `ts`), so an agent that does have an https
endpoint can later get a `fleet.message.created` event from the same
delivery code without a new message shape.

## Class/name notes and the public wake signal

Alongside the connector channels and fleet mailboxes, the bridge keeps
a deliberately small note system (`src/notes.js`) for Drew and agents
to leave each other notes. An identity is a class and a name —
`muse/archer` is Archer. A registered identity has its own bearer
token; the Worker stores only the token's SHA-256 hash, mapped to the
identity and to what the token may do (send, use the inbox, or both).
Token values live in the vault by name: `bridge.send-token` is Drew's
sending token (identity `user/drew`) and `bridge.archer-token` is
Archer's (identity `muse/archer`). Registration is the one
administrative route: `POST /notes/register` with the owner's existing
access token as a Bearer token and a JSON body of
`{"identity", "token", "can_send", "can_inbox"}`.

A sender posts `{"to", "note"}` to `/notes/send`. `to` names one
identity, or uses `*` for either segment: `muse/*` reaches every
registered Muse identity, `*/archer` every registered identity named
Archer, and `*/*` every registered identity. One row lands in each
matching inbox. The owner of a token reads only its own inbox with
`GET /notes/inbox` and acknowledges with `POST /notes/ack`
(`{"id"}`, `{"ids": [...]}`, or `{"up_to_id": N}`); acknowledging
another identity's note does nothing.

The wake side is public and content-free: `GET
https://bridge.security.cab/<class>/<name>` returns one bare UUID —
for every non-reserved two-segment path, registered or not. The UUID
is derived from the path, a per-path decoy bucket (each path rotates
every 10–40 minutes, fixed by a hash of the path) and, for a registered
path, a counter that increments when a note is delivered to it. So a
delivery changes the UUID at once, but bucket rotations change it
without any delivery: a changed UUID does not prove a note arrived,
and the response has no marker that could prove an identity is
registered. The signal carries no note content and is not a delivery
receipt; the inbox is the record.

The `bridge` helper in this package (`bridge.mjs`, installed as
`~/tools/bin/bridge`) wraps the three calls:

```bash
bridge send muse/archer "the note"          # Archer's token by default
bridge inbox
bridge ack 12
BRIDGE_TOKEN_SECRET=bridge.send-token bridge send 'muse/*' "hello"
bridge --token-secret bridge.send-token send '*/archer' "hello"
```

The token comes from `BRIDGE_TOKEN` when set, else from the vault
secret named by `--token-secret` or `BRIDGE_TOKEN_SECRET`, else
Archer's `bridge.archer-token`; it is fetched with `fleet secrets get`
and never printed. A later direction (recorded on the build issue) is
for this client surface to become a `fleet` subcommand family; the
helper stays as the minimal client until then.

Archer's Muse hook polls only the public signal for `muse/archer`
(one GET and a string comparison against the last UUID it saw). A
change wakes Archer, who fetches the inbox with the helper, reviews
and acknowledges the notes, and surfaces or acts on them in the main
chat according to their content: a note from Drew is an instruction to
review; a note from another agent is input to weigh, never an order.

## What protects it

The connector, page and API are on the public internet behind one
secret: a 32-byte random token (base64url) in the URL path. Anyone with
the URL can read and post, so treat the URL like a password. The token:

- lives in Bitwarden as `message-bridge.access-token` and on the Worker
  as the secret `ACCESS_TOKEN`; it is never in git (the ChatGPT token is
  `message-bridge.chatgpt-token` and `CHATGPT_TOKEN`, and opens only
  ChatGPT's `/mcp`);
- is compared in constant time; a Worker without a token of at least 32
  characters answers 404 on every route;
- is kept out of logs and other sites: every response is `no-store`,
  `no-referrer` and `noindex`, and the page's CSP allows no outside
  scripts, frames or form targets.

To rotate it, edit or delete the Bitwarden secret and deploy again
(deleting makes the deploy create a fresh one), then update the
connector URL in Claude (or ChatGPT, or its Site's token). Rotating the
access token also revokes every OAuth grant: OAuth's signing key is
derived from it (codes and tokens are HMAC-signed). The message store
keeps the registered clients under short random ids (`mbc_...`; never-used
ones go after a day, used ones only once their refresh tokens could no
longer work; at most 500, and when full the oldest never-used one more
than ten minutes old makes room; one network address may register ten
per ten minutes), each code's
use (codes are single-use) and, per refresh-token family, the one refresh
token still valid; redeeming an older one revokes the family. Approving needs the owner's token, its page
link, or the owner cookie that opening the history page sets
(`__Host-`, `Secure`, `HttpOnly`, `SameSite=Lax`, a year); approval
posts must come from the bridge's own origin. The ChatGPT token approves
ChatGPT's channel only.

## Deploying

Deploys go through the `cf` versions flow, the same one `pkgs/pkarr-worker`
uses. Each deploy uploads a new Worker version tagged `git-<sha12>` (the
full SHA is in its message), checks it is deployable and still carries
`ACCESS_TOKEN`, deploys it at 100%, then checks the deployment serves it
and `/healthz` reports the token. Older versions stay, so rolling back is
a deployment at an older version id with no upload. Every deploy also
keeps `PUBLIC_URL`'s host attached to the Worker as a Workers custom
domain (Cloudflare makes its DNS record and certificate) and waits for it
to answer; the deploy checks themselves use the `workers.dev` address.

`cloudflare.config.ts` is the deploy config; `wrangler.jsonc` drives
`wrangler dev` and must stay in step (a test checks the main fields).

From a checkout, with Bitwarden and sops access:

```bash
nix run .#message-bridge-deploy -- --dry-run                 # build and check, upload nothing
nix run .#message-bridge-deploy                              # upload a version with ACCESS_TOKEN, deploy it
nix run .#message-bridge-deploy -- --deploy-version <id>     # roll back (add --dry-run to preview)
nix run .#message-bridge-deploy -- --print-urls              # connector/page URLs (contain the token)
```

The first full deploy creates the access token in Bitwarden if it is
missing. In an account where the Worker doesn't exist yet, it bootstraps
with `cf deploy` (the versions flow can't create a Worker), which uploads
and deploys the first version in one step; every later deploy uses the
versions flow. `--code-only` refuses to bootstrap, since it has no token
to give the new Worker. Version ids are in the Cloudflare dashboard (Worker > Deployments)
or in each deploy's output. Cloudflare blocks rolling back to a version
uploaded before a secret changed (such as a rotated token); add
`--allow-secret-change` to `--deploy-version` to deploy it anyway, and
the connector then uses that version's token. A rollback never goes to a version
from before the ChatGPT channel (one without `CHATGPT_TOKEN`) while the
live one has it, since that older code would show ChatGPT's messages on
Claude's connector.

### From the repo

`.github/workflows/message-bridge-deploy.yml` deploys on every push to
`main` that touches `pkgs/message-bridge/`, and by hand from the Actions
tab. It builds and tests the flake package, then runs
`nix run .#message-bridge-deploy -- --code-only` with the
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` repository secrets. A
code-only version keeps the deployed `ACCESS_TOKEN`, so the connector URL
stays the same and the runner needs no Bitwarden or sops access. It
refuses to upload when a version newer than the live one came from a
full deploy (say one that uploaded a rotated token and then failed),
since the new version would inherit that token instead. It checks again
after the upload, so a full deploy that lands in between leaves the new
version undeployed. Cloudflare numbers versions in upload order, so that
check waits until the list shows every version numbered between the
live one and the new one, and refuses if it never does. After promoting,
every deploy waits for `/healthz` to answer from the new version, and a
code-only deploy checks that it reports the same token fingerprint as
the version it replaced (`/healthz` shows a 16-hex SHA-256 fingerprint of
the token, never the token). A code-only deploy refuses to promote when
the live version reports no fingerprint or the live version can't be
read, and fails if another deployment went live between its checks and
its promotion (then that deployment goes back live). If promoting or any
check after
it fails (a token mismatch, or the new version never answering), the
previous version goes back live at once (unless a newer deployment than this
run's own, even one that splits traffic or redeploys the same version,
has gone live, which it then leaves alone) and the deploy fails, so the
token can't change silently whatever the versions list showed. A full deploy from a
checkout clears the condition. A manual
run with a `rollback_version` deploys that existing version instead,
and fails if `/healthz` keeps answering from any other version.
Without the two secrets the workflow builds, tests and skips the upload.
Setting or rotating the access token is still a local full deploy.

Tests: `node --test test/*.test.mjs` (also run by the flake package's
checkPhase).

## Any to any

One address grammar (`src/relay.js`) for every sender: `send_message`'s
`to`, and `to` on an agent's `POST /fleet/<fleet token>/send`. A list or a
comma-separated string, case-insensitive.

| Address                         | Reaches                                                                     |
| ------------------------------- | --------------------------------------------------------------------------- |
| `drew` (or `owner`, `central`)  | Drew: an outbound row on a connector's channel, or a post to central        |
| `claude`, `chatgpt`             | that connector (also `channel:claude`, `channel:chatgpt`)                   |
| `connectors`                    | every other connector                                                       |
| `agent:<name>` (or a bare name) | that agent's mailbox (registered or not; unregistered ones are listed back) |
| `group:<name>`                  | every registered agent in that group                                        |
| `agents`                        | every registered agent (an error if none has registered)                    |
| `all`                           | every other connector and every registered agent                            |

- The sender never gets its own message: `all`, `connectors` and groups
  leave it out, and naming yourself is an error.
- `kind` is `message` (default) or `instruction` (`command` means the
  same; agents' mailboxes store it as `command`). The user gets messages only.
- With no `to`, a connector's send goes to the user and an agent's to
  central, exactly as before. `reply_to` with no `to` answers whoever sent
  that row: back to the user/central when the user sent it, otherwise to the
  `from_addr` connector or agent.
- Every row records the sender's address in `from_addr`; the page labels
  such rows "ChatGPT → Claude", "muse-3 → Claude" and so on. The
  recipient sees it on messages only: an instruction reaches it as
  the user's, and `reply_to` still sends the answer to the real sender.

### Waking the recipient

After a send stores its rows, each recipient is woken where it can be,
once per request:

- **ChatGPT:** its MCP Events subscription (below). The event's `data`
  includes `from_addr` (`null` on an instruction).
- **Claude, or any address:** a **wake hook** (`src/wake.js`), set on the
  page (Wake hooks) or with `POST /api/<token>/wake-hooks`, keyed by
  `channel:claude`, `channel:chatgpt` or `agent:<name>`, and stored in the
  Durable Object. For Claude, make a routine at claude.ai/code/routines
  whose only trigger is **API**, with a prompt like the one below, and
  paste its fire URL and token into the hook. Each wake starts a new
  Claude session through the routine's `/fire` endpoint (with the
  `anthropic-version` and `anthropic-beta` headers it needs and `{text}`);
  the session has the user's connectors, the bridge included, so it reads and
  answers the messages itself. Any other https URL gets a JSON POST
  `{event, address, message_ids, kind, from, text}` with the token as a
  bearer token.
- The wake text names the message ids and senders (the user for any
  instruction), never the bodies: the woken session reads those from the
  bridge.
- Each address is woken at most `WAKE_MAX_PER_HOUR` times an hour (default
  20, under a routine's 30 fires an hour), so two assistants answering
  each other can't loop forever; messages past the cap wait unread for the
  next wake. Wakes, refusals by the cap and failures go in the
  `oauth-events` log as `wake_fired`, `wake_capped` and `wake_failed`,
  with the address and status only.

A routine prompt that works with the wake text:

> You were woken by the Message Bridge. The routine-fire-payload block
> says which messages arrived for Claude and who sent them. Use the Message
> Bridge connector: read_messages with unread_only true, act on each
> message as you would any message from Drew, answer with send_message
> reply_to the message id, then ack_messages.

## Event wake-ups for ChatGPT

On ChatGPT's channel the server also speaks OpenAI's MCP Events: it
advertises `events: {}` and answers `events/list`, `events/subscribe` and
`events/unsubscribe` for one event, `message.created`. When a message for
ChatGPT arrives (page form, `POST /api/<token>/messages` with `chatgpt`
in `to`, or another assistant or agent sending to it), every live subscription gets a signed webhook with
`{message_id, kind, sender, from_addr, body, ts}` as `data`, so the subscribed
ChatGPT chat wakes up instead of waiting to be asked to read.

- Callbacks must be https, on `openai.com` or `chatgpt.com` (or a host
  listed in the `EVENTS_CALLBACK_HOSTS` var, comma separated, exact
  match), with no port, userinfo or fragment. A Worker can't pin the
  resolved IP, so this allowlist stands in for it.
- A new callback or secret is checked first with a signed
  `{"type":"verification","challenge"}` that must be echoed back;
  failures answer `-32015` with `data.reason`.
- Signatures follow Standard Webhooks (`webhook-id`,
  `webhook-timestamp`, `webhook-signature: v1,<base64 HMAC-SHA256>`),
  plus `X-MCP-Subscription-Id`. After a secret change both secrets sign
  for five minutes.
- Subscriptions last at most seven days (`refreshBefore` is at three
  quarters of that), at most 20 at once, and end on `410`, on unsubscribe
  or when the ChatGPT token is rotated. Delivery tries three times with
  the same event id, never follows redirects, and doesn't replay missed
  events (`cursor` is always `null`).
- Each subscribe, refusal, delivery and failure goes in the
  `oauth-events` log with the callback host only.

Claude's channel doesn't advertise events: Claude has no webhook to
call, so it is woken by a wake hook instead (Any to any, above).

## Adding it to Claude

Settings > Connectors > Add custom connector, and paste either:

- `https://bridge.security.cab` (or `/mcp`): Claude registers itself and
  opens the bridge's approval page. Tap Allow if this browser has opened
  the history page, otherwise paste the page link first. Claude refreshes
  its token on its own; access tokens last a day, refresh tokens a year
  from their last use.
- the `connector:` URL that `--print-urls` prints, with the token in the
  path: no sign-in step.

No OAuth client id or secret is needed either way.

If connecting fails, `GET /api/<token>/oauth-events` shows how far the
sign-in got and why it stopped.
