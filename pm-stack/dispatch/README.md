# pm-dispatch

Authenticated intake for the fleet: launch an OpenCode session on Drew's
laptop **now**, or just enqueue work into the Supabase repo-health queues —
either way the work is tracked by a queue task and (usually) a GitHub
issue, so nothing runs untracked and no queue worker double-books it.

Lives in `pm-stack/dispatch/`, deployed on the laptop next to the
aggregator (`~/pm-stack/dispatch`, user unit `pm-dispatch.service`,
port 4879, loopback + tailnet bind only).

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
  "repo": "developing-today/code",        // required, allowlisted fleet repo
  "prompt": "Fix the flaky widget test",  // required
  "title": "fix(widget): flaky test",     // optional; default: prompt's first line
  "model": "opencode-go/gpt-5.6-luna",    // optional "provider/model" or {"providerID","id"}
  "directory": "/home/user/git/code",     // optional; default: the repo's laptop primary
  "priority": "high",                     // optional: 10-90 or critical|high|normal|low → 10/20/40/70
  "issue": true,                          // optional; default true
  "wait": true                            // optional; default true
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
  "repo": "developing-today/opencode",  // required
  "title": "investigate the red build", // required
  "prompt": "…",                        // required ("body" accepted as an alias)
  "kind": "triage",                     // optional: triage|small_fix|pr_shepherd|ci_investigation
  "priority": 40,                       // optional, same mapping as above
  "model": "deepseek/deepseek-chat",    // optional; stored as payload.model_hint only
  "issue": false,                       // optional; default false
  "dedupe_key": "…",                    // optional; default: canonical key from the
                                        // created issue, else a unique dispatch key
  "issue_number": 123                   // optional; attach to this existing issue
                                        // in repo instead of creating one (see below)
}
```
Returns `201 {task_id, issue_number|null, queue}`. With `issue: true`
the issue is created first (labels `queue/queued` + `source/dispatch`)
and gets an agent-note comment naming the task id.

With `issue_number` set, no issue is created: the task attaches to
that existing issue in `repo` — the canonical dedupe key is derived
from it, and after the enqueue the issue gets the same agent-note
comment naming the task id plus the `queue/queued` and
`source/dispatch` labels. The response is `201 {task_id,
issue_number, issue_url, queue}`, where `issue_url` is
`https://github.com/<repo>/issues/<issue_number>`. `issue_number`
takes precedence over `issue` when both are sent. For dezren39/nix,
where the GitHub App is not installed, the attach is skipped with a
`warnings` entry; the task is still enqueued and `issue_number` is
still returned.

### Why assignment is a direct PATCH
The queue's `claim_repo_task` RPC claims the *next* task in a queue —
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
  The token's `repository` claim must be in the allowlist. **Grants
  mesh:** every allowlisted repo may target every allowlisted repo —
  the claim need not equal the request body's `repo`. Repos outside
  the allowlist can neither call nor be targeted. Bearer callers are
  bound only by the allowlist on the target.
  See `examples/dispatch-workflow.yml`.
- **Static bearer secret**: `DISPATCH_API_TOKEN` (in the laptop's
  `~/secrets/dispatch.env`), compared in constant time — the shape a
  repo stores as a GitHub Actions secret.

The caller identity (`{type:"bearer"}` or `{type:"oidc", repository,
actor, workflow_ref}`) is recorded in the task payload.

## GitHub Actions (reusable)

Three composite actions in this repo wrap dispatch so call sites do
not hand-roll the OIDC + curl plumbing. Two are low-level primitives
that wrap the dispatch endpoints directly; the third is the
task-level entry point built on top of them:

- `.github/actions/pm-dispatch-start` ("the starter") — POSTs to
  `/v1/sessions`; outputs `task_id`, `session_id`, `issue_number`
  and `issue_url`.
- `.github/actions/pm-dispatch-queue` ("the queuer") — POSTs to
  `/v1/tasks`, including the `issue` / `issue_number` inputs;
  outputs `task_id`, `issue_number` and `issue_url`.
- `.github/actions/pm-dispatch-task` ("the task action") — dispatches
  a named, recurring task (`bolt`, `palette`, `suggestions`,
  `ci-fix`, …) through the starter/queuer as `uses:` steps, using the
  prompt library bundled with the action. It contains no OIDC or HTTP
  plumbing of its own. Outputs `task_id`, `session_id`,
  `issue_number` and `issue_url`, whichever apply to the path taken.

Usage, pinned to `@main` so fleet call sites pick up central updates:

