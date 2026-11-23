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

| Kind              | Becomes                                                                             | Labels                                            | Default rank |
| ----------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------- | ------------ |
| bug               | an issue, with a triage task                                                        | `kind/bug`, `source/human`, the capability's      | now          |
| note              | a comment on the issue it is about; else an issue with a triage task                | `roadmap/notes`, `source/human`, the capability's | next         |
| change            | an issue saying what to change, with a triage task                                  | `roadmap/notes`, `source/human`, the capability's | next         |
| any, as a session | a prompt (`owner/prompt`, `expedite`) that starts a session with the words verbatim | as above                                          | expedite     |

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
