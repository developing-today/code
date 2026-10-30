# pm-dispatch

Authenticated intake for the fleet: launch an OpenCode session on Drew's
laptop **now**, or just enqueue work into the Supabase repo-health queues —
either way the work is tracked by a queue task and (usually) a GitHub
issue, so nothing runs untracked and no queue worker double-books it.

Lives in `pm-stack/dispatch/`. On the laptop (`amd`), the
`pm-dispatch.service` user unit is declared by the NixOS pm-stack
module (`nixos/services/pm-stack`) and runs from the primary code
checkout (`~/git/code/pm-stack/dispatch`), like the aggregator;
`install.sh` (below) is the fallback installer and still renders
the env files. Port 4879, loopback + tailnet bind only, plus the
public tunnel hostname (see Public reachability).

## Endpoints

### `GET /health`

Unauthenticated. `{ ok: true, status: "ok", version: <build version> }`
(the source commit of the deployed checkout).

### `POST /v1/sessions` — launch now

Launches an OpenCode session on the laptop's standing server
(127.0.0.1:13420) and assigns the tracking queue task immediately.

Body:

```jsonc
{
  "repo": "developing-today/code", // required, allowlisted fleet repo
  "prompt": "Fix the flaky widget test", // required
  "title": "fix(widget): flaky test", // optional; default: prompt's first line
  "model": "opencode-go/gpt-5.6-luna", // optional "provider/model" or {"providerID","id"}
  "directory": "/home/user/git/code", // optional; default: the repo's laptop primary
  "priority": "high", // optional: 10-90 or critical|high|normal|low → 10/20/40/70
  "issue": true, // optional; default true
  "wait": true, // optional; default true
}
```

Flow:

1. Create the tracking GitHub issue (`source/dispatch` label; body = the
   prompt + a dispatch footer). Skipped with a `warnings` entry for
   dezren39/nix, where the pm-stack GitHub App is not installed.
2. Enqueue a repo-task/v1 queue task (`small_fix` with the canonical
   `repo-health:v1:fix:<repo>:issue:<n>` dedupe key when an issue exists,
   else `triage` with a unique `repo-health:v1:triage:<repo>:dispatch:<rand8>`
   key). The payload carries `origin: "dispatch"`, the caller identity,
   the model hint, and the directory.
3. Create the OpenCode session (directory + model when given) and send
   the prompt, prefixed with `[dispatch] Queue task #<id> · Issue <url> ·
Repo <repo>` so the session knows its tracking ids.
4. **Assign immediately**: the task row is PATCHed to `status: claimed`,
   `worker_id: dispatch-opencode`, `claimed_at` now, `lease_until` +6h,
   `progress: {lane: "opencode", session_id, issue_number, launched_by}`.
5. Comment the task/session ids on the issue; label it `queue/claimed` +
   `work/assigned`.

Responses: `wait: true` → `201 {task_id, session_id, issue_number|null,
queue, status: "claimed"}` after the launch. `wait: false` → `202
{task_id, issue_number|null, status: "launching"}` right after the
enqueue; steps 3–5 finish asynchronously and any failure is PATCHed onto
the task's `last_error`.

If the launch fails after the enqueue (wait: true), the task is
cancelled via `cancel_repo_task`, the failure is noted on the issue, and
the response is `502 {task_id, error}` — a task is never left `queued`
for a session that launched, nor `claimed` without a session id.

### `POST /v1/tasks` — enqueue only

```jsonc
{
  "repo": "developing-today/opencode", // required
  "title": "investigate the red build", // required
  "prompt": "…", // required ("body" accepted as an alias)
  "kind": "triage", // optional: triage|small_fix|pr_shepherd|ci_investigation
  "priority": 40, // optional, same mapping as above
  "model": "deepseek/deepseek-chat", // optional; stored as payload.model_hint only
  "issue": false, // optional; default false
  "dedupe_key": "…", // optional; default: canonical key from the
  // created issue, else a unique dispatch key
}
```

Returns `201 {task_id, issue_number|null, queue}`. With `issue: true`
the issue is created first (labels `queue/queued` + `source/dispatch`)
and gets an agent-note comment naming the task id.

### Why assignment is a direct PATCH

The queue's `claim_repo_task` RPC claims the _next_ task in a queue —
it cannot claim a specific row. A dispatch launch knows exactly which
task it created and which session owns it, so it PATCHes that row
directly (the service-role key can), setting the same fields a claim
would. This is deliberate and is the only place dispatch writes task
rows outside the RPCs. One FK wrinkle: `tasks.worker_id` references
`agent.workers`, so dispatch idempotently registers its own worker row
(`dispatch-opencode`, role `build`) before the first claim PATCH.
Right after the PATCH it also calls `assign_repo_task_lane` — the lane
ledger assignment the queue workers record after claiming — reading
the lane from the progress the PATCH just set, so dispatch tasks are
indistinguishable from worker-claimed ones downstream (including
completion with an execution record).

