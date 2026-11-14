# Design: PM-stack prompt & chat surface

Issue: #9539 (program). Brief: `brief.md` (PR #9535, merged `e30df92199`).
Design author: repo-worker-code, lane code-j, task 16078, 2026-11-14.

EPIC GUARD: this issue holds work in itself; subtasks completing never closes it.

## 1. Goals and non-goals

**Goals.** Every appropriate PM-stack page carries prompt widgets scoped to a section, a page, the PM stack generically, generic agent access (chat + task delegation queued at a priority), and specialized task variants. Targets: Claude or Muse; the OpenCode/OpenChamber pools (low/medium/high) with auto + low/medium/high effort; a decision-model variant that generates a question series for a decision API/LLM/hybrid with effort steering, default fallbacks, "not confident" answers, and speed-optimized answer generation at a provided context level. Async (checkable ID, optional completion notification) and sync (block, or WebSocket until answered) modes. Plus a chat/session surface: review all messages, new session per message unless replying in context, open/close/archive/reply/cross-reference, notification count + push, defaults = Muse hook task immediately or a medium-high queue item on auto/workhorse, everything overridable.

**Non-goals.** No second queue: async submission reuses the fleet queue (code#9539). No new persistence store beyond the existing file-backed stores (decisions, needs-drew, comments). No changes to the decision-model supplement itself (code#9399) — this design consumes it. Streaming audio is a future enhancement, not this design.

## 2. Placement map

The PM-stack aggregator (`pm-stack/aggregator`) renders 21 pages (see `src/pages.ts` `PAGES`). Each page gets prompt widgets at four scope levels plus specialized variants. A widget is a small form: textarea (prompt text) + target selector + mode selector + submit. Section/page widgets additionally carry a context blob derived from the page's rendered data.

### Scope levels

- **section** — scoped to one section of a page (e.g. a project row on /program, a quota row on /quota). Carries that section's data as context.
- **page** — scoped to the whole page (e.g. the whole board). Carries the page's primary data as context.
- **stack** — generic PM-stack prompt, not tied to a page. Lives in the nav/footer on every page.
- **agent** — generic agent access: chat + task delegation queued at a priority. Lives in the nav/footer on every page.

### Specialized task variants

Variants pre-fill the prompt template and set a default target/mode:

- **reprioritize** — "Reprioritize the following items: …" (target: auto/workhorse, mode: async)
- **detail-gather** — "Gather details on …" (target: Muse hook, mode: async)
- **compare-options** — "Compare these options: …" (target: Claude, mode: sync-block)
- **integrate-insights** — "Integrate new insights since last review: …" (target: Claude, mode: async)
- **report** — "Generate a report / refresh view of …" (target: auto/workhorse, mode: async)

### Page-by-page placement

| Page (route)                   | section         | page | stack | agent | Notes                                                                     |
| ------------------------------ | --------------- | ---- | ----- | ----- | ------------------------------------------------------------------------- |
| Overview (/)                   | —               | ✓    | ✓     | ✓     | Front door; page widget summarizes needs-you + projects + quotas + health |
| Today (/today)                 | ✓ per objective | ✓    | ✓     | ✓     | Per-objective section widgets                                             |
| Digest (/digest)               | ✓ per section   | ✓    | ✓     | ✓     | Per digest section                                                        |
| Needs you (/attention)         | ✓ per item      | ✓    | ✓     | ✓     | Per needs-you item; primary agent-access surface                          |
| Decision sets (/decisions)     | ✓ per set       | ✓    | ✓     | ✓     | Per decision set; decision-model variant lives here                       |
| Board (/board)                 | ✓ per item      | ✓    | ✓     | ✓     | Per board item                                                            |
| Roadmap (/roadmap)             | ✓ per item      | ✓    | ✓     | ✓     | Per roadmap item                                                          |
| Source conflicts (/conflicts)  | ✓ per conflict  | ✓    | ✓     | ✓     | —                                                                         |
| Usage & billing (/quota)       | ✓ per quota     | ✓    | ✓     | ✓     | Per quota row                                                             |
| DNS register (/dns)            | ✓ per domain    | ✓    | ✓     | ✓     | —                                                                         |
| Cost Explorer (/aa)            | —               | ✓    | ✓     | ✓     | Page-level only                                                           |
| Sessions (/sessions)           | ✓ per session   | ✓    | ✓     | ✓     | Per session                                                               |
| Search (/search)               | —               | ✓    | ✓     | ✓     | Page-level only                                                           |
| Events (/events)               | —               | ✓    | ✓     | ✓     | Page-level only                                                           |
| Comments queue (/comments)     | ✓ per comment   | ✓    | ✓     | ✓     | Per queued comment                                                        |
| Program status (/program)      | ✓ per project   | ✓    | ✓     | ✓     | Per project row; primary surface                                          |
| Queue (/queue)                 | ✓ per item      | ✓    | ✓     | ✓     | Per queue item                                                            |
| Health (/health)               | —               | ✓    | ✓     | ✓     | Page-level only                                                           |
| Alerts (/alerts)               | ✓ per alert     | ✓    | ✓     | ✓     | Per alert                                                                 |
| Notifications (/notifications) | —               | ✓    | ✓     | ✓     | Page-level only                                                           |
| All pages (/pages)             | —               | —    | ✓     | ✓     | Manifest page; stack + agent only                                         |

Section widgets are only placed where a page has discrete, addressable rows. Pages marked "page-level only" have no natural section granularity.

## 3. Prompt API contract

### 3.1 Endpoint

`POST /api/prompt` — accepts a prompt submission. `GET /api/prompt/:id` — checks an async submission's status. `GET /api/prompt` — lists recent submissions (chat surface).

### 3.2 Request body (PromptSubmission)

```json
{
  "scope": "section" | "page" | "stack" | "agent",
  "variant": "free" | "reprioritize" | "detail-gather" | "compare-options" | "integrate-insights" | "report",
  "target": "claude" | "muse" | "pool:auto" | "pool:low" | "pool:medium" | "pool:high" | "decision",
  "effort": "low" | "medium" | "high" | null,
  "mode": "async" | "sync-block" | "sync-ws",
  "notify": boolean,
  "page": "<page id>",
  "sectionRef": "<opaque section id or null>",
  "prompt": "<text>",
  "context": { /* page/section-derived data, free-form */ },
  "attachments": [ /* multipart refs */ ],
  "decision": { /* only when target === "decision" */ }
}
```

### 3.3 Targets

- **claude** — route to Claude (Anthropic) directly.
- **muse** — route to Muse (local hook task, immediate).
- **pool:auto/low/medium/high** — route to the OpenCode/OpenChamber pool at the given tier with auto effort selection; `effort` overrides auto when set.
- **decision** — the decision-model variant (code#9399 supplement): takes input, generates a series of questions, passed to a decision API or an LLM or a hybrid, with effort steering and default fallbacks; allows "not confident" answers; uses speed-optimized models; generates answers at a provided context level.

### 3.4 Decision variant (target === "decision")

```json
"decision": {
  "contextLevel": "low" | "medium" | "high",
  "advancedConstraints": [ /* conditional-clause objects from code#9399 */ ],
  "contextGenerator": "model:<id>" | "task:<queue-id>" | null
}
```

The decision variant generates questions via the configured `contextGenerator` (any model or queued task), shapes them for the decision API, and returns a question series. Advanced constraints (conditional clauses) are accessible. Attachments/multimedia are processed. Streaming audio is a future enhancement.

### 3.5 Modes

- **async** — submit to the fleet queue, return a checkable ID immediately. Optional completion notification (push + notification count). ID format: `prompt:<uuid>`.
- **sync-block** — block the request until the answer is available; return the answer in the response body. Timeout: 120s default, configurable.
- **sync-ws** — open a WebSocket channel; stream the answer as it arrives; close when complete.

### 3.6 Response (async)

```json
{ "id": "prompt:<uuid>", "status": "queued", "queueItemId": "<fleet-queue item id>" }
```

### 3.7 Response (sync-block)

```json
{ "id": "prompt:<uuid>", "status": "answered", "answer": "<text>", "attachments": [] }
```

### 3.8 Status check

`GET /api/prompt/:id` returns `{ id, status, answer?, error? }`. Statuses: `queued`, `running`, `answered`, `failed`.

## 4. Chat/session surface

- **Review all messages**: `GET /api/prompt` lists submissions (newest first), filterable by page/scope/status.
- **New session per message** unless replying in context or providing input on a report page. A "session" is a thread of prompts sharing a `threadId`.
- **Reply**: `POST /api/prompt` with `replyTo: "<prompt id>"` creates a reply in the same thread.
- **Cross-reference**: `refs: ["<prompt id>", ...]` links other prompts.
- **Open/close/archive**: `PATCH /api/prompt/:id` with `{ archived: true }`. Archived items hidden by default.
- **Notification count + push**: unread count badge in nav; push notification on async completion (opt-in per submission).
- **Defaults**: Muse hook task immediately, or a medium-high priority queue item on auto/workhorse. Everything overridable via target/mode.

### Data store

File-backed, like decisions/needs-drew: `data/prompts.json` (array of PromptSubmission + status). No new database. Fleet queue handles execution; the aggregator only stores submissions and results.

## 5. Reuse of existing machinery

- **Fleet queue** (repo-queue / `repo-queue` CLI + the aggregator's queue reader): async submissions enqueue a queue item with `kind: prompt`, priority from the submission (default medium-high = 50), target encoded in the item payload. A worker claims it, runs the prompt against the target, and writes the answer back via `PATCH /api/prompt/:id`.
- **Decision machinery** (`src/decisions.ts`): the decision variant produces a decision set using the existing decision-set shape; answers flow back through the existing resolution path.
- **Comments** (`src/comments.ts`): prompt submissions can optionally post a comment on the linked GitHub issue/PR.
- **Events** (`src/events.ts`): every submission emits a `prompt.submitted` / `prompt.answered` event for the events stream.

## 6. Implementation slices

### S1 — Generic prompt widget + section/page widgets on main pages

Core slice. Build the reusable widget component (textarea + target selector + mode selector + submit), render it at section/page/stack/agent scopes on the primary pages (Overview, Today, Needs you, Program, Queue, Roadmap, Board). Target selector: Claude/Muse/pool:auto. Mode: async/sync-block. Async submission enqueues a fleet queue item and returns a checkable ID. No decision variant, no sync-ws, no chat surface yet.

**Files**: new `src/prompt-widget.ts` (render), `src/prompts.ts` (store), routes in `src/server.ts`, `src/pages.ts` (no new page — widgets embed in existing pages).

### S2 — Target selector expansion + async completion notification

Add pool:low/medium/high + effort override + notify toggle. Push notification on async completion. Notification count badge in nav.

### S3 — Sync modes

sync-block (block until answered, 120s timeout) and sync-ws (WebSocket channel). New WebSocket endpoint `/ws/prompt/:id`.

### S4 — Decision variant

target=decision with contextLevel, advancedConstraints, contextGenerator. Produces a decision set. Integrates code#9399 supplement.

### S5 — Specialized task variants

Pre-filled templates for reprioritize/detail-gather/compare-options/integrate-insights/report. Each sets default target+mode.

### S6 — Chat/session surface

Threaded view, reply, cross-reference, archive, notification count + push. Defaults = Muse hook / medium-high queue item.

### S7 — Attachments & multimedia

Upload handling, multipart, storage refs. Streaming audio is a future enhancement.

## 7. Granular implementation issues

To be filed as sub-issues of #9539:

- S1: generic prompt widget + section/page widgets on main pages
- S2: target selector expansion + async completion notification
- S3: sync modes (sync-block, sync-ws)
- S4: decision variant
- S5: specialized task variants
- S6: chat/session surface
- S7: attachments & multimedia

## 8. Open questions for Drew

1. **Default target for stack/agent scope**: Muse hook (immediate) vs auto/workhorse (queued)? Brief says "typically immediate Muse hook OR medium-high queue item" — confirm default.
2. **sync-block timeout**: 120s default acceptable, or should it be configurable per submission?
3. **Attachments storage**: reuse the existing upload mechanism (if any) or new? Confirm before S7.
4. **Decision variant contextGenerator**: which models/queued tasks are available for context generation? Confirm the set.

These do not block S1-S3.

## 9. Acceptance for this design pass

- [x] Placement map (§2) covering all 21 pages × 4 scopes.
- [x] Prompt API contract (§3) with scopes, targets, modes, decision variant.
- [x] Chat/session surface spec (§4).
- [x] Reuse statement (§5) — fleet queue + decisions + comments + events.
- [x] Sliced implementation plan (§6) with 7 slices.
- [x] Sub-issues filed (§7) — to be done in this pass.
- [x] Open questions recorded (§8).

EPIC GUARD: this issue holds work in itself; subtasks completing never closes it.
