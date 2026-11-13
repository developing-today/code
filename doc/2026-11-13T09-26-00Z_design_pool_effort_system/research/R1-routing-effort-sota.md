# R1 — State of the art: model routing, effort tiers, fallback, and trial methodology

**Program:** design-mill, code#9399 · **Stage:** research input for the pool/effort design
**Brief:** `../brief.md` (read in full) · **Fleet state:** `.opencode/route-fallbacks.md` (read)
**Author route:** opencode-route/workhorse · **Date:** 2026-11-13 · **Status:** draft for orchestrator review, nothing committed

## 0. How to read this report

- Every factual claim cites a URL. Where a claim comes from a third-party aggregator, a blog, or a search snippet rather than the vendor's own page, it is labelled **[third-party]**.
- Claims I could not check against a primary source in this session are labelled **[UNVERIFIED]**. Treat those as leads, not facts.
- Numbers I derived myself (sample sizes, arithmetic on prices) are labelled **[derived]** and show the formula.
- Vendor pricing and model vocabularies changed repeatedly during 2026 in the sources I read. Anything price- or enum-specific must be re-checked at the time it is written into config.
- Coverage limits: I fetched primary docs for OpenRouter (provider routing, reasoning), LiteLLM (routing, session affinity), OpenAI (reasoning), RouteLLM and FrugalGPT (arXiv abstracts), and NixOS (module system). Everything else (Portkey, Not Diamond, Martian, Cloudflare, Kong, Envoy, Bedrock, Azure, Anthropic caching, DeepSeek caching, Gemini thinking, Kustomize, Arena/Bradley-Terry, bandits, always-valid testing) comes from search snippets, which I cite by the page the snippet came from. Requesty, Unify, and Helm/CSS-cascade were not researched in this pass.

---

## 1. Summary of the findings that matter for this fleet

