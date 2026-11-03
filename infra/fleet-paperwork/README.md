# fleet-paperwork — issue filing as the `developing.today` GitHub App

## Purpose

Fleet bookkeeping sometimes has to file GitHub issues in bulk (for example,
one decision issue per branch preserved by a worktree cleanup). Doing that
through a personal user token has two failure modes, both observed live on
2026-11-01:

1. **Secondary rate limits are per-actor.** A burst of ~322 `gh issue create`
   calls tripped the *user's* secondary rate limit; content creation for the
   account was then blocked outright (HTTP 403) even though nothing was wrong
   with the requests.
2. **Silent non-creation.** While limited, `gh issue create` twice returned
   exit 0 with empty output and created *nothing*. A later audit found one
   batch issue missing precisely because its creation had been "confirmed"
   from the filer's own lists instead of from GitHub readback.

`fleet_paperwork.py` fixes both:

- It authenticates as the **`developing.today` GitHub App** (ID 5454422,
  permissions: contents write, issues write, metadata read, pull requests
  write — the write permissions were accepted on both installations and
  verified at token level on 2026-11-03). An App *installation* token is a
  separate actor from any user token, with its own rate-limit bucket, so
  fleet paperwork no longer competes with — or gets benched by — a human's
  interactive `gh` usage. That separation is also why pull requests are now
  opened as the app whenever the user token is secondary-limited (as it was
  again on 2026-11-03).
- The app is installed twice. The **org installation** (developing-today,
  ID 174792676) covers `code`, `opencode`, and `openchamber`; the
  **personal installation** (dezren39, ID 175747567) covers `dezren39/nix`
  and `dezren39/fleet`.
- **A filing is verified, or it is a failure.** Every create / comment / close
  is followed by a readback GET, and the result is compared (title, body,
  state). A missing, empty, or mismatched readback exits nonzero.
- Creates are **deduped by exact title** (search API + byte-exact title
  comparison) and **paced** (default 3 s minimum between writes, enforced
  across invocations via the ledger), so re-running a batch is safe and
  bursts stay polite.

Issues filed this way are authored by the app's bot identity
(`developing-today[bot]`), which keeps bulk bookkeeping visibly separate from
human-authored issues.

Because the app now holds write permissions beyond issues, the helper also
exposes it **generically** (the `token` and `api` commands under Usage):
anything a token can do — PR create/edit, reviews, comments anywhere — can
ride the app instead of a user token, without hand-rolling the JWT flow
each time.

## How the token flow works

Same flow the pm-stack services already use (see `../../pm-stack`):

1. The App private key lives only in the sops-encrypted secrets file
   `~/secrets/pm-stack.sops.yaml` under the keys `github_app_id` and
   `github_private_key_base64`. The script extracts those two values with
   `sops -d --extract` at runtime. **No credential value is stored in this
   repo, printed, or logged** — the key is materialized only as a `0600`
   temp file for the seconds it takes `openssl` to sign one JWT, then
   unlinked.
2. The script builds an RS256 JWT (`iss` = App ID, 9-minute expiry) and
   lists the App's installations (`GET /app/installations`). The
   installation is chosen by matching the target repo's **owner login** —
   the org installation for `developing-today/*`, the personal installation
   for `dezren39/*`. Nothing about installation IDs is hardcoded.
3. It mints an installation access token
   (`POST /app/installations/{id}/access_tokens`) and confirms the target
   repo is actually in that installation's scope
   (`GET /installation/repositories`) before writing anything.
4. Writes go to the plain issues API (`POST /repos/{owner}/{repo}/issues`,
   `.../issues/{n}/comments`, `PATCH .../issues/{n}`) — comments and closes
   ride the same token and the same verification rule.
5. The generic commands stop after step 3: `token` prints the minted token
   (and nothing else) on stdout; `api` makes one caller-specified REST call
   with it and reports the status on stderr.

## Usage

```sh
# Read-only sanity: app, installations, and whether a repo is in scope.
python3 fleet_paperwork.py check --repo developing-today/code

# Create (deduped by exact title, verified by readback).
python3 fleet_paperwork.py create --repo developing-today/opencode \
  --title "Preserved branch: … — decision needed (merge / cherry-pick / drop)" \
  --label priority/low --body-file body.md

# Comment / close (also verified).
python3 fleet_paperwork.py comment --repo developing-today/opencode \
  --issue 1203 --body-file comment.md
python3 fleet_paperwork.py close --repo developing-today/opencode --issue 1203

# Generic access: print an installation token (stdout only — every
# diagnostic goes to stderr, so the capture is clean), then drive gh or
# curl with it.
GH_TOKEN=$(python3 fleet_paperwork.py token --repo developing-today/code) \
  gh pr create --repo developing-today/code --title "…" --body "…"

# One arbitrary REST call as the app. HTTP status goes to stderr, the
# response JSON to stdout, and HTTP >= 400 exits nonzero. --method is
# required for every call — a write can never happen by default — and
# there is no readback verification: the caller owns verifying what it did.
python3 fleet_paperwork.py api --repo developing-today/code \
  --method GET --path /repos/developing-today/code
python3 fleet_paperwork.py api --repo developing-today/code \
  --method POST --path /repos/developing-today/code/pulls --input pr.json
```

Options: `--dry-run` on any write command (resolves auth + dedupe, prints
what it would do, writes nothing), `--no-dedupe` on create, `--pace SECONDS`
(default 3), `--sops PATH` or `$FLEET_PAPERWORK_SOPS` to point at a different
sops file.

## Ledger

Every verified write appends one TSV line —
`timestamp, action, repo, issue number, title/detail` — to
`~/.local/state/fleet-paperwork/ledger.tsv` (override with
`$FLEET_PAPERWORK_LEDGER`). The ledger lives outside the repo, next to the
other fleet state, and doubles as the pacing clock.

## First live use (2026-11-01)

Filed the three paperwork items left pending by the user-token rate limit:
the preserved-branch decision issue for
`rescue/3041-631-subagent-ancestor-2026-11-01` in opencode, the resolution
comment + close on opencode#1203, and the audit-gap decision issue for
`queue/3201-2167-shebang-dispatch` in code. Issue numbers are recorded in the
ledger and in the run reports (`~/workspace/laptop-hygiene-2026-11-01.md`,
`~/workspace/issue-pr-audit-2026-11-01.md`).

## Tests

```sh
python3 -m unittest discover -s tests -v   # pure-function tests, no network
python3 fleet_paperwork.py check --repo developing-today/code   # live, read-only
```
