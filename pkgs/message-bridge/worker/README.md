# Message bridge Worker (`message-bridge`)

A remote MCP server that Claude can add as a **custom connector**, plus a
page where Drew sends messages to Claude and sees everything sent and
received. Every message is stored, with its direction, source, sender
and time, in one SQLite-backed Durable Object.

- **Outbound** messages are pushed by Claude through the connector
  (`send_message`).
- **Inbound** messages are sent to Claude from the page (or the JSON
  API). They start unread; Claude reads them with `read_messages` and
  marks them read with `mark_read` or `ack_messages`.

The behaviour is deliberately minimal for now; more comes later.

## Routes

`<token>` is the access token (below). A wrong or missing token gets the
same plain 404 as an unknown path.

| Route                        | What                                                                                                                         |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `POST /mcp/<token>`          | MCP server, Streamable HTTP, stateless JSON answers (`GET` is 405: no server stream)                                         |
| `GET /p/<token>`             | History page: send form + every message, newest first; refreshes every 10 s                                                  |
| `POST /p/<token>`            | Form post (`body`, optional `sender`): stores an inbound message, redirects to the page                                      |
| `GET /api/<token>/messages`  | JSON history: `direction` (`inbound`/`outbound`/`all`), `since_id`, `unread_only`, `limit` (max 200), `order` (`asc`/`desc`) |
| `POST /api/<token>/messages` | JSON `{"body": "...", "sender": "..."}`: stores an inbound message (source `api`)                                            |
| `POST /api/<token>/ack`      | JSON `{"up_to_id": N}` or `{"ids": [...]}`: marks inbound messages read                                                      |
| `GET /healthz`               | `{"ok":true,"configured":true}`, no data                                                                                     |

## MCP tools

- `send_message(body, sender?)` stores an outbound message and returns it.
- `read_messages(direction="inbound", since_id?, unread_only?, limit?, mark_read?)`
  returns messages oldest first, `next_since_id` for polling, and counts.
- `ack_messages(up_to_id? | ids?)` marks inbound messages read.

## What protects it

The connector, page and API are on the public internet behind one
secret: a 32-byte random token (base64url) in the URL path. Anyone with
the URL can read and post, so treat the URL like a password. The token:

- lives in Bitwarden as `message-bridge.access-token` and on the Worker
  as the secret `ACCESS_TOKEN`; it is never in git;
- is compared in constant time; a Worker without a token of at least 32
  characters answers 404 on every route;
- is kept out of logs and other sites: every response is `no-store`,
  `no-referrer` and `noindex`, and the page's CSP allows no outside
  scripts, frames or form targets.

To rotate it, edit or delete the Bitwarden secret and deploy again
(deleting makes the deploy create a fresh one), then update the
connector URL in Claude. OAuth can replace the path token later.

## Deploying

Deploys go through the `cf` versions flow, the same one `pkgs/pkarr-worker`
uses. Each deploy uploads a new Worker version tagged `git-<sha12>` (the
full SHA is in its message), checks it is deployable and still carries
`ACCESS_TOKEN`, deploys it at 100%, then checks the deployment serves it
and `/healthz` reports the token. Older versions stay, so rolling back is
a deployment at an older version id with no upload.

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
the connector then uses that version's token.

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
the token, never the token). If it doesn't, the previous version goes
back live at once and the deploy fails, so the token can't change
silently whatever the versions list showed. A full deploy from a
checkout clears the condition. A manual
run with a `rollback_version` deploys that existing version instead.
Without the two secrets the workflow builds, tests and skips the upload.
Setting or rotating the access token is still a local full deploy.

Tests: `node --test test/*.test.mjs` (also run by the flake package's
checkPhase).

## Adding it to Claude

Settings > Connectors > Add custom connector, and paste the `connector:`
URL that `--print-urls` prints. No OAuth client id or secret is needed.
