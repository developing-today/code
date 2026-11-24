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

## 2026-11-24T04:30Z Update: what is held, done here when the edge can (#13831)

**What it was.** Something the owner did while the edge was turned
away and no machine could be reached was held for a machine, and only
a machine ever did it. Since the edge keeps to GitHub's own time for
asking again, that could be most of an hour of waiting for a laptop
that was not coming, for a comment or a rank the edge could have made
itself a few minutes later.

**What it does now.** When it can act again (the time GitHub gave has
come, the hour it leaves a kind alone is up, or the owner says to try
again) the edge wakes itself and does what it holds of the kinds it
runs: the oldest first, each thing's actions in the order they were
done, and nothing about a thing behind one it could not do. The wake
is the object's own alarm, asked for whenever something is kept; there
is no other place the held list is worked through.

**Never twice.** An action is one side's or the other's, never both.
(As first written here this was wrong in one case, found before it was
merged: see the correction below.)

- _A machine's_ once a report's answer has handed it out: the machine
  may be doing it. The edge never takes it, nor anything else about
  the same thing while it waits.
- _The edge's_ once it is out of the held list, which happens, and is
  written down (`taken`), before anything is begun. A machine that
  reports meanwhile is not handed it, nor what waits behind it about
  the same thing (that would be done out of turn).
- _Turned away again_ before anything was written, it goes back where
  it was, a machine's by name as before.
- _Put away part-way_, it is settled by the next object from its
  record: ended; never begun, and it waits again; or begun and not
  known to have ended, which neither side does again, and the owner is
  told to look.

**Four faults in what was there, found doing this.** The edge already
did an older held action itself in one case, when a newer one about
the same thing arrived (#13398, a fleet lane's #13652).

- It began the older one while it was still in the held list. A
  machine reporting before it ended was handed it too, and would have
  done it a second time.
- Turned away while trying it, the record that made the name a
  machine's was cleared and not put back: sent again under that name
  later, it could have been done here and by a machine.
- A held rank was dropped as "replaced" by any newer action about the
  same issue, a comment included. Only a newer rank replaces a rank.
- An older held action that only a machine can do (marking a need
  done) was tried here when a comment on the same need arrived, failed
  for want of the means, and was lost. The newer one now waits behind
  it.

**Also.** The day's limit on what is held was only kept when a machine
reported; it is kept on the edge's own wake now. A pass that could do
nothing is not tried again for a minute, so that a fault cannot have
the object waking every second.

**Checked.** Fifteen tests at the edge, each starting from the owner's
action being held: done here once with no machine; at GitHub's time
and not before; in order; a handed one left alone, with what waits
behind it; a machine reporting half-way through; a newer one arriving
half-way through; turned away again; put away part-way (each of the
three records); the last rank winning, and a comment not replacing
one; what only a machine can do left for it; a side that no longer
does the core doing none of it; the day's limit with no machine; a
wake asked for with nothing else to wake it; and the pause after a
pass that did nothing. 41 single faults planted in the new code: 40
fail a test. The one that does not takes a failure of storage in the
middle of an action for being turned away; the store the tests have
cannot fail in the middle of one and work again for the next write,
so the two cannot be told apart there. In the real runtime: with the
host out of reach and GitHub's stand-in turning the Worker away, a
comment is held and nothing written; when it answers and the owner
says to try again, the Worker's own alarm fires and the comment is
made once.

**Not done.** An action handed to a machine that then never reports
stays the machine's until the day is up: the edge cannot know whether
it was done. Every kind is kept to GitHub's time, though moving a task
asks only the queue store.

## 2026-11-24T05:00Z Correction: an action sent to the host may have been done there (#13831)

**The fault, mine, found reading the change before merging it.** The
rule above took "never handed out in a report's answer" to mean "never
reached a machine". It does not. Nearly everything held gets there by
being sent to the host and getting no answer, and no answer has two
meanings: the host never had the request, or the host did it and the
answer was lost on the way back. The second kind was done again by the
edge once it could. A test written first showed it: a comment the
laptop made, its answer lost, the laptop then away; an hour later the
edge made the same comment a second time. The code this was built on
(#13652) had the same hole in its one case, and waking by itself made
it reachable with nobody doing anything.

**What can be known.** What comes back from asking the host is one of
three things: the host's own answer; nothing (the time ran out, the
connection broke); or an answer from what stands in front of it. Only
Cloudflare's 530 says the host never had the request. Cloudflare gives
it (with error 1033 in the body) when the tunnel has no connection to
send anything down, which is what a laptop asleep or off looks like.
Its documentation says so of both: the 530 page, and the 1033 page
("cannot find a healthy `cloudflared` instance to receive the
traffic"). A 502 from the tunnel's own daemon does not say as much (it
is also what a connection lost part-way looks like), and is treated as
no word.

**The rule now.** A held action is the edge's to do only when it is
known to have reached no machine: it never set off toward one, or it
set off once and that one asking was answered 530. It then carries the
word `unsent`. Without the word it is a machine's by name, as
everything held was before this change: handed to the host when it
reports, which says what became of it. The word is the positive one on
purpose. What is held in service today was kept by a version that did
not write down how the asking ended, and has no word either way; read
as "may have", it is left to the laptop.

**The same name, asked twice.** A device gives up on a send after a
minute and offers the name again, while the edge gives the host a
little longer. So two askings under one name can be out at once, and
word that the second never arrived says nothing of the first. Every
setting-off is counted under the name, and a 530 is believed only for
a name that set off once. Three more cases follow from the same
thought:

- One waiting here that reached nobody is not sent toward the host
  when its device offers it again. It would be on its way there while
  the edge may be taking it for its own.
- A name the edge has taken to do itself, offered again in that
  moment, is not sent to the host as well. The device is answered with
  what came of doing it here, or told that it is not over.
- A name whose first asking is still out, offered again while
  something else about the same thing waits here, is kept as the
  host's.

Also: a newer action about a thing waits behind an older one the host
may have, where before it waited only behind one handed out (a newer
rank would otherwise have replaced an older rank the host may be about
to set, and the older would have been the last word). And a host's
answer that cannot be read settles the name as any answer of the
host's does; it used to leave a hold under it.

**What the owner sees.** Each held action is one of three, and is said
as that when it is kept, in "What you did", on the Machines screen and
in the note the device keeps: either side's, whichever can first; a
machine's only; or one a machine was given or may have got, which
nobody else does meanwhile. Dropped after a day, the last kind is said
to be worth checking, not "not sent". The health answer counts the
three (`held`), so that what the Worker made of a laptop being away
can be seen from outside.

**Checked.** Thirteen more tests at the edge (28 now for this change),
the first of them written before the fix and seen to fail: the answer
lost; out of reach each way, side by side; offered again while
waiting; offered again with the first asking still out; the same with
something else waiting; a newer one behind one the host may have, a
rank among them; offered again at the moment the edge has taken it,
done and turned away; one kept by the version before; a name from
before the counting; an unreadable answer; and a take that could not
be written down leaving nothing marked as being done. In the real
runtime the Worker's own reading of what comes back is exercised both
ways: the host's stand-in answering 530, the comment is held as either
side's and the Worker makes it once; answering 502, it is held for the
host and the Worker does not make it, able as it is.

71 single faults planted over the whole of the change: 68 fail a test.
The three that do not change nothing that can be seen. One takes a
failure of storage in the middle of an action for being turned away
(the tests' store cannot fail in the middle of one and work again for
the next write). One drops the check the taking itself makes that a
machine does not have the action: everything that takes asks the same
thing first, and between the asking and the taking there is nothing
but the object's own storage to wait for, during which Cloudflare lets
nothing else in. One drops the limit on how many times a newer action
looks again at what is ahead of it, which cannot be reached. The
Worker's reading of the host's answer is outside those tests, so it
was turned each way in the real runtime: taking everything for "never
reached", the Worker makes the comment it should have left to the
host; believing nothing, it leaves the one it should have made. Each
fails the checks it should.

**Not done, and filed.** The 530 has not been seen in service: that
needs the owner's action to be turned away by GitHub while the laptop
is away. What it was taken for is then in the health answer. An older
action still out at the host does not hold back a newer one about the
same thing that the edge can do at once (a rank sent to the host,
then another the edge makes directly within the minute): the edge does
not keep track of askings that are out by what they are about.

## 2026-11-24T05:55Z Update: what is labelled as needing the owner, read at the edge (#13491, first part)

**What it was.** "What needs you" came from the laptop's dashboard,
which censuses two GitHub labels (`user/review`, `user/escalated`)
across the four repositories. With the laptop away the list was as old
as its last report, and nothing told the owner of a new thing only he
could do.

**What it does now.** The edge reads it from GitHub itself, like the
owner's prompts: each repository's own list of open issues carrying
each label. The three lists the app shows (what only he can do,
reviews, the rest) are made by one function, used for both readings,
so the app shows the same thing whoever read it. The dashboard's two
readers of an item's body (a stated reason, the hands mark of
FDR-0013) are copied into `src/needs.ts`, because the dashboard's
module needs a machine's files and cannot be part of the Worker; a
test holds each copy to the original for any body, and another holds
the whole list read here to the list the dashboard makes of the same
issues.

