# Benchmarks and real-use evidence

This chapter holds the measured record: 16 evidence entries, B1–B16, each dated, each with its numbers as the source recorded them. A dated result is a historical fact. The inference drawn from a result ("seat this model") rots as model ids, prices and programme terms change (pack §5, read 2026-11-23). Every probe citation in this chapter names the serving provider where the source names it, not the requested route; entry B6 records why that distinction is binding. The plans tested here are in [The plans we use](the-plans.md).

## B1 — Luna as decision engine (the headline benchmark)

- Date and system (eval files in `~/workspace/decision-eval/`, read 2026-11-23): `gpt-6-luna` at low reasoning effort (`openai-payg/gpt-6-luna#low`) plus a decision-engine harness with strict per-item JSON Schema forcing `{"value", "confidence"}` (read 2026-11-23). Work performed on code#13675 Part 3b (read 2026-11-23).
- Eval set (read 2026-11-23): 36 items, 12 per family (noul yes/no, choice, score), deterministic expected answers. The bar was posted before any run (bar-comment.md, read 2026-11-23).
- Results (results.json and verdict-comment.md, read 2026-11-23): schema-faithful 36/36 (100%); accuracy 35/36 = 97.2% (noul 12/12, choice 11/12, score 12/12 with a tolerance of 1 level, read 2026-11-23).
- Baseline (read 2026-11-23): Google Decision 1 native (`gemini-decision-1`): 36/36 schema, 36/36 accuracy, run on the free tier at $0 (read 2026-11-23).
- Bar check (verdict-comment.md, read 2026-11-23): schema 100% against a bar of 95% — pass; accuracy 97.2% against a bar of 90% — pass; lowest family 91.7% against a bar of 80% — pass; gap to baseline 2.8 points against a bar of 5 points — pass. Verdict recorded 2026-11-23: PASS, "Luna low is seatable in `systemone` after the free legs" (verdict-comment.md, read 2026-11-23).
- The one miss (read 2026-11-23): choice item c07, third-party email with no per-item approval recorded; expected `escalate`, Luna answered `hold` (results.json, read 2026-11-23).
- Spend (read 2026-11-23): 10,818 prompt tokens plus 1,161 completion tokens = 11,979 tokens, $0.08 at assumed rates of $5/M input and $20/M output, and under $0.16 at double those rates, inside the $2 bound (verdict-comment.md, read 2026-11-23). The rates are assumed rates, stated in the verdict dated 2026-11-23; Luna's pay-as-you-go list price was not carried in the API response (pack §5, read 2026-11-23).
- Calls (read 2026-11-23): all 72 calls returned HTTP 200 (read 2026-11-23).

## B2 — OpenAI complimentary-token mechanics

