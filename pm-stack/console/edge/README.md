# pm-console-edge

The Cloudflare Worker in front of `https://pm.security.cab`. It serves
the PM app, its state, its notifications and the phone widget from
Cloudflare, so the app opens and works whether or not the laptop that
runs the fleet's tools can be reached.

## What runs where

| Thing                                                    | Where                          | When the laptop cannot be reached                                           |
| -------------------------------------------------------- | ------------------------------ | --------------------------------------------------------------------------- |
| The app's page, script and icons                         | edge                           | unchanged                                                                   |
| Checking who is calling                                  | edge, and again on the laptop  | the edge still answers the owner; anyone it is unsure of gets nothing       |
| State (in progress, next, needs me, prompts, pull…)      | edge, from the laptop's report | pull requests can be refreshed from GitHub after one minute of silence; other data remains as of the last report |
| Notifications: keys, subscriptions, sending              | edge                           | unchanged; and one says the laptop has gone quiet                           |
| Linked phones and the widget's summary                   | edge                           | unchanged                                                                   |
| Actions safe to do late (prompts, answers, comments, …)  | laptop, relayed at once        | held at the edge, run once when it is back, dropped after a day and said so |
| Actions not safe to do late (merge, units, unblock, …)   | laptop, relayed at once        | refused, saying why                                                         |
| The dashboard's own pages, logs, one pull request's page | laptop                         | a plain notice with a link to the app                                       |

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
  └─ src/edge.ts     the state, in one Durable Object
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
- **The laptop reports** to `POST /console/edge/sync`: the sections of
  state that changed, how current each unchanged one is, and what became
  of anything held for it. The answer says whether anyone is looking, and
  carries the actions the owner took while it was away.
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
- **A phone** is whoever holds a token the edge issued after the owner
  approved it. The edge keeps only the token's hash.
- **The laptop trusts the edge** for one thing: the held actions in the
  answer to its own report, fetched over TLS from this hostname. It runs
  only actions on the late list, and records each in its audit log as
  `via: edge-held` with who took it and when.

The optional direct pull-request read uses the `GITHUB_TOKEN` Worker
secret only when `GITHUB_READ_ENABLED=on`. It is disabled by default;
`/console/health` reports whether it is enabled, whether the secret exists,
and whether the capability is ready. The token is sent only to GitHub's
read endpoint and is never returned by health. Other edge reads and writes
are not implemented by this first slice.

## Deploy, and going back

Pushes to `main` that touch the console deploy it
(`.github/workflows/pm-console-edge-deploy.yml`): the console's tests,
which drive this Worker's routing and state against the laptop's real
handler; a bundle; `wrangler deploy`; then a check that the deployed
Worker answers its health check as that commit.

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

- State at the edge is the laptop's last report except for open pull
  requests, which can refresh from GitHub after the laptop has been quiet
  for one minute and the direct-read switch and secret are present.
- Fresh board, ranked-issue and prompt reads, and direct actions remain
  unimplemented. Prompt queue-task creation still requires the documented
  fleet queue contract or explicit laptop enqueue on return.
- The storage is one Durable Object. A section larger than one stored
  value is kept in pieces.