**Why the lists and not the search.** The lists are on the hour's
allowance and answer "not modified" by their tags, so eight lists that
have not changed cost nothing; the search is counted by the minute,
gives no tags, and lags. And the search no longer takes the question
the way the dashboard asks it: see below.

**How it fails.** A repository the token cannot see is named and the
rest is shown, with the app saying the list is not whole. Called too
often, out of reach, or GitHub in trouble: the read ends at the first
such answer and nothing more is asked. No repository readable at all
is a failed read, so the last good copy stays and says why. Refused
for all of it, the list is left to a machine for the hour, as with
anything else the edge reads. A machine that has not said which
repositories to look in goes on reporting the list itself.

**Three faults found on the way, none of them in this change.**

- The dashboard's census asks the search for
  `repo:<repo> state:open label:"<label>"`. GitHub answers that 422
  ("Query must include 'is:issue' or 'is:pull-request'"), and its
  documentation says so of any such query. Asked from here both ways I
  can ask, that is the answer. If the dashboard's token is answered the
  same, its census has been empty and marked incomplete, and "What
  needs you" has shown nothing. I cannot see the laptop to confirm it.
  Filed as #14103 with the check to run there. With this change the
  app's list no longer depends on it.
- The screen never said that a list was incomplete: an empty,
  incomplete census read as "Nothing needs your hands." It says so now,
  and names what could not be read where the reader says.
- Each card's "updated" time was cut like a title, with a mark at the
  cut, which made it no time at all: every card said "updated" and
  nothing after. It is cut to the second now.

**Checked.** 459 tests pass (22 new: 13 of the reading in
`test/needs.test.ts`, two of the console's own command for it, six at
the edge, one of the app's wording). At the edge: read from each
repository's list with no machine; a repository that cannot be seen;
none that can; by tag, with only the list that changed read whole and
nothing written again; refused and left to the laptop; a laptop that
names no repositories; and a new hands item told to a subscribed
device by the edge's own wake. In the real runtime (47 checks): the
Worker's own check of the read comes back good, the stand-in's one
review is shown, and the lists were asked for. 40 single faults
planted in the new code: all 40 fail a test.

**One thing it costs.** When GitHub turns the edge away for the
hour, two sections of the first screen now ask at once (the prompts
and this), so two calls are refused before the wait is kept to, where
it was one.

**Not done.** Questions and sessions are still a machine's (#13491
stays open). What a session is waiting on the owner for comes from the
dashboard's own tiers, not from labels, and is still the laptop's.
Whether the edge's token can see all four repositories has not been
seen in service; if it cannot, the screen names the ones it cannot.

