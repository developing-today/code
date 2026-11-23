# pm-console-edge

The Cloudflare Worker in front of `https://pm.security.cab`. It serves
the PM app, its state, its notifications and the phone widget from
Cloudflare, and with its two tokens it does the core itself: it reads
and writes GitHub and the queue store directly, whether or not a
machine is connected. The laptop that runs the fleet's tools is one
machine that connects to it: it reports what only a machine has and
does what only a machine can.

## What runs where

With the switch on and both tokens there (the usual state; see
"The two tokens" below):

| Thing                                                                                                         | Where                                                                                       | With no machine connected                                                                                                                                                    |
| ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The app's page, script and icons                                                                              | edge                                                                                        | unchanged                                                                                                                                                                    |
| Checking who is calling                                                                                       | edge, and again on the laptop                                                               | the edge still answers the owner; anyone it is unsure of gets nothing                                                                                                        |
| The board, the owner's prompts, pull requests, ranked issues                                                  | edge, read from GitHub and the queue store on each section's own interval                   | unchanged                                                                                                                                                                    |
| One pull request's page, the queue by state, roadmaps, bugs, intake, releases                                 | edge, read when asked for                                                                   | unchanged                                                                                                                                                                    |
| Ask, reply, close a prompt, comment, rank, move a task, merge, start a deploy, file a note, a bug or a change | edge, at once; never sent to a machine, so never done twice                                 | unchanged                                                                                                                                                                    |
| The queue task of a prompt, a note or a bug filed at the edge                                                 | a machine: the edge files the issue and holds the task (`prompt.enqueue`, `triage.enqueue`) | held, filed once when one connects, dropped after a day and said so                                                                                                          |
| Sessions, questions, things for the owner's hands, units, agents                                              | a machine's report, kept at the edge                                                        | as of its last report, and the app says how old                                                                                                                              |
| Notifications: keys, subscriptions, sending                                                                   | edge                                                                                        | unchanged; a reply to a prompt is noticed by the edge itself, which wakes each minute while a device is subscribed; one says the laptop has gone quiet                       |
| Linked phones and the widget's summary                                                                        | edge                                                                                        | unchanged                                                                                                                                                                    |
| Actions only a machine can do, safe to do late (answers, messages to agents, …)                               | a machine, relayed at once                                                                  | held at the edge, run once when it is back, dropped after a day and said so                                                                                                  |
| Actions only a machine can do, not safe to do late (units, unblock, …)                                        | a machine, relayed at once                                                                  | refused, saying why                                                                                                                                                          |
| The dashboard's own pages, logs                                                                               | a machine                                                                                   | a plain notice with a link to the app                                                                                                                                        |
| The signed Android build (`/console/pm-console.apk`)                                                          | GitHub's release first; else the laptop, and a copy at the edge                             | the release is served (checked against its own description of itself); with none published, the copy is, and the app says so when the copy is older than the laptop's newest |

With the switch off, or a token missing, the edge reads and does
nothing itself: every section is the laptop's last report, and every
action is the laptop's, relayed at once or held.

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
  └─ src/direct.ts   the console's own classes (Ops, Work, Plan) with
                     stand-ins for the fleet binary: GitHub's REST API
                     and the queue store, called from here
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
- **No class keeps the platform's `fetch` itself.** In the Worker
  runtime `fetch` refuses to be called as a method of another object
  ("Illegal invocation"), and bun does not mind, so no test under bun
  could see it: for the first day every notification the edge sent and
  every read of the Android release failed that way. Each class keeps a
  plain caller (`../src/plain.ts`); the tests hand the classes a fetch
  that refuses as the runtime does (`asThePlatformFetch`); and the
  deploy's run of the bundle in the runtime itself checks that the
  state can make a request of its own. `/console/health` says why the
  release could not be read when it could not
  (`androidReleaseTrouble`).
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

### The two tokens

Three Worker secrets let the edge do the core: `GITHUB_TOKEN` (the
repository automation token), `QUEUE_TOKEN` (the queue store token),
and `EDGE_DIRECT`, the switch, which must read `on` for either to be
used. The laptop's `scripts/expose.py` puts them there, reading the
two tokens by name from the secret store and sending them through
Cloudflare's API with the token it already holds; nothing is installed
on the laptop for it, and no value is printed or written anywhere.

