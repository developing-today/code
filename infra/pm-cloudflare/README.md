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
- `apply.py` — enforces the state through Cloudflare's API using the
  workflow-owned `cloudflare.py` module, in gate-first order. `--check`
  (the default) reports and changes nothing; `--apply` makes the changes.
  It keeps nothing on the machine it runs on: no console file, no unit,
  no remembered markers. The state file is the memory.
- `cloudflare.py` — Cloudflare API helpers for the repository applier;
  the laptop script does not import them or manage repository-owned state.
- `test_apply.py` — checks the declared state and applier against an
  independent account model. Tunnel ingress is tested separately with
  `pm-stack/console/scripts/test_expose.py`.

Run the model and boundary tests:

```sh
python3 -I infra/pm-cloudflare/test_apply.py
python3 -I pm-stack/console/scripts/test_expose.py
```

## Secrets (repository secrets, never in the repo)

| Variable                                              | Repository secret              | What it is                                                                            |
| ----------------------------------------------------- | ------------------------------ | ------------------------------------------------------------------------------------- |
| `PMCF_CLOUDFLARE_TOKEN`                               | `PMCF_CLOUDFLARE_TOKEN`        | the Cloudflare API token; source it from `cloudflare.api-token-account-wide`          |
| `PMCF_OWNER_EMAIL`                                    | `PMCF_OWNER_EMAIL`             | whose sign-in is let in                                                               |
| `PMCF_GITHUB_TOKEN`                                   | `PMCF_GITHUB_TOKEN`            | the Worker automation token; source it from `github.developing-today-code-automation` |
| `PMCF_QUEUE_TOKEN`                                    | `PMCF_QUEUE_TOKEN`             | the Worker queue store token; source it from `turso.fleet-queue-rw-token`             |
| `PMCF_BRIDGE_TOKEN`                                   | `PMCF_BRIDGE_TOKEN` (optional) | the PM stack bridge token; source it from `bridge.pm-stack-token`                     |
| `PMCF_GOOGLE_CLIENT_ID` / `PMCF_GOOGLE_CLIENT_SECRET` | same names                     | Google sign-in (optional)                                                             |

Google sign-in credentials must be supplied as a pair. When both are
omitted, an existing Google identity provider is preserved; the applier
never sends empty credentials to Cloudflare.

## Ownership boundary

The laptop timer's `expose.py` manages only the named tunnel's ingress
for its local console. This workflow remains the sole applier for the
Access application and policy, DNS, Worker route and Worker settings.

The app's Cloudflare screen says where the configuration was last
applied from: the latest successful dispatched run of this
workflow on main, with its commit, completion time and a link to
the run (the console's `cloudflare-apply` read). It says provenance
is unavailable when no successful apply is on record.
