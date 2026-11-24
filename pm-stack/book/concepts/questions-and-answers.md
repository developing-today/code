# Question sets and the answer-back flow

## One system, eight surfaces

The PM stack asks Drew questions on eight surfaces. The eight surfaces are one system: a question is raised against an item in the Muse-side decisions ledger, presented on a surface, answered once, and applied back to the ledger. This chapter treats the eight as one flow with eight front doors. The surfaces are F1–F8, following the survey's numbering in `website-pages.md` §F.

Three invariants hold across the surfaces that record resolutions:

- **First resolution wins.** Per question, the first recorded resolution stands (`decisions.ts` `recordResolutions`).
- **Drafts die on resolution.** When a question resolves, its drafts are deleted (`pruneResolvedDrafts`).
- **An action that cannot be recorded is not taken.** Every console action is audit-logged before execution (`console/src/server.ts`).

## F1. Decision sets: the batched question set

A decision set mirrors a group of items from the Muse-side decisions ledger (`pending-questions.json`, per the header of `aggregator/src/decisions.ts`). A set has items, addressed by `ledgerId`. An item has 1–16 questions. A question has a `qid` of the form `\<ledgerId\>#\<1-based index\>`, 0–12 options, a mode of `single` or `multi` (version 1 defaults to `single`), recommended option ids, context, description, detail, group, order, and version history. A question with 0 options is a free-text question and is legitimate.

Surfaces: the index at `/decisions`, one page per set at `/decisions/\<setId\>?t=\<token\>`, and the console segment `#needs/questions`, which reads the same sets from the decisions file (`console/src/work.ts` `Work.questions()`). The console list shows open questions only, newest set first, with context clipped to 280 characters; `Work.question()` returns the full context for one question.

Answers: the set page posts to `/api/decisions/\<setId\>/submit`, `/defer`, and `/request-info`, and sends draft and edit events to `/api/decisions/\<setId\>/events`. A resolution is one of three kinds: `answered` (option ids, free text, comment), `deferred` (with a lean of `none`, `skeptical`, or `approve`), or `request-info`. The page supports submit-all. The console supports "Accept \<n\> recommended" per set (`Work.acceptRecommended`), batched at 200; the dashboard cap noted in code is 256 per request.

Write-back: resolutions are recorded in the aggregator's decisions store (`recordResolutions`), exported as resolution records for the ledger and fleet-feed path, and applied to the Muse-side ledger per question by the dropped-thread catcher. Decision events also sync out through `scripts/sync-decision-events.ts` to Turso and to the decision-events Worker, which fans new batches out live to the set page's WebSocket subscribers. The delivery path is in [Push notifications: detection to phone to answer](push-notifications.md).

Producers: `scripts/raise-decision-set.ts` posts a set through the alert throat (`POST /api/alerts`, `decisionSet` extension) and produces one push for the whole set. `scripts/submit-decision-set.ts` posts through `POST /api/decision-sets` and supports `--refresh` for standing ledger-mirror sets, including the standing open-questions set.

Two registry facts belong to this surface. The page manifest gives the `decisions` entry no API twin, yet the server implements the full decisions write API in this section (D7). And `/decisions/\<setId\>` is missing from the manifest's `SUBROUTE_PARENTS` map, so decision-page renders sit outside the render registry that `/api/health` reports (D8). The page exists; the health accounting does not cover it.

## F2. Needs-Drew single questions

A single raised question rides its push notification. The notification candidate carries the question: `NotificationCandidate.needsDrew = {ledgerId, question, options}` in `aggregator/src/notifications.ts`. The service worker turns the first two options into notification actions. A tap posts directly to `POST /api/needs-drew/\<ledgerId\>/answer` with the token and the chosen option id; no page opens. The answer page is the fallback and calls the same endpoint. The per-question token in the answer-page URL is the authority.

The write-back uses the same resolution machinery as F1: the ledger item's question resolves. One fact is UNVERIFIED and this book does not resolve it (D9): `decisions.ts` names a question store in `needs-drew.ts` distinct from the decision-set store, while `notifications.ts` and the service worker route single-question answers into the same machinery, and the console reads sets from "the decisions file" for the same questions. Which file a single-question answer ultimately writes is not stated unambiguously in the code surveyed. This chapter describes the endpoint and the ledger application, and names no store for F2.

## F3. Needs-Drew census sets: hands, reviews, escalated

The census is a different question shape: not a raised question, but every open item waiting on Drew. `GET /api/needs-drew.json` (`aggregator/src/needs-drew.ts`) collects open issues and PRs labelled `user/review` or `user/escalated` across the fleet repos, deduped, with `hands:v1` markers of kind `physical`, `interactive-login`, `payment`, or `signature`. The console splits the census (`Work.census()`) into three sets: hands (an item carries a `hands:v1` action), reviews (`user/review`, no hands marker), and other/escalated (no stated action; the fleet decides). Session-derived waiting items from `/api/attention.json` join the same view. On the phone the console names the segments Questions, Only you can do, Reviews, and Escalated (`console/public/app.js` `viewNeeds`); the default segment is chosen by counts: questions, then hands, then reviews.

