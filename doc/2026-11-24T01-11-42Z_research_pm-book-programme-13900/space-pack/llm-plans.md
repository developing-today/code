# Project Management Space — LLM plans: highlights

> Pack file 6 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Source: `survey/llm-evidence.md` (lane LLM-EVIDENCE, assembled
> 2026-11-23 ~18:34–18:50 CST). **Every figure below carries its date, from
> that pack. No figure is presented as current. Stale figures are labelled
> stale.** The pack's §5 rot rules apply to this file: quota percentages rot
> in hours (a read older than 60 minutes cannot gate a dispatch); fees rot in
> weeks–months; tier membership rots with every config PR; benchmark results
> are historical facts whose *inferences* rot.
>
> Source keys (all read 2026-11-23 unless a row says otherwise): **REG** =
> provider-quota-registry.md (plan-facts transcribed 2026-11-23 from Drew's
> 2026-11-07 dictation; stewardship reads through cycle 471, 2026-11-23) ·
> **EFF** = model-efficiency-plan-2026-10-25.md (dated 2026-10-25) · **FREE**
> = free-census-2026-11-23.md (dated 2026-11-23) · **GUIDE** =
> provider-model-effort-guide.md (pools verified 2026-10-25; its pool-state
> snapshot is 2026-10-25 and stale wherever cited) · **EVAL** =
> `~/workspace/decision-eval/` · **I13677** = code issue #13677 + 15 comments,
> read 2026-11-23 · **LAPTOP** = deployed `opencode.jsonc` read on the laptop
> 2026-11-23 18:41:54 CST (279,445 bytes, mtime 2026-11-23 17:10:53 CST).

---

## 1. Plans table — condensed (30 rows, as in the evidence pack)

Fees/windows are from REG's plan-facts block unless the row names another
source. A `limit: null` in plan-facts means **no established fact exists —
never guess a limit**.

