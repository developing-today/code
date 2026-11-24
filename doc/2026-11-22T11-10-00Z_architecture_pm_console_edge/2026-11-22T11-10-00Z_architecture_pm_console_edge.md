# PM console served from the edge

Status at writing: built and tested, about to be deployed. Later
sections record what happened when it went live.

## What was asked

The owner, 2026-11-21 23:58 CST: the app's pages "still don't load
correctly", and "making everything work inside of the PM stack from
Cloudflare without requiring the laptop is completely within your
purview". Then, 2026-11-22 04:32 CST: "continue fully implementing this
system and relying on cloudflare instead of laptop for operations where
possible".

## What was wrong

The app (`pm-stack/console`) ran only on the laptop, behind a Cloudflare
tunnel. Measured on 2026-11-22:

- The laptop's uplink dropped every tunnel connection at 04:58:53Z and
  flapped until 05:14 (evidence on #12414). During that time nothing at
  `pm.security.cab` loaded at all.
- The laptop ran at load average 47 to 75 on 16 cores while the fleet
  compiled, which starved the management services (`CPUWeight=900` was
  set on them in #12452).

A service on one laptop is as available as that laptop's uplink and
processor. The app should not be.

## Decision

Put a Cloudflare Worker in front of the hostname and serve the app from
it. The laptop stays the machine with the fleet's tools, and becomes a
worker behind the edge.

1. **The edge serves** the app's files, its state (from a copy the
   laptop reports), notifications (keys, subscriptions, sending) and the
   linked-phone summary.
2. **The laptop reports** the sections of state that changed, signed
   with its own key, and says how current each unchanged section is.
3. **An action that needs the laptop is relayed at once** when it is in
   reach. When it is not, an action that is safe to do late is held at
   the edge and run once when the laptop is back; one that is not safe
   (a merge, a unit operation, a change to a task's state) is refused.
4. **The edge never refuses on its own.** It answers when it is sure of
   the owner; otherwise it passes the request to the laptop. With the
   laptop out of reach an unsure caller gets a plain notice and nothing
   else.
5. **Deploys run from GitHub Actions**; the laptop's `expose.py` only
   decides whether the hostname goes through the Worker, and can take it
   off again.

## Why this shape

| Choice                                                      | Instead of                                              | Reason                                                                                                                                                                                                                               |
| ----------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Laptop pushes state to the edge                             | Edge pulls from the laptop                              | A push needs the edge to recognise the laptop (a public key). A pull needs the laptop to recognise the edge (a secret kept at the edge).                                                                                             |
| The laptop's key, fetched through the tunnel                | A shared secret in Bitwarden and in the Worker          | Nothing to store, rotate or leak. Only the laptop can answer through its tunnel.                                                                                                                                                     |
| Edge config (owner, issuer, audience) arrives in the report | Worker variables                                        | One place it is set, and nothing private in a public repository.                                                                                                                                                                     |
| Serve from the copy, tell the laptop someone is looking     | Ask the laptop first and fall back                      | The copy answers in milliseconds whatever the laptop's load; asking first makes every read as slow as the laptop.                                                                                                                    |
| Held actions limited to a named list                        | Hold everything                                         | A merge or a unit operation acts on what the owner saw. Done hours later it may act on something else.                                                                                                                               |
| At most once for a held action                              | At least once                                           | A second prompt or a second comment is worse than one reported as interrupted.                                                                                                                                                       |
| Defer to the laptop when unsure                             | Refuse at the edge                                      | A fault in the edge's check must not lock the owner out. The laptop's check is the one already proven.                                                                                                                               |
| One Durable Object                                          | KV or D1                                                | One writer, small state, an alarm for the silent-laptop check, and consistent reads straight after writes.                                                                                                                           |
| `wrangler deploy` from CI                                   | The `cf` versions flow under Nix used by message-bridge | The Worker has no secrets, so a code-only deploy is the whole deploy, and it could be run and checked on the build machine before it was written into CI. Moving to the `cf` flow is a packaging change with no change of behaviour. |

## What the laptop trusts

The laptop used to act only for a caller with a sign-in token it had
verified. It now also runs the held actions in the answer to its own
report. That answer comes over TLS from the hostname's Worker, in the
owner's Cloudflare account, which verified the owner's token when the
action was taken. An attacker who controls that account already controls
the sign-in in front of the laptop, so this adds no one to the set of
parties the laptop relies on. The laptop still limits what it will run
late to the named list, runs each at most once, and records each with
who took it and when.

## What this does not do

- With the laptop off, state does not go on changing at the edge: it is
  the last report, with its age shown. Reading the queue store and
  GitHub straight from the edge would fix that for the queue and for
  prompts; it needs those two credentials at the edge and is a separate
  step.
- Questions and their answers still live in the laptop's decision store.
  An answer given with the laptop away is held, not applied.
- The dashboard's own pages are the laptop's. With it away they are a
  notice with a link to the app.

## How it was checked before going live

- `bun test` in `pm-stack/console`: the Worker's real routing and state,
  with the laptop's real handler behind them and the laptop's real link
  reporting through the same routing. Stand-ins only at the outside
  (storage, the sign-in issuer's key, the push service).
- A mutation check: 37 planted faults in the edge, the routing, the link
  and the protocol. 36 were caught. The one left (dropping a length
  check on a signature) changes no behaviour: verification refuses a
  short signature anyway.
- One of those faults found a real defect: a section that had not
  changed looked stale at the edge while the laptop was reporting
  normally, because only changed sections carried a time. The report now
  says how current every section is.
- The Worker in the real runtime (`wrangler dev`) against a local host:
  17 checks, including the signed report, the key fetch, a 2 MB section
  kept in pieces, and the host going away.
- The app driven in Chrome through the edge code: open, send a prompt
  with the host up, take the host down, send another (held), bring it
  back (sent once, shown as sent late).

Not checked before going live: the owner's real sign-in token at the
edge. The edge counts requests it answered as the owner's and requests
it left to the laptop (`/console/health`), so the first real visit shows
which happened.

## 2026-11-22T11:50Z Update: going live, and the first review

Two things stopped the first go-live, and neither was in the design.

- The laptop's checkout had stopped following `main` two hours earlier.
  A system rebuild had left an empty marker commit on the local `main`,
  and the sync only ever fast-forwards, so it stood still, 28 commits
  behind, and nothing merged since had been deployed. The marker was
  kept under the tag `rebuild-marker/amd-569` and `main` was moved to
  `origin/main` (nothing was discarded: the commit was empty and the
  tree was clean). The sync still cannot do this by itself; that is a
  separate change to `pm-git-sync`.
- The setup script judged the Worker "not deployed" while it was
  answering. Its health check went out under Python's default client
  name, which Cloudflare refuses with a 403; `curl` from the same
  machine got the answer. The check now names its program (and nothing
  else), asks three times before believing a failure, and is covered by
  a test that looks at the request actually made.

What the first review found, and what was done:

- **An action could run twice.** If the laptop did a relayed action and
  its answer was lost on the way back, the edge held the action under a
  new name and the laptop ran it again at the next report. An action is
  now named at the edge before it goes anywhere; the laptop writes the
  name down before starting and the outcome after; a held action
  offered under a name the laptop knows is answered from that record,
  and one still running is left to finish. Tested with the answer lost
  after the work, and with the edge giving up while the work was still
  going.
- **A source the laptop could no longer read looked healthy at the
  edge.** The laptop keeps its last good copy, so the data and its
  fingerprint do not change; the report now says of every section
  whether its latest read failed and why, and the edge shows that with
  the copy, across its own restarts.
- **A late answer the dashboard refused was logged as done.** It is now
  logged as not recorded, with the reason.
- **Notifications after the move.** A browser's subscription is made
  for one sender's key, and the edge has its own. The app now checks, on
  opening, whether the sender still holds this device and under which
  key, and makes the subscription again or hands it over again without
  asking (the owner already said yes on that device). The sender tells
  the app which devices it holds by a short mark of each address, never
  the address. Until a device next opens the app, the laptop goes on
  sending to it. Phones linked for the widget are not carried across an
  edge on/off change yet: one linked on one side has to be linked again
  on the other.
- **The setup script**: `--check --edge off` now previews that choice
  instead of ignoring it; a Worker built for another hostname is not
  given this one (the Worker says which it is built for); the redeploy
  check reads quoted settings the way the service does; the build and
  the smoke check refuse arguments they do not take.

One change nobody asked for: the contact handed to push services with
every notification was the owner's address. It is now the project's
page. Delivery does not depend on it, and an address has no reason to
travel to Google or Mozilla with each message.

## 2026-11-23T00:20Z Update: the owner's switch, the report's limits, the build's copy

Direct mode (the edge reading and acting by itself while the laptop is
away) was built by the fleet in #12579 and deployed switched off. What
was added around it since:

- **The switch is the owner's, in the app** (#12712). More, Laptop away
  says what the mode covers and what turning it on hands over: the
  repository automation token and the queue store token, as secrets of
  the Worker. It runs `expose.py --edge-direct` on the laptop, where
  the tokens are. The script now sends them through Cloudflare's API
  rather than a tool that had to be installed; turning the mode off
  takes them away again; an attempt that fails part-way leaves the
  Worker holding nothing; a mode that is on gets back what the Worker
  has lost; and runs take turns, so the timer cannot undo what the app
  is doing.
- **Reports are refused unread** when they say they are too large or
  are not shaped like a report, and an address with twenty refused
  reports in a minute is refused unread until the minute is out
  (#12715). A report the laptop signed that is merely wrong is not held
  against its address.
- **The edge says which commit the laptop runs** (`hostVersion` in
  `/console/health`, #12715). That a merge has reached the laptop can
  be seen from anywhere. It was first used to confirm exactly that when
  the session had no other way to look at the laptop.
- **A unit that keeps failing on its timer is reported**, once, with
  what it is for, and so is its mending (#12719). This is what was
  missing when `pm-git-sync` failed for two hours unseen.
- **The signed Android build is kept at the edge**, sent once per
  build in the laptop's report and kept only if it matches its
  fingerprint. The download is served from that copy when it is the
  newest build or the laptop is away.

Still open, and why:

- Direct mode is off until the owner turns it on. Its check against
  the real queue store and GitHub runs when it is turned on and shows
  on the same screen; the five-minute laptop-away check in #12518 has
  not been done.
- A rebuild commit that changes `flake.lock` still leaves the laptop's
  `main` ahead and behind at once (#12520). It is now reported within
  five minutes instead of never; carrying the commit to `main` without
  a person is not built, because it has to be tried on the laptop.
- The deploy still uses `wrangler deploy`. The `cf` release flow is
  staged in #12561 and needs two protected environments with their own
  tokens before it can run.
- The Android project is still on androidbrowserhelper 2.5.0.

## 2026-11-23T01:25Z Update: the build never holds up a report

The last three commits of the Android-build change (#12723) merged
before any reviewer read them, so they were read again afterwards. Two
real faults came out of that: one in how the build travels, one in how
the edge keeps it.

A report that carries the build is megabytes; every other report is
kilobytes. The laptop gives a report twenty seconds. On a connection
too slow to send the build in that time, the report failed, and the
next was built the same way and failed too, for as long as the edge
still lacked the build. In that time no report arrived at all, so the
edge took the laptop for gone while it was there. Nothing of the kind
could happen before the build was put into reports.

- A report that carried the build and did not get through, whether cut
  off or turned away, is followed at once by one without the build.
  The build is tried again ten minutes later. At worst one report in
  ten minutes is lost to it.
- A build file that cannot be read is left out the same way, where it
  used to stop the report.
- Whether the build and the changed sections fit one report is now
  measured on the report as sent, in bytes. It was estimated in
  characters, and text outside ASCII is up to three bytes a character,
  so the estimate could pass a report the edge then refused as too
  large.
- When the build goes alone, the sections it displaced follow at once
  instead of at the next ordinary report.

- The edge wrote a new build over the pieces of the old one and only
  then recorded the new build. A write that stopped half-way left
  pieces of two builds under the old record, and with the laptop away
  those were handed out as the old build. The old record is now removed
  before its pieces are touched, so a half-written copy is no copy: the
  edge says it has none, hands nothing out, and asks for the build
  again. Pieces left by a larger build are removed whether or not its
  record survived.

## 2026-11-23T06:10Z Update: released by itself, and what checking from outside found

Asked for: the app built and released by GitHub with nobody involved,
the Worker stood up from nothing and deployed by GitHub, every page
checked, notifications shown to work, and an audit end to end.

### The Worker is stood up and released by GitHub (#12765, #12775)

- `edge/release.mjs` decides from Cloudflare's own answers what a
  release needs: a Worker that is not there is first stood up empty
  (`edge/init/`, which answers 503 so nothing is routed to it), then the
  commit is uploaded as a version and promoted; one that is there is
  uploaded and promoted; a change to how the object is declared is a
  full deploy. A release whose health check does not come back as that
  commit puts the previous version back.
- On request, and whenever the release itself changes, the same path is
  first run against a throwaway Worker, from nothing to serving and
  gone again.
- The deploy runs the bundle in the Worker runtime (`wrangler dev`)
  against a host on the runner before anything is uploaded. That exists
  because of the first fault below, and it found the second.

- One stand-up from nothing failed tonight and is not explained. At
  `dec4e883b8b6` (run 45121364544) the scratch Worker answered as
  empty, took the version at 100%, and then gave no health answer for
  four minutes; the run put the empty version back, removed the scratch
  Worker and failed, with production untouched, as designed. The same
  steps one commit later (run 45122274288, `40e0c2ac4418`) answered
  within a second, and production took that commit. The run could only
  say "silent"; since #12976 a wait that fails says what it heard (the
  status and the start of the body, or the error), so the next one will
  be explained.

### Two faults no test under bun could see

Both are the same lesson: the tests run the Worker's code under bun,
and bun is not the runtime it is deployed to.

1. **A request's upload** (#12775). `request.clone()` with the first
   copy never read stalls the second in the Worker runtime. The first
   report from a host hung for twenty seconds. Fixed by reading the
   copy that is not used.
2. **`fetch` as a method** (#12954). The runtime's `fetch` refuses to be
   called as a method of another object ("Illegal invocation"). The
   edge kept the global `fetch` on itself and called it as
   `this.fetchImpl(url)`. From its first deploy, every notification it
   tried to send failed before leaving, it never read the Android
   release, and the queue half of direct mode would have failed the
   moment it was switched on. The failures were caught and counted and
   shown nowhere. It was found because health had just been made to say
   which release the edge knows of, and on the deployed Worker it said
   none.

What is in place so that neither kind returns quietly:

- No class keeps a `fetch` it was given; each keeps a plain caller
  (`src/plain.ts`).
- The stand-in `fetch` in the tests refuses to be called as a method, as
  the runtime's does. Against the code as it was, 21 tests fail on it.
- The run in the real runtime checks that the state can make a request
  of its own, and that files are tagged and revalidated.
- Health says, to anyone, which release the edge knows of and, when it
  could not ask, why (`androidRelease`, `androidReleaseTrouble`).

Decision: a thing the runtime can refuse and bun cannot gets both a
stand-in that refuses in the tests and a check in the real-runtime run.
A feature whose failure is caught and counted must also be shown: the
Notifications screen now says what came of the last send.

### The app is built by GitHub and signed where its key is (#12770, #12940)

- GitHub builds on every change to the app; the version is the
  commit's, so each build installs over the one before.
- GitHub does not hold the signing key. Moving it there was considered
  and not done: nothing here holds or moves the key. Instead the host
  that holds it runs `android/release-built.sh` every ten minutes
  (`pm-console-android-release.timer`): it takes the newest unsigned
  build this repository made on `main`, signs it there, refuses a
  result that does not carry the certificate the site lists, and
  releases it. What leaves the host is the signed APK, which is public.
- The first release made that way, `2026.11.23-0f311ae`, was published
  five and a half minutes after its merge. Checked from outside with
  `apksigner`: signed by the listed certificate (v2 and v3), and its 55
  files are byte for byte the ones GitHub built.
- If the owner one day puts the key into GitHub's environment, the
  workflow signs and releases by itself and the host finds nothing to
  do. That is his choice; nothing waits on it.

### Notifications, checked against the real push services (#12934)

- The console's own sender was run against Mozilla's service (received
  over the network and opened with a decryption that shares nothing
  with this code) and Google's (a real Chrome subscribed; Google took
  the message).
- Found: a Chrome off the stable channel names `jmt17.google.com`, which
  the sender refused; and the Notifications screen said "On" when the
  sender did not hold the device. Both fixed. The service worker has
  tests that run the file itself.
- The larger fault, that nothing left the deployed Worker at all, is
  the second one above.

### The audit (#12946), and one screen more (#12967)

- Every screen through the axe rules in both themes: 24 failures in the
  light theme (status colours made for the dark page, links told by
  colour alone, a skipped heading level), now none.
- The app's files are tagged and revalidated, where every visit used to
  download them again.
- More, Releases: what is deployed and released, and putting either out
  again, which until now was done by asking in a chat.

### Not checked

- The signed-in pages of the live site: no browser with the owner's
  sign-in was available, and none was made. Checked instead: every
  screen on the test copies, the bundle in the real runtime, and the
  live site's public surface.
- A notification arriving on a phone. Seen: Google and Mozilla taking
  the message, Mozilla delivering it, the service worker showing it in
  a real Chrome, and the deployed Worker now able to make requests.
- A deploy started from the app on the laptop (it goes through
  `fleet gh` there).
