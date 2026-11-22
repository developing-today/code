# One-time Muse credential handoff

This service passes the initial BWS access token and 1Password service-account
token to one new Muse lane. It stores only AES-GCM ciphertext in a Durable Object.
The opaque claim secret appears only in the URL fragment, is sent to the Worker
in a POST body, expires after 15 minutes, and can be consumed once. Responses
are non-cacheable; Worker request logs are disabled.

## Deploy once, then create one link per agent

From a trusted checkout on the laptop:

```bash
scripts/muse/handoff/deploy.sh
python3 scripts/muse/handoff/create-url.py
```

The deploy script reads the scoped `cloudflare.api-token-day-to-day` secret
through the existing BWS login. It stores a separate local admin token and
encryption key under `${XDG_CONFIG_HOME:-~/.config}/muse-agent-handoff`,
with mode 0600. They are not committed. The creator prompts for BWS and OP
tokens with hidden input and prints only an agent name plus a 15-minute URL.
Keep the URL for the prompt that starts that one lane; do not reuse it.

To rotate/redeploy the Worker, run `deploy.sh` again. It reuses the local
admin and encryption keys, so already-issued links remain decryptable until
they expire. If the admin file is lost, provision a new admin token and update
the Worker secret before issuing another link.

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
- `deploy.sh`: deploy and configure Worker secrets without writing them to Git.
- `create-url.py`: laptop-side hidden-input URL generator.
- `enroll-agent.py`: Muse-side handoff consumer and bootstrap runner.
- `PROMPT.md`: copy-ready starting prompt for a new Muse lane.
- `../feedback/TEMPLATE.md`: record deployment/bootstrap issues and changes.
