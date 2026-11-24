# pm-console

The write side of the PM stack, and the gate in front of it. The
dashboard (`pm-aggregator`) shows the fleet; the console acts on it, from
a phone or a browser, at `https://pm.security.cab/console/`.

The whole public hostname is routed here. `/console` is this service.
Every other path is the dashboard's, and the console passes it through
only for a verified owner: the dashboard has no sign-in of its own, so
the check is made at this origin as well as at the edge, and does not
depend on how the edge reads a path. Writes from another site are
refused. Five files a phone fetches before anyone has signed in are
public, matched exactly: `/manifest.webmanifest` and the four icons under
`/icons/`.

## The app

`/console/` is one page with five tabs. The first four are what the
owner does every day; everything else is under More.

| Tab      | What it is for                                                                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Now      | Tasks in progress with their next step, sessions that are moving, and what the owner did lately                                                               |
| Next     | The queue in the order it will be claimed; move a task first, sooner or later; why tasks are blocked                                                          |
| Needs me | Open questions (one tap to answer, undo for a few seconds), things only hands can do, reviews                                                                 |
| Ask      | Say what you want in your own words (typed or dictated). It becomes an issue and a top-of-queue task in one pass                                              |
| More     | The plan (roadmaps, bugs, intake), queue by state, pull requests, ranked issues, agents, messages, services, notifications, phones, releases, and the dashboard's pages |

The page never waits on a live fan-out. The server keeps three documents
ready (`core`, `questions`, `ops`), each built from sections that are
read in the background and served from memory with the time they were
read (`GET /console/api/doc/<name>`, ETag, gzip). Every change goes
through one endpoint, `POST /console/api/act {kind, args}`; the kinds are
the table in `src/actions.ts`.

| Source                                    | Read by                                 | Written by                                            |
| ----------------------------------------- | --------------------------------------- | ----------------------------------------------------- |
| The queue store                           | `src/work.ts` (one round trip)          | `fleet queue`; one priority update for "move"         |
| GitHub issues and pull requests           | `fleet gh`                              | `fleet gh`                                            |
| The dashboard's questions and needs lists | the dashboard's own JSON and store file | the dashboard's submit endpoint, with the set's token |
| Agents and messages                       | `fleet agent`, `fleet bridge`           | `fleet bridge`                                        |
| The stack's units                         | `systemctl --user`, `journalctl --user` | `systemctl --user`                                    |

### Notifications

Web Push, sent by this service (`src/push.ts`: RFC 8291 and RFC 8292
with WebCrypto, keys in `~/.local/state/pm-console/push.json`). A device
subscribes under More, Notifications, and chooses what reaches it: a
reply to something asked, something new that needs the owner, new
questions, a failed unit. The Android app shows them as its own.

The screen says "On for this device" only when the sender holds this
device too: a browser that said yes while the sender refused or lost its
address is shown as not finished, with one button to finish. It also
says what came of the last notification: how many devices the push
services took it for, and which service refused and with what status.
"Took" is the word, because a push service accepting a message is not
the phone showing it.

Addresses are accepted only at the push services browsers really name:
Google's (`fcm.googleapis.com`, and `jmt17.google.com`, which a Chrome
off the stable channel gives), Mozilla's, Apple's and Microsoft's.

A unit that runs on a timer (a service with a timer of the same name,
such as `pm-git-sync`) is treated differently from one that stays up.
Between runs it shows as failed whenever its last run failed, so one bad
run that the next run mends says nothing; five minutes of failing run
after run is reported once ("A unit keeps failing", with what the unit
is for), and so is its mending. That is what tells the owner when the
laptop's checkout has stopped following `main`, and, for the console's
own copy of `main`, when `pm-console-redeploy` cannot put it into
service.

### The plan: roadmaps, bugs, intake

Three screens under More, all from `src/plan.ts`, and all read from
where the plan already lives: nothing is copied into a store here.

- **Roadmap**: one page per capability (id, fleet, atlas and PM stack
  first, then every other file in `planning/roadmaps/`). A page is the
  file's own rows and the capability's issues, in five groups: in
  progress, planned immediately, short term, long term, built. An
  issue can be moved between the planned groups (its rank, at once); a
  row or an issue can be given a note; a row can be changed (moved,
  given a sub-item, marked built, rewritten, dropped), and an item
  added.
