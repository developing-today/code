# Project Management

The Project Management stack (the PM stack) is the system Drew works in while the fleet works underneath it. Its own description, in `pm-stack/aggregator/src/program.ts`, is exact: this dashboard is the aggregator of the PM stack — the one screen Drew lives in while the fleet works in GitHub, Kaneo, and Pad underneath. The stack joins work that lives in many places — GitHub issues and pull requests, a Kaneo board, Grist roadmap tables, roadmap files in repositories, the fleet work queue, agent sessions, and quota and health data — into one state that can be read on a laptop and answered from a phone.

There is no single source of truth inside the stack, and the stack does not pretend otherwise. Each source keeps its own record. The aggregator reads the sources, the projector links items across them, and disagreements between sources are shown on a dedicated page instead of being hidden. See [The aggregator: one state, many sources](concepts/aggregator.md) and [The projector and projections](concepts/projector.md).

## Why it exists

Work in this estate is spread across four repositories in the core set — `developing-today/code`, `developing-today-forks/opencode`, `developing-today-forks/openchamber`, and `dezren39/darwin-nix` — and across the services that track it. Without the PM stack, answering three ordinary questions means opening each source in turn: what needs Drew, what is moving, and what is blocked. The stack exists to answer those questions in one place, and to carry the answer back.

1. **One state to read.** The aggregator polls and reads its sources and publishes pages and machine twins of the same data: an overview, a needs-you queue, a board, a roadmap, a program view, a queue view, quota and health views, and the rest of the 29 aggregator routes counted in the survey. The console presents 27 further views built for a phone. The full census is a reference chapter; the mental model is in [The shape of the system](getting-started/shape.md).
2. **Questions that wait in one place.** When work needs a decision, the question is raised as a decision set, a needs-Drew item, a prompt, or a console ask — eight question sets in total (F1–F8 in the survey). Each set has a defined answer route and a defined write-back. See [Question sets and the answer-back flow](concepts/questions-and-answers.md).
3. **Answers that reach the work.** An answer is not a note beside the work. A decision resolution is recorded and applied to the decisions ledger; a dashboard action is consumed by the orchestration pass, which is the single writer of Project fields; a console action is audit-logged before it is taken and then performed. The flows differ by surface, and this book keeps them separate.
4. **Push that reaches a phone.** Waiting questions and incidents are pushed to subscribed devices, and the phone is an answering surface, not only an alert surface. The relay that would answer at the edge is built but not deployed, and its answer routes are not routed (D5); today answers go to the laptop aggregator. See [Push notifications: detection to phone to answer](concepts/push-notifications.md).

## Who it serves

- **Drew, the owner.** The primary reader and the only person the needs-you surfaces are built for. He reads the overview and the console's Now view, answers questions in the Needs views, ranks issues, and follows items from issue to done.
- **The orchestrator.** The supervising agent work that keeps the fleet moving. It is the single writer of GitHub Project fields, consumes dashboard actions, and applies decision resolutions to the ledger. The aggregator service itself never writes GitHub labels; the console, as part of the same stack, does write labels through its own audited actions (D4).
- **Agents and sessions.** OpenCode and Muse sessions whose work is tracked as queue tasks, sessions, and items. Dispatch launches or enqueues their sessions; the claimant keeps claimed work moving; the registry records who ran what.
- **Operators of the stack itself.** Anyone keeping the laptop services, containers, edge Workers, timers, and guards running. The health page, the heal stack, and the operations chapters serve this reader.

## What the stack is made of

The survey counted 36 components under `pm-stack/` and its Nix packages. Grouped, they are:

| Group                     | What it contains                                                                                                                                                                  |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Core services             | The aggregator (port 4870), the console (port 4872), the projector (port 7801), and pm-dispatch (port 4879)                                                                       |
| External apps, integrated | Kaneo (execution board, port 5173), Grist (roadmap tables, port 8484), Pad (planning layer, port 7777), ready-for-agent (dispatch UI, port 6056)                                  |
| Edge Workers              | The console edge Worker, the decision-events Worker, the voice-line Worker, and the notify relay — the relay is recorded as not deployed in `pm-stack/platform/workers.json` (D5) |
| Execution                 | pm-dispatch, the OpenCode claimant, scheduled jobs, and the dispatch UI layer                                                                                                     |
| Guards and safety nets    | The Kaneo sync guard, the closure watchdog, the invariant sweeper, lane-guard, and the health hooks                                                                               |
| Quota and usage           | The quota gatherers and exporters, provider-usage, quota-check, and the usage surfaces behind the quota page                                                                      |
| Shared contracts          | The shared schema and the orchestration writer that writes orchestration state to the Turso `fleet-platform` store. See [The shared schema](concepts/shared-schema.md)            |

The component-by-component account is [Architecture](concepts/architecture.md).

## How to read this book

- **First hour:** [Installation and bootstrap](getting-started/installation.md), then [Quickstart](getting-started/quickstart.md). You will know what runs where, and you will have read the console, answered a waiting question, and followed an item.
- **Understanding the system:** [The shape of the system](getting-started/shape.md), then [Architecture](concepts/architecture.md), then the concept chapter for the part you are working in.
- **Answering and prioritising:** [The roadmap hierarchy: initiatives, projects, epics, aggregators](concepts/roadmap-hierarchy.md) states the real ladder — program, project, epic, task — and why "aggregator" is a service, not a hierarchy level (D10). [Prioritisation: how order is decided and recorded](concepts/prioritisation.md) covers the four places priority is recorded.
- **Looking something up:** the reference part lists every aggregator page, every console view, every question set, every worker, and every data store, one entry per item, in the same shape throughout.
- **Changing the stack:** the development and operations parts cover layout, building, adding a page or projection, running the services, and keeping this book current.

## Conventions used in this book

- **Code wins.** Where a design document and the code disagree, this book states the code behaviour and cites the disagreement by number (D1–D14). Example: the deployed Kaneo board is the upstream image pinned in `pm-stack/compose.yml`, not the fork the aggregator's program text names (D1).
- **UNVERIFIED means UNVERIFIED.** Where the survey could not establish a fact from the repository — a live process, an exact route, a version behind a digest — the chapter says UNVERIFIED in the sentence that needs the fact, and does not fill the gap with a plausible value.
- **Counts are counted.** Page counts, component counts, and class counts in this book come from the survey's direct counts: 56 primary pages and routes (29 aggregator, 27 console), 36 components, eight notification preference classes (D14).
- **Secrets are named, never valued.** This book names files such as `pm-stack/BOOTSTRAP-CREDENTIALS.md` as the place a bootstrap record lives. It prints no secret value.
- **Placeholders are escaped.** A value you supply is written as &lt;name&gt;, for example `/decisions/&lt;setId&gt;`.

Sources. Survey files `~/workspace/pm-book/survey/apps.md`, `~/workspace/pm-book/survey/website-pages.md`, `~/workspace/pm-book/survey/roadmap.md`, `~/workspace/pm-book/survey/disagreements.md`; book shape from `~/workspace/pm-book/skeleton.md`; repo paths `pm-stack/README.md`, `pm-stack/PORTS.md`, `pm-stack/aggregator/src/program.ts`, `pm-stack/platform/workers.json`.
