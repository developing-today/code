# Pools, routes and the switchboard

A pool is a plan allowance behind one or more doors. A route is an ordered list of legs the resolver walks. The switchboard is the `disabled_providers` list in the deployed config: a provider named there is off, and a provider absent from the list is on. This chapter records the deployed state read on the laptop on 2026-11-23 18:41:54 CST and the standing rules that govern the walk. The plans behind the pools are in [The plans we use](the-plans.md). The tier definitions are in [Models by tier, and why](models-by-tier.md).

The routes in this chapter serve sessions running in the forks programme's harness; that programme has its own section: [OpenCode and OpenChamber: our forks programme](../opencode-openchamber/index.md).

## Pools and doors

One pool, several doors is the normal shape. The Meta High Usage plan is one pool (POOL MAP, Drew, 2026-10-25, read 2026-11-23) and its doors changed across the record read 2026-11-23: the `meta` provider was the only door (2026-10-25, read 2026-11-23), the direct `meta` door was declared dead and routes re-seated on `muse/*` leaves (registry cycle 144 audit, 2026-10-30, read 2026-11-23), `muse-direct` became the sole enabled door (Drew, 2026-11-02, PR #4357, read 2026-11-23), and the deployed config read 2026-11-23 18:41 CST holds `_muse` and `muse-direct` present with `meta` in disabled_providers (laptop read, 2026-11-23). The pool stayed one pool through every door change (POOL MAP, read 2026-11-23).

OpenAI runs the same shape in reverse order since 2026-11-22: the alt door `_openai.alt` (popesdrew) is ordered first in the `openai-plan` pool and burns first, and the main door `_openai.main` (drewrypope) is its failover (Drew's order 2026-11-22, landed PR #12731, read 2026-11-23). OpenCode Go is one shared bucket per workspace, not per-model buckets (verified 2026-10-27, read 2026-11-23); the four workspaces in the reads dated 2026-11-23 are four pools, not four doors onto one pool.

Provider names rot. The record read 2026-11-23 carries three renames: `openai.main` to `_openai.main` (2026-11-10), Lab to popesdrew (2026-11-03), and `cline` to `cline-pass` (2026-10-25). A search of older records under a current name misses material filed under the old name, and a search under an old name misses current material (pack §5, read 2026-11-23).

## Route heads and defaults

Roles state requirements only. Stewardship Decision 008 (Drew, 2026-10-25, quoted in the efficiency plan dated 2026-10-25 and read 2026-11-23) states the rule in full: roles state a strength tier as a hard minimum-intelligence floor plus a speed flag; the model is picked live at dispatch time; named-model pins in agent and role defaults are banned. The tiers are route heads in the deployed config (`opencode-route/&lt;tier&gt;`, config read 2026-11-23) whose pools the resolver walks live.

Deployed defaults, read on the laptop 2026-11-23 18:41 CST: `"model": "opencode-route/workhorse"` and `"small_model": "opencode-route/speed"` (laptop read, 2026-11-23). Tier keys present in the deployed file, one each (laptop grep, 2026-11-23): `scout`, `workhorse`, `specialist`, `summit`, `speed`, `background`, plus the workhorse variants `workhorse-primary`, `workhorse-weighted`, `workhorse-ordered`, `workhorse-last-resort` (laptop grep, 2026-11-23). The deployed file holds 82 `opencode-route/` references (laptop grep, 2026-11-23) and 40 named plan pools (laptop grep, 2026-11-23); the 40 pools are summarised by family in [Models by tier, and why](models-by-tier.md).

The workhorse route is the default route head, and it is also the route Muse itself dispatches OpenCode sessions on (Drew, 2026-11-01, read 2026-11-23).

## The switchboard

The deployed `disabled_providers` list holds 9 entries (laptop read, 2026-11-23 18:41 CST): google, google-vertex, cloudflare-ai-gateway, vercel, huggingface, github-copilot, cloudflare-workers-ai, meta, _antigravity.

Two entries in the list dated 2026-11-23 carry findings:

- `meta` is disabled while `_muse` and `muse-direct` are present in the same deployed config (laptop read, 2026-11-23 18:41 CST). The disabled entry is the old door, not the pool: the Meta pool's current doors are the two present entries (door history, read 2026-11-23).
- `_antigravity` is disabled in the deployed config (laptop read, 2026-11-23 18:41 CST) while its quota reads are healthy (D23): Gemini and Claude/GPT weekly 1.64% each, resets 2026-11-30T01:41Z (registry cycle 471, measured 2026-11-23T12:15Z). The pack records this as a standing door-state finding since cycle 459 (registry, read 2026-11-23). State for both pools is in [Walls, resets and quota doctrine](walls-and-resets.md).

Cloudflare sits behind the same switch: `cloudflare-workers-ai` and `cloudflare-ai-gateway` are both in the list dated 2026-11-23, the Workers AI provider is a disabled tombstone with no definition, and the vault AI-inference token is named `cloudflare.api-token-ai-inference.REVOKED` (free census, read 2026-11-23). The older metered Cloudflare gateway routes were disabled fleet-wide on 2026-10-25 after $29.60 to $45.10 of metered DeepSeek accrual on Oct 24–25, 2026 (registry and efficiency plan, read 2026-11-23).

## Wall scoping: the narrowest true scope

A wall benches the thing that ran out, at the narrowest true scope. Pool-level quota verdicts were abolished by Drew's rule of 2026-11-07 23:02 CST: "a pool can't run out — only models and providers run out" (registry, read 2026-11-23). Pool language in records from before 2026-11-07 that speaks of a pool running out is superseded by this rule (D29). Fork PR #1979 merged that rule (registry, read 2026-11-23). Telemetry proof, recorded 2026-11-08 00:37 CST: zero `pool-quota-exhausted` decisions in the 10 minutes after cutover across 2,419 decisions (registry, read 2026-11-23).

The WALL-SKIP work (Drew's order, 2026-11-23 14:36 CST, tracking issue #13693, read 2026-11-23) extends the same scoping into the config: a checker-validated `wallScope` provider declaration, plus the first free/paid alias split for a mixed plan-plus-free provider, two alias ids on the same credential source, so a paid-side wall never benches the free models (issue #13677 comment, read 2026-11-23). The OpenAI free alias in [The plans we use](the-plans.md) (row 24, read 2026-11-23) is that pattern executed: `wallScope: "account"` on `openai-payg-free`, free leg weight 3 ahead of the paid tail at 0.1 in luna-plan and systemone (read 2026-11-23). The evidence record for WALL-SKIP holds no measured results; [Benchmarks and real-use evidence](benchmarks-and-evidence.md) states that in full.

At a known 100% a pool is closed until its reset or a fresh read shows it below 100% (registry, read 2026-11-23). OpenAI doors reset early at times: the main door produced an unplanned mid-window reset on 2026-10-29, weekly 46% to 23% with the reset label unchanged (registry, read 2026-11-23). The labelled reset is a read, not a promise; the fresh read decides.

## Free-first ordering

Free legs lead every pool their capability fits; plan and paid legs follow (Drew, 2026-11-23, standing rule, read 2026-11-23). The free-first ordering pass landed in PR #13691 (merged 2026-11-23, read 2026-11-23). Scout leads with beam, longcat, iceman, inkling and openrouter-free before paid legs (free census §5, read 2026-11-23). Background runs the full free cluster before workhorse (free census §5, read 2026-11-23). Zen free models sit as last-resort legs wherever they fit, with limited context, per config comments (free census §5, read 2026-11-23). Two recorded exceptions stand in the pass dated 2026-11-23: background's clock-gated paid chain, a recorded value-order design (issue #13677 comment, read 2026-11-23), and Beam :free seated on Drew's direct order of 2026-11-23 despite its tier being recorded ended 2026-11-13 (read 2026-11-23).

The three TEMPORARY pay-as-you-go tails sit at the end of their pools, last by construction: DeepSeek direct last in the DeepSeek pool, OpenAI pay-as-you-go Luna low last in the OpenAI pool, Claude pay-as-you-go Haiku low last in the Claude pool (Drew's order, 2026-11-23 13:20 CST, read 2026-11-23). TEMPORARY marks all three tails in [The plans we use](the-plans.md) (rows 25–27, read 2026-11-23).

## Gates on specific legs

- Astra (GPT-6 via the ChatGPT pool) is gated: only when Aster calls for it, or with a specific reason logged at time of use (registry and efficiency plan, read 2026-11-23).
- Antigravity Argon: use sparingly. Argon is the expensive Gemini tier and its price doubles on 2026-12-01 (efficiency plan, dated 2026-10-25, read 2026-11-23). Flash tiers are the preferred Gemini legs (registry ladder, read 2026-11-23).
- Cloudflare gateway: one session at a time, nothing important (registry, read 2026-11-23). The gateway is disabled fleet-wide in the deployed config read 2026-11-23, per the switchboard list in this chapter.

## Effort is part of the route

The unit of decision is the (model, effort) pair (effort guide, read 2026-11-23). Effort is always set explicitly; a turn running variant "default" is a process failure (effort guide rules, read 2026-11-23). A hot prompt cache weights staying on the current pair (effort guide, read 2026-11-23). Sizing, from the effort guide read 2026-11-23: flagships at medium, mid tier at medium to high, small and cheap models at high to max. Tier-level effort assignments are in [Models by tier, and why](models-by-tier.md).

Sources. LLM evidence pack, `~/workspace/pm-book/survey/llm-evidence.md` §2 and appendix (assembled 2026-11-23), and the sources the pack names for §2: REG (provider-quota-registry.md, read 2026-11-23), EFF (model-efficiency-plan-2026-10-25.md, dated 2026-10-25), FREE (free-census-2026-11-23.md, dated 2026-11-23), GUIDE (provider-model-effort-guide.md, read 2026-11-23), I13677 (developing-today/code issue #13677 and its 15 comments, read 2026-11-23), LAPTOP (deployed opencode.jsonc, read 2026-11-23 18:41:54 CST).