On is how this is set up: a run of that script that was never told
either way gives the Worker all three, and its timer keeps them there.
`--edge-direct off` turns the switch off, takes the two tokens away,
and is remembered, also on a laptop that never had it on; the owner
has the same switch in the app, under More, Cloudflare. A run that was
never told and cannot do it (a secret not readable, Cloudflare
refusing) says so and carries on, and the next run tries again.

`/console/health` says whether the switch is on, whether each token is
present, whether the edge is doing the core (`primary`), and the result
of its self-check: one read of each kind, run when the secrets arrive
and at most once an hour after. A secret is sent only to its own
service (the GitHub token only to GitHub, the queue token only to the
queue store) and is never returned by health. The other keys the
Worker uses it made itself (notification signing) or were published to
it (the laptop's, the sign-in issuer's).

### How the edge reads

- **A section the edge reads is the edge's alone.** The laptop's copy
  of it is not taken, nor its word on how current it is: two sources
  for one section would take turns being shown. The edge says which
  sections those are in its answer to a report (`owns`), and the laptop
  stops sending them. If the switch is turned off, the laptop's copies
  come back with its next report.
- **Never in the way of an answer.** A document is answered at once
  from what the edge has; a section whose interval has passed is read
  again behind the answer, and the app's next look has it. A read that
  fails leaves the last good copy and says why.
- **Kept across restarts.** Each good read is stored, so an object that
  has just started shows it at once, as old as it is, and reads again
  behind that. Content that has not changed is not written again. The
  prompts kept this way also say what each thread held, so a prompt
  that has not changed is not fetched again.
- **A minute's wake, only when there is someone to tell.** While a
  device takes notifications the object wakes itself each minute, reads
  the prompts, and tells the owner of a reply. With nobody subscribed
  it wakes only to notice a laptop gone quiet.
- **One place for each read.** The reads that need only GitHub and the
  queue store are `../src/reads.ts`, used by the laptop's server and by
  the edge alike.
- **A refusal is handed on, never shown as the owner's failure.** The
  edge's token is not the laptop's way in, and may be allowed less. When
  GitHub or the queue store refuses it (401, 403) before anything was
  written, nothing has been done, so the action goes to the laptop as it
  would have without the tokens: done at once, held, or refused as not
  safe to do late. The edge leaves that kind of action to the laptop for
  an hour, and `/console/health` lists the kinds under `refused`, which
  is how a missing permission is seen. A refusal after a write was
  taken, a write that got no answer, and any failure that is not a
  refusal are not handed on: they are said as they are. Reading the
  queue is not a write; changing it is.
- **Calling too often is not a refusal.** GitHub turns a caller away
  for that with the same 403 (and says so in its answer), or a 429.
  It is handed on in the same way, but for five minutes, not an hour:
  it passes by itself. The token is the fleet's too, and search is
  what runs out first.
- **The same for what it reads.** A section the edge is refused for is
  the laptop's to read for the hour, exactly as it was before the edge
  read anything: the edge stops naming it under `owns`, the laptop's
  next reports bring its copy, and health lists it as
  `read.<section>`. A section is judged by the answers of the services
  it is read from (the board from the queue store, pull requests and
  ranked issues from GitHub, the prompts from both), so one service
  refusing does not hand over what is read from the other. While a
  device takes notifications and the prompts are the laptop's, the
  object asks to be woken when their time is up, so that with nobody
  looking and no laptop reporting it still tries again. A copy of the
  laptop's that is already here is shown without a gap; a copy the edge
  read itself is shown with the reason beside it until the laptop's
  comes. A screen read on request
  (a pull request, the deploys, a roadmap) that is refused, or that
  answers in part because one of its reads was, is asked of the laptop
  while one is reporting; with none, what could be read is shown, or
  the refusal is said as what it is. So a token that may do less than
  was thought leaves the app no worse off than before it was given.
- **Remembered across restarts, and tried again when the owner says.**
  What the edge is turned away for is written to its storage with the
  time it stands until. The object is put away when idle; one that
  forgot would claim every section again on each start, the laptop,
  told so, would stop sending its copy, and nothing would show the
  section at all. When the token has been put right, More, Cloudflare
  has "Try again now" (`edge.retry`), which is the edge's own to do and
  clears the list at once.
