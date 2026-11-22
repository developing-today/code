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
