# PM stack needs-user action model — design (2026-10-26)

> Living document — committed 2026-11-11 from ~/workspace/pm-stack-needs-user-actions-design-2026-10-26.md as the tracking home for a completed design (60-day design register entry P1.4; register committed as pm-stack/design-register-2026-11-11.md in developing-today/code, tracking epic developing-today/code#8805). Status: shipped. Update this doc as the design shifts; the register is the frozen audit snapshot.

---

# PM stack — needs-Drew actions, custom options, surveys, and delegation modes

**Date:** 2026-10-26 (America/Chicago) · **Track:** design only — no code changed by this document.
**Parent design:** [pm-stack-coherence-design-2026-10-26.md](sandbox://workspace/pm-stack-coherence-design-2026-10-26.md) (epic [developing-today/code#1184](https://github.com/developing-today/code/issues/1184), Cluster 3). This addendum refines that cluster per Drew's requirement of 2026-10-26 ~23:36 CDT and is the design input for the issues filed with it (listed at the end).
**Explicitly out of scope:** #1110 (the orchestration section being rejected — a separate agent is fixing it; everything here assumes the section flows again), the generic triage verbs of #1152 (acknowledge/snooze/escalate/answer on tier items), #1154 (status requests), #1156 (session actions). This addendum designs what those do not: **structured per-item options and survey questions, and orchestrator delegation modes with proposal and inability returns.**

## Grounding — what was actually read tonight

- **Current published item shape** (`~/workspace/goals/keep-the-four-repos-healthy/hidden_files/orchestration.json`, run `orch-2026-10-27T04:13:53Z`, read directly): `tiers.needsDrew` = **11 items**, `tiers.needsOrchestrator` = 9. Every needsDrew item carries exactly: `id`, `kind` (`task` | `decision` | `session` | `gap`), `title`, `source`, `reason`, `since`, `lastSeenAt`, and optionally `ref` (4/11), `horizon` (6/11), `blockedBy` (1/11). **No** needsDrew item tonight carries `question`, `recommendedAction`, `options`, or any delegation state (needsOrchestrator items do carry `recommendedAction`, 6/9). Examples that motivate this design: "Account connection campaign: Plaid (Citi) sign-in" (reason: only Drew can do it in his browser — an _access_ item), "code #812: public directory feed and presence counting design" (reason: owner design decision — a _decision_ item with real alternatives the item itself could enumerate).
- **Aggregator model** (`pm-stack/aggregator/src/model.ts` on origin/main): `TierItem` (id, kind, title, source, state?, reason, since, lastSeenAt, ref?, sessionId?, question?, horizon?, blockedBy?, recommendedAction?) and `OrchestrationSection` — every field this addendum adds is optional and additive on `TierItem`; nothing existing changes type.
- **Validator** (`src/feed.ts`): `isOrchestrationSection` checks section-level shape only; the per-goal gate comment documents the #949 lesson — strictness below the top level rejects whole sections. #1158's design already established the pattern this addendum follows: _optional new members are validated per-item in the live source, never in the section gate._
- **/attention rendering** (`src/render.ts`): `tierItemCard` renders badge(source) + badge(kind) + title + comment badge, then `Why you:` + reason, then optional question / blockedBy / recommendedAction / ref. There is no what-happens-if-nothing line, no action controls, no options, no survey. Session-derived needs-Drew entries (`TieredEntry` with `drewReason`, from `src/derive.ts` `buildAttention`) are a separate card type (`sessionCard`) — they get standard actions only (§4.4).
- **Write path** (coherence design, Cluster 3): `src/actions.ts` store mirroring `src/comments.ts` — `DashAction` envelope, `POST /api/actions`, `GET /api/actions.json`, `POST /api/actions/result`, clientToken dedupe, consume-once result transition, superseding actions for undo, orchestrator/keeper as the only consumers and the only writers of GitHub/Project/session state. **This addendum extends that envelope; it does not design a second one.**
- **Orchestration pass** (cron `orchestration-pass`, read-only tonight): hourly; publishes the whole section every run; keeps per-item bookkeeping in `orchestrator-state.json` (items by id: tier, reason, since, lastSeenAt, escalatedAt, actions[]); its publish step already asserts contract shape before writing (the provenance assertion added for #1110) — §3.3 extends that same assertion block. Its standing classification rule — "if you cannot state in one line why only Drew can resolve it, it is NOT needs-drew" — is what `whyDrew` (§1.1) makes structural.

---

## 1. Item model extension (publisher → section → aggregator)

All new fields are **optional** on `TierItem`. An item with none of them renders and behaves exactly as tonight (plus the §4 fallbacks). Field names are camelCase like the existing contract.

### 1.1 `whyDrew` — the blocker class

```ts
export type WhyDrew = "decision" | "missing-info" | "approval" | "access" | "policy";
whyDrew?: WhyDrew;
```

| Class          | Meaning                                                                                                 | Canonical example tonight        |
| -------------- | ------------------------------------------------------------------------------------------------------- | -------------------------------- |
| `decision`     | A product/design/scope choice no recorded context settles                                               | code #812 directory-feed design  |
| `missing-info` | The fleet lacks a fact only Drew holds; pairs with `questions[]` (§1.3)                                 | a session's pending `question`   |
| `approval`     | A drafted/proposed thing exists (or will, via first-pass) and needs Drew's yes                          | a first-pass proposal (§2)       |
| `access`       | A credential, sign-in, device tap, or physical step only Drew can perform                               | Plaid/Citi sign-in               |
| `policy`       | Spend beyond standing authorizations, account-level installs, anything a standing rule reserves to Drew | dezren39/nix GitHub App decision |

The pass MUST set `whyDrew` on every needsDrew item it publishes (self-check, §3.3). The aggregator derives a fallback when absent (older runs): `kind === "decision"` → `decision`; item has `question` → `missing-info`; otherwise `decision`. The class drives three things: the card's class badge (§4), which standard actions are offered (§1.2), and the auto-delegation hard exclusions (§2.3).

### 1.2 Standard actions — the canonical set (every needsDrew item, always)

These are Drew's words made canonical. They render on every needsDrew card regardless of what else the item carries:

| Action                    | Verb (§4.3) | Payload                                      | Notes                                                                                                                                     |
| ------------------------- | ----------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| **Answer**                | `answer`    | `text` (and/or survey/option payloads, §1.4) | Free-text reply; the pass delivers it like its own classifications                                                                        |
| **Delegate — first pass** | `delegate`  | `mode: "first-pass"`                         | §2.2                                                                                                                                      |
| **Delegate — auto**       | `delegate`  | `mode: "auto"`                               | §2.3; not offered when `whyDrew` is `access` or `policy` (the control renders disabled with the reason — the #1152/#1156 honesty pattern) |
| **Snooze until**          | `snooze`    | `until` (ISO)                                | From #1152; item leaves NEEDS DREW until the horizon, renders in the "snoozed by you" strip meanwhile                                     |
| **Dismiss with reason**   | `dismiss`   | `reason` (**required**)                      | §1.5                                                                                                                                      |
| **Escalate**              | `escalate`  | `reason?`                                    | On a needsDrew item: pin to top of tier, horizon forced to `today`, and flag in the pass's next delivery report                           |

(#1152's **acknowledge** remains as a non-resolving standard control beside these — it records "seen", it does not resolve, snooze, or delegate, and the card keeps rendering until one of the six above fires.)

`ifIgnored?: string` — one line, publisher-written: what happens if Drew does nothing ("Citi stays unlinked; the campaign goal stays at 15/17"). When absent, the card computes an honest fallback from `since`: "Stays in your queue and is re-published every run until it resolves or expires — in your queue since <date>." Never invented consequences.

`summary?: string` — optional one-line plain-language "what this is" for jargon-heavy titles; the card falls back to `title` alone when absent.

### 1.3 Custom `options[]` — the item's own suggested choices

```ts
export interface TierItemOption {
  id: string;            // stable within the item, slug form: /^[a-z0-9][a-z0-9-]{0,39}$/
  label: string;         // button text, ≤ 80 chars
  description?: string;  // one line: what this choice means
  effect?: string;       // one line: predicted effect if chosen ("closes #812 as option B; fleet implements")
}
options?: TierItemOption[];   // ≤ 6
```

The item knows its own alternatives; the pass writes them when it classifies (a `decision` item without options is a classification smell — the pass should usually be able to name at least the two real branches, even if one is "defer"). Choosing one is an action (§4.3), not a page-local state.

### 1.4 Survey `questions[]` — cover what is missing

```ts
export interface TierItemQuestion {
  id: string;                       // stable within the item, same slug rule
  prompt: string;                   // the question, ≤ 200 chars
  type: "single" | "multi" | "text" | "yesno";
  options?: { id: string; label: string }[];  // required for single/multi (≥ 2), ignored otherwise
  required?: boolean;               // default true
  covers: string;                   // the missing information this answer supplies, one line
}
questions?: TierItemQuestion[];     // ≤ 5
```

`covers` is mandatory in the contract (an unlabelled question is a quiz, not a survey): it renders beside the prompt as "Needed: <covers>", so Drew sees what each answer unblocks. A `missing-info` item SHOULD carry at least one question; the pass may also attach questions to other classes (e.g. a `decision` item asking one scoping question alongside its options).

### 1.5 Dismiss semantics

Dismissal is recorded by the consuming pass into `orchestrator-state.json` with the reason and the item's signal fingerprint (ref + reason + blockedBy set). **Resurface rule:** a dismissed item stays out of the tier unless a _new material fact_ appears (blocker changed, new comment/activity on `ref`, horizon passed); when it resurfaces, its `reason` must cite the new fact ("resurfaced: …"). A dismiss with no reason is rejected at the actions boundary (`validateNewAction`, §4.3) — "dismiss-with-reason" is the requirement, not a label.

---

## 2. Delegation modes — first-class per-item state

### 2.1 The state, and where it lives

```ts
export interface TierItemDelegation {
  mode: "drew" | "first-pass" | "auto";
  status: "idle" | "working" | "proposal-ready" | "unable" | "done";
  updatedAt: string;                 // ISO, last transition
  runId?: string;                    // run that last transitioned it
  proposal?: {
    summary: string;                 // what the first pass produced, one paragraph max
    detail?: string;                 // the draft itself (answer text, plan, proposed wording)
    proposedChanges?: string[];      // concrete changes approval would apply
    at: string;
  };
  unable?: {
    reason: string;                  // why the orchestrator cannot do this
    whatItTried: string[];           // ≥ 1 concrete attempt/route considered
    whatItNeeds: string;             // the specific thing that would unblock it
    at: string;
  };
}
delegation?: TierItemDelegation;
```

**DECISION — one item, one identity.** Delegation state is a field on the same tier item (same `id`, same card, same `tierItemObjectRef` for comments/actions). A proposal is _not_ a new item; an inability return is _not_ a new item. The pass is the publisher of `delegation` (it mirrors into the section the mode Drew set via actions, which it reads from the actions store each run and records in `orchestrator-state.json`). Before Drew has ever delegated an item, `delegation` is absent — equivalent to `{mode: "drew", status: "idle"}`.

### 2.2 `first-pass` — draft, return, approve

```
drew ──delegate{mode:"first-pass"}──▶ first-pass/working
first-pass/working ──pass completes a bounded first pass──▶ first-pass/proposal-ready  (+ proposal{})
first-pass/proposal-ready ──approve-proposal──▶ (pass applies the proposal next run) ──▶ item expires out, actions[] logs it
first-pass/proposal-ready ──reject-proposal{reason}──▶ drew/idle   (proposal cleared; reason kept in orchestrator-state)
first-pass/working ──pass cannot draft──▶ first-pass/unable  (+ unable{})
any state ──delegate{mode:"drew"} or dismiss/snooze──▶ back under Drew's direct control
```

Rules: the first pass is **bounded and non-applying** — it may read everything the item touches, draft an answer/plan/proposed change set, and stage nothing. It must not perform the Drew-reserved effect itself (no Project write of the decision, no session answer delivered, no spend). Approval is what converts the draft into the effect, applied by the pass on its next run as the single writer, with the run's `actions[]` entry naming the approving action id (`da-…`) — the same provenance pattern as #1154. Rejection requires a reason (same boundary rule as dismiss); the reason is recorded so the pass does not re-propose the same draft — if it re-delegates later, the new proposal must differ materially or say why it doesn't.

### 2.3 `auto` — proceed unless unable

```
drew ──delegate{mode:"auto"}──▶ auto/working
auto/working ──pass resolves end-to-end──▶ item expires out; actions[] logs what was done
auto/working ──pass hits a wall──▶ auto/unable  (+ unable{})  → item re-surfaces to Drew WITH the context
auto/unable ──Drew answers/dismisses/switches mode──▶ per that action
```

Rules:

- Auto means the pass treats the item as its own queue item with full standing authority (its normal ladder, caps, and measurement gate apply unchanged) — it is a _mode on this item_, not a new power.
- **The only return path is `unable{}`.** Silence, partial progress notes in a log Drew never sees, or quietly re-classifying the item to NEEDS ORCHESTRATOR are all contract violations: if the pass cannot finish it, the item stays in NEEDS DREW with `status: "unable"` and the structured reason/tried/needs triple. `whatItTried` must name at least one concrete route (a doc read, a recorded decision checked, a dispatch considered and why it failed) — "couldn't do it" without a tried-list fails the self-check (§3.3).
- **Hard exclusions:** items with `whyDrew: "access"` or `"policy"` can never enter auto — by definition the orchestrator lacks what they need (Drew's browser, Drew's authority). A `delegate{mode:"auto"}` action on such an item is `rejected` via `/api/actions/result` with the note naming the class. The dashboard control is disabled for them up front (§1.2), so this is defense in depth, not the primary UX.
- **DECISION — RESOLVED BY DREW (2026-10-27 00:31 CDT, via the options widget in the dashboard-status chat): the standing toggle is ON.** Drew's selection: "Standing auto toggle ON for decision/info/approval items (access & policy always stay mine)". Effect, effective when #1290/#1292 land: new needsDrew items whose `whyDrew` is `decision`, `missing-info`, or `approval` default to `mode: "auto"` — the orchestrator proceeds unless it returns a structured inability (§2.3). `access` and `policy` items NEVER enter auto under any setting; they always stay Drew's. Drew can still switch any individual item to first-pass or manual on its card (per-item override beats the standing default), and can turn the standing toggle off the same way he turned it on — the toggle state is a dashboard preference action record (auditable, supersede-to-change, never a config file edit). The pass MAY still surface `recommendedAction` on items, but for eligible classes the default is now auto rather than a suggestion. This supersedes the earlier per-item-opt-in-only proposal in this section and in §6.

### 2.4 No silent rot while delegated

A delegated item remains published in NEEDS DREW every run (badge: mode + status + age), so it can never vanish into the fleet's internals. Stall rule, pass-side: **a delegated item must reach a terminal outcome — `proposal-ready`, `done` (expired out), or `unable` — within 2 runs (2 h) of entering `working`.** On the second run without an outcome, the pass MUST publish `unable{reason: "no terminal outcome within 2 runs", whatItTried: […], whatItNeeds: "a decision on scope or inputs"}` rather than re-publish `working` again; the publish self-check (§3.3) fails the run's own section if a `working` item older than 2 runs has no outcome, so the violation is caught by the publisher, not by Drew noticing. This is the delegation-specific instance of #1148's principle (dead inputs surface, never decay silently); #1148's watchdog items remain the mechanism for dead _inputs_ (feed, services), and no separate watchdog item is created for a stalled delegation — the item itself carries the signal, in the tier Drew already watches.

---

## 3. Publisher contract

### 3.1 Exact additions to the section

`OrchestrationSection` itself is unchanged (version stays 1 — all additions are optional per-item fields, the #1158 precedent). On `TierItem`, all optional: `whyDrew`, `summary`, `ifIgnored`, `options[]`, `questions[]`, `delegation`. Caps: ≤ 6 options, ≤ 5 questions, string caps as in §1 (label ≤ 80, prompt ≤ 200, covers/effect/description one line each). The pass publishes them on needsOrchestrator items too when useful (an item Drew later escalates keeps its structure), but the contract requirement (§3.3) binds needsDrew.

### 3.2 Validation — per-item, always

`isOrchestrationSection` does **not** change and does **not** look at any new field. In the live source, where tier items are consumed (`src/sources/live.ts` `orchestration()`, the same seam #1158 designs for `nextUp`), each item passes a per-item sanitizer `sanitizeTierItemExtensions(item)`:

- `whyDrew` not in the enum → field dropped (fallback derivation applies).
- `options`: non-array → dropped. Each option kept only if `id` matches the slug rule and `label` is a non-empty string; duplicate ids within the item → later ones dropped. Result capped at 6.
- `questions`: non-array → dropped. Each question kept only if `id` slug-valid, `prompt` non-empty, `type` in the enum, `covers` non-empty; `single`/`multi` additionally require ≥ 2 valid options. Capped at 5.
- `delegation`: validated **as a unit** — `mode`/`status` in enums and `updatedAt` a string, plus `proposal` present iff status is `proposal-ready` and `unable` present iff status is `unable` (each with its required string fields) — any failure drops the _whole_ `delegation` field (item renders as plain `drew` mode), never the item.
- A malformed item extension can therefore degrade that item's card to tonight's rendering. It can never reject the item, the tier, or the section. (The #949 lesson, and the exact failure #1110/#1140 exist to never repeat at section level.)

### 3.3 Orchestration-pass self-check additions

Extending the publish step's existing assertion block (the same place the provenance-shape assertion lives — assertions run against the exact object about to be written, and a failed assertion stops the publish, it does not publish-and-hope):

1. Every needsDrew item has `whyDrew` in the enum.
2. Every `options[]`: ids unique within the item, count ≤ 6, every option has a label.
3. Every `questions[]`: count ≤ 5, every question has non-empty `covers`; `single`/`multi` questions have ≥ 2 options.
4. No item with `whyDrew` `access` or `policy` has `delegation.mode === "auto"`.
5. Every item with `delegation.status === "working"` has `updatedAt` within the last 2 runs, **or** is transitioned to `unable` in this publish (§2.4).
6. Every `proposal-ready` item has a `proposal` with non-empty `summary`; every `unable` item has `unable.reason`, ≥ 1 `whatItTried` entry, and non-empty `whatItNeeds`.
7. Dismissed items (per `orchestrator-state.json`) are absent from the tiers unless the resurface rule (§1.5) fired, in which case `reason` starts "resurfaced:".

---

## 4. Dashboard rendering & the actions mapping

### 4.1 The legible needs-Drew card (`tierItemCard`, extended)

Top to bottom, single column, mobile-first (the aggregator's existing card CSS; action controls ≥ 44 px touch targets, wrapping rows, no hover-only affordances):

1. **Header row:** source badge · kind badge · **whyDrew class badge** (labelled in words: "Decision", "Missing info", "Approval", "Only you can", "Policy") · age ("in your queue since <date>" from `since`) · comment badge · `ref` link.
2. **What:** `title`, then `summary` when present.
3. **Why you:** `reason` (existing).
4. **If nothing:** `ifIgnored`, or the §1.2 computed fallback.
5. **Question strip:** existing `question` rendering, when present.
6. **Your options (from the item):** one button per `options[]` entry — label on the button, `description` under it, `effect` as the muted "→ <effect>" line. Each button is its own form (§4.3).
7. **Survey:** when `questions[]` present, one inline form for the whole set: prompt, "Needed: <covers>" muted line, control per type (`single` → radios, `multi` → checkboxes, `yesno` → two radios, `text` → textarea). Required questions marked; one submit: **Send answers**.
8. **Proposal box** (only when `delegation.status === "proposal-ready"`): "First pass by the orchestrator — <runId>, <age>", `proposal.summary`, `proposedChanges` as a list, `detail` in a `<details>`, and **Approve** / **Reject** (reject reveals a required reason field).
9. **Unable box** (only when `status === "unable"`): "The orchestrator can't do this — <reason>", "Tried:" list, "Needs:" line. The standard action row below it is the way out.
10. **Standard action row (§1.2):** Answer (reveals a free-text field — always available, survey or not), Delegate ▾ (two buttons: "First pass — I'll approve" / "Auto — unless it can't"), Snooze (horizon select: tonight / tomorrow / next week / pick date), Dismiss (reveals required reason field), Escalate. Controls the item's class forbids (auto on access/policy) render **disabled with the reason beside them**, never hidden.
11. **Delegation strip** (whenever `delegation` exists and status ≠ idle): mode + status badge, "delegated <age> · last update <age>", and the pending-action badges the coherence design already specifies ("⏳ first pass requested 12m ago") until the pass's result lands.

Pending and applied states for every control come from the actions store exactly as the coherence design specifies (badges from `GET /api/actions.json`, loaded like `commentsStore.counts()`); the card joins section item ↔ actions by `tierItemObjectRef`.

### 4.2 Forms and script

One new form class `.aform` beside the comment form's `.cform`, handled by an `ACTION_SCRIPT` that follows `COMMENT_SCRIPT`'s exact pattern (delegated submit handler, `fetch` POST to `/api/actions`, `location.reload()` on success, inline `.err` text on failure, hidden `clientToken` per rendered form). No framework, no new client dependency — consistent with the app's rules.

### 4.3 Verb mapping into the DashAction envelope

The coherence design's `DashAction.kind` for tier items is `"triage"`. This addendum adds these verbs to `DashActionVerb` (kind `"triage"` unless noted) and their payloads:

| Verb               | Payload                                                                             | Boundary validation (`validateNewAction`)                                                                                                                                                           | Consumer behavior (orchestration pass)                                                                                                                                                                                       |
| ------------------ | ----------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `answer`           | `text?`, `optionId?`, `answers?: {questionId, value}[]` — at least one of the three | `optionId` must name an option on the item as published; `answers` ids must be among the item's published question ids (checked against the current section at POST time — the aggregator holds it) | Delivers the content: option choice recorded as the decision + its `effect` executed/routed; answers fill the item's missing info (recorded in state, cited in the next publish); plain `text` delivered like #1152's answer |
| `delegate`         | `mode: "first-pass" \| "auto" \| "drew"` (`drew` = take it back)                    | mode in enum                                                                                                                                                                                        | Sets the item's delegation mode in state; executes per §2 on this and later runs                                                                                                                                             |
| `approve-proposal` | —                                                                                   | item must be `proposal-ready` in the current section                                                                                                                                                | Applies `proposal.proposedChanges` as single writer; logs with the action id                                                                                                                                                 |
| `reject-proposal`  | `reason` (**required**)                                                             | non-empty                                                                                                                                                                                           | Clears proposal, mode → `drew`, reason recorded (§2.2)                                                                                                                                                                       |
| `snooze`           | `until`                                                                             | valid ISO, future                                                                                                                                                                                   | Per #1152                                                                                                                                                                                                                    |
| `dismiss`          | `reason` (**required**)                                                             | non-empty                                                                                                                                                                                           | Per §1.5 (state record + resurface rule)                                                                                                                                                                                     |
| `escalate`         | `reason?`                                                                           | —                                                                                                                                                                                                   | Per §1.2                                                                                                                                                                                                                     |

Idempotency, consume-once results, supersede-to-undo, and the three-layer audit trail are inherited from the coherence design unchanged — a `delegate` followed by a second `delegate` supersedes the first; an `approve-proposal` result POSTed twice applies once.

### 4.4 Session-derived items

Cards from `sessionCard` (live sessions tiered needs-Drew) get the standard action row and Answer (which routes through #1156's session answer) — but no `options[]`/`questions[]`/delegation, because those are publisher-authored on section items. If the pass wants a session item to carry a survey, it publishes it as a section tier item (the mechanism `buildAttention` already uses to consume section items into session entries by `sessionId`).

---

## 5. Acceptance tests (per piece)

**Publisher contract (pass-side harness + aggregator fixtures):**

- A section whose needsDrew items carry every new field round-trips `isOrchestrationSection` unchanged (fields are invisible to the gate).
- Self-check: a fixture item with `whyDrew: "access"` and `delegation.mode: "auto"` fails the publish assertions; a `working` item 3 runs old without outcome fails; an `unable` with empty `whatItTried` fails.

**Aggregator sanitizer (`test/orchestration.test.ts` style):**

- Item with a duplicate option id → duplicate dropped, item and section intact; question with 1 option → question dropped, item intact; `delegation` missing `unable` while `status: "unable"` → whole `delegation` dropped, item renders as drew-mode; counts: a section containing one fully-malformed-extension item still serves all 11-style items.
- Fallback: item without `whyDrew`, `kind: "decision"` → class badge renders "Decision".

**Actions boundary (`test/actions.test.ts`, with #1152's store tests):**

- `dismiss` without `reason` → 400; `reject-proposal` without `reason` → 400; `answer` with an `optionId` not on the published item → 400; `delegate{mode:"auto"}` on an `access`-class item → accepted at the store (authorship is honest), then **rejected by the consumer** with the class named in `/api/actions/result` — and a render test asserts the dashboard disables that control up front so the rejection path is defense-in-depth only.
- Double-submit of one `clientToken` → one record; double result POST → one application.

**Rendering (`test/render.test.ts` / `test/slice2.test.ts` style):**

- Fixture needsDrew item with options + questions renders: class badge, If-nothing line, one button per option with its effect line, survey form with "Needed:" per question, standard row with all six actions.
- `proposal-ready` fixture renders the proposal box with Approve/Reject; `unable` fixture renders reason/tried/needs; a delegated item renders exactly **once** in the tier (no duplicate card) across two consecutive fixture sections.
- Access-class fixture renders Delegate-auto disabled with its reason text.

**Pass behavior (orchestration-pass run evidence, per its own run-log discipline):**

- First real first-pass cycle: an item delegated from the dashboard returns `proposal-ready` with a proposal; approval applies it and the item expires out with the action id in `actions[]`.
- First real auto cycle: an item resolves and expires out, **or** returns `unable` with a tried-list — a run that leaves it `working` past the 2-run bound fails §3.3 assertion 5.

## 6. Sequencing (within coherence Cluster 3 / slice E)

1. **Publisher contract** (fields + self-check) — independent; fields are optional, so the dashboard ignores them until step 3. Can land before the actions store.
2. **Verbs in the actions store** — with #1152's slice (same store, same PR family); `dismiss`/`delegate`/`approve-proposal`/`reject-proposal` join the verb enum from the start so #1152 doesn't ship a store that needs re-shaping.
3. **/attention UI** — card extension + `.aform` script; renders options/questions/delegation the moment the publisher emits them.
4. **Delegation execution in the pass** — mode consumption, first-pass drafting, auto execution, inability returns, stall rule.
5. **Proposal review handling** — the proposal-ready state join (actions ↔ section) and its edge cases (proposal for an item that expired, approval racing a new publish) — separable because steps 3–4 work with proposals produced _and_ consumed in the simple path; this step hardens the seams.

## 7. Filed with this design

- (a) Publisher contract: [#1290](https://github.com/developing-today/code/issues/1290) — needsDrew item contract: whyDrew class, structured options[], survey questions[], delegation state.
- (b) /attention action UI: [#1291](https://github.com/developing-today/code/issues/1291) — standard actions, item-suggested options, inline survey, delegation controls.
- (c) Delegation execution: [#1292](https://github.com/developing-today/code/issues/1292) — first-pass proposals and auto-unless-unable with structured inability returns.
- (d) Proposal review state handling: [#1293](https://github.com/developing-today/code/issues/1293) — proposal identity, supersede/expiry races, cross-surface counts.

Each references #1152 and #1184 and this doc.

## 8. Decisions index

1. **One item, one identity** — proposal and inability are fields on the tier item, never new items (§2.1).
2. **Auto opt-in (PROPOSED, wants Drew's confirmation)** — per-item explicit opt-in; pass may suggest, never set; a standing default toggle is designed but off until Drew turns it on; `access`/`policy` classes are hard-excluded under any setting (§2.3).
3. **Chosen options ride the `answer` verb** (`payload.optionId`) — one content verb for text, choices, and survey answers keeps consumer dispatch simple (§4.3).
4. **Dismiss requires a reason and carries a resurface rule** — new material fact or it stays out (§1.5).
5. **Delegation stall = `unable` within 2 runs, enforced by the publisher's own self-check** — no separate watchdog item for delegations (§2.4).
6. **All new fields optional, validated per-item** — the section gate never learns about them (§3.2).

## Decision update — 2026-10-27 01:05 CDT (naming + escalation semantics)

Drew: "please continue and handle all updates, Needs User user/escalated and so on." Resolved:

- Visible wording is **Needs user** (the intake tier), never "Needs Drew". The honesty/freshness slice is applying this across the aggregator's user-visible copy; internal published fields (`needsDrew`) are legacy until a versioned contract change, which must be fail-soft on both shapes (the #1110 pattern).
- The tier's semantics mirror the fleet's 2026-10-25 label family: **user/review = intake** — the orchestrator/Muse reviews and handles what it can first; only a genuinely-user item becomes **user/escalated**, surfaced to Drew with Muse's read attached. "Needs user" therefore names a queue the system works, not a pile waiting on Drew personally.
- New contract identifiers in this program are role-based user naming; no new Drew-named fields/verbs. The standing auto-toggle decision in §2.3 is unchanged: decision/missing-info/approval default to auto; access/policy never auto.
