# Models by tier, and why

The tiers are route heads, not model names. Decision 008 (Drew, 2026-10-25, read 2026-11-23) bans named-model pins in role defaults: a role states a strength floor and a speed flag, and the resolver picks the model live at dispatch time. This chapter records the six tier route heads in the deployed config (tier keys read on the laptop, 2026-11-23 18:41 CST), what serves each tier as built (config audits and census reads, each dated in the table), and the reasoning the sources record for each placement. The pools behind the legs are in [The plans we use](the-plans.md); the walk rules are in [Pools, routes and the switchboard](pools-and-routes.md).

## The six tiers

| Tier (route head) | What serves it, as built | Why, in the sources' recorded reasoning |
|---|---|---|
| scout | Free cluster first: beam-plan, longcat-plan, iceman-plan, inkling-plan, openrouter-free-plan, commandcode-free-plan, kilo-free-plan (`kilo-auto/free`), then paid legs (free census §5, read 2026-11-23; issue #13677 PR #13691 seating, read 2026-11-23) | Free census §5, read 2026-11-23: "scout leads with beam/longcat/iceman/inkling/openrouter-free before falling to paid legs." Free-first is a standing route rule from Drew's order of 2026-11-23 (issue #13677 body, read 2026-11-23). Zen free models sit last-resort wherever they fit, with limited context, per config comments (free census §5, read 2026-11-23) |
| background | The full free cluster (flash-free, gemini-free, exaone-free, hy4-flash, glm Zen and Z.ai, flint, big-pickle, nemotron x2, nvidia-free, openrouter-free, commandcode-free, kilo-free) before workhorse (free census §5, read 2026-11-23) | Free census §5 tier placement, read 2026-11-23. One recorded exception stands in the free-first pass dated 2026-11-23: background's clock-gated paid chain, a recorded value-order design (issue #13677 comment, read 2026-11-23) |
| speed (also the config's `small_model`, read 2026-11-23) | spark-plan class legs; since 2026-11-07 22:00 CST the speed route's first leg is the direct `_muse/muse-spark-1.3-contributor#xhigh` at weight 3, then deepseek-flash-plan at weight 2, haiku-plan at weight 2, gemini-flash-plan at weight 1 (registry speed-route entry, Drew's direct order, read 2026-11-23) | Meta Spark at its low variant is a Light-tier model (registry capability tiers, read 2026-11-23). Effort on the Meta door is the `reasoning_effort` parameter via variants; no `-low` model ids exist behind this door (probed 2026-10-26, registry, read 2026-11-23). Contributor rule (Drew, 2026-10-26, registry, read 2026-11-23): "whenever a Meta model would be used, use its `-contributor` id instead — it is always cheaper on this plan" |
| workhorse (the config default, read 2026-11-23; also the route Muse itself dispatches OpenCode sessions on, Drew, 2026-11-01, read 2026-11-23) | deepseek-flash-plan at high effort, luna-plan, haiku-plan, Meta spark-contributor, sol-plan#low (registry model audit, deployed-config read dated 2026-10-27) | Efficiency plan ladder, dated 2026-10-25, read 2026-11-23: standard work is "DeepSeek Flash at high effort — still cheaper than most options even if it takes a couple of iterations. On the OpenAI pool, Luna Max is the default for most tasks — it lasts much longer. On the Claude side, Sonnet (medium or high)." POOL MAP, read 2026-11-23: "Default workhorse: DeepSeek Flash on HIGH effort (cheapest sufficient; a couple of iterations still beat one flagship pass)." Effort guide §5, read 2026-11-23: small and cheap models at high to max effort; "the tokens are cheap; effort buys back the capability gap" |
| specialist | sonnet-plan#high, argon-plan#high, spark-plan#max, sol-plan#medium; ordered Claude, Meta, OpenAI, DeepSeek Pro in the audit dated 2026-10-27, and OpenAI last in the audit dated 2026-10-31 (registry cycles 144/168, read 2026-11-23) | Efficiency plan ladder, dated 2026-10-25, read 2026-11-23, Hard tier: "Luna Max at higher effort, Sonnet high, or Opus at low/medium. Medium Sol only when something is genuinely complicated." Effort guide §4, read 2026-11-23: Hard is "Opus class or the strongest open model available" |
| summit | opus-plan#high, argon-plan#high, muse-direct / `_muse` Wonder-contributor at #xhigh to #max, sol-plan#xhigh; OpenAI last (registry audits dated 2026-10-31 and 2026-11-02, read 2026-11-23) | Efficiency plan ladder, dated 2026-10-25, read 2026-11-23, Very complex design: "Opus high — deliberate, not a default. If in doubt, it isn't this class." POOL MAP, read 2026-11-23: "high Opus only for very complex design, deliberate and rare"; "Medium Sol is the routine ceiling on the OpenAI pool". Effort guide §5, read 2026-11-23: flagships at medium by default; "High only for genuinely hardest-tier work; max is almost never cost-efficient here" |

## The 40 deployed pools, by family

The deployed config holds 40 named plan pools (laptop grep, 2026-11-23). Grouped by family, with the grouping made in this chapter on 2026-11-23 from that dated list:

- OpenAI family, 4 pools (laptop grep, 2026-11-23): openai-plan, luna-plan, sol-plan, terra-plan.
- Claude family, 4 pools (laptop grep, 2026-11-23): fable-plan, haiku-plan, opus-plan, sonnet-plan.
- Meta family, 1 pool (laptop grep, 2026-11-23): spark-plan.
- DeepSeek family, 2 pools (laptop grep, 2026-11-23): deepseek-flash-plan, deepseek-pro-plan.
- GLM family, 4 pools (laptop grep, 2026-11-23): glm-5-3-plan, glm-5-3-flash-plan, glm-5-3-flash-zen-plan, glm-flash-free-plan.
- Gemini family, 3 pools (laptop grep, 2026-11-23): gemini-free-plan, gemini-hydrogen-plan, gemini-embed-plan.
- Antigravity family, 1 pool (laptop grep, 2026-11-23): argon-plan.
- Mistral family, 1 pool (laptop grep, 2026-11-23): mistral-plan.
- Free-catalogue plans, 7 pools (laptop grep, 2026-11-23): commandcode-free-plan, exaone-free-plan, flash-free-plan, groq-free-plan, kilo-free-plan, nvidia-free-plan, openrouter-free-plan.
- Free-model plans, 10 pools (laptop grep, 2026-11-23): beam-plan, big-pickle-plan, flint-plan, hy4-flash-plan, iceman-plan, inkling-plan, longcat-plan, mimo-flash-plan, nemotron-lightning-plan, nemotron-vl-plan.
- Utility plans, 3 pools (laptop grep, 2026-11-23): prompt-guard-plan, stt-plan, tts-plan.

The family counts in this section add to 40, the count in the dated laptop grep of 2026-11-23. Tier membership and weights rot with every config PR (pack §5, read 2026-11-23); the tier definitions in Decision 008 (Drew, 2026-10-25, read 2026-11-23) are standing rules and rot slowly.

## Capability tiers for failover

Failover swaps a leg for a like-tier leg. The registry capability tiers (standing text, read 2026-11-23):

- Flagship (registry, read 2026-11-23): openai gpt-6.1-sol / gpt-6-sol / gpt-5.6-sol; opencode-go gpt-5.6-sol; meta muse-wonder-1.0; antigravity claude-opus-5-5.
- Strong (registry, read 2026-11-23): openai gpt-6-luna / gpt-5.6-terra / gpt-5.6-luna; meta muse-spark-1.3; antigravity/google claude-sonnet-5-5; cline-pass glm-5.3, kimi-k3, deepseek-v4-pro; opendesign deepseek-v4-pro, glm-5.3.
- Light (registry, read 2026-11-23): cline-pass glm-5.3-flash, deepseek-v4.1-flash; opendesign deepseek-v4.1-flash, glm-5.3-flash; meta muse-spark-1.3 at its low variant; openai gpt-5.6-luna-fast; opencode big-pickle.

Failover effort rule (registry, read 2026-11-23): prefer the same effort variant; drop at most one step when quota forces a lighter pick; never jump flagship to light without a note in the supervision log.

## Cross-tier rules

- Cheapest sufficient first, measured fresh. The efficiency plan's one rule, dated 2026-10-25 and read 2026-11-23: "Never use more model than the task needs, and never spend money to keep working." The measurement gate is in [Walls, resets and quota doctrine](walls-and-resets.md).
- Family preference, all else equal (registry, Drew, 2026-10-24, read 2026-11-23): "claude first, then openai; gemini sits wherever the specific job fits it". Right-sizing beats family preference, and where the ladder and the older family text disagree, the ladder wins (registry, read 2026-11-23).
- Effort is set per (model, effort) pair, never left at a default variant (effort guide, read 2026-11-23). The effort sizing is in [Pools, routes and the switchboard](pools-and-routes.md).

The harness that runs these tiers is the forks programme's; its own section is [OpenCode and OpenChamber: our forks programme](../opencode-openchamber/index.md).

Sources. LLM evidence pack, `~/workspace/pm-book/survey/llm-evidence.md` §2 (assembled 2026-11-23), and the sources the pack names for §2: REG (provider-quota-registry.md, read 2026-11-23), EFF (model-efficiency-plan-2026-10-25.md, dated 2026-10-25), FREE (free-census-2026-11-23.md, dated 2026-11-23), GUIDE (provider-model-effort-guide.md, read 2026-11-23), I13677 (developing-today/code issue #13677 and its 15 comments, read 2026-11-23), LAPTOP (deployed opencode.jsonc, read 2026-11-23 18:41:54 CST).
