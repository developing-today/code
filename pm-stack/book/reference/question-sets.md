# The question sets, enumerated

A question set is any surface where the stack asks the owner a question and writes the answer back. The survey enumerates eight, F1–F8. Every entry below follows the same shape: what the set is, where it surfaces, who answers, where the answer writes back, and its failure shapes and open points where the survey records them. The routes named here are specified in [Aggregator HTTP routes and API](aggregator-routes.md); the pages and views are specified in [The aggregator website, page by page](website-pages.md) and [The console, view by view](console-views.md).

## F1. Decision sets (aggregator)

- **What it is:** The main batched question set. The model is `pm-stack/aggregator/src/decisions.ts`: a set mirrors a group of items from the Muse-side decisions ledger (`pending-questions.json`, per that file's header). A set has items (`ledgerId`); an item has 1–16 questions; a question has a `qid` of the form `&lt;ledgerId&gt;#&lt;1-based index&gt;`, 0–12 options (`{id, label, tags?, combinationOf?}`), a `mode` of `single` or `multi`, `recommendedOptionIds`, context, description, detail, group, order, and version history (`DecisionQuestionVersion`, #9335 v2).
- **Defaults:** Version 1 defaults `mode` to `single`. A free-text-only question — 0 options — is legitimate.
- **Where it surfaces:** The index `/decisions`, the per-set page `/decisions/&lt;setId&gt;?t=&lt;token&gt;`, and the console view `#needs/questions`, which reads the same sets from the decisions file (`console/src/work.ts`, `Work.questions()`): open questions only, newest set first, context clipped to 280 characters in the list, full context through `Work.question()`.
- **Who answers:** The owner, on the set page or in the console. Producers raise sets: `scripts/raise-decision-set.ts` (through `POST /api/alerts` with the `decisionSet` extension, one push per set) and `scripts/submit-decision-set.ts` (through `POST /api/decision-sets`, v2, with `--refresh` for standing ledger-mirror sets such as the standing open-questions set).
- **Where the answer writes back:** The page client posts to `/api/decisions/&lt;setId&gt;/submit` with `{submissions}`, to `/defer`, and to `/request-info`; draft and edit events go to `/api/decisions/&lt;setId&gt;/events` (alias `/api/decision-sets/&lt;setId&gt;/events`). Resolution kinds (`DecisionResolution` in `decisions.ts`) are `answered` (an `optionId` or `optionIds`, plus free text and a comment), `deferred` (with a lean of `none`, `skeptical`, or `approve`), and `request-info`. First resolution wins per question; drafts are deleted when a question resolves (`pruneResolvedDrafts`). Resolutions record in the aggregator's decisions store (`defaultDecisionsFile`, `recordResolutions` in `decisions.ts`), export as `DecisionResolutionRecord` values for the ledger, `listDecisionResolutions`, and fleet-feed path, and — per the header of `decisions.ts` — the dropped-thread catcher applies them to the ledger per question. Decision events sync out through `scripts/sync-decision-events.ts` to Turso and to the decision-events Worker.
- **Bulk answers:** The page offers submit-all. The console offers "Accept &lt;n&gt; recommended" per set (`Work.acceptRecommended`, batched at 200; the dashboard cap noted in code is 256 per request).
- **Push shape:** One push covers the whole set, and its tap opens the set page. A decision set has no tappable notification answers: the page is the answer surface.

## F2. Needs-Drew single questions (aggregator, #8568)

- **What it is:** A single raised question riding its notification candidate: `NotificationCandidate.needsDrew = {ledgerId, question, options}` in `src/notifications.ts`.
- **Where it surfaces:** The push notification itself, and the per-question answer page. The push's first two options become notification actions (`src/sw-source.ts`).
- **Who answers:** The owner, by tapping a notification action or by using the answer page. On a tap, the service worker posts directly to the answer endpoint with no page open; errors in the service worker are silent, and the answer page remains the fallback.
- **Where the answer writes back:** `POST /api/needs-drew/&lt;ledgerId&gt;/answer` with `{token, optionId}`; the per-question token in the answer-page URL is the authority. The write-back is the same decisions-ledger resolution machinery as F1: the ledger item's question resolves.
- **Single-question store (D9, settled in source):** `decisions.ts` states that sets and the per-question store in `needs-drew.ts` are not the same store. `recordAnswer` in `pm-stack/aggregator/src/needs-drew.ts` writes it to the needs-Drew question store, the file at `cfg.needsDrewFile`. That path is `PM_AGG_NEEDS_USER_FILE`, else the legacy `PM_AGG_NEEDS_DREW_FILE`, else `needs-user.json` (or the legacy `needs-drew.json` where only that exists) under `~/.local/state/pm-aggregator/`, resolved by `needs-user-state-path.ts`. Decision sets keep their own store, `decisions.json`.

## F3. Needs-Drew census sets (console `#needs`)

- **What it is:** The census of work waiting on the owner, built from `GET /api/needs-drew.json` (`src/needs-drew.ts`): open issues and PRs labelled `user/review` or `user/escalated` across the fleet repos, deduped, with `hands:v1` markers (`kind: physical`, `interactive-login`, `payment`, `signature`).
- **Where it surfaces:** The console `#needs` view. `Work.census()` in `console/src/work.ts` splits the census into three sets: **hands** (an item with a `hands:v1` action), **reviews** (`user/review`, no hands marker), and **other/escalated** (no stated action — the fleet's to decide). Session-derived waiting items from `/api/attention.json` (`Work.sessions()`, tier `needs-drew`) join the same view.
- **Who answers:** The owner, in the console.
- **Where the answer writes back:** **Reply** posts `act("need.comment")`, which runs `Work.needComment`: a GitHub issue comment carrying the marker `&lt;!-- owner-reply:v1 --&gt;`. For hands items, **I did it** posts `act("hands.done")`, which runs `Work.handsDone`: a GitHub comment ("Done by the owner…") and deletion of the `user/escalated` label.
- **Disagreement carried:** D4 — the aggregator census itself never writes labels; the console writes them. D13 — the shipped census vocabulary (hands, reviews, escalated) is the book's vocabulary; the 2026-10-26 needs-user design doc is history.

## F4. Prompt and chat questions (aggregator `/chat`, console `#ask`)

- **What it is:** Free-form prompts and their threads. Aggregator side: `src/prompts.ts`, `src/prompt-ws.ts`, `src/prompt-notify.ts`, `src/prompt-widget.ts`, `src/prompt-attachments.ts`.
- **Where it surfaces:** The aggregator page `/chat`, and the console view `#ask`. The console view is the owner-facing version of the same idea against GitHub and the queue, not against the aggregator store.
- **Who answers:** Fleet sessions answer prompts; the owner asks, replies, and closes.
- **Where the answer writes back:** Aggregator routes: `POST /api/prompt`, `GET /api/prompt`, `GET /api/prompt/threads`, `GET /api/prompt/thread/&lt;id&gt;`, `GET /api/prompt/&lt;id&gt;`, attachments at `GET /api/prompt/&lt;id&gt;/attachments/…`, `PATCH /api/prompt/&lt;id&gt;`, `GET /api/prompt/unread-count`, and the WebSocket `GET /ws/prompt/&lt;id&gt;`. A prompt submitted with notifications on receives a `prompt`-class push when it is answered or fails (#10035). Console side: `act("ask.send")` files an issue and enqueues a top-band (priority 10) triage packet (`promptPacket` and `notePacket` patterns in `console/src/work.ts` and `console/src/plan.ts`, marker `&lt;!-- owner-prompt:v1 --&gt;`); replies read back from the issue (`ask.reply`, `ask.close` in `console/src/actions.ts`).

## F5. Dashboard comments and discuss (aggregator)

- **What it is:** Comments on dashboard items, written from `/discuss/&lt;subject&gt;` and tracked on `/comments`.
- **Where it surfaces:** The discuss page per item, and the comments queue page.
- **Who answers:** The owner comments; the applying fleet run answers by its outcome.
- **Where the answer writes back:** `POST /api/comments` (dashboard-native comment; store `src/comments.ts`), `POST /api/comments/triage`, and `POST /api/comments/github-result`, where the external poster reports the GitHub result back; reads are `GET /api/comments.json` and the queue twin. The dashboard itself never writes GitHub — a fleet job applies and posts and reports back (`src/actions.ts` header). This set is a question-and-answer-back surface in the loose sense: a comment on an item is answered by the applying run's outcome.

## F6. Dashboard actions (aggregator)

- **What it is:** Structured answers on tier items: the action records of `src/actions.ts`.
- **Where it surfaces:** The dashboard's tier items; consumers poll for new actions.
- **Who answers:** The owner issues actions; consumer runs apply them and return outcomes.
- **Where the answer writes back:** `POST /api/actions` and `POST /api/actions/batch` create actions; consumers poll `GET /api/actions.json?state=new`; outcomes return through `POST /api/actions/result` with `{id, outcome: applied|rejected, byRun, note}`. Kinds: `triage` (acknowledge, snooze, escalate, de-escalate, **answer**, delegate, dismiss), `status-request`, `session` (poke, **answer**, stop — consumed by the fast session-action consumer for `opencode-laptop`), and `preference` (set-delegation-default). The triage `answer` verb is an answer-back: the pass delivers payload text, option, or answers like its own classifications (`actions.ts` header).
- **Outage variant:** `GET /api/actions/outage-queue.json`, `POST /api/actions/outage-queue/confirm`, `POST /api/actions/outage-queue/reject` (`src/outage-queue.ts`).

## F7. Console generic actions (`POST /console/api/act`)

- **What it is:** The console's audited action table: `console/src/actions.ts` (`ownActions`, `actions(...)`, `perform`).
- **Where it surfaces:** Every console view posts here; the question actions are `questions.answer`, `questions.defer`, `questions.accept`, `need.comment`, `hands.done`, `ask.send`, `ask.reply`, and `ask.close`, alongside the queue, priority, issue, service, and PR actions.
- **Defaults:** `questions.defer` is the "Decide for me" action; its lean defaults to `approve`.
- **Who answers:** The owner, from the console.
- **Where the answer writes back:** Through the same stores as F1–F4: `Work.submit()` calls the dashboard's own `/api/decisions/&lt;setId&gt;/submit`; needs and hands answers write GitHub comments and labels as in F3; asks file issues and queue packets as in F4.
- **Behaviour:** Every action is audit-logged before it is taken — an action that cannot be recorded is not taken. When the laptop is unreachable, the edge holds the action and replays it through the same table on the host's return.

## F8. Reports, roadmap annotations, and alerts

- **What it is:** Three smaller answer-backs, grouped by the survey.
- **Reports:** `POST /api/reports/resolve` resolves a prose conflict by choosing a stored version; `POST /api/reports/late-update` records a late update. Surface: `/reports`. The answer writes back to the report store, and stored text is not overwritten.
- **Roadmap annotations:** `POST /api/roadmap/annotations` and `POST /api/roadmap/annotations/alias` (`src/roadmap-annotations.ts`). Surface: `/roadmap`. Annotations write back onto roadmap rows.
- **Alerts:** Ack and close on `/alerts` write the incident lifecycle (`open`, `acked`, `closed` in `src/alerts.ts`). Preference toggles on `/alerts/preferences` write the subscription's class map, which the engine reads on its next tick (`src/notifications.ts`, `NOTIFICATION_CLASSES`). D14: the class count is eight, not the six of issue #8882's premise.

Sources: PM-SURVEY website census §F (`pm-stack/aggregator/src/decisions.ts`, `pm-stack/aggregator/src/notifications.ts`, `pm-stack/aggregator/src/needs-drew.ts`, `pm-stack/aggregator/src/actions.ts`, `pm-stack/console/src/actions.ts`, `pm-stack/console/src/work.ts`), survey files `website-pages.md`, `push.md`, and `disagreements.md` (D4, D9, D13, D14), census date 2026-11-23.
