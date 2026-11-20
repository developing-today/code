# Message bridge Worker (`message-bridge`)

A remote MCP server that Claude and ChatGPT both use as a **custom
connector**, plus a page where Drew sends a message to Claude, ChatGPT
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
  to one agent or several, but the MCP tools hand them to the agent
  without a `sender`; the page and the JSON API still show who sent
  them. `read_messages(kind="instruction")` returns only instructions,
  and the counts include `unread_instructions`. Agents can't send them.

The behaviour is deliberately minimal for now; more comes later.

## Routes

`<token>` is the owner's access token and `<chatgpt token>` the ChatGPT
connector's (below). A wrong or missing token gets the same plain 404 as
an unknown path; the ChatGPT token opens only its `/mcp`. The bare
address (`/` or `/mcp`, no token) is the same MCP server behind OAuth
instead: see "Adding it to Claude".

| Route                        | What                                                                                                                                                                                    |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /mcp/<token>`          | MCP server on Claude's channel, Streamable HTTP, stateless JSON answers (`GET` is 405: no server stream)                                                                                |
| `POST /mcp/<chatgpt token>`  | The same MCP server on ChatGPT's channel                                                                                                                                                |
| `GET /p/<token>`             | History page: send form + every message, newest first; refreshes every 10 s                                                                                                             |
| `POST /p/<token>`            | Form post (`body`, optional `sender`, one `to` per recipient, `kind`): stores the inbound messages, redirects to the page                                                               |
| `GET /api/<token>/messages`  | JSON history: `direction`, `channel` (`claude`/`chatgpt`/`all`), `kind`, `since_id`, `before_id`, `unread_only`, `limit`, `order`                                                       |
| `POST /api/<token>/messages` | JSON `{"body", "sender", "to", "kind"}`; `to` is `"claude"` (default), `"chatgpt"` or both as a list: one inbound row per recipient; `kind` is `"message"` (default) or `"instruction"` |
| `POST /api/<token>/ack`      | JSON `{"up_to_id": N}` or `{"ids": [...]}`, with `channel` (default `claude`): marks inbound messages read                                                                              |
| `GET /healthz`               | `ok`, `configured`, `chatgpt_configured`, `version`, `token_fingerprint`; no data                                                                                                       |
| `POST /`, `POST /mcp`        | The MCP server with an OAuth bearer token (channel from the approval); 401 with `resource_metadata` without one                                                                         |
| `/.well-known/oauth-*`       | Protected-resource and authorization-server metadata (also under `/mcp` and as `openid-configuration`)                                                                                  |
| `POST /register`             | Dynamic client registration: https (or localhost) redirect URIs, public or secret clients                                                                                               |
| `GET/POST /authorize`        | Approval page (PKCE S256 only): one tap in a browser that opened the history page, otherwise paste the page link or a token                                                             |
| `POST /token`                | `authorization_code` (with `code_verifier`) and `refresh_token` grants                                                                                                                  |

## MCP tools

- `send_message(body, sender?)` stores an outbound message and returns it.
- `read_messages(direction="inbound", since_id?, kind?, unread_only?, limit?, mark_read?)`
  returns messages oldest first, `next_since_id` for polling, and counts.
  Instructions come without their `sender`.
- `ack_messages(up_to_id? | ids?)` marks inbound messages read.

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
derived from it, and nothing else is stored (client ids, codes and
tokens are HMAC-signed). Approving needs the owner's token, its page
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