- **What changed is read from where it changed.** After an action the
  edge reads again the sections it touched before it answers, from
  their source and never from a copy kept before a restart. When a
  machine did the action (one handed on, one only a machine does, one
  held and done late), the sections it says it changed that the edge
  reads are read again here at once, not when their interval comes
  round. A read that was already under way when the change was made is
  waited for and not trusted: the section is read once more.

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
   deploy. This step is made up to four times: Cloudflare is sometimes
   not done making a new Worker when wrangler, having uploaded it, goes
   on to ask about it ("This Worker does not exist", code 10007), and
   the command leaves the same empty Worker each time it is made.
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
npx wrangler dev --port 8799 --var PUBLIC_HOST:localhost --var HOST_ORIGIN:http://127.0.0.1:4898 --var VERSION:smoke \
  --var EDGE_DIRECT:on --var GITHUB_TOKEN:smoke-github --var QUEUE_TOKEN:smoke-queue \
  --var GITHUB_API_ORIGIN:http://127.0.0.1:4898/github --var QUEUE_DB_ORIGIN:http://127.0.0.1:4898/queue
SMOKE_PRIMARY=1 bun run ../scripts/edge-smoke.ts   # in another shell: the real Worker runtime against a local host
```

`HOST_ORIGIN`, `GITHUB_API_ORIGIN` and `QUEUE_DB_ORIGIN` exist for that
last run only: they put a host, GitHub's API and the queue store on
this machine, so that the Worker's own reads are made in its real
runtime against stand-ins. Deployed, none is set: the laptop is what
is behind the hostname, GitHub is GitHub, and the queue store is the
one a machine reports. The two tokens in that command are made up and
open nothing.

## Limits worth knowing

- What only a machine has (sessions, questions, things for the owner's
  hands, units, agents) is its last report. It ages while no machine is
  connected, and the app says how old it is.
- Something filed at the edge still needs a machine for its queue task:
  the issue is filed at once and only the task is held, as
  `prompt.enqueue` or `triage.enqueue`, until one connects. The queue
  task is never written from the edge itself: there is no fleet binary
  there, and copying the queue's rows by hand would break its contract.
  (The fleet's own hosted queue door, `pkgs/fleet/hosted-worker`, is
  where that changes.)
- The edge learns where things are (the repository, the queue, who the
  owner is) from a machine's report, and keeps it. One that has never
  had a report serves nothing.
- GitHub's search allows thirty requests a minute for the token, shared
  with whatever else uses it. Ranked issues are read every three
  minutes and a roadmap page at most once a minute for that reason.
- An action the edge runs itself is never sent to the laptop, so the
  laptop cannot repeat it; an action that was relayed and got no answer
  is only ever resolved by the laptop, which wrote its name down before
  starting. That is the rule against doing anything twice.
- The same rule reaches the device. An action may come with a name the
  device gave it (`id`, a UUID), because a device with no connection
  keeps what the owner did and sends it later, and again if it never
  heard back. What the edge does itself it writes down under that name
  before starting and with its answer after (a week, three hundred
  names, an answer of up to four thousand characters): the name offered
  again is answered from that, marked `again`, and not done again. One
  still being done is answered 409 `running`; one started by an object
  that was put away before it ended is said to be unknown, once, and
  not retried. What goes to the laptop goes under the same name, and
  the laptop keeps the record. Held twice under one name, an action
  waits once. A name is taken by one arrival at a time, here and on the
  laptop, before anything else is waited for: of two that arrive
  together one does it and the other is told it is being done.
- What is done late is done in the order the owner did it. An action
  that may be held goes behind an earlier one about the same thing (the
  same issue, task, reference or set of questions) that a machine has
  not taken yet, even when the machine can be reached again: sent
  straight on it would be done first, and the older one would undo it.
  What is about something else goes straight on. Not covered (#13398):
  an older action held for a machine and a newer one about the same
  thing that the edge does itself.
- The storage is one Durable Object. A section larger than one stored
  value is kept in pieces.
