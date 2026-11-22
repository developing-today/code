# One-time Muse credential handoff

This service passes the initial BWS access token and 1Password service-account
token to one new Muse lane. It stores only AES-GCM ciphertext in a Durable Object.
The opaque claim secret appears only in the URL fragment, is sent to the Worker
in a POST body, expires after 15 minutes, and can be consumed once. Responses
are non-cacheable; Worker request logs are disabled.

## Deployment and link creation

Routine Worker releases use two manual, main-only workflows:
`.github/workflows/muse-handoff-upload.yml` and
`.github/workflows/muse-handoff-promote.yml`. Upload first builds the exact
source on the isolated `muse-agent-handoff-staging` Worker, deploys that
version there, validates the generated settings and bindings, then runs a
one-use create/claim/replay smoke using disposable fake credentials. The
staging Worker has its own Durable Object namespace. The workflow then
uploads the same source as an immutable production Worker Version and saves
the exact `cf workers versions create` response, source archive, configuration
evidence, smoke proof, and production traffic snapshot in a run-specific
artifact. It does not create a production deployment.

The separate promotion workflow accepts only the successful upload run ID.
It verifies the run came from the trusted main upload workflow, the commit is
still an ancestor of `main`, every artifact digest and the exact source
archive match, the version ID came from that upload response, and the stage
proof covers the same source. It also checks production still serves the
captured deployment and traffic split before creating a separate 100%
deployment. The workflow verifies Cloudflare's current deployment list serves
that exact version at 100% afterward. No Boolean input can stand in for
stage evidence. Production traffic remains unchanged until promotion passes
those checks and the protected production environment is approved.

Both workflows intentionally have no pull-request trigger. Cloudflare
credentials are available only through the `muse-handoff-staging` and
`muse-handoff-production` GitHub environments. Before enabling the flow,
configure both environments with required reviewers and these environment
secrets:

- `muse-handoff-staging`: `MUSE_HANDOFF_STAGING_CF_API_TOKEN` and
  `MUSE_HANDOFF_STAGING_ACCOUNT_ID`, plus the nonsecret environment variable
  `MUSE_HANDOFF_STAGING_WORKERS_DEV_HOST` set to the staging Worker’s full
  workers.dev hostname.
- `muse-handoff-production`: `MUSE_HANDOFF_PRODUCTION_CF_API_TOKEN` and
  `MUSE_HANDOFF_PRODUCTION_ACCOUNT_ID`.

Each token must be a dedicated Cloudflare API token with `Individual Workers
Editor`, scoped only to its respective Worker. Do not copy the existing
shared token or put credentials in repository-level secrets. The staging
Worker must be bootstrapped with the same `Handoff` SQLite class and a
separate namespace before dispatching upload. These environment protections,
scoped tokens, and staging Worker are cutover prerequisites; this change
does not configure repository settings, create secrets, or provision the
staging Worker. The workflows need only `actions: read` and `contents: read`
and do not grant OIDC permissions.

The existing production bindings/settings are retained: `muse-agent-handoff`,
compatibility date `2026-11-21`, workers.dev enabled, preview URLs and
observability disabled, the `HANDOFFS` binding to SQLite `Handoff`, and the
`ADMIN_TOKEN` and `HANDOFF_ENCRYPTION_KEY` secret bindings. The source contract
also guards the existing `v1` SQLite migration. Upload never reads secret
values. Cloudflare does not issue Version URLs for Workers that implement
Durable Objects, so staged parity is checked on the isolated Worker before
the production version can be promoted.

The existing local `deploy.sh` remains the initial Worker bootstrap and
secret-provisioning path described below; it is not the GitHub Actions
routine code-release path. It still invokes Wrangler deploy as part of that
combined local bootstrap/secret-provisioning flow; this change leaves that
path intact. Do not use it for routine code releases. Both Workers must
already exist before the Actions upload can run.

From a trusted checkout on the laptop, bootstrap the Worker and provision
its local admin/encryption secrets when needed:

```bash
scripts/muse/handoff/deploy.sh
python3 scripts/muse/handoff/create-url.py
```

The deploy script reads the existing BWS login token from
`~/.config/bitwarden/access-token` (Muse bootstrap) or
`~/.config/bws/access-token` (BWS CLI), then resolves the scoped
`cloudflare.api-token-day-to-day` secret. It stores a separate local admin token and
encryption key under `${XDG_CONFIG_HOME:-~/.config}/muse-agent-handoff`,
with mode 0600. They are not committed. The creator prompts for BWS and OP
tokens with hidden input and prints only an agent name plus a 15-minute URL.
When the OP token is already in BWS, use `--bws-token-file` and
`--op-from-bws-key` to resolve it locally without displaying it; for example:

```bash
python3 scripts/muse/handoff/create-url.py \
  --bws-token-file ~/.config/bws/access-token \
  --op-from-bws-key onepassword.service-account-token
```

Keep the URL for the prompt that starts that one lane; do not reuse it.

For local admin/encryption key rotation, run `deploy.sh` again. It reuses the
local admin and encryption keys, so already-issued links remain decryptable
until they expire. If the admin file is lost, provision a new admin token and
update the Worker secret before issuing another link. Routine code releases
use the GitHub Actions upload/review/promotion flow above.

## Agent command

Use the URL from the generator output and the generated name:

```bash
curl -fsSL https://raw.githubusercontent.com/developing-today/code/codex/muse-agent-enrollment-20261122/scripts/muse/handoff/enroll-agent.py -o /tmp/enroll-agent.py
python3 /tmp/enroll-agent.py --name <agent-name> --handoff-url '<one-time-url>'
```

The helper consumes the URL, saves both tokens to 0600 files, downloads the
pinned bootstrap script, runs all bootstrap phases, enrolls the lane, and runs
`fleet agent doctor`. If a phase fails, the token files remain and the helper
prints a resume command:

```bash
python3 /tmp/enroll-agent.py --name <same-agent-name> --resume
```

After successful enrollment, run `~/tools/fleet agent next`. Exit 5 means
there is no task available on this pass; it is not an enrollment failure.

## Security and operations

- Never pass OP/BWS token values as command arguments, prompt text, or environment
  shown in a shell transcript. The operator enters them in hidden prompts.
- The generated URL is a bearer claim code. Send it only to the intended lane,
  and use it within 15 minutes.
- The URL fragment is not sent in ordinary HTTP requests or access logs. The
  consumer posts the claim id and code in the request body; the Worker atomically
  removes the Durable Object record before returning credentials.
- A network failure after consumption can invalidate the link. Run the helper
  again with `--resume` if the local 0600 files were written; otherwise mint
  a fresh link.
- The handoff carries the OP token as well as BWS because Fleet's secrets
  provider can use either. Bootstrap installs/restores the existing `op`
  binary; it does not attempt an interactive 1Password login.

## Source files

- `worker/src/index.js`: Worker API and per-link Durable Object.
- `worker/wrangler.jsonc`: deployment binding and migration.
- `deploy.sh`: bootstrap the Worker and configure its secrets without writing
  them to Git.
- `create-url.py`: laptop-side hidden-input URL generator.
- `enroll-agent.py`: Muse-side handoff consumer and bootstrap runner.
- `PROMPT.md`: copy-ready starting prompt for a new Muse lane.
- `../feedback/TEMPLATE.md`: record deployment/bootstrap issues and changes.
