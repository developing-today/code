# V1 audit: the decision path (Jev / Clef / System One) and OpenChamber Auto

Status: V1 baseline, read-only audit. Program: code#9399, design-mill pool & effort system.
Date: 2026-11-13 (CST). Author: subagent on `design/pool-effort-system`, route opencode-route/workhorse.

Sources, all read-only:

- Fork opencode: `~/git/opencode`, `origin/HEAD` after `git fetch -q origin`. Cited as `oc:<path>:<line>`.
- Fork OpenChamber: `~/git/openchamber-fork`, `origin/HEAD`. Cited as `oc-ui:<path>:<line>`.
- Live data: `~/.config/openchamber/*` (read only), `.opencode/opencode.jsonc` in this worktree, `route_usage`, one live probe per endpoint, `gh issue view` / `gh search`.

Headline findings (details below):

1. **Jev is not a Command Code model in code.** Nothing in the fork links `systemone`, `evaluation` or Jev to `_commandcode`. Jev is TypeSafe's `jev-latest` (`api.typesafe.ai/v1/systemone`), OpenCode Zen's `jev-1.13-free` / `jev-1.13`, OpenRouter's `typesafe/jev-1.13`, or Vercel's `typesafe-ai/jev`. Clef is Cloudflare Workers AI, served by the fork's `/v1/systemone` façade. The supplement's §0 premise is not supported by the code.
2. **Jev is reachable today, through OpenCode Zen's free promo.** A live probe to `https://opencode.ai/zen/v1/systemone` with `jev-1.13-free` returned 200 with a typed choice answer. OpenChamber's active source is `zen-promo` (`~/.config/openchamber/classification.json`). The Clef façade path is dead by ruling (#1651, won't-do, Drew 2026-11-04). Its endpoint answers 401 from this shell.
3. **Auto is configured and should be ready.** `routing.json` has `enabled: true`, a fallback of `opencode-route/workhorse`, and 5 enabled categories, each with a pool. The code computes `autoReady` at `oc-ui:packages/web/server/lib/routing/runtime.js:232`. The live data was not run through that function, so "should be ready" is inferred, not observed.
4. **There are no decision pools, no decision default, no retries and no decision IDs.** The fork has one endpoint per provider and no route, pool or fallback for decisions. Invalid output means one attempt, then a fallback to the session's model. Decisions are kept only in memory, latest per session.
5. **The brief's provider order (free → paid → Cloudflare) is not what the code does.** OpenChamber's fallback order is typesafe → custom → openrouter → vercel → zen-key, and it never falls back to zen-promo. Cloudflare is not an OpenChamber source at all.

---

## Schema

### Wire request (what OpenChamber and the fork send)

Defined in the fork's ai package, `oc:packages/ai/src/experimental/system-one.ts:48-55`, which is the wire schema for the outbound body:

```
Request = { model: string, state: EvaluationInput, questions: Record<string, Question>, ...any }
EvaluationInput = string | JsonObject | Json[]                     (evaluation.ts:13)
Question = Choice | Score | Noul                                    (system-one.ts:35-47)
  Choice: { type:"choice", instructions, criteria: Record<string, criterion>, nonempty, <=255 keys }   (evaluation.ts:23-27, system-one.ts:37)
  Score:  { type:"score",  instructions, criteria: criterion[], length 2..10 }                       (evaluation.ts:30-35, system-one.ts:42)
  Noul:   { type:"noul",   instructions, criteria?: {true?, false?} }                                   (system-one.ts:25-34)
criterion = string | JsonObject | Json[] | null                       (evaluation.ts:16)
```

- The public SDK question type is `boolean`. On the wire it is sent as `noul` (`system-one.ts:115`).
- OpenChamber's client sends `{ ...request, model }` (`oc-ui:packages/web/server/lib/routing/jev.js:145`).
- The server validates the body against `{model, state, questions}` and forwards only `{state, questions}` upstream (`oc:packages/core/src/systemone.ts:99-113` and `:240`).

### Wire response (what the endpoint returns)

Decoded by `oc:packages/ai/src/experimental/system-one.ts:80-87`:

```
Response = { model: string (REQUIRED), answers: Record<id, Answer>, usage?, id?, provider?, provider_metadata? }
Answer (tagged by "type", system-one.ts:57-72):
  choice: { type:"choice", choice: string, probabilities: Record<key, p in [0,1]>, confidence?: p }
  score:  { type:"score",  score: number, probabilities: Record<"0".."n-1", p>, legend?: {...}, confidence?: p }
  noul:   { type:"noul",   noul: p in [0,1] }
```

The SDK then re-checks that the answers match the questions (`oc:packages/ai/src/experimental/evaluation-client.ts:46-83`). Choice: the chosen key must exist, and if probabilities are present they must have exactly the criteria keys. Score: the score must lie in `[0, n-1]`, and probability keys must be `"0".."n-1"`. A mismatch fails with `InvalidProviderOutput`.

The **server's** success body is narrower: `{ "answers": ... }` only, no `model`, no `usage` (`oc:packages/core/src/systemone.ts:125-128`, `oc:packages/server/src/handlers/completions.ts:335`). So the SDK's `SystemOne` client, which requires `model`, cannot decode a façade reply. OpenChamber does not use that client, so this does not bite today, but it is a contract mismatch to record.

### Example, copied from the fixture

Source: `oc:packages/ai/test/fixtures/recordings/opencode-evaluation/evaluates-choice-score-and-boolean-questions.json` (recorded 2026-09-22 against `jev-1.13-free`).

Request body:

```json
{"model":"jev-1.13-free",
 "state":"I was charged twice for the same invoice. Please refund the duplicate payment today.",
 "questions":{
   "department":{"type":"choice","instructions":"Which team should handle this support request?",
                 "criteria":{"billing":"Payments, invoices, refunds, or failed charges","technical":"Bugs, outages, or integrations","sales":"Pricing, upgrades, or new accounts"}},
   "urgency":{"type":"score","instructions":"How urgent is this support request?",
              "criteria":["Can wait for normal support","Needs prompt attention","Actively blocking revenue"]},
   "refund":{"type":"noul","instructions":"Is the customer asking for a refund?"}}}
```

Response body, status 200:

```json
{"model":"jev-1.13-free",
 "answers":{
   "department":{"type":"choice","choice":"billing","confidence":1,"probabilities":{"technical":0,"sales":0,"billing":1}},
   "urgency":{"type":"score","score":1.02,"confidence":0.95,
              "legend":{"0":"Can wait for normal support","1":"Needs prompt attention","2":"Actively blocking revenue"},
              "probabilities":{"0":0,"1":0.97,"2":0.03}},
   "refund":{"type":"noul","noul":0.99}},
 "usage":{"input_tokens":422,"output_tokens":69}}
```

Example OpenChamber probe, from `oc:packages/core/test/facade-parity.test.ts:1085-1092` and `oc-ui:packages/web/server/lib/routing/jev.js:83-92`:

```json
{"state":{"request":"Which of these is a colour: blue or table?"},
 "questions":{"probe":{"type":"choice","instructions":"Pick the word that names a colour.","criteria":{"blue":"A colour.","table":"A piece of furniture."}}},
 "model":"@cf/cloudflare/clef-flash"}
```

Example live reply from Zen, from this audit (2026-11-13, `POST https://opencode.ai/zen/v1/systemone`, header `x-opencode-client: openchamber`, `model: jev-1.13-free`, body is the probe above with that model id):

```json
{"model":"jev-1.13-free","answers":{"probe":{"type":"choice","choice":"blue","confidence":1,"probabilities":{"blue":1,"table":0}}},"usage":{"input_tokens":331,"output_tokens":31,"cost":0,"opencode":{"billing_source":"free","list_cost":0,"tokens":{"input":331,"output":31,"reasoning":0,"cache_read":0,"cache_write_5m":0,"cache_write_1h":0}}}}
```

### Limits and rules (server and SDK)

| Rule | Where |
|---|---|
| Body must be a JSON object; array or scalar is 400 before upstream | `oc:packages/core/src/systemone.ts:99-103` |
| Resolved model must match `^@cf/cloudflare/clef[A-Za-z0-9._-]*$` (Clef-only grammar) | `oc:packages/core/src/systemone.ts:66`, `:102` |
| `state` and `questions` default to `{}` | `oc:packages/core/src/systemone.ts:108-109` |
| Choice: at most 255 options, at least one | `oc:packages/ai/src/experimental/system-one.ts:37`, `evaluation.ts:17-21` |
| Score: at least 2 levels, at most 10 | `oc:packages/ai/src/experimental/system-one.ts:42`, `evaluation.ts:33` |
| Probabilities are numbers in [0, 1] | `evaluation.ts:60` |
| Upstream body limit | `oc:packages/core/src/systemone.ts:212` (5 MiB). OpenChamber's own reader caps at 1 MiB: `oc-ui:.../jev.js:20` |
| Deadline (fetch + body), default 30 s, max 300 s | `oc:packages/core/src/systemone.ts:204-210` |

OpenChamber's own question shape, built in `oc-ui:packages/web/server/lib/routing/jev.js:52-59`, is one `choice` question named `category`, with criteria = category id → description text (plus pool text when the category runs on a pool, `jev.js:39-49`). It parses the reply as `{answers}` or `{result:{answers}}` (`jev.js:124-129`) and requires `choice` (string) and `confidence` (number) (`jev.js:94`).

---

## Endpoints

### Fork opencode server (façade)

| Method and path | Purpose | Source |
|---|---|---|
| `POST /v1/systemone` | Clef-family systemone door; the only path served (no `/systemone` alias) | `oc:packages/protocol/src/groups/completions.ts:83-95`; `oc:packages/server/src/handlers/completions.ts:311-336` |
| `GET /v1/models` | Lists routes and a static Clef floor (`@cf/cloudflare/clef`, `-flash`) | `oc:packages/core/src/completions.ts:287-294` |

Responses from the handler: 400 for bad JSON or unparsable model (`{error}`), 401 `Missing Cloudflare AI inference API token` (`oc:packages/core/src/systemone.ts:234`), 504 on timeout, 500 or upstream status on failure with `{error:"Cloudflare Workers AI Clef execution failed", details}` (`systemone.ts:264-280`). The door's envelope is kept on purpose (`systemone.ts:115-123`).

Backend: Cloudflare AI Gateway, `POST <gateway>/<account>/<gw>/workers-ai/run/<model>` (`systemone.ts:236`). Credentials are read in place: `Authorization` passthrough, then `CLOUDFLARE_API_TOKEN`, then `~/.config/cloudflare/ai-inference-token` (`systemone.ts:153-160`). The `openapi.json` entry is `oc:packages/protocol/openapi.json:5704`.

### Provider-side System One endpoints (fork ai package, client)

| Provider id | Base URL | Model examples | Auth | Source |
|---|---|---|---|---|
| `opencode` | `https://opencode.ai/inference/systemone/v1` + `/systemone` | `jev-1.13-free` (recorded) | `OPENCODE_API_KEY` | `oc:packages/ai/src/providers/opencode.ts:6,16` |
| `openrouter` | `https://openrouter.ai/api/v1` + `/systemone` | `typesafe/jev-1.13` | `OPENROUTER_API_KEY` | `oc:packages/ai/src/providers/openrouter.ts:187-196` |
| `typesafe-ai` | `https://api.typesafe.ai/v1` + `/systemone` | `jev-latest` | `TYPESAFE_API_KEY` | `oc:packages/ai/src/providers/typesafe-ai.ts:3,16` |
| `vercel-ai-gateway` | Its own `/evaluate` endpoint, not systemone | `typesafe-ai/jev` | AI Gateway key | `oc:packages/ai/src/providers/vercel-ai-gateway.ts:278-352` |

The vercel evaluation sends a different body (`providerOptions` and camelCase usage, `vercel-ai-gateway.ts:289-300`). It is not systemone wire. OpenChamber's Vercel source uses `https://ai-gateway.vercel.sh/typesafe/v1/systemone` (`oc-ui:packages/web/server/lib/routing/defaults.js:68`), a systemone path, so it is a different route from the fork's Vercel evaluator.

OpenChamber's own endpoint table, with the URL and model per source (`oc-ui:packages/web/server/lib/routing/classifier.js:130-150`; defaults in `defaults.js:28-69`):

| Source | URL | Model | Credential |
|---|---|---|---|
| `zen-promo` | `https://opencode.ai/zen/v1/systemone` | `jev-1.13-free` | none (header `x-opencode-client: openchamber`) |
| `zen-key` | same | `jev-1.13` | Zen key |
| `typesafe` | `https://api.typesafe.ai/v1/systemone` | `jev-latest` | TypeSafe key |
| `openrouter` | `https://openrouter.ai/api/v1/systemone` | `jev-latest` | OpenRouter key |
| `vercel` | `https://ai-gateway.vercel.sh/typesafe/v1/systemone` | `typesafe-ai/jev` | Vercel key |
| `custom` | user URL, exactly as pasted | user model (default `jev-latest`) | optional key |

### Provider that can decide but is not in the fork code

Cloudflare Clef (`@cf/cloudflare/clef*`) is a server-side façade provider only. OpenChamber reaches it only through the `custom` source, where the user's URL is `http://127.0.0.1:13420/v1/systemone` (`~/.config/openchamber/classifier-endpoint.json`).

---

## Providers able to decide

| Provider / path | Native decision API | Who uses it | Status 2026-11-13 |
|---|---|---|---|
| OpenCode Zen (`zen-promo`, `zen-key`) | Yes, systemone | OpenChamber default-ish source | **Live: 200, free promo.** `ZEN_JEV_PROMOTION_ACTIVE = true` is hard-coded (`oc-ui:.../defaults.js:49`). Promo may end; see §Live health. |
| TypeSafe (`typesafe`, `jev-latest`) | Yes | OpenChamber | Not configured on this machine (`routing-auth.json` absent). Not tested. |
| OpenRouter (`typesafe/jev-1.13`, OpenChamber uses `jev-latest`) | Yes | OpenChamber, fork ai package | Not configured. Not tested. |
| Vercel AI Gateway (`typesafe-ai/jev`) | Yes, via `/evaluate` in the fork; systemone path in OpenChamber | Both | Not configured. Not tested. |
| Cloudflare Clef via façade `/v1/systemone` | Yes, Clef family only | OpenChamber `custom` (the only current consumer of `classifier-endpoint.json`) | **Dead.** Ruled won't-do (#1651, Drew 2026-11-04). Probe returned 401 from this shell (see below). |
| LLM adaptation (a chat model asked to emit JSON) | **Not implemented.** No code path in the fork or OpenChamber asks a chat model to emit decision JSON. Supplement §4 (LLM-as-decider) has no implementation to start from. | — | — |

Command Code (`_commandcode`) is not a decision provider in code. Its live usage is all failures (see §Live health).

---

## Config surface

### opencode (fork and this repo)

- `experimental.model_routes` (`.opencode/opencode.jsonc:2155`): pools for chat traffic. Each has `targets`, `selection` (`ordered`, `weighted`) and `autonomy`. **There is no decision-pool key, no decision default key, and no `classifier` or `systemone` key in config.** Grep for `evaluation`, `decision` and `systemone` in `packages/core/src/config` found only comments (`oc:packages/core/src/config/plugin/model-routes.ts:284,621,648,920`).
- `OPENCODE_SYSTEMONE_GATEWAY_BASE`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_GATEWAY_ID`, `CLEF_PROXY_DATA_PATH_TIMEOUT_MS` (`oc:packages/core/src/systemone.ts:153-160,194,207`).
- The `classifier` and `classifier-complex` agents in `.opencode/opencode.jsonc:1632,1756` are **chat subagents** pinned to `opencode-route/scout` and `workhorse`. They are unrelated to Jev. Name clash to watch.

### OpenChamber

Files under `~/.config/openchamber/` (paths from `oc-ui:packages/web/server/lib/routing/store.js:180-184`):

| File | Holds | This machine |
|---|---|---|
| `routing.json` | `enabled`, `fallback`, `minConfidence` (default 0.6, `defaults.js:160`), `preserveCache` (default true), `safetyNet`, `stickiness`, `categories` (built-in overrides and user categories) | Present, see category table |
| `classification.json` | `source`: one of `off`, `zen-promo`, `zen-key`, `openrouter`, `vercel`, `typesafe`, `custom` (`classifier.js:37`) | `{"version":1,"source":"zen-promo"}` |
| `classifier-endpoint.json` | `endpoint.url`, `endpoint.model`, optional `key` (mode 0600) | url `http://127.0.0.1:13420/v1/systemone`, model `@cf/cloudflare/clef-flash`, no key. Not selected, since source is `zen-promo`. |
| `routing-auth.json` | TypeSafe key | Absent |
| `preferences.json` / `settings.json` | UI defaults. `defaultModel: opencode-route/workhorse`, `smallModelOverride: opencode-route/speed` | Auto is **not** the default model |

Environment and policy: `OPENCHAMBER_JEV_URL`, `OPENCHAMBER_JEV_MODEL` (default `jev-latest`), `OPENCHAMBER_JEV_API_KEY`, or the `jev` key in the machine policy file (`oc-ui:.../classifier.js:64-85`). A pin replaces the saved endpoint and forces the source to `custom` (`runtime.js:205-212`).

Source resolution: `resolveClassifier` (`classifier.js:95-114`). Chosen = stored pick, or `typesafe` if a key exists, else `off`. If the chosen source is unusable, `FALLBACK_ORDER` applies: `typesafe`, `custom`, `openrouter`, `vercel`, `zen-key` (`classifier.js:39`). `zen-promo` is never a fallback (`classifier.js:17-20`).

Gating: Auto is ready only when `jevAvailable && config.enabled && fallback && enabledCategories >= 2` (`runtime.js:232`). The UI message is `settings.routing.status.tooFewCategories` (`oc-ui:packages/ui/src/lib/i18n/messages/routing.i18n.ts:117`).

---

## OpenChamber Auto: categories

Built-in categories, `oc-ui:packages/web/server/lib/routing/defaults.js:132-158`. The description is sent verbatim to Jev as the criteria for that category. Names and descriptions are shipped defaults. A user can override them, and a user can add categories (`store.js:97-105`). Up to 32 categories (`store.js:146`).

Current mapping, from `~/.config/openchamber/routing.json` (read only). The fallback is `opencode-route/workhorse`, with no variant.

| id | name | description (abridged, see defaults.js) | model (current) | thinking (variant) | agent | enabled |
|---|---|---|---|---|---|---|
| `trivial` | Trivial | Mechanical or purely factual work: rename, typo, reformat, add a comment, one obvious command. Not design judgement. | `opencode-route/speed` | `high` | `build` | yes |
| `research` | Research | Read and explain existing code without changing it. Not for edits or plans. | `opencode-route/workhorse` | `max` | `build` | yes |
| `implement` | Implement | Ordinary bounded engineering: well-specified function, endpoint or component; tests for existing behaviour; localised fix with known cause. | `opencode-route/workhorse` | `high` | `build` | yes |
| `review` | Review | Inspect a change for correctness, regressions, security and tests. Report without editing unless asked. | `opencode-route/specialist` | `high` | `build` | yes |
| `hard` | Hard | Hard reasoning, ambiguity, planning or high blast radius: unknown-cause failures, multi-module design, auth, concurrency, migrations, money. | `opencode-route/summit` | `high` | `build` | yes |

No user categories exist. Every category has a pool, so the brief §1 requirement "appropriate pools for all categories" is met on this machine. Two items to check: `research` uses variant `max`, which may not be a rung on the workhorse pool (`runtime.js:378-397` silently drops unknown variants when the catalog lists the model, and only then); and `workhorse` is a pool, so the catalog check may not apply at all.

Each category has a `variant` only when the user set one. Thinking `high` on `speed`, for example, is a pool-level variant, not a model variant.

---

## Classification flow

Path for an Auto send (`oc-ui:packages/web/server/lib/routing/runtime.js:411-460`):

```
user sends with model = openchamber/auto
  -> resolveAutoSelection                                  runtime.js:411
     - no fallback?          -> throw 400 (never reaches OpenCode)
     - describe(): autoReady = jevAvailable && enabled && fallback && >=2 enabled categories   :232
     - readHistory(session)  -> last 3 turns, cut to 600/300/300 chars   history.js, defaults.js:128
     - resolveAccess()       -> classifier source + endpoint              :199-216
     - jev.ask({state:{history,request}, questions:{category:choice}}, endpoint)   jev.js:137-168
         POST endpoint.url, body {...request, model: endpoint.model}, timeout 4000 ms (defaults.js:72)
         reply: {answers} or {result:{answers}}, cap 1 MiB
     - decideRouting(answers.category)                        jev.js:108-115
         not a choice / unknown category  -> reason unknown-category, conf 0 or conf
         confidence < minConfidence (0.6) -> reason low-confidence
         else                              -> reason routed, category
     - cacheHold(...)   preserve-cache: stay on the category the warm model already serves  runtime.js:309-324, cache.js
     - chooseWithStickiness(...)  hysteresis (2 to leave start model, 3 to leave a jump)   runtime.js:347-367, stickiness.js
     - withKnownVariant(...)  drop variant the catalog does not list                       runtime.js:378-397
     - decision broadcast 'openchamber:routing.decision'      runtime.js:458
  -> applySessionSelection: session.switchModel + switchAgent   runtime.js:486-490
```

Failure branch: any throw in the Jev block sets `reason = 'error'`, stores the message, and calls `chooseWithStickiness` with `noSignal: true`. That keeps the session on its current model if it has one, else uses the fallback (`runtime.js:448-452`, `chooseSelection` at `:328-338`).

Permission path, for `safety` sessions (`runtime.js:518` on, `jev.js:65-77`): one `noul` question (`ask`) plus one `choice` question (`kind`). Hold if `ask.noul >= threshold` (default 0.6, `defaults.js:163`). Jev failure leaves the request for the user with a toast (`oc-ui:packages/ui/src/hooks/useRoutingSync.ts:38-41`).

Choice of pool: Jev does not see pool ids as options. For a pool category it sees the pool's description and details appended to the category text (`jev.js:39-49`). Jev answers a category, and OpenChamber then maps the category to `opencode-route/<pool>`. OpenCode resolves the pool. OpenCode's pool chooses the seat.

---

## Recording and logging today

| What | Where | Persisted |
|---|---|---|
| Latest decision per session (category, confidence, reason, ms, model, variant, agent, held, error) | `oc-ui:packages/ui/src/stores/useRoutingStore.ts:26,102-103` (in memory) | No |
| Decision event | `openchamber:routing.decision`, broadcast (`runtime.js:458`) | No (event stream only) |
| Auto session marks | `auto-sessions.json` (`runtime.js:175-180`, `store.js:171-178`) | Path exists in code; **file absent** from `~/.config/openchamber/` on 2026-11-13, so none is saved |
| Stickiness state | Process memory (`stickiness.js` header) | No, restart resets |
| Permission decisions | Process memory, 15-minute TTL (`runtime.js:37`) | No |
| Jev call attempt, tokens, latency | Not recorded by OpenChamber. The fork ledger (`route_attempt`) records only pool and façade attempts. The `/v1/systemone` façade does record `route_attempt` with session_id NULL (`oc:packages/core/src/systemone.ts:37-43`). Zen-promo and TypeSafe calls go straight from OpenChamber and are not ledgered. | Partial |
| Category chosen per send | Not written to history; the routing-review skill notes that `route_usage` cannot split pools by category. | No |

Supplement §5 and §9 (per-decision ID, prompt version, tokens, outcome, invalid-JSON path) are absent. There is no decision ID, no prompt version beyond the constant `ROUTING_INSTRUCTIONS` in `defaults.js:96-101`, and no outcome feedback.

---

## Failure behaviour today

| Failure | Behaviour | Source |
|---|---|---|
| Jev unreachable / HTTP error | Thrown with `status`; decision `error`; session keeps model or uses fallback. No retry. | `jev.js:149-151`, `runtime.js:448-452` |
| Timeout (4 s) | `code: 'timeout'`; same as above. Fork's cold TLS note says ~1 s. | `jev.js:139,163`, `defaults.js:71-72` |
| Body too large (>1 MiB) | `response_too_large`; same as above. | `jev.js:20,152-154` |
| Non-JSON or no `answers` | `Jev response has no answers`; same as above. **No retry** (supplement §4 asks for two tries with feedback). | `jev.js:159-160` |
| Answer not a choice, or unknown category | `reason: unknown-category`; stickiness keeps current model. | `jev.js:110-112` |
| Confidence below 0.6 | `reason: low-confidence`; keep current model. | `jev.js:113` |
| No source usable (`off`, or no key) | `jevAvailable = false`; Auto hidden from picker; Auto send throws 400 only if fallback is missing. With no Jev source, `resolveAutoSelection` still runs `chooseSelection(config, null)` and sends to the fallback (`runtime.js:453-455`). | `runtime.js:199-216,453-455` |
| Pool exhaustion | Not a Jev concern. Handled by OpenCode pool expansion for the chosen pool. | Not in Jev path |
| Fork façade: no token | 401 `Missing Cloudflare AI inference API token` | `systemone.ts:233-234` |
| Fork façade: upstream 401 | 401 upstream-failure envelope, same as door | `systemone.ts:280` |
| Fork façade: timeout | 504 `Cloudflare Workers AI Clef request timed out` | `systemone.ts:255-264` |

There is no circuit breaker and no "next source" failover inside a single decision. FALLBACK_ORDER only applies when the chosen source is unusable from configuration, not when it fails at call time (`classifier.js:106-108`).

---

## Live health

Checked 2026-11-13 ~06:08 CST.

| Check | Result |
|---|---|
| Active OpenChamber source | `zen-promo` (`classification.json`) |
| Zen promo, live probe, `jev-1.13-free` | **200**, valid typed choice answer, 331 in / 31 out tokens |
| Façade `127.0.0.1:13420/v1/systemone` (current `classifier-endpoint.json`) | **401**, `{"_tag":"UnauthorizedError","message":"Authentication required"}`. This is the server's own auth envelope, not the systemone one, so the request is rejected before the handler. It is the `custom` source, which is not selected. |
| Door `127.0.0.1:18742` (retired) | `000`, nothing listening, as #1410 records |
| Clef upstream credential | Dead by ruling (#1651 closed NOT_PLANNED, 2026-11-04; the 2026-11-06 comment keeps the plumbing but bans other use). Last recorded upstream failure: code 10000 "Authentication error". |
| `route_usage` `clef-flash` (96 h) | 0 targets. Jev calls from OpenChamber are not in the route ledger, so this proves nothing about Jev's health. |
| `route_usage` `_commandcode` (96 h) | 9,316 `deepseek-v4.1-flash` requests, **8,884 failed**; 299 `gpt-6.1-sol` failed; 513 `reflection/beam:free` failed; 4 `mimo-v2.6-flash` failed. Metered tokens only on 2026-11-10. Not a decision path. Command Code is not a Jev backend in code. |
| `auto-sessions.json` | absent, so no session is persisted as Auto |
| Auto readiness inputs | enabled true, fallback set, 5 enabled categories, Jev source usable (promo) → `autoReady` should be true (not run live) |

Live caution: the promo is a limited-time free model. `ZEN_JEV_PROMOTION_ACTIVE` is a hard-coded flag (`defaults.js:49`). When OpenCode ends it, Auto stops being offered until the user adds a key. The fallback order does not cover `zen-promo`, so there is no automatic fallback to a paid key.

---

## Issues

Open or closed as seen by `gh` on 2026-11-13.

| Repo | Issue | State | Title (abridged) | Relevance |
|---|---|---|---|---|
| developing-today-forks/opencode | #1146 | **OPEN** | Design: generic OpenAI façades on the server; `/v1/systemone` is the decision-family endpoint; 46 comments, design PR #1156 merged, search façade linked to PR #2100 | Parent of the decision-door design. The façade rule "one endpoint per real standard" is set here. |
| developing-today-forks/opencode | #1410 | CLOSED 2026-11-04 | Door front migration: clef-proxy (18742) callers repoint to the façade | Done: OpenChamber repointed `classifier-endpoint.json`; door retired (code#5583, #5591, #5594). Note: the repoint did not change the active source. |
| developing-today-forks/opencode | #1627 | CLOSED | Façade routes in OpenAPI but not served (SPA/405 on /v1/*) | Fixed by redeploy. |
| developing-today-forks/opencode | #1651 | CLOSED NOT_PLANNED | Cloudflare AI gateway credential fails upstream (code 10000) | Ruled dead 2026-11-04; plumbing kept 2026-11-06, use banned except sanctioned tests and search. **Clef is dead by ruling.** |
| developing-today-forks/opencode | #1477 | CLOSED | systemone data-path timeout unbounded | Fixed (clamped to 300 s). |
| developing-today-forks/opencode | #1152 | CLOSED | Façade parity suite against live doors | Parity test exists. |
| developing-today-forks/opencode | #2253 | OPEN | Router: model opendesign membership caps | Tangential, pool-level. |
| developing-today-forks/openchamber | #763 | CLOSED | Jev classifier client buffers response with no cap | Fixed (1 MiB). |
| developing-today-forks/openchamber | #148 | CLOSED | Route pools: short and long descriptions given to the classifier | Done: pool text in `jev.js:39-49`. |
| developing-today-forks/openchamber | #147 | CLOSED | Learn from manual model and effort choices: notes for the classifier and Settings suggestions | Not implemented; not in code. |
| developing-today-forks/openchamber | #1608 | CLOSED | routing store reads JSON files with no size gate | Fixed. |
| developing-today-forks/openchamber | #1665 | CLOSED | Pool schedules settings search lands on wrong tab | UI only. |
| developing-today-forks/openchamber | #1762 | CLOSED | TypeSafe key search lands on Routing | UI only. |
| developing-today-forks/openchamber | #152 | OPEN | Tracking: threads from the 2026-10-20 config, goal editor and integrations session | Matched "classifier"; not read in full. |
| developing-today/code | — | not searched | rate-limited (HTTP 403, GitHub search quota exhausted at 12:07 UTC) | Gap. Retry later. |

`gh search` for "auto routing" and "categories" in opencode found no matching issue. "systemone" and "jev" matched only the issues above.

---

## Gaps vs supplement §1–§10 and brief §1–§2

Supplement §1–§10 are the decision-model system; brief §1–§2 are the OpenChamber categories and self-switching asks.

| Ask | State today | Gap |
|---|---|---|
| Supp §0: Jev is Command Code's decision model | Not supported by code. Jev = TypeSafe / Zen / OpenRouter / Vercel. Clef = Cloudflare. | Decide whether to keep the premise. Code says no. |
| Supp §1: accept all conversation endpoints, pass through or adapt | Fork accepts `/v1/chat/completions`, `/v1/responses` (façade, #1146) and `/v1/systemone` (Clef only). Systemone is not a chat dialect and is not adapted. | No pass-through for systemone to other providers from the façade. |
| Supp §1: long-term caching of sessions, search by id, tags | Not implemented for decisions. | Full gap. |
| Supp §2: decision pools, free first, then plan, then PAYG; Cloudflare included budget | No decision pool, no decision default key, no Cloudflare budget tracking. | Full gap. |
| Supp §2: Cloudflare $10 included usage | Cloudflare path is dead by ruling. | Likely drop, or rule again. |
| Supp §3: executor skill/runbook/mcpx tool | None. Jev is called directly from OpenChamber's server. | Full gap. |
| Supp §3: every decision has an ID; caller and called give points 0–5 | None. Decisions have no ID. | Full gap. |
| Supp §4: LLM-as-decider, two tries on invalid JSON, then next, then default | One attempt, no retry, then session fallback. No LLM decider exists. | Full gap. |
| Supp §5, §9: outcome capture and decision record | Not in code. | Full gap. |
| Supp §6: effort as tie-break lean and fallback pool | Not in code. Category has one `variant`. | Full gap. |
| Supp §7: conditional clauses | Not in code. | Full gap. |
| Supp §8: PM-stack review surface | Not in code. | Full gap. |
| Supp §10: cycle-safe pool traversal | OpenCode pool expansion has a per-call visited set (per `.opencode/opencode.jsonc` comments; not re-verified here). | Not applied to decisions. |
| Brief §1: pools for all categories | Met on this machine (5 of 5 built-ins mapped). | No user categories. Variant `max` on `research` to check. |
| Brief §1: Auto must work | `autoReady` should be true; live check not run through the function. Promo may end. | Verify once in the UI; add a health check. |
| Brief §1: provider order free → paid → Cloudflare | Code: typesafe → custom → openrouter → vercel → zen-key, promo never a fallback, no Cloudflare source. | Order differs. Decide. |
| Brief §2: model may switch its own pool, only to category or route pools | Not implemented for Jev. Pool choice for a category is fixed in `routing.json`. | Gap. |
| Brief §2: default to auto | `defaultModel` is `opencode-route/workhorse`; Auto is opt-in in the picker. | Gap. Decide default. |
| Brief §2: fallback to workhorse / general / build; small options | Fallback = workhorse; agent build. `small` = `speed` (`smallModelOverride`). | Mostly met. |

---

## Open questions (with defaults I chose)

1. **Is Jev a Command Code model?** Default: no, per code. Confirm with Drew before any design leans on it.
2. **Does the free promo end soon?** Default: treat `zen-promo` as the active source and record the risk. Flag `ZEN_JEV_PROMOTION_ACTIVE`.
3. **Is Auto actually working end to end?** Default: unverified. Next step is one Auto send with a logged `openchamber:routing.decision`.
4. **Is `research` variant `max` a real rung on `opencode-route/workhorse`?** Default: unverified, leave as-is.
5. **Provider order.** Default: keep code order (paid first, promo never a fallback), and record the brief's free-first order as a decision to make.
6. **Decision default and pool.** Default: none exists; the design must add one.
7. **developing-today/code issues** were not searched (GitHub search rate limit, 12:07 UTC). Default: record as a gap; retry later.

Report written; no commits, no pushes, no other files changed.
