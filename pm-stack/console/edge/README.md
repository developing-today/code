# pm-console-edge

The Cloudflare Worker in front of `https://pm.security.cab`. It serves
the PM app, its state, its notifications and the phone widget from
Cloudflare, so the app opens and works whether or not the laptop that
runs the fleet's tools can be reached.

## What runs where

| Thing                                                                            | Where                                                       | When the laptop cannot be reached                                                                                                                                                            |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The app's page, script and icons                                                 | edge                                                        | unchanged                                                                                                                                                                                    |
| Checking who is calling                                                          | edge, and again on the laptop                               | the edge still answers the owner; anyone it is unsure of gets nothing                                                                                                                        |
| State (in progress, next, needs me, prompts, pull…)                              | edge, from the laptop's report                              | the board, pull requests, ranked issues and prompts refresh from GitHub and the queue store after one minute of silence; the rest stays as of the last report                                |
| Notifications: keys, subscriptions, sending                                      | edge                                                        | unchanged; and one says the laptop has gone quiet                                                                                                                                            |
| Linked phones and the widget's summary                                           | edge                                                        | unchanged                                                                                                                                                                                    |
| Actions safe to do late (prompts, answers, comments, …)                          | laptop, relayed at once                                     | held at the edge, run once when it is back, dropped after a day and said so                                                                                                                  |
| Direct actions (reply to or close a prompt, comment, rank an issue, move a task) | edge, once the laptop has been quiet for a minute           | run at the edge through the laptop's own classes, and never sent to the laptop, so they are not done twice; a new prompt is filed at the edge and only its queue task is held for the laptop |
| Actions not safe to do late (merge, units, unblock, …)                           | laptop, relayed at once                                     | refused, saying why                                                                                                                                                                          |
| The dashboard's own pages, logs, one pull request's page                         | laptop                                                      | a plain notice with a link to the app                                                                                                                                                        |
| The signed Android build (`/console/pm-console.apk`)                             | GitHub's release first; else laptop, and a copy at the edge | the release is served (checked against its own description of itself); with none published, the copy is, and the app says so when the copy is older than the laptop's newest                 |

The list of actions that may be done late is `HOLDABLE` in
`../src/edge-proto.ts`.

## How it is put together

```
phone / browser
      │  HTTPS
Cloudflare Access ── sign-in (email code; Google once its client exists)
      │
pm-console-edge (this Worker, on the hostname's route)
  ├─ src/worker.ts   wiring: the object, the app's files, the laptop
  ├─ src/entry.ts    what to do with a request
  ├─ src/edge.ts     the state, in one Durable Object
  └─ src/direct.ts   the laptop's own classes (Ops, Work) with stand-ins
                     for GitHub and the queue store, for while it is away
      │  same hostname, through the tunnel
laptop: pm-console (../src/server.ts), reporting through ../src/sync.ts
```

- **Every request goes to the object first**, which checks who is
  calling with the same code the laptop uses (`../src/auth.ts`). It
  answers only when it is sure of the owner. When it is not, it does not
  refuse: it passes the request to the laptop, which decides as it always
  has. So a mistake at the edge can cost the edge's speed, never the
  owner's way in, and a stranger gets from the laptop exactly what a
  stranger got before.
- **The app's own files are not downloaded twice.** The platform's file
  store tags each file by its content; the Worker passes the tag on and
  answers a browser that names it with "still that" (304) and nothing
  else. They are still asked for every time (`no-cache`), so a deploy
  shows at once. `/console/health` says, to anyone, which release of the
  Android app this side knows of (`androidRelease`), beside the copy of
  the laptop's build (`androidBuild`): that is how either arriving here
  is checked from outside.
- **The laptop reports** to `POST /console/edge/sync`: the sections of
  state that changed, how current each unchanged one is, and what became
  of anything held for it. The answer says whether anyone is looking, and
  carries the actions the owner took while it was away.
- **The signed Android build travels in a report**, once per build, when
  the edge says it lacks it, and is kept only if it matches its
  fingerprint. It never stands in the way of the reports themselves,
  which are how the edge knows the laptop is there. Beside sections that
  would take the report past its room (counted in bytes as sent) it goes
  alone and they follow at once. If a report carrying it does not get
  through (a slow connection runs out of time, or it is turned away), or
  the file cannot be read, the reports after that go without it, and it
  is tried again ten minutes later.
- **An action for the laptop is relayed at once** when the laptop is in
  reach; its answer carries what it read again, so the edge's copy shows
  the change before the next report.
- **A held action is run at most once.** The laptop writes its id down
  before starting it; if it dies part-way, the action is reported as
  interrupted and never run again.
- **Before the laptop's first report, and whenever the Worker throws**,
  requests go to the laptop exactly as they did before this Worker
  existed.

## Who trusts what

- **The owner** is whoever presents a sign-in token that verifies
  against the issuer's published keys, for this application, carrying the
  owner's address. The edge learns the issuer and the address from the
  laptop's report.
- **The laptop** is whoever signs a report with the laptop's key. The
  key pair is made on the laptop and the private half never leaves it.
  The edge learns the public half by asking the laptop through the
  tunnel (`GET /console/edge/key`), which only the laptop can answer.
  There is no shared secret, here or in the repository. A report is
  refused unless the signature verifies, its time is within two minutes
  of the edge's clock, and it is later than the last one accepted.
  The path is open to anyone, since the laptop cannot sign in, and a
  report may be megabytes: one that says it is too large, or is not
  shaped like a report, is refused without being read, and an address
  with twenty refused reports in a minute is refused unread until the
  minute is out. A report the laptop signed that is merely wrong (an
  old clock, a replay) is not held against its address.
