# The PM platform: Cloudflare is the system, machines connect to it

Status at writing: design and plan, written before the work. Later
sections record what was built and what was found.

## What was asked

The owner, 2026-11-23 00:39 CST, of the console as it stood:

> this was a good start but core functions require the laptop. rework
> the structure to rely on cloudflare for all core functionality. the
> laptop should just be connected to the system as one of many which
> happens to have extra capabilities

and, in the same message: full Cloudflare and Turso management; every
aspect managed by the app and Cloudflare; the app working offline for
all capabilities, holding results until connected again; managed
roadmap pages for each major capability (id, fleet, atlas, pm-stack)
showing what is built, in progress, and planned for the immediate,
short and long term, with actions including notes that become
prioritised issues; a bug tracker page; a system that routes new
notes, bugs and changes to existing items; every change in the app
able to start an issue or a session that takes the note verbatim;
issues for every gap not built; and the existing Workers merged into
one platform.

## Where things stood

`pm-console-edge` (doc `2026-11-22T11-10-00Z_architecture_pm_console_edge`)
serves the app from Cloudflare, but it is a front for the laptop:

| Function                                              | Done by, before this                                                                                                              |
| ----------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| The app's files, sign-in check, notifications, phones | Cloudflare                                                                                                                        |
| What is in progress, next, ranked, open pull requests | the laptop's last report; Cloudflare itself only after a minute of the laptop's silence, and only if a switch was on (it was off) |
| Asking, replying, ranking, commenting, moving a task  | the laptop; held for it when away                                                                                                 |
| Merging, checking a pull request, deploys             | the laptop only                                                                                                                   |
| Questions, needs, services, agents, dashboards        | the laptop only                                                                                                                   |
| The queue's tasks                                     | written only by the `fleet` binary on a machine                                                                                   |
| Sign-in and address configuration                     | a script on the laptop (`expose.py`)                                                                                              |
| Opening the app with no connection                    | not possible: nothing was kept on the device                                                                                      |

