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