- **A phone** is whoever holds a token the edge issued after the owner
  approved it. The edge keeps only the token's hash.
- **The laptop trusts the edge** for one thing: the held actions in the
  answer to its own report, fetched over TLS from this hostname. It runs
  only actions on the late list, and records each in its audit log as
  `via: edge-held` with who took it and when.

Three secrets are optional, put there by the laptop's
`scripts/expose.py --edge-direct on` (off by default, remembered). The
owner turns it on and off in the app, under More, Laptop away, which
runs that script on the laptop; `--edge-direct off` takes the two
tokens away again. They travel through Cloudflare's API with the token
the script already holds, so nothing is installed on the laptop for it:
`GITHUB_TOKEN` (the repository automation token), `QUEUE_TOKEN` (the
queue store token), and `EDGE_DIRECT`, the explicit opt-in that must
read `on` for either to be used. With them, the edge reads the board,
pull requests, ranked issues and prompts itself, and runs the direct
actions, once the laptop has been quiet for a minute; without them, or
with the flag off, it serves only the laptop's last report and holds
what the owner does, as before. `/console/health` says whether the
flag is on, whether each token is present, whether the direct
capability is ready, and the result of the self-check: one direct read
of each kind, run when the secrets arrive and at most once an hour
after. A secret is sent only to its own service — the GitHub token
only to GitHub, the queue token only to the queue store — and is never
returned by health. The other keys the Worker uses it made itself
(notification signing) or were published to it (the laptop's, the
sign-in issuer's).

## Deploy, and going back

Pushes to `main` that touch the console deploy it
(`.github/workflows/pm-console-edge-deploy.yml`): the console's tests,
which drive this Worker's routing and state against the laptop's real
handler; a bundle; a run of that bundle in the Worker runtime itself
(`scripts/edge-smoke.ts`, below), because the tests run under bun and
the two do not behave alike everywhere; the release; then a check that
the deployed Worker answers its health check as that commit.

The release is [`release.mjs`](release.mjs), and it starts from whatever
is there, including nothing:

1. No Worker of that name: an empty one is stood up
   ([`init/`](init/worker.ts): the Durable Object's class and an answer
   that says it is empty). A version cannot be uploaded to a Worker that
   does not exist, and the storage behind the class is only made by a
   deploy.
2. This commit is uploaded as a version. It serves nothing yet.
3. That version is put at 100%.
4. The Worker is asked which version it is. If it does not answer as
   this commit, the version that was serving is put back and the run
   fails.

A new Durable Object migration in `wrangler.jsonc` cannot travel in a
version, so then steps 2 and 3 are one `wrangler deploy`.

So an account with no Worker needs nothing but this workflow to get
one. That is not taken on trust: whenever a push changes the release
itself (`release.mjs`, `release-plan.mjs`, `init/`, `wrangler.jsonc`,
the lockfile or the workflow), and on a manual run with `scratch`, the
same steps are first run on `pm-console-edge-scratch`, which is removed
before (so the start is nothing) and removed after. The real Worker is
not touched by that, and the scratch one is the only Worker the script
will ever delete.

```bash
gh workflow run pm-console-edge-deploy.yml                    # deploy main's head
gh workflow run pm-console-edge-deploy.yml -f scratch=true    # and show the stand-up from nothing first
```

The Worker has no route in `wrangler.jsonc` on purpose. The laptop's
`scripts/expose.py` (on a 15 minute timer) sends the hostname through it
once it answers its health check, after telling the console where to
report. Three ways back, quickest first:

```bash
python3 pm-stack/console/scripts/expose.py --edge off   # hostname goes straight to the laptop; remembered
gh workflow run pm-console-edge-deploy.yml -f rollback_version=<version id>   # an earlier version of the Worker
git revert <merge>                                       # and the next push deploys it
```

## Run it here

```bash
cd pm-stack/console && bun test                 # routing, state and the laptop's link, end to end
bun run scripts/edge-demo.ts                    # the app behind the edge with made-up state: http://127.0.0.1:4897/
cd edge && npm ci && node build.mjs
npx wrangler deploy --dry-run --outdir .wrangler/dry    # does it bundle
npx wrangler dev --port 8799 --var PUBLIC_HOST:localhost --var HOST_ORIGIN:http://127.0.0.1:4898 --var VERSION:smoke
bun run ../scripts/edge-smoke.ts                # in another shell: the real Worker runtime against a local host
```

`HOST_ORIGIN` exists for that last run only. Deployed, the laptop is
simply what is behind the hostname.

## Limits worth knowing

- State at the edge is the laptop's last report, except for the board,
  pull requests, ranked issues and prompts, which the edge reads itself
  after the laptop has been quiet for a minute. The rest ages, and the
  app says how old it is.
- A new prompt filed at the edge still needs the laptop for its queue
  task: the issue is filed at once and only the task is held, as
  `prompt.enqueue`, for the laptop's return. The queue task is never
  written from the edge itself: there is no fleet binary there, and
  copying the queue's rows by hand would break its contract.
- An action the edge runs itself is never sent to the laptop, so the
  laptop cannot repeat it; an action that was relayed and got no answer
  is only ever resolved by the laptop, which wrote its name down before
  starting. That is the rule against doing anything twice.
- The storage is one Durable Object. A section larger than one stored
  value is kept in pieces.