## 2026-11-24T06:55Z Update: seen in service, and the dashboard's census put right (#14103)

**Seen in service.** At 06:45:48Z the edge's hourly check of its own
reads ran with the new read in it: `selfCheck.kinds.census` is `ok`,
`refused` is empty, and `github.kept` went from 3 to 11, one for each of
the eight lists. All four repositories are readable by the edge's
token, so the list the app shows is whole.

**The dashboard's census.** `pm-stack/aggregator/src/needs-drew.ts`
asked GitHub's search for `repo:<repo> state:open label:"<label>"`,
which GitHub answers 422 to. It now asks each repository's own list of
open issues by label, as the edge does: the same items, on the core
allowance, with no index to lag behind. An answer under a 200 that is
not a list is a failed query, never an empty queue (the list has no
"incomplete results" flag to say so itself). A test stands in for
GitHub as it answers today, refusing the old form with a 422, and
fails on the census as it was. The console's test that holds the two
readings to each other now also holds them to asking for the same
pages.

**The repositories, named two ways.** The console's list of where to
look (`needsRepos`) named two forks and a repository by names they no
longer have. The dashboard's list was repointed when they changed (the
two forks to another organisation on 2026-11-04, the third renamed on
2026-11-08, #5595); the console's, written on 2026-11-22, took the old
names. Nothing was seen to fail: all eight lists answered the edge
under the old names, and asking git where each name leads gives the
same commit for both of each pair, two of them pushed within hours, so
GitHub answers the old names with the present repositories. But an item
was `developing-today/opencode#5` in the app and
`developing-today-forks/opencode#5` on the dashboard, and the console
takes a reply only for an item in a repository on its list: a reply
from the app to an item the dashboard had named would have been refused
as "not an item on the Needs me list". Whether an answer by way of an
old name costs a second request against the allowance was not measured.
The console's default is the dashboard's list now, and a test holds the
two to each other.