1. **Every serious router separates "which door" from "how much effort".** OpenRouter's default routing picks a provider by measured effective price, throughput, latency, rate limits and reliability, and `sort`, `order`, `only`, `ignore`, and `:floor`/`:nitro` are explicit overrides ([OpenRouter provider routing](https://openrouter.ai/docs/guides/routing/provider-selection)). The fleet's `*-plan` door failover and its effort rungs are the same two axes.
2. **Effort is not a comparable number across providers.** OpenAI uses `none|minimal|low|medium|high|xhigh|max` with model-dependent support (GPT-6 Astra rejects `none`; GPT-6.1 Sol rejects `none` and `minimal` and defaults to `medium`) ([OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)). Google's Gemini 3.x uses `thinkingLevel` with a per-model enum and cannot disable thinking on 3.x; `thinkingBudget` is the 2.5 mechanism and the two cannot be combined ([Google Cloud Thinking](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/thinking), [Firebase AI Logic thinking](https://firebase.google.com/docs/ai-logic/thinking)). Anthropic exposes `output_config.effort` (low/medium/high/xhigh/max, per model) and, on Haiku 4.5, only a manual `budget_tokens` ([Amazon Bedrock adaptive thinking](https://docs.aws.amazon.com/bedrock/latest/userguide/claude-messages-adaptive-thinking.html), [iBuildWith effort guide](https://www.ibuildwith.ai/blog/effort-thinking-opus-4-7-changed-the-rules)). **The fleet's `#low/#medium/#high` labels are therefore per-model vocabularies that happen to share names.** A tier map must be keyed by (model, effort), never by the effort word alone.
3. **Effort moves quality and cost non-linearly and differently per model.** Artificial Analysis (AA) v5 reports Opus 5.5 at 35 points on low and 58 on max, at roughly 10× the cost per Intelligence Index task; Sonnet 5.5 spans 29 points low→max; GPT-6.1 Sol's spread doubled to 20 points ([AA Intelligence Index v5](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v5)). Higher effort is not monotonic in quality for every task: there is published evidence of overthinking, where more reasoning tokens lower accuracy ([OptimalThinkingBench](https://arxiv.org/html/2508.13141v1), [When More Thinking Hurts](https://arxiv.org/html/2604.10739v1)). This supports the brief's "punishes effort above high" observation as a general phenomenon, not a quirk of one vendor.
4. **The strongest prior art for "equivalence" is a quality band, not a point match.** Azure Model Router's Balanced mode picks the cheapest model whose predicted quality is within roughly 1–2% of the best for that prompt; Cost mode uses a wider 5–6% band ([Microsoft Learn: model router](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router)). Bedrock Intelligent Prompt Routing does the same with a fallback model as the 100% reference and a configurable "response quality difference" ([AWS Bedrock IPR](https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-routing.html)). The brief's "summit low ≈ workhorse high" is exactly this shape: an equivalence is "within band δ of reference, at lower cost", and the band is the tunable.
5. **Frontier math is standard and cheap to adopt.** RouterBench plots each router's cost against quality and takes the non-decreasing convex hull as the achievable frontier; AIQ is the average quality along that hull over a shared cost interval ([RouterBench, alphaXiv](https://www.alphaxiv.org/abs/2403.12031)). A (model, effort) point is dominated if another point is at least as good on quality and cheaper, or at least as cheap and better; dominated points are never worth a leg.
6. **Trial methodology is the weak point of nearly every routing product.** Most gateways ship weights and fallbacks but not a statistically sound way to decide whether a swap is safe. For the fleet's low-volume reality, the honest answer is: at p≈0.9 success, you need on the order of 400 tasks per arm to see a 5-point drop and 3,000+ to see a 2-point drop (§4.4). Bandits (Thompson/UCB) allocate traffic well but do not give a decision with a stated error rate; always-valid p-values allow peeking but cost power. Use bandits for allocation inside a pre-approved safe set, and a fixed or always-valid test for the swap decision.
7. **Overlays have solved versions of this problem.** NixOS priorities (`mkDefault`=1000, `mkForce`=50, lower number wins, equal priority conflicts are errors; `mkBefore`/`mkAfter` for list order) and Kustomize strategic-merge/JSON 6902 patches map directly onto "set / remove / replace a model-effort" ([NixOS manual](https://nixos.org/manual/nixos/stable), [Noogle mkOverride](https://noogle.dev/f/lib/modules/mkOverride), [Kubernetes Kustomize docs](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization)). The NixOS choice to make equal-priority conflicts loud is the most important property to copy.
8. **Cache economics dominate cost for agentic workloads and change the calculus of switching effort or door mid-session.** Anthropic cache writes cost 1.25× base input (5-min TTL) or 2× (1-h TTL), reads 0.1× ([Claude prompt caching docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)). DeepSeek caching is automatic, with cache-hit input priced at about 2% of miss on the Flash tier and half price off-peak [third-party: [deepseekai.guide](https://deepseekai.guide/api/deepseek-api-pricing), [maximem](https://www.maximem.ai/blog/deepseek-api-pricing-2026)]. Switching door or effort mid-conversation can invalidate the prefix. OpenAI's `configuration_update` item is the one vendor mechanism that changes effort mid-conversation without changing the cached prefix ([OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)); OpenRouter mirrors it ([OpenRouter reasoning](https://openrouter.ai/docs/use-cases/reasoning-tokens)).

---

## 2. LLM routers and gateways

For each: what it does · mechanism · pros · cons · **LEARN / COPY / AVOID** for the fleet · URL.

### 2.1 OpenRouter

- **What:** hosted gateway with per-request provider selection across many door-equivalents per model; `openrouter/auto` (task-classified, market-spend driven) and `openrouter/pareto-code` (min coding score) as routers ([Auto Router](https://openrouter.ai/openrouter/auto), [Pareto Router](https://openrouter.ai/docs/guides/routing/routers/pareto-router)).
- **Mechanism (provider routing):** default routing weights providers by effective price (with cache hit rate), throughput, TTFT, 429 rate, error rate and tool-call quality; traffic is random, weighted by "closeness to best"; a provider with far worse errors or 429s gets zero traffic until it recovers; follow-ups stay on the same provider while its prompt cache is warm ([provider routing](https://openrouter.ai/docs/guides/routing/provider-selection)). Explicit overrides: `order` (try in order, fallbacks on unless `allow_fallbacks: false`), `only`, `ignore`, `sort` (price/throughput/latency, which bypasses default routing), `preferred_min_throughput` and `preferred_max_latency` with p50/p75/p90/p99 cutoffs (preferences that deprioritise rather than exclude), `max_price`, `require_parameters`, `:floor` (sort by price, plus flex tier) and `:nitro` (sort by throughput, plus priority tier) ([provider routing](https://openrouter.ai/docs/guides/routing/provider-selection)).
- **Model fallbacks:** a `models` array; `sort.partition` defaults to `"model"`, meaning the primary model's endpoints are always tried first. `partition: "none"` sorts globally across models, which is the mechanism for "cheapest model that meets a speed floor" ([provider routing, Advanced Sorting](https://openrouter.ai/docs/guides/routing/provider-selection)).
- **Effort normalization:** one `reasoning` object: `effort` (`max|xhigh|high|medium|low|minimal|none`) or `max_tokens`. Effort is translated to a fraction of the request's output budget: max/xhigh 0.95, high 0.8, medium 0.5, low 0.2, minimal 0.1. Anthropic `budget_tokens = max(min(max_output × ratio, 128000), 1024)`. Discovery: `GET /api/v1/models` returns per-model `reasoning.supported_efforts` (descending), `default_effort`, `default_enabled`, `supports_max_tokens`, `mandatory` ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)). Gemini 3 maps `effort` to `thinkingLevel`, with `xhigh→high` and "map to nearest supported level" when a model lacks a value ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)). **Note the translation is lossy and the target is server-side; OpenRouter documents the mapping, not the quality effect.**
- **Mid-conversation effort:** a `configuration_update` item (Responses) / system message with `configuration_update` (Chat) / `output_config` (Messages) changes effort from a point in the transcript; cache stays warm; adjacent updates rejected; unsupported effort on a model returns 400 instead of silently dropping ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)).
- **Pros:** the best public documentation of a per-model capability catalogue (`supported_efforts`); explicit percentile-based performance preferences; sticky-while-cached behaviour; mid-conversation effort.
- **Cons:** routing internals are a black box beyond the documented inputs; "default routing" can change the door under you (the fleet must pin); `preferred_*` are soft.
- **LEARN:** per-model capability discovery (`supported_efforts`) — the fleet should keep its own version of this in config, because OpenAI and Gemini reject unsupported values with 400 (per OpenAI and Google docs above). **COPY:** the "preference that deprioritises rather than excludes" pattern for speed (the brief's "speed in small/scout" rule). **COPY:** the `partition` idea — tiers that span models need a deliberate global sort. **AVOID:** silent remapping of an unsupported effort. The fleet's `#medium→#high` remap on the Go and ClinePass doors (brief §9) is exactly the failure OpenRouter rejects with a 400 on the mid-conversation path.
- **Source:** [provider routing](https://openrouter.ai/docs/guides/routing/provider-selection) · [reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens) · [Auto Router](https://openrouter.ai/openrouter/auto)

### 2.2 LiteLLM Router / Proxy

- **What:** open-source router inside the SDK and proxy: deployment groups by `model_name`, strategies (simple-shuffle default with rpm/tpm/weight; latency-based; least-busy; cost-based; rate-limit aware), routing groups with per-group strategy, cooldowns, retries, fallbacks, context-window fallbacks, budgets ([LiteLLM routing](https://docs.litellm.ai/docs/routing)).
- **Mechanism worth copying:**
  - **`order`** on a deployment: lower order is tried first; a failure escalates to the next order; equal orders are load-balanced by the strategy. This is an explicit, numeric tier mechanism — the same shape as NixOS priorities (§2.11).
  - **Weighted failover** (`enable_weighted_failover`): retry inside the same model group by re-picking among peers with weights, excluding deployments that just failed, capped by `max_fallbacks` (default 5); only then cross-group fallbacks; not triggered for context-window or content-policy errors ([LiteLLM routing](https://docs.litellm.ai/docs/routing)).
  - **Max-parallel as a hard 429** rather than a queue (the proxy returns 429 at the per-deployment cap and the router fails over), a documented behaviour change from earlier queueing ([LiteLLM routing](https://docs.litellm.ai/docs/routing)).
  - **Cooldowns per deployment**, with allowed-fails policies per error class, including 429 → immediate cooldown, so a single bad door cannot take a whole group down ([LiteLLM routing](https://docs.litellm.ai/docs/routing)).
  - **Retry-count precedence** (header > request body > deployment > global) and a deliberate rule that the provider SDK retries are pinned to 0 so retries do not multiply ((1+N)² upstream calls) ([LiteLLM routing](https://docs.litellm.ai/docs/routing)). This is the precise bug to avoid if the fleet ever retries at both the harness and the gateway.
  - **Session affinity:** pin a conversation to the deployment that served its first request via `x-litellm-session-id`; pins expire after an idle TTL (default 3600 s); if the pinned deployment is cooled down the request falls through, and the pin is kept so the session returns when healthy; optional re-pin on failover ([LiteLLM routing, Session Affinity](https://docs.litellm.ai/docs/routing)).
  - **Daily quotas** (`rpd`, `tpd`, `daily_reset_time` in UTC) enforced to move traffic off a spent deployment ([LiteLLM routing](https://docs.litellm.ai/docs/routing)). Directly relevant to the fleet's per-plan quota windows.
  - **Traffic mirroring** to a silent model for evaluation, which does not affect the primary response's latency ([LiteLLM routing, Traffic Mirroring](https://docs.litellm.ai/docs/routing)). This is the shadow-traffic primitive for §4.
  - **Context-window pre-call checks** filter deployments whose window is too small for the request ([LiteLLM routing, pre-call checks](https://docs.litellm.ai/docs/routing)).
- **Pros:** explicit numeric order, per-group strategies, daily quotas, shadow traffic, affinity. Closest in spirit to the fleet's chain-of-pools.
- **Cons:** the documentation itself warns that behaviour differs by version and by strategy (for example session affinity does not pin under `usage-based-routing` v1 or under sync methods); a lot of surface.
- **LEARN / COPY:** numeric `order` as the tier mechanism, with equal-order = weighted within tier; per-deployment `allowed_fails`; daily-quota movement; session affinity for cache warmth. **AVOID:** multiplying retries across layers (the `(1+N)²` problem); queueing at a cap (it hides latency). **Mapping:** `order` ↔ the fleet's chain position; `weight` ↔ the fleet's round-robin `w` values; `routing group` ↔ a fleet class route.
- **Source:** [docs.litellm.ai/docs/routing](https://docs.litellm.ai/docs/routing)

### 2.3 Portkey

- **What:** managed AI gateway with a JSON "config" object composing three strategies: `fallback`, `loadbalance`, `conditional` ([Portkey conditional routing](https://docs.portkey.ai/docs/product/ai-gateway/conditional-routing)).
- **Mechanism:** conditional routing evaluates request metadata and parameters with sequential `query`/`then` conditions and a required default; targets are composable (a conditional target can itself be a load balancer or fallback chain) ([Portkey conditional routing](https://docs.portkey.ai/docs/product/ai-gateway/conditional-routing), [weaveos review](https://weaveos.com/blog/best-llm-routers-and-model-routing-tools-in-2026) [third-party]).
- **Pros:** composability is the key idea: fallback, load balance and conditional are siblings, nestable. Each conditional branch carries its own fallback chain, so one family's outage does not affect another.
- **Cons:** SaaS config; the strategies are rule-based, not learned; I did not verify the current config schema.
- **LEARN / COPY:** each class/tier route owns its own fallback chain (already true in the fleet's `route-fallbacks.md`); "isolate failures between families" is the reason the fleet's class edges are per route. **AVOID:** a required default that silently absorbs unmatched traffic; the fleet's guard should fail on unmatched keys.
- **Source:** [Portkey conditional routing](https://docs.portkey.ai/docs/product/ai-gateway/conditional-routing)

### 2.4 Martian, Not Diamond (learned routers)

- **What:** commercial learned routers that predict which model will answer a prompt best and return a recommendation executed through a gateway ([Not Diamond: model routing vs gateways](https://www.notdiamond.ai/blog/model-routing-vs-gateways-breaking-down-the-difference), [Not Diamond: comprehensive guide](https://www.notdiamond.ai/blog/a-comprehensive-guide-to-model-routing)). Martian is still listed as active with a Model Router and provider leaderboard ([QVeris guide](https://qveris.ai/guides/martian-llm-router-alternatives) [third-party]).
- **Mechanism:** Not Diamond's own taxonomy: deterministic routing (rules, fallback on latency, percentage splits) vs intelligent routing (content-aware). Their guidance is to use both: an intelligent router for the primary decision, deterministic fallbacks for failures ([Not Diamond guide](https://www.notdiamond.ai/blog/a-comprehensive-guide-to-model-routing)). Vendor claims about savings are not independently verified; the IntuitionLabs review says so explicitly ([IntuitionLabs](https://intuitionlabs.ai/articles/ai-model-routing-cost-quality) [third-party]).
- **LEARN:** the gateway-vs-router split. A gateway governs deterministic access (auth, quota, failover); a router decides which model handles a request. The fleet already has the gateway (pools, doors) and is designing the router (auto/category pools). **AVOID:** adopting a learned router before there is labelled outcome data for the fleet's own tasks; the fleet's stats store (brief §6) is the prerequisite.
- **Source:** [Not Diamond blog](https://www.notdiamond.ai/blog/model-routing-vs-gateways-breaking-down-the-difference), [Not Diamond guide](https://www.notdiamond.ai/blog/a-comprehensive-guide-to-model-routing)

### 2.5 RouteLLM (LMSYS)

- **What:** open framework that trains routers on human preference data to choose between a strong and a weak model at inference time, trading cost against quality ([arXiv:2406.18665](https://arxiv.org/abs/2406.18665)).
- **Mechanism:** formulated as a binary choice; the paper compares several router models (matrix factorisation, BERT classifiers, causal LLM variants) and reports cost–quality curves; abstract claims over 2× cost reduction "in certain cases" without quality loss, and transfer when the strong/weak models change at test time ([arXiv:2406.18665](https://arxiv.org/abs/2406.18665)). I did not read the body; the specific router names and curve numbers are **[UNVERIFIED]** here.
- **Pros:** the transfer result matters for the fleet: a router trained on one strong/weak pair survived swapping the models. That is the property the fleet needs when models change every few months.
- **Cons:** trained on Arena-style pairwise preferences, which may not match agentic coding tasks; needs a preference dataset.
- **LEARN:** the binary strong/weak framing maps onto the fleet's escalation (workhorse ↔ summit). **COPY (later, V4):** train a router on the fleet's own labelled outcomes using the same cost–quality curve method. **AVOID:** using a router trained on general chat for agentic work without a fleet-specific validation set.
- **Source:** [arXiv:2406.18665](https://arxiv.org/abs/2406.18665)

### 2.6 RouterBench

- **What:** benchmark plus evaluation framework for multi-LLM routers: a dataset of over 405k inference outcomes across models and tasks, and a "Zero Router" and "Oracle Router" as lower and upper bounds ([RouterBench, alphaXiv](https://www.alphaxiv.org/abs/2403.12031); [paper page](https://huggingface.co/papers/2403.12031)).
- **Mechanism:** each router yields a cost–quality operating curve; the frontier is the non-decreasing convex hull (mixing two systems is allowed); AIQ = average quality over a shared cost interval ([RouterBench, alphaXiv](https://www.alphaxiv.org/abs/2403.12031)). The repo is marked stale (last push June 2024) [third-party aggregator note].
- **LEARN / COPY:** the evaluation method, not the dataset. The fleet's "cost per %-of-quota vs quality" comparison should be a convex-hull frontier, and AIQ-style averages over a cost band are the right single number to rank (model, effort) points. **AVOID:** comparing routers at one operating point; always report the curve.
- **Source:** [arXiv:2403.12031](https://arxiv.org/abs/2403.12031), [withmartian/routerbench](https://github.com/withmartian/routerbench)

### 2.7 Cloudflare AI Gateway (Dynamic Routing, fallbacks)

- **What:** gateway-level fallbacks (ordered list, `cf-aig-step` header reports which step served) and Dynamic Routing: a visual/JSON flow with Start, Conditional (request body/header/metadata expressions), Percentage (weighted split, A/B and gradual rollout), Model, Rate Limit, Budget Limit, and End nodes; publish a new route version without redeploying ([Cloudflare dynamic routing](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing), [fallbacks](https://developers.cloudflare.com/ai-gateway/configuration/fallbacks)).
- **Mechanism worth copying:** a **Budget Limit node** that routes to a fallback model when a cost quota is reached, so cost is a routing signal rather than a hard 429. **Percentage node** as the built-in canary primitive.
- **Limitation found:** Dynamic Routing is chat-completions only in the cited issue, so it could not drive models that require the Responses API; budget-driven graceful fallback was therefore blocked for those models as of the issue date ([cloudflare-docs#29840](https://github.com/cloudflare/cloudflare-docs/issues/29840)). **[UNVERIFIED as current]**.
- **LEARN:** budget-as-route-condition and the `step` header (report which fallback step served; the fleet already records this in stats). **COPY:** percentage split as the trial primitive. **AVOID:** a cost gate that returns 429 instead of degrading; the fleet's quota walls should move traffic down the class chain (the existing second-pass semantics already do this).
- **Source:** [Dynamic routing](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing), [Fallbacks](https://developers.cloudflare.com/ai-gateway/configuration/fallbacks), [issue 29840](https://github.com/cloudflare/cloudflare-docs/issues/29840)

### 2.8 Kong AI Gateway (AI Proxy Advanced)

- **What:** AI load balancer with algorithms: round-robin (weighted), consistent-hashing (sticky by header), least-connections, lowest-latency, lowest-usage (token/cost), semantic (prompt-to-model similarity via embeddings + vector DB), and **priority** (tiered failover across groups; the highest group takes all traffic until it fails; `weight` within a group) ([Kong AI proxy advanced](https://developer.konghq.com/plugins/ai-proxy-advanced), [Kong load balancing](https://developer.konghq.com/ai-gateway/load-balancing)).
- **Retry and failover:** `failover_criteria` decides what triggers failover (client errors do not; add `http_429`, `http_502`, and `non_idempotent` for POST) ([Kong AI proxy advanced](https://developer.konghq.com/plugins/ai-proxy-advanced)).
- **LEARN / COPY:** the **priority-group** model (a group is a tier; within the group, weights; a tier is only entered when the one above is wholly unavailable) is the cleanest description of the fleet's "regular chain first, fallback is second pass" rule. The fleet's rule is stricter than Kong's (Kong does not have the visited-set walk). **AVOID:** semantic routing without a labelled dataset; it needs a vector store and a description per model.
- **Source:** [developer.konghq.com/plugins/ai-proxy-advanced](https://developer.konghq.com/plugins/ai-proxy-advanced), [ai-gateway/load-balancing](https://developer.konghq.com/ai-gateway/load-balancing)

### 2.9 Envoy AI Gateway / Agent Router

- **What:** Envoy-based AI gateway (now branded Agent Router, same code), with provider fallback, `modelNameOverride` for model virtualisation, token-aware rate limiting and `QuotaPolicy` for token budgets per time window, and per-backend priority groups ([kgateway failover](https://kgateway.dev/docs/envoy/2.1.x/ai/failover), [Agent Router](https://theagentrouter.ai), [Agent Router releases](https://github.com/theagentrouter/agent-router/releases)).
- **Mechanism worth copying:** failover is **priority groups in list order**; models in one group are round-robined; a higher group is only used when all its members are down ([kgateway failover](https://kgateway.dev/docs/envoy/2.1.x/ai/failover)). **Stream idle timeout with failover**: if the upstream sends nothing for N seconds before the first token, retry the next backend; after the first token it returns 504 ([Agent Router releases](https://github.com/theagentrouter/agent-router/releases)). This is the correct rule for the fleet's streaming pools: fail over before the first token, never mid-stream.
- **Source:** [kgateway failover](https://kgateway.dev/docs/envoy/2.1.x/ai/failover), [Agent Router releases](https://github.com/theagentrouter/agent-router/releases)

### 2.10 AWS Bedrock Intelligent Prompt Routing; Azure AI Foundry Model Router

- **Bedrock IPR:** routes between models *within one family* (for example Claude Sonnet and Haiku, Llama 70B and 8B) by predicting each model's response quality for each prompt. Configured with a **response-quality-difference threshold** relative to a **fallback model** that defines 100% quality; the router picks the cheapest model whose predicted quality is within the threshold of the fallback. Raising the threshold makes it more aggressive about the cheap model ([AWS IPR docs](https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-routing.html), [AWS ML blog](https://aws.amazon.com/blogs/machine-learning/use-amazon-bedrock-intelligent-prompt-routing-for-cost-and-latency-benefits) [the blog's example: a 10% threshold with Haiku as fallback]). Adds about 85 ms P90 overhead [third-party: [AWS Medium post](https://medium.com/aws-tip/amazon-bedrock-intelligent-prompt-routing-optimize-cost-and-quality-with-automatic-model-selection-4efa40bd8796)]. Not available in batch [AWS re:Post](https://repost.aws/questions/QUtUiffLebQvqjentL0AvX2w/technical-inquiry-intelligent-prompt-routing-in-amazon-bedrock-batch-interface).
- **Azure Model Router:** one deployment, a subset of models, and three modes. **Balanced** (default): cheapest model within about 1–2% quality of the best for that prompt. **Cost:** a wider band of about 5–6%. **Quality:** the best model regardless of cost. The effective context window is capped by the smallest model in the subset; the subset is also the fallback set; use at least two models in the subset ([Microsoft Learn: model router](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router), [how to use](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/model-router), [Foundry failover note](https://medium.com/@badrkacimi/the-model-router-explained-intelligent-cost-performance-optimization-in-azure-ai-foundry-c2614a403471) [third-party]). Mode changes take up to five minutes to apply ([how to use](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/model-router)).
- **LEARN / COPY (most important in this section):** express equivalence as a **quality band relative to a reference model**, and make the band the tunable: `low = cheapest within wide band`, `high = best within narrow band`. This directly answers the brief's "summit low ≈ workhorse high" question: define the equivalence as "within δ of reference on the fleet's own measured quality", and calibrate δ per tier. **COPY:** the rule that the subset/pool's smallest context caps the whole pool (brief §3 already states this as "strictest limit"). **AVOID:** mixing routing modes and per-model rate limits; Azure says the TPM applies to the router as a whole, and per-model limits are a trap.
- **Sources:** [AWS IPR](https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-routing.html), [Microsoft model router](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router), [Microsoft how-to](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/model-router)

### 2.11 OpenRouter Pareto Router (min_coding_score bands) — as a design pattern

- **What:** `openrouter/pareto-code` takes `min_coding_score` in [0,1] and selects the cheapest model that clears that bar, ranked by AA coding percentiles. Bands: ≥0.66 high (top of AA coding field), 0.33–0.66 medium, <0.33 low (capable coders beating AA's median); default high ([Pareto Router docs](https://openrouter.ai/docs/guides/routing/routers/pareto-router)). A `:nitro` variant re-ranks the tier by measured throughput ([OpenRouter Pareto announcement](https://x.com/OpenRouter/status/2053170520087024109) [social post, weak source]).
- **LEARN / COPY:** this is the cleanest public example of a **single scalar quality floor** with the tier implemented as a band over a ranked list. It is the same structure as the Azure band, expressed with a score. The fleet's "low/medium/high" can be the same thing: a floor on a measured score, with the selection being cheapest-above-floor. **AVOID:** setting a floor on a benchmark score that is not the fleet's own task distribution; AA coding percentiles are not agentic-fleet success rates.
- **Source:** [Pareto Router](https://openrouter.ai/docs/guides/routing/routers/pareto-router)

### 2.12 Cross-cutting: router vs gateway vs cascade

The taxonomy in [Not Diamond](https://www.notdiamond.ai/blog/a-comprehensive-guide-to-model-routing) and the 2026 survey ([Dynamic Model Routing and Cascading survey](https://arxiv.org/html/2603.04445v2)) separates: **deterministic routing** (fixed rules: fallback, percentage, static), **pre-generation learned routing** (classify the query, pick a model), **post-generation cascading** (run cheap first, escalate on low confidence). The fleet's current design is deterministic plus class fallback; the brief's self-switching ("a model may suggest its own pool") is a learned/agent-chosen pre-generation signal with its own risks.

---

## 3. Effort / reasoning-budget semantics across providers, and what the evidence says

### 3.1 Vendor vocabularies (what a `#high` actually is)

| Provider / API | Control | Values and per-model constraints | Default | Source |
|---|---|---|---|---|
| OpenAI (Responses) | `reasoning.effort` | `none, minimal, low, medium, high, xhigh, max`; GPT-6 Astra rejects `none` (400); GPT-6.1 Sol rejects `none` and `minimal`; GPT-5.5 default `medium` | `medium` on GPT-6.1 Sol/Sol/Luna | [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning) |
| OpenAI | `reasoning.mode` | `standard` / `pro` (independent of effort) on GPT-5.6+ | `standard` | [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning) |
| OpenAI | `configuration_update` item | change effort (and text format) mid-conversation; keeps prefix cache; cannot be combined with auto-compaction/truncation | — | [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning) |
| Anthropic (Messages) | `output_config.effort` | `low, medium, high, xhigh, max` (per model: Opus 4.6/Sonnet 4.6 low–max; Opus 4.7 low–max; Haiku 4.5 not supported); manual `thinking.budget_tokens` on Haiku 4.5 | model default (`medium` on Opus 5.5 per Bedrock) | [Bedrock adaptive thinking](https://docs.aws.amazon.com/bedrock/latest/userguide/claude-messages-adaptive-thinking.html), [LiteLLM Anthropic effort](https://docs.litellm.ai/docs/providers/anthropic_effort), [iBuildWith](https://www.ibuildwith.ai/blog/effort-thinking-opus-4-7-changed-the-rules) |
| Anthropic | `thinking.type` adaptive/enabled/disabled | Opus 4.7 adaptive-only; disabling thinking forbids `xhigh`/`max` on some models (400) | off on several | [Bedrock adaptive thinking](https://docs.aws.amazon.com/bedrock/latest/userguide/claude-messages-adaptive-thinking.html) |
| Google Gemini 3.x | `thinkingLevel` | Per model: 3.1 Pro `low/medium/high` (no `minimal`, 400); some models `MINIMAL..HIGH`; default `HIGH` on 3.1 Pro; cannot disable on 3.x | model-specific | [Google Cloud Thinking](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/thinking), [LaoZhang](https://blog.laozhang.ai/en/posts/gemini-3-1-pro-thinking-level) [third-party] |
| Google Gemini 2.5 | `thinkingBudget` | integer; 2.5 Flash 0–24,576 (0 disables); 2.5 Pro 128–32,768 (cannot disable) | dynamic (-1) | [Firebase AI Logic](https://firebase.google.com/docs/ai-logic/thinking) |
| Google (combined) | — | `thinkingLevel` and `thinkingBudget` in one request → 400 | — | [Google Cloud Thinking](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/thinking) |
| Mistral | `reasoning_effort` | Docs say `low/medium/high` (recommended `high`); model cards for Medium 3.5 say `none`/`high` only | — | [Mistral reasoning](https://docs.mistral.ai/capabilities/reasoning), [Ollama model card](https://ollama.com/library/mistral-medium-3.5) [third-party: contradicts the Mistral doc, verify on the live API] |
| DeepSeek | reasoning / effort | **[UNVERIFIED]** — not checked in this pass; the fleet's own note says `#medium` silently remaps to `#high` on two doors (brief §9) | — | — |
| OpenRouter (normalised) | `reasoning.effort` / `max_tokens` | `max|xhigh|high|medium|low|minimal|none` → fraction of output budget (0.95/0.95/0.8/0.5/0.2/0.1); Anthropic budget = clamp(out×ratio, 1024, 128000) | — | [OpenRouter reasoning](https://openrouter.ai/docs/use-cases/reasoning-tokens) |

**What this means for the fleet:** (a) an effort rung must be stored as a pair `(model, provider-door, effort-string)` and validated against the model's advertised set, not a global `low/medium/high`; (b) "`xhigh`/`max`/`fast` → `high`" (brief §4) is a safe alias only if written as data per model, because on Opus `max` exists and on Haiku it does not; (c) on Gemini 3.x you cannot express "off", so "low" is the floor, which matters for the fleet's `#low` rungs; (d) on Anthropic, `effort` also changes tool-use behaviour, not only reasoning depth (see 3.3).

### 3.2 How gateways normalise effort

- OpenRouter translates one `effort` into each provider's vocabulary and documents the fraction of output tokens reserved for reasoning ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)). It returns `supported_efforts` so UIs can filter ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)). It rejects an unsupported mid-conversation effort with 400 rather than dropping it ([reasoning tokens](https://openrouter.ai/docs/use-cases/reasoning-tokens)).
- Reasoning tokens count against `max_output_tokens` on most providers; if the budget is used up by reasoning, the response is `incomplete` with no visible output but is still billed. OpenAI recommends reserving at least 25,000 tokens for reasoning+output when starting ([OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning), [OpenRouter reasoning](https://openrouter.ai/docs/use-cases/reasoning-tokens)). **This is a context-limit fact the fleet must encode per tier**: a high-effort rung with a small output cap will return empty.
- LiteLLM maps `reasoning_effort` onto `output_config` for Claude models that support it, and raises a validation error for `max` on models that do not ([LiteLLM Anthropic effort](https://docs.litellm.ai/docs/providers/anthropic_effort)).

### 3.3 Evidence on cost/quality versus effort

- **AA v5 (vendor benchmark):** Opus 5.5 low 35 → max 58; cost per Intelligence Index task $0.98 (low) vs $10.04 (max); GPT-6.1 Sol (max) within 4 points of Opus 5.5 (max) at about one-eighth the cost; GPT-6 Astra (max) $6.24 per task ([AA v5](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v5)). Output tokens per task rose from a median 35k (v4.3) to 59k (v5), and agentic evaluations are a median ~90% of output tokens ([AA v5](https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v5)). Implication: **the cost of an effort rung is dominated by the agentic evaluations**, so the fleet's own agentic task mix, not AA's average, sets the cost curve.
- **AA GPT-5 effort study:** token usage varied 23× between high and minimal effort; GPT-5 high used 82M tokens to complete the index vs 3.5M for minimal ([AA GPT-5](https://artificialanalysis.ai/articles/gpt-5-benchmarks-and-analysis)).
- **Third-party per-effort table for GPT-6 Sol:** low 50.3 / $0.175 per task; medium 51.3 / $0.271; high 54.7 / $0.387; xhigh 61.3 / $0.582; max 57.0 / $0.985 — note max scores *below* xhigh on this aggregator's measure [third-party: [AI Agent Store](https://aiagentstore.ai/ai-models/reasoning-effort), data reviewed 2026-11-10; **[UNVERIFIED]** against AA's own page]. This is the kind of non-monotonic point the fleet needs to measure itself: the curve can peak below the top rung.
- **Overthinking:** on OptimalThinkingBench, most thinking models generate at least 100 thinking tokens on trivial queries, most more than 700 ([OptimalThinkingBench](https://arxiv.org/html/2508.13141v1)). Phi-4-reasoning used ~16× the tokens of Phi-4 for a 6.7-point accuracy deficit, and constraining it to 1,024 tokens dropped accuracy to 53% ([Do LLMs Overthink Basic Math, ACL Findings 2026](https://aclanthology.org/2026.findings-acl.1285.pdf)). A separate study finds accuracy has an inverted-U against chain-of-thought length and that extra budget can cause answers that were correct to be abandoned ([When More Thinking Hurts](https://arxiv.org/html/2604.10739v1)). **Consequence:** "high" can be *worse* on easy tasks, which is the quantitative basis for the brief's "low or medium, or don't overlay" rule.
- **Test-time compute theory:** Snell et al. find that adaptive, per-prompt allocation of test-time compute can match a model ~14× larger on problems where the small model has non-trivial success, and that the best allocation depends on prompt difficulty ([arXiv:2408.03314](https://arxiv.org/abs/2408.03314), ICLR 2025 [PDF](https://proceedings.iclr.cc/paper_files/paper/2025/file/1b623663fd9b874366f3ce019fdfdd44-Paper-Conference.pdf)). Fleet reading: difficulty-aware effort is the right abstraction, and the fixed tier map is the baseline it must beat.
- **Effort is not pure reasoning depth on Claude:** a practitioner analysis found that Anthropic's `effort` also governs how many tool calls Opus 4.6 issues and how thoroughly it cross-checks, and whether it follows methodology instructions, so low effort can look like laziness in agent loops [futuresearch.ai](https://futuresearch.ai/blog/claude-effort-parameter) [third-party, single-author analysis; **[UNVERIFIED]** beyond the author's own tests]. **Consequence:** equivalence for Claude effort rungs must be measured on task success and tool-call count, not tokens.

---

## 4. Cross-model equivalence and cost–quality frontier methods

### 4.1 Pareto frontier and dominance

- **Frontier:** for each (model, effort) point with a cost-per-task c and quality q, the achievable set is the convex hull of points, non-decreasing in q with c ([RouterBench](https://www.alphaxiv.org/abs/2403.12031)). Points strictly below the hull are **dominated** (another point is at least as good on q at lower or equal c).
- **Dominated-model detection (for the brief's "replace clear successors or dominated models"):** a model M is dominated in pool P if there exists a model N in P such that q(N) ≥ q(M) − ε, c(N) ≤ c(M), and latency(N) ≤ latency(M) (for the fleet: N is at least as good on the success metric within the measurement's confidence interval). Use a non-inferiority test for ε rather than a point comparison; see §4.4.
- **Per-task versus per-benchmark-point cost:** AA reports cost per Intelligence Index *task*, weighted across evaluations and using measured cache rates and provider token counts ([AA methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking)). The brief's "cost per point bracket" should be cost per task divided by the success-rate delta against a reference, and only reported where the delta is significant.

### 4.2 Elo and Bradley–Terry for ranking

- Chatbot Arena fits a Bradley–Terry model to pairwise human votes, where P(i≻j) = e^{s_i}/(e^{s_i}+e^{s_j}); Elo is the online version of the same model ([Chatbot Arena arXiv:2403.04132](https://arxiv.org/abs/2403.04132), [alphaXiv summary](https://www.alphaxiv.org/abs/2403.04132)). Their paper treats the ranking as static and fits BT directly for stability ([Colab notebook, Arena](https://colab.research.google.com/drive/1KdwokPjirkTmpO_P1WByFNFiqxWQquwH) [weak source]). Arena's statistical extensions add uncertainty via bootstrap and E-values ([alphaXiv](https://www.alphaxiv.org/abs/2403.04132)).
- **Use for the fleet:** when the fleet has a pairwise judge (two outputs for the same task, a judge or human picks one), fit BT over (model, effort) points. This gives a single strength per point with a confidence interval, comparable across pools, and handles the case where pools never see the same tasks through a shared-opponent design. **Caveat:** BT assumes a single strength per item; agentic tasks may need per-task-class strengths (the Arena extension to "subsystem" strengths is the relevant idea ([Arena extended BT](https://arena.ai/blog/extended-arena))).

### 4.3 Cascades

- **FrugalGPT (Chen, Zaharia, Zou, 2023):** learns an ordered cascade and response scorers; reports matching GPT-4's performance with up to 98% cost reduction, or +4% accuracy at the same cost ([arXiv:2305.05176](https://arxiv.org/abs/2305.05176)). The method is prompt adaptation, LLM approximation, and LLM cascade.
- **Mixture of Thoughts cascade (Yue et al., 2023/2024):** a weak model samples several reasoning chains; agreement among them is the escalation signal. Reported ~0.929 accuracy at ~40% of GPT-4's cost, versus 0.931 for GPT-4 alone, on six reasoning datasets ([arXiv:2310.03094](https://arxiv.org/abs/2310.03094)). A 2025 follow-up (C3PO) notes MoT incurs extra sampling cost and lacks budget guarantees ([C3PO arXiv:2511.07396](https://arxiv.org/html/2511.07396v1)).
- **Fleet reading:** a cascade is precisely the fleet's second-pass fallback, but triggered by a *quality signal* (failed tests, low self-consistency, verifier rejection, a tool error loop) rather than by quota. The fleet's chain walks on quota/failure; cascades add a success-trigger. The cost of a cascade is the sum of attempts, so the escalation must be gated by a budget (C3PO's point). The 2026 survey classifies cascade routing as a unified framework combining routing and cascading ([survey](https://arxiv.org/html/2603.04445v2)).

### 4.4 Trial methodology

**4.4.1 Bandits (allocation).**
- **Thompson sampling:** per arm keep a Beta posterior over success; sample each posterior and route to the highest sample; update per outcome. Early on traffic explores; later it concentrates on the best arm while still occasionally testing others ([Kalibr, Thompson for LLM routing](https://kalibr.systems/blog/thompson-sampling-llm-routing) [third-party]). Thompson is known to be asymptotically optimal-ish and self-correcting; it requires posterior sampling, which is more compute than UCB ([Wikipedia: Thompson sampling](https://en.wikipedia.org/wiki/Thompson_sampling); [MDPI nonstationary bandits](https://www.mdpi.com/2079-9292/12/13/2814) notes the compute cost).
- **UCB:** pick the arm with the highest mean plus a confidence bonus √(log t / n); deterministic given the data; well-understood regret bounds in stationary settings ([MDPI](https://www.mdpi.com/2079-9292/12/13/2814); [UCB intro, Medium](https://medium.com/@reddyyashu20/what-is-thompson-sampling-and-upper-confidence-bound-d8088a703b02)).
- **Non-stationarity:** model versions change, peak/off-peak windows change, and providers degrade. Thompson with discounting (or a sliding window) is the usual fix; the MDPI paper covers non-stationary bandit variants ([MDPI](https://www.mdpi.com/2079-9292/12/13/2814)).
- **Fleet fit:** Thompson over (door, effort) arms **inside a pool**, with a hard guard that never routes to an arm whose lower confidence bound on success is below the pool's floor. Bandits give allocation, not a decision.

**4.4.2 Decisions (swap or keep).**
- **Peeking is invalid with fixed-horizon p-values.** Johari, Pekelis & Walsh define always-valid p-values and confidence sequences (mSPRT-based) that remain valid under continuous monitoring, at a power cost ([arXiv:1512.04922](https://arxiv.org/abs/1512.04922); published Operations Research 2022 per [Atticus Li summary](https://atticusli.com/replication-crisis/ab-testing-peeking-problem) [third-party]).
- **Kohavi's advice:** prefer group-sequential interim analyses with a few pre-planned looks to continuous always-valid monitoring, because always-valid methods lose power ([Kohavi comment](https://www.linkedin.com/posts/aleksandermolak_can-you-peek-at-p-values-in-your-ab-test-activity-7366386909360590849-9g24) [third-party]). For the fleet: pre-register 2–3 looks.
- **Sample sizes [derived]:** for two success proportions with two-sided α=0.05 and power 0.8, a standard approximation is n per arm ≈ (1.96+0.84)² · [p₁(1−p₁)+p₂(1−p₂)] / (p₁−p₂)² ≈ 7.84 · Σvar / Δ². Worked values:
  - p = 0.90 vs 0.95 (5-point drop): n ≈ 7.84 × 0.1375 / 0.0025 ≈ **431 per arm**.
  - p = 0.90 vs 0.92 (2-point drop): n ≈ 7.84 × 0.1636 / 0.0004 ≈ **3,207 per arm**.
  - With 50 tasks per arm: the smallest detectable difference is ≈ 17 points at p = 0.90 and ≈ 28 points at p = 0.50.
  - This is the core argument against "trial many models at 2+ efforts" on low-volume pools: most single-class pools cannot resolve a 5-point quality difference in a month. The honest instrument for those is a **non-inferiority** test on cost-per-success with a pre-declared margin, or an **equivalence band** (the Azure/Bedrock design, §2.10) judged on a larger shared sample.
- **Interleaving and shadow traffic:** shadow (mirror) traffic runs a candidate on the same requests without serving it; LiteLLM's traffic mirroring is the primitive ([LiteLLM routing](https://docs.litellm.ai/docs/routing)). Interleaving alternates which arm serves within the same user/session, reducing variance from task differences; the fleet's round-robin trials (PR #9018, #9381) are interleaving. Interleaving is only valid if the arms are exchangeable within a session (no cache/context carryover bias); cache-warm arms are not exchangeable, so interleave at task granularity, not turn granularity.
- **Canary:** Portkey markets canary testing [third-party: Portkey summary on Medium]; Cloudflare's Percentage node is the same primitive ([Cloudflare](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing)).

### 4.5 Practical rule set for the fleet's trial design (recommendation, derived from the above)

1. Every trial declares: arms, the success metric, the non-inferiority margin, the minimum n from §4.4.2, the review date, and the revert condition (this is already the Luna pattern in brief §9).
2. Use shadow traffic first when the candidate has a cost or quality risk; use interleaved round-robin only for arms already inside the safe set.
3. Allocation inside a pool: Thompson with a floor guard. Decision to swap a model or to move an effort rung: fixed or group-sequential test with pre-registered looks.
4. Report cost per success and the success CI, not means; use the AIQ-style average over the cost band to rank points (RouterBench).

---

## 5. Overlay and configuration-layering prior art

The brief asks for: set any effort; remove a model-effort; replace a model or a specific model-effort; default to medium; collapse xhigh/max/fast to high. The prior art solves the same problem.

### 5.1 NixOS module system

- **Priorities:** `mkOverride n v`; lower n wins; `mkDefault = mkOverride 1000`; `mkForce = mkOverride 50`; any other value takes an explicit priority ([NixOS manual](https://nixos.org/manual/nixos/stable), [NixOS & Flakes book](https://nixos-and-flakes.thiscute.world/nixos-with-flakes/modularize-the-configuration), [Noogle mkOverride](https://noogle.dev/f/lib/modules/mkOverride)).
- **Conflict rule:** if two definitions of a non-mergeable option have the same priority, evaluation fails. Mergeable types (lists, attrsets) merge.
- **Ordering:** `mkOrder n` for list-valued options; `mkBefore = mkOrder 500`, default 1000, `mkAfter = mkOrder 1500` ([NixOS manual](https://nixos.org/manual/nixos/stable)).
- **Mapping to the fleet:**
  - `set effort E on (model M, pool P)` → an overlay at priority 1000 (the default layer).
  - `replace leg (M,E) with (M',E')` → an overlay at priority 50 (forced), used sparingly.
  - `remove (M,E)` → a tombstone at a stated priority that wins over the default layer.
  - `default to medium` → the base layer at priority 1000; overlays at 1000 conflict loudly, so a second overlay must use an explicit priority.
  - Ordering of legs → `mkBefore/mkAfter` equivalents, or better, numeric `order` (see LiteLLM, §2.2).
- **The copy-worthy property:** equal-priority conflict is an error, not "last one wins". The brief's overlays should fail the guard when two overlays set the same (pool, model, effort) at the same priority.

### 5.2 Kustomize (strategic merge and JSON 6902)

- **Strategic merge patch:** merges by object shape, with merge keys for lists. Supports merge, replace, and delete ([Fabian Lee on Kustomize SMP](https://fabianlee.org/2022/04/18/kubernetes-kustomize-transformations-with-patchesstrategicmerge)). `patchesStrategicMerge` and `patchesJson6902` are deprecated in newer Kustomize versions in favour of `patches`, which is the path to use ([Nick Janetakis](https://nickjanetakis.com/blog/replace-kustomize-commonlabels-patchesstrategicmerge-and-patchesjson6902) [third-party]).
- **JSON 6902:** explicit `op: add|remove|replace` with a JSON pointer path ([Kubernetes Kustomize docs](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization), [KodeKloud patches](https://notes.kodekloud.com/docs/Kustomize/Kustomize-Basics/Patches-Dictionary/page) [third-party]).
- **Mapping:** the fleet's natural key is the (model, effort) pair, not an index. Strategic merge keyed on `model` gives "replace this model's effort-set" without depending on list position. JSON-pointer patches on list indices break when a list is reordered; a **keyed** merge is the right default, and an index-based op is an escape hatch only.
- **Copy:** `delete` as an explicit directive in the patch rather than "absence means keep". Absence-means-keep makes "remove a model-effort" impossible to express without a tombstone.

### 5.3 Helm values layering

**[UNVERIFIED — not fetched this pass.]** Helm merges values files with later files and `--set` winning, and deep-merges maps while replacing lists. This is the well-known source of surprise for lists (you cannot remove a list entry by overriding). The fleet's leg lists have that exact property; a keyed structure avoids it.

### 5.4 CSS cascade

**[UNVERIFIED — not fetched this pass.]** Specificity and source order decide; `!important` is the explicit-override escape. The fleet analogue is specificity = (pool < class < model < model-effort), with source order as the tie-break, and a reviewed `!important` equivalent (the forced layer). Recommend against order-as-tie-break for routing: it makes configs read-order-sensitive.

### 5.5 Recommended overlay semantics (synthesis)

| Operation | Semantics | Prior art |
|---|---|---|
| set E on (pool, model) | overlay at default priority; keyed by (pool, model, effort) | NixOS mkDefault; Kustomize SMP keyed merge |
| remove (model, effort) | explicit tombstone; must name a priority | Kustomize `$patch: delete` |
| replace (model, effort) → (model', effort') | keyed replace at a stated priority | Kustomize SMP replace |
| replace model entirely in a pool | keyed by model, replaces its whole effort set | Kustomize strategic merge by merge key |
| default to medium | base layer, priority 1000 | NixOS default |
| xhigh/max/fast → high | alias table as data, per model, not code | — (fleet-specific) |
| list ordering of legs | numeric order, equal = weighted | LiteLLM `order`; NixOS mkOrder |
| two overlays, same key, same priority | **error** | NixOS |
| unknown key | **error** (no silent drop) | OpenRouter 400 on unsupported effort |

---

## 6. Context-limit handling, prompt caching, and sticky routing

### 6.1 Limits

- **Strictest-limit-wins across a pool** (the brief's §3) is the Azure rule: the effective context window is the smallest model in the subset, so the subset must be tested at its longest prompts ([Microsoft model router](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router), [Sanket Joshi](https://sanketdevstories.in/blog/model-router-in-azure-ai-foundry-optimizing-cost-and-quality) [third-party]).
- **Four separate ceilings:** input, output, cache, total. OpenAI's `max_output_tokens` covers reasoning+visible output together; Anthropic requires `max_tokens` strictly greater than the thinking budget ([OpenRouter reasoning](https://openrouter.ai/docs/use-cases/reasoning-tokens)). The effort rung therefore has a **minimum output cap**; the tier must carry it.
- **Minimum cacheable prefix** (third-party, **[UNVERIFIED]** against Anthropic's table): 4,096 on Opus 4.5/4.6 and Haiku 4.5; 1,024 on Sonnet 4.6 and Opus 4.8; 512 on Opus 5/Fable 5 ([technspire](https://technspire.com/en/blog/anthropic-prompt-caching-pricing-mechanics) [third-party]). Store the minimum per model; a prefix below it is never a hit.
- **Context-window pre-check** as an explicit filter before routing, not a post-failure fallback. LiteLLM does this with `enable_pre_call_checks` ([LiteLLM routing](https://docs.litellm.ai/docs/routing)). Its context-window fallbacks sit ahead of generic fallbacks (LiteLLM precedence rule), so a context-length failure should not consume a quota-fallback step.

### 6.2 Prompt caching economics

- **Anthropic:** cache write 1.25× base input (5-min) or 2× (1-h); read 0.1× ([Claude docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)). Break-even is one read for 5-min, two reads for 1-h ([technspire](https://technspire.com/en/blog/anthropic-prompt-caching-pricing-mechanics) [third-party]). Third-party sources report lower read multipliers on some current models (0.05× on Opus 5.5 and Sonnet 5.5; 0.025× on Fable 5.1) [Respan](https://www.respan.ai/articles/claude-prompt-caching), [TrueFoundry](https://www.truefoundry.com/blog/claude-prompt-caching-cost-math) — **[UNVERIFIED]** against Anthropic's own pricing table; re-check before using. Breakpoints are free ([TrueFoundry](https://www.truefoundry.com/blog/claude-prompt-caching-cost-math)).
- **Anthropic TTL behaviour:** the 5-minute TTL refreshes on each read; backgrounded or long tool runs can outlive it ([Claude docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching); the backgrounded-task point is an anecdotal Reddit report **[UNVERIFIED]**).
- **DeepSeek:** automatic context caching, no write fee; cache-hit input about 2% (Flash) to 3.3% (V4-Pro) of miss; output has a single rate per period; peak/off-peak pricing with off-peak at half price [third-party: [deepseekai.guide](https://deepseekai.guide/api/deepseek-api-pricing), [maximem](https://www.maximem.ai/blog/deepseek-api-pricing-2026), [activepieces](https://www.activepieces.com/blog/deepseek-api-pricing-2026-models-cache-output)]. Peak windows reported as 01:00–04:00 and 06:00–10:00 UTC, Monday–Friday [third-party; the fleet's `model_route_windows` is the source of truth]. **[UNVERIFIED]** against DeepSeek's own pricing page.
- **OpenAI:** automatic prefix caching, no write fee; discount 50% on some models and 90% on GPT-5.4/5.5 family; default 5–10 min inactivity TTL, extended retention on some models ([ofox.ai](https://ofox.ai/blog/prompt-caching-cost-math-anthropic-vs-openai-2026) [third-party]).
- **Economic consequence for the fleet:** a door or effort switch that breaks the prefix costs the full uncached input on the next turn. On Anthropic that is 1.25× (write) on the new key; on DeepSeek it is the miss rate, which is 30–50× the hit rate. The fleet's peak/off-peak windows interact with caching: a door switch at a window boundary can cost more than the price difference saves.

### 6.3 Sticky sessions and cache-preserving routing

- **OpenRouter:** follow-ups stay with the same provider while the prompt cache is warm ([provider routing](https://openrouter.ai/docs/guides/routing/provider-selection)).
- **LiteLLM:** session affinity with an idle TTL (default 3600 s) and an explicit fallthrough-and-keep-pin rule on cooldown ([LiteLLM routing](https://docs.litellm.ai/docs/routing)).
- **Kong:** consistent-hashing on a header gives sticky sessions ([Kong AI proxy advanced](https://developer.konghq.com/plugins/ai-proxy-advanced)).
- **Fleet fit:** sub-sessions with limited context (brief §2) are the cheapest cache-friendly unit. Pin a session to its door for its lifetime; move doors only at session start, or at an explicit `configuration_update`-style boundary.

---

## 7. Design options catalogue — effort tiers and equivalence mapping

Assumption (stated because the program's V2/V3/V4 meanings were not in my brief): **V2 = config-level mechanism the fleet can ship now from its own data; V3 = measured and automatic (frontier, trials, cascades); V4 = learned or adaptive per request.** Re-map if the program defines them differently.

For each option: what it is · pros · cons · scalability/reliability · suggested placement.

**O1. Per-pool, per-tier explicit member lists** (brief: "entirely separate member lists per model").
- Each pool has three ordered lists: low, medium, high, each a list of (model, door, effort).
- Pros: explicit, auditable, zero magic. Cons: 3× the lists to maintain; drift between tiers; no shared equivalence.
- Scale: O(pools × tiers × legs). Reliability: high (no inference). **Placement: V2** as the baseline; the fleet already uses this shape for class routes.

**O2. Per-model effort vocabulary + canonical rung map (data, keyed by model).**
- Each model lists its advertised efforts (from the vendor or `supported_efforts`) and maps each to a canonical rung (`low|medium|high`). Pools reference rungs, not raw efforts.
- Pros: one mapping per model; handles OpenAI `minimal`, Gemini `LOW..HIGH`, Anthropic `xhigh/max` uniformly; the "xhigh/max/fast → high" alias becomes data. Cons: needs a per-model table kept current with vendor enums (which changed in 2026). Scale: O(models). Reliability: high if the table is validated against live `supported_efforts` on each sync. **Placement: V2**, and it is a prerequisite for everything else.

**O3. Equivalence edges with provenance** ("summit low ≈ workhorse high").
- An explicit table of (model, effort) → (model', effort') equivalence with: the reference, the measured quality delta, the cost ratio, the sample size, and a status (`measured` | `extrapolated-from-AA` | `aliased`).
- Pros: directly expresses the brief's cross-pool equivalences; the status field stops extrapolations from being treated as measurements (the brief's §7 concern). Cons: O(n²) if naive; needs maintenance. **Placement: V2** (table + status), filled with measurements from V3.

**O4. Pareto-frontier tiers from measured points.**
- Each (model, effort) point gets cost-per-success and quality with CI; the frontier is computed; tiers are defined as frontier points: low = cheapest frontier point above floor A, high = best frontier point below cost ceiling B, medium = the midpoint in cost.
- Pros: tiers become a computed function of data, which keeps them honest as prices change; dominated points fall out automatically. Cons: needs enough data per point; unstable when the frontier is flat. Scale: O(points log points). Reliability: high if smoothed with a minimum-n rule. **Placement: V3.**

**O5. Quality-band tiers (Azure/Bedrock style).**
- Tier = pick the cheapest point within band δ_tier of the pool's reference (the best point for the class). δ_low wide (5–6%), δ_medium mid, δ_high narrow (1–2%).
- Pros: matches the brief's "equivalent within a band" directly; one tunable per tier; standard in production routers. Cons: depends on a per-prompt or per-class quality predictor, or on class-level averages; band widths need calibration. **Placement: V3** (class-level bands from measured data), V4 for per-prompt.

**O6. Bradley–Terry strengths over (model, effort) points from pairwise judgments.**
- Fit BT (or an extended BT with per-class strengths) on pairwise outcomes; use strength CIs for equivalence (two points equivalent if their CIs overlap within a margin).
- Pros: one comparable strength across pools; handles missing shared tasks; principled uncertainty (Arena). Cons: needs a pairwise judge; agentic tasks may need per-class strengths; judge bias. **Placement: V3/V4**, depends on the stats store (brief §6).

**O7. Overlay layer with explicit priorities and keyed merge.**
- Base tier map (O1/O2) plus overlays `{op: set|remove|replace, key: (pool, model, effort), priority}`; equal-priority conflicts are errors; `remove` is a tombstone.
- Pros: implements the brief's overlay requirement exactly, with NixOS/Kustomize-proven semantics; loud conflicts. Cons: a second source of truth needs a validator; users will reach for `force` too often (keep the priority set small). Scale: fine to hundreds. Reliability: high with the guard. **Placement: V2** (it is the requested feature).

**O8. Index-addressed patch operations (JSON 6902 style).**
- `replace /pools/x/legs/3` style patches.
- Pros: precise. Cons: breaks when lists are reordered; not keyed. **Placement: avoid as default; V2 as escape hatch only.**

**O9. Effort ladder on a token budget instead of an enum.**
- For providers without an effort enum (or where it is coarse), express a rung as a `max_tokens` budget or a fraction of output (OpenRouter's 0.95/0.8/0.5/0.2/0.1 scheme).
- Pros: works on Anthropic thinking budgets and Gemini 2.5 `thinkingBudget`; provider-independent. Cons: Gemini 3 maps budgets to levels internally, so precision is lost; the fraction scheme is OpenRouter's, not the vendor's. **Placement: V3** (per provider where the enum is absent).

**O10. Cascade on quality signal (escalate only on failure).**
- Start a task at low; escalate to medium/high on a verifier failure, self-consistency disagreement, or tool-loop detection; budget-gated.
- Pros: strongest published cost result (FrugalGPT, MoT: ~40% of frontier cost at near-parity) ([arXiv:2310.03094](https://arxiv.org/abs/2310.03094), [arXiv:2305.05176](https://arxiv.org/abs/2305.05176)). Cons: needs a reliable success signal; total cost is the sum of attempts; latency grows. **Placement: V3**, with the escalation edges reusing the existing class second-pass walk.

**O11. Trial bandit inside a pool (Thompson with a floor guard).**
- Arms = (door, effort) inside a pool's safe set; Thompson on success; never route to an arm whose lower CI is below the floor.
- Pros: allocates traffic to the best safe arm automatically; fits round-robin trial patterns the fleet already uses (PR #9018, #9381). Cons: non-stationarity (use discounting); no decision output on its own. **Placement: V3.**

**O12. Mid-conversation effort switching (`configuration_update` style).**
- Keep the request-level effort fixed for the session and change effort by a transcript item at the turn it applies to, so the prefix cache is preserved.
- Pros: avoids cache loss when raising effort for one hard turn; vendor-supported on OpenAI GPT-6 and some Claude models via OpenRouter ([OpenRouter reasoning](https://openrouter.ai/docs/use-cases/reasoning-tokens), [OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)). Cons: the harness must support it; not all models; adjacent updates rejected. **Placement: V3** (only if the harness can emit it; otherwise sub-session per hard step).

**O13. Sub-session effort at spawn, by task class.**
- Each sub-session (the brief's "limited context" children) is launched with an effort rung chosen by its task class, instead of changing effort inside a session.
- Pros: cache-friendly (fresh prefix per child); matches the brief's parallel-subsession guidance; the cheapest mechanism to ship. Cons: the parent must decide the class; children cannot escalate without a relaunch. **Placement: V2.**

**O14. Dominated-model replacement rule (automated detector + human gate).**
- Flag a point as dominated in pool P when non-inferiority holds for a successor at lower cost and latency; propose replacement; a trial with a revert condition confirms it.
- Pros: operationalises the brief's "replace clear successors or dominated models". Cons: false positives on small n; needs §4.4 sample sizes. **Placement: V2 (report only), V3 (automated proposal).**

**O15. Adaptive per-request effort (classifier picks the rung).**
- A classifier (or the model itself, as the brief's self-switching suggests) chooses low/medium/high from the request. Azure Balanced and Bedrock IPR are the production precedents; OpenRouter `openrouter/auto` picks model and effort together ([OpenRouter Auto](https://openrouter.ai/openrouter/auto)).
- Pros: highest potential savings. Cons: needs labelled data; the self-switch variant lets the model choose its own cost, which is a conflict of interest and a fallback-loop risk; hard to audit. **Placement: V4**, and gate self-chosen rungs behind the same overlay guard (O7). Keep the brief's constraint that a model may only switch to category or opencode-route pools, not to arbitrary models.

**O16. Provider-normalised effort with a fail-loud policy.**
- The fleet sends canonical rungs; the adapter translates to the provider's vocabulary; an unsupported value is an error recorded in stats, not silently remapped. (OpenRouter's 400 behaviour; the fleet's current silent `#medium→#high` is the counter-example.)
- Pros: removes the hidden remap that corrupts the stats; keeps trial results attributable. Cons: a rung that is unavailable on one door now fails and must be handled by the chain. **Placement: V2 (mandatory before any effort trial is trusted).**

**O17. Quality-floor scalar selection (Pareto-Router style).**
- The caller states a minimum quality score; the router picks the cheapest point above it.
- Pros: one knob, easy for agents to reason about. Cons: only as good as the score; the score must be the fleet's own. **Placement: V3** (after O4/O6 supply a score).

**O18. Provenance and status on every equivalence (measured / extrapolated / aliased).**
- Cross-cutting: every mapping entry carries its status, the AA version it was derived from, the sample size, and the review date (ties to brief §6 "AA version" tags and claim-model stamping).
- Pros: stops AA extrapolation from being read as measurement; makes reviews possible. Cons: metadata discipline. **Placement: V2**, as a field from the start.

**Summary placement:** V2 = O1, O2, O3 (with O18), O7, O13, O16, O14 (report-only), O8 (escape hatch only). V3 = O4, O5 (class level), O9, O10, O11, O12, O17, O14 (automated). V4 = O6 (if not earlier), O5 per-prompt, O15.

---

## 8. Top five recommendations

1. **Key every effort rung by (model, door, effort-string) with a per-model vocabulary and a canonical low/medium/high mapping as data (O2 + O18).** Gemini, OpenAI and Anthropic enums differ by model and change; a global `#low` is not a comparable quantity. Validate the table against each provider's advertised efforts on sync, and fail loud on an unsupported effort instead of remapping (O16). This also fixes the `#medium→#high` silent remap the brief already documents.
2. **Implement the overlay as keyed, prioritised operations with conflict errors and explicit tombstones (O7).** Copy NixOS `mkOverride` semantics (lower number wins, equal-priority conflicts are errors, `mkDefault` for the base medium layer) and Kustomize keyed merge with `delete`. Do not use list-index patches.
3. **Define equivalence as a quality band against a reference, calibrated per tier, and store the status of every entry (O5 + O3 + O18).** This is the Azure/Bedrock/OpenRouter pattern and it answers the brief's "summit low ≈ workhorse high" directly: an equivalence is "within δ of the reference on the fleet's measured success, at lower cost-per-success", with δ tunable per tier. Mark entries `measured`, `extrapolated-from-AA` or `aliased`, and never tier-map on extrapolated data alone.
4. **Trial with the right instrument for the sample size (§4.4).** Use Thompson sampling with a floor guard for allocation inside a pool (O11) and a pre-registered, group-sequential or non-inferiority test for swap decisions (O14). Write the per-arm n requirement into each trial (≈431 per arm to detect a 5-point drop at p=0.9; ≈3,200 for 2 points). For pools that cannot reach that, judge on cost-per-success with a pre-declared margin rather than on a point difference.
5. **Use sub-session effort at spawn (O13) now, and defer mid-session switching and self-chosen rungs (O12, O15) until the stats store and guards exist.** Sub-sessions are the cache-friendly unit the brief already wants, and they preserve the prefix cache that cross-door or cross-effort switching destroys. Gate any model-proposed rung behind the same overlay guard and the class-only target rule from brief §2.

---

## 9. Open questions (recorded with the default I used)

- **Q1. Are V2/V3/V4 defined as I assumed in §7?** Default: yes, as stated there. Re-map if not.
- **Q2. DeepSeek effort vocabulary.** Not verified. Default: treat DeepSeek as a per-model vocabulary to be read from the live API; the fleet's own note (`#medium` remaps to `#high` on Go and ClinePass doors) is the operative fact.
- **Q3. Current Anthropic cache read multipliers and minimums per model.** Third-party sources disagree (0.1× vs 0.05× vs 0.025×). Default: use the vendor's table at implementation time; do not hard-code.
- **Q4. RouteLLM's specific router models and curve numbers.** Not read in full. Default: cite the abstract's "over 2×" only.
- **Q5. Requesty and Unify were not researched.** Default: omitted; not needed for the design decisions above.
- **Q6. AA per-effort tables vs the third-party GPT-6 Sol table.** The third-party table shows max below xhigh; AA's own page was not fetched for that model. Default: use AA's own published numbers when the fleet ingests AA data; treat the aggregator as a lead.
- **Q7. Whether the harness can emit mid-conversation `configuration_update`.** Default: assume no; O12 deferred.

---

## 10. Source index (primary first)

Primary (fetched or quoted from the vendor/paper page):
- OpenRouter provider routing — https://openrouter.ai/docs/guides/routing/provider-selection
- OpenRouter reasoning tokens — https://openrouter.ai/docs/use-cases/reasoning-tokens
- OpenRouter Auto Router — https://openrouter.ai/openrouter/auto
- OpenRouter Pareto Router — https://openrouter.ai/docs/guides/routing/routers/pareto-router
- LiteLLM routing — https://docs.litellm.ai/docs/routing
- LiteLLM Anthropic effort — https://docs.litellm.ai/docs/providers/anthropic_effort
- Portkey conditional routing — https://docs.portkey.ai/docs/product/ai-gateway/conditional-routing
- Cloudflare dynamic routing — https://developers.cloudflare.com/ai-gateway/features/dynamic-routing
- Cloudflare fallbacks — https://developers.cloudflare.com/ai-gateway/configuration/fallbacks
- Kong AI proxy advanced — https://developer.konghq.com/plugins/ai-proxy-advanced
- Kong AI gateway load balancing — https://developer.konghq.com/ai-gateway/load-balancing
- kgateway failover (Envoy AI Gateway lineage) — https://kgateway.dev/docs/envoy/2.1.x/ai/failover
- Agent Router (formerly Envoy AI Gateway) releases — https://github.com/theagentrouter/agent-router/releases
- AWS Bedrock Intelligent Prompt Routing — https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-routing.html
- AWS Bedrock adaptive thinking — https://docs.aws.amazon.com/bedrock/latest/userguide/claude-messages-adaptive-thinking.html
- Microsoft Learn model router concepts — https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/model-router
- Microsoft Learn model router how-to — https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/model-router
- OpenAI reasoning guide — https://developers.openai.com/api/docs/guides/reasoning
- Google Cloud thinking docs — https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/thinking
- Firebase AI Logic thinking — https://firebase.google.com/docs/ai-logic/thinking
- Mistral reasoning docs — https://docs.mistral.ai/capabilities/reasoning
- Claude prompt caching — https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- Artificial Analysis Intelligence Index v5 — https://artificialanalysis.ai/articles/artificial-analysis-intelligence-index-v5
- Artificial Analysis GPT-5 benchmarks — https://artificialanalysis.ai/articles/gpt-5-benchmarks-and-analysis
- Artificial Analysis methodology — https://artificialanalysis.ai/methodology/intelligence-benchmarking
- NixOS manual, module system — https://nixos.org/manual/nixos/stable
- NixOS & Flakes book, modularize — https://nixos-and-flakes.thiscute.world/nixos-with-flakes/modularize-the-configuration
- Noogle lib.modules.mkOverride — https://noogle.dev/f/lib/modules/mkOverride
- Kubernetes Kustomize — https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization
- arXiv RouteLLM 2406.18665 — https://arxiv.org/abs/2406.18665
- arXiv RouterBench 2403.12031 — https://arxiv.org/abs/2403.12031 (alphaXiv summary: https://www.alphaxiv.org/abs/2403.12031)
- arXiv FrugalGPT 2305.05176 — https://arxiv.org/abs/2305.05176
- arXiv Mixture of Thoughts cascades 2310.03094 — https://arxiv.org/abs/2310.03094
- arXiv C3PO 2511.07396 — https://arxiv.org/html/2511.07396v1
- arXiv survey, dynamic routing and cascading 2603.04445 — https://arxiv.org/html/2603.04445v2
- arXiv test-time compute 2408.03314 — https://arxiv.org/abs/2408.03314
- arXiv Chatbot Arena 2403.04132 — https://arxiv.org/abs/2403.04132
- arXiv Always Valid Inference 1512.04922 — https://arxiv.org/abs/1512.04922
- OptimalThinkingBench 2508.13141 — https://arxiv.org/html/2508.13141v1
- When More Thinking Hurts 2604.10739 — https://arxiv.org/html/2604.10739v1
- Do LLMs Overthink Basic Math (ACL Findings 2026) — https://aclanthology.org/2026.findings-acl.1285.pdf
- Snell et al., ICLR 2025 PDF — https://proceedings.iclr.cc/paper_files/paper/2025/file/1b623663fd9b874366f3ce019fdfdd44-Paper-Conference.pdf
- Chatbot Arena extended BT — https://arena.ai/blog/extended-arena
- Wikipedia: Thompson sampling — https://en.wikipedia.org/wiki/Thompson_sampling
- MDPI nonstationary bandits (LLM-informed) — https://www.mdpi.com/2079-9292/12/13/2814

Third-party (lead, not relied on alone):
- Not Diamond guides — https://www.notdiamond.ai/blog/model-routing-vs-gateways-breaking-down-the-difference , https://www.notdiamond.ai/blog/a-comprehensive-guide-to-model-routing
- IntuitionLabs routing review — https://intuitionlabs.ai/articles/ai-model-routing-cost-quality
- Kalibr Thompson sampling — https://kalibr.systems/blog/thompson-sampling-llm-routing
- AI Agent Store effort table — https://aiagentstore.ai/ai-models/reasoning-effort
- futuresearch effort analysis — https://futuresearch.ai/blog/claude-effort-parameter
- technspire / TrueFoundry / Respan / Requesty caching summaries — https://technspire.com/en/blog/anthropic-prompt-caching-pricing-mechanics , https://www.truefoundry.com/blog/claude-prompt-caching-cost-math , https://www.respan.ai/articles/claude-prompt-caching
- DeepSeek pricing summaries — https://deepseekai.guide/api/deepseek-api-pricing , https://www.maximem.ai/blog/deepseek-api-pricing-2026
- ofox.ai OpenAI vs Anthropic caching — https://ofox.ai/blog/prompt-caching-cost-math-anthropic-vs-openai-2026
- Azure model router practitioner notes — https://sanketdevstories.in/blog/model-router-in-azure-ai-foundry-optimizing-cost-and-quality
- Kohavi comment on SAVI — https://www.linkedin.com/posts/aleksandermolak_can-you-peek-at-p-values-in-your-ab-test-activity-7366386909360590849-9g24
- cloudflare-docs issue on Responses API gap — https://github.com/cloudflare/cloudflare-docs/issues/29840
- RouterBench repo (stale, June 2024 per aggregator) — https://github.com/withmartian/routerbench

**Not fetched (flagged):** Helm values layering, CSS cascade specificity (MDN), Requesty, Unify, DeepSeek official pricing and reasoning parameter, Anthropic per-model cache table.

---

## 11. Appendix: derived arithmetic

- Sample size, two proportions, two-sided α=0.05, power 0.8: n per arm ≈ (z₀.₉₇₅ + z₀.₈₀)² · [p₁(1−p₁) + p₂(1−p₂)] / (p₁−p₂)², with (1.96+0.84)² = 7.84. Unpooled approximation; a pooled-variance version differs slightly. Values in §4.4.2 follow from this.
- Minimum detectable difference with n per arm: Δ ≈ √(7.84 · Σvar / n). At n=50, p=0.90: Σvar=0.18 → Δ≈0.168; p=0.50: Σvar=0.50 → Δ≈0.280.
- Cache break-even (Anthropic, from published multipliers; cached path = one write plus k reads, uncached path = k+1 full sends, prefix normalised to 1.0): 5-min write gives 1.25 + 0.1k < 1 + k once k > 0.28, so one read already pays off (k=1: 1.35 vs 2.0). 1-h write gives 2.0 + 0.1k < 1 + k once k > 1.11, so two reads are needed (k=1: 2.1 vs 2.0 uncached; k=2: 2.2 vs 3.0). Source for the multipliers and the "one read / two reads" statement: [technspire](https://technspire.com/en/blog/anthropic-prompt-caching-pricing-mechanics) [third-party, consistent with Anthropic's docs].
