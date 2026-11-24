# Tutorial: answer a waiting question end to end

This tutorial answers one waiting question from the phone, and then answers the same kinds of question from the console. A waiting question arrives in three forms, and each form has its own answer surface:

- A **single needs-Drew question** rides a push notification. The first two options are tappable actions on the notification itself.
- A **decision set** is a group of questions from the decisions ledger. One push covers the whole set, and the set's page is the answer surface.
- A **needs-Drew census item** is an open issue or pull request labelled `user/review` or `user/escalated`. The console answers it with a reply, or with the hands-done action.

Do the form in front of you. Do not convert one form into another.

## Before you start

- The phone holds a push subscription. Set it up and check it on `/alerts/preferences` on the aggregator, or in the console at `#more/notify`.
- The question exists. A producer raised it in the Muse-side decisions ledger, or a fleet repo carries the item with a `user/review` or `user/escalated` label.
- For the console path, sign in to the console at `/console`. The console records every action in its audit log before it takes the action. An action the console cannot record is an action the console does not take.

## Path A: tap an answer on the push (single question)

### Step 1: Read the notification

The push for class `needs-drew` carries the question and its options in the notification data. The service worker in `aggregator/src/sw-source.ts` turns the first two options into notification actions. Options past the first two are not on the notification. Open the answer page for those options; Step 3 covers that page.

### Step 2: Tap an option

Tap one of the two actions. On `notificationclick` with an action, the service worker sends the answer directly:

```text
POST /api/needs-drew/&lt;ledgerId&gt;/answer
body: {token, optionId}
```

No page opens. The token comes from the answer-page URL's `?t=` value, which the service worker parsed and stored in the notification data as `answerToken`, alongside `answerLedgerId` and `answerUrl`. The per-question token in the URL is the authority for the answer.

Service-worker errors are silent. A tap with no visible result is not proof the answer landed. Check the result in Step 4.

### Step 3: Use the answer page when the tap is not enough

Open the answer page from the notification. The page presents the same endpoint through client code generated in `aggregator/src/server.ts`. Pick the option and submit. The request is the same `POST /api/needs-drew/&lt;ledgerId&gt;/answer` with `{token, optionId}`.

### Step 4: Confirm the write-back

The endpoint records the resolution against the ledger item's question, through the decisions machinery in `aggregator/src/decisions.ts` and `aggregator/src/needs-drew.ts`. The dropped-thread catcher then applies resolutions to the Muse-side decisions ledger, per question.

One limit stands in the code, recorded as D9: the surveys name `needs-drew.ts` as "the question store" for single questions and name decision sets as a distinct store, and the exact file a single-question answer writes is UNVERIFIED. This tutorial therefore names the endpoint and the ledger application, and no store file for the single-question path.

## Path B: answer a decision set on its page

### Step 1: Open the set

Tap the set's push. The tap opens `/decisions/&lt;setId&gt;?t=&lt;token&gt;`. Decision sets carry no tappable answers by design: "the page is the answer surface" (`aggregator/src/notifications.ts`).

A set has items. An item has 1–16 questions. A question has a `qid` of the form `&lt;ledgerId&gt;#&lt;1-based index&gt;`, 0–12 options, a mode of `single` or `multi`, and recommended option ids. A question with 0 options takes a free-text answer.

### Step 2: Answer, defer, or ask for information

On the page, the client sends one of three calls:

```text
POST /api/decisions/&lt;setId&gt;/submit        {submissions}
POST /api/decisions/&lt;setId&gt;/defer
POST /api/decisions/&lt;setId&gt;/request-info
```

Draft and edit events go to `POST /api/decisions/&lt;setId&gt;/events`. A resolution is one of `answered` (option ids, free text, comment), `deferred` (with a lean of `none`, `skeptical`, or `approve`), or `request-info`. First resolution wins per question. The store deletes a question's drafts when the question resolves.

Use submit-all on the page to answer the set in one pass.

### Step 3: Confirm the fan-out

Resolutions land in the aggregator's decisions store through `recordResolutions` in `aggregator/src/decisions.ts`. The store exports them as decision resolution records for the ledger and fleet-feed path. `scripts/sync-decision-events.ts` drains the decision-event outbox to Turso (the `fleet-platform` store) and to the decision-events Worker, which fans new batches out to the set page's live subscribers. The dropped-thread catcher applies the resolutions to the Muse-side ledger per question.

## Path C: answer from the console

### Step 1: Open the questions view

Open the console and go to `#needs`. The default segment is chosen by counts: questions first, then hands, then reviews. Open `#needs/questions` directly for decision sets.

The console reads the same sets from the decisions file through `Work.questions()` in `console/src/work.ts`: open questions only, newest set first, context clipped in the list view. `Work.question()` returns the full context for one question.

### Step 2: Answer through the action table

Use the action that matches the item:

- `questions.answer` — answer one question in a set.
- `questions.accept` — accept the recommended options for a set. The console batches this action at 200; the dashboard cap noted in code is 256 per request.
- `questions.defer` — defer, recorded in the console as "Decide for me", with lean default `approve`.
- `need.comment` — reply to a census item. `Work.needComment` posts a GitHub issue comment carrying the `&lt;!-- owner-reply:v1 --&gt;` marker.
- `hands.done` — close out a hands item. `Work.handsDone` posts a "Done by the owner…" comment and deletes the `user/escalated` label through `gh`.

All five go through the audited action endpoint:

```text
POST /console/api/act
```

`Work.submit()` in the console calls the dashboard's own `POST /api/decisions/&lt;setId&gt;/submit` for set answers, so a console answer and a page answer land in the same store.

One correction to older design text, recorded as D4: the aggregator service never writes GitHub labels. The console does, through the actions above. The orchestrator is not the only label writer in the stack.

### Step 3: Handle the away case

When the laptop is unreachable, the console edge holds the action instead of dropping it. The edge replays the held action through the same action table when the host returns. The console's `#more/away` view shows what the edge holds: held actions and kept answers. Do not re-submit a held action from a second surface; the replay delivers the first one.

The edge answer-intake route in the notify relay is a separate path, and it is not routed. See [Tutorial: trace a push notification from event to answer](trace-a-push.md) and D5.

## You are done when

- The question no longer appears as open in `/decisions`, in `#needs/questions`, or in the needs-Drew census for its item.
- The resolution is in the decisions store, and the ledger application has a record to apply for that question.
- For a census item answered from the console, the GitHub item carries the owner-reply comment, and a hands item no longer carries the `user/escalated` label.

Sources. survey/website-pages.md §F1–F3, §F7; survey/push.md §4a–§4c; survey/disagreements.md D4, D5, D9. Ground truth: laptop clone at 4d937b3, 2026-11-23.