- **Bugs**: report one in a sentence; the open ones soonest first; the
  ones fixed lately.
- **Intake**: say something that is about nothing in particular, see
  where each kind goes, and see everything sent from the app as GitHub
  has it now.

Everything said on these screens is one action, `intake.send`: the
words as typed, what they are about, how soon, and whether an issue or
a session should take them. Where it goes is one table (`ROUTES`):

| Kind              | Becomes                                                                    | Labels                                                       | Rank unless said |
| ----------------- | -------------------------------------------------------------------------- | ------------------------------------------------------------ | ---------------- |
| bug               | an issue and its triage task                                               | `kind/bug`, `source/human`, `owner/intake`, the capability's | now              |
| note              | a comment on the issue it is about, and a task; else an issue and its task | `source/human`, `owner/intake`, the capability's             | next             |
| change            | an issue saying what to change, and its task                               | `source/human`, `owner/intake`, the capability's             | next             |
| any, as a session | the owner's prompt (as on Ask), which starts a session                     | the prompt's own, and those above                            | expedite         |

The words are the first thing in whatever is filed, untouched; what the
app adds comes after a line. They are not written to the audit log.
`owner/intake` is how the Intake screen finds them again.

A page costs GitHub three requests (the file, the open issues that are
being worked on, ranked or epics, and what closed in two weeks) and is
read at most once a minute, since GitHub's search allows thirty a
minute and the fleet shares them. The refresh button asks again at
once; so does anything sent from the app.

### Releases

More, Releases says what is out and lets either be put out again. A
merge already does both by itself; this is for looking, and for doing it
over.

- **This site**: which commit Cloudflare serves and whether the laptop
  runs the same (from the edge's health answer), and the last deploys as
  GitHub records them. "Deploy again" starts the deploy workflow on
  `main`; "From nothing first" has it stand a throwaway copy of the
  Worker up from nothing, check it and remove it before deploying.
- **Android app**: the build that is released, the last builds, a
  download, and "Build again".

The runs are read, and a run is started, through the laptop
(`fleet gh`); with the laptop away the screen still says what is being
served and what is released. Starting one is never held to be done
late.

### Phones

The Android app's home-screen widget reads a short summary (three counts
and the top line of each list) with a token of its own, since it cannot
use the browser's sign-in. Linking passes no secret through an address:
the phone shows the signed-in owner a hash, the owner approves it under
More, Phones, and the phone then presents the value behind the hash to
`POST /console/device/link` and is given its token once. The store
(`devices.json`) keeps hashes of tokens. `GET /console/device/summary`
answers a linked phone and nobody else; both paths are limited, and they
are the only ones under `/console` that answer without the sign-in.

Every operation is an argument vector, never a shell line, and text you
type travels on stdin or as one argument.

Each write leaves two lines in `~/.local/state/pm-console/audit.jsonl`
(who, what, when; message text is not copied): one before the action
runs, one with how it went. If the first cannot be written the action is
not taken. If the second cannot be written the action's own result still
comes back, so a merge that happened is never reported as failed.

Work filed from Priorities is created as an issue and enqueued as a
triage task in the same pass, so a worker picks it up without waiting for
the coverage reconciliation. The rank sets the task's place in the queue.
If the enqueue fails, the issue still exists and the page says it is not
queued.

### Served from the edge

The app is served by a Cloudflare Worker in front of the hostname
([`edge/`](edge/README.md)), so it opens and works whether or not this
host can be reached. This service stays what it is, the machine with the
fleet's tools on it, and becomes a worker behind the edge:

- It reports its state to the edge (`src/sync.ts`): the sections that
  changed, signed with a key made here whose private half never leaves.
  The edge learns the public half by asking this host through the tunnel
  (`GET /console/edge/key`).
- An action that needs this host is relayed to it at once. One taken
  while it could not be reached is held at the edge if it is safe to do
  late, and run here at most once when it is back, recorded in the audit
  log as `via: edge-held`.
- Every response this service writes carries `X-PM-Console: 1`, so the
  edge can tell "the host answered, with an error" from "the host could
  not be reached".