The machinery for Cloudflare to act by itself existed (`edge/src/direct.ts`
runs the laptop's own classes over GitHub's REST API and the queue
store's HTTP API), but was a fallback behind two gates: a switch, off
by default, and a minute of the laptop's silence.

## Decision

1. **Cloudflare is the system of record for managing.** With its two
   tokens the Worker reads and writes GitHub and the queue store itself,
   always. It does not wait for a machine to go quiet, and it does not
   send to a machine what it can do itself. Managing means: seeing what
   is in progress and next, ranking, asking, noting, reporting, merging,
   deploying, releasing, being notified.
2. **A machine is anything that connects and reports**: the laptop
   today, others later. A machine brings capabilities the platform
   cannot have: running agents' sessions, its own services, its
   dashboards, a signing key, the `fleet` binary. What needs a
   capability is held until a machine with it is connected, and says so.
   Nothing that needs no capability waits for a machine.
3. **The tokens get to the Worker without anyone handling them.** The
   machine that holds the secret store hands them to Cloudflare's API
   itself (`expose.py`, which already did this behind the switch). That
   is now what the declaration says by default; the switch in the app
   remains, to turn it off. The owner asked for Cloudflare to carry the
   core, and it cannot without them.
4. **The device keeps a copy.** The app's files and the last state it
   showed are kept on the device, so it opens and reads with no
   connection. What is done while offline is kept in an outbox on the
   device, under a name of its own, and sent when the connection is
   back; the platform does each named action once, however many times
   it arrives.
5. **One way in for everything the owner says.** A note, a bug, and a
   change to a roadmap item are one thing, an intake item: the words
   verbatim, what they are about, how urgent, and whether an issue or a
   session should take them. Routing is a table, not code scattered
   over screens: the kind and the capability decide the labels and the
   queue task. An item about an existing issue becomes a comment on it
   (and a new rank, if asked); anything else becomes an issue, or a
   prompt that starts a session. The source of truth is GitHub: the
   Intake screen is read back from the issues themselves, so it is the
   same wherever the action ran.
6. **Roadmaps are read from where they already live**: the roadmap
   files in `planning/roadmaps/` for the curated rows, and the issues
   under each capability's labels for the work itself. Moving an issue
   between phases is a label change, done at once. Changing a curated
   row is an intake item that a session carries out in the file.
7. **The other Workers fold in by doors, not by a rewrite.** See "One
   platform" below.

## What a capability's roadmap page shows

| Group                | From                                                                                                              |
| -------------------- | ----------------------------------------------------------------------------------------------------------------- |
| In progress          | file rows with status in-progress; issues being worked (`work/in-progress`, `queue/claimed`, `queue/in-progress`) |
| Planned: immediately | file rows under Now; issues ranked `expedite`, `priority/critical`, `now`                                         |
| Planned: short term  | file rows under Next; issues ranked `priority/high`, `next`                                                       |
| Planned: long term   | file rows under Later; the capability's open epics                                                                |
| Built                | file rows marked done; issues closed in the last two weeks                                                        |

Capabilities and their labels: id (`app/id`), fleet (`app/fleet`),
atlas (`app/atlas`), pm-stack (`app/pm-stack`), then every other file
in `planning/roadmaps/`. fleet and atlas have no roadmap file yet; their
pages are built from their issues until one is written (an intake
item can ask for it).

## Routing

| Kind              | Becomes                                                                             | Labels                                                       | Default rank |
| ----------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------ | ------------ |
| bug               | an issue, with a triage task                                                        | `kind/bug`, `source/human`, `owner/intake`, the capability's | now          |
| note              | a comment on the issue it is about, with a task; else an issue with a triage task   | `source/human`, `owner/intake`, the capability's             | next         |
| change            | an issue saying what to change, with a triage task                                  | `source/human`, `owner/intake`, the capability's             | next         |
| any, as a session | a prompt (`owner/prompt`, `expedite`) that starts a session with the words verbatim | as above, added to the prompt's own                          | expedite     |

`owner/intake` is new, and is how the Intake screen finds what the app
filed. A note and a change carry no `kind/` label: whoever takes the
item decides what kind of work it turned out to be. (`roadmap/notes`
looks like the label for this and is not: it is the roadmap programme
named "notes".)

## One platform

Sixteen Workers are configured in this repository. Three are separate
products and stay so (`id-hosted-worker`, `pkarr`, the proofs of
concept). The rest are parts of one control plane built at different
times:

| Worker                                                                                                                | What it is                                                                                                                 | Where it goes                                                                                       |
| --------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `pm-console-edge`                                                                                                     | the app, its state, notifications, sign-in check                                                                           | the platform's front and state                                                                      |
| `fleet-hosted`                                                                                                        | the fleet's hosted doors: queue, stamping, query, ingestion, sync, sessions (built, not deployed; no real queue store yet) | the platform's work API; the console calls its queue door instead of holding enqueues for a machine |
| `message-bridge`                                                                                                      | messages between the owner, assistants and agents                                                                          | a messages door; its page becomes a screen                                                          |
| `decision-events`                                                                                                     | decision events in D1                                                                                                      | a decisions door; Questions stop needing the laptop                                                 |
| `queue-transport-alert`                                                                                               | alerts on the queue's transport                                                                                            | a check inside the platform, notifying through it                                                   |
| `github-ci`                                                                                                           | GitHub webhooks for CI                                                                                                     | the ingestion door                                                                                  |
| `lab-ops-hub`, `voice-line`, `muse-agent-handoff`, `atlas-search`, `cold-store-rehydrator`, `developing-today-backup` | their own jobs                                                                                                             | stay deployed as they are; listed, health-checked, deployed and rolled back from the platform       |

The order is: (1) one list, one health, one deploy and rollback for all
of them, from the app; (2) the console uses `fleet-hosted`'s queue door
once it has a real store and is deployed; (3) messages, decisions and
alerts move in as doors, each retiring its Worker. Each step is an
issue. None of them is done by this change.

## Plan

1. This record.
2. Intake, roadmaps and bugs in the shared classes (`src/ops.ts`), so
   they run on a machine today and at the edge unchanged; their screens.
3. The edge acts by itself always when it has its tokens: no silence
   gate; merges, deploys, pull-request checks, the queue list, roadmaps
   and intake done there; a machine's copies of what the edge reads
   itself are not taken. `expose.py` hands the tokens over by default.
4. Offline: the service worker keeps the app and its last state; an
   outbox on the device; each action named by the device and done once.
5. Machines: a report says which machine it is and what it can do; a
   Machines screen.
6. Issues for what is not built.

## What this does not do

- It does not run agents on Cloudflare. Sessions run on machines.
- It does not write the queue's tasks from the Worker. Enqueueing is
  the `fleet` binary's, with its dedupe and identity rules; until
  `fleet-hosted`'s queue door is deployed with a real store, the Worker
  files the issue at once and a connected machine files the task.
- It does not move Questions, needs, services or agents off machines.
  Those are machine capabilities or need the decisions door.

## 2026-11-23T08:20Z Update: the plan's screens

Step 2 is built (`src/plan.ts`, with its screens under More).

- **Roadmap.** One page per capability, chosen along the top: the four
  named first, then every other file in `planning/roadmaps/`. Three
  reads per page, not one per phase: the file, one search for the open
  issues that are being worked on, ranked or epics (sorted into phases
  here by their labels, so an issue is shown once, where it stands
  furthest along), and one for what closed in the last two weeks. A
  page is read at most once a minute however often it is looked at,
  because GitHub's search allows thirty requests a minute and the
  fleet's own searches count against the same thirty; the refresh
  button asks again at once, and so does anything sent from the app.
  Measured on the day: 45, 39, 6 and 31 ranked-or-working issues under
  id, fleet, atlas and pm-stack, so one page of a hundred holds each.
- **What can be done from a page.** On an issue: move it between the
  planned phases (its rank, set at once) or leave a note on it. On a
  row of the file: a note, or a change (move it, add a sub-item, mark it
  built, rewrite it, drop it). On the capability: a note, or a new
  item. Each takes the words as typed and may be sent as a session.
- **Bugs.** A form (what went wrong, where, how soon), the open bugs
  soonest first, and the ones fixed lately.
- **Intake.** A plain note, the routing table as the service has it,
  and everything sent from the app as GitHub has it now.
- **The queue half.** Filing a task is still the fleet binary's. Each
  intake hands its packet to the same stand-in point the owner's
  prompts use, with the action a machine would be asked to do in its
  place (`triage.enqueue`, or `prompt.enqueue` for a session): the
  arguments are checked again on the machine and the packet is built
  there, never taken from the edge. A note on an existing issue has a
  task of its own wording (`notePacket`), under the issue's one triage
  key, so it joins a live task or starts a new one.
- **Checked.** 31 tests for this (GitHub and the queue as recorders;
  packets against the queue's own schema file); 74 planted faults, 71
  caught, two that change nothing, and one that showed a step to be
  redundant, which was removed; 27 checks in a phone-sized Chrome
  against the demo, direct and behind the edge; axe-core over all 19
  screens and the three form dialogs, light and dark, no violations.

Not in this step: at the edge these reads and `intake.send` still go to
the laptop (held when it is away). Step 3 moves them.

## 2026-11-23T11:10Z Update: the edge does the core itself

Step 3 is built. What changed from the plan above is in how, not what.

- **One gate.** `primary`: the switch on, both tokens there, and a
  machine's report having said where things are. The minute of silence
  that used to be waited for is gone, and with it the two-source
  problem it was avoiding, which is settled the other way round: a
  section the edge reads is the edge's alone. The laptop's copy of it
  is not taken, nor its word on how current it is; the edge names those
  sections in its answer to a report (`owns`) and the laptop stops
  sending them. Turn the switch off and the laptop's next report brings
  them back.
- **The edge reads the way the laptop does.** There is no separate
  path any more for a read made at the edge: each such section's loader
  reads its source, and the document keeper (`src/docs.ts`) does the
  rest as it does on the laptop: its interval, one read at a time, the
  last good copy and the reason when a read fails. A document is
  answered at once from what there is and a stale section is read
  behind the answer. Measured in the Worker runtime before relying on
  it: a read started behind an answer finishes, with or without being
  handed to `waitUntil`, and a wake can read and write.
- **Across restarts.** The object is put away when idle, so everything
  it would otherwise learn again is kept: each good read (content
  written only when it changes), and through the kept list of prompts
  what each thread held, so a restart reads the list and no thread that
  has not changed. Without that, a wake a minute would have meant every
  thread every minute.
- **Notifications with no machine.** While a device is subscribed the
  object wakes itself each minute, reads the prompts and tells the
  owner of a reply. The same wake notices a laptop gone quiet, and a
  telling that failed is tried again a minute on (it used to wait for
  the next report, which a quiet laptop never sends).
- **More done here.** Merge, start a deploy, and intake, beside what
  was already there. The reads that need only GitHub and the queue
  store are one module (`src/reads.ts`) used by the laptop's server and
  the edge alike: one pull request's page, the queue by state, ranked
  issues, releases, roadmaps, bugs, intake.
- **A refusal is handed on.** The edge's token is a fine-grained one
  for automation, and what it may do was not known when this was
  written; the laptop's own way in is another credential. So that
  moving an action to the edge can never take it away from the owner:
  when the edge's token is refused before anything was written, the
  action goes to the laptop exactly as it did before, the edge leaves
  that kind to the laptop for an hour, and health lists it under
  `refused`. That list is how a missing permission will be seen.
- **On by default.** `expose.py` gives the Worker its tokens unless told
  off (`--edge-direct off`, or the switch under More, Cloudflare, both
  remembered). This is the decision recorded above under "Decision",
  point 3: the owner asked for Cloudflare to carry the core, and that
  cannot happen without them. No value passes through anything but the
  laptop's own script and Cloudflare's API. A run that was never told
  and cannot do it says so and does not fail, so the rest of what that
  script keeps is not held up.
- **Checked.** 281 tests: 257 under bun and the keeper's 24. 82 planted
  faults in the edge and its shared pieces: 78 caught, four that change
  nothing that can be reached. 10 in the keeper's default: all caught. The
  bundle in the real Worker runtime with the switch on, against
  stand-ins for GitHub and the queue store on the runner: its own read
  of each kind came back good, each carrying its token; this now runs
  on every deploy. A phone-sized Chrome over the app's changed screens,
  and axe-core over all 19 and the Cloudflare screen in each of its
  states, light and dark: no violations.

Not done in this step, and still a machine's: filing a queue task (held
for one, as before); questions, needs and sessions; units, agents and
messages; the sign-in settings. The edge still learns where things are
from a machine's first report. Each has an issue (step 6).

## 2026-11-23T11:12Z Update: step 3 after review

Review of the change above (Codex, and a second reading of it here)
found five ways it could show the owner less than the laptop did. All
are closed in the same change, before it was merged.

- **`off` was forgotten on a laptop that never had it on.** With on as
  the default, `--edge-direct off` before any run had turned it on
  took nothing away and remembered nothing, so the timer's next run
  turned it on. It is remembered now, before the tidying, so a tidying
  that is refused leaves the mode off all the same.
- **After a restart, an action's change could be shown as it was
  before.** The first look of an object that has just started shows
  the copy kept from before; the read after an action took that
  shortcut. What an action touched is now read from its source, always.
- **What a machine did was not read again here.** A section the edge
  reads is not taken from a machine, so an action the laptop did (one
  handed on after a refusal, one only a machine does, one held and done
  late) showed only when the section's interval came round: up to three
  minutes for ranked issues. The laptop already says which sections an
  action changed; those the edge reads are now read again at once.
- **A refused read had nowhere to go.** Actions were handed on when the
  edge's token was refused; sections were not, and a screen that
  answers in part (the deploys, a roadmap) was answered here with its
  error. Now a section the edge is refused for is the laptop's again for
  the hour (out of `owns`, the laptop's copy shown, health saying
  `read.<section>`), and a refused or part-refused screen is asked of
  the laptop while one is reporting. This is the same rule as for
  actions, and for the same reason: what the edge's token may do was
  not known when this was built, and giving the edge its tokens must
  not be able to take anything away.
- **A read under way was trusted after a change.** The document keeper
  shared a read already in flight, which may have been taken before
  the action finished, and then counted the section fresh. After a
  change it now waits for that read and reads once more
  (`Docs.reread`); on the laptop too.

Three more came out of building those, each a way the hand-over itself
could have gone wrong:

- **Calling too often looked like a refusal.** GitHub answers both
  with a 403. The token is the fleet's too, and search (which the
  ranked issues are read with) is what runs out first; an hour with a
  machine for that is too long when no machine is there. It is told
  apart by GitHub's own words, and handed on for five minutes.