Answers on census items are two console actions. Reply (`need.comment`) posts a GitHub issue comment carrying the marker `<!-- owner-reply:v1 -->`. I did it (`hands.done`, hands items only) posts the comment "Done by the owner…" and deletes the `user/escalated` label through `gh`. The aggregator census itself never writes labels; the console does (D4).

The census vocabulary in this section is the shipped vocabulary. The 2026-10-26 needs-user design document uses its own terms, and its acceptance line "#1007 appears in the queue" is impossible by design: #1007 is closed and the census is open-only. The code records that acceptance as "recorded on #1226 rather than claimed as met" (D13). This book uses the census vocabulary and treats the 2026-10-26 document as history.

## F4. Prompt and chat questions

The aggregator prompt surface (`/chat`) is a threaded question channel. Routes: `POST /api/prompt`, `GET /api/prompt/threads`, `GET /api/prompt/thread/\<id\>`, attachments under `/api/prompt/\<id\>/attachments/…`, `PATCH /api/prompt/\<id\>`, `GET /api/prompt/unread-count`, and a WebSocket at `/ws/prompt/\<id\>`. A prompt submitted with notifications on produces a `prompt`-class push when it is answered or fails.

The console `#ask` view is the owner-facing form of the same idea, built on GitHub and the queue instead of the aggregator store. `ask.send` files an issue and enqueues a top-band (priority 10) triage packet, marked `<!-- owner-prompt:v1 -->`. Replies are read back from the issue. `ask.reply` and `ask.close` complete the action set.

## F5. Dashboard comments and discuss

`POST /api/comments` records a dashboard-native comment. `POST /api/comments/triage` triages it, and `POST /api/comments/github-result` is the route the external poster uses to report the GitHub result back. The queue page is `/comments`; the twins are `GET /api/comments.json` and `/api/comments/queue.json`. The dashboard itself never writes GitHub: a fleet job applies and posts the comment, then reports the result back. A comment on an item is answered by the applying run's outcome.

## F6. Dashboard actions: structured answers on tier items

`POST /api/actions` and `POST /api/actions/batch` record structured answers against tier items. Consumers poll `GET /api/actions.json?state=new`; outcomes return through `POST /api/actions/result`. Action kinds: `triage` (acknowledge, snooze, escalate, de-escalate, answer, delegate, dismiss), `status-request`, `session` (poke, answer, stop; consumed by the fast session-action consumer for `opencode-laptop`), and `preference` (set-delegation-default). The triage `answer` verb is an answer-back: the consuming pass delivers the payload text, option, or answers as its own classification. An outage-queue variant exists at `GET /api/actions/outage-queue.json` with confirm and reject posts. Priority effects of these actions are in [Prioritisation: how order is decided and recorded](prioritisation.md).

## F7. Console generic actions

Every console answer in F1, F3, and F4 reaches its store through one endpoint: `POST /console/api/act`. The action table in `console/src/actions.ts` includes `questions.answer`, `questions.defer` ("Decide for me", lean default `approve`), `questions.accept`, `need.comment`, `hands.done`, `ask.send`, `ask.reply`, and `ask.close`, alongside queue, priority, issue, service, and PR actions. `Work.submit()` in the console calls the dashboard's own `/api/decisions/\<setId\>/submit`, so console answers and dashboard answers land in the same store.

When the laptop is unreachable, the console edge Worker holds the action and replays it through the same table when the host returns. Held actions and kept answers are the edge's whole job; the `#more/away` view shows what the edge currently holds.

## F8. Smaller answer-backs

- **Reports.** `POST /api/reports/resolve` resolves a prose conflict by choosing a stored version. `POST /api/reports/late-update` records a late update. Neither overwrites stored text silently; `/reports` exists to resolve conflicts without overwriting.
- **Roadmap annotations.** `POST /api/roadmap/annotations` and `POST /api/roadmap/annotations/alias` attach Drew-side notes and aliases to roadmap rows. The annotation store is the aggregator's own (`roadmap-annotations.ts`), reconciled at render (`roadmap-reconcile.ts`). An annotation edits neither Grist nor the repo files.
- **Alerts.** Acknowledge and close on `/alerts` write the incident lifecycle (`open | acked | closed`) in the alerts store. Preference toggles on `/alerts/preferences` write the subscription's class map, which the notification engine reads on its next tick.

Sources. Survey `website-pages.md` §F (F1–F8) and §§A–B, E; survey `roadmap.md` §4; survey `push.md` §§4a–4c; survey `disagreements.md` D4, D7, D8, D9, D13; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