```yaml
jobs:
  dispatch:
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: developing-today/code/.github/actions/pm-dispatch-start@main
        with:
          title: "fix(widget): flaky test"
          prompt: "Fix the flaky widget test"
      - uses: developing-today/code/.github/actions/pm-dispatch-queue@main
        with:
          title: "investigate the red build"
          prompt: "…"
          issue_number: "123"   # optional: attach instead of creating
```

The calling job MUST grant `permissions: id-token: write`; without
it the OIDC token-request environment is absent and the actions fail
with a clear error. `repo` defaults to the caller's repository, and
under the grants mesh any allowlisted repo may target any
allowlisted repo. See `examples/dispatch-workflow.yml` for a
complete workflow.

### Prompt sources (the primitives)

Both primitives take the prompt from one of three inputs. `prompt`
is not required at the YAML level (a composite action cannot express
one-of); the action's step enforces that one source yields text.
The sources are resolved in this order:

1. `prompt-file` — a path to a file containing the prompt, relative
   to the caller's workspace. The action reads it at runtime, so the
   calling job must check out its repo first (e.g.
   `actions/checkout`). A missing or empty file fails the step. This
   is the form scheduled workflows use to keep prompts in repo files
   instead of inline YAML strings.
2. `prompt-url` — an https URL the prompt is fetched from at
   runtime (`curl -fsS`). A fetch error or an empty body fails the
   step.
3. `prompt` — the inline prompt text.

A selected source that is missing, empty or unfetchable fails
loudly rather than falling through to a lower-priority source, and
if none of the three yields non-empty text the step fails with a
clear error. For example, with a prompt kept in the calling repo:

```yaml
    steps:
      - uses: actions/checkout@v4
      - uses: developing-today/code/.github/actions/pm-dispatch-queue@main
        with:
          title: "nightly triage"
          prompt-file: "prompts/nightly-triage.md"
```

The primitives keep these overrides unchanged. The task action
below uses the same three overrides and adds its prompt library as
a fourth, final source.

### pm-dispatch-task — the task-level entry point

`.github/actions/pm-dispatch-task` is the entry point callers
should normally use. A call names a task and a target repo, and the
action supplies the prompt from its bundled library:

```yaml
jobs:
  bolt:
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: developing-today/code/.github/actions/pm-dispatch-task@main
        with:
          task: bolt
          repo: developing-today/code
```

Inputs: `task` (required), `repo` (default: the caller's
repository), `title`, `model`, `priority`, the prompt overrides
`prompt` / `prompt-file` / `prompt-url`, the ci-fix context inputs
below, and `default-branch` (default: the caller's repository's
default branch). `task` accepts ANY string: the name selects the
library prompt, so a future library entry works with no change to
the action. For non-ci-fix tasks an empty `title` defaults to
`Scheduled <Task> — <repo>`, and the task is dispatched through the
starter with `wait: "true"`.

#### The prompt library

The library lives in `.github/actions/pm-dispatch-task/prompts/`,
one Markdown file per task:

- `bolt.md` — one small, measurable performance improvement, with
  before/after evidence.
- `palette.md` — one micro-UX / accessibility improvement.
- `suggestions.md` — a verified code-review pass (Cleanup /
  Performance / Security / Code Health / Testing) that files at
  most 5 issues with file/line evidence.
- `ci-fix.md` — the ci-fix prompt template (see "ci-fix mode"
  below); it carries `{{workflow}}`, `{{run_id}}`, `{{run_url}}`,
  `{{branch}}`, `{{sha}}` and `{{pr}}` placeholders that the action
  substitutes from the ci-fix context inputs before dispatching.

Prompt resolution in the task action follows the primitives' order
and adds the library as the final fallback:

1. `prompt-file` (relative to the caller's workspace),
2. `prompt-url` (`curl -fsS`),
3. `prompt` (inline),
4. the library file `prompts/<task>.md`.

The first non-empty source wins. If none yields text — an unknown
task with no override — the step fails with
`no library prompt for task '<task>'`.

To add a common task: drop a `prompts/<task>.md` file into the
library in this repo, and add one step/call for it in the caller.
No change to the action itself is needed.

#### ci-fix mode (`task: ci-fix`)

`task: ci-fix` replaces the old standalone `pm-dispatch-ci-fix`
action (removed; no caller ever used it). Give the task action the
failed run's context — `workflow`, `run-id`, `run-url`, `branch`,
`sha`, and optionally `pr` — and it:

1. Resolves the prompt (normally the `ci-fix.md` library template)
   and substitutes the run-context placeholders.
2. Dedupes on the conventional tracking-issue title
   `ci-fix: <workflow> failing on <branch>`: it searches the target
   repo's open issues for that exact title.
3. Dispatches by case, with priority `high` when the failed branch
   is the repo's default branch and `normal` otherwise (unless the
   caller passes an explicit `priority`):
   - an open issue with the conventional title already exists — the
     queuer attaches a new `ci_investigation` task to that issue
     (`issue_number`), so repeat failures of the same workflow on
     the same branch do not pile up duplicate issues;
   - no such issue and the failed branch is the default branch —
     the starter runs with `wait: "true"`, creating the tracking
     issue (titled with the conventional title) and launching a
     session immediately;
   - no such issue and the failed branch is any other branch — the
     queuer runs with `issue: "true"`, queueing a
     `ci_investigation` task and creating the tracking issue.

