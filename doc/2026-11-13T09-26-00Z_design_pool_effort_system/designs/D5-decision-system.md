# D5 — Decision system (endpoint, decision pools, executor, LLM decider, effort lean, clauses, traversal)

Program: code#9399 (design-mill, queue item 15864). Parent map: `../main.md` (AD5, AD6, AD7 binding).
Drafter: route `opencode-route/workhorse`, detached run, branch `design/pool-effort-system`.
Status: living design. Versions layer; they never replace. Nothing here is committed or filed.

Statement ids: no `inventory/statement-map.json` exists yet, so statements are cited as
`brief §n` and `supp §n` (line-cited where the section is long). The inventory themes are mapped
to those sections by this drafter as follows, and must be re-checked against the map when it lands:
T9 endpoint = supp §0–§1; T10 decision pools = supp §2 and brief §1 provider order; T11 executor and
records-from-executor = supp §3; T12 LLM-decider protocol = supp §4; T14 decision effort = supp §6;
T15 conditional clauses = supp §7; T17 cycle-safe traversal = supp §10.

---

## Scope

| Theme | Brief / supplement sections | Statement ids |
| --- | --- | --- |
| T9 endpoint | supp §0, §1 | supp §0 (decode), supp §1 bullets 1–3 |
| T10 decision pools | supp §2; brief §1 "wire up … free then paid then cloudflare"; brief §8 "DS Flash last resort" (decision use only) | supp §2 bullets 1–6; brief §1 bullet 2 (decoded per questions.md Q2) |
| T11 executor | supp §3 | supp §3 bullets 1–4 |
| T12 LLM decider | supp §4 | supp §4 bullets 1–3 |
| T14 decision effort | supp §6 | supp §6 both meanings and "dependent probabilities" |
| T15 conditional clauses | supp §7 | supp §7 bullets 1–4 (all eight examples, see §7.4) |
| T17 cycle-safe traversal | supp §10; AD6 | supp §10 |
| Provider order | brief §1 bullet 2 | Q2 default |

Out of scope here (owned elsewhere): decision record schema and feedback gauges (D6, supp §5, §9),
calibration statistics (D3/V3), effort tiers for chat pools (D1), categories and Auto (D2).

---

## 1. V1 — what exists today (never redesigned)

- **Wire contract.** Request `{model, state, questions}`; `questions` are `choice`, `score`, `noul`;
  answers carry `probabilities` and `confidence`. Source: `baseline/v1-decisions-auto.md` "Schema"
  (fork `packages/ai/src/experimental/system-one.ts:48-55`, `:80-87`).