| # | Provider / plan (pool) | Fee + window (dated) | State at last dated read |
|---:|---|---|---|
| 1 | OpenAI plan — main door `_openai.main` (drewrypope); final-reserve pool | $100/mo, 168h (REG plan-facts, transcribed 2026-11-23 from 2026-11-07 dictation) — **superseded in part:** upgraded $100 → **$200/mo** per Drew 2026-11-10 15:38 CST (REG, code#8309) | Weekly 59%, resets 2026-11-24T21:23Z (REG cycle 471, measured 2026-11-23T12:15Z) |
| 2 | OpenAI plan — alt door `_openai.alt` (popesdrew, student); ordered FIRST in `openai-plan` since 2026-11-22 (PR #12731) | $0/mo, 168h (REG plan-facts, 2026-11-23) | Weekly 100% CLOSED, resets 2026-11-23T18:49Z (REG cycle 471, measured 2026-11-23T12:15Z) |
| 3 | OpenAI `openai-payg` API org + data-sharing complimentary tokens | PAYG (manual only, never routine) + daily complimentary buckets: Large group 250K/day (Build tiers 1–2) → 1M/day (Launch/Grow); Small group 2.5M/day → 10M/day; resets 00:00 UTC; $5 minimum prepaid purchase (OpenAI Help Center art. 10306912, fetched 2026-11-23, I13677) | Enrolment approved by Drew 2026-11-23 ("any and all data"); state assumed in place per Drew's 2026-11-23 statement, **not API-verifiable** (I13677, 2026-11-23) |
| 4 | Anthropic / Claude Max (`claude-max`, `_claude` pool); Fable 7d window metered separately | $100/mo, 168h, limit null (REG plan-facts, 2026-11-23) | **UNMEASURED** at last read: gatherer error "Rate limited", last measured 2026-11-23T03:00Z carrying 5h 100% (past its 04:50Z reset) / 7d 32% / Fable 28%, resets 2026-11-25T19:00Z (REG cycle 471) — do not cite carried figures as state |
| 5 | Meta Muse API **High Usage plan** — ONE pool (the only Meta pool) | $15/mo, 168h, limit null (REG plan-facts, 2026-11-23) | **STALE:** freshest console read 2026-10-27 00:03 CDT (27 days stale at pack time): 5h 1.02%, weekly 8.83%; latest gatherer state stale, freshest 2026-11-21T04:45Z weekly 100% resetting 2026-11-23T00:00Z (now past); new window's first clean read had NOT landed (REG cycle 471) |
| 6 | OpenCode Go — workspace drewrypope (one shared bucket per workspace, NOT per-model) | $10/mo per workspace (two workspaces at $10/mo each — REG plan-facts note, 2026-11-23); plan-facts row: 720h, limit 60, calendar reset | Monthly 100%, resets 2026-12-03 (REG cycle 471, measured 2026-11-23T12:15Z) |
| 7 | OpenCode Go — workspace popesdrew (replaced retired Lab workspace 2026-11-03, PR #5057) | $10/mo (REG plan-facts note, 2026-11-23) | Monthly 100%, resets 2026-12-03 (REG cycle 471, measured 2026-11-23T12:15Z) |
| 8 | OpenCode Go — workspace developing-today-0 | No fee row in plan-facts — fee not established | Monthly 100%, resets 2026-12-08 (REG cycle 471) |
| 9 | OpenCode Go — workspace 0x111dev (only one actively burning at last read) | No fee row in plan-facts — fee not established | 5h 64% / weekly 66% / monthly 83%, monthly resets 2026-12-18 (REG cycle 471, measured 2026-11-23T12:15Z) |
| 10 | ClinePass (Annual) (`cline-pass`) | $79.92/year (REG plan-facts, 2026-11-23); dashboard corroboration 2026-10-28: $79.92/yr + $2.60 fee, renews Oct 5, 2027 (REG cycle 96) | Monthly 100% CLOSED (resets 2026-12-06T14:40Z), weekly/5h 0% (REG cycle 471) |
| 11 | Open Design Go (`opendesign`) — token provider only, no harness | $84/year, 168h (REG plan-facts, 2026-11-23); windows $10 per 5h + $30 per 7d rolling (EFF, 2026-10-25) | 7d 100% CLOSED, resets 2026-11-24T11:44Z (REG cycle 471) |
| 12 | Command Code Go (`_commandcode` / CLI-proxy door; no direct API on this tier, bridge on :18748) | $1/mo → $10 credits/mo, 720h (REG plan-facts, 2026-11-23) | Monthly 99.51% (REG cycle 471); freshest console read 2026-10-27 10:05 CDT — **27 days stale**, re-reads blocked on sign-in (REG) |
| 13 | Mistral Education (`mistral` + `vibe-direct`): Included API $15/mo + Included Vibe Code $150/mo, both reset the 1st (REG correction 2026-11-05) | $57.50/year, 720h, limit 15, calendar reset (REG plan-facts, 2026-11-23) | API 100% + Vibe 100%, reset 2026-12-01; **freshness stale — measured 2026-11-16** (REG cycle 471) |
| 14 | Antigravity quotas (`antigravity-direct`; `_antigravity` in switchboard) | **No plan-facts row — plan identity/fee not established** (REG, 2026-11-23) | Gemini + Claude/GPT weekly 1.64% each, resets 2026-11-30T01:41Z — healthy (REG cycle 471). Caveat: `_antigravity` is in deployed `disabled_providers` while reads are healthy (standing finding since cycle 459; LAPTOP confirmed 2026-11-23 18:41 CST) |
| 15 | Google AI Studio FREE tier (`google-free` → `gemini-free-plan`): per-model-per-project chat quota ×4 project aliases; 5 chat models wired; meters requests/day, not tokens; free-tier content marked "Used to improve our products: Yes" | $0 (FREE §1a/§2, verified 2026-11-23) | Config-recorded live probes 2026-11-16: Flash/Lite models 200 on every project; Hydrogen 200 on 3 of 4 (FREE, 2026-11-23) |
| 16 | OpenCode Zen free models (`opencode` provider, `zen-plans` route): 11 free chat models, unquantified quota, "limited time" | $0 (FREE §2, verified 2026-11-23) | Seated as last-resort legs (limited context). Roster rotates without notice: Nemotron 3 Ultra Free dropped from Zen 2026-11-22 (FREE §4) |
| 17 | NVIDIA NIM free tier (`nvidia-free-plan`): up to 40 RPM personal maximum, no per-token billing, no published daily cap | $0 (FREE §2, fetched 2026-11-23: 83-model public catalog) | Our account's live list is only **32 ids**, 15 probed entitled for chat (2026-11-23, I13677); plan seats 12 entitled general chat models round-robin, free-first, weights 3 (PR #13777 merged, 2026-11-23) |
| 18 | Groq free plan (`groq-free-plan`): gpt-oss-120b / gpt-oss-20b / Qwen3.8-27B at 200K TPD each (~600K TPD total); TPM caps 7–8K are the binding choke — chat seats are tiny-prompt only | $0 (FREE §2, fetched 2026-11-23) | Direct probes 3/3 seated chat models HTTP 200 (2026-11-23, I13677). Plan is in no tier route (FREE §5) |
| 19 | OpenRouter `:free` tier (`openrouter-free-plan`): 15 models at $0/$0 | $0; cap 50 requests/day as held, rising to 1,000/day after a one-time $10 lifetime top-up (**not authorized, not made**) (FREE §2, fetched 2026-11-23) | Account-level 429 at probe time 2026-11-23 (limit 50, remaining 0, resets 2026-11-24T00:00Z); post-reset through-route probe completed but was **not** OpenRouter-served — see evidence B6 |
| 20 | Z.ai free Flash models (`glm-flash-free-plan`): GLM-4.7-Flash + GLM-4.5-Flash, $0 in/out permanently, 200K ctx, no published numeric cap | $0 (FREE §2, fetched 2026-11-23) | Seated free (FREE §2, 2026-11-23) |
| 21 | EXAONE free key (`exaone-free-plan`): one free model on 30-day platform keys | $0 (FREE §2/§4, 2026-11-23) | Current key issued 2026-11-16, **expires 2026-12-16**; rotation check fires Thu 2026-12-10 ~10:40 CST (I13677, arranged 2026-11-23) |
| 22 | Kilo Gateway `:free` (`kilo-free-plan`): keyless free lane, 200 requests/hour per IP; some free upstreams log prompts — non-sensitive volume only | $0 (Kilo Gateway docs fetched 2026-11-23, I13677; live probe HTTP 200 / $0) | `kilo-auto/free` seated in scout + background (PR #13691, merged 2026-11-23) |
| 23 | Command Code free shelf (`commandcode-free-plan`): 5 free-named catalog models | $0 (FREE §2, 2026-11-23) | Provider was disabled at census read → re-enabled and seated free-first in scout + background by PR #13691, merged 2026-11-23 (I13677) |
| 24 | OpenAI free alias (`openai-payg-free`): same-model free/paid split over row 3's org; alias catalog = gpt-6-luna, gpt-6-astra, gpt-6-sol; `wallScope: "account"`; free leg weight 3 ahead of paid tail 0.1 | $0 leg over the PAYG org (I13677, PR #13836 merged `7367db1040c1`; PR #13838 merged `1b969709c029`; live read-back 2026-11-23) | Present in deployed config (LAPTOP grep, 2026-11-23 18:41 CST); live proof in evidence B3 |
| 25 | DeepSeek direct API — **TEMPORARY PAYG tail** (routine DeepSeek comes from plan routes, never this door) | PAYG balance; **temporary tail order: Drew, 2026-11-23 13:20 CST** — added LAST in the DeepSeek pool until he lifts it | Balance reads are visibility only: ¥43.82 (REG cycle 404, 2026-11-10) — **stale** |
| 26 | OpenAI PAYG Luna tail — **TEMPORARY PAYG tail**: `openai-payg/gpt-6-luna#low` last in luna-plan (weight 0.1) and systemone (0.1) | PAYG; **temporary tail order: Drew, 2026-11-23 13:20 CST** | Seating weights as landed 2026-11-23 (I13677) |
| 27 | Claude PAYG Haiku tail — **TEMPORARY PAYG tail**: PAYG Haiku (low) last in the Claude pool | PAYG; **temporary tail order: Drew, 2026-11-23 13:20 CST** | Corroborating sighting: 2026-11-24T00:15Z OpenRouter probe sessions were in fact served by `claude-payg` / `claude-haiku-5-5` (I13677, 2026-11-23) — the tail exists live |
| 28 | Cloudflare Workers AI free tier: 10,000 Neurons/day (~160–550 LLM calls/day depending on model) | $0 tier exists (FREE §2, fetched 2026-11-23) | **Not wired:** provider is a disabled tombstone; vault token named `cloudflare.api-token-ai-inference.REVOKED` (FREE §4); LAPTOP 2026-11-23 confirms `cloudflare-workers-ai` + `cloudflare-ai-gateway` in `disabled_providers` |
| 29 | SambaNova — defined, seated NOWHERE | No working free tier in practice (probes 2026-11-23, I13677) | Live model list = 4 models, every completion HTTP 402 `PAYMENT_METHOD_REQUIRED` at balance 0 (2026-11-23); FREE §2's census row is superseded by these same-day probes |
| 30 | LongCat direct platform — NOT wired | No recurring free tier: billing launched 2026-06-30; current pricing $0.30/1M uncached input, $1.20/1M output (limited-time discount), model LongCat-2.5 (LongCat docs fetched 2026-11-23, I13677) | FREE §2/§3 (500K tokens/day free) is superseded. Disposition awaits Drew's decision: wire as cheap paid, or drop. Existing LongCat 2.5 Free legs ride the Go/Zen plans (rows 6/16) |

**Deliberately NOT rows:** the assistant Muse's own subscription (Maximum,
$80/mo per REG plan-facts preamble, 2026-11-23) — per the POOL MAP it is not a
pool, no session can draw on it. Antigravity's fee is absent because it is
not established (row 14). Mistral's free API tier no longer exists (FREE
§2/§4, 2026-11-23) — row 13 is a paid plan, not a free tier.

**Deployed config snapshot (LAPTOP, 2026-11-23 18:41 CST):** defaults
`"model": "opencode-route/workhorse"`, `"small_model": "opencode-route/speed"`;
6 tier keys (`scout`, `workhorse`, `specialist`, `summit`, `speed`,
`background`) + workhorse variants; **40 named plan pools**; 9 providers in
`disabled_providers` (google, google-vertex, cloudflare-ai-gateway, vercel,
huggingface, github-copilot, cloudflare-workers-ai, meta, _antigravity).

---

## 2. Tiers — models in use by role, and why

**Governing rule — Stewardship Decision 008 (Drew, 2026-10-25), quoted at the
head of EFF:** roles state requirements only (strength tier = hard
minimum-intelligence floor, plus a speed flag); the model is picked live at
dispatch time; named-model pins in agent/role defaults are **banned**. The
tiers below are **route heads** in the deployed config (`opencode-route/<tier>`)
whose pools the resolver walks live.

| Tier (route head) | What serves it (as built, dated) | Why (reasoning in the sources) |
|---|---|---|
| **scout** | Free cluster first: beam-plan, longcat-plan, iceman-plan, inkling-plan, openrouter-free-plan, commandcode-free-plan, kilo-free-plan (`kilo-auto/free`), then paid legs (FREE §5, 2026-11-23; PR #13691 seating, 2026-11-23) | Free-first is a standing route rule from Drew's 2026-11-23 order. Zen free models sit last-resort wherever they fit — "limited context" (FREE §5) |
| **background** | The full free cluster (flash-free, gemini-free, exaone-free, hy4-flash, glm Zen + Z.ai, flint, big-pickle, nemotron ×2, nvidia-free, openrouter-free, commandcode-free, kilo-free) before workhorse (FREE §5, 2026-11-23) | FREE §5 tier placement. Exception kept: "background's clock-gated paid chain (recorded value-order design)" (I13677, 2026-11-23) |
| **speed** (also the config's `small_model`) | First leg since 2026-11-07 22:00 CST: direct `_muse/muse-spark-1.3-contributor#xhigh` (weight 3), then deepseek-flash-plan (w2), haiku-plan (w2), gemini-flash-plan (w1) (REG speed-route entry, Drew's direct order) | Contributor rule (Drew, 2026-10-26, REG): "whenever a Meta model would be used, use its `-contributor` id instead — it is always cheaper on this plan". Effort on the Meta door is the `reasoning_effort` parameter — "There are NO `-low` model ids behind this door" (probed 2026-10-26, REG) |
| **workhorse** (config default; also the route Muse itself dispatches OpenCode sessions on — Drew, 2026-11-01) | deepseek-flash-plan at **high** effort, luna-plan, haiku-plan, Meta spark-contributor, sol-plan#low (REG model audit, 2026-10-27 deployed-config read) | EFF ladder (2026-10-25): "DeepSeek Flash at high effort — still cheaper than most options even if it takes a couple of iterations. On the OpenAI pool, Luna Max is the default… On the Claude side, Sonnet (medium or high)." GUIDE §5 (2026-10-25): small/cheap models at high–max effort — "The tokens are cheap; effort buys back the capability gap" |
| **specialist** | sonnet-plan#high, argon-plan#high, spark-plan#max, sol-plan#medium — ordered Claude → Meta → OpenAI → DeepSeek Pro in the 2026-10-27 audit; OpenAI last in the 2026-10-31 audit (REG cycles 144/168) | EFF ladder, Hard tier (2026-10-25): "Luna Max at higher effort, Sonnet high, or Opus at low/medium. Medium Sol only when something is genuinely complicated." |
| **summit** | opus-plan#high, argon-plan#high, muse-direct / `_muse` Wonder-contributor at #xhigh–#max, sol-plan#xhigh — OpenAI last (REG audits 2026-10-31, 2026-11-02) | EFF ladder, Very complex design (2026-10-25): "Opus high — deliberate, not a default. If in doubt, it isn't this class." REG POOL MAP: "high Opus only for very complex design, deliberate and rare" |

### Cross-tier rules that decide the actual pick
- **Cheapest sufficient first, measured fresh.** EFF's one rule (2026-10-25): "Never use more model than the task needs, and never spend money to keep working." Measurement gate: no placement/switch/dispatch without a quota read **≤60 minutes old** for that pool; burns over 1h re-check hourly and at every failover.
- **Burn to 100%, then closed.** "Use every pool all the way to 100%" (Drew, 2026-10-27, REG); at a known 100% the pool is CLOSED until reset or a fresh read shows it below 100%.
- **NEVER FUNDS.** "Plan quotas only. A 402/quota error is a wall, never a fallback to balances or PAYG. At most ONE retry on the same pool… When every plan is walled, the session STOPS." (REG POOL MAP.) The three TEMPORARY PAYG tails (rows 25–27, Drew 2026-11-23 13:20 CST) are a dated exception, not a repeal.
- **Burn-by-expiry; OpenAI is the final reserve.** "Burn the soonest-expiring pool first; OpenAI is the final reserve." (REG.) GUIDE rule 6 (2026-10-25) states the same.
- **Free-first.** "Free models sit in front wherever they can serve" (Drew, 2026-11-23, standing rule; executed in PR #13691, 2026-11-23).
- **Gates.** Astra: GATED — only when Aster calls for it, or with a reason logged at time of use (REG/EFF). Antigravity Argon: "use sparingly — the expensive Gemini tier (price doubles 2026-12-01). Prefer Flash tiers" (REG ladder; EFF, 2026-10-25). Cloudflare gateway: "ONE session at a time, nothing important" (REG) — and in fact disabled fleet-wide (row 28).
- **Effort is part of the model choice.** GUIDE rules 1–3 (2026-10-25): effort is always set explicitly ("A turn running variant 'default' is a process failure"); the unit of decision is the (model, effort) pair. Sizing: flagships medium; mid tier medium–high; small/cheap high–max.
- **Family preference, all else equal** (REG, Drew 2026-10-24): "claude first, then openai; gemini sits wherever the specific job fits it" — but "right-sizing beats family preference", and where the ladder and the older family text disagree, "this ladder wins".

### Capability tiers for like-for-like failover (REG, standing text, read 2026-11-23)
- **Flagship:** openai gpt-6.1-sol / gpt-6-sol / gpt-5.6-sol; opencode-go gpt-5.6-sol; meta muse-wonder-1.0; antigravity claude-opus-5-5.
- **Strong:** openai gpt-6-luna / gpt-5.6-terra / gpt-5.6-luna; meta muse-spark-1.3; antigravity/google claude-sonnet-5-5; cline-pass glm-5.3, kimi-k3, deepseek-v4-pro; opendesign deepseek-v4-pro, glm-5.3.
- **Light:** cline-pass glm-5.3-flash, deepseek-v4.1-flash; opendesign deepseek-v4.1-flash, glm-5.3-flash; meta muse-spark-1.3 at its low variant; openai gpt-5.6-luna-fast; opencode big-pickle.
- Failover effort rule (REG): prefer the same effort variant; drop at most one step when quota forces a lighter pick; never jump flagship → light without noting it in the supervision log.

---

## 3. Top evidence items (of 16 in the pack — B1–B16)

Full items B1–B16 are in `survey/llm-evidence.md` §3. The load-bearing ones:

- **B1 — Luna-as-decision experiment (headline benchmark).** Eval set: 36 items, bar posted BEFORE any run (EVAL). System: `gpt-6-luna` at low effort + decision-engine harness. Results: **schema-faithful 36/36 (100%); accuracy 35/36 = 97.2%**; baseline Google Decision 1 native: 36/36 schema, 36/36 accuracy. Bar check: all four bars passed — **VERDICT: PASS**. Spend: 11,979 tokens ≈ **$0.08** at assumed $5/M in / $20/M out (rates assumed, stated in the verdict). Artifacts in `~/workspace/decision-eval/`; read 2026-11-23.
- **B3 — OpenAI free-alias live proof (2026-11-23).** After PR #13836/#13838, three consecutive `systemone` turns were served by **`openai-payg-free/gpt-6-luna`** with no errors; the paid tail served none of the post-fix probes (I13677).
- **B4 — NVIDIA entitlement probe (2026-11-23).** Account live list **32 ids** (public catalog 83); **15 × HTTP 200**, 6 × 404 listed-but-not-entitled, 1 dead seat removed (`deepseek-v4.1-flash` answered 410); after seating (PR #13777): **12/12 seated models HTTP 200** (I13677).
- **B5 — Groq + OpenRouter + Kilo + SambaNova probes (all 2026-11-23).** Groq **3/3 HTTP 200**; OpenRouter account-level **429** (quota state, not a model verdict); Kilo unauthenticated completion **HTTP 200, $0**; SambaNova **4/4 HTTP 402** (I13677).
- **B6 — OpenRouter fall-through probe (2026-11-23 18:22 CST).** Two probe sessions both `succeeded` with reply "OK", **but neither completion was served by an OpenRouter :free model** (one by `claude-payg`/`claude-haiku-5-5`, one by `_muse`/`muse-spark-1.3-contributor`). Verdict on the issue: **no verified OpenRouter-served completion through the route exists** — any future probe citation must name the **serving** provider, not the requested route.
- **B7 — WALL-SKIP.** A **design + coordination record, not a measured result** (Drew's order 2026-11-23 14:36 CST, tracking issue #13693). **No WALL-SKIP benchmark numbers exist in any source read — do not cite any.** Related deployed behaviour (REG, 2026-11-08): pool-level quota verdicts abolished by Drew's 2026-11-07 rule; telemetry proof: zero `pool-quota-exhausted` decisions in the 10 minutes after cutover across 2,419 decisions.
- **B11 — OpenCode Go value arithmetic (computed 2026-10-27, REG).** $10/mo buys a $60 list-rate meter = **6.0x** at full burn; Flash-concentrated **8.0x**; blended realized that period **4.26x** (console read 2026-10-27). Comparison points: ClinePass 2–5x, Command Code GOAT 7x, DeepSeek API direct 1.0x.
- **B14 — WIRE-FREE validation suites (2026-11-23).** PR #13691: suites **114/114**; PR #13777: **146/146**; PRs #13836/#13838: **77/77** + **48/48** (I13677).
- **B15 — Billing incidents (measured losses, 2026-10-25).** Meta spill: **$20.00 (05:15 CDT) + $3.91 (07:47 CDT) = $23.91** on 2026-10-25, plus a third $20.00 METAPAY charge 2026-10-26 10:08 CDT (REG). Cloudflare gateway DeepSeek accrual Oct 24–25: **$29.60 per budget alert vs $45.10 per gateway cost attribution** (REG). These motivate the measurement gate.
- **B16 — Effort-guide pool snapshot (STALE specimen).** GUIDE §8, 2026-10-25 ~06:00 CDT — **~29 days old at pack time; every state in it has since changed.** Included in the pack only as a dated specimen.

Remaining items (in the pack, not condensed here): B2 (OpenAI complimentary-token mechanics, fetched 2026-11-23), B8 (Command Code ACP retrofit measurement, verified 2026-10-24/25), B9 (Claude Code bridge verification, 2026-10-25: route_attempts 469/470 success), B10 (Meta door probe series 2026-10-26 → 2026-11-04), B12 (gate-fix telemetry, 2026-11-08), B13 (tool-call support audit, 2026-11-05).

---

## 4. Walls & resets — as of their last dated reads

**Governing fresh read: REG cycle 471 gatherer, measured 2026-11-23T12:15Z —
~12.4 hours old at pack assembly (2026-11-23 ~18:40 CST). Nothing here is
fresher than its row's read.**

| Pool / window | State at last read | Reset |
|---|---|---|
| OpenAI alt weekly | **100% CLOSED** | 2026-11-23T18:49Z — treat as just-reset, unverified until a fresh read |
| OpenAI main weekly | 59% — open, final reserve | 2026-11-24T21:23Z |
| ClinePass monthly | **100% CLOSED** (weekly/5h 0%) | 2026-12-06T14:40Z |
| Open Design 7d | **100% CLOSED** | 2026-11-24T11:44Z |
| OpenCode Go drewrypope / popesdrew monthly | 100% | 2026-12-03 |
| OpenCode Go developing-today-0 monthly | 100% | 2026-12-08 |
| OpenCode Go 0x111dev | 5h 64% / weekly 66% / monthly 83% — open | monthly 2026-12-18 |
| Command Code monthly | 99.51% — not walled | monthly window (console period data 2026-10-27, stale) |
| Mistral API + Vibe | Both **100%** — **measured 2026-11-16, ~7 days stale inside the read itself** | 2026-12-01 |
| Claude (`_claude`) | **UNMEASURED** (see row 4) | carried reset 2026-11-25T19:00Z |
| Meta weekly | **STALE** (see row 5) | new window's reset not yet read |
| Antigravity weekly (both families) | 1.64% each — healthy | 2026-11-30T01:41Z |
| OpenRouter :free daily | Account-level **429**, remaining 0 | 2026-11-24T00:00Z |
| EXAONE free key | Not a wall — a hard expiry | **Key dies 2026-12-16** |

Historical note (REG standing text): OpenAI main has produced **unplanned
mid-window resets** (2026-10-29: weekly 46% → 23% with the reset label
unchanged) — a closed pool reopens earlier if a fresh measurement shows it
dropped below 100%; never assume the labelled reset is the real one for this
door.

## 5. Rot rules (from the pack's §5 — binding on Space pages built from this file)

- **Fees & plan structure: slow rot (weeks–months), two fast exceptions** —
  EXAONE's key dies **2026-12-16** (dated, certain); a provider's *published*
  free tier can already be withdrawn while its docs describe it (SambaNova /
  LongCat, 2026-11-23) — any free-tier claim older than 2026-11-23 needs a
  live probe, not a docs read.
- **Quota percentages & resets: fastest rot (hours).** Cite the read
  date/time in the same sentence as any figure. Two pools were
  unmeasured/stale at their freshest read (Claude, Meta) — say so wherever
  their 2026-11-23 state is mentioned.
- **Tiers & route heads: medium rot (days–weeks).** Tier *definitions*
  (Decision 008, the ladder, the gates) rot slowly; tier *membership* rots
  with every config PR (the deployed config grew from 244,128 bytes at cycle
  471's canonical read to 279,445 bytes by 17:10 CST the same day,
  2026-11-23). Re-check the deployed `opencode.jsonc` before citing membership.
- **Benchmarks: results don't rot; inferences do.** B1's numbers are
  historical facts; "therefore seat this model" rots with model ids, prices,
  and programme terms (OpenAI's complimentary programme carries a
  30-days'-notice termination clause; B1's $0.08 rests on assumed rates).
- **Names rot too:** `openai.main` → `_openai.main` (2026-11-10), Lab →
  popesdrew (2026-11-03), `cline` → `cline-pass` (2026-10-25). The registry
  file itself contains an unresolved merge-conflict marker pair in its
  reconciliation section (observed 2026-11-23) — figures from that one
  section should be double-checked against its two dated runs.