```yaml
jobs:
  ci-fix:
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      issues: read
      contents: read
    env:
      GH_TOKEN: ${{ github.token }}
    steps:
      - uses: developing-today/code/.github/actions/pm-dispatch-task@main
        with:
          task: ci-fix
          workflow: ${{ github.event.workflow_run.name }}
          run-id: ${{ github.event.workflow_run.id }}
          run-url: ${{ github.event.workflow_run.html_url }}
          branch: ${{ github.event.workflow_run.head_branch }}
          sha: ${{ github.event.workflow_run.head_sha }}
```

On top of the starter/queuer requirement (`permissions: id-token:
write`), a ci-fix calling job MUST also provide `GH_TOKEN` in the
job environment (as above) with issues read permission in the
target repo; the dedupe search fails loudly without it. The step
summary records the task, the path taken, and the dispatched task /
session / issue ids.

#### Design note

Every call carries `repo` and `task` explicitly, and the prompt
library is central (it lives with the action in this repo, not in
the callers). Because of that, repo- or time-based filtering — for
example "only run `suggestions` on repos X and Y", or "skip `bolt`
overnight" — can later be added inside the task action without
touching any caller.

## Configuration (env)
| Name | Default | Notes |
| --- | --- | --- |
| `DISPATCH_HOSTS` | `127.0.0.1` | install.sh renders loopback + the laptop tailnet IP |
| `DISPATCH_PORT` | `4879` | |
| `DISPATCH_BUILD_VERSION` | `dev` | set by install.sh to the deployed commit |
| `DISPATCH_API_TOKEN` | — | from `~/secrets/dispatch.env` |
| `DISPATCH_OIDC_AUDIENCE` | `pm-dispatch` | |
| `DISPATCH_REPO_ALLOWLIST` | the six fleet repos | |
| `SUPABASE_URL` | `https://nltgxorqwxhgfzseiirt.supabase.co` | |
| `SUPABASE_SERVICE_ROLE_KEY` | — | from `~/secrets/dispatch.env` (Bitwarden `supabase.service-role-key`) |
| `OPENCODE_URL` | `http://127.0.0.1:13420` | laptop standing server |
| `OPENCODE_AUTH_FILE` | `~/.local/state/opencode/service-prod.json` | password read at runtime, never logged |
| `GITHUB_APP_ID` / `GITHUB_APP_PRIVATE_KEY_BASE64` | — | rendered by install.sh from `~/secrets/pm-stack.sops.yaml` |

GitHub issue creation uses the pm-stack GitHub App (installed on code,
opencode, openchamber and dezren39/nix — **not** developing-today/zettel
or dezren39/dezren39, where issue creation is skipped with a warning).
If the app route is unusable, the laptop's
authenticated `gh` CLI is the fallback; if both fail, the request
proceeds without an issue and says so in `warnings` (for `/v1/tasks`
with `issue: true`, same behavior).

## Deploy
On the laptop, from the primary checkout at latest `main`:
```sh
cd ~/git/code && git pull
bash pm-stack/dispatch/install.sh
```
install.sh copies the tree to `~/pm-stack/dispatch`, renders `.env`
from the sops file, ensures `~/secrets/dispatch.env` exists (generating
`DISPATCH_API_TOKEN` if absent — value never printed), installs
`pm-stack/systemd/pm-dispatch.service` as a user unit, restarts only on
change, and fails loudly unless `/health` reports the deployed build
version.

## Remaining step — public reachability (Drew's)
The service is reachable on loopback + tailnet only. For GitHub Actions
(runners are outside the tailnet) to reach it, add a hostname to the
existing remotely-managed Cloudflare tunnel **amd-6-pm-stack** in Drew's
Cloudflare account:

- Public hostname: `dispatch.security.cab` → `http://localhost:4879`

That change lives in the tunnel's Cloudflare-side configuration and is
deliberately not automated here. Until it exists, OIDC dispatch from
Actions cannot reach the service; tailnet callers (the laptop itself,
other fleet machines) can use either auth form today.

## Tests
```sh
bun test        # auth (bearer + stubbed-JWKS OIDC + repo binding),
                # priority mapping, model parsing, validation, packets
bunx tsc --noEmit
```
