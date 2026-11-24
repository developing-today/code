# Quickstart

This quickstart does the three things the PM stack is used for every day: read the console, answer a waiting question, and follow an item. It assumes the stack is installed and reachable — see [Installation and bootstrap](installation.md) — and it uses only surfaces the survey counted: the console's hash views under `/console/` and the aggregator's pages behind the same gate at `pm.security.cab`.

## 1. Read the console

Open the console and land on `#now`, the Now view. Now shows the current board: work in progress and next-up work, from the console's work module — queue counts, claimed work, queued work, and blocked work grouped by reason. Read it top to bottom; it is the phone-sized answer to "what is happening".

Then read the other two daily views:

- `#next` — Next: the next-up work and an operations preview.
- `#more` — More: the index of the remaining console views, plus links to every aggregator dashboard page grouped by the dashboard's own page groups (Overview, Work, Ops, System).

The console has 21 hash views in total. The daily three are Now, Next, and Needs. The rest — queue, pull requests, ranked issues, agents, services, notifications, phones, roadmap, intake, and the platform and machine views — are reference material, listed view by view in the reference part of this book.

On the aggregator side, the front door is `/`, the Overview page: needs-you, projects, quotas, and health at a glance. Its machine twin is `/api/overview.json`. Every aggregator page has a twin of the same data unless the chapter for that page says otherwise; the decisions index is one of the exceptions in the manifest, although the server does implement a decisions write API (D7).

## 2. Answer a waiting question

Open `#needs` in the console. Needs is segmented, and the console picks the default segment by counts — questions first, then hands, then reviews:

| Segment                        | What waits there                                                                                                                           |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Questions (`#needs/questions`) | Decision sets: batched questions with options, raised for Drew                                                                             |
| Hands (`#needs/hands`)         | Items carrying a `hands:v1` marker: a physical action, an interactive login, a payment, or a signature — the four kinds the marker defines |
| Reviews (`#needs/reviews`)     | Open issues and pull requests labelled `user/review`, with no hands marker                                                                 |
| Other                          | Escalated items labelled `user/escalated` with no stated action — in the census wording, the fleet's to decide                             |

Answer a question in the Questions segment:

1. Open the set. In list view the context is clipped to 280 characters; the single-question read gives the full context.
2. Pick an option, or write free text. A question has 1–16 questions per set item and 0–12 options; a question with 0 options is a legitimate free-text-only question. A question's mode is single or multi; version 1 defaults to single.
3. Submit. Or defer — the console's defer action is labelled "Decide for me", with a lean that defaults to approve. Or request information instead of answering.
4. To clear a set whose recommendations you accept, use the set's accept-recommended action. It accepts the recommended option for each open question in the set, batched at 200 in the console; the dashboard cap noted in code is 256 submissions per request.

The same set exists on the aggregator at `/decisions`, and each set has its own page at `/decisions/&lt;setId&gt;`, reached with a token in the URL as `?t=&lt;token&gt;`. The token in the answer-page URL is the authority for that page. First resolution wins per question, and a draft is deleted when its question resolves.

What happens to the answer: the resolution is recorded in the aggregator's decisions store and exported for the decisions ledger; the dropped-thread catcher applies it to the ledger per question. The dashboard does not write the consequence itself. If the answer implies a Project status change, that change becomes a dashboard action and is performed by the orchestration pass, the Project's single writer. The full model is in [Question sets and the answer-back flow](../concepts/questions-and-answers.md).

Two neighbouring answer routes belong in a first pass:

- **Hands items.** Do the physical action, then use "I did it". The console posts a GitHub comment recording that the owner did it and deletes the `user/escalated` label. The aggregator census that listed the item never writes labels itself (D4).
- **Reviews and other items.** Reply from the console. The reply is posted as a GitHub issue comment carrying the `&lt;!-- owner-reply:v1 --&gt;` marker.

Every console action passes through one endpoint, `POST /console/api/act`, and is audit-logged before it is taken. An action that cannot be recorded is not taken. When the laptop is unreachable, the edge Worker holds the action and replays it through the same action table when the host returns.

## 3. Follow an item

Pick an item from Now, the board at `/board`, or the queue at `/queue`, and open its detail page at `/item/&lt;subject&gt;`. A subject names the source and the item: `gh:` for GitHub, `kaneo:&lt;slug&gt;/&lt;taskId&gt;` for a Kaneo card, `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;` for a roadmap row, plus `session:` and `orch:` subjects for sessions and orchestration items. An unresolved item id is a 404.

From the detail page:

- `/discuss/&lt;subject&gt;` is the discussion surface for the same item. Dashboard comments are posted through `POST /api/comments`. The dashboard itself never writes GitHub; a fleet job applies the comment and reports the result back.
- The projector's links for the subject connect the item to its counterparts in the other sources — the same work as a GitHub issue, a Kaneo card, and a roadmap row. See [The projector and projections](../concepts/projector.md).
- `/program` and `/program/&lt;id&gt;` place the item in its project: status, roadmap, tracked work, and the epic rollup it belongs to. An epic is a Project item with Kind equal to Epic, and its children are resolved through native GitHub sub-issue parents — not inferred from labels. The ladder is stated in [The roadmap hierarchy: initiatives, projects, epics, aggregators](../concepts/roadmap-hierarchy.md).

To watch work move rather than one item, use `/today` for the day's objectives and per-project momentum, `/queue` for in-progress, blocked, recently completed, and next-up work across projects, and `/events` for the cross-stack chronological stream.

## Ask for something new

`#ask` in the console is the owner's own-words composer: type or dictate an instruction, and the console files an issue and enqueues a top-band packet — priority 10, the owner-prompt band — for triage. The thread list under the composer shows everything asked, with replies, and carries Reply and Mark done. Intake for a note, bug, or change goes through `#more/intake`, which files the issue with the route's labels and rank and enqueues the packet in the same action.

Sources. Survey files `~/workspace/pm-book/survey/website-pages.md` (§A–§F, question sets F1, F3, F5, F7), `~/workspace/pm-book/survey/roadmap.md` (§1, §4), `~/workspace/pm-book/survey/disagreements.md` (D4, D7); repo paths `pm-stack/console/public/app.js`, `pm-stack/console/src/work.ts`, `pm-stack/console/src/actions.ts`, `pm-stack/console/src/server.ts`, `pm-stack/aggregator/src/pages.ts`, `pm-stack/aggregator/src/decisions.ts`, `pm-stack/aggregator/src/server.ts`.