Set `PM_CONSOLE_EDGE_URL` to turn the reporting on; `scripts/expose.py`
does that when it sends the hostname through the Worker. Without it this
service reports to nobody and serves the app itself, as before.

### Machines

This host is a machine, and the platform treats it as one of however
many there are. A machine is anything that reports to the edge under a
key the edge knows. In every report it says what it is called
(`PM_CONSOLE_MACHINE`, or the host's own name) and what it can do: the
kinds of action it has, and the sections it reads. Nothing about a
machine is assumed from its being "the laptop".

More, Machines shows:

- **Cloudflare's own part**: whether it does the core itself, and what
  it was turned away for lately.
- **Each machine**: whether it is reporting and when it was last heard
  from, the commit it runs, and what only it reports and only it can
  do, in words. What Cloudflare does itself is not counted as the
  machine's.
- **What waits for a machine**: everything the owner did that is held
  until one is back, oldest first, each with "Take back" (`held.drop`).
  Taken back, it is recorded as not sent under its name, so the device
  that sent it shows it under "could not be done"; offered again by a
  device that never heard it was kept, it stays taken back.

The screen keeps itself current while it is open: a machine going
quiet or coming back, or something new waiting, draws it again.

The edge answers that screen itself, so it is there with every machine
away. Served straight from a machine, the machine answers with itself.

One machine can report today: the one the tunnel leads to, whose key is
the only one the edge takes reports under. Enrolling another under its
own key, and sending each action to a machine that can do it, are not
built.

### Platform

More, Platform lists every Cloudflare Worker the repository holds, as
one platform: where each lives, what it does, whether it is in service,
whether it is deployed from Actions or by hand from a machine, how its
last deploy went, and what joining the platform means for it. It is
drawn from the repository's own list
([`../platform/workers.json`](../platform/README.md)), read from GitHub
with the latest run of each deploy workflow, so the edge answers it
with no machine and the list changes with a merge.

### Appearance

**More → Appearance** is where the app is made to look the way its owner
wants, on the device in hand:

- **Colours.** 33 themes: the two the app always had (Slate and Paper),
  eight **true black** ones whose page and panels are `#000000` (an OLED
  screen leaves those pixels off), three **high contrast** ones (text at
  15 to 1 or more, heavier edges), and dark and light palettes many
  people know from their editors. The app either follows the system
  between a theme for light and a theme for dark, each of the owner's
  choosing, or wears one theme always.
- **Typeface.** 22: the device's own sans, serif and fixed-width, and 19
  the app carries (`public/fonts/`, each under the SIL Open Font License,
  its licence beside it in `public/fonts/licences/`). Among them are four
  made to be easy to read (Atkinson Hyperlegible, Lexend, OpenDyslexic,
  Comic Neue). A typeface is fetched when first shown and kept for when
  there is no connection; the ones not chosen cost nothing.
- **Size and spacing.** Seven sizes of text, five amounts of room around
  things, four kinds of corner, and whether anything moves.

The choices are kept on the device (`localStorage`, key `pm.look`), not
with the account: a phone and a desk can each look their own way. They
are put on the page before it is first drawn.

How it is made, for adding to it:

- Each choice is **one attribute on the root element** (`data-theme`,
  `data-font`, `data-size`, `data-space`, `data-corners`, `data-motion`),
  put there by `public/look.js`. Nothing writes a style, so the page's
  content policy stays `default-src 'self'`.
- **A new theme** is a block of thirteen colours in `public/themes.css`
  and a line in `THEMES` in `public/look.js`. The tests hold every theme
  to the contrast rules written at the top of `themes.css`, so one that
  cannot be read does not merge.
- **A new typeface** is its file in `public/fonts/` with its licence, an
  `@font-face` and a `[data-font]` rule in `public/look.css`, a line in
  `FONTS`, and a line in `public/files.json`.
- The style sheet (`public/app.css`) is drawn entirely from those
  settings: colours only through the theme's variables, padding, margins
  and gaps through `--space`, corners through `--round`, edges through
  `--edge`. A rule that writes its own colour or a bare pixel padding
  fails a test.

### Trends

**More → Trends** is what got done, by day, over 7, 14, 30 or 90 days,
each set against the same time before it:

- pull requests merged, by what each is (a feature, a fix, docs, tests);
- features merged by project (id, Fleet, Atlas, PM stack, mcpx), with
  each project's latest features listed and linked;
- issues opened against issues closed, and bugs reported against fixed;
- how long a pull request waits from opened to merged (the middle one
  of each day);
- tasks the queue finished, by kind.

Whoever serves the app reads it, and no machine is needed: Cloudflare
asks GitHub and the queue's store itself (`src/trends.ts`).

- **GitHub's history is read once and kept.** GitHub has no "merged per
  day" to ask for; its list of issues takes a time and answers with
  everything changed since, oldest change first. It is read newest
  first, in stretches: the last day to the end of the list (and from
  then on kept up by asking from where it stopped), then three days
  back, a week, two weeks, a month, two months, ninety days. The screen
  says from which day on the counts are whole. What is kept is small
  (per day: the numbers of what was opened and closed, and for a merged
  pull request what its title says it is), in the Worker's own storage.
- **What a pull request is comes from its title**: `feat(pm-console): …`
  is a feature of the PM stack; `id-pg: …` is counted under id but not
  as a feature; a title that is only words is "everything else".
- **It reads on in the background while somebody looks.** For a day
  after the screen was last opened, Cloudflare's own clock reads the
  history on (six pages a minute until it is whole, then what changed,
  every half hour). After that it does not wake for it; the next
  opening reads on at once.
- **Tasks come from the queue's store each time**: one read of the
  finishing events by day, by the index the Done screen uses.

Days are UTC days, and today's column is drawn fainter: it is not over.

### Reports

**More → Reports** is the same history in words: a day, a week or thirty
days, with what merged and how that stands against the time before,
the features by project (the latest three of each to open), issues
opened and closed and what that did to the backlog, bugs, and the
queue's finished tasks (and, in a report that ends today, the queue as
it stands). **Earlier** and **Later** step it back and forth by its own
length, as far back as the history goes, so the report for a week gone
by can be made at any time. **Copy** and **Share** give it as text.