Documentary measurement, not a benchmark run (pack §3, read 2026-11-23). Source: OpenAI Help Center art. 10306912 and the eligible-models list (reading dated 2026-10-20), fetched 2026-11-23 (issue #13677 comment, read 2026-11-23).

- Two per-group daily buckets, resetting 00:00 UTC (fetched 2026-11-23): Large group (gpt-6-luna, gpt-6-astra, gpt-6-sol in the group, fetched 2026-11-23) at 250K/day at Build tiers 1–2 and 1M/day at Launch/Grow; Small group at 2.5M/day rising to 10M/day (fetched 2026-11-23).
- Per-request check (fetched 2026-11-23): a request that crosses the day's quota is billed in full, with no pro-rating.
- Stacking order (issue #13677 comment, read 2026-11-23): complimentary daily tokens first, then ChatGPT-benefit credits, then purchased balance. Within-quota requests are not billed (read 2026-11-23). The separate ChatGPT-Pro benefit is $100/mo API credits for the $100 Pro tier, linked to one organisation, used before purchased credits (read 2026-11-23).
- Exhaustion is silent (fetched 2026-11-23): no error returns; traffic bills.
- Excluded (fetched 2026-11-23): fine-tuned models, fine-tuning, evals, tool use. ZDR and Enterprise organisations are excluded from the programme (fetched 2026-11-23).
- Termination term (pack §5, read 2026-11-23): the complimentary programme carries a 30-days'-notice termination clause (read 2026-11-23).

## B3 — OpenAI free-alias live proof

After PR #13836 and PR #13838 (2026-11-23), three consecutive `systemone` turns were served by `openai-payg-free/gpt-6-luna` with no errors, and `route_attempt` holds success rows for the alias; the paid tail served none of the post-fix probes (issue #13677 comment, read 2026-11-23). Two runtime defects came from the probes dated 2026-11-23: the credential store resolves by the alias's own id, and a row for `openai-payg-free` had to be seeded; `instanceOf` does not inherit the base's npm package, and PR #13838 re-declares `"npm": "@ai-sdk/openai"` on the alias (read 2026-11-23).

## B4 — NVIDIA entitlement probe

Probed 2026-11-23 with the escrowed key (issue #13677 comment, read 2026-11-23). The account's live `GET /v1/models` returned 32 ids on 2026-11-23; the public catalogue held 83 models on 2026-11-23 (read 2026-11-23). Minimal completions on 2026-11-23: 15 returned HTTP 200 (Nemotron-3-Ultra 550B, Mistral Large 4, Kimi K3.1 Pro, Gemma 4 31B in the 15, read 2026-11-23); 6 returned 404, listed but not entitled (gemma-3-12b/4b and cosmos-reason2-8b in the 6, read 2026-11-23); 3 were entitled with non-chat shapes (422/500, read 2026-11-23); 1 was inconclusive (glm-5.3-flash, two timeouts, not seated, read 2026-11-23); 7 embed-shelf ids were listed for the decision lane and not chat-probed (read 2026-11-23). A dead seat came out of the probe dated 2026-11-23: seated `deepseek-ai/deepseek-v4.1-flash` answered 410 and was removed (read 2026-11-23). After seating in PR #13777 (2026-11-23): 12/12 seated models returned HTTP 200 on direct completions (read 2026-11-23); a through-route prompt was accepted (HTTP 200, 2026-11-23) before a server restart cut off reply capture (read 2026-11-23). Listing is not entitlement: the 32-id account list and the 83-model public catalogue are different sets (both read 2026-11-23).

## B5 — Groq, OpenRouter, Kilo and SambaNova probes

All probes dated 2026-11-23 (issue #13677 comments, read 2026-11-23):

- Groq (probed 2026-11-23): direct completions 3/3 HTTP 200 (gpt-oss-120b, gpt-oss-20b, qwen3.8-27b) with the key created 2026-11-23 (read 2026-11-23).
- OpenRouter (probed 2026-11-23): every candidate completion returned the account-level 429 (`free-models-per-day`, limit 50, remaining 0, resets 2026-11-24T00:00Z, read 2026-11-23). The account's own traffic consumed the day's free quota on 2026-11-23; the 429 is a quota state, not a model verdict (read 2026-11-23).
- Kilo (probed 2026-11-23): unauthenticated completion on `kilo-auto/free` returned HTTP 200 at $0, rotated to `nvidia/nemotron-3.5-super-vl:free` (read 2026-11-23).
- SambaNova (probed 2026-11-23): 4/4 models returned HTTP 402 `PAYMENT_METHOD_REQUIRED` (read 2026-11-23). The full correction is in [The plans we use](the-plans.md) (row 29, read 2026-11-23).

## B6 — OpenRouter fall-through probe (D27)

Run after the free-tier reset and recorded 2026-11-24T00:22Z, which is 2026-11-23 18:22 CST (issue #13677 comment, read 2026-11-23). Two identical probe sessions exist: the probe client's transport timed out and the laptop-agent SSH fallback re-ran the script once; the attempt budget of 2 attempts is spent (read 2026-11-23). Sessions created 2026-11-24T00:15:26Z and 2026-11-24T00:17:28Z, both set to `opencode-route/openrouter-free-plan`, both recorded `succeeded` with reply "OK" and no 429 surfaced (read 2026-11-23). Neither completion was served by an OpenRouter :free model: one completion was served by `claude-payg` / `claude-haiku-5-5` and the other by `_muse` / `muse-spark-1.3-contributor` (read 2026-11-23).

Verdict recorded on the issue, read 2026-11-23: the owed item is NOT closed in substance. No verified OpenRouter-served completion through the route exists after the reset of 2026-11-24T00:00Z (read 2026-11-23), and the sessions' empty route-target records do not distinguish "legs failing and masked by route fallback" from "legs never attempted" (read 2026-11-23). A route success record names the route's outcome, not the serving provider. The open verification item is listed in [LLM plans: prioritised list](priorities.md).

## B7 — WALL-SKIP (route wall scoping)

WALL-SKIP has no measured results. No WALL-SKIP benchmark numbers are recorded in any source read for the evidence pack dated 2026-11-23, and this chapter cites none.

What exists in the sources read 2026-11-23 is a design and coordination record: Drew's order of 2026-11-23 14:36 CST, tracking issue #13693 (issue #13677 comment, read 2026-11-23). The lane's only config touch is its slice C: a checker-validated `wallScope` provider declaration plus the first free/paid alias split for a mixed plan-plus-free provider, two alias ids on one credential source, so a paid-side wall never benches the free models (read 2026-11-23). Slice C was sequenced after the WIRE-FREE config PR; the fork and proxy slices do not touch the config (read 2026-11-23). The pattern was executed for OpenAI by the OPENAI-FREE-SHAPE lane (entry B3 in this chapter; `_commandcode-free` precedent cited there as #13693 slice C, PR #13784, read 2026-11-23).

Related deployed wall behaviour (registry, recorded 2026-11-08 00:37 CST, read 2026-11-23): pool-level quota verdicts were abolished by Drew's rule of 2026-11-07 23:02 CST, fork PR #1979 merged (read 2026-11-23); telemetry proof recorded 2026-11-08: zero `pool-quota-exhausted` decisions in the 10 minutes after cutover across 2,419 decisions (read 2026-11-23). The scoping rule itself is in [Pools, routes and the switchboard](pools-and-routes.md).

## B8 — Command Code resident-ACP retrofit measurement

Sandbox, verified 2026-10-24 at 11:27 CDT (registry, read 2026-11-23): resident ACP bridge, two-turn codeword recall at 5s plus 5s, against the phase-1 stateless baseline of 10–19s per turn and 11–15s raw `cmd -p` probes (read 2026-11-23). Laptop two-turn after its IPv6 fix: 18.8s / 9.3s with recall on the same resident process (registry, recorded 2026-10-25 at 00:43 CDT, read 2026-11-23). Caveat recorded in the registry, read 2026-11-23: OpenCode's own per-message counts show 26.1k input on both turns; OpenCode sends that count to the proxy and the proxy delta-fies to cmd (read 2026-11-23).

## B9 — Claude Code bridge verification

Provider-sync verification in opencode.db (registry, recorded 2026-10-25 at 06:20 CDT, read 2026-11-23): route_attempts 469/470 for claude-code/claude-haiku-5-5 success (19.96s cold, 1.75s recall turn, recorded 2026-10-25); 472 openai non-regression successes (recorded 2026-10-25, read 2026-11-23). Independent health check on the same morning, 2026-10-25: bridge ok, 3 live sessions and 4 stored sessions (read 2026-11-23).

## B10 — Meta door probes

A dated series, kept as three separate entries in the pack dated 2026-11-23:

- (a) Probed 2026-10-26 14:30–14:42Z: Meta pool blocked; the API returned 402 billing_not_configured on all 11 probes (registry, read 2026-11-23).
- (b) Probed 2026-10-27 at 15:35 CDT: live API probe with the stored `meta` key returned HTTP 200 completion on muse-spark-1.3-contributor (registry, read 2026-11-23); laptop route telemetry that day (2026-10-27) recorded 147 successful meta attempts and 0 failures, with 648 successes all-time against exactly 10 historical 402s from the batch dated 2026-10-26 (registry addendum, evening of 2026-10-27, read 2026-11-23). Recorded verdict dated 2026-10-27: "The 402 era is closed history" (read 2026-11-23).
- (c) Determination recorded 2026-11-04 at 23:50 CST (registry, read 2026-11-23): a live OAuth-route attempt returned 429 "Subscription quota exhausted… resets at 2026-11-09T00:00:00Z" while the key endpoint returned no `subs_usage` at all (read 2026-11-23). The endpoint's usage publication is unreliable in every state and was retired as the percentage source; the dev.meta.ai portal API is the proven percentage source, and its fetcher restore is filed as code#6079 (read 2026-11-23).

## B11 — OpenCode Go value arithmetic

Computed, not probed (registry, computed 2026-10-27, read 2026-11-23; full arithmetic in the ai-plans record dated 2026-10-20, addendum 2026-10-27, read 2026-11-23). Go at $10/mo buys a $60 list-rate meter: 6.0x value per dollar at full burn (computed 2026-10-27). DeepSeek V4.1 Flash's $80 promo cap gives 8.0x Flash-concentrated, at 300 AA-sized max-effort Flash tasks per workspace per month in the computation dated 2026-10-27 (read 2026-11-23). Realised in that period (console read 2026-10-27, both workspaces then recorded Cancelled, read 2026-11-23): Personal 80% used = $48 list burned (4.8x realised); Lab 62% = $37.20 list burned (3.72x realised); blended 4.26x, with $34.80 list left unburned (computed 2026-10-27, read 2026-11-23). Caps never roll over; the ratios are conditional on utilisation (read 2026-11-23). Comparison points in the same verdict dated 2026-10-27: ClinePass 2–5x, Command Code GOAT 7x, DeepSeek API direct 1.0x (read 2026-11-23).

## B12 — Gate-fix telemetry

Real-use, post-deploy (registry, recorded 2026-11-08 00:37 CST, read 2026-11-23): in the first 20 minutes of the build that abolished pool verdicts, workhorse successes via flash-pool legs counted cline-pass 220, opendesign 184, `_vibe` 177, `_commandcode` 42 (recorded 2026-11-08, read 2026-11-23). The same night (recorded 2026-11-07 22:51–22:57 CST, read 2026-11-23): four routed OpenDesign successes via `speed`, route_attempt ids 353151/353224/353251/353259, all `opendesign/claude-haiku-5.5`, the first routed OpenDesign attempts since the latch of 2026-11-06 19:32Z (read 2026-11-23).

## B13 — Tool-call support audit

Live round-trips (registry audit, dated 2026-11-05, read 2026-11-23; tracking opencode#1708, read 2026-11-23):

- vibe-direct: PASS, true one-message parallel calls with byte-exact argument fidelity live (audit dated 2026-11-05, read 2026-11-23).
- opencode-route façade: PASS via the openai.main leaf, with a cross-leaf signature caveat at opencode#1705 (audit dated 2026-11-05, read 2026-11-23).
- antigravity-direct: FAIL live, then FIXED in code; PR #1716 merged; the wire model id was unconditionally effort-suffixed (audit dated 2026-11-05, read 2026-11-23).
- claude-code and commandcode: marked CRITICAL if enabled; CLI-transcript shapes, tool calls never emitted (code#6117, audit dated 2026-11-05, read 2026-11-23).
- muse-direct, cline-pass, opendesign, opencode-go: recorded BLOCKED-BY-WALL at audit time (2026-11-05), with wall reset dates in [Walls, resets and quota doctrine](walls-and-resets.md) (read 2026-11-23).

## B14 — WIRE-FREE config validation suites

Suite counts, each dated 2026-11-23 (issue #13677 comments, read 2026-11-23):

- PR #13691 (2026-11-23): checkers passed, suites 114/114 (read 2026-11-23).
- PR #13777 (2026-11-23): checkers PASS, suites 146/146 (read 2026-11-23).
- OPENAI-FREE-SHAPE PRs #13836/#13838 (2026-11-23): route-rules plus metered plus go-seating suites 77/77; catalog-sync plus snapshot suites 48/48 (read 2026-11-23).
- The go-seating CLI findings in the run dated 2026-11-23 were identical on pristine origin/main: live-traffic freshness, unrelated to the change (read 2026-11-23).

## B15 — Billing incidents behind the measurement gate

Measured losses, recorded 2026-10-25 (registry and efficiency plan, read 2026-11-23):

- Meta, 2026-10-25: weekly quota read once at 03:20 CDT (46% used, read 2026-10-25) and never re-measured during a burn of 7h (recorded 2026-10-25); spill metered as pay-as-you-go: $20.00 charged 05:15 CDT plus $3.91 charged 07:47 CDT = $23.91 (recorded 2026-10-25, read 2026-11-23). A third $20.00 METAPAY charge landed 2026-10-26 10:08 CDT on a different card (registry, read 2026-11-23).
- Console correction recorded 2026-10-25 09:38 CDT: no charge-past-plan toggle exists; spill is automatic when any plan window runs out, and the wall on the morning of 2026-10-25 was the short window, not the weekly window (read 2026-11-23).
- Cloudflare gateway, Oct 24–25, 2026: DeepSeek V4 Pro accrual $29.60 per budget alert against $45.10 per gateway cost attribution (registry, read 2026-11-23). The two figures are two attributions of one accrual, recorded 2026-10-25 (read 2026-11-23).
- A stewardship visibility build on Astra burned a week of the GPT pool, $25 in value in the record's estimate, on basic work (efficiency plan, dated 2026-10-25, read 2026-11-23). The $25 figure is the source's estimate, recorded 2026-10-25 (read 2026-11-23).

The doctrine these losses produced is in [Walls, resets and quota doctrine](walls-and-resets.md).

## B16 — Effort-guide pool snapshot (stale specimen)

Effort guide §8, snapshot dated 2026-10-25 at 06:00 CDT (read 2026-11-23): claude-code live; cline-pass live (probe-verified 05:56 CDT, 2026-10-25); opencode-go behind a 402 funds wall since 04:43 CDT, 2026-10-25; antigravity-ls Gemini live and Claude fraction at 429 (all dated 2026-10-25, read 2026-11-23). The snapshot is 29 days old at pack time (2026-11-23) and is labelled stale in the pack dated 2026-11-23; every state in it has since changed (pack §3, read 2026-11-23). Current-at-last-read state is in [Walls, resets and quota doctrine](walls-and-resets.md), with each row's read date. Effort guide §3 pool descriptions carry the same verification date, 2026-10-25 (read 2026-11-23).

Sources. LLM evidence pack, `~/workspace/pm-book/survey/llm-evidence.md` §3 (assembled 2026-11-23), and the sources the pack names for §3: EVAL (decision-eval/ files, read 2026-11-23), I13677 (developing-today/code issue #13677 and its 15 comments, read 2026-11-23), REG (provider-quota-registry.md, read 2026-11-23), EFF (model-efficiency-plan-2026-10-25.md, dated 2026-10-25), FREE (free-census-2026-11-23.md, dated 2026-11-23), GUIDE (provider-model-effort-guide.md, read 2026-11-23).
