# One-time Muse credential handoff

This service passes the initial BWS access token and 1Password service-account
token to one new Muse lane. It stores only AES-GCM ciphertext in a Durable Object.
The opaque claim secret appears only in the URL fragment, is sent to the Worker
in a POST body, expires after 15 minutes, and can be consumed once. Responses
are non-cacheable; Worker request logs are disabled.

## Deployment and link creation

Worker code releases use `.github/workflows/muse-handoff-deploy.yml`.
Changes merged to `main` upload an immutable Worker Version through the
pinned `cf` CLI and Cloudflare Workers Versions API; this upload does not
create a deployment or move production traffic. The workflow intentionally
has no pull-request trigger, and it passes Cloudflare credentials only to
jobs running on `main`.
Configure the repository secrets `MUSE_HANDOFF_CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID` before enabling deployment. Create a dedicated API
token with `Individual Workers Editor` scoped to this Worker; do not reuse
an existing shared token until its scope is verified. The workflow needs
only `contents: read` in GitHub and no OIDC permission.

Before promotion, inspect the uploaded version and verify it matches the
currently serving Worker: the `muse-agent-handoff` target, compatibility
date, workers.dev setting, disabled preview URLs and observability, the
`HANDOFFS` binding to `Handoff`, SQLite storage and the existing `v1`
migration, plus the existing `ADMIN_TOKEN` and `HANDOFF_ENCRYPTION_KEY`
secret bindings. The workflow never reads or changes those secret values.
The current config disables preview URLs, and Cloudflare does not issue
Version URLs for Workers that implement Durable Objects. A safe runtime
parity check therefore needs an isolated staging Worker with its own
Durable Object namespace; do not test a candidate against the production
namespace or promote it based only on the workflow checkbox. That staging
path is a cutover prerequisite and is intentionally not created here. The
current production deployment stays live until isolated parity is verified.
After that, dispatch the workflow on `main`, provide the uploaded version
ID, and confirm the parity review. It then creates a separate 100%
deployment for that version.

The existing local `deploy.sh` remains the initial Worker bootstrap and
secret-provisioning path described below; it is not the GitHub Actions
routine code-release path. It still invokes Wrangler deploy as part of that
combined local bootstrap/secret-provisioning flow; this change leaves that
path intact. Do not use it for routine code releases. The Actions workflow
assumes the Worker already exists.

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
