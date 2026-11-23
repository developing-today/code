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

| Tab      | What it is for                                                                                                             |
| -------- | -------------------------------------------------------------------------------------------------------------------------- |
| Now      | Tasks in progress with their next step, sessions that are moving, and what the owner did lately                            |
| Next     | The queue in the order it will be claimed; move a task first, sooner or later; why tasks are blocked                       |
| Needs me | Open questions (one tap to answer, undo for a few seconds), things only hands can do, reviews                              |
| Ask      | Say what you want in your own words (typed or dictated). It becomes an issue and a top-of-queue task in one pass           |
| More     | Queue by state, pull requests, ranked issues, agents, services, notifications, phones, releases, and the dashboard's pages |

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
laptop's checkout has stopped following `main`.

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

### Merges deploy themselves

`pm-console-redeploy.timer` runs `redeploy.sh` every minute. When the
checkout holds console code that is not the code in service, the new
code is started in a self-check that listens on nothing
(`bun run src/server.ts --check`, with the service's own settings); only
if that passes is it put into service. Code that does not start is left
out, the old code keeps running, and the unit fails, which the Services
screen and notifications show.

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
bash pm-stack/console/install.sh          # user unit pm-console, port 4872, loopback only; and its three timers
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
Sign-in (the Google client) and Laptop away (whether Cloudflare acts
by itself while the laptop cannot be reached; see edge/README.md).

The owner's address is given once with `--owner` and remembered in the
console's environment file; it is not written in the script.

`expose.py` is safe to run again: it changes only what is not already so
and refuses to touch DNS records or Access policies it did not write. It
puts the sign-in gate in place first and the route last, and stops before
the route if the console does not come up with its sign-in settings.

## Configuration

| Variable                                                  | Default                                            | Meaning                                            |
| --------------------------------------------------------- | -------------------------------------------------- | -------------------------------------------------- |
| `PM_CONSOLE_PORT`, `PM_CONSOLE_BIND`                      | `4872`, `127.0.0.1`                                | Listener; the address must be loopback             |
| `PM_CONSOLE_OWNER_EMAILS`                                 | none (nobody)                                      | Comma-separated owner addresses                    |
| `PM_CONSOLE_ACCESS_TEAM_DOMAIN`, `PM_CONSOLE_ACCESS_AUD`  | unset                                              | The Access issuer; set together                    |
| `PM_CONSOLE_ISSUERS_FILE`                                 | unset                                              | JSON list of further issuers                       |
| `PM_CONSOLE_CERT_NAMES`                                   | none                                               | Certificate common names that are the owner        |
| `PM_CONSOLE_DASHBOARD_URL`                                | `http://127.0.0.1:4870`                            | The dashboard behind the gate; loopback, or empty  |
| `PM_CONSOLE_UNITS`, `PM_CONSOLE_PROTECTED_UNITS`          | the stack's units                                  | Units the console manages; units it will not stop  |
| `PM_CONSOLE_ANDROID_PACKAGE`, `PM_CONSOLE_ANDROID_SHA256` | `today.developing.pmconsole`, none                 | The Android app's identity in the asset-links file |
| `PM_CONSOLE_ANDROID_APK`                                  | `~/.local/state/pm-console/android/pm-console.apk` | The signed build the console hands to the owner    |
| `PM_CONSOLE_DEVICES_FILE`                                 | `~/.local/state/pm-console/devices.json`           | Linked phones (hashes of their tokens)             |
| `PM_CONSOLE_EDGE_URL`                                     | unset (reports to nobody)                          | The public address the edge answers on, `https://` |
| `PM_CONSOLE_EDGE_KEY_FILE`, `PM_CONSOLE_EDGE_DONE_FILE`   | `edge-key.json`, `edge-done.json` in the state dir | This host's signing key; what it ran for the edge  |
| `PM_CONSOLE_REPO`, `PM_CONSOLE_QUEUE`                     | `developing-today/code`, `repo-health-code-pilot`  | What it operates on                                |

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
