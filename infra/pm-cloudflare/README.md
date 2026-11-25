# The PM stack's Cloudflare config, from the repository

Slice 1 of
[developing-today/code#13495](https://github.com/developing-today/code/issues/13495):
the hostname's route, its sign-in gate and the edge Worker's settings
are described in [`state.json`](state.json) and applied by
[`apply.py`](apply.py), by the workflow
`pm-cloudflare-config.yml`, with the repository's own secrets.

## What is here

- `state.json` — the declared account state: hostname, tunnel, the
  Access application and its owner policy, the five paths that bypass
  sign-in, DNS, the edge Worker route, and the Worker's secrets
  (names only; values come from the environment at apply time).
- `apply.py` — enforces the state through Cloudflare's API by calling
  the same functions `pm-stack/console/scripts/expose.py` uses on the
  laptop, in the same gate-first order. `--check` (the default)
  reports and changes nothing; `--apply` makes the changes. It keeps
  nothing on the machine it runs on: no console file, no unit, no
  remembered markers. The state file is the memory.
- `test_apply.py` — the state and the applier against a model of the
  account. The parity test holds the workflow and the laptop's timer
  to the same account state: for the same hostname and owner, a check
  run records exactly the changes `expose.py --check` records.

Run the tests: `python3 -I infra/pm-cloudflare/test_apply.py`.

## Secrets (repository secrets, never in the repo)

| Variable | Repository secret | What it is |
|---|---|---|
| `PMCF_CLOUDFLARE_TOKEN` | `PMCF_CLOUDFLARE_TOKEN` | the Cloudflare API token; source it from `cloudflare.api-token-account-wide` |
| `PMCF_OWNER_EMAIL` | `PMCF_OWNER_EMAIL` | whose sign-in is let in |
| `PMCF_GITHUB_TOKEN` | `PMCF_GITHUB_TOKEN` | the Worker automation token; source it from `github.developing-today-code-automation` |
| `PMCF_QUEUE_TOKEN` | `PMCF_QUEUE_TOKEN` | the Worker queue store token; source it from `turso.fleet-queue-rw-token` |
| `PMCF_BRIDGE_TOKEN` | `PMCF_BRIDGE_TOKEN` (optional) | the PM stack bridge token; source it from `bridge.pm-stack-token` |
| `PMCF_GOOGLE_CLIENT_ID` / `PMCF_GOOGLE_CLIENT_SECRET` | same names | Google sign-in (optional) |

Google sign-in credentials must be supplied as a pair. When both are
omitted, an existing Google identity provider is preserved; the applier
never sends empty credentials to Cloudflare.

## What this does not do yet (the follow-on)

- The laptop timer's `expose.py` still enforces all of this too.
  Removing it there — keeping only the laptop's tunnel — happens
  after this workflow's first apply is confirmed on #13495, not
  before, so the gate is never unenforced in between.
- The app Cloudflare screen does not say where each item was last
  applied from and when; that display reads the confirmation above.