- **Reading the queue counted as having written.** Every call of the
  queue store was noted as a write, so an action refused after it had
  only read the queue was said as a failure when a machine could have
  done it, and a section's refusal could be hidden by another
  section's good read at the same moment. A request of the queue store
  is now a write only if a statement in it is not a SELECT, and a
  section is judged by the answers of the services it is read from.
- **Turned away with nobody looking, the wakes would have ended.** The
  minute's wake runs only while the edge reads the prompts. When it is
  turned away for them the object now asks to be woken when that time
  is up, if a device is there to tell.
- **The hand-over was forgotten whenever the object was put away.** It
  was kept in memory only. An object that starts again would have
  claimed every section afresh, told the laptop so, and been refused
  again on the next look: with the app closed, the laptop's copy of the
  prompts would never have been taken, and no reply would have been
  told of. What the edge is turned away for is now kept in its storage
  until its time is up, and the owner can have it tried again at once
  (More, Cloudflare, "Try again now") when the token has been put right.

One thing found on the way that is not about the edge: text on the
status colours. The success toast on the light page (3.3 to 1) and the
failure toast and the danger button on the dark one (3.35 to 1) were
below what can be read comfortably; none of them is on screen when the
screens are scanned, which is how they were missed. Each status colour
now has an ink of its own per theme, and a test measures every such
pair with the WCAG formula on each run.