A report is made when it is asked for, on the device, from the Trends
read (`public/pages/reports.js`; the read takes a day to end on). It
needs no machine, and the service worker keeps each one asked for, so
it opens again with no connection. A report says when its numbers are
not whole yet. For anything these do not cover, a question on Ask is
answered by an agent and stays there to read.

### Charts, and pages of their own

`public/charts.js` is a small kit any screen can use: columns stacked by
what they are made of, lines, and the tiles a screen leads with. Its
rules are written at its top and held by its tests: colour only says
which series a mark is (six theme colours in a fixed order, checked for
people who see colour differently, and never the status colours); words
are never in a series colour; two or more series always have a legend;
one scale to a chart; pointing at, tapping or arrowing to a day reads
out every series for it; and every chart has its numbers as a table.

A screen can be **a module of its own** in `public/pages/`: a file that
exports its address, title, the part of More its tile goes in, and a
`view` that returns elements. The app gives it its route, its tile and
its offline copy, and hands it the app's own parts (reading, acting,
moving about). **To add a page:** write `public/pages/<name>.js`, add it
to `public/pages/index.js`, and name it in `public/files.json`; the
tests check the three agree. Trends is the first, and the example.
Elements are built with `h` (and `s` for drawings) from `public/ui.js`
and the classes in `app.css`, so a new page looks like the rest.

### The app's files

`public/files.json` is the one list of the files the app is made of. The
laptop's service, the Worker, the Worker's build and the service worker
all go by it: **a new file for the app is one line there** (its path,
what it is served as, and when the service worker keeps a copy:
`installed` for what the page needs to open, `used` for what is fetched
on demand, such as a typeface). A file in `public/` that the list does
not name is not served.

### With no connection

The app opens and can be used with no connection, in a browser and in
the Android app alike (which is the same site in Chrome).

- **It opens.** The service worker (`public/sw.js`) keeps the app's own
  files, fetched when it is installed.