- **Façade door.** `POST /v1/systemone` (`packages/server/src/handlers/completions.ts:311-336`)
  parses the body (`packages/core/src/systemone.ts:99-113`), refuses any model outside
  `@cf/cloudflare/clef*` (`systemone.ts:66`, `:102`), forwards only `{state, questions}` upstream
  (`systemone.ts:240`) and answers `{answers}` only (`systemone.ts:125-128`). Errors are `{error}`.
  Upstream is the Cloudflare AI gateway (`systemone.ts:236`). Ruled dead for credentials (#1651).
- **Client.** `packages/ai/src/experimental/system-one.ts` requires `model` in the response
  (`Response` schema), so it cannot decode the façade reply. Request is `StructWithRest`, so
  extra keys are accepted client-side and dropped by the door.
- **OpenChamber.** Sends `{...request, model}` to the endpoint in `routing.json`/`classification.json`
  (`oc-ui:packages/web/server/lib/routing/jev.js:145`); active source `zen-promo`; one 4 s timeout,
  no retry (`defaults.js:71-72`); invalid output means one attempt then session fallback
  (`jev.js:159-160`; `runtime.js:448-452`).
- **Config.** `experimental.model_routes` (`.opencode/opencode.jsonc:2155`) with `targets`
  (strings, or objects with `model`, `window`, `limit.context`), `selection` (`ordered`/`weighted`),
  `autonomy`. The `limited` pool (`.opencode/opencode.jsonc:3597-3671`) is the pattern for
  per-member caps: `{ "model": "opencode-route/luna-plan#medium", "limit": { "context": 100000 } }`.
  No decision key exists (`baseline/v1-decisions-auto.md` "Config surface").
- **Ledger.** `route_attempt` and `route_decision` in `opencode.db` (main.md §1 fact 7). The façade
  writes `route_attempt` with `session_id` NULL and tokens 0/0 (`systemone.ts:37-43`, `:186-191`).
- **Traversal.** Fallback is walked with a per-call visited set; guard tests enforce termination
  (brief §9, PR #9385; `.opencode/route-fallbacks.md`). Decisions do not use it (supp §10 gap).

---

## 2. Decision pool taxonomy — settling Drew's two shapes

Drew gave two taxonomies (supp §2 bullets 1 and 2):

- A: decision plan · decision LLM plan · decision free · decision LLM free · decision PAYG · decision LLM PAYG
  (six shapes: {plan, free, PAYG} × {native, LLM}).
- B: decision bland · decision free · decision pay-as-you-go, composed of native free, LLM free, etc.

**Settled shape: `decision` = `decision-free` ∪ `decision-plan` ∪ `decision-payg`, then the last
resort. Each tier is composed of a `native` part and an `llm` part, in that order.** A is the
product, B is the outer view; both are the same graph. A tier's native part is tried before its LLM
part. This keeps tier (cost class) as the outer order, which is what Drew asked for ("free, then
plan, then PAYG"), and keeps native-vs-LLM as the inner order, which is what the measurement decides
(§9). Only leaf pools exist as config; tier pools are composed by targets.

### 2.1 Options considered (taxonomy)

| # | Option | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| O-a | Six flat pools (A verbatim) | Literal to Drew | No single default; order across pools is hand-written; the free/plan/PAYG tiers cannot share a native or LLM leg | — |
| O-b | Three tier pools only (B) | One list per tier | Native and LLM legs of a tier are hidden; cannot say "native first" | — |
| **O-c** | **Tier pools composed of native and LLM sub-pools; one default** | Both views are real routes; default is one key; reuses existing pool composition | six leaf pools to maintain | `model_routes` tier pools nest per-model pools (opencode.jsonc:2157-2159) |
| O-d | Single weighted pool across all legs | Simple | Loses the free→plan→PAYG order that brief §2 asks for; weights hide cost class | NixOS priorities (R1) |

### 2.2 V2 config

Existing route syntax only (`targets` strings and objects, `selection`, `autonomy`). The default is
one new key under `experimental` (§2.4).

```jsonc
"model_routes": {
  // ---- native and LLM sub-pools (leaf pools) ----
  "decision-native-free": {
    "name": "Decision native free (systemone, free door first)",
    "targets": [
      { "model": "opencode-zen/jev-1.13-free" },              // Zen promo, live 200 2026-11-13
      { "model": "cloudflare-ai-gateway/clef-flash", "window": "decision-budget-open" } // §5, flag
    ],
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-llm-free": {
    "name": "Decision LLM free (only measured, flagged)",
    "targets": [],   // empty until the free LLM legs pass M5 in the trial (§9); never a guessed leg
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-native-plan": {
    "name": "Decision native plan (subscription-included native systemone)",
    "targets": [],   // no plan-included native door exists in V1 (baseline §Providers able to decide)
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-llm-plan": {
    "name": "Decision LLM plan (off-peak DeepSeek Flash first)",
    "targets": [
      { "model": "opencode-route/deepseek-flash-plan#low", "window": { "name": "deepseek-peak-early", "invert": true } },
      { "model": "opencode-route/deepseek-flash-plan#low", "window": "deepseek-peak-early" }
    ],
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-native-payg": {
    "name": "Decision native PAYG (Jev paid doors)",
    "targets": [
      { "model": "typesafe-ai/jev-latest" },
      { "model": "openrouter/typesafe/jev-1.13" },
      { "model": "vercel-ai-gateway/typesafe-ai/jev" },
      { "model": "opencode-zen/jev-1.13" }
    ],
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-llm-payg": {
    "name": "Decision LLM PAYG",
    "targets": [
      { "model": "deepseek/deepseek-flash" }                  // PAYG official API, cache-miss price
    ],
    "selection": "ordered",
    "autonomy": "rules"
  },

  // ---- tier pools: native part first, then LLM part ----
  "decision-free": {
    "name": "Decision tier free",
    "targets": ["opencode-route/decision-native-free", "opencode-route/decision-llm-free"],
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-plan": {
    "name": "Decision tier plan",
    "targets": ["opencode-route/decision-native-plan", "opencode-route/decision-llm-plan"],
    "selection": "ordered",
    "autonomy": "rules"
  },
  "decision-payg": {
    "name": "Decision tier PAYG",
    "targets": ["opencode-route/decision-native-payg", "opencode-route/decision-llm-payg"],
    "selection": "ordered",
    "autonomy": "rules"
  },

  // ---- the default decision pool ----
  "decision": {
    "name": "Decision (default): free → plan → PAYG → Luna 100k last resort",
    "targets": [
      "opencode-route/decision-free",
      "opencode-route/decision-plan",
      "opencode-route/decision-payg",
      { "model": "opencode-route/luna-plan#medium", "limit": { "context": 100000 } }
    ],
    "selection": "ordered",
    "autonomy": "rules"
  }
}
```

Notes on the rows above, each a V2 gate, not a fact:

- Model ids are the `model` spellings the fork's door accepts today; `opencode-zen/...`,
  `typesafe-ai/...` and `openrouter/...` provider names must be checked against the catalog when
  the slice lands (D5-S07). Where a spelling differs, the slice fixes it; the design does not
  guess a provider id as fact.
- `decision-native-plan` is **empty by design**: no subscription-included native systemone door is
  recorded in V1. An empty tier part is skipped (`skipped-empty`, not an error), so the pool still
  orders correctly.
- `decision-llm-free` is **empty until measured**. Groq and Cerebras are "super-fast free" candidates
  (brief §4) with third-party prices only (R2 §1.8). They enter only after the M5 invalid-JSON test in
  §9 passes; until then the empty part is skipped.
- Luna @100k `#medium` is the last target and is pinned directly, never via `small` or `limited`
  (Q4; supp §4 bullet 1).
- The Cloudflare row carries `window: "decision-budget-open"`, a window named like the existing
  `deepseek-peak-*` windows. The window is true while the UTC-day neuron counter is under cap (§5).
  It is a **flag** (`experimental.decision.cloudflare.enabled`, default `false`) pending the Clef
  ruling (Q5): the leg is not in the list at all while the flag is off.

### 2.3 Concrete V2 leg list (ordered, with the evidence for each)

| Order | Leg | Class | Evidence | Status |
| --- | --- | --- | --- | --- |
| 1 | Zen `jev-1.13-free` (`/zen/v1/systemone`) | native, free | Live 200, typed answer, 331 in / 31 out, 2026-11-13 (baseline "Live health") | Promo; `ZEN_JEV_PROMOTION_ACTIVE` is a hard-coded flag (`oc-ui:…/defaults.js:49`). When it ends the leg fails over by pool order, not by a code path |
| 2 | Cloudflare Clef-flash, budget-gated | native, included allowance | 10,000 neurons/day; $0.038/Mtok in = 3,455 neurons/Mtok (R2 §1.2) | **Flagged off**; Clef ruled won't-do (#1651) |
| 3 | `decision-llm-free` (measured free LLMs) | LLM, free | none measured | **Empty until M5** |
| 4 | `decision-native-plan` | native, plan | none in V1 | **Empty** |
| 5 | `decision-llm-plan`: DeepSeek Flash off-peak, then peak | LLM, plan | deepseek-plan route exists (`opencode.jsonc` `deepseek-flash-plan`); off-peak $0.15/Mtok in, peak $0.30 (R2 §1.6) | Live route; peak-aware windows exist |
| 6 | TypeSafe `jev-latest` (`api.typesafe.ai/v1/systemone`) | native, PAYG | $0.042/Mtok in, output free (R2 §1.1); key `TYPESAFE_API_KEY` by name | **Not configured on this machine** (`routing-auth.json` absent) |
| 7 | OpenRouter `typesafe/jev-1.13` | native, PAYG | `openrouter.ts:187-196` | Not configured |
| 8 | Vercel `typesafe-ai/jev` | native, PAYG | Vercel path is an evaluator, not systemone (`baseline` "Provider-side"); systemone path `ai-gateway.vercel.sh/typesafe/v1/systemone` (OpenChamber) | Not configured; spelling checked at D5-S07 |
| 9 | Zen `jev-1.13` (Zen key) | native, PAYG | `zen-key` source (`defaults.js`) | Key by name `OPENCODE_API_KEY` |
| 10 | `decision-llm-payg`: DeepSeek Flash official API | LLM, PAYG | $0.15/$0.60 off-peak, $0.30/$1.20 peak (R2 §1.6) | Candidate |
| 11 | OpenAI cheap LLM (gpt-5-nano class) | LLM, plan or PAYG | $0.05/$0.40; strict Structured Outputs (R2 §1.3) | Admitted only by §6 quota-% test; not in the first list |
| 12 | Local decision model | local | none | V3 (O22 in R2), needs 300+ labels |
| last | Luna 100k `#medium` | LLM, plan | `limited` pool, 100k cap (`opencode.jsonc:3610-3613`) | Pinned last resort |

Order inside a tier is native first. Whether native beats the LLM leg is the measurement in §9; the
config is changed by a trial, not by this design.

### 2.4 The default key

```jsonc
"experimental": {
  "decision": {
    "default": "opencode-route/decision",        // the one key that makes decision the default
    "default_answers": { /* §4.6 */ },
    "cloudflare": { "enabled": false, "neurons_per_utc_day": 10000, "paid_neuron_cap": null },
    "lean": { "delta": 0.05, "step": 0.01 },      // §6
    "fallback_pool": { "low": "opencode-route/speed", "medium": "opencode-route/workhorse", "high": "opencode-route/summit" },
    "max_tries": 2,
    "max_repairs_per_answer": 1
  }
}
```

Config-check rules (a fork slice, D5-S01): `default` must name a route that exists; `lean.step ≤
lean.delta`; `max_tries ∈ {1, 2}`; `fallback_pool` values must exist. A violated rule is a config error,
not a silent default.

### 2.5 Options considered (default-key placement)

| # | Option | Verdict |
| --- | --- | --- |
| O-p | `experimental.decision.default` (chosen) | Keeps decision config in one object; the default is one key, as supp §3 asks |
| O-q | A top-level `decision` key | Rejected: top-level keys are not versioned as experimental; harder to gate |
| O-r | Reuse `routing.json` `fallback` | Rejected: that key is OpenChamber's chat fallback, not decision |
| O-s | Per-category decision pool | Rejected for V2: categories choose a chat pool; decisions have one default (supp §2 bullet 3) |

---

## 3. The endpoint (T9; supp §0, §1)

### 3.1 What it accepts

The façade keeps `POST /v1/systemone` and accepts the **standard body** `{model, state, questions}`
unchanged, plus an optional `x-` envelope. Today it refuses everything except Clef (`systemone.ts:102`).
V2 changes that: the `model` may name a decision route (`opencode-route/decision` or any
`opencode-route/decision-*`), a native systemone provider model, or a Clef model (unchanged behaviour
for the existing grammar).

Pass-through vs adapt:

| Upstream | Action | Why |
| --- | --- | --- |
| Native systemone (Zen, TypeSafe, OpenRouter, Vercel-systemone-path) | **Pass-through**: body `{state, questions}` only, `model` as the upstream id | Native and standard; no transform, no downside |
| Clef (Cloudflare) | Pass-through (today's door, unchanged) | Existing contract (`systemone.ts:240`) |
| LLM leg | **Adapt**: the prompt v1 (§4.2) and the schema (§4.3) turn the question set into a JSON answer; the output is validated and mapped to standard `answers` | No native probabilities; this is the only transform |
| Chat completions / responses | Unchanged (façade rule, #1146) | Out of scope for D5 |

Option table (endpoint shape):

| # | Option | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| E-a | Keep Clef-only door; decision goes through OpenChamber only | No change | No decision route, no ID, no fallback | — |
| E-b | Second endpoint `/v1/decisions` | Clean | Breaks "one endpoint per real standard" (#1146) and the parity suite | — |
| **E-c** | Same `/v1/systemone`; route by `model`; standard body; `x-` envelope for extras | One standard; clients unchanged; extras never bend the standard | Model grammar widens (must be tested: a non-Clef model no longer 400s) | JSON Schema annotations `x-` (R2 §4.5) |
| E-d | Accept only chat-shaped decisions via tool calls | Uses existing chat door | Not systemone; no probabilities | — |

Chosen: **E-c**. Caveat recorded (challenge C2 below): the grammar change is a behaviour change for
the façade; the parity suite (`facade-parity.test.ts`) must be updated to pin the new refusals.

### 3.2 The `x-` envelope: superset that never bends the standard

Rules, each a guard slice (D5-S04, PBT):

1. The façade **strips every key starting with `x-`** from the request body before any upstream call
   (AD5). Stripping is the only place `x-` keys are read on the way out.
2. Standard keys are `model`, `state`, `questions` (plus the standard optional fields the fork already
   defines). An `x-` key is never a standard key, so no standard field changes meaning.
3. A request with no `x-` keys is **byte-identical** on the wire to the standard request. The guard
   checks `strip(x_envelope(req)) == standard(req)` for random requests.
4. Response: the body stays the standard `{model, answers, usage?}` shape. Decision identity travels
   in **headers**, not the body: `x-opencode-decision-id`, `x-opencode-decision-leg`,
   `x-opencode-decision-pool`. A standard client never sees a new body key. Body `x-` keys are added
   only when the request sent `x-envelope: "full"`.
5. Unknown `x-` keys are **ignored**, never 400. Unknown non-`x-` keys keep today's behaviour (dropped
   by the door, `systemone.ts:240`).

Envelope fields (V2): `x-effort` (`low|medium|high`, §6), `x-lean-order` (criteria keys in
direction order, §6), `x-clauses` (§7, V2 accepts the field and stores it; V3 compiles it),
`x-tags` (string array, for search by tag), `x-session` (session id string, for continuation),
`x-default` (per-question default answers, §4.6), `x-max-tries` (int 1–2), `x-decision-id` (client
supplied idempotency key; if reused, the stored decision is returned, not re-asked).

### 3.3 Continuation and caching (supp §1 bullet 2) — scoped

| Ask | V2 | V3 | V4 |
| --- | --- | --- | --- |
| Every decision gets an id, returned in headers | **yes** | — | — |
| Look up a decision by id | **yes**: `GET /v1/systemone/decisions/{id}` returns the record (D6 schema) | — | — |
| Search by tag, date, session | **yes, tags and session only** (`x-tags`, `x-session`) | date and full-text search | — |
| Idempotent re-ask with `x-decision-id` | **yes** | — | — |
| Conversation continuation (multi-turn state) | **no** | `x-session` carries prior answers into `state`, with probabilities as soft evidence (R2 §2.4) | learned context pruning |
| Long-term caching | **no by default** | content-addressed cache keyed on canonical `{state, questions, prompt_version}`, TTL, opt-in `x-cache: "allow"` | cache with calibration-aware invalidation |

Why caching is off in V2: a cached decision answers the old state; decisions are re-asked when facts
change, and the feedback loop (D6) needs a fresh decision per event. The V3 gate is "≥ 14 days of
decisions and a measured repeat rate" (main.md §0). Storage of the state is the D6 record, not a
second cache.

### 3.4 Failure modes at the endpoint

| Failure | Response | Recorded |
| --- | --- | --- |
| Body not an object | 400 `{error}` (as `systemone.ts:99-103`) | no decision (it is never one) |
| Unknown model, not a route, not Clef, not a systemone provider | 400 `{error}` with the list of accepted models | refusal event |
| Every leg failed, default answer used | **200** with standard answers from the defaults; headers say `x-opencode-decision-leg: default` | decision record with `default_used=true` |
| Upstream timeout | 504 only when no leg could answer in time; otherwise the next leg answers | attempt rows per leg |
| Auth (no key for any leg in the pool) | 401 `{error}`, same wording family as `systemone.ts:233-234` | refusal event |

Default-answer 200 is a deliberate change from the door's 500 (`systemone.ts:264-280`): supp §4 says
"set a default answer", and a 200 with `default_used` is honest because the record says so.

### 3.5 Challenges to main.md (endpoint)

- **C2 — AD5 "byte-compatible" needs a precise meaning.** Today the façade refuses non-Clef models
  (`systemone.ts:102`), so "byte-compatible" cannot mean "same accepted models". This design reads AD5
  as: same request and response **shapes** for the standard keys; the accepted `model` set widens.
  Default: widen, and pin the old refusals in the parity suite as a documented change.
- **C3 — AD5 says the façade strips `x-` before a native call; it also needs to ignore unknown `x-`
  keys on the way in.** Without that, a client sending an envelope field the server does not know
  gets a 400. Default: ignore.
- **C4 — SDK response decode.** The fork's client requires `model` in the response (`system-one.ts`
  `Response`), which the door does not return (`systemone.ts:125-128`). The new façade returns
  `model` in the body (standard), so the client decodes. Recorded as a contract fix, not a rule change.

---

## 4. LLM-as-decider protocol (T12; supp §4)

### 4.1 Context and optional file scans (supp §4 bullet 2)

- The decider receives the **whole state up to its leg's context limit, uncompacted**. Compaction is
  not applied. If the state does not fit a leg's cap (`limit.context` on the target, §2.2), the leg is
  skipped as ineligible (the `limited` rule, `opencode.jsonc:3608-3609`), not truncated.
- A **quick file scan** is optional and bounded: up to `scan.max_reads = 3` file reads and
  `scan.max_chars = 20 000` per read, only through an allow-listed read tool. Scans are logged in the
  decision record (`scan_count`, `scan_chars`). Default: scans off (`scan.max_reads = 0`) for V2
  decisions, on for the executor's own runbook. Scans cost input tokens; they are measured, not assumed.

Option table (context use):

| # | Option | Pros | Cons |
| --- | --- | --- | --- |
| C-a | Full state, no compaction, leg cap decides (chosen) | Honest; the decider sees what the caller sent | Long states go to larger legs |
| C-b | Summarise state first | Fits small legs | A second model call that can drop the deciding fact |
| C-c | Truncate to the cap | Simple | Silent loss of evidence; the worst outcome for a decider |
| C-d | Unbounded tool scans | Thorough | Unbounded cost; not measurable |

### 4.2 Prompt: versioned and dated

Prompt id `decision-llm`, version `v1`, dated `2026-11-13`. The full text is in **Appendix A**. Every
record carries `prompt_id`, `prompt_version`, `prompt_date` (supp §9 bullet 1). The prompt is a file in
the repo (`.opencode/prompts/decision-llm.v1.md`, D5-S16) so a version bump is a diff, never an edit
in place. A changed prompt gets a new version; old decisions keep theirs.

### 4.3 Output schema (what "valid" means)

The LLM must emit one JSON object, nothing else, with:

```
{ "model": string (echo of the route id),
  "answers": { <question id>: Answer } }
Answer = choice:  { "type":"choice", "choice": <criteria key>, "probabilities": {key: p}, "confidence": p }
       | score:   { "type":"score",  "score": n, "probabilities": {"0".."k-1": p}, "confidence": p }
       | noul:    { "type":"noul", "noul": p }
p ∈ [0,1]; probability keys exactly the criteria keys (choice) or "0".."k-1" (score)
```

This is the standard answer shape (`system-one.ts` `Answer`), so the adapted reply goes out in the
standard body unchanged.

### 4.4 Validation and the invalid-JSON loop (supp §4 bullet 3)

**Failure taxonomy** (each class has its own remedy):

| Class | Detected by | Remedy |
| --- | --- | --- |
| refusal | provider refusal field, or the model's text starts with a refusal | no repair; next leg (refusals are not fixed by a second try) |
| truncation | `finish_reason` = length, or the JSON does not close | next leg (a second try with the same context will truncate again) |
| schema | JSON parse error, or missing key, wrong type, unknown key, `choice` not in criteria, score out of range | **one retry with feedback** (below) |
| business | probabilities outside [0,1] (not repairable), probabilities missing for a question, probability sums ≠ 1 | sum ≠ 1 with all values in [0,1]: **renormalise locally, count a repair, no retry**; other business errors: one retry |
| provider error | HTTP 5xx, 429, timeout | next leg (no decider-side retry) |

**Loop** (`max_tries = 2` total model calls per leg; a repair does not count as a try):

```
for leg in pool.legs (walk per §8):
  out = call(leg, prompt_v1, state, schema)
  for try in 1..max_tries:
    cls, errs = classify(out)
    if cls == ok: return answers
    if cls == business and sum_only(errs): renormalise; return answers   (repair)
    if cls in {refusal, truncation, provider}: break            (next leg)
    if try == max_tries: break                                   (next leg)
    out = call(leg, feedback_prompt(prompt_v1, schema, out, errs, hints(errs)))
next leg
... all legs exhausted -> default answers (§4.6), default_used = true
```

**Feedback message** (sent on the second try): the invalid output verbatim, the compact schema (not
the full prompt again), each validator error as a JSON pointer with the expected form, and one hint
line from a fixed table: "choice must be one of the criteria keys: [...]", "probabilities must sum to
1 over the criteria keys", "output must be a single JSON object with no prose". Practitioner consensus
(R2 §3.3 bullet 2); the second-try success rate is measured (M5), not assumed.

Why two calls, not three: R2 §3.4 gives a design guess that re-routing after one failed try is better
when second-try success is below ~50 %. Two calls is the brief's rule; the measurement decides whether
a leg should be dropped sooner (a later V3 change).

### 4.5 Options considered (retry policy)

| # | Option | Verdict |
| --- | --- | --- |
| R-a | No retry, next leg at once | Cheapest; wastes good legs on local errors |
| **R-b** | 2 total calls, feedback on the second, classes route the rest (chosen) | Brief's rule; cheap for local faults |
| R-c | Constrained decoding only (OpenAI strict, grammar) | Removes schema class where available; not available on every leg; chosen **where available** as a parameter, not as the policy (O6) |
| R-d | Up to 5 tries with growing context | Cost unbounded; error compounds |

### 4.6 Default answer (the last step)

If every leg fails, the decision returns **default answers**. The defaults resolve in this order:
per-question `x-default` in the request; the config `experimental.decision.default_answers` by
question id; else the type default:

- `noul`: `noul = 0` (the question is answered "no", recorded `default_used`, confidence 0),
- `choice`: the first criteria key, `probabilities` = `{first: 1, others: 0}`, confidence 0,
- `score`: the lowest level, same shape with confidence 0.

`default_used = true` and `confidence = 0` are always set, so a default is never mistaken for a
decision by downstream consumers (AD7, D6). A question with no default and no type default is an
error, returned as 500 with `{error}` (this should not occur; the config check requires a default for
every question type used).

---

## 5. Budget legs and the Cloudflare counter (Q3; supp §2 bullet 4)

### 5.1 What the allowance is

10,000 neurons per UTC day, reset 00:00 UTC; free plan fails requests at exhaustion; paid plan bills
overage with **no hard stop** (R2 §1.2; Cloudflare pricing page). Neuron price $0.011 per 1,000.
Q3 default: the executor keeps its own counter and does not use 429 as the stop.

### 5.2 Counter

Table `decision_budget(utc_day TEXT, provider TEXT, neurons INTEGER, PRIMARY KEY(utc_day, provider))`
in `opencode.db` (D5-S10). Neurons per decision are **estimated** from input tokens with the model's
published rate: `neurons = ceil(in_tokens × rate_per_Mtok / 1e6)` with the per-Mtok neuron rate from
R2 §1.2 (clef-flash 3,455; clef 21,818). Output tokens are free on Clef. Because Cloudflare reports no
usage on the door (`systemone.ts` records 0/0), `in_tokens` is the **pre-call estimate** (chars ÷ 3.5,
ceiling) and the row is marked `estimated=1`. The counter is corrected from the dashboard once a day
(manual in V2; automatic in V3).

### 5.3 Algorithm (disengage and pivot)

```
function cf_leg_open(now_utc, est_neurons):
  if not cfg.cloudflare.enabled: return CLOSED                  # flag off = leg absent
  used = counter(utc_day(now_utc), 'cloudflare')
  cap  = cfg.cloudflare.neurons_per_utc_day
  if used + est_neurons > cap: return CLOSED                    # disengage before the call
  if paid_cap and used + est > paid_cap: return CLOSED
  return OPEN

on leg attempt: if CLOSED -> record outcome 'budget-skipped', continue to next leg (pivot)
on success:     counter += est_neurons_actual_or_estimate
on 429/overage: record, never used to decide the cap
```

The `budget-skipped` outcome is distinct from `failed` and `walled-skipped` (§8). A budget-skipped
leg is not counted as a failure and does not trigger the failure-class logic.

### 5.4 Options considered (budget)

| # | Option | Pros | Cons |
| --- | --- | --- | --- |
| B-a | Let 429 / quota errors stop the leg | Zero code | Paid plan has no 429; overage bills silently; the record cannot say why |
| **B-b** | Own UTC-day counter, cap before the call, pivot (chosen) | Works on free and paid; reason is recorded | Estimate error; needs a daily correction |
| B-c | Dollar budget only (`$10/month` overage ceiling) | Matches Drew's wording | Neurons are the unit Cloudflare bills; conversion needed |
| B-d | Drop Cloudflare entirely | No budget code | Loses the free allowance Drew asked for |

Chosen B-b, with B-c as a **second cap on the same counter** (`paid_neuron_cap`, default null). Q3
default stands: the 10k/day allowance is the first cap.

### 5.5 Open: the $10 wording

Drew wrote "$10 (or whatever)". The verifiable allowance is 10k neurons/day ≈ $0.11/day at overage
rates (R2 §1.2), not a $10 credit. The design uses neurons. If Drew means a $10/month ceiling, it is
`paid_neuron_cap` ≈ 909 neurons/day equivalent (= $10 ÷ 30 ÷ $0.011 × 1000). Recorded as a RULING
candidate, default neurons.

---

## 6. Decision effort (T14; supp §6)

### 6.1 Two meanings, one config

- **Tie-break lean** (supp §6 bullet 1): low/medium/high add a very slight boost toward one end of the
  routing direction. Applied only inside a tie band.
- **Fallback pool choice** (supp §6 bullet 2): which pool serves when every decision leg fails. Set by
  `experimental.decision.fallback_pool` (§2.4).

Both come from `x-effort`. Absent `x-effort` means **no impact** (supp §3 bullet 3): no lean, and the
fallback pool is the config default `workhorse`. The UI does not change (supp §6: "avoid new
widgets"). The API and task records distinguish the two (`lean_applied`, `fallback_pool_used`).

### 6.2 Lean algorithm (precise)

Inputs: `probs` over the choice's criteria; `order` = `x-lean-order` (criteria keys from the low end to
the high end; default: criteria key order); `tier` ∈ {low, medium, high, absent}; `δ` = `lean.delta`
(0.05); `ε` = `lean.step` (0.01), with `ε ≤ δ` enforced by config check.

```
function lean(probs, order, tier):
  if tier is absent: return (probs, winner=argmax(probs), applied=false)
  n = len(order)
  ranked = sort(order, key = (-probs[k], index_in_order[k]))   # ties keep low→high order
  a, b = ranked[0], ranked[1]
  gap  = probs[a] - probs[b]
  if gap > δ: return (probs, a, applied=false)                 # outside the band: no lean
  target = {low: 0, medium: (n-1)//2, high: n-1}[tier]         # middle rounds toward low end
  pick = a if |idx(a)-target| <= |idx(b)-target| else b        # closer to the lean target; tie -> a
  p2 = copy(probs); p2[pick] += ε                              # additive, bounded
  winner = argmax(p2) with the same tie rule
  return (p2, winner, applied = (winner != a or pick != a))
```

Properties (checked in the guard, §6.4): the lean changes the winner only when `gap ≤ ε`; the lean
never moves a probability by more than `ε`; no call with `tier` absent changes anything; the result
is the same for any ordering of equal-probability criteria within the tie rule.

Why additive inside a band, not absolute or multiplicative (supp §6 "not an absolute amount, not a
plain multiplication"): an absolute override ignores the model's answer; a multiplier scales every
probability, which changes the ranking far outside the band and breaks calibration (R2 §2.5 bullet 4).
An additive ε only in a δ band is a tie-break and leaves calibrated probabilities untouched elsewhere.

Option table (lean):

| # | Option | Verdict |
| --- | --- | --- |
| L-a | Absolute: override the winner for `high` | Rejected: ignores the answer |
| L-b | Multiplier on all probabilities | Rejected: changes ranking far outside the band; breaks calibration |
| **L-c** | Additive ε toward the lean target, only inside a δ band among the top two (chosen) | Tie-break semantics, bounded, invariant-testable |
| L-d | Lean as a log-odds prior on every option (R2 §4.3) | Stays in the calibrated space; a V3 option once labels exist |
| L-e | Effort chooses the whole pool, no lean | Fallback-pool choice only; used in §6.3 |

### 6.3 Fallback pool choice

When every leg fails and `x-effort` is set, the **caller's** fallback is `fallback_pool[tier]` (§2.4):
low → `speed`, medium → `workhorse`, high → `summit`. This is the "which default is used when auto is
used" meaning of supp §6. It is a single lookup and is recorded as `fallback_pool_used`.

### 6.4 Dependent probabilities (supp §6 "dependent probabilities")

Chain rule: P(Q1..Qn) = ∏ P(Qi | Q<i) (R2 §2.4). V2 handles dependence the way TypeSafe documents
it: independent questions in one call; dependent questions in a second call with the earlier
**answers** placed in `state`, using the probabilities as soft evidence (R2 §2.4 warning), not the
argmax. Recorded: `call_index`, `depends_on`. CPT or Bayesian tables for ≤ 3 parents are V3 (D5-S21).
The "combination Q1+Q3 determines Q4" and "Q2+Q4 shape Q5" cases are carried by §7 clauses, which
compile the shift into the second call's `state` and into a bounded log-odds adjustment (§7.3).

---

## 7. Conditional clauses (T15; supp §7) — above the endpoint

### 7.1 Options considered (clause language)

| # | Option | Termination | Fit | Verdict |
| --- | --- | --- | --- | --- |
| K-a | CEL (Common Expression Language) | guaranteed (non-Turing complete) | typed, standard; cel-go and cel-python exist (R2 §4.1, cel.dev); **no TypeScript implementation verified in this pass** | Candidate for V3 once a TS or Python-sidecar runtime is verified |
| **K-b** | JsonLogic predicates + declarative effects, all JSON (chosen for V2–V3) | guaranteed (no loops) | JSON-native, lives in `x-clauses`, serialises with the request; JS implementation exists [M] (R2 §4.1) | Chosen: fits the JSON envelope and the TypeScript fork |
| K-c | Tiny custom DSL with a parser | ours | Smallest surface, but a parser and type checker to maintain; no tooling | Rejected: maintenance cost without a gain over K-b |
| K-d | DMN decision tables (FEEL, hit policies) | table-driven | Strong for small combinations (≤ 8 rows, R2 §4.2); explodes beyond three binary inputs | Used as a **review view** of the same rules, not as the engine (O13) |
| K-e | Templates only (Jinja/Handlebars) | Jinja loops | Good for rendered text, poor for logic (type-unsafe, string output) | Rejected for logic; used for rendering (§7.2) |
| K-f | OPA/Rego | bounded | Heavy; access policy, not soft preferences | Rejected |

Decision: **K-b for predicates and effects; §7.2 template for rendered text; K-a is the V3 gate.** The
tie-break between K-a and K-b is the JSON-native envelope and that no verified TypeScript CEL runtime
exists yet (a challenge to the R2 recommendation, which preferred CEL; recorded in §7.5).

### 7.2 Rendering template (the brief's "template string with variables")

Templates render **instruction text** only, never logic. A variable map exposes `answers.<qid>`,
`answers.<qid>.choice`, `answers.<qid>.probability`, and `state.*`. Syntax: `{{ answers.q1.choice }}`,
HTML escaping off, no loops, no helpers except `default` and `upper`. Sandboxed by construction (a
fixed helper set; no expression evaluation). The output is inserted into the question's `instructions`
of a later call (§6.4).

### 7.3 Clause shape

```jsonc
"x-clauses": [
  {
    "id": "c1",
    "when": { "and": [ {"==": [{"var":"answers.q1.choice"}, "yes"]},
                       {"==": [{"var":"answers.q2.choice"}, "no"]},
                       {"==": [{"var":"answers.q3.choice"}, "yes"]} ] },
    "then": [ { "set": { "q4": { "type":"noul", "noul": 0 } } } ]
  }
]
```

Effects (closed set, V2 accepts, V3 compiles):

| Effect | Meaning | Bound |
| --- | --- | --- |
| `set {qid: answer}` | Fix a downstream answer (applied after the call, marked `clause`) | the answer must type-check against the question |
| `default {qid: answer}` | Used only when the question is blank, deferred or missing | never overrides a real answer |
| `narrow {qid: [keys]}` | Restrict a choice's candidate keys (the "check only option 1" effect) | at least one key remains |
| `boost {qid, keys, strength}` | Log-odds add on the listed keys, `soft` = +0.2 nats, `strong` = +1.5 nats | per-option cap ±3 nats (R2 §4.3) |
| `rank {qid, order}` | Preferred order for ties (optional ranking) | ordering only, no probability change |
| `add_option {qid, key, text}` | Add a make-your-own option | key must be new |

### 7.4 Every supplement §7 example, encoded literally

Predicates use JsonLogic operators: `==`, `!=`, `>`, `and`, `or`, `!`, `var`, `missing`.

| # | Supp §7 wording | Encoding (the `when` / `then`) |
| --- | --- | --- |
| 1 | "If 1, 2, 3 are yes/no/yes → switch Q4 to no" | `when`: `and(q1==yes, q2==no, q3==yes)`; `then`: `set q4 = noul 0` |
| 2 | "If yes/no/no → Q4 is yes but Q5 is no" | `when`: `and(q1==yes, q2==no, q3==no)`; `then`: `set q4 = noul 1`, `set q5 = noul 0` |
| 3 | "If yes/yes/yes → Q4 is '1 and 2', Q5 is yes, Q6 is both" | `when`: `and(q1==yes, q2==yes, q3==yes)`; `then`: `set q4 = choice "1 and 2"`, `set q5 = noul 1`, `set q6 = choice "both"` |
| 4 | "if blank then this is X" | `when`: `missing(q1)`; `then`: `default q1 = choice X` (fills only the downstream rule, never a real answer, §4 of R2) |
| 5 | "if this and this and this then slight preference / strong preference for or against each" | `when`: `and(q1==yes, q2==yes, q3==yes)`; `then`: `boost q4 keys=[…] strength=soft` (or `strong`), sign by the criterion |
| 6 | "If this is for 1 and that is for 2, then for Q4, if 3 is yes, strongly boost options B and C" | `when`: `and(q1==1, q2==2, q3==yes)`; `then`: `boost q4 keys=[B,C] strength=strong` |
| 7 | "If deferred or not answered → prefer checking just option 1" | `when`: `or(q==deferred, missing(q))`; `then`: `narrow q = [option1]` |
| 8 | "If no, or a number greater than 100 → check all the boxes and insert into the make-your-own" | `when`: `or(q==no, q>100)`; `then`: `narrow q = all keys`, `add_option q key=make-your-own` |
| — | "optional rankings and other subclasses" | `rank {qid, order}`; the other subclasses are the same effects with different `when` |

Rules 1–3 are a row each in the DMN view (K-d). Clause 6's "then for Q4" is the second call of §6.4:
the clause fires after answers 1–3 return and its effect applies to Q4 within the same decision (the
caller's "if 3 is yes" is an answer, so the clause is evaluated after Q3, which is why §6.4's
two-call form is needed).

### 7.5 Superset rule and challenges

- `x-clauses` never reaches the endpoint; the executor compiles it (§7.6) and sends only
  `{state, questions}` (R2 §4.5). The standard request is unchanged (guard D5-S04).
- **Challenge C5 — R2 recommended CEL (R2 §4.2).** This design chooses JsonLogic for V2–V3 because
  the envelope is JSON and no TypeScript CEL runtime is verified. CEL stays as the V3 gate, recorded.
- **Challenge C6 — supp §7's "template string with access to variables" as the clause shape.** A
  template cannot type-check logic. The design keeps templates for rendering (§7.2) and gives logic a
  JSON form. This is the brief's shape with the logic part made checkable.

### 7.6 Compile algorithm and cycle check

```
compile(clauses, questions):
  build graph: clause -> reads(when vars) -> writes(then targets)
  topo-sort; a cycle among clauses is a compile error (cycle-error), never a silent drop
  for each clause in order: evaluate when on answers so far; apply then to the next call's state
     or to post-hoc adjustments (boost), never to the wire body
```

Cycle handling uses the same visited-set rule as the pools (§8): a clause already evaluated is not
evaluated again in the same decision.

---

## 8. Cycle-safe traversal (T17; supp §10; AD6)

### 8.1 Algorithm

One **visited set per call** shared by every expansion of the decision (chat pools, tier overlays,
decision pools; AD6). Depth-first, declared order:

```
walk(node):
  if node is a leaf:
      if node in visited:            record already-processed; return
      visited.add(node)
      if node is walled (100% walled door): record walled-skipped; return     # never attempted
      if node is budget-closed:       record budget-skipped; return
      attempt(node) -> success ends the walk; failure continues
  else (pool):
      if node in visited:             record cycle-skipped; return           # fully processed earlier
      visited.add(node)
      for child in members(node, ordered): if walk(child) == SUCCESS: return SUCCESS
```

Outcome vocabulary (distinct, and never merged):

| Outcome | Meaning | Counts as a failure? |
| --- | --- | --- |
| `attempted` / `success` / `failed` | a door was called | yes for `failed` |
| `cycle-skipped` | a pool already on the visited set (fully processed) | **no** |
| `already-processed` | a leaf already attempted via another pool | **no** |
| `walled-skipped` | door 100% walled; never attempted (standing rule) | **no** |
| `budget-skipped` | budget closed (§5) | **no** |
| `skipped-empty` | tier part with no members (§2.2) | **no** |

**Challenge C7 — AD6 applies the visited set to pools only.** The prototype found that a leaf
reachable through two pools is attempted twice, which violates "every reachable leaf at most once".
This design puts leaves on the same visited set, with the separate outcome `already-processed`
(not `cycle-skipped`, because nothing cycles). AD6 needs that sentence.

### 8.2 Prototype and oracle

`research/prototypes/traversal.py` (stdlib, run `python3 -I research/prototypes/traversal.py`). On
600 random graphs (seeds 0–599; up to 12 pools, up to 8 leaves, 1–4 children per pool drawn from all
nodes, so self-loops and mutual references occur) it checks:

1. **Termination**: visits ≤ pools + edges + 1.
2. **At-most-once**: no leaf attempted twice, no pool expanded twice.
3. **Equivalence**: when no success occurs, attempted ∪ walled-skipped = brute-force reachable leaves
   (BFS with no visited set); expanded pools = reachable pools.
4. **Success prefix**: when a success occurs, the walk ends on that success leaf and the leaf is
   reachable.
5. **Cycle outcome**: every `cycle-skipped` names a pool that was expanded; no walled leaf is attempted.

Result at the time of writing: `PASS 600 random graphs … invariants held`. Port the same oracle to the
TypeScript fork in D5-S02 as a property test (`fast-check`-style or the fork's existing PBT harness),
with the brute-force reachable set as the oracle.

### 8.3 Options considered (traversal)

| # | Option | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| T-a | Depth limit only | Trivial | Cuts valid long paths; misses duplicates | — |
| **T-b** | Visited set, DFS, distinct `cycle-skipped` (chosen) | Terminates; at-most-once; honest outcomes | Mutual references are not resolved, only skipped | DFS grey/black (R2 §6) |
| T-c | Grey/black marks (cycle detection with a stack) | Detects cycles explicitly | More state; same result for failover | CLRS DFS |
| T-d | Datalog semi-naive fixpoint | Handles mutual references | Heavy; wrong model for ordered failover | Datalog (R2 §6) |
| T-e | Nix module fixpoint (`lib.fix`) | Values across cycles | Infinite recursion when forced (R2 §6); V4 idea | nixpkgs `fixed-points.nix` |

T-b is the chosen V2 algorithm. T-c adds nothing observable for ordered failover; T-d and T-e are V4
research (supp §10: "mutual references might be supported later"), D5-S24.

---

## 9. Measurement: native vs cheap LLM, per provider

### 9.1 Design

- **Labelled set**: 300–500 historical decisions with outcome labels from PRs and issues (D6 outcome
  schema). Labels are a set (supp §5), converted to one **correct/incorrect** per decision by a
  pre-registered rule written before any scoring (AD9): "correct" = the decision's routed category or
  answer was not reverted and no rework label applies within 14 days.
- **Arms**: native Zen `jev-1.13-free`; native TypeSafe `jev-latest` (when a key exists); DeepSeek Flash
  off-peak; gpt-5-nano; Luna 100k `#medium` (the last resort, as the reference).
- **Method**: each decision replayed to each arm under the same prompt version and schema; each item
  permuted 3× on choice order (M3); 10 repeats on a 20-item subset for variance (M4).
- **Metrics** (R2 §7.2): accuracy vs label (M1); ECE and Brier after Platt fit, cross-validated (M2);
  order flip rate (M3); invalid rate and second-try success (M5); TTFT and total P50/P95 measured by us
  (M6); realised cost per 1k from usage fields (M7); UTC-day counter (M8); context fit (M9).
- **Decision rule** (R2 §7.1, pre-registered): choose native if cost per correct decision is lower than
  the best LLM option **and** ECE ≤ 0.05 after recalibration **and** invalid rate ≈ 0 **and** order flip
  rate small. Choose the LLM if its accuracy is ≥ 5 points higher at ≤ 2× the native cost per correct,
  or native is unavailable. Cost per correct = Σ cost ÷ Σ correct, with a Wilson 95 % interval on
  accuracy; no decision is taken if the interval spans the threshold.
- **Sample size**: with n = 300 per arm, a 5-point accuracy difference at 80 % power needs ≈ 0.1 SD;
  the design treats 300 as a floor and 500 as the target, and reports `n` with every number.

### 9.2 Slices

D5-S15 (trial, V2) runs it. Gate: the decision record (D6) has ≥ 300 labelled decisions, or the
trial waits. The trial is pre-registered with hypothesis, margin, review date and revert condition
(AD9).

---

## 10. The executor (T11; supp §3)

### 10.1 Formats (define them — the brief says "two or three main formats")

The executor accepts **three** input formats; the normaliser maps all to one internal form.

**F1 — native systemone** (the standard body):

```json
{"model":"opencode-route/decision",
 "state":"I was charged twice…",
 "questions":{"department":{"type":"choice","instructions":"…","criteria":{"billing":"…","sales":"…"}},
              "refund":{"type":"noul","instructions":"Is the customer asking for a refund?"}}}
```

**F2 — x-extended envelope** (F1 plus envelope keys; standard keys unchanged):

```json
{"model":"opencode-route/decision",
 "state":{"request":"…","history":[…]},
 "questions":{…F1 questions…},
 "x-effort":"medium","x-lean-order":["scout","workhorse","summit"],
 "x-clauses":[…§7.3…], "x-tags":["auto","category"], "x-session":"ses_…",
 "x-default":{"refund":{"type":"noul","noul":0}}, "x-decision-id":"dec_…"}
```

**F3 — compact question list** (for skills and humans; the executor expands it):

```json
{"state":"…",
 "ask":[
   {"id":"department","choice":["billing","technical","sales"],"q":"Which team?"},
   {"id":"urgency","score":["normal","prompt","blocking"],"q":"How urgent?"},
   {"id":"refund","yn":true,"q":"Is the customer asking for a refund?"}]}
```

F3 expansion rules: `choice` array → criteria with the strings as keys and the key as the text;
`score` array → criteria list; `yn` → `noul`; `id` is the question id; `q` is the instructions.
A choice criterion given as `{key: text}` object is accepted in F3 as well.

### 10.2 Normaliser → output validator → ID

- Normalise each format to the F2 internal form (with `x-` fields optional). Reject any other shape with
  `invalid-input` and the expected list of formats (400-style error in the executor's own output).
- Validate the input: ≤ 255 choice criteria, 2–10 score levels, unique question ids, and a state that
  fits the chosen pool's smallest cap (else the executor warns and routes to a larger pool).
- Call `/v1/systemone` (§3). The response is **re-validated** against the question set (the same rules
  as `evaluation-client.ts:46-83`) before it is returned.
- Output: the standard `{model, answers}` plus `decision_id`, `leg`, `pool`, `default_used`,
  `confidence`. Probabilities are rounded to 2 decimals (the `EvaluationRounding` rule, R2 §0).
- **ID**: `dec_` + ULID, generated by the executor before the first call. The ID is the idempotency key
  (`x-decision-id` reuse returns the stored decision). Caller and callee gauges (0–5 correct and wrong,
  supp §3 bullet 3) are append-only rows referencing the ID (AD7; D6 owns them).

### 10.3 The mcpx tool, the skill, the runbook

- **mcpx tool**: `decision.run({input})`, `decision.get({id})`, `decision.feedback({id, who, correct, wrong})`
  exposed through the existing mcpx namespace pattern (`.opencode/instructions/mcpx.md` lists namespaces;
  a new `decision` namespace is added by config, D5-S06).
- **Skill**: `decision-run` (SKILL.md under the repo's skill directory, D5-S06), with the three formats,
  one worked example each, the default-answer rule, and the rule "a decision is an event: never edit a
  stored decision, append a feedback row".
- **Runbook**: `doc/runbooks/decision-run.md` (D5-S06), the operator steps: check the Cloudflare counter,
  check the Zen promo flag, read the last ten `decision_record` rows, what to do on `default_used` spikes.

### 10.4 Options considered (executor shape)

| # | Option | Pros | Cons |
| --- | --- | --- | --- |
| X-a | Only a skill | No code | Not callable from OpenChamber's server; no validation |
| X-b | Only an mcpx tool | Callable everywhere through mcpx | Agents need the format knowledge; no runbook |
| **X-c** | Tool + skill + runbook, one contract (chosen) | Each reader gets what it needs; one validator | Three artefacts to keep in step (one slice each) |
| X-d | Only a library in the fork server | Typed | Not reachable by sessions; OpenChamber already calls the endpoint itself |

---

## 11. Records, IDs and failure modes (summary; D6 owns the schema)

Every decision writes, through D6, `decision_id`, `prompt_id/version/date`, `pool`, `route_chain` (each
leg with its outcome class: `success`, `refusal`, `truncation`, `schema`, `business`, `provider`,
`walled-skipped`, `budget-skipped`, `cycle-skipped`, `already-processed`, `skipped-empty`),
`final_leg`, `default_used`, `repairs`, `tries`, `tokens`, `neurons_est`, `ttft_ms`, `total_ms`,
`lean_applied`, `fallback_pool_used`, `scan_count`. Gauges are append-only.

Failure modes that matter most: a decision that answers from the default looks like a decision
(prevented by `default_used` and confidence 0); a cycle recorded as failure makes a walled provider look
broken (prevented by the outcome vocabulary); a budget stop recorded as a 429 hides the cap (prevented
by §5.3).

---

## 12. V3 and V4 layers

### V3 — measured and automatic (gate: ≥ 14 days of decision events and the §9 labels)

- **Calibration**: Platt or temperature scaling per decision type and per leg, versioned, raw and
  calibrated stored together (R2 §2.3). Gate: ≥ 300 labels.
- **Clause compiler**: K-a CEL runtime if verified, otherwise keep K-b; compile `x-clauses` into the
  second call's state and post-hoc log-odds (R2 §4.3).
- **Dependent probabilities**: CPT for ≤ 3-parent groups (R2 §2.4, O15).
- **Cache and continuation**: opt-in content-addressed cache, `x-session` conversation continuation.
- **Automatic Cloudflare counter correction** from the dashboard.
- **Lean as a log-odds prior** (L-d) once calibrated.
- **Local decision model** (R2 O22) once 300+ labels exist.
- **Trial-driven leg order**: the §9 result rewrites the tier order through a named trial, not by hand.

### V4 — learned and adaptive (gate: V3 metrics show the hand-tuned order is the limit)

- Bandit allocation of decision traffic **inside the trial's safe set** (AD9), with propensities logged
  so IPS is possible (R2 §5.1).
- Mutual references between pools via a fixpoint (T-d or T-e), only if a real case needs values
  computed across a cycle (supp §10).
- Calibrated self-switching informed by decisions (AD10).
- OpenAI Decisions API as a pool member once it has a public contract and price (R2 O24; "wait").

---

## 13. Worked examples (V1 pools and models)

### Example 1 — routine triage, free leg answers

Caller: OpenChamber Auto, `model: opencode-route/decision`, `x-effort` absent, 3 questions (department
choice, urgency score, refund noul), state 412 tokens.

1. `decision` → `decision-free` → `decision-native-free`: target 1 is `opencode-zen/jev-1.13-free`,
   pass-through (F1 body). Attempt 1 returns 200, all answers valid, probabilities sum to 1.
2. Outcome `success`, leg `zen-jev-free`, `tries 1`, `repairs 0`, `default_used false`.
3. Response headers carry the decision id; body is the standard `{model, answers}`.
4. Target 2 (`cloudflare`) was never reached: the flag is off, so the leg is absent. No `budget-skipped`
   row is written, because a flag-off leg is not in the list (§2.2).

### Example 2 — free leg fails, LLM part answers with one repair, lean applies

Same caller, `x-effort: "high"`, `x-lean-order: ["scout","workhorse","summit"]`, choice question
`route` with probabilities {scout 0.41, workhorse 0.37, summit 0.22}.

1. Zen answers 503 → `provider`, next target in the same tier. `decision-native-free` exhausts.
   `decision-llm-free` is **empty** → `skipped-empty`.
2. `decision-native-plan` empty → `skipped-empty`. `decision-llm-plan`: DeepSeek Flash off-peak
   (`deepseek-flash-plan#low`, window `invert` true at the current UTC time) answers. Try 1: JSON has
   `"confidence": 1.4` → `business` (not a sum error) → one retry with feedback.
3. Try 2 valid. Lean: gap = 0.41 − 0.37 = 0.04 ≤ δ (0.05). Target for `high` = index 2 (summit).
   Between scout (0) and workhorse (1), the one closer to 2 is workhorse. `probs[workhorse] += 0.01`
   → 0.38 vs 0.41: winner stays scout. `lean_applied = true`, winner unchanged.
4. Had the gap been 0.005, the winner would flip to workhorse, and `lean_applied` would be recorded as
   a flip. The record says which.

### Example 3 — walled leg, cycle, and the last resort

A pool `decision-payg` contains `decision-native-payg` and a target that points back at `decision`
(a mistaken config). Walk: `decision` (visited) → … → `decision-payg` → `decision` again → `cycle-skipped`
(not a failure, not attempted), then the walk continues to `decision-native-payg`. The OpenRouter leg is
100 % walled (standing rule) → `walled-skipped`, never attempted. TypeSafe has no key → `failed` with
class `auth`, and the walk continues. All PAYG legs fail → Luna 100k `#medium` answers (`final_leg
luna-plan#medium`). Its record shows exactly which legs were skipped and why.

---

## 14. Risks and open questions (each with the default I chose)

| # | Question | Default chosen | Why / where applied |
| --- | --- | --- | --- |
| Q-a | Is Jev a Command Code model? (supp §0) | No, per code; Jev is TypeSafe / Zen / OpenRouter / Vercel. The design does not depend on Command Code. | baseline Q5; §2.3 |
| Q-b | Is "Luna 100k" `luna-plan#medium` or OpenAI `gpt-6-luna`? | `luna-plan#medium` with `limit.context 100000`, the `limited` pool's leg (Q4). | §2.2 last target |
| Q-c | Is the Zen promo the "opencodego free" in brief §1? | Yes, most plausibly (Q2/Q5). | §2.3 row 1 |
| Q-d | Clef: revive opencode#1651? | **RULING for Drew.** Default: leg behind flag, off. Recommendation: revive only as a budget-capped leg after the §9 comparison. | §2.2, §5 |
| Q-e | "$10 included" meaning | Neurons (10k/day), with optional `paid_neuron_cap` (RULING candidate). | §5.5 |
| Q-f | OpenAI admission threshold (quota-% dollars) | Admit only if measured Δquota-% per decision ≤ that of `deepseek-flash-plan` off-peak. Formula in §6.5 below. | §2.3 row 11 |
| Q-g | Two tries = two total calls or one retry? | Two **total** calls (1 + 1 retry), as §4.4. | §4.4 |
| Q-h | Default answers for `noul` | `noul = 0`, confidence 0, `default_used`. | §4.6 |
| Q-i | Lean numbers δ = 0.05, ε = 0.01 | Design guesses; tuned only by trial (AD9). | §6.2 |
| Q-j | Image input for Jev 1.14 | Text only until verified on the pinned version (R2 open Q). | §2.3 |
| Q-k | Cached decisions | Off in V2. | §3.3 |
| Q-l | Groq/Cerebras "super-fast free" | Not in V2 order until M5 passes. | §2.2 |
| Q-m | Who answers the feedback gauges? | Caller and callee, 0–5, append-only (supp §3). | §10.2, D6 |

Risks: free promos end (Zen promo flag hard-coded); Cloudflare Paid has no stop (§5); façade
grammar widening breaks a parity test (C2); LLM decider compounding error (R2 §2.4); measurement
under-powered on rare labels (§9.1 floor 300).

### 6.5 OpenAI admission — quota-% dollar conversion (Q-f)

The plan's window quota is measured in percent (`route_usage`, quota analytics, `plan_info` quotas).
Define for a subscription with fee `F` (USD per month), window length `W` hours, billing period `P`
hours (720 for a month):

```
usd_per_pct(window) = F × (W / P) / 100          # dollars of subscription capacity per 1 % of one window
pct_per_decision    = Δquota_pct over N decisions / N     # measured from route_usage for that leg (≥ 14 d)
usd_per_decision    = pct_per_decision × usd_per_pct(window)
admit if usd_per_decision ≤ usd_per_decision(deepseek-flash-plan off-peak)
```

Worked check with the formula only (numbers are illustrative, not measured): F = $200, W = 5 h,
P = 720 ⇒ usd_per_pct = 200 × 5/720 / 100 = $0.0139. If a gpt-5-nano decision costs 0.02 % of the
window, usd_per_decision = $0.000278; that is admitted only if the DeepSeek off-peak figure is at least
that. Until `pct_per_decision` is measured, the OpenAI leg stays out of the list (`deferred`). Recorded
as a V2 formula with a measured input, not a price.

### Other challenges to main.md

- **C1 — AD5 leg order.** "free → plan → PAYG with a burn-first budget leg" is satisfied: the Cloudflare
  leg is a *free* (included) leg and sits first in `decision-free`, not as a separate burn-first stage.
  No change needed; recorded so the reader does not look for a fourth tier.
- **Note on AD7.** The decision record lives in `opencode.db` (D6); D5 only fixes the IDs and the
  header names. No conflict.

---

## 15. Statement coverage

| Statement | Section | Slice |
| --- | --- | --- |
| supp §0 Jev = Command Code decision model (premise) | §1 fact, §14 Q-a | D5-S26 (AD note) |
| supp §0 clef and jev are decision models on the systemone schema | §1, §3.1 | D5-S03 |
| supp §1 bullet 1 all conversation endpoints accepted; pass-through or adapt | §3.1 | D5-S03 |
| supp §1 bullet 2 better protocol; long-term caching; continuable by id; search by tags | §3.3 | D5-S27, D5-S22 |
| supp §1 bullet 3 accept model endpoints for raw pool and provider use | §3.1 | D5-S03 |
| supp §2 bullet 1 two taxonomies → one shape | §2 | D5-S07 |
| supp §2 bullet 2 parts build into one default decision pool | §2.2, §2.4 | D5-S01, D5-S07 |
| supp §2 bullet 3 free first, then plan, then PAYG; compare cost and capability | §2.3 | D5-S07, D5-S15 |
| supp §2 bullet 4 Cloudflare included usage burned, disengage, pivot | §5 | D5-S10 |
| supp §2 bullet 5 OpenAI option if cheap in quota-% dollars | §6.5 | D5-S13 |
| supp §2 bullet 6 per-provider native vs cheap LLM answered with data | §9 | D5-S14, D5-S15 |
| supp §2 bullet 7 local decision models | §12 | D5-S23 |
| supp §3 bullet 1 executor skill / runbook / mcpx tool; formats; valid output | §10 | D5-S05, D5-S06 |
| supp §3 bullet 2 config sets the decision default to the decision pool | §2.4 | D5-S01, D5-S07 |
| supp §3 bullet 3 every decision has an ID; caller and callee gauges 0–5 | §10.2, §11 | D5-S05 (D6 gauges) |
| supp §3 bullet 4 decision effort settable, no impact by default | §6.1 | D5-S11 |
| supp §4 bullet 1 small not used; Luna 100k last resort; fast free LLM candidates | §2.2, §2.3 | D5-S07, D5-S14 |
| supp §4 bullet 2 context used without compaction; quick file scans | §4.1 | D5-S08, D5-S16 |
| supp §4 bullet 3 invalid JSON: feedback, two tries, next leg, default answer | §4.4, §4.6 | D5-S08, D5-S09 |
| supp §5 feedback capture; linked outcome labels | §11 (summary; schema in D6) | D5-S05 |
| supp §6 tie-break lean vs fallback pool | §6.1–§6.3 | D5-S11, D5-S12 |
| supp §6 dependent probabilities | §6.4 | D5-S21 (V3), D5-S17 (V2 sequencing) |
| supp §7 conditional clauses above the endpoint | §7 | D5-S17 (V2), D5-S18 (V3) |
| supp §7 all eight examples encoded | §7.4 | D5-S19 |
| supp §8 PM-stack review surface | out of D5 (D6) | D6 slices |
| supp §9 decision record: prompt version and date; tokens; time; perspective; invalid-JSON path; final leg | §11 | D5-S05, D6 |
| supp §10 cycle-safe traversal; visited-pool skip | §8 | D5-S02 |
| supp §10 mutual references later (fixpoint) | §12 V4, §8.3 | D5-S24 |
| brief §1 wire decision models, free then paid then Cloudflare | §2.3 | D5-S07, D5-S10 |
| brief §2 self-switching to category or route pools (decision part) | §2.2, §10.3 | D5-S06 |
| brief §4 xhigh / max / ultra / fast → high (decision effort alias) | §6.1 (D1 aliasing) | D5-S11 |
| brief §8 DS Flash last resort in other pools (decision use) | §2.3 row 5 | D5-S07 |
| AD5 binding | §2, §3.2, §10 | D5-S01, D5-S03, D5-S04 |
| AD6 binding (+ challenge C7: leaves on the visited set) | §8 | D5-S02, D5-S26 |
| AD7 binding | §10.2, §11 | D5-S05 |
| T9 endpoint, T10 pools, T11 executor, T12 decider, T14 effort, T15 clauses, T17 traversal | §3–§8 | as listed above |

## Appendix A — decision-llm prompt v1 (dated 2026-11-13)

```
[decision-llm v1 · 2026-11-13]
You are the decision leg of OpenCode's decision system. You are not a chat assistant.

Input: one JSON object with "state" (the context, may be long) and "questions"
(each with "type": "choice" | "score" | "noul", "instructions", and "criteria").

Task: answer every question. Use the state. You may ask for at most 3 file reads
when — and only when — a question cannot be answered from the state; each file
read is at most 20000 characters. Do not guess file contents.

Output rules (strict):
1. Output ONE JSON object and nothing else: no prose, no markdown fence.
2. Shape:
   {"model": "<the route id you were given>",
    "answers": { "<question id>": <answer> }}
3. Answer by type:
   - choice:  {"type":"choice","choice":"<one criteria key>",
               "probabilities":{"<every criteria key>": p, ...}, "confidence": p}
   - score:   {"type":"score","score": <number 0..levels-1>,
               "probabilities":{"0":p,"1":p,...}, "confidence": p}
   - noul:    {"type":"noul","noul": p}      (p = probability the answer is yes)
4. Every p is a number in [0,1]. Probabilities over a question's options sum to 1.
   "choice" must be exactly one of the criteria keys. Do not add keys.
5. If a question is unanswerable from the state, still answer it, with confidence
   low (≤ 0.3), and do not invent facts.
6. Dependent questions: if the state contains an earlier answer, weigh it as
   evidence, not as a certainty.

If your last output was rejected, you will receive: the rejected output, the schema,
and the validator errors with JSON paths. Fix exactly those errors. Output the
corrected JSON object only.
```

Length: about 1,800 characters. Versioning: `decision-llm` `v1` · 2026-11-13. Changing any rule
produces v2 with a new date. Records store the version (§4.2).

---

## Final checks (drafter)

- Files: `designs/D5-decision-system.md` (this file), `slices/D5-slices.json`,
  `research/prototypes/traversal.py` (run and passing).
- No commits, no pushes, no issues filed.
- Detached: no `question` tool used; open questions recorded in §14 with defaults.
