# D2 — OpenChamber categories, Auto, self-switching and sub-session guidance

Program: code#9399 (pool & effort system). Owner: D2 drafter (workhorse head, detached).
Binding inputs: `../main.md` (AD1–AD10), `../brief.md` §1–§2, `../brief-decisions-supplement.md` §0, §6, §9, `../questions.md`,
`../baseline/v1-decisions-auto.md` (the OpenChamber audit), `../baseline/v1-pools-efforts.md` (pools, effort, variants),
`../research/R1-routing-effort-sota.md` §6, `../research/R2-decisions-sota.md` §0, §5.5.
`../inventory/` is empty (no statement map exists yet), so statement ids below are **brief § ids**
(`B1.1` = brief §1 item 1, `S6.1` = supplement §6 item 1). Re-key them when the inventory lands.

Source reads for this design were made read-only: `~/git/openchamber-fork` and `~/git/opencode` at `origin/HEAD`
(via `git show`), `~/.config/openchamber/{routing,settings,classification}.json`, and `~/.local/share/opencode/opencode.db`
(`mode=ro`). Nothing was written, committed or pushed.

---

## 0. Summary (read this first)

1. **Auto works in code and is unproven live.** The path is complete (classify → cache hold → stickiness → variant check →
   switch session). No Auto decision is persisted anywhere: `route_decision` has 2.43 M rows but none from OpenChamber,
   `auto-sessions.json` is absent, and no session has `openchamber` as its model. The first deliverable is one logged send (§5).
2. **Category variants are probably silently dropped on every pool category today.** `withKnownVariant` (runtime.js ~378–397)
   drops a variant the catalog does not list for that model. Route pools advertise variants from `routeVariants()`, which is
   empty whenever any leaf is pinned (`oc:packages/core/src/config/plugin/model-routes.ts:1269`, V1 `b.3`). Every pool that
   contains a pinned leaf therefore advertises no variant, so `high` on `speed`, `workhorse`, `specialist` and `summit`, and
   `max` on `workhorse`, are probably dropped. Source-derived, not yet observed live. Fixing it is a D1 dependency (§2.4).
3. **Proposed mapping: 5 built-ins kept, 2 categories added (`docs`, `ops`), `review-light` offered but off by default,
   `decision` rejected.** Each category gets a pool plus an effort tier in the D1 vocabulary (`low|medium|high`, with
   `xhigh|max|ultra|fast` → `high`). Medium is the pool as written (AD2).
4. **Auto default: possible without new code.** Settings `defaultModel` is set to `openchamber/auto`. Two traps: the project
   entry `/home/user/git/code` overrides the global default (its `defaultModel` is `opencode-route/workhorse`), and a stale Auto
   falls through the cascade to the next default, so there is no loop (`useConfigStore.ts` `dropStaleAutoSelection`).
5. **Self-switching, V2:** a bounded tool `pool_switch` that **suggests** or **launches a subsession** on an allow-listed pool
   or category, and records every call as a decision (AD7/AD10). **AD10 needs one correction:** the fork already has a
   `session_switch_model` tool (`oc:packages/core/src/tool/plugin/opencode.ts:179-195`), absent from this session's Code Mode
   catalog. "Cannot switch in place" is verified for this catalog only. The fork's version has no allow-list for top-level sessions
   (`oc:packages/core/src/session/session.ts:137-160`, governance runs only when `parentID` is set). V3 needs a fork guard.