Checked again: 302 tests (277 under bun, the keeper's 25). 65 more
planted faults in all of this: 62 caught, three that change nothing
that can be seen (a section a machine reads being read once more from
the copy here; a wake that would find nothing to do; a kept entry past
its time, which is dropped on first use anyway). The bundle in the
Worker runtime with the switch on: all checks passed. A phone-sized
Chrome over the Cloudflare screen in each state with the new control,
and axe-core over the toasts and the danger dialog while they show,
light and dark: no violations (three before the inks).

## 2026-11-23T12:52Z Update: in service, and step 4 (with no connection)

**Step 3 is in service.** Merged as #13205. At 12:00Z the public health
answer said `primary: true`, both tokens present, nothing refused,
and its own read of each kind (the board, pull requests, ranked issues,
the prompts) good with the real tokens. Not checked: a write with the
real token, which needs the owner's sign-in; a refusal would show under
`refused` and be the laptop's to do.

The deploy of that merge failed twice at its rehearsal on a scratch
Worker: Cloudflare answered "This Worker does not exist" (10007) about
a Worker wrangler had uploaded a second before. The real Worker was
deployed by a manual run without the rehearsal; standing a Worker up is
now made up to four times (#13235), and the next deploy passed on the
third.

**Step 4 is built: the app with no connection.** Decision 4 above, as
it turned out:

- **The device keeps a copy of everything it read.** The service
  worker asks the network first and keeps each good answer under the
  address it was asked for: the app's files, the three documents, and
  every screen's own read. A copy is shown only when the network gives
  no answer or is too slow to wait for, and is marked as one; an answer
  that says no is never replaced by a copy. So nothing about a screen
  had to change for it to work offline, and a screen added later works
  offline from its first visit.
- **Every action has a name, given on the device.** It is written to
  the browser's database before it is sent, and sent at once when there
  is a connection, as before. With none it waits, and goes when there
  is one: from the page, or from the service worker if the app is
  closed (Background Sync). One file (`public/outbox.js`) holds the
  rules for both.
- **Once, however often it arrives.** The edge writes down each name it
  did with its answer; a machine does the same (`src/named.ts`, one
  record for actions sent straight to it, relayed to it, and held for
  it). A name offered again is answered from the record. This is the
  rule that already protected a relayed action whose answer was lost,
  extended back to where the action starts.
- **What may have changed is not sent unasked.** The list the platform
  already had for a machine that is away (what may be done late) is the
  device's list too. A merge, a deploy, a cancel, an unblock and a
  service action are kept, sent by themselves only within ten minutes,
  and after that wait for the owner to look again. A merge now names
  the commit the owner was shown, and GitHub merges only that: this
  guards a merge asked for a second ago as much as one kept for hours.
- **Signed out is told from offline** by asking past the service
  worker without following the way to the sign-in page.

What the owner sees: `offline` and how old the state is in the corner;
a line under the header saying there is no connection and what is kept;
More, On this device, where what is waiting, kept back, failed, with
the platform or sent can be looked at, dropped, sent or tried again,
and where a copy of every screen can be fetched before a flight.

Checked: 321 tests under bun and the keeper's 25. Planted faults: 23 in
the once-only rule on the edge and the laptop, 57 on the device (the
outbox, the worker, the app's wording), all caught. In a real Chrome,
against the demo laptop and the demo edge, light and dark: the app
taken offline opens, shows each screen it had seen, says so, keeps
three actions across a reload, sends each once when the connection is
back, answers a repeat of each name without doing it again, keeps a
three-hour-old merge back until it is confirmed, and forgets when
asked; and with the app closed, the worker, woken as the browser wakes
it, sends what was kept. axe-core over all 20 screens, the dialogs, the
toasts and the new screen in each state: no violations.

Not checked: a real phone with no signal, and the Android app (it is
this site in Chrome, so nothing in it changed). Not done: copies are
not encrypted on the device beyond what the device does itself; a
screen never visited has no copy until it is visited or "keep a copy of
every screen" is pressed.

## 2026-11-23T15:40Z Update: step 4 after review

Review of the offline work found six real faults, and looking at them
found three more of the same kinds. All are fixed in the same change.

- **A name could be done twice by two arrivals at the same moment.**
  The laptop looked a name up, then waited for the audit's write, and
  only then marked it as being done: two arrivals could both look and
  both do it. A name is now taken first, with no waiting in between
  (`Named.take`), and what is known of it is read after, on the laptop
  and at the edge alike; the second arrival is told it is being done,
  and asks again. The laptop's record of names is read once however
  many ask together, and written one write at a time.
- **A held action being done on the laptop looked "lost" to a retry.**
  Doing a held action wrote "started" without marking the name as being
  done, so a device sending it again meanwhile was told the laptop had
  restarted, and offered a retry under a new name. Held actions now go
  through the same taking and giving back as any other.
- **Something done now could overtake something older.** A new action
  was sent by itself, ahead of what was waiting on the device; the
  older one, sent later, undid it (a rank changed twice ended as the
  first). A new action is now written down and sent through the same
  oldest-first pass as everything else (`sendInOrder`).
- **Order is kept per thing, to the end.** Found while fixing the last:
  an older change that nothing could take just now (503) or that was
  still being done (409) let a newer change about the same thing go
  first. Each action says what it is about (`subject` on the device,
  `subjectOf` on the platform; a test holds them together); a later one
  about the same thing waits behind an earlier one that has not been
  taken, on the device and in what the edge holds for a machine. What
  is about something else is not held up.
- **"Do not send" during a send was ignored**, and so was an entry
  another tab or the worker had dealt with meanwhile: each entry is now
  read again just before it is sent.
- **The worker gave up with things still waiting.** A 503 or a
  "being done" left an entry waiting but ended the browser's wake as a
  success, so nothing called again. The worker now asks to be woken
  again while anything waits to be sent, and on the browser's last call
  says what still waits.
- **A full device broke reads.** A refused write to the cache failed
  the read it belonged to. Keeping copies is now on a best effort; and
  with no cache at all (a browser that gives the site no storage) the
  worker installs and answers as if it kept nothing.

A second review of those fixes found two more, both real:

- **A name could be done once on each side.** Which side does a kind
  can change (the edge is turned away for an hour, then takes it
  back). An action given to the laptop, its answer lost, and sent again
  after the change was done again by the edge, which had no record of
  the name; and the other way about. The edge now writes down a name
  it hands to a machine as given, and looks a name up before choosing
  a side: a name keeps to the side that first took it.
- **A refused answer was counted as sent.** An answer to a question
  settled in the meantime comes back as a success that says "not
  recorded". Sent from what the device kept, it was marked sent. It is
  now "could not be done", with why, as the laptop already says of one
  it does late.

A third review found two more, both real, both in the page:

- **An action waited its turn in memory.** It was written down only
  once it held the turn to send; with another tab or the worker in the
  middle of a slow send, closing the page lost it. It is now written
  down first, and only the sending waits. One try at sending is given a
  minute, so a request that hangs cannot hold every sender up.
- **A taken-back answer left its question hidden.** The page kept its
  own list of questions answered here. The list is now worked out from
  what is kept: a question is out of the list while its answer waits
  (across a reload too) and back when the answer is taken back or does
  not count.

A fourth review found three more, all real:

- **A draft was erased by a send in the background.** When something
  kept earlier was sent, the screen was drawn again whatever the owner
  was doing, and a bug report, a note or a message half typed was
  gone. A background send now draws the screen again only when nobody
  is in the middle of something, and those three forms keep what is
  typed across any redraw (as the prompt box already did).
- **A cut-off answer was read as "sign in again".** An answer whose
  body stops arriving is no answer, and is offered again; only a body
  that arrives and is not JSON is taken for something other than the
  platform.
- **A failed answer could leave its question hidden**, if the list was
  read while the answer was on its way. On failure the list is read
  again whole.

Checked: 342 tests under bun, the keeper's 25. 52 planted faults in
the new code, all caught (two more were planted and found to change
nothing: each is behind a second guard). In a real Chrome against both
demos: the offline flow, light
and dark; the worker woken with the app closed; an older waiting rank
and a new one leave the device in order and are done in order (and with
the old direct send planted, that check fails); an answer kept, taken
back and kept again, and an action done while another sender holds the
turn and the page then closed; a draft kept through a background send
with the cursor in it and with it set aside, and a question back after
its answer failed (each of these fails against the code before its
fix); axe-core over 20
screens, the dialogs and the toasts: no violations. The Worker in its
real runtime passes its own checks.

Found on the way: the console's TypeScript is never type-checked. A
method added to the laptop's link had the name of one already there,
and the second silently replaced the first; the tests passed for the
wrong reason until a later change exposed it. A type check finds it at
once, and 53 smaller complaints across the console, none of them a
fault. Adding the check to CI is the next change.

Not done, and filed as #13398: an older action held for a machine is
not ordered against a newer one about the same thing that the edge
does itself.

## 2026-11-23T16:50Z Update: step 4 in service, and step 5 (machines)

**Step 4 is in service.** Merged as #13287 after four rounds of review
(thirteen findings, all real, each with a test and most with a check
in a real browser that fails against the code before its fix). At
16:38Z the Worker and the laptop both ran `2a96e875037d`; the Worker
said it does the core itself and had been turned away for nothing.

Seen in that answer: the Worker's own read of pull requests had failed
a quarter of an hour earlier with GitHub's "rate limit exceeded" for
the owner's account. The Worker's token is the fleet's, and the fleet
had used the hour's allowance. The Worker treats that as "one too many
just now" and asks again in five minutes, which is right; but a
platform whose core reads stop when agents are busy is not doing the
core by itself. Filed with the gaps: its own allowance.

**Step 5 is built: the laptop is a machine.** Decision 5 above, as it
turned out:

- **A machine says what it is.** Each report carries the machine's name
  and what it can do: the kinds of action it has and the sections it
  reads. The edge keeps what it last said beside when it last reported.
  A console older than this says nothing, and is shown as a machine
  that has not said what it can do.
- **The edge answers for the whole.** `GET /console/api/machines` is
  answered at the edge with no machine asked: what the edge does itself
  right now, each machine and whether it is reporting, and what waits
  for one. A machine serving the app by itself answers the same read
  with itself.
- **What waits can be taken back.** `held.drop` removes one held action
  before a machine has it, and records it as not sent under its name,
  so the device that sent it learns so.
- **The screen** (More, Machines) says each of those in words, and that
  one machine can be connected today.

What this does not do yet, and is filed: a second machine under its own
key; each action sent to a machine that can do it (today there is one,
so "a machine" is "the machine"); what is held kept per machine.

Review found two faults, both real. Taken back, an action could be
kept again when a device that never heard it was kept offered it again:
taking back now ends the name, and the offer is answered "taken back".
And the screen did not notice a machine going quiet unless something
else changed: it now asks again while it is open, and is drawn again
when what it shows has changed.

Checked: 351 tests under bun. 28 planted faults in the new code, all
caught. In a real Chrome against both demos, light and dark: the
screen with nothing in front of the machine and with the edge in front;
something held while the machine is out of reach, listed in the owner's
words, taken back after a question, and recorded as not sent. axe-core
over 21 screens and the new dialog: no violations.

## 2026-11-23T17:20Z Update: step 5 in service, and step 6 begun (one platform)

**Step 5 is in service.** Merged as #13471 after one round of review
(two findings, both real). At 17:00Z the Worker ran `cf40e59c6e92`.

**The Workers, counted.** The repository holds eighteen. Asked at their
addresses from outside today, fourteen answered and four did not exist
(the fleet's hosted front door and three of id's, all local-only by
their own account). Of the fourteen, two are deployed from Actions (the
PM console, the pkarr door). Twelve were deployed by hand from a
machine: nothing in the repository says which commit each runs, and
none can be put back from the app.

**What is built: the list, and the screen.** "One platform" starts as
one list that is true. `pm-stack/platform/workers.json` names every
Worker, where it lives, what it does, whether it is in service, how it
gets there, and what joining the platform means for it; a test beside
it holds it to the repository, so it cannot quietly go stale. The edge
reads it from the repository with the latest run of each deploy
workflow, and More, Platform shows it with no machine. What joining
means is written down in `pm-stack/platform/README.md`: deployed from
Actions with a check and a way back, saying its version, its secrets
set by the deploy, seen from the app.

**What is not built.** No Worker was moved to Actions in this change:
each has its own secrets and bindings, and deploying one I have not
stood up from nothing would be a guess with someone's service. Each is
its own issue, filed with the gaps, in the order the list gives.
Whether each is in service is not checked by the edge itself: a Worker
cannot ask another on the same account by its public address without a
setting the console does not have, and changing how the console reaches
out needs its own care (the same setting changes how it reaches the
laptop).

**Seen in service, and fixed here.** The Worker's own check of its
reads had failed once on GitHub's rate limit and stood as failed for
the hour, on the Cloudflare screen too. A check in which something
could not be read is now made again in five minutes.

**Every gap is an issue.** Sixteen, under the tracker #13504: the
edge's own GitHub allowance (#13489), queue tasks from Cloudflare
(#13490), questions and sessions with no machine (#13491), a message
to an agent with no machine (#13502), the ordering case left out
(#13398), a second machine (#13492), the twelve Workers deployed from
Actions (#13493), the edge checking each Worker (#13494), platform
settings from Actions (#13495), the stores managed from the app
(#13496), the fold-in of two Workers (#13503), roadmap files for fleet
and atlas (#13497), a type check (#13498), the signed-in flows in the
real runtime (#13499), the Android app on a device (#13500), and what
is left of offline (#13501). The PM stack's roadmap file has the rows.

Checked: 357 tests under bun, 4 beside the list. In a real Chrome
against both demos, light and dark: the Platform screen's counts,
groups, each kind of row and its links. axe-core over 22 screens: no
violations.

## 2026-11-23T18:10Z Update: after step 6 (review answered, and the shared allowance)

**A review finding that arrived as #13487 merged.** The Worker-list
parser had example tests only, and the testing doctrine
(`doc/agents/testing-doctrine.md`) asks for property tests where code
has invariants. It was right, and so it was for the two other parsers
of this work. #13517 adds properties on the seeded generator the
console's tests already use: rows and accounts built to keep every
rule are read back as written, single faults refuse them, the list is
held to a model written out in the test, and the device's and the
platform's account of what an action is about are compared on
generated actions. One fault was found by them: a machine's name cut
at 63 characters just after a blank kept the blank. Fixed there.

**The shared allowance (#13489), the half that needs no new
identity.** The edge reads GitHub with the fleet's token, so it draws
on the account's hourly allowance with the agents, who used the hour
up twice on 2026-11-23.

- _Asked by tag._ Each answer to a read is kept under its address
  with GitHub's tag and asked for again by it. "Not modified" does not
  count against the allowance, so a list that has not changed costs
  nothing. GitHub is still asked every time and decides whether the
  kept answer stands: this is not a cache with an age, and nothing
  shown is older than it was before.
- _GitHub's time is kept to._ Turned away for calling too often,
  GitHub says when to ask again. Until then the edge asks it nothing
  that draws on what ran out, where before it asked again every five
  minutes for as long as the hour had left. Sixty-five minutes at
  most, and the owner's "Try again now" lifts it.
- _Two allowances, kept apart._ GitHub counts searches by the minute
  and everything else by the hour. Ranked issues are a search, so they
  are still read while the hour's allowance is spent, and a minute's
  searches running out (the commoner case) no longer has anything to
  do with the lists. Going too fast (`retry-after`) is said of the
  caller and hushes both.
- _Said in the app._ Why a read failed now carries that time and
  GitHub's own words, and the notice says which it was ("the allowance
  for the hour is used up" or "turned away for asking too often just
  now"), "GitHub says it answers again at 12:00", and how old the copy
  shown is.

Considered and not done: serving the kept answers for a time without
asking (a cache with an age). It would save the calls that change
too, at the price of showing things late, and the edge is not what
uses the hour up. And persisting the answers in the object's storage:
a restart costs one full answer of each, which is less than the
writes would.

What this does not do: give the edge an allowance of its own. When
the agents use the hour up the edge is still turned away until it
turns. That takes an identity of its own on the account (a GitHub
App's installation token), which is the owner's to make; #13489 stays
open for it.

**Not folded in, and next.** With the allowance used up and no
machine, something the owner does is kept for a machine, as it was for
the five minutes before. Now that the wait can be most of an hour, the
edge should do what it holds itself once GitHub answers again, and not
wait for a machine that may not come. That touches the rule against
doing anything twice (a machine may already have been handed it), so
it is its own change.

## 2026-11-23T22:40Z Update: the shared allowance, after review

Review of #13570 found two faults, both real, and one more was found
answering them.

- _The age shown was the last try's, not the copy's._ A document says
  when each of its lists was last tried (`read`), and a failed try
  moves that on as much as a good one: "read 12 min ago" would have
  said "just now" of a list hours old, and again after every failed
  try. Documents now also say when what is shown of each list was read
  (`good`), and the notice uses that. A copy of a document from before
  this (one kept on a device) gives no age, and none is made up.
- _An answer cut off part-way was kept under its tag._ A list whose
  body could not be read to its end was an empty answer, as before,
  and was then kept: GitHub's next "not modified" would have made it
  the answer again, every time, until the list changed. Only an answer
  that arrived whole is kept now.
- _An action could have been left to a machine for most of an hour._
  Whether this side was turned away is judged by its calls since an
  action began, whoever made them. With calls held back all through a
  wait, an action that failed for a reason of its own beside one of
  them would have been a machine's for the rest of the wait. An action
  is left for five minutes, as it always was; only a section read from
  named services is left until GitHub's time.

The type check that #13498 added to the deploy (by a fleet lane, while
this was in review) caught a fourth before it could reach the deploy:
a test passing a list where the page's function, read by the checker,
took none.

## 2026-11-24T01:45Z Correction: the kept answers were not kept

**What was wrong.** The two updates above say GitHub's answers are
kept and asked for again by their tags, so that a list that has not
changed costs nothing of the hour's allowance. As deployed (#13570)
that was not so. The answers, the times GitHub said to ask again and
the count of its answers were held in the Worker object's memory, and
the 18:10Z update gives the reason: persisting them was "considered
and not done", because "a restart costs one full answer of each, which
is less than the writes would". That took a restart to be rare. It is
the ordinary case. Cloudflare takes an idle Durable Object out of
memory after about ten seconds
([its lifecycle](https://developers.cloudflare.com/durable-objects/concepts/durable-object-lifecycle/))
and starts another, with nothing in memory, on the next request; this
object's work is a read every minute or so. Every read in service was
made by an object that had kept nothing, so every read was a full one
and nothing was saved. What the owner was told on 2026-11-23, that an
unchanged list no longer used the allowance, was true of the tests and
not of the Worker.

**How it was found.** By looking at the live Worker on 2026-11-24 at
00:45Z, two hours after the deploy: health showed
`github: {answers: 0, unchanged: 0, kept: 0}` seven minutes after a
self-check that had made four reads, with no deploy in between. The
local runtime showed the same when left alone: an object's memory was
gone forty seconds after its last request.

**Why nothing caught it.** Every unit test, every planted fault and
the real-runtime check read a list twice within one object's life.
The edge's tests have been able to start a new object over the same
storage since they were first written (2026-11-22), and the code
beside the memory says in three places that the object is put away
when idle; the memory was written as if that were rare, and none of
its tests started a new object.

**What changed.**

- _Kept in the object's storage._ An answer is stored in pieces of
  600,000 characters. A stored value holds about two million bytes,
  and the list of sixty open pull requests, as GitHub gave it on
  2026-11-24, was 1.4 million characters of two bytes each: three
  pieces. Each piece is marked with the address
  and tag it belongs to; one index names the pieces and is written
  last. An answer half stored when the object was put away, or when
  the store failed, never reads as an answer: its pieces do not carry
  what the index names.
- _Bounded._ At most 240 answers and thirty million characters; the
  one longest unasked-for goes first, never the one just kept. Places
  are few and used again, and whoever writes to a place or empties it
  clears whatever else is in it, so what a stopped write left behind
  lasts only until its place is next used.
- _One change at a time._ An object's first look reads every list side
  by side, none of them kept before: two answers kept at once would
  have taken the same place.
- _GitHub's wait and the count, the same way._ An object that starts
  during a wait keeps to it; "Try again now" is stored too. Health's
  `github.answers` and `github.unchanged` are counted since
  `github.since`, whichever object answered.
- _A store in trouble._ One that cannot be written is a memory that
  forgot. One that cannot be read is not written to (what it holds is
  not known, and is not to be written over with less), is tried again
  by the next to ask, and the object keeps to GitHub's wait in its own
  mind meanwhile. What is read back is held to what could have been
  written: a wait for no longer than one can be set for, an index line
  for no more than an answer kept here can be.
- _Health says when the object started_ (`woke`), so how long one
  lasts can be seen from outside.
- _Nothing left running._ `AbortSignal.timeout` leaves its timer
  running for the whole of the deadline after the request is over, and
  an object with a timer running is not put away. In the local runtime
  an object that had asked GitHub one thing (twenty seconds allowed)
  was still in memory 26 seconds after its last request and gone by
  32; one that had left no timer was there at 11 seconds and gone
  within 14. Deadlines are now made with `timed` (`src/timed.ts`), which
  clears its timer when the request is over, for GitHub, the push
  services, the key set, the released build and the dashboard. A test
  follows the Worker's imports and fails if anything it runs uses
  `AbortSignal.timeout`. An answer whose body is not going to be read
  (a push service's, a refusal's) is let go of at once, which the old
  deadline did only when it ran out.

**How it is checked now.**

- Unit tests start a new memory over the same store between reads: the
  tag, the wait, "Try again now" and the count all carry over. With the
  memory given no store, as first built, the edge's test of this fails.
- A property over generated runs of keeping, asking, letting go,
  starting again and being stopped part-way at any write (pieces as
  small as one character, so most answers are several): what is read
  back is an answer that was kept or nothing, each place holds its
  first so many pieces with none missing, the places are few, no more
  is kept than there is room for, and a change that finishes leaves
  nothing behind it.
- 73 single faults planted in the memory, the deadline and the edge's
  use of them: 71 fail a test. The two that do not each remove a check
  that another doubles (a missing index line, also caught by the catch
  around the read; a missing piece, also caught by the answer's
  length), and either way the read gives the same "nothing kept".
- The real-runtime check leaves the Worker alone until health says
  another object answers, then asks the first one's questions of it: a
  long answer (over a million characters of two units each, one cut in
  half by a piece's edge) is still asked for by its tag and GitHub's
  "not modified" still stands for it, a wait GitHub set is still kept
  to, "Try again now" lifts it, and the count goes on from the one
  before.

**What it costs.** One small write for each answer of GitHub's (the
count), and for an answer that changed its pieces and the index. One
hour of the object's own clock with nothing changing, in the edge's
tests: 60 reads of GitHub, all "not modified", and 74 writes to
storage, 60 of them the count. Storage is bounded at thirty million
characters of answers.

**Also here.** A test of the plan pages asked that the id roadmap have
at least ten rows. The planners tidied it to eight at 00:34Z, and from
then every run of the console's tests failed, which would have stopped
the next deploy. The test now asks that the roadmap files have rows
between them, and leaves how many each has to the planners.

**Considered and not done.** Writing the count less often (it would
no longer be exact, and it is how this fault was seen). Storing an
answer's pieces and the index in one batch, which the store allows up
to a size: it would remove the half-stored case in service, but the
case still has to read as nothing kept, and does. Listing the store to
sweep what stopped writes left: the list reads every value it lists.

**Not yet shown, as this is written.** That the deployed Worker now
gets "not modified" from GitHub itself. It is to be read from health
after the deploy: `github.kept` above nought and `github.unchanged`
growing while `woke` changes.

**Next.** The same wrong assumption may sit elsewhere in the object:
the ten-minute gate on asking for the released Android build is in
memory, so every object that starts asks again; the queue store is
asked with no deadline at all. #14001 has the list, to be gone through
one by one.

## 2026-11-24T02:30Z Update: in service, and the first of what else was only in memory

**In service.** #14000 was in service by 02:13Z as `fbbba7f607e9`. Over the
three minutes after, health read every twenty seconds had a different
`woke` on eight of nine reads: the object is put away in service as
quickly as supposed, and nothing in its memory lasts from one read of
a list to the next. In those minutes it asked GitHub nothing
(`answers: 0`): nobody had the app open, and its hourly check of its
own reads was not yet due. What GitHub answers the deployed Worker's
tagged reads is appended below when it has been seen.

**The first of #14001.**

- _The released Android build._ What GitHub last said of it, and when
  it was asked, were in memory, so the ten minutes between askings
  meant nothing: every object that started asked again. Stored now
  under `released`, read back once by each object, and removed when a
  description is found out of step with its file.
- _The queue store had no deadline._ Every other request the edge
  makes has one. It has twenty seconds for its answer and the body,
  and a read that runs out says so ("the queue store did not answer
  within 20 seconds"). The token is fetched before the time starts.
- _Health says what the object's own clock is doing_ (`clock.wanted`,
  `clock.next`). After the deploy nothing was asked of GitHub for
  three minutes, and from outside it could not be told whether no
  device takes notifications or the clock had stopped. The time it has
  asked to be woken at is the platform's to hold, so an object that
  has just started can say it.

**A fault in #14000's own hardening, found writing the first of
these.** A time read back from storage that was further off than could
have been set was cut down to the longest allowed, counted from now.
Every object that starts counts from its own now, so such a time would
have been cut down afresh each time and never come: for the released
build the gate would never have opened, and for GitHub's wait the edge
would never have asked again. Nothing the code writes is that far off,
so it would have taken a damaged record. A time yet to come on the
release's record is now asked for again at once, and a wait further
off than GitHub can set is not kept to at all. Both are tested across
several objects, which is what the first test of the wait's bound did
not do.

The same fault had been in the edge since step 3, in what it reads
back of being turned away (which actions and which lists are a
machine's for now): a time further off than could have been set was
cut down to an hour from now, and its test asked one object and then
moved the clock. Asked of a new object each time, as in service, a
year written down stood for a year. It is not believed at all now, and
the test starts a new object for every look.

**Checked.** The release's test is run again with a new object for
every request; 20 more single faults planted (the gate, the queue
store's deadline, the clock in health, the two bounds), all failing a
test; the real-runtime check passes with the clock said by health
(42 checks).

**Seen in service, 02:39Z.** The hourly check of its own reads ran at
02:39:26 and all four came back good. Health then said
`answers: 5, unchanged: 0, kept: 3`, and said the same from two
objects that started after it (02:39:58 and 02:40:28), with `since`
still 02:39:26: what is kept, and the count, outlive the object in
service. Two of the five answers were not kept because GitHub gives
its search answers no tag (checked against its API): ranked issues are
a search, a full answer every time, drawn on the search allowance and
not the hour's. Not yet seen: GitHub's "not modified" to the deployed
Worker. Nothing has asked for a kept list a second time; the next
check of its own reads is at 03:39Z.

**Seen in service, 03:42Z: "not modified".** The next check of its own
reads ran at 03:42:46, on a later deploy (`92e3463a139d`, the rest of
#14001 from a fleet lane, #14045). Health went from
`answers: 5, unchanged: 0, kept: 3` to `answers: 9, unchanged: 2`, and
a minute later `answers: 11, unchanged: 3`, with three kept
throughout. The object that asked had started at 03:34:44; the tags it
asked by were stored at 02:39:26 by another, two deploys back. GitHub
answers the deployed Worker's tagged reads "not modified", so the
saving that was claimed on 2026-11-23 and was not real then is real
now, within the two bounds already said: a search carries no tag, and
a list that has changed is a full answer.

One small thing seen with it: the check's time moved from 03:42:46 to
03:42:49 between two reads of health. Two health reads arrived while
the hourly check was due, and each ran it. At most a few reads of
GitHub once an hour, and nothing shown wrongly; noted and left.