The edge is told the list by the laptop, which is the larger fault: the
laptop's checkout is behind main (#13833), so the edge goes on asking
by the old names until the laptop runs this. Which repositories the
platform looks in should be the platform's own to hold, not whatever
the last machine to connect was running (#14163).

**Not seen.** The laptop's dashboard itself: it gets this when its
checkout follows main again (#13833).

## 2026-11-24T07:20Z Plan: the platform's settings are its own (#14163)

**What is so.** The Worker holds no settings of its own. Every report
from the laptop carries the laptop console's whole configuration, and
the Worker stores what it is sent and acts by it: who may sign in, which
repository and queue, where the queue's store answers (which is where
the queue's token is sent), which repositories are looked in for what
needs the owner. So the platform runs by the environment and the
defaults of whatever the laptop's console was started from.

That is `~/git/code` on the laptop, which agents use as a working tree
(the lanes that looked report in #13833 that it had edits nobody had
committed on 2026-11-23, and was on a lane's branch the day after; the
console has run `f86b52989c31`, hours of commits behind main, since).
Two things follow. A correction to a setting that is
merged and deployed to Cloudflare changes nothing in service until the
laptop runs it: today's correction of the repositories' names
(#14165) is the case in hand. And a default changed on a branch nobody
merged is told to the Worker as readily as main's: for where the
queue's token is sent, that is a wider door than main.

**Whose each setting is.**

- _The repository's_: the repository and the queue, where the queue's
  store answers and what its token is called, the Android app's
  package, the label that marks an instruction, and the repositories
  looked in for what needs the owner. Nothing on the laptop sets any of
  them; they are defaults in the console's code.
- _The deployment's_: who may sign in (the owners, the sign-in issuers,
  the certificate names) and the fingerprints of the Android signing
  certificate. The script that sets the hostname up writes them into
  the laptop console's environment, and the laptop tells them.
- _A machine's own_: its name, whether it has a signed build to hand
  out, what it can do. These are facts it reports, not settings.

**The plan, in order.**

1. _One definition, and see first._ The repository's settings are
   written once (`pm-stack/console/src/settings.ts`), and both the
   console and the Worker carry them. The Worker says in its health
   answer in which of them the machine told it otherwise, by name and
   never by value (`machineSettingsDiffer`). It still acts by what it
   was told. What that shows in service decides step 3: the console
   that tells them is behind main, and whether any of them was set by
   hand on the laptop cannot be known from here any other way.
2. _The last action about these repositories that only a machine does._
   "Done" on something that needed the owner's hands is a comment and a
   label taken off, and is sent to the machine, which takes it only for
   a repository on its own list. Once the Worker goes by its own list a
   machine on another would refuse an item the app shows. The Worker
   does it itself, as it does a reply.
3. _The switch._ The Worker acts by the repository's settings, as its
   own deploy carries them. What a machine tells for them is compared
   and shown (Machines, with both values, to the owner signed in) and
   is not taken. Where the queue's token is sent is then decided by
   main alone.
4. _With #13495._ The deployment's settings are applied from Actions,
   and a machine's report stops changing who may sign in. Not before:
   the script on the laptop is the only thing that sets sign-in up
   today, and a Worker that stopped listening to it would keep the old
   sign-in the next time it is set up again, and lock the owner out.
5. _Settings changed from the app._ Which repositories are looked in is
   the first: kept at the edge, told to machines. After 3.

**Kept true on the way.** Sign-in never opens wider: steps 1 to 3 do
not touch who may sign in or how it is checked. No value of any
setting is said to anyone not signed in.

## 2026-11-24T07:35Z Update: what the Worker says in service, and the list of repositories set apart (#14163)

**Seen in service.** Deployed as `e053b2753c3b`, the Worker says
`machineSettingsDiffer: ["needsRepos"]`. The laptop tells the
repository, the queue, where its store answers, its token's name, the
Android package and the instruction label exactly as main has them:
none of them is set by hand on the laptop. So the Worker going by its
own for those six changes nothing it does today.

**The list of repositories is not like the other six.** A machine takes
a reply, or "done", only for an item in a repository on its own list
(`Work.ref`). The Worker does a reply itself, but leaves it to a
machine when its own token is turned away, and "done" is a machine's.
Whether the Worker's token may write in the other three repositories
cannot be known from here: it reads them, which anyone may. If the
Worker read by main's list while the laptop still had the old one, the
app would show `dezren39/darwin-nix#12`, and a reply left to the laptop
would be refused as not on its list. A reply that works today would
fail until the laptop ran main. So the plan above is changed in two
places:

- Step 3 is for the six. For the list, the Worker goes on by the
  machine's while a machine tells one. It says that the list differs,
  and the Machines screen says which list is gone by and why.
- The list becomes the platform's own together with what makes that
  safe: a console that takes the platform's list from the Worker's
  answer to its report (step 5 needs that anyway), or an action sent
  only to a machine that lists the repository (#13492). Until then the
  correction in #14165 takes effect when the laptop runs main (#13833).

Step 2, "done" at the edge, no longer has to come first. It is still
worth doing for itself (#13491).

## 2026-11-24T07:45Z Update: the Worker goes by its own for the six (#14163)

**Done.** What a machine tells is kept as it told it (`cfg` in the
Worker's store, as before). What the Worker goes by is worked out from
it: the repository, the queue, where the queue's store answers and what
its token is called, the Android package and the instruction label are
the ones its own bundle carries (`goesBy` in `src/settings.ts`); who
may sign in and the list of repositories are the machine's. So those
six change with a merge and a deploy and with nothing else, and where
the queue's token is sent is no longer anything a machine's report can
move.

Two things follow from the Worker knowing where things are by itself.
It does the core as soon as a machine has said who may sign in, whether
or not that machine's console is new enough to say where the queue is
(before, such a console left the Worker serving reports and doing
nothing). And it still does nothing before any machine has said who
may sign in: that it cannot know.

**Said to the owner.** The Machines screen says, for a machine that
runs with other settings, each one with what the machine says, what
main says, and which of the two Cloudflare goes by; and it says when a
machine and Cloudflare are on different commits (the half of #13833
that is about saying it). The values are shown only signed in; the
health answer goes on saying names only.

**Checked.** In the real Worker runtime the stand-in host now tells
others than main's for all six, throughout: every read and action in
the run has to be by main's, and none of what reached the stand-ins for
GitHub and the queue store may name anything the host told. With
`goesBy` made to pass on what it was told, that check fails (5 of 21
calls named the host's). Under bun, 26 faults planted one at a time
were each caught.

**Not seen.** Nothing in service changes by this today: the laptop
tells those six as main has them. What will be seen is the Machines
screen saying that the laptop's list of repositories is not main's,
and that it is on other code.

## 2026-11-24T08:10Z Update: "done" on what needed the owner's hands, at the edge (#13491)

**Done.** "I did it" on something only the owner could do was a
machine's: with the laptop away it waited. The Worker does it itself
now, as it does a reply. It is two things asked of GitHub, and the
order was changed for both sides: the label is taken off first, then
the fleet is told. Taking a label off twice is the same as once
(GitHub answers "Label does not exist", read from GitHub itself on
2026-11-24), and saying "done" twice is two comments. So a failure
part-way is tried again from the start, by the owner or by whoever
holds the action for him, and it is said once. Any other failure to
take the label off stops it there with nothing said. The list of what
needs the owner is read again before the answer goes back.

**A rule with no kind of today's to apply to.** What comes after
something only a machine can do, about the same thing, waits behind
it. "Done" was the one kind of that sort that shares what it is about
with something the Worker does (a reply). The rule is kept for the
next such kind, and its test stands one in by telling the Worker under
test not to do "done" (`direct.kinds`).

**Found on the way.** The app had no words for the Worker being turned
away for reading what needs the owner: it would have shown
`read.census`. A test now holds the app's words to the Worker's own
lists of what it does and reads.

**Checked.** In the real Worker runtime: the label off, then the
comment, each with the Worker's token and once, no machine asked, and
the item gone from the next look. Nine faults planted one at a time
under bun, each caught.

## 2026-11-24T08:25Z Correction: the dashboard's census was not failing in service (#14103)

The section of 06:55Z says the dashboard's census "asked GitHub's
search for `repo:<repo> state:open label:"<label>"`, which GitHub
answers 422 to". That was read from what GitHub answered the token of
the session that wrote it, and never checked against the dashboard that
runs. Checked on the machine that runs it (`amd`, 08:10Z):

- `GET http://127.0.0.1:4870/api/needs-drew.json`: `complete: true`,
  eight of eight sources good, 54 items, fetched a minute before.
- The search in the old form, asked with the fleet's identity:
  `total_count 13`. The repository's own list: the same 13.

So the 422 is what GitHub gives some tokens (a GitHub App's) and not
the one the dashboard runs with. Nothing the owner saw was broken, and
nothing was fixed for him by #14165. What stands of it: the census
reads the same items from the list, on the core allowance, whichever
kind of token it is given; and the console's list of repositories is
the dashboard's. The issue's title, its priority and the comments in
the code that said "every query failed" are corrected.

**What it shows about checking.** A claim about what is in service was
made from outside the service. The machine can be asked now (below),
and a claim of that kind is made from its answer.

## 2026-11-24T08:30Z Update: the live pages, looked at on the machine

The console at main, started on the machine beside the one in service
(its own port, its own empty state, no report to the edge), read by a
headless browser at a phone's width. What was wrong, and what was done:

- **"394 need you"** on the first screen, and "99+" on the tab: 347 of
  them were questions the fleet had put with an answer it recommends.
  What leads now is what only the owner can do (29); the tab's count
  adds the reviews asked of him. The Needs screen opens on that list,
  and the questions have one button that takes every recommendation.
- **"Why 272 tasks are blocked"** stood above six lines that added up
  to fifteen: almost every blocked task carries a reason of its own, so
  the commonest six said nothing of the rest. The board now counts them
  by the kind of reason (one written on the task, a time waited for and
  whether it has passed, other tasks, none), which adds up to all of
  them. The query is run on a real SQLite in its test.
- **A task being worked on could not be steered from its card.** It has
  "Comment" (on its issue, which Cloudflare posts itself) and "Message
  its agent".
- **"failed · failed"** on a failed unit: said once.
- The console in service on the machine runs from `~/git/code`, which
  was on a lane's branch: the app it served was that branch's, and its
  code was seven hours behind main (#13833). Not fixed by this.

## 2026-11-24T08:40Z Update: the app is told when something changes

**Intent.** The app looked every twenty seconds. What changed in
between was seen at the next look, and every look was a request
whether or not anything had changed. The owner asked for an app that
receives events as they happen.

**Done.** The documents class says when what a document would say has
changed (`Docs.onChange`: a section read with other content or another
health, a value beside the sections, what the owner did), and never
for a read that changed nothing. The Worker's object holds a WebSocket
for each open copy of the app, by the platform's hibernation API, and
passes those changes on: which documents, once for however many changes
arrive together. The app reads them again by their tags. While someone
is connected the object wakes every twenty seconds to read what has
gone stale; with nobody connected it does not.

**Not over the connection.** What the owner does. It goes by requests
that are kept on the device and sent again, which a connection that may
drop is no place for.

**Checked.** In the real Worker runtime, with a real WebSocket: the
owner is connected and a stranger is not (401), the platform answers
the app's ping, something the owner does is said to the connected app
by the document's name, and nobody is counted as connected after the
app goes away. Nineteen faults planted one at a time: eighteen caught,
and the nineteenth showed a flag that did nothing, which was removed.

**Not seen.** A phone connected in service: `listening` in the health
answer says when one is.

## 2026-11-24T08:45Z Update: a notification carries what can be done about it

**Intent.** A notification said that something had happened and opened
the app. The owner asked for notifications with the details in them and
buttons that answer.

**Done.** Each message has up to two buttons, most useful first: on a
reply from the fleet, "Reply" (typed on the notification where the
device can take a line of text; where it cannot, the thread is opened)
and "Close it"; on something only he can do, "I did it" and "Open"; on
a unit that failed, "Restart it" and "Look". The body says enough to
act on. Pressing a button that does something goes through the same
keeping and sending as the app: written down on the device first, sent
under a name, kept and sent later with no connection, never done twice.
The Android app shows the site's notifications as its own, so it has
the buttons too.

A message had no bound on its length: a body of twenty thousand
characters was sealed and posted as twenty thousand bytes, which a push
service refuses (it takes 4096, RFC 8030 section 7.2). With fuller
bodies that mattered more, so a message that would be too long as it is
sealed is now cut to the longest start that fits, in whole characters,
marked as cut, with its buttons kept.

**Checked.** The service worker's file is run against a stand-in for
the browser's worker scope: the buttons shown, what pressing each does
(one request, as the app would send it), a typed reply, no text given,
no connection, the platform turning it down, the app open. Fourteen
faults planted one at a time, all caught. The cut is measured on what
is really posted, against the 4096 bytes, and read back the way a phone
reads it, for characters of one, two and four bytes and one that JSON
writes as two; eight faults planted, all caught. (Cutting by code unit
and not by character would give the same result, since half a
character costs more bytes written out than the whole one: it is cut
by character because that is what makes the search plainly right.)

**Not seen.** A real phone showing the buttons: it takes a real push to
a subscribed device, which is the owner's.

## 2026-11-24T09:25Z Update: what was finished lately

**Intent.** The app said what is happening, what is next and what needs
the owner. It did not say what was done. He asked for that.

**Done.** A `done` document, read by Cloudflare itself like the rest of
the core: the tasks the queue says were finished, the pull requests
merged and the issues closed, newest first, with how many of each in
the last day. Now has a fifth tile for it; the Done screen lists each
kind, with what came of a task (merged, issue closed, left as a draft
with a follow-up, superseded, no work) and links to its pull request
and issue.

**How it is read.** The queue has some seventeen thousand finished
tasks and keeps no order of when each was finished, so going through
them would read every one, every time. The events that say a task was
finished are kept in order of time: the read starts from those of the
last week and looks each task up by its number. SQLite's own plan for
the statements is checked in the tests (the events by their index, the
tasks by their key, nothing gone through whole). GitHub is asked for
one page of what is closed, most lately touched first, at one unchanging
address, so an unchanged answer is "not modified" and costs nothing.

**Checked.** The statements are run in SQLite on the queue's own tables
with rows written the way the fleet writes them: finished once, twice,
taken up again, another queue's, older than the week, written by
something older. GitHub's list is given as GitHub documents it. In the
Worker's real runtime the document is read with no machine. Fifty
faults planted one at a time across the read, the wiring and the app's
words, all caught after three tests were tightened.

**Seen on the machine,** drawn from the live queue and GitHub: 715
tasks finished in a day (292 second passes over an issue, 142 feature
slices, 140 small fixes, 130 triage), at least 46 pull requests merged
and at least 39 issues closed. That look changed the screen. Of the
forty latest finished tasks, thirty-seven were second passes with no
record of what came of them, so the tasks tab now says the day by kind
and lists the tasks with a result on record first. Every merged pull
request said the same author (the owner's name, which his agents work
under), so the author is said only where they differ. A lane the pool
made up ran across two lines as forty letters, and is said as a pooled
lane.

**Not seen.** The screen as Cloudflare serves it to a signed-in owner.

## 2026-11-24T09:29Z Correction: whole numbers from the queue's store came as text

**Found.** The first look at the Done screen on the machine, drawn from
the live queue: 709 tasks finished in the last day, and not one of them
showed its issue. The tasks had issues. The queue's store sends a whole
number over HTTP as text marked as an integer (`{"type":"integer","value":"7920"}`),
and the console read the text and dropped the mark. Every screen so far
had coped, by treating numbers loosely. The new read asked for a number
and, given text, showed nothing.

**Why the tests had not said so.** The tests' stand-in for the store
answered with plain numbers, which the store never does. The oracle did
not match the thing it stood in for.

**Fixed.** The store's values are read by their marks: a whole number
that a number can hold exactly is read as that number, one that is too
large stays the text it came as, and text that looks like a number
stays text. The work tests' store now answers the way the real one
does, so a read that mishandles it fails there. Checked against the
store's own format, six faults planted and caught, and seen on the
machine afterwards: each of the forty latest with its issue, where none
had shown one.

## 2026-11-24T09:55Z Update: a question, something to do, or a standing instruction

**Intent.** Ask had one box, and everything typed into it became
"something he wants done, or a question he wants answered", for an
agent to work out which. He asked to be able to ask questions and give
general directives as well as start work.

**Done.** He says which it is: "Do this", "Answer this" or "From now
on". The issue is titled for it and its note and task ask for what that
kind needs. A question is answered in a comment with what the answer
rests on; nothing is changed to answer it, and the issue is left open
for him to close. A standing instruction is recorded where every agent
reads it, by a merged pull request, and work already queued or under
way is brought in line with it. Something to do is as it always was,
down to the last character of what is filed and queued. The kind
travels with the task when Cloudflare files the issue and a machine
files the task later, and a reply on a question is queued as a question.
The list marks questions and standing instructions, a kept one is said
as what it is, and the fleet's reply on a question is announced as its
answer.

**What it does not do.** A standing instruction is still carried out by
whichever agent takes its task: nothing here writes to AGENTS.md
itself. The queue has one key for "someone must look at this issue",
and all three kinds use it, so an issue never has two tasks.

**Checked.** The issue and the packet for each kind are compared with
what each must make an agent do differently, written out in the tests;
something to do is compared with what was filed before, whole.
Twenty-two faults planted one at a time, all caught. In the Worker's
real runtime, 54 of 54.

## 2026-11-24T10:14Z Update: a machine's console runs from its own copy of main

**What was wrong.** On the laptop the console ran from the checkout the
fleet's lanes work in. That checkout is not kept at main: lanes leave it
on their own branches, with half-written files, and the unit that would
pull main into it rightly refuses to. So the console in service was
main's code from the day before, and from 08:39Z on 2026-11-24 another
branch's code altogether (`9c2b4ad096e3`, a commit of a design branch),
put into service by the console's own redeploy when the checkout was
switched to that branch. Five passes over #13833 by five lanes each
found it still so, and changed nothing.

**The source of truth is GitHub's main, not a directory on the laptop.**
A machine's console now runs from a copy that is its own, under
`~/.local/share/pm-console`: a clone that fetches only main and only
the console's part of the tree, a release laid out for each commit, and
a link to the one in service. Every minute `redeploy.sh` asks GitHub
for main; when the console's code there is not what is in service it
lays the commit out as a release, starts it in a self-check, and only
then moves the link and runs the release's own `install.sh`, which
restarts the service and waits for it to answer its health check as
that commit. A release that does not start or does not come up does not
stay: the one before it is put back and the unit fails, where the
Services screen and the notifications show it. GitHub out of reach
changes nothing, and is a failure after half an hour.

**What a machine is, after this.** Still one of the machines that
report to Cloudflare, with what only a machine has: its units, the
dashboard, the fleet's own commands. What code it runs is no longer a
fact about a directory someone works in.

**Checked.** Both scripts are run for real: against a bare repository
standing in for GitHub (reached over git's own protocol, allowing the
partial fetches GitHub allows), with stand-ins only for the program
that runs the self-check, for `systemctl` and for the health check.
What is in service is compared with the files of the commit pushed to
main, read back from git. A checkout on another branch with
half-written files, the script run from inside it: main is what goes
into service, and the checkout is left as it was. Thirty-three faults
planted one at a time, all caught after one test was tightened.

**Not yet.** The edge still cannot tell by itself that a machine runs
other console code than it does: each says a commit, and two commits
can hold the same console. A release now records its console's tree
beside its commit, which is what the two sides would compare (#13833's
third point).

## 2026-11-24T10:36Z Seen in service: the laptop follows main by itself

**The move.** At 10:19Z on 2026-11-24 the new `redeploy.sh` was run once
by hand on `amd`. It made the console's own copy of main, checked it
with the service's settings, and put it into service:
`redeploy: e16c2459a649 is in service`, with the service's working
directory a release under `~/.local/share/pm-console`. The unit files
from before are kept on that machine at
`/tmp/pm-review-claude/units-backup`. It had first been run there
against GitHub with a made-up home directory and stand-ins for
`systemctl` and the health check, which took 23 seconds and passed.

**By itself.** Two merges later the same morning were followed with
nobody at the machine: `6dfb0637a5bb` at 10:23:27Z, about a minute after
it merged, and `4a7a5161e257` at 10:33:32Z, before Cloudflare's own
deploy of the same commit had finished. Read from outside at 10:35Z,
`version` and `hostVersion` were both `4a7a5161e257`. The console's
other two timers (the public address, the Android release) have each
run from the new place and finished.

**What else it settled.** `machineSettingsDiffer` on the Worker's health
went from `["needsRepos"]` to empty: the laptop had been telling the
list of repositories that its old code held, and main's console takes
the platform's.

**What stopped a deploy on the way.** The console's deploy for the
change itself (`e87bfb92c726`) failed at its tests, on a test that had
nothing to do with it: the repository's list of Workers had changed on
main hours before (`1ea1346348`), a console test said how many Workers
and reads to expect, and nothing had run the console's tests for that
change. The test now takes what it expects from the list (#14290). The
gap is #14297: the console's tests run only when the console deploys.
The laptop took `e87bfb92c726`'s console regardless, since it follows
main and not the Worker's deploy.

**The same fault elsewhere.** The Services screen, looked at on the
machine, listed `fleet-queue-backup-export.service` as failed: last
night's export of the queue store did not run, because its unit points
into a lane's worktree that has been removed (#14302). A service on the
laptop running out of a directory someone works in is what #13833 was.

## 2026-11-24T13:34:00Z Feature: the app's look is the owner's to choose

**Asked for.** "Implement a wide variety of themes", "multiple full
black OLED themes to choose from, high contrast, and settings to change
the font, along with a dozen or more fonts", "to change the spacing
around elements", and that "consistent UI elements are used throughout"
and it is "straightforward to implement new pages".

**What was built.** A screen, More → Appearance, with three parts:
colours (33 themes: Slate and Paper as before, eight true black, three
high contrast, thirteen dark, seven light), typeface (22: three of the
device's own and nineteen the app carries, each under the SIL Open Font
License), and size and spacing (seven text sizes, five amounts of room,
four kinds of corner, movement on, off or as the device asks). The app
follows the system between a theme for light and a theme for dark, each
chosen, or wears one always.

**How.** Each choice is one attribute on the root element. `look.js`
(a plain script in the page's head, after the style sheets) reads the
choices from the device and sets the attributes before anything is
drawn; `themes.css` has a block of thirteen colours per theme, and
`look.css` a rule per typeface and per step. Nothing writes a style, so
the content policy is untouched. A theme block put on any element
colours just that element, which is how the chooser shows every theme
at once without a second copy of any colour.

The style sheet was already drawn from thirteen colour variables. What
it lacked was the same for everything else, so every padding, margin
and gap now goes by `--space`, every corner by `--round`, every hairline
by `--edge`, the text size by `--scale` and the typeface by `--font`;
the one rule with colours of its own (the badge on a tab) takes the
theme's. With every setting at its default the page is pixel for pixel
what it was. Tests refuse a rule that writes its own colour, a bare
pixel padding or a bare radius, which is what keeps one element looking
like the next as screens are added.

**Decisions.**

- _Kept on the device, not with the account._ A phone with an OLED
  screen and a monitor want different things, and the choice has to be
  there before the first read of anything.
- _Contrast is a rule, not a hope._ Every theme is held, by the WCAG
  formula, to 7 to 1 for text and 4.5 to 1 for small print, the status
  colours and the ink on each; the high-contrast ones to 15, 10 and 7,
  with edges at 3 to 1. The well-known palettes are their makers'
  colours with any that missed a rule moved toward white or black until
  it met it. Paper's own edge colour measures 1.29 to 1 on the page; the
  rule for an edge was set at 1.25 so that the look the app shipped with
  is not changed by this.
- _True black means `#000000`._ The page and the panels of a true-black
  theme are exactly that; a card is told from the page by its edge.
- _Typefaces are carried, and fetched only when used._ They are the
  Latin files Fontsource publishes (npm), 19 families in 22 files, 870
  kB in all, with each licence beside them. A browser fetches a file
  only when something is set in it; the service worker keeps what was
  fetched. Opening the typeface chooser shows each as itself, which
  fetches them all once.
- _One list of the app's files._ Adding the style sheets and typefaces
  would have meant editing four lists (the laptop's service, the Worker,
  its build, the service worker) that a test held together.
  `public/files.json` is now the only one, read by all four; the service
  worker reads it when installed and keeps what it marks as needed to
  open. A new file for the app is one line.

**Checked.** 594 tests. Thirty-four faults were planted one at a time
(a faint colour, a near-black called black, day and night swapped, a
choice not kept, a typeface missing from the list, the build shipping
less, a bare padding, and so on) and each was caught. In the real
Worker runtime the smoke run fetches all 31 files as the signed-in
owner and compares each with the file on disk (56 checks pass). On the
laptop, a copy of the branch was opened in Chromium at a phone's width
and driven: the default look is unchanged; choosing Black with Inter
gives a page whose background the browser reports as `rgb(0, 0, 0)`
and a bar colour of `#000000`; all 22 typeface files load; pressing a
theme, then "One theme", then another keeps and shows each.

## 2026-11-24T14:26:00Z Feature: Trends, read and kept by Cloudflare; charts; pages as modules

**Asked for.** "Build some analytics dashboard pages that show: trends
over time, PRs merged, major feature items merged per project, and so
on", and that it be "straightforward to implement new pages for both
the website and the app".

**What was built.** A Trends screen (More → Trends) over 7, 14, 30 or 90
days: pull requests merged by kind, features merged by project with
each project's latest features listed, issues opened against closed,
bugs reported against fixed, the middle time from opened to merged, and
tasks finished by kind; five leading numbers, each set against the same
number of days before. A chart kit (`public/charts.js`) and the first
page that is a module of its own (`public/pages/trends.js`).

**Where the numbers come from.** Whoever serves the app reads them, by
the same code on Cloudflare and on a machine (`src/trends.ts`); no
machine is needed.

- _Tasks_ are one read of the queue's finishing events grouped by day,
  by the index the Done screen uses, each task once on the day it was
  last finished.
- _GitHub has no "merged per day" to ask for._ Search would answer it,
  and is refused to some tokens and limited to thirty questions a
  minute. The list of issues is not: asked for everything changed at or
  after a time, oldest change first, it gives every issue and pull
  request once, at its last change (checked on GitHub itself: an item
  is in the answer to its own time). Read a page at a time, each page's
  last change being where the next starts, that is everything touched
  since, each seen at least once in its present state however they
  change meanwhile. What a pull request is comes from its title
  (`feat(scope): …`); the project from the scope.
- _Newest first._ The first version read from ninety days back toward
  now, which made the days anyone looks at the last to be whole. It is
  read in stretches instead: the last day to the end of the list (that
  reading then goes on for good, asking from where it stopped), then
  three days back, a week, two weeks, a month, two months, ninety days,
  each up to where the one after it began. The screen says from which
  day on the counts are whole, and a total is set against the days
  before only when those are whole too.
- _Kept small, in the Worker's own storage_: per day, the numbers of
  what was opened and closed, and for a merged pull request its kind,
  project, minutes open and (a feature only) its words. A day is about
  50 kB at this repository's rate.
- _Not kept among the answers it asks for again._ The Worker remembers
  GitHub's answers by address to ask "changed?" cheaply. An address
  with a time in it is never asked twice, so those are not remembered:
  they would push out the ones that are.
- _The screen waits for GitHub only when the wait is short._ Looked at
  on the laptop, a screen that read three pages on at every opening
  made every opening slow for as long as the older stretches were
  coming in. It waits now only for the first reading (until the last
  day is in), for one look at what changed once the history is whole,
  and when the owner presses refresh. In between it answers from what
  is kept and says how far back that is whole; the rest is read behind
  it, by Cloudflare's clock, or after the answer on a machine.
- _It reads on only while somebody looks._ For a day after the screen
  was last asked for, the object's own clock reads the history on (six
  pages a minute until whole, then what changed, each half hour). After
  that it does not wake for it, which keeps to the rule that an object
  nobody needs is not kept awake; the tests that hold that rule pass
  unchanged.

**Measured on the laptop, reading the real repository** (the machine's
own reader, through the fleet binary): after 44 asks of the screen
(three pages each) in about eight and a half minutes, everything
changed in the last two weeks had been read. A day of changes here is
several thousand items, because sweeps touch old ones.
With the history whole back to 11 November, the 23rd read as 526 pull
requests merged: 151 features, 95 fixes, 71 docs, 29 tests, 180 other;
the middle pull request of the week waited five minutes to merge.

**Charts.** The kit's rules are written at the top of the file and held
by its tests. Colour says which series a mark belongs to and nothing
else: six colours in a fixed order (blue, orange, aqua, yellow, magenta,
green), in a set stepped for dark themes and a set for light ones,
chosen with a palette validator that measures whether neighbours can be
told apart under each kind of colour vision. Run against the card
colour of all 33 themes, nothing failed; on eleven (the light ones,
Nord and Dracula) one or more colours has under 3 to 1 against the
card, which the rule answers with what every chart has anyway: a legend
with each series' total, every series read out for the day pointed at,
and the numbers as a table. "Everything else" is always grey; the
status colours are never used for a series. One scale to a chart;
columns at most 24 wide with gaps and rounded tops; today drawn fainter
because it is not over; arrow keys, a pointer or a finger choose a day.
Nothing writes a style: marks take classes, the classes take the
theme's variables.

**Pages as modules.** `app.js` is one file of four thousand lines. A new
screen can now be a file in `public/pages/` that exports its address,
title, tile and `view`; the app gives it its route, its tile on More
and its offline copy, and hands it the app's own parts. Adding one is
the file, a line in `pages/index.js` and a line in `files.json`; a test
checks the three agree. The element builder moved to `public/ui.js` so
that pages and the app build with the same one.

**Checked.** 636 tests. Sixty-five faults planted one at a time; the
five that survived were closed (a one-word scope taken for a kind; an
older stretch counting what was not its own, now held by which days it
writes; a condition that could not be false, removed; the dot at the
end of a tile's shape drawn half off its edge, which is how it first
looked on the laptop; Cloudflare reading on behind an answer instead of
by its clock). In the real
Worker runtime (58 checks) the Worker reads the history from the
stand-in GitHub, keeps it in its storage, and answers a second ask
without asking again. In Chromium on the laptop, on the real
repository's numbers, in a dark theme and a light one: the charts draw,
the series take the theme's colours, and arrowing to a day reads it
out.

## 2026-11-24T14:46:00Z Feature: reports, made when asked for

**Asked for.** "To request reports which generate and are accessible
through the app."

**What was built.** More → Reports: a day, a week or thirty days in
sentences. What merged and how that stands against the time before, the
features by project with the latest three of each to open, issues
opened and closed and what that did to the backlog, bugs, the queue's
finished tasks, and, in a report that ends today, the queue as it
stands. Earlier and Later step a report back and forth by its own
length as far as the history goes. Copy and Share give it as text.

**Decisions.**

- _Made from the history, not written and stored._ The history behind
  Trends is kept by day, so a report for any stretch of it can be made
  at any time; the read was given a day to end on, and a single day as
  a window. That leaves nothing to store, nothing to go stale, and no
  machine in it. The second page written as a module of its own, it is
  one file, a line in the list of pages and a line in the list of
  files.
- _It says what it cannot vouch for._ A total is set against the days
  before only when those are whole; a report over days not yet read
  says from which day on its numbers are whole; one that ends today
  says the day is not over.
- _What it does not cover goes to Ask._ A question there is answered by
  an agent, with what the answer rests on. A kind of ask that is "a
  report" with its own listing on this screen is not built: it needs
  the queue task filed, which is still a machine's (#13490).

**Checked.** 639 tests; every sentence of a report is worked out by
hand from a small history written out in the test. Fourteen faults
planted, the one survivor closed (every feature listed instead of the
latest three).