- **It shows what the device last saw.** The worker keeps the last
  answer to everything the app reads: the three documents and every
  screen's own read, each under the address it was asked for. The
  network is always asked first; a copy is shown only when the network
  gives no answer (or is too slow to wait for), and is marked as a copy.
  Keeping copies is on a best effort: a device with no room, or a
  browser that gives the site no cache, still gets the network's answer.
  An answer that says no (a refusal, a way to the sign-in page, an
  error) is passed on as it is and never papered over. The corner of
  the header then says `offline` and how old the state is, and a line
  under the header says there is no connection. A screen the device
  has never seen says it has no copy of it. More, On this device has
  "Keep a copy of every screen now", for before a flight.
- **What the owner does is kept and sent once.** Every action is given
  a name on the device (a UUID) and written to the browser's own
  database before it is sent (`public/outbox.js`), and before it waits
  its turn behind a send already under way, so a page closed meanwhile
  loses nothing. One try is given a minute, and an answer that stops
  arriving part-way is no answer (it is offered again, never taken for
  a sign-in that ran out). With no connection it
  waits there, and is sent when the connection is back: by the page if
  the app is open, by the service worker if it is not (Background Sync,
  where the browser has it). The platform does an action once under its
  name however often it arrives, so sending again after an answer that
  never came is safe: Cloudflare keeps the names it did (a week, three
  hundred), and a machine keeps its own (`src/named.ts`). A name is
  taken by one arrival at a time: a second one while the first is still
  being done is told so, and asks again. And a name keeps to the side
  that first took it, Cloudflare or a machine, whichever would be
  chosen for it later.

  On Android the app is the same site in Chrome, so the same worker and
  the same outbox run there — the difference is only the page's life.
  Swiping the app away closes the page, not the worker: the sync stays
  registered, and Chrome wakes the worker when the network is back, so
  an answer made offline is still sent with the app closed. There is no
  deadline in the app: the sync stays registered until it runs, or the
  browser says it is its last call for it, and then opening the app
  sends what waits. The one thing the page owns is the few seconds an
  answer waits before it is written down (the undo pause): if the app
  is closed in that pause, the page tries to write it as it hides, and
  if that write does not reach the outbox the answer is lost and the
  question stays open. The `pm-console-android-offline` workflow
  checks this every night and on demand: an emulator, the app against
  the demo edge, aeroplane mode, an answer made offline, the app
  swiped away, and the answer arriving exactly once when the network
  is back.

- **"Sent" means recorded.** An answer the platform took and did not
  record (a question that was settled in the meantime) is not counted
  as sent: it is listed under "could not be done", with why. A question
  is left out of the list while its answer is kept and not yet
  recorded, across a reload too, and is shown again if the answer is
  taken back or does not count.
- **What is being typed is not lost to a send in the background.** A
  bug report, a note or a message half written stays where it is when
  something kept earlier gets sent and the screen is drawn again; it is
  let go when it is sent.
- **In the order it was done.** Something done now goes behind whatever
  is already waiting, oldest first, and is not sent ahead of it. Two
  about the same thing (an issue ranked twice, a task moved twice) keep
  their order to the end: while the earlier one has not been taken, the
  later one waits, on the device and, for what a machine does late, on
  the platform too. What is about something else is not held up by it.
- **Nothing waits unseen.** Where the browser wakes the worker with the
  app closed and something still could not be taken, the worker asks to
  be woken again; on the browser's last call it says what still waits,
  and opening the app sends it.
- **Not everything is sent unasked.** What the owner says or decides
  (a prompt, a reply, a rank, a note, a bug, an answer) does not go
  stale and goes whenever it can. A merge, a deploy, a cancel, an
  unblock and a service action act on something that may have changed:
  they are kept too, sent by themselves only within ten minutes of
  being asked for, and after that wait under "kept back" until the
  owner looks again and says to send them. A merge also carries the
  commit the owner was shown, and GitHub merges only that. What sets
  the connection up or carries a secret (notifications, phones,
  sign-in, the Cloudflare switch) is never kept: it says it needs a
  connection.
- **A sign-in that ran out is not "offline".** The app tells the two
  apart by asking past the worker without following the way to the
  sign-in page. Signed out, it shows what it last saw, keeps what is
  done, and says to sign in again; signing out by hand makes the device
  forget what it last saw.

More, On this device lists what is waiting, kept back, could not be
done, is with the platform for a machine, and was sent; each can be
dropped, sent, or tried again there.