## Auth

Every endpoint except `GET /health` requires `Authorization: Bearer …`:

- **GitHub Actions OIDC** (preferred): a JWT from
  `token.actions.githubusercontent.com`, RS256-verified against GitHub's
  JWKS (cached), `aud` = `pm-dispatch` (env `DISPATCH_OIDC_AUDIENCE`).
  The token's `repository` claim must be in the allowlist **and equal
  the request body's `repo`** — a workflow can only dispatch for its own
  repo. See `examples/dispatch-workflow.yml`.
- **Static bearer secret**: `DISPATCH_API_TOKEN` (in the laptop's
  `~/secrets/dispatch.env`), compared in constant time — the shape a
  repo stores as a GitHub Actions secret.

The caller identity (`{type:"bearer"}` or `{type:"oidc", repository,
actor, workflow_ref}`) is recorded in the task payload.

## Configuration (env)

| Name                                              | Default                                     | Notes                                                                 |
| ------------------------------------------------- | ------------------------------------------- | --------------------------------------------------------------------- |
| `DISPATCH_HOSTS`                                  | `127.0.0.1`                                 | install.sh renders loopback + the laptop tailnet IP                   |
| `DISPATCH_PORT`                                   | `4879`                                      |                                                                       |
| `DISPATCH_BUILD_VERSION`                          | `dev`                                       | set by install.sh to the deployed commit                              |
| `DISPATCH_API_TOKEN`                              | —                                           | from `~/secrets/dispatch.env`                                         |
| `DISPATCH_OIDC_AUDIENCE`                          | `pm-dispatch`                               |                                                                       |
| `DISPATCH_REPO_ALLOWLIST`                         | the four fleet repos                        |                                                                       |
| `SUPABASE_URL`                                    | `https://nltgxorqwxhgfzseiirt.supabase.co`  |                                                                       |
| `SUPABASE_SERVICE_ROLE_KEY`                       | —                                           | from `~/secrets/dispatch.env` (Bitwarden `supabase.service-role-key`) |
| `OPENCODE_URL`                                    | `http://127.0.0.1:13420`                    | laptop standing server                                                |
| `OPENCODE_AUTH_FILE`                              | `~/.local/state/opencode/service-prod.json` | password read at runtime, never logged                                |
| `GITHUB_APP_ID` / `GITHUB_APP_PRIVATE_KEY_BASE64` | —                                           | rendered by install.sh from `~/secrets/pm-stack.sops.yaml`            |

GitHub issue creation uses the pm-stack GitHub App (installed on code,
opencode, openchamber — **not** dezren39/nix, where issue creation is
skipped with a warning). If the app route is unusable, the laptop's
authenticated `gh` CLI is the fallback; if both fail, the request
proceeds without an issue and says so in `warnings` (for `/v1/tasks`
with `issue: true`, same behavior).

## Deploy

On the laptop, the unit lifecycle belongs to the NixOS pm-stack
module: `nixos-rebuild switch` deploys the module's
`pm-dispatch.service`, which runs from the primary checkout — so a
`git pull` in `~/git/code` (kept current by pm-git-sync) plus a
switch is the whole deploy, and home-manager renames the old
hand-installed unit to `*.backup`. The module does NOT render env
files: `~/pm-stack/dispatch/.env` and `~/secrets/dispatch.env`
are still produced by install.sh, below.

Fallback (no switch): on the laptop, from the primary checkout at
latest `main`:

```sh
cd ~/git/code && git pull
bash pm-stack/dispatch/install.sh
```

install.sh copies the tree to `~/pm-stack/dispatch`, renders `.env`
from the sops file, ensures `~/secrets/dispatch.env` exists (generating
`DISPATCH_API_TOKEN` if absent — value never printed), installs
`pm-stack/systemd/pm-dispatch.service` as a user unit, restarts only on
change, and fails loudly unless `/health` reports the deployed build
version. On `amd` that unit-install step is superseded by the
NixOS module; the env rendering is not.

## Public reachability

Done (2026-10-30): the existing remotely-managed Cloudflare tunnel
**amd-6-pm-stack** maps `dispatch.security.cab` →
`http://localhost:4879`, so GitHub Actions (runners are outside the
tailnet) can reach the service and use either auth form — OIDC is
verified end to end against that hostname. Tailnet callers (the
laptop itself, other fleet machines) can also call the loopback /
tailnet binds directly. The tunnel connector itself is the
`pm-tunnel` unit, declared by the same NixOS pm-stack module.

## Tests

```sh
bun test        # auth (bearer + stubbed-JWKS OIDC + repo binding),
                # priority mapping, model parsing, validation, packets
bunx tsc --noEmit
```