6. **Provider order:** code order is typesafe → custom → openrouter → vercel → zen-key, applied at config-resolve time only, with
   zen-promo never a fallback. Proposed: **user pick first, then free → paid, with Cloudflare Clef excluded** (dead by ruling,
   #1651). Decision for Drew, §6.

---

## 1. Scope

| Theme (inventory) | Brief / supplement sections | Statements (brief §-ids) |
|---|---|---|
| T1 OpenChamber categories, pools, Auto | brief §1 | B1.1 set pools for all categories; B1.2 "clef jev opencodego free then paid then cloudflare"; B1.3 ensure Auto works; B1.4 customisations |
| T2 self-switching, sub-sessions, guidance, Auto default | brief §2 | B2.1 suggest/switch own pool (category or route pools only); B2.2 parallel sub-sessions with limited context, when not to use Auto, when to change own model; B2.3 sub-sessions make sub-sessions; B2.4 default Auto, fallback workhorse / general / build, small options scout / speed |
| Supplement touch-points | supplement §0 (Jev = decision model), §6 (effort as a tie-break lean for Auto), §9 (decision record) | S0, S6.1, S6.2, S9 (recording only) |
| Binding decisions | main AD5 (Auto's classifier is a decision), AD10 (self-switching bounded and recorded) | — |

Not in scope: the decision pool and its leg order (D5), the decision record store and feedback gauges (D6), effort-tier
resolution and overlays (D1), the stats store (D3). D2 names the interfaces it needs from them (§2.4, §4.4).

---

## 2. V1 — what exists today

### 2.1 Categories, mapping and gating

Built-in categories and their Jev criteria text: `oc-ui:packages/web/server/lib/routing/defaults.js:132-158` (sent verbatim as
the choice criteria, `jev.js` `buildRoutingRequest`). Shipped set: `trivial`, `research`, `implement`, `review`, `hard`.
`routing.json` stores only deviations, and this machine's file (`~/.config/openchamber/routing.json`) stores all five:

| id | model (machine) | variant (machine) | agent | enabled | Pool exists in `opencode.jsonc`? |
|---|---|---|---|---|---|
| trivial | `opencode-route/speed` | `high` | build | yes | yes (`speed`, 3544) |
| research | `opencode-route/workhorse` | `max` | build | yes | yes (`workhorse`, 3176) |
| implement | `opencode-route/workhorse` | `high` | build | yes | yes |
| review | `opencode-route/specialist` | `high` | build | yes | yes (`specialist`, 3445) |
| hard | `opencode-route/summit` | `high` | build | yes | yes (`summit`, 3490) |

No user categories. Global flags: `enabled: true`, `minConfidence: 0.6` (defaults.js), `preserveCache: true`,
`safetyNet.enabled: false`, `stickiness` default (`defaults.js` `DEFAULT_STICKINESS`: enter 2, exit 3, strong 0.9).
Fallback: `opencode-route/workhorse` (model only; agent comes from the composer).

Gating: `autoReady = jevAvailable && config.enabled && fallback && enabledCategories >= 2` (`runtime.js` `describe`, ~232).
On this machine all terms are true, provided a source is usable. The active source is `zen-promo` (`classification.json`),
and `ZEN_JEV_PROMOTION_ACTIVE = true` is hard-coded (`defaults.js:49`).

### 2.2 The send path (per baseline, re-read this pass)

`resolveAutoSelection` (`runtime.js` ~411–460): `describe()` → `readHistory` (3 turns, 600/300/300 chars, `history.js`) →
`resolveAccess()` → `jev.ask(buildRoutingRequest(...))` (4000 ms, `JEV_TIMEOUT_MS`) → `decideRouting` (`routed`,
`unknown-category`, `low-confidence`) → `cacheHold` (`cache.js` `cacheHoldCategory`, PROMPT_CACHE_TTL 5 min, warm window from
OpenCode `warming`) → `chooseWithStickiness` (`stickiness.js`) → `withKnownVariant` → broadcast `openchamber:routing.decision`
→ `applySessionSelection` (`session.switchModel` + `switchAgent`). Any throw sets `reason: 'error'` and keeps the session on its
current model, or on the fallback if it has none (`chooseWithStickiness` with `noSignal`).

Verified in source this pass: the cache hold only holds a **same-agent** category and only downwards; moving up is never held
(`cache.js` header, `cacheHoldCategory`).

### 2.3 Recording today

Decision: in memory, per session (`useRoutingStore.ts:26,102-103`), plus an event. Nothing persisted. Jev calls from
OpenChamber are **not** in the OpenCode ledger (`route_attempt` has only pool and façade attempts). `routing-review`/`route_usage`
cannot split pools by category (baseline `v1-decisions-auto.md` "Recording and logging today").

### 2.4 Variant drop (source-derived, not yet live)

`withKnownVariant` drops a variant the catalog does not list for `providerID/modelID`, and keeps it when the model is unknown
or the catalog cannot be read. For `opencode-route/*` models the catalog lists the pool with `variants: routeVariants(expanded)`
(`model-routes.ts:333`). `routeVariants` returns `[]` when any leaf is pinned (`:1269`). Every class route in V1 `a.1` reaches a
pinned leaf, so the advertised variant list is **empty**, and every category variant on a pool category is dropped (`speed:high`,
`workhorse:high`, `workhorse:max`, `specialist:high`, `summit:high`). The machine's project default `/home/user/git/code` also
sets `defaultVariant: low`, which is dropped the same way.

Consequence: **today an Auto category's variant does not reach the model**, so effort never reached the pools. This is the single
most important fact for effort, and it is inferred from source. The first live check (slice `D2-S1`) confirms or refutes it.

### 2.5 Defaults and where they live

| Key | Value (machine) | Where | Read by |
|---|---|---|---|
| OpenCode `model` (no model chosen) | `opencode-route/workhorse` | `.opencode/opencode.jsonc:254` | OpenCode; non-OpenChamber surfaces. VS Code never offers Auto (routing DOCUMENTATION.md) |
| OpenChamber `defaultModel` (global) | `opencode-route/workhorse`, variant unset | `~/.config/openchamber/settings.json` (keys `defaultModel`, `smallModelOverride: opencode-route/speed`) | `DefaultsSettings.tsx` `handleModelChange` → `updateDesktopSettings` |
| OpenChamber `defaultModel` (per project) | `/home/user/git/code`: `opencode-route/workhorse`, variant `low`; `/home/user`: none | `settings.json` `projects[]` | `resolveProviderModelSelection` (project wins over global, `useConfigStore.ts` ~3600) |
| preferences | `defaultModel: opencode-route/workhorse` | `~/.config/openchamber/preferences.json` (baseline) | UI defaults |

Cascade for a session with no model (`useConfigStore.ts` ~320–345 and the `dropStaleAutoSelection` comment at ~3600): project
default → global default → agent's preferred → `opencode config.model` → last pick → first. A **stale** Auto (routing not ready)
is skipped by `isStaleAutoSelection` (`useConfigStore.ts:290`), so the cascade does not stall on Auto.

### 2.6 Fallback behaviour and small-model slots

Jev unreachable / timeout / bad JSON: session keeps its model, or uses `fallback` (workhorse). There is **no** retry and **no**
call-time source failover (`classifier.js:106-108`, FALLBACK_ORDER applies only when the picked source is not usable from config).
`small_model` (`opencode.jsonc:260`) is `opencode-route/speed`, deliberately: compaction and summaries must read a lot
(comment 255–259). `explore` → `scout` (1754), `classifier` agent → `scout` (1757), `classifier-complex` → `workhorse` (1633)
are chat subagents, unrelated to Jev.

### 2.7 Subagent depth and guidance that exists

`.opencode/instructions/subagents.md` already states: subagent-first, background mode, `experimental.subagent_depth` default 5, the
detached `question` rule, model switching not possible in session. `AGENTS.md` is **7,493 of 8,000 bytes** (checked this pass), so
new guidance cannot go in the index. It goes in the topic file (§7).

---

## 3. Options considered

### 3.1 Category set and pool mapping

| Option | Description | Pros | Cons | Prior art |
|---|---|---|---|---|
| **A. Keep 5, retune pools and tiers** (recommended as the base) | Same 5 categories; fix pools and tiers; no new criteria text | Smallest change; Jev criteria stable; the 5 already have pools | Gaps: no docs or ops category, so those requests land in `implement` or `hard` | OpenRouter Pareto bands (R1 §2.11): few bands, tuned by data |
| B. Keep 5 + `docs` + `ops` (recommended addition) | Add two categories with their own criteria | Cheap writing and infra work stop hitting `hard`; clearer cost | Two more criteria for Jev to separate; more confusion at edges | RouteLLM-style binary → multi-class; R1 §2.5 |
| C. Keep 5 + `docs` + `ops` + `review-light` + `classify` | Seven+ categories | Finest cost control | Jev confusion grows; `classify` recurses (the classifier classifies with a classifier) | Not Pareto-tuned; no measured basis |
| D. Pools keyed by effort only (one category per tier) | low / medium / high categories | Maps straight to D1 tiers | Loses the task-type signal Jev is good at; over-routes to cheap tiers | Supplement §6 "low/medium/high is a tie-break, not a category" |
| E. Single category, effort chosen by Jev | One category, Jev picks effort | Simple UI | Jev is not asked about effort; criteria fight; no measured basis | — |

Decision: **A + B** (7 categories, 2 new), with `review-light` as an option (§4.2, default off), `classify` rejected.

### 3.2 Effort on a category

| Option | Mechanism | Pros | Cons |
|---|---|---|---|
| **A. Category `variant` as the tier (today's field)** | `variant ∈ {low, medium, high}`; resolved by D1 when the pool advertises tiers | No new key; the same field `model#variant` already means effort (AD1) | Dropped today on pools (§2.4). Needs D1 to make pools advertise tiers |
| B. Pin the tier in the model string (`opencode-route/speed#low`) | Route pin | Works with today's fork | A pin makes the pool advertise no variants (`model-routes.ts:1269`), so the category is a pin again. This is the problem D1 removes |
| C. New `effort` key beside `variant` | Separate key, mapped at routing time | Clear naming | Two fields for one thing; migration cost. Only worth it if `variant` must keep a per-model meaning |
| D. Tier chosen per send by Jev | Jev answers effort too | Adapts per request | Second question, no measured basis, adds latency; supplement §6 says effort is a lean, not a per-send answer |

Decision: **A**, the category `variant` holds the D1 tier. Validation accepts `low|medium|high` and maps `xhigh|max|ultra|fast`
to `high` (AD1). Until D1 ships, the only safe value is **unset**, which means the pool's medium (AD2). That is a change from the
current machine config (see §4.1).

### 3.3 Auto's classifier source (V2 vs V3)

| Option | Description | Pros | Cons |
|---|---|---|---|
| **A. Keep Jev through OpenChamber's own sources** (V2, recommended) | Current code paths; add decision recording and call-time failover | Works now; no fork work; the classifier stays cheap | Not the D5 decision pool; the default answer is the fallback, not a pooled decision |
| B. Route Auto's classifier through the D5 decision pool | `decision` key, `/v1/systemone` façade, leg order free→plan→PAYG | Single decision machinery (AD5); recorded per AD7 | Needs D5 and the façade; the decider LLM protocol does not exist (`baseline` "LLM adaptation: not implemented") |
| C. Use a chat LLM as the classifier (Luna @100k #medium, last resort) | Prompted JSON via `/v1/chat/completions` | Works on any provider | Slower; the protocol has no validation loop yet (supplement §4) |
| D. Local classifier (`route-local-model.js`) | Local model answers | No quota | Quality unmeasured; hardware-dependent |

Decision: **A in V2**, **B in V3** once D5 exists (AD5 says the classifier is a decision; V2 keeps its existing call and records it).

### 3.4 Auto as the default

| Option | Description | Pros | Cons |
|---|---|---|---|
| **A. Auto as the OpenChamber default (global + the `/home/user/git/code` project entry)** (recommended) | Settings UI sets `openchamber/auto`; the project entry is cleared or set to Auto | Brief B2.4; a stale Auto falls through; the UI already hides Auto when not ready | Adds a classifier call (≤4 s) per send; the default's behaviour depends on the classifier |
| B. Auto as default for new sessions only | `defaultModel` only; existing sessions keep their model | Lower risk | Existing project entry overrides global, so the code project would still not default to Auto |
| C. Keep workhorse as default; Auto opt-in | Status quo | No latency | Contradicts B2.4 |
| D. Auto default in OpenCode `model` too | `opencode.jsonc` `model: openchamber/auto` | One place | Breaks VS Code, CLI and fleet automation, which cannot use Auto (routing DOCUMENTATION.md) |

Decision: **A**, applied in OpenChamber settings only. `opencode.jsonc` `model` stays `opencode-route/workhorse` (option D rejected).

### 3.5 Self-switching

| Option | Mechanism | Pros | Cons | Prior art |
|---|---|---|---|---|
| **A. Guidance only** | Text tells the model when to change; no tool | Free; zero new code | Unbounded; nothing recorded; the model can pick anything | AGENTS-style rules |
| **B. Suggest tool** `pool_switch(mode: "suggest")` | Validated, recorded recommendation; no switch | Bounded and recorded; safe | The human or parent must act | MCP advisory tools; R1 §6.3 sticky advice |
| **C. Subsession launch** `pool_switch(mode: "subsession")` (recommended V2) | Validated, recorded; launches a subagent with `model: opencode-route/<pool>[#effort]` | Works today (the `subagent` tool takes `model`); the child gets a fresh context, so no cache is lost in the parent; depth cap applies | Child cost and latency; the parent must read its result |
| D. In-place `session_switch_model` (fork tool, exists) | The session switches its own model next turn | One session, no child | Not in this catalog; top-level sessions unchecked (`session.ts:137-160`); prompt cache lost (the tool's own text says so); needs a fork allow-list | Anthropic prompt-cache economics, R1 §6.2 |
| E. Parent-side switch by OpenChamber on a model's request | UI applies the switch after a `pool_switch` suggestion | Human in the loop | UI work; same cache cost as D | — |

Decision: **B + C in V2**; **D with a fork allow-list and the cache gate in V3** (§4.5). **E** is a later UI option.

### 3.6 Provider order for the classifier

| Option | Order | Pros | Cons |
|---|---|---|---|
| A. Keep code order: typesafe → custom → openrouter → vercel → zen-key, promo never a fallback | Config-time only | Matches code; no behaviour change | Not the brief's free → paid → Cloudflare; no call-time failover; with the promo the machine's only usable source, a failure has no fallback |
| **B. User pick first, then free → paid, Cloudflare excluded** (recommended) | pick → zen-promo (if promo active) → zen-key → typesafe → openrouter → vercel | Honours the user's pick; free first in failover; the brief's order; a failed call moves on at call time | Reverses the "promo never a fallback" rule (§6 Q6); adds up to 3 extra calls, so the budget must be capped |
| C. Pure free → paid → Cloudflare | zen-promo → zen-key → typesafe → openrouter → vercel → custom | Simple | Overrides the user's pick on every send; the promo is the default for any user |
| D. Add Cloudflare Clef now | … → custom (façade) | Brief's wording | Dead by ruling (#1651, 2026-11-04); the façade probe returned 401 |

Decision: **B**, with a total budget across the chain of **5 s** (one attempt is 4 s today). Cloudflare Clef stays out of the chain
until Drew re-rules (§6 Q4).

### 3.7 Failure path

| Option | Behaviour | Pros | Cons |
|---|---|---|---|
| A. Keep today's: keep current model, else workhorse | — | Works | No call-time source failover; no record of which source failed |
| **B. Call-time failover along the chain (§3.6 B), then fallback model + build agent** (recommended) | Next source inside the 5 s budget; then `fallback` = `opencode-route/workhorse` with `build`; then recorded `error` | Auto keeps working through one provider outage | More code; needs the decision record to show the source used |
| C. Fallback to `small` or `speed` on failure | Cheapest on failure | Cheap | `speed` is the small-model slot: compaction needs real capability (`opencode.jsonc:255-259`). Fallback should not be the cheapest tier |
| D. Fallback to `general` agent | — | — | `general` is a subagent type, not a composer agent; the brief lists it with `build` but it is not a sending agent. Default: `build` (§6 Q5) |

Decision: **B**. The fallback model is `opencode-route/workhorse` (brief B2.4). The brief's "small, scout, speed as small-model
options": `speed` stays the `small_model` slot; `scout` is the `docs` pool and the `explore` pool; `small` is the `trivial` pool.
All three are listed as options, not one default.

### 3.8 Recording the decision (V2)

| Option | Store | Pros | Cons |
|---|---|---|---|
| **A. Append-only JSONL, one line per decision, in the OpenChamber data dir** (recommended V2) | `auto-decisions.jsonl` next to `routing.json` | No new DB access from OpenChamber; trivially greppable; append-only (AD7) | Not joinable to `route_attempt` without a key (it carries `decision_id` and the session id) |
| B. Row in `opencode.db` `route_decision` | Fork write path | Joins the ledger | OpenChamber has no DB write path; the table is written by OpenCode (`route_decision` 2.43 M rows, `candidates`, `chosen`, `reason`) |
| C. `decision_record` table (D6) | New table (AD7) | The target shape | Owned by D6; not ready |

Decision: **A in V2**, with the schema chosen to map onto AD7 `decision_record` so D6 can import it (§4.4).

---

## 4. V2 design

### 4.1 The category mapping (the proposal)

Each row: category, criteria source, pool, tier, and why. Tier = D1 vocabulary. "medium" means the pool as written (AD2).

| Category | Pool (`opencode-route/…`) | Tier (`variant`) | Today | Why |
|---|---|---|---|---|
| `trivial` | `small` (limited → speed chain) | `low` | `speed`, `high` | Mechanical work. `small` is the light tier with per-member caps (`limited` 3597); the class fallback `small → speed` already exists, so escalation is free. `high` is dropped today anyway |
| `research` | `workhorse` | `medium` | `workhorse`, `max` | Read-only explanation. Workhorse is cost-first (brief §7), and `max` is dropped today. Alternative: `scout` (§3.1) |
| `implement` | `workhorse` | `medium` | `workhorse`, `high` | Pool as written. The `high` variant is dropped today; a real `high` comes from D1 overlays |
| `review` | `specialist` | `high` | `specialist`, `high` | Keep. Review needs judgement, and specialist is the sink (route-fallbacks.md) |
| `hard` | `summit` | `high` | `summit`, `high` | Keep. Summit already carries high-tier members (`fable-plan#medium`, `opus-plan#high`) |
| **`docs`** (new) | `scout` | `medium` | — | Writing prose and docs. Scout is the cheap-diversity pool (Beam, DeepSeek #low, Haiku #low); its class fallback is `workhorse` |
| **`ops`** (new) | `workhorse` | `high` | — | Infra, CI, git operations and deploys: real blast radius, not design. Destructive operations stay in `hard` by criteria text |
| `review-light` (option, default **off**) | `speed` | `medium` | — | Quick review of a small diff. Only worth adding if `review` is over-used on small diffs (measure first, D3) |
| `decision` / `classify` (**rejected**) | — | — | — | Not a chat category. Decisions belong to D5. A classifier that classifies classifier work recurses (§3.1 C) |

Criteria text for the two new categories (the Jev `criteria` string; written to match the style of `defaults.js:132-158`, about
the reasoning demanded, not reply length):

- `docs`: "Writing or editing prose that explains or documents: README sections, changelog entries, comments that explain why, doc
  pages and release notes. Not for code changes, design decisions, or docs that describe a system's behaviour being changed in the same
  request (that is implement or hard)."
- `ops`: "Operating infrastructure or the repository itself: CI configuration, deploys, git branch and merge operations, dependency
  upgrades, environment and secrets wiring. Not for application logic. Destructive or irreversible operations (force-push, data
  deletion, production changes) are hard, not ops."

Weak points: `ops` and `hard` overlap on destructive ops; `docs` and `implement` overlap on code comments. The criteria say which
wins (destructive → hard; code change → implement). Revisit with real decisions (D6 outcomes).

Machine config after approval (for `~/.config/openchamber/routing.json`, applied through Settings, not by hand-editing):

```jsonc
// ~/.config/openchamber/routing.json (only deviations from built-ins are stored)
{
  "version": 1,
  "enabled": true,
  "minConfidence": 0.6,
  "preserveCache": true,
  "fallback": { "model": { "providerID": "opencode-route", "modelID": "workhorse" }, "agent": "build" },
  "categories": {
    "trivial":   { "builtin": true, "model": { "providerID": "opencode-route", "modelID": "small" },      "variant": "low", "agent": "build" },
    "research":  { "builtin": true, "model": { "providerID": "opencode-route", "modelID": "workhorse" }, "variant": "medium", "agent": "build" },
    "implement": { "builtin": true, "model": { "providerID": "opencode-route", "modelID": "workhorse" }, "variant": "medium", "agent": "build" },
    "review":    { "builtin": true, "model": { "providerID": "opencode-route", "modelID": "specialist" }, "variant": "high", "agent": "build" },
    "hard":      { "builtin": true, "model": { "providerID": "opencode-route", "modelID": "summit" },     "variant": "high", "agent": "build" },
    "docs": {
      "builtin": false,
      "name": "Docs",
      "description": "Writing or editing prose that explains or documents: ...",
      "model": { "providerID": "opencode-route", "modelID": "scout" }, "variant": "medium", "agent": "build"
    },
    "ops": {
      "builtin": false,
      "name": "Ops",
      "description": "Operating infrastructure or the repository itself: ...",
      "model": { "providerID": "opencode-route", "modelID": "workhorse" }, "variant": "high", "agent": "build"
    }
  }
}
```

Note the `fallback.agent` key: today the fallback carries only a model; the code takes the agent from the composer (`chooseSelection`).
Adding `agent` to the fallback is an OpenChamber change (slice `D2-S4`).

Where `ops`' `high` and `research`' `medium` are not yet real: until D1 advertises tiers, `variant` is dropped and the send runs the
pool at its default. D2-S3 makes that drop **recorded**, so a dropped tier is never silent.

### 4.2 Effort on categories: the rule that keeps it honest

- Category `variant` ∈ `{low, medium, high}`; `xhigh|max|ultra|fast` → `high` at save time (AD1).
- The OpenChamber save path validates the value against the pool's **advertised** tiers once D1 exposes them. Before that, a
  variant the pool does not advertise is saved with a warning and recorded at send time as `variant-dropped`.
- Silent drops are removed: `withKnownVariant` writes `decision.variantDropped = {requested, pool}` (the decision record carries it).

### 4.3 Auto end to end: the verification procedure

Preconditions (all checked by the procedure, none assumed):

1. `GET` the OpenChamber routing state (the `describe()` payload from `routes.js`): `autoReady: true`, `jevAvailable: true`,
   `classifier.effective` non-null, `config.categories` with ≥ 2 enabled.
2. Settings → Routing → **Test** (`testClassifier`, `buildProbeRequest`): the answer is `blue` or `table` with a confidence number.
3. A scratch session in `/home/user/git/code` (or a worktree) with the model set to `openchamber/auto`.

Send 1 (trivial): "rename the local variable `tmp` to `buffer` in this one function", expected category `trivial`.
Send 2 (hard): "the cache sometimes serves stale data after a deploy; the cause is unknown; plan the fix across the fetch and invalidation modules", expected `hard`.
Send 3 (held): a follow-up "thanks, that works", expected `held` by stickiness, not a switch.

For each send, check the **decision line** (after `D2-S3`; before that, the in-memory event `openchamber:routing.decision`):
`category` ∈ enabled set, `model` = that category's pool (or fallback when `reason ≠ routed`), `reason` ∈ enum, `ms` ≤ 4000 + 1000, `error` absent or
recorded. Then check the OpenCode side: `route_usage` for the session shows the chosen pool, not the fallback.

Oracle: for every decision line, `category ∈ enabled ∪ {null}` and `model == pool(category)` when `reason == "routed"`, else `model == fallback`
(`D2-S2` tests this on stubs; `D2-S1` runs it live once).

### 4.4 Recording (V2): the decision line

Append-only, one JSON object per line, in `~/.config/openchamber/auto-decisions.jsonl` (the OpenChamber data dir; file mode 0600
like the other stores). Fields map onto AD7 `decision_record`:

```ts
type AutoDecisionLine = {
  decision_id: string;              // "dec_" + ULID, minted per Auto send or self-switch
  kind: "auto-send" | "self-switch";
  at: string;                       // ISO 8601 UTC
  session_id: string;
  directory: string;
  prompt_version: string;           // sha256 of ROUTING_INSTRUCTIONS + criteria, first 12 hex, plus "2026-11-13"-style date
  categories_enabled: string[];     // ids
  classifier: { source: string; model: string; attempt: number; ms: number };
  answer: { category: string | null; confidence: number };
  reason: "routed" | "unknown-category" | "low-confidence" | "error" | "not-ready" | "no-signal";
  held: boolean;
  held_reason?: string;             // stickiness reason
  cache_hold?: { requested: string };
  chosen: { pool: string; variant: string | null; agent: string | null };
  variant_dropped?: { requested: string; pool: string };
  error?: string;
  // self-switch only:
  requested_by?: { parent_session: string; reason: string };
  to?: string; effort?: "low" | "medium" | "high"; mode?: "suggest" | "subsession";
  validator?: "ok" | "not-allowed" | "self-target" | "rate-limited" | "depth-exhausted";
  child_session_id?: string;
};
```

Invariants (checked by `D2-S6`): `reason === "routed"` ⇒ `answer.category ∈ categories_enabled`; `chosen.pool` equals the
category's pool or the fallback; `decision_id` unique; the file only grows (no rewrite, AD7).

### 4.5 Self-switching: the tool (V2)

Name: `pool_switch`, exposed as a code-mode tool in the `routing` namespace (`tools.routing.pool_switch`) by a small script in the
repo. Signature:

```ts
pool_switch({
  to: string,                              // "category:<id>" | "pool:<id>"; pool names are opencode-route/<id>
  effort?: "low" | "medium" | "high",      // aliases xhigh|max|ultra|fast are normalised to "high"
  mode: "suggest" | "subsession",
  reason: string,                          // required, 1..200 chars
  prompt?: string                          // required for mode "subsession", the child's task
}): Promise<{ decision_id: string; validator: string; launched?: { child_session_id: string } }>
```

Validation (the allow-list), in order; the first failure returns `validator` and records the line anyway:

```text
1. to ∈ allow-list  →  allow-list = { category pools of enabled categories } ∪ { opencode-route/<id> for every key of experimental.model_routes }
                       else "not-allowed"
2. to ≠ openchamber/auto and to is not a raw provider model  →  else "not-allowed"
3. to ≠ the caller's own pool/category                        →  else "self-target"
4. effort (if given) normalised with the alias table; unknown → "not-allowed"
5. rate: at most 3 subsession launches per parent session per hour  →  else "rate-limited"
6. depth: remaining subagent depth ≥ 1 (experimental.subagent_depth, default 5)  →  else "depth-exhausted"
7. mode = "suggest": return; mode = "subsession": launch via the subagent tool with
   model = "opencode-route/<id>" + "#" + effort, agent = "build" (or the category agent), background = true, prompt = prompt
```

Pseudocode for launch:

```text
function pool_switch(args):
  rec = newDecision(kind="self-switch", session=ctx.sessionID, to=args.to, effort=norm(args.effort), mode=args.mode, reason=args.reason)
  v = validate(args, allowList(), ctx)            # steps 1–6 above
  rec.validator = v
  if v != "ok": append(rec); return {decision_id: rec.id, validator: v}
  if args.mode == "suggest": append(rec); return {decision_id: rec.id, validator: "ok"}
  child = subagent.launch({agent: "build", model: poolRef(args.to, rec.effort), prompt: args.prompt, background: true})
  rec.child_session_id = child.sessionID
  append(rec)
  return {decision_id: rec.id, validator: "ok", launched: {child_session_id: child.sessionID}}
```

Why launch and not switch: the parent keeps its warm cache and its context; the child starts clean on the pool. The depth cap and
the subagent tool's own `model` argument already exist, so V2 adds no fork code.

Failure modes: a validator refusal is a result, not an error (the model can read it and continue). A child that errors reports
its error verbatim (`subagents.md`). A recording failure does not block the call; the call returns `validator: "ok"` with
`recorded: false`, and the failure is logged.

### 4.6 Sub-session guidance (see Appendix A for the text)

Guidance is written into `.opencode/instructions/subagents.md` (topic file, not `AGENTS.md`, which is at 7,493/8,000 bytes). One
index line may go in `AGENTS.md` under "Subagent-first workflow" if budget allows (§7, slice `D2-S9`).

### 4.7 Auto as default: exact changes

| Where | Change | How |
|---|---|---|
| `~/.config/openchamber/settings.json` `defaultModel` | `openchamber/auto` | Settings → Defaults → Default model (`DefaultsSettings.tsx` `handleModelChange`, which also clears `defaultVariant`) |
| `settings.json` `projects[]` entry `/home/user/git/code` | clear `defaultModel`/`defaultVariant`, or set Auto | Settings → Project → Defaults. Without this the project entry wins over the global default (`useConfigStore.ts` ~3600) |
| `.opencode/opencode.jsonc` `model` | **no change** (`opencode-route/workhorse`) | Non-OpenChamber surfaces cannot use Auto |
| `small_model` | **no change** (`opencode-route/speed`) | Compaction needs capability (`opencode.jsonc:255-259`); `small` is a category pool, not the summary model |

Safety: a stale Auto is skipped (`isStaleAutoSelection`), so if Jev and every source are down and routing reports not ready, the
cascade lands on the project or global default. If both are Auto, it lands on `opencode config.model` (workhorse). `D2-S8` tests that
no path lands on Auto when routing is not ready.

### 4.8 Failure path (V2)

```text
send with openchamber/auto
  autoReady false            -> chooseSelection(config, null): fallback (workhorse, build), reason "not-ready"
  autoReady true:
    for source in chain (pick, then zen-promo if active, zen-key, typesafe, openrouter, vercel), total budget 5 s:
      try jev.ask(source)    -> ok: decideRouting(...), break
                              -> error/timeout/bad JSON: next source (attempt++)
    chain exhausted          -> reason "error", chooseWithStickiness(noSignal): keep model if one, else fallback
  every path               -> one AutoDecisionLine
```

The brief's failure order "workhorse / general / build" is the model `workhorse` with the agent `build`. `general` is listed in §3.7.

### 4.9 Code changes (V2, all in `developing-today-forks/openchamber` unless stated)

| File (fork) | Change |
|---|---|
| `packages/web/server/lib/routing/runtime.js` | Call-time chain (§4.8); `variant_dropped`; decision line write |
| `packages/web/server/lib/routing/classifier.js` | `FALLBACK_ORDER` becomes the call-time chain; the pick first; promo included when active |
| `packages/web/server/lib/routing/defaults.js` | Add `docs`, `ops` built-ins (criteria text §4.1); `fallback.agent` default `build` |
| `packages/web/server/lib/routing/store.js` | Read/write `auto-decisions.jsonl` (append, 0600), size gate |
| `packages/web/server/lib/routing/*.test.js` | Smoke and invariant tests (§5, §8) |
| `packages/ui` settings | Category `variant` validation against advertised tiers (after D1) |
| `code` repo: `bin/routing-pool-switch.mjs` (new) | `pool_switch` validator and launcher (§4.5), tested with fixtures |
| `code` repo: `bin/check-routing-categories.mjs` (new) | Guard `D2-S7` (§5); same family as `bin/check-opencode-route-rules.mjs` |

### 4.10 Interfaces D2 needs from other components

- **D1:** advertised tiers on `opencode-route/*` (so `variant` survives `withKnownVariant`); the rung table for `low|medium|high`.
- **D5:** the decision pool key (`decision`) and the systemone façade, for the V3 classifier (§4.11).
- **D6:** import of `auto-decisions.jsonl` into `decision_record`, and outcome links (PR merged, reverted) per decision id.

### 4.11 V3 and V4 layers

**V3 — measured and automatic.** Entry gate: ≥ 14 days of `auto-decisions.jsonl` lines (AD7, main §0).

- **V3-a Calibrated category confidence.** Use the decision records to measure, per category, how often `routed` answers were kept
  (not held, not reverted). Set `minConfidence` per category from the measured curve, not 0.6. Guard: fails if a category's kept-rate drops below its floor.
- **V3-b Classifier in the decision pool.** Replace the direct Jev call with the D5 decision pool (AD5). The decision record gets
  `leg` and `route_chain`. Gate: D5 shipped.
- **V3-c In-place switch with the cache gate.** Fork guard first (§4.12), then:

```text
function allowInPlaceSwitch(session, target, now):
  if session.hasRunningTool or session.turnOpen: return "wait-for-turn-boundary"
  idle = now - session.lastAnswerAt
  if idle >= PROMPT_CACHE_TTL (5 min) and not session.warmedByOpenChamber: return "cache-cold: switch"
  if target.door == session.door: return "same-door: switch"          # cache prefix survives within a door
  rebuildCost = session.contextTokens * target.inputPrice * target.cacheWriteMult
  remainingTurns = estimate(session)                                    # from the stats store (D3)
  perTurnGain = (session.currentPrice - target.price) * session.avgTurnTokens
  return perTurnGain * remainingTurns > rebuildCost ? "switch" : "hold"
```

Gate for V3-c: measured cache-hit rate after switches (research slice `D2-S15`) and the fork guard (`D2-S14`).

**V4 — learned and adaptive.** Bandit allocation over categories inside the trial's safe set (AD9); effort as a tie-break lean
(supplement §6) using the classifier's probability vector, which requires D5 probabilities. Gate: V3 metrics show the hand-tuned
mapping is the limiting factor.

### 4.12 The fork guard for in-place switching (V3 prerequisite)

`session_switch_model` (`oc:packages/core/src/tool/plugin/opencode.ts:179-195`) takes any `providerID/modelID`. Governance is applied only
when the session has a `parentID` or fork source (`oc:packages/core/src/session/session.ts:137-160`). A top-level session can therefore
switch to any model. The fork change: for self-switch calls (`sessionID` omitted or equal to the caller), enforce the same allow-list as
`pool_switch` (opencode-route pools and enabled categories only), and record the call. Filed as `D2-S14`, fork repo.

---

## 5. Smoke test: one Auto send, one logged decision

Runnable without a live provider (`D2-S2`): a unit test beside `packages/web/server/lib/routing/*.test.js`, using the
`createRoutingRuntime` injection points (`readSessionHistory`, `readConfigEntries`, `listCatalogModels`, a stub `jev`):

```text
given: routing.json (5 built-ins + docs + ops), a catalog listing opencode-route/* with variants [] (the V1 case)
given: stub jev answers { choice: "trivial", confidence: 0.92 }
when:  send with model openchamber/auto, request "rename tmp to buffer"
then:  the decision line has category "trivial", reason "routed", chosen.pool "small", variant_dropped {requested:"low", pool:"small"}
oracle (invariant): category ∈ enabled set; chosen.pool == pool(category); decision_id unique; one line appended
```

Live (`D2-S1`, manual, gated by `AUTO_SMOKE_LIVE=1`): the §4.3 procedure with three sends, checked against the real classifier source and
`route_usage`. Recorded in the slice's result comment, not committed.

---

## 6. Provider order: the decision to make

Proposal B (§3.6): pick → zen-promo (if active) → zen-key → typesafe → openrouter → vercel, total budget 5 s. Cloudflare Clef excluded.

Pros of changing from code order: the brief's free-first order; a failed source moves on at call time; the user's pick is still first.
Cons: the promo was deliberately never a fallback (comment `classifier.js:17-20`, "needs no credential, so only a pick sends there").
Under B it becomes a fallback. Dax approved OpenChamber's use of the free promo on terms that include a limited-time notice in the UI
(`defaults.js:28-40`), so B keeps the notice and must keep it.

---

## 7. Guidance placement and the AGENTS.md budget

- Topic file: `.opencode/instructions/subagents.md`, new section "Heavy parallel work and self-routing" (text in Appendix A).
- `AGENTS.md` (7,493 bytes): at most one index line (≤ 400 bytes), only if `scripts/research_workflow.py check` still passes.
- Skill: no new skill; the text is for every session, not one workflow.

---

## 8. Worked examples

### 8.1 A normal Auto send that routes

Session `ses_A` (Auto), `/home/user/git/code`, prompt "why does the workhorse route fall back to scout after a 60 s timeout? read only".

1. Routing state `autoReady: true`; source `zen-promo` (usable).
2. Jev answers `{choice: "research", confidence: 0.88}`.
3. `cacheHold`: last answer from `speed`/`small`-class `ses_A` was on `workhorse`, same agent, `research` rank > `trivial`: no hold (moving up is not held).
4. `chooseWithStickiness`: first prompt → initial, applied.
5. `withKnownVariant`: `opencode-route/workhorse` advertises no variants (pinned leaves), `medium` dropped → `variant_dropped {requested: "medium", pool: "workhorse"}`.
6. Line appended: `reason routed`, `chosen {pool: workhorse, variant: null, agent: build}`.
7. OpenCode `route_usage` for `ses_A` shows `workhorse` attempts.

### 8.2 Jev down, then a subsession on `summit`

`ses_B` (Auto), prompt "plan the migration across the billing and ledger modules".

1. Jev: source `zen-promo` times out at 4 s; the chain tries `zen-key` (not configured, skipped as not usable); chain exhausted at ~5 s.
2. Reason `error`; `ses_B` has no earlier model, so the fallback `workhorse` + `build` runs. Line appended with `error`.
3. In the reply, the model judges this is `hard`. It calls `tools.routing.pool_switch({to: "category:hard", effort: "high", mode: "subsession", reason: "multi-module migration", prompt: "…"})`.
4. Validator: `category:hard` → pool `summit` ∈ allow-list; not self; effort `high` normalised; rate ok (0 of 3); depth 4 remaining.
5. Child session launched on `opencode-route/summit#high`; decision line `validator: ok`, `child_session_id` set.
6. The parent reads the child's diff and reports; the parent stays on `workhorse`, so its cache is intact.

---

## 9. Risks, open questions and defaults

Each question has the default I chose. Nothing here calls the `question` tool (detached).

| # | Question | Default chosen | Why / what would change it |
|---|---|---|---|
| Q1 | Make Auto the default? | **Yes**, OpenChamber only (§3.4 A) | Brief B2.4. Revert = set `defaultModel` back to workhorse |
| Q2 | Add `docs` and `ops`? | **Yes** (§4.1). `review-light` **off**, `classify` rejected | Measure first (D3). Two additions keep Jev's criteria distinct |
| Q3 | Which provider order? | **Proposal B** (§3.6) | Drew's ruling |
| Q4 | Cloudflare Clef in the chain? | **No** | Dead by ruling (#1651, 2026-11-04). Re-open only on Drew's new ruling |
| Q5 | Fallback agent: `build` or `general`? | **`build`**. `general` is a subagent type, not a composer agent | Brief lists both. Confirm |
| Q6 | Promo as a fallback? | **Yes while active**, UI notice kept | Reverses the code's rule (`classifier.js:17-20`). Needs Drew's and Dax's agreement; see §6 |
| Q7 | Self-switch modes in V2? | **suggest + subsession** (§3.5 B, C). In-place is V3 | AD10 correction below |
| Q8 | Is `session_switch_model` exposed to sessions in the running build? | **Unknown**: absent from this session's catalog. Default: treat as not exposed | `D2-S10` checks a fresh session's catalog |
| Q9 | Does a route pool advertise tiers today? | **Probably not** (§2.4, source-derived) | `D2-S1` confirms with one live send and a `model.list` read |
| Q10 | Cross-component slice ids (`D1-S*`, `D5-S*`, `D6-S*`) | **Provisional**; reconcile when those designs land | Blocked-by only names D2 slices plus the ones marked "reconcile" in the JSON |
| Q11 | Jev and "clef jev" | Jev = the TypeSafe / Zen / OpenRouter / Vercel System One decision model (supplement §0 decoded; code confirms); Clef = Cloudflare façade, dead | Supplement §0 says Command Code; code does not (baseline Q1) |
| Q12 | Should Auto's effort be a lean (supplement §6)? | **Not in V2**: Jev returns one category and one confidence, no probability vector | V3-b / V4 with D5 probabilities |
| Q13 | Is 4 s per attempt the right budget? | **5 s total** across the chain | Measure with `D2-S1` (`ms` in each line) |

### 9.1 Challenges to main.md

- **AD10 wording.** "A session cannot switch its own model in place today (verified)" holds for this session's Code Mode catalog
  only. The fork's `origin/HEAD` has `session_switch_model`, governed for child sessions and unchecked for top-level ones. Proposed
  wording: "not exposed in this catalog; the fork has an ungoverned top-level switch that needs an allow-list before any use."
- **AD5 default answer.** For the Jev classifier the default answer is the fallback model (`workhorse`), not a decision leg. AD5's
  "Luna @100k last resort" applies to the V3 decision-pool classifier, not to Jev. Consistent, but not stated in AD5.
- **AD1 and the category variant.** AD1 says tiers resolve per (door, model). Categories set `variant` on pool refs, so the same field
  now carries a tier for pools and a native variant for models. The tier table (D1) must accept both, or categories pin a native
  variant by mistake.

---

## 10. Statement coverage

| Statement | Section | Slice(s) |
|---|---|---|
| B1.1 pools for all categories | §2.1, §4.1, §3.1 | D2-S5, D2-S12 |
| B1.2 provider order free → paid → Cloudflare | §3.6, §4.8, §6 | D2-S4, D2-S13 |
| B1.3 ensure Auto works | §2.2, §4.3, §5, §8 | D2-S1, D2-S2, D2-S3, D2-S6 |
| B1.4 customisations (new categories, effort, variants) | §3.2, §4.1, §4.2 | D2-S3, D2-S5, D2-S7 |
| B2.1 suggest/switch own pool (category or route pools) | §3.5, §4.5, §8.2 | D2-S10, D2-S11, D2-S7 |
| B2.2 parallel sub-sessions, limited context, when not to use Auto, when to change own model | §4.6, Appendix A | D2-S9 |
| B2.3 sub-sessions make sub-sessions | Appendix A, §4.5 (depth) | D2-S9, D2-S11 |
| B2.4 Auto default; fallback workhorse / build; small options | §3.4, §3.7, §4.7, §4.8 | D2-S4, D2-S8 |
| S0 Jev is the decision model | §9 Q11 | — (decision recorded) |
| S6.1 / S6.2 effort as lean; defaults change under Auto | §9 Q12 | D2-S17 (V3 seed) |
| S9 decision record | §4.4 | D2-S3, D2-S6, D2-S18 |
| AD5, AD10 | §9.1, §4.11 V3-b, §4.12 | D2-S14, D2-S17 |
| Q8 / Q9 facts (session catalog, variant drop) | §2.4, §9 | D2-S1, D2-S10 |

---

## Appendix A — Guidance text (for `.opencode/instructions/subagents.md`)

Draft, in the file's voice. Replace the section "Heavy parallel work and self-routing" at the end of the file.

```markdown
## Heavy parallel work and self-routing

**Fan out with small contexts.** When a task splits into independent reads or checks, launch one sub-session per piece
(background, one piece per child, a short prompt with the file paths, the question and the result shape you want). Keep each
child's context narrow: a child that reads the whole repository is slower and dearer than three children that each read what they need.
Read-only fan-out (search, explain, check) belongs on `scout` or `small`; writes belong on the pool the category names.

**Sub-sessions can make sub-sessions.** A child may launch its own children, up to the depth cap (default 5, the same
`experimental.subagent_depth` the parent uses). Split a child's job again only when the parts are independent; do not nest for
convenience. Count your remaining depth before you launch, and keep the last level for work that does not need to split.

**Auto is for the unknown, not the known.** Use Auto (`openchamber/auto`) for a request whose difficulty you cannot tell from the
first line. Do not use Auto:
- for a sub-session with a task already scoped: launch it on its pool (`subagent` with `model: opencode-route/<pool>`), which avoids the
  classifier call and keeps the cache warm;
- for a long run on one category: pin the model; Auto's hold and stickiness are for changing topics, not for steady work;
- for a destructive operation (force-push, data deletion, production change): name the model and the `hard` pool yourself;
- when the classifier is down: each send waits up to 5 seconds before it routes, so a pinned model is faster.

**Change your own model only for a real change of kind of work.** A session cannot switch its own model in place in this build, and a
switch loses the prompt cache. Use `pool_switch` instead:
- `mode: "suggest"` when you think the work has moved to another category and want the record of your reason;
- `mode: "subsession"` with a `prompt` when one step needs another pool (for example a multi-module design inside a `workhorse` task
  goes to `hard`); read its result and continue.
Do not change pools to chase a cheaper or faster answer for steady work; that costs the cache and the reasoning state.
A `validator` refusal is an answer: read it, and do the work on your own pool or on the one it names.

**Every routing call is recorded.** `pool_switch` and each Auto send write a decision line with your session id and reason. Give a real
reason; the reason is what the review reads.
```

---

## Appendix B — Slice index

See `../slices/D2-slices.json`. V2 (`D2-S1`–`D2-S13`): live check and smoke test (S1, S2), decision record (S3), failover (S4),
categories (S5, S7, S12), guards (S6, S7), Auto default (S8), guidance (S9), catalog check (S10), `pool_switch` (S11), provider-order docs (S13).
V3 (`D2-S14`–`D2-S18`): fork guard (S14), cache research (S15), in-place switch (S16), classifier and effort research (S17), calibration (S18).