### Merges deploy themselves

On a machine the console runs from a copy of `main` that is its own,
under `~/.local/share/pm-console`:

| There                | What it is                                                                        |
| -------------------- | --------------------------------------------------------------------------------- |
| `repo/`              | a clone that fetches only `main` from GitHub, and only the console's part of it   |
| `releases/<commit>/` | the console's files at one commit of `main`, with `.release` saying which         |
| `current`            | a link to the release in service: what every one of the console's units runs from |

Nothing else works there. (Until 2026-11-24 the console ran from the
checkout the fleet's lanes work in, and was whatever that checkout held:
`main`'s code from the day before, or another branch's. #13833.)

`pm-console-redeploy.timer` runs `redeploy.sh` every minute. It asks
GitHub for `main`. When the console's code there (or one of its units,
or the dashboard's list of pages) is not what is in service, that
commit's files are laid out as a new release and started in a
self-check that listens on nothing (`bun run src/server.ts --check`,
with the service's own settings). Only if that passes is the release
made current and its own `install.sh` run, which installs the units,
restarts the service and waits for it to answer its health check as
that commit. A release that does not start, or does not come up, does
not stay: the one before it is put back, and the unit fails, which the
Services screen and notifications show. With GitHub out of reach
nothing changes, and after half an hour of that the unit fails too.
The newest releases are kept, always including the one in service and
the one before it.

## Sign-in

The console runs no sign-in of its own. A caller arrives with a signed
token from an issuer the deployment trusts; the console checks the
signature, issuer, audience and times, then asks whether the identity is
the owner (`PM_CONSOLE_OWNER_EMAILS`).

- **Today:** Cloudflare Access at the public edge runs the interactive
  sign-in and issues the token. The sign-in methods are set on the Access
  application, not here.
- **Google:** an Access sign-in method, offered beside the emailed code.
  It needs an OAuth client, which Google lets only a signed-in person
  make. The app walks through it under More, Sign-in: it shows the two
  addresses to give Google, takes the client ID and secret, and hands
  them to `scripts/expose.py --google-stdin`, which gives them to
  Cloudflare. They are kept nowhere else: not in a file, a log, the
  record of actions, or (the action is never held) the edge. The same
  screen replaces the client later. `expose.py` also turns Google on by
  itself if it finds the secrets `google.access-sso.client-id` and
  `google.access-sso.client-secret`.
- **The central OIDC provider (#9077):** either an Access sign-in method
  (generic OIDC), or trusted directly: add
  `{name, issuer, jwksUri, audience}` to the file named by
  `PM_CONSOLE_ISSUERS_FILE` and its ID tokens are accepted as
  `Authorization: Bearer`. RS256 and ES256.
- **Certificates from the central CA (#9155):** upload the CA to Access
  mTLS; the token then carries the certificate's common name, and names
  listed in `PM_CONSOLE_CERT_NAMES` are the owner.

A request is local, and treated as the owner, only when the connection
itself comes from this machine and it crossed no proxy (loopback Host, no
forwarding headers). The connection's address is the test, since headers
are the caller's to write; for the same reason the console only listens
on loopback and refuses to start on any other address. Everything else
needs a token. Outside the install files above, these answer without
one: `/console/health`, `/.well-known/assetlinks.json` (the Android
app's link to this origin), and the two phone paths under
`/console/device/`, which check a linked phone's own token instead.

## Run

```bash
bash pm-stack/console/redeploy.sh         # from any checkout, once: makes the console's own copy of main and installs
                                          # user unit pm-console (port 4872, loopback only) and its three timers from it
python3 pm-stack/console/scripts/expose.py --owner you@example.com   # first time: Access, console env, tunnel route, DNS, the edge's route
python3 pm-stack/console/scripts/expose.py --check   # what public exposure would change
python3 pm-stack/console/scripts/expose.py           # later runs (the timer's): the owner is remembered
python3 pm-stack/console/scripts/expose.py --status  # which sign-in methods are on, as JSON; changes nothing
python3 pm-stack/console/scripts/expose.py --edge off   # hostname straight to this host, not through the edge
cd pm-stack/console && bun test
bun run scripts/demo.ts          # the app with made-up state, on this host alone
bun run scripts/edge-demo.ts     # the same behind the edge
```

The app's own switches for what this script sets up are under More:
Sign-in (the Google client) and Cloudflare (whether Cloudflare does the
core itself; on unless turned off; see edge/README.md).

The owner's address is given once with `--owner` and remembered in the
console's environment file; it is not written in the script.

`expose.py` is safe to run again: it changes only what is not already so
and refuses to touch DNS records or Access policies it did not write. It
puts the sign-in gate in place first and the route last, and stops before
the route if the console does not come up with its sign-in settings.

## Configuration

| Variable                                                  | Default                                            | Meaning                                               |
| --------------------------------------------------------- | -------------------------------------------------- | ----------------------------------------------------- |
| `PM_CONSOLE_PORT`, `PM_CONSOLE_BIND`                      | `4872`, `127.0.0.1`                                | Listener; the address must be loopback                |
| `PM_CONSOLE_OWNER_EMAILS`                                 | none (nobody)                                      | Comma-separated owner addresses                       |
| `PM_CONSOLE_ACCESS_TEAM_DOMAIN`, `PM_CONSOLE_ACCESS_AUD`  | unset                                              | The Access issuer; set together                       |
| `PM_CONSOLE_ISSUERS_FILE`                                 | unset                                              | JSON list of further issuers                          |
| `PM_CONSOLE_CERT_NAMES`                                   | none                                               | Certificate common names that are the owner           |
| `PM_CONSOLE_DASHBOARD_URL`                                | `http://127.0.0.1:4870`                            | The dashboard behind the gate; loopback, or empty     |
| `PM_CONSOLE_UNITS`, `PM_CONSOLE_PROTECTED_UNITS`          | the stack's units                                  | Units the console manages; units it will not stop     |
| `PM_CONSOLE_ANDROID_PACKAGE`, `PM_CONSOLE_ANDROID_SHA256` | `today.developing.pmconsole`, none                 | The Android app's identity in the asset-links file    |
| `PM_CONSOLE_ANDROID_APK`                                  | `~/.local/state/pm-console/android/pm-console.apk` | The signed build the console hands to the owner       |
| `PM_CONSOLE_DEVICES_FILE`                                 | `~/.local/state/pm-console/devices.json`           | Linked phones (hashes of their tokens)                |
| `PM_CONSOLE_EDGE_URL`                                     | unset (reports to nobody)                          | The public address the edge answers on, `https://`    |
| `PM_CONSOLE_EDGE_KEY_FILE`, `PM_CONSOLE_EDGE_DONE_FILE`   | `edge-key.json`, `edge-done.json` in the state dir | This host's signing key; what it ran for the edge     |
| `PM_CONSOLE_MACHINE`                                      | the host's own name                                | What this machine is called where machines are listed |
| `PM_CONSOLE_REPO`, `PM_CONSOLE_QUEUE`                     | `developing-today/code`, `repo-health-code-pilot`  | What it operates on                                   |

A malformed port, a listen address or dashboard address that is not
loopback, an edge address that is not plain `https://host`, or a
malformed issuer stops the process at boot and names the variable.

## Android

`pm-stack/console/android/` shows the same site in Chrome without browser
chrome (a Trusted Web Activity), so sign-in is Chrome's own and the app
never falls behind the site. Around that it uses what only an installed
app has: a home-screen widget, dictation from a shortcut or a
quick-settings tile, text shared or selected in any app, long-press
shortcuts, and notifications shown as its own. The `pm-console-android`
workflow builds it on every change to it. Once the owner has given
GitHub the signing key, the workflow signs and releases each build by
itself, and More, Phones offers the newest release to the signed-in
owner at `/console/pm-console.apk`, so the phone gets the app from the
site it opens. Until then the host signs, and its build is what is
offered. See its README.

Because it is the site, it opens with no connection and keeps what is
done there exactly as the site does (above, "With no connection"): text
dictated or shared into it with no signal lands in Ask, is kept when
sent, and goes when the phone is connected again. The Android half of
that is checked on a device in CI, not only in a desktop browser: the
`pm-console-android-offline` workflow starts an emulator, installs the
built APK, and drives it against the demo edge with the network off
and on again.
