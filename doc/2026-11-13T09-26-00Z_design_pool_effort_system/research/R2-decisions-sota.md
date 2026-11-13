# R2 — Decision and classification APIs: state of the art

Program: design-mill, starter issue code#9399. Supplement read: `../brief-decisions-supplement.md`.
Author: research subagent (route opencode-route/workhorse), 2026-11-13. Detached run; nothing committed.

Conventions:
- **[V]** = read from a primary page fetched in this pass. **[S]** = secondary or third-party source (blog, aggregator, HN). **[M]** = recalled from memory, not fetched in this pass; treat as unverified until checked. Every claim carries a URL or a tag.
- Prices are USD list prices as shown on the vendor pages fetched on 2026-11-13. "Per 1k decisions" uses the reference decision: **2,000 input tokens, 100 output tokens**.
- "V2/V3/V4" placement uses the brief's language. The brief does not define these versions. This report assumes V2 = next slice, V3 = later, V4 = deferred. The mill should settle the mapping.

---

## 0. What the fork already encodes (grounding)

Read from `~/git/opencode` at `origin/HEAD` (read-only):

- `packages/core/src/systemone.ts`: the **door** for `POST /v1/systemone`. It forwards only `{state, questions}` to Cloudflare Workers AI Clef (`…/workers-ai/run/<model>`). It returns the upstream `result.answers` verbatim, with no usage/id fields. The error envelope is plain `{error: string}`. Model grammar is confined to `@cf/cloudflare/clef*`. Single deadline 30 s, env override capped at 300 s. Accounting records tokens as 0/0 because the gateway reports no usage.
- `packages/ai/src/experimental/system-one.ts`: the **client route**. It sends `{model, state, questions}` (with `boolean` questions rewritten to `noul`). It decodes `answers` into `boolean | choice | score` answers with `probabilities`, optional `confidence`, optional `legend`. Probabilities are constrained to [0,1]. Choice questions cap at 255 options; Score questions cap at 10 levels. The Request schema is `StructWithRest(...)`, which allows extra body keys, but the door drops them. That matters for §4.
- `packages/ai/src/experimental/evaluation.ts`: the **question types** (choice, score, boolean) and answer shapes, with `EvaluationRounding` to 2 decimals.

Implication: the fork already has a superset-tolerant client and a strict door. Any clause layer must **compile away before the call**, because the door discards unknown keys. This is consistent with the brief's "above the endpoint" rule (§7).

---

## 1. Native classification / decision / reranker APIs, and prices

### 1.1 Jev, TypeSafe, and the Command Code / Clef family

- **Jev 1.14** (`jev-1.14.0`, alias `jev-latest`): **$0.042 per Mtok input, output free**. Context 64k per request, 32k per question. Input: text and images. Question types: Choice, Score, Noul, Box (Box needs 1.14). Jev reads the state once and evaluates every question in parallel. [V] https://docs.typesafe.ai/models
- Docs: "Ask independent questions together, combine the outputs in code." Dependent questions go in later calls, with earlier answers placed in `state`. [V] https://docs.typesafe.ai/models (Customizing Jev) and https://docs.typesafe.ai/concepts/system-one
- Calibration claim: "calibration is measured across groups of predictions; it does not guarantee that an individual answer is correct." [V] https://docs.typesafe.ai/concepts/system-one
- Command Code exposes the same model as `typesafe/jev` at `api.commandcode.ai/provider/v1/systemone`, with `usage.input_tokens`/`output_tokens` in the response. No price on that page. [V] https://commandcode.ai/models/jev
- Jev's own explainer shows the raw token-level probability of one answer as 0.36 before normalization, and 0.98 after merging the split variants "Edit" and "edit". So probability mass depends on tokenization and normalization. [V] https://victordibia.com/explainers/jev/
- Third-party claims, unverified: P50 latency 0.21 s on OpenRouter telemetry, and a reported price of $0.042/Mtok. [S] https://www.firecrawl.dev/blog/openai-decisions-api-vs-jev . The OpenRouter Jev page returned 404 when fetched, so it is not verified directly.
- Firecrawl's comparison says Jev is text-only. The TypeSafe models page now says Jev 1.14 reads images. Treat the image claim as **version-dependent**; check the version we pin. [S] vs [V].

### 1.2 Cloudflare Workers AI: Clef family and the included allowance

- **Clef-flash** (`@cf/cloudflare/clef-flash`, 9B, context 24,576 tokens, vision, zero data retention): **$0.038 per Mtok input, output free** = 3,455 neurons per Mtok input. [V] https://developers.cloudflare.com/workers-ai/models/clef-flash/
- **Clef** (`@cf/cloudflare/clef`, 27B, context 65,536): **$0.24 per Mtok input**, output free = 21,818 neurons per Mtok. [V] https://developers.cloudflare.com/workers-ai/models/clef/
- Also listed on the pricing page: `@cf/cloudflare/clef-omni` at $0.150/Mtok, `@cf/cloudflare/clairic` at $0.100/Mtok, `@cf/cloudflare/clef` at $0.240/Mtok, and `@cf/huggingface/distilbert-sst-2-int8` at $0.026/Mtok (a sentiment classifier, not a general decision model). [V] https://developers.cloudflare.com/workers-ai/platform/pricing/
- Neuron price: **$0.011 per 1,000 neurons**. Free allocation: **10,000 neurons per day**, reset at 00:00 UTC. On the Workers Free plan, requests fail after the allocation is used up. On the Workers Paid plan, usage above the allocation is billed. Workers AI is included in both plans. [V] https://developers.cloudflare.com/workers-ai/platform/pricing/
- **Correction to the brief's framing.** The brief says "$10 (or whatever) of Cloudflare usage included with Workers". The verifiable figure is **10,000 neurons/day, about $0.11/day at the overage rate, about $3.30 per 30 days**. The Workers Paid plan is a **$5/month minimum** for Workers, KV, D1, Durable Objects and so on, and it does not add a Workers AI credit. [V] https://developers.cloudflare.com/workers/platform/pricing/ and the Workers AI pricing page above.
- **Consequence for "disengage once the included amount is hit":** on the Free plan, exhaustion fails the request, which is a natural failover trigger. On the Paid plan there is **no hard stop** at 10k/day; overage is billed. The executor needs its own budget guard (a neuron counter per UTC day) if it is meant to disengage on Paid as well.
- Free-allowance capacity, derived from the figures above, per UTC day:
  - Clef-flash at 2,000 input tokens: 6.91 neurons per decision, so **about 1,447 decisions/day**.
  - Clef 27B at 2,000 input tokens: 43.6 neurons per decision, so **about 229 decisions/day**.
  - Llama-3.2-1B-instruct (`@cf/meta/llama-3.2-1b-instruct`, $0.027/$0.201 per Mtok, neurons 2,457/18,252 per Mtok): about 6.7 neurons per 2k-in/100-out decision, so **about 1,480 decisions/day**. Quality of a 1B model for multi-question decisions is unmeasured. [V] for prices.
- How to measure consumption: the Workers AI dashboard shows neurons (the pricing page links it). Whether API responses expose neuron counts is **unverified**. The Clef response contains a `usage` object, but the door's accounting records 0/0 tokens, so do not rely on it without a test call. [V] https://developers.cloudflare.com/workers-ai/platform/pricing/ ; door behaviour in `packages/core/src/systemone.ts`.

### 1.3 OpenAI

- **Decisions API** (announced at DevDay 2026, limited preview): no public request/response schema, endpoint, SDK method or price. OpenAI's own slide claims 150 ms vs 1.6 s for GPT-6 Luna. Image input is confirmed. [S] https://www.firecrawl.dev/blog/openai-decisions-api-vs-jev citing https://openai.com/index/devday-2026-recap/ . Not fetched directly in this pass.
- **GPT-6 Luna** (`gpt-6-luna`): **$0.10/Mtok input, $0.50 output, cached input $0.01**; batch $0.05/$0.25. [V] https://developers.openai.com/api/docs/pricing
- Other rows fetched: gpt-5-nano $0.05/$0.40 (batch $0.025/$0.20); gpt-5.4-nano $0.20/$1.25; gpt-5.6-luna $0.20/$1.20; text-embedding-3-small $0.02/Mtok; `omni-moderation-latest` **free**. [V] https://developers.openai.com/api/docs/pricing
- **Structured Outputs** (`text.format` with `json_schema`, `strict: true`) "constrains completed, non-refusal model output". The docs say you must still handle refusals, incomplete responses and API errors, and validate business rules the schema does not express. [V] https://developers.openai.com/api/docs/guides/structured-outputs
- **Moderation** returns fixed categories (harassment, hate, illicit, self-harm, sexual, violence, with `/` sub-categories) with scores, and **no prompt-injection category**. Scores can recalibrate across model updates, per the docs. [V] https://platform.openai.com/docs/guides/moderation
- Naming caveat: the brief says "Luna small / Luna 100k". The OpenAI price list has "gpt-6-luna" with no context tier. Anthropic's Haiku 5.5 has the 100k prompt-length price tier. The brief's "Luna 100k" may refer to something else. **Unverified mapping; ask Drew.**

### 1.4 Anthropic

- **Claude Haiku 5.5**: **$0.10/Mtok input, $0.50 output** for prompts up to 100,000 tokens; $0.50/$2.50 above 100k. Cache hit $0.01/Mtok. Batch $0.05/$0.25 (up to 100k). [V] https://platform.claude.com/docs/en/about-claude/pricing
- Haiku 4.5: $1/$5. [V] same page.
- Tool-forced JSON: the pricing page lists `tool_choice` `auto/none` vs `any/tool` modes, with per-model tool-use system-prompt token counts (Haiku 5.5: 286 / 406 tokens). So forcing a tool call is the structured-output path. [V] https://platform.claude.com/docs/en/about-claude/pricing (Tool use pricing)
- No native probabilities. Confidence must be requested in the schema (see §2).

### 1.5 Google

- **Gemini 3.5 Flash-Lite**: $0.30/Mtok input, $2.50 output; batch $0.15/$1.25; the **Free Tier shows input and output "Free of charge"** (free tier data may be used to improve products). [V] https://ai.google.dev/gemini-api/docs/pricing
- **Gemini 3.8 Flash**: free tier shown; paid $0.75/$3.75 **through Dec 31, 2026**, then $1.50/$7.50. [V] same page.
- **Gemini 4 Hydrogen**: free tier shown; paid $1.50/$7.50. [V] same page.
- Free-tier rate limits are not on that page. **Unverified.**
- Gemini has controlled generation (JSON schema) in the API. Not fetched in this pass. **[M]**

### 1.6 DeepSeek (Flash class), with off-peak

- **deepseek-flash** (V4.1-Flash-1017): cache-miss input **off-peak $0.15 / peak $0.30** per Mtok; output **off-peak $0.60 / peak $1.20**; cache-hit input off-peak $0.003 / peak $0.006. Context 1M. JSON output, tool calls and Anthropic-format endpoint are supported. [V] https://api-docs.deepseek.com/quick_start/pricing
- Off-peak = half of peak. **Peak hours: 01:00–04:00 and 06:00–10:00 UTC, Monday to Friday, excluding Chinese public holidays.** All other hours are off-peak. [V] same page.
- deepseek-v4-pro: off-peak $0.66/$1.98; peak $1.32/$3.96. [V] same page.

### 1.7 Cohere

- Rerank 5 (`rerank-v5.0-pro`, `rerank-v5.0-fast`): a reranker over query/document pairs, returns relevance scores. Not a multi-question decision model. [V] https://docs.cohere.com/docs/rerank-overview
- **Price not extracted** (the pricing page fetched returned navigation only). **Unverified.** Classify (Cohere's classification endpoint) was not checked in this pass. **[M]** only.

### 1.8 Fast inference hosts (Groq, Cerebras)

- Groq and Cerebras prices are only available from **third-party** aggregators in this pass:
  - gpt-oss-120b: Groq $0.15/$0.60; Cerebras ~$0.35/$0.75. [S] https://www.spheron.network/blog/groq-lpu-vs-cerebras-wse-cheapest-inference-2026/
  - Llama 3.1 8B on Cerebras: $0.10/$0.10 per Mtok. [S] https://tokenando.ai/models/cerebras-llama-3-1-8b-cerebras
  - Groq's own pricing page did not show per-model rates (it returned marketing text). Llama 3.1 8B on Groq appeared as "contact sales" on a third-party page. [S] https://www.morphllm.com/groq-pricing
  - Free tiers: Groq and Cerebras both listed free tiers in a 2026-05-31 snapshot, with Groq's token-per-minute ceiling as the practical limit. [S] https://ianlpaterson.com/blog/free-llm-api-2026/ . Cerebras' catalogue was reported as volatile. **Unverified now.**
- The brief asks for "a super-fast free one that doesn't need to be smart at all". These hosts are the first candidates. Their JSON-mode validity must be measured (§3).

### 1.9 Local options

- Not priced; zero marginal API cost. Candidates **[M]**: small ONNX classifiers, embeddings + logistic regression, SetFit few-shot heads, llama.cpp grammars for a small instruct model. Hugging Face zero-shot pipelines **[M]**. None was fetched in this pass.
- Cloudflare's embedding prices, for reference: bge-small $0.020/Mtok, bge-base $0.067/Mtok, bge-m3 $0.012/Mtok, embeddinggemma-2 $0.0017/Mtok, and a reranker bge-reranker-base $0.003/Mtok. [V] https://developers.cloudflare.com/workers-ai/platform/pricing/

### 1.10 Price and latency table per 1,000 decisions

Reference decision: 2,000 input tokens, 100 output tokens. Costs computed from the list prices above. "Native" rows return probabilities without generating text. "LLM" rows generate JSON and need a schema check.

| Option | Class | Input $/Mtok | Output $/Mtok | $ per 1k decisions | Latency evidence | Free allowance / notes |
|---|---|---|---|---|---|---|
| Clef-flash (CF Workers AI) | native | 0.038 | free | **0.076** | not measured | 10k neurons/day ≈ 1,447 decisions/day [V] |
| Jev 1.14 (TypeSafe) | native | 0.042 | free | **0.084** | P50 0.21 s on OpenRouter [S] | 64k/32k context [V] |
| Clef-omni (CF) | native | 0.150 | free (assumed as Clef family) | 0.30 | not measured | model page not fetched [V price only] |
| Clef 27B (CF) | native | 0.240 | free | 0.48 | not measured | 65k context; ≈229/day free [V] |
| OpenAI Decisions API | native (claimed) | unpublished | unpublished | n/a | vendor claim 150 ms [S] | limited preview; do not build on it yet |
| gpt-5-nano | LLM | 0.05 | 0.40 | 0.14 | not measured | strict Structured Outputs [V] |
| Llama-3.2-1B (CF) | LLM | 0.027 | 0.201 | 0.074 | not measured | ≈1,480/day free; quality unmeasured [V] |
| Llama 3.1 8B (LlamaGate, aggregator) | LLM | 0.03 | 0.05 | 0.065 | not measured | [S] only; unverified |
| Llama 3.1 8B (Cerebras, aggregator) | LLM | 0.10 | 0.10 | 0.21 | very fast (vendor) | [S] only; free tier reported |
| gpt-6-luna | LLM | 0.10 | 0.50 | 0.25 (batch 0.125) | not measured | cached input $0.01 [V] |
| Claude Haiku 5.5 | LLM | 0.10 | 0.50 | 0.25 (batch 0.125) | not measured | ≤100k prompt tier [V] |
| gpt-5.4-nano | LLM | 0.20 | 1.25 | 0.525 | not measured | [V] |
| Gemini 3.5 Flash-Lite | LLM | 0.30 | 2.50 | 0.85 (batch 0.425) | not measured | free tier shown [V] |
| DeepSeek flash, off-peak, cache miss | LLM | 0.15 | 0.60 | **0.36** | not measured | cache hit 0.066 per 1k [V] |
| DeepSeek flash, peak, cache miss | LLM | 0.30 | 1.20 | 0.72 | not measured | peak UTC windows above [V] |
| Gemini 3.8 Flash (promo) | LLM | 0.75 | 3.75 | 1.875 | not measured | from 2027-01-01: 3.75 [V] |
| Gemini 4 Hydrogen | LLM | 1.50 | 7.50 | 3.75 | not measured | free tier shown [V] |
| Haiku 4.5 | LLM | 1.00 | 5.00 | 2.50 | not measured | [V] |
| Cohere Rerank 5 | reranker | unverified | n/a | n/a | n/a | [V] doc only; not a decision model |
| OpenAI moderation | fixed classifier | free | n/a | 0 | n/a | fixed safety categories only [V] |

Cost per correct decision = $/1k ÷ accuracy. This is the number that matters, and it is not in the table because we have no labelled accuracy yet (see §8).

Latency: the only evidence in this pass is the Jev P50 figure (third-party telemetry) and OpenAI's own slide (vendor claim). **No latency was measured by us.** The measurement plan is in §8.

### 1.11 What the table says (provisional)

- Per decision, the two Cloudflare Clef models and Jev are the **cheapest paid options by 2–4×**, and their output tokens are free. A decision that generates 100 output tokens costs as much as the input on an LLM.
- The cheapest general LLM with a usable schema is **gpt-5-nano at $0.14 per 1k**. DeepSeek flash off-peak with cache misses is $0.36 per 1k. Luna and Haiku 5.5 are $0.25 per 1k.
- **Multi-question packing changes the picture.** Jev and Clef price the state once and evaluate all questions in one call. If a decision set has 10 questions over the same 2k state, native cost per question drops ten-fold. LLM cost also falls if questions are batched in one JSON response, but output tokens grow with the number of questions.

---

## 2. Probability outputs

### 2.1 Mechanisms

| Mechanism | How | Pros | Cons | Cost |
|---|---|---|---|---|
| Native probabilities (Jev, Clef) | Model outputs a distribution over fixed options, trained for calibration | Probabilities are typed, no text, free output | Calibrated on groups, not each answer [V]; order sensitivity reported [S]; vendor lock | $0.04–0.24/Mtok input |
| Logprob-derived (LLM API) | Read `logprobs` / top-k over label tokens at the decision position | Real model distribution, no extra call | Tokenization splits labels (Jev explainer shows 0.36 → 0.98 after merging variants [V]); needs provider support; not on every endpoint **[M]** | Usually free with the call **[M]** |
| Verbalized (LLM states a number in JSON) | Ask for `confidence` or per-option probabilities in the schema | Works on any model | Overconfident, inconsistent (Tian et al. below) | Free |
| Sampled consistency | Run N samples, use vote share | Simple, model-agnostic | N× cost; agreement ≠ accuracy **[M]** | N× |
| P(True) style | Ask "is this answer correct?" and read probability | Calibrates a candidate answer | Second call; needs candidate | 2× |

### 2.2 Verbalized confidence: evidence

- Tian et al., "Just Ask for Calibration" (EMNLP 2023): verbalized top-1 confidence on RLHF-tuned models gave ECE 0.071 (TriviaQA), 0.196 (SciQ), 0.386 (TruthfulQA). Verbalized top-k lists and linguistic confidence did better on some sets and worse on others. [V] https://aclanthology.org/2023.emnlp-main.330.pdf ; arXiv https://arxiv.org/abs/2305.14975
- Practical lesson: verbalized numbers are **usable only after recalibration on our own labels**. Calibration error varies widely by task. Do not assume transfer.

### 2.3 Calibration methods

- **Temperature scaling** (Guo et al., 2017): one scalar on logits fitted on a validation set. Cheap, preserves ranking. **[M]** https://arxiv.org/abs/1706.04599
- **Platt scaling** (sigmoid on the score): two parameters, good for binary decisions. **[M]** Platt 1999.
- **Isotonic regression**: non-parametric, needs more labels (roughly thousands). **[M]**
- **Histogram / Bayesian binning**: simple reliability bins. **[M]**
- Measurement: ECE (10 bins), Brier score, reliability diagram, and selective risk (accuracy at each coverage). All require labels (§8).
- Recommendation for us: fit a per-decision-type, per-model calibrator (Platt or temperature) on labelled decisions, and version it. Store the raw and calibrated values together.

### 2.4 Multi-question dependent probabilities

- The brief needs: 3+ outcome choices conditioned on another decision; a combination Q1+Q3 that fixes Q4; Q2+Q4 that shift Q5.
- **Chain rule**: P(Q1..Qn) = ∏ P(Qi | Q<i). Any joint can be written this way. **[M]** (standard probability).
- **Bayesian networks / CPTs**: a DAG of questions with conditional probability tables. Exact for small sets (3 binary parents = 8 rows). **[M]**
- Practical route with the endpoint we have: TypeSafe's own guidance is to ask independent questions in one call and combine outputs in code, and to feed dependent answers back into `state`. [V] https://docs.typesafe.ai/models
- So for the brief's "if Q1,Q2,Q3 = yes/no/yes then Q4 = no": **ask Q1–Q3 in call 1, then condition Q4 in call 2 by placing the answers in `state`, or apply a CPT in the clause layer (§4).** Either route keeps the endpoint standard.
- Warning: conditioning by writing answers into `state` treats model output as evidence. Use the **probabilities** as soft evidence, not the argmax, or the error compounds.

### 2.5 Recommendation (probabilities)

1. Store full probability vectors per question, not only the chosen option. This enables counterfactual evaluation later (§5).
2. Calibrate per decision type and per model before any threshold is used.
3. For order sensitivity, run a permutation check on choice options (§8). Jev's order-sensitivity is a third-party claim [S], so test it.
4. Use "effort" only as a **small tie-break** over the calibrated probabilities (brief §6), not as a probability transform. This keeps the calibration stable.

---

## 3. Structured output reliability

### 3.1 Constrained decoding

- **OpenAI strict mode**: schema adherence for completed, non-refusal output. Refusals, incomplete responses and API errors still need handling. [V] https://developers.openai.com/api/docs/guides/structured-outputs
- **JSONSchemaBench** (EPFL, arXiv 2501.10868): 10k real-world JSON schemas, evaluates Guidance, Outlines, llama.cpp, XGrammar, OpenAI and Gemini. It measures compliance and coverage of the JSON Schema test suite. **Failure-rate numbers were not pulled in this pass.** [V] existence and scope: https://arxiv.org/html/2501.10868v1 . Fetch the results table before quoting numbers.
- **Engines**: XGrammar (default in SGLang; vLLM supports it) and llguidance. Overhead claims (~40–50 µs/token) are from secondary blogs **[S]**, so treat them as unverified: https://zylos.ai/research/2026-04-11-structured-output-constrained-decoding-production-agents-2026/ . SGLang doc confirms XGrammar default and Outlines as options: https://docs.sglang.ai/advanced_features/structured_outputs.html [V].
- A constrained decoder guarantees **shape, not semantic correctness**. A schema-valid answer can still be wrong (secondary source; principle is standard) [S].
- Constrained decoding can hurt reasoning when the grammar forces low-probability tokens. A cited mitigation alternates constrained and unconstrained windows (CRANE, ICML 2025, cited secondhand). **Unverified in this pass.** [S]

### 3.2 Validation loop (Instructor and similar)

- Instructor validates output against a Pydantic model and, on failure, **re-asks with the validation error**. Retry count is a parameter (`max_retries`). [V] https://python.useinstructor.com/ ; reask concept: https://python.useinstructor.com/concepts/reask_validation/
- The reask message includes the validation error text. The loop is: parse → validate → on error, send the model's previous output and the error → retry.
- BAML, Pydantic AI, Outlines and Guidance are alternatives. **Not verified in this pass.** **[M]**

### 3.3 Best practice for feeding back invalid output (synthesis)

1. Classify the failure first: **refusal, truncation/incomplete, provider error, schema-invalid, business-invalid** (e.g. probability sums ≠ 1, an option not in the list, a probability outside [0,1]). Each has a different remedy.
2. For schema-invalid: resend the **invalid output, the compact schema, the exact validator messages with JSON paths, and one hint**. Do not resend the whole original prompt. Practitioner consensus **[S]**; not measured by us.
3. Cap at two tries (the brief's rule). Second-try success rate per model is a metric (§8).
4. For probability vectors: sum ≠ 1 is repairable locally (renormalize) and should **not** cost a retry. Log the repair.
5. Decision after two failures: next model in the pool, then next pool, then default answer (brief §4).

### 3.4 When retry beats re-route

- Retry is cheaper when the failure is local (one field, one enum) and the model is strong on the task. Re-route is better when the model's **schema-compliance rate** is low or the task is out of its range.
- Rule to test: if second-try success on a given model/schema is below ~50%, re-route after one try. This threshold is a design guess; **measure it**.
- Failure records should carry the model, the schema version and the failure class, so this can be learned per model (§5).

---

## 4. Conditional and rules layers over a decision API

### 4.1 Candidate languages

| Tool | What it is | Termination | Fit for us | Notes / source |
|---|---|---|---|---|
| CEL (Common Expression Language) | Non-Turing-complete expression language, embeddable, typed, "fast, safe" | Guaranteed (non-Turing complete) | **Good**: conditions over answers, typed; cel-go and cel-python exist | [V] https://cel.dev/ ; spec https://github.com/cel-expr/cel-spec ; Google Cloud uses it for policy and Eventarc filters [S] |
| JsonLogic | Rules as JSON objects (`{"if":[…]}`, `{"and":[…]}`, `missing`) | Terminates (no loops) | Good for JSON-only portability | [M] https://jsonlogic.com/ |
| OPA / Rego | Policy language, Datalog-inspired | Bounded by design | Heavier; good for access policy, not soft preferences | [M] https://www.openpolicyagent.org/docs |
| DMN decision tables (FEEL) | OMG standard; decision tables with hit policies (UNIQUE, FIRST, COLLECT) | Table-driven | Strong for "if Q1,Q2,Q3 = yes/no/yes then Q4 = no" style tables; hit policies map to our "boost" semantics | [M] https://www.omg.org/spec/DMN |
| Jinja (sandboxed) / Handlebars / Mustache | Templating: text output | Jinja can loop; sandbox needed | Use for **rendering instructions text**, not for logic. Output is a string. | [M] Jinja SandboxedEnvironment |
| Nix module fixpoint | Lazy `lib.fix` over attribute sets | Only if laziness avoids the cycle | Idea only (§6) | [M] https://github.com/NixOS/nixpkgs/blob/master/lib/fixed-points.nix |

### 4.2 How each handles the brief's clauses

Brief clauses (§7): "if 1,2,3 = yes/no/yes → Q4 = no"; "if yes/no/no → Q4 yes, Q5 no"; "if blank then X"; "if all yes → slight/strong preference"; "if deferred → check option 1 only"; "if >100 → check all".

- **CEL**: `q1 == "yes" && q2 == "no" && q3 == "yes" ? {q4: "no"} : {}` as one expression returning an effect map. Missing answers: use `has()`/optional handling. Clear and typed. Good for effects.
- **JsonLogic**: same with `{"if":[{"and":[…]}, {...}, {}]}`. Deferred test via `{"missing":"q1"}`. Verbose but serializable.
- **DMN table**: one row per combination; hit policy COLLECT for additive preferences, FIRST for overrides. Easiest for humans to review. Combinatorial explosion for 3+ binary inputs is still small (8 rows).
- **Template string** (brief's shape): fine for producing text such as "Given Q1=yes…" in the instructions, or for building the next question's `state`. Poor for logic: string output cannot be type-checked and is easy to mis-quote.

**Recommendation:** conditions and effects in **CEL** (or JsonLogic if we want a JSON-only portable form). Use templates only to **render** human-readable text. The brief's "template string with access to variables" should be a *renderer* over a variables map, with logic expressed as CEL.

### 4.3 Soft vs strong preference

Model preferences as **additive evidence in log-odds space** on each option's probability:

- Soft boost: δ_soft (small, e.g. +0.2 nats).
- Strong boost: δ_strong (e.g. +1.5 nats) or a hard constraint.
- Combine with the calibrated probability by adding δ to the logit and renormalizing over options.
- Cap total adjustment per option (e.g. ±3 nats) to avoid runaway rule stacking.
- The effort tie-break in brief §6 is the same mechanism with δ ≈ 0.02–0.05 nats. Keep it inside the cap.

This is the standard way to combine independent evidence (naive Bayes / logistic combination). **[M]** The numbers above are design guesses; tune them on labels.

### 4.4 Defaults for deferred or blank answers

- A missing answer is not `no`. Represent it as an explicit state: `unanswered`, `deferred`, `blank`.
- Each question carries a default effect: "if blank then X" fills X **only** for the downstream rule. The question's own probability stays missing.
- "If deferred → check option 1 only" is a **default selection set**. Model as a rule that narrows the candidate set, not as an answer.

### 4.5 Superset-compatible protocol extension

- JSON Schema: unknown keywords are ignored by default in the spec; keywords starting with **`x-` are annotations** by the json-schema.org decision. [V] https://json-schema.org/blog/posts/custom-annotations-will-continue . This is the standard way to add vendor fields without bending the spec.
- OpenAPI: `x-` specification extensions are the same convention **[M]**.
- Fork fact: the client's Request schema already accepts extra keys (`StructWithRest`), and the door drops them (`packages/core/src/systemone.ts`).
- **Design rule:** clauses live under an `x-` key on our request envelope (e.g. `x-clauses`), are **compiled into the base `{state, questions}` before the call** by the executor, and the endpoint never sees them. The endpoint contract is unchanged, which satisfies the brief's "superset that does not affect the real standard".
- Compiled form: clauses become (a) rewritten `state` (answers from earlier calls), (b) rewritten `instructions` (rendered text), or (c) post-hoc log-odds adjustments applied after `answers` return. None of these changes the standard request.

### 4.6 Ordering and cycles inside clauses

- Clauses can depend on other clauses. Order them with a topological sort over the question dependency graph, and reject cycles at compile time.
- This is the same problem as §6 (pools referencing pools); reuse one implementation.

---

## 5. Decision tracking and feedback loops

### 5.1 Decision logs and propensities

- **Contextual bandit logging**: log context, the chosen action, and the **propensity** (probability of choosing it). Propensities allow inverse-propensity scoring (IPS) for counterfactual evaluation of other policies. [V] VW tutorial https://vowpalwabbit.org/tutorials/off_policy_evaluation.html ; IPS in the contextual bandit bake-off https://arxiv.org/abs/1802.04064
- **Microsoft Personalizer** and Azure's DSJSON logs: same idea, productised. [S] https://www.microsoft.com/en-us/research/publication/optimal-adaptive-off-policy-evaluation-contextual-bandits/ . Personalizer docs not fetched.
- **Implication for us:** our pool choice is currently deterministic (argmax with default). Propensities are then ~0 or 1, so IPS is useless. To learn from counterfactuals we need **deliberate exploration** on near-tie pools, or we need to **store full probability vectors and re-run the candidate decision models offline** on logged inputs. The second is cheaper and should be the default.

### 5.2 Feature-flag evaluation logs

- Feature-flag systems log evaluation with variant, rule and context, for audit and experiment analysis. **[M]** (LaunchDarkly, Statsig). Not fetched.
- Lesson transferred: every decision logs `decision_id`, the rule/clause version and the model version that produced it.

### 5.3 LLM observability and score schemas

- **Langfuse**: scores with four data types: **Numeric, Categorical, Boolean (as 0/1 float), Text**. Score configs can be enforced (min, max, categories). Scores attach to traces or observations. [V] https://langfuse.com/docs/evaluation/evaluation-methods/scores-via-sdk ; data model https://langfuse.com/docs/evaluation/scores/data-model
- Fit for the brief's **caller/callee 0–5 gauges**: four **Numeric** scores with score-config range 0–5 (`caller_correct`, `caller_wrong`, `callee_correct`, `callee_wrong`). Langfuse can hold them per decision ID.
- Fit for **multi-label outcomes** (PR merged, reverted, rework, too-big, perfect, …): Langfuse categorical is one value per score. Use one **Boolean** score per label, or keep the label set in our own table. Our own table is the source of truth; export to Langfuse for dashboards only.
- LangSmith feedback, Braintrust, Helicone and Arize Phoenix: not fetched in this pass. **[M]** each supports numeric/categorical feedback on runs.

### 5.4 Delayed-outcome attribution

- Outcomes arrive hours to weeks later (PR merged, revert, incident). Attribution must use a **stable join key**, not timestamps.
- Recommend: a `Decision-Id:` git trailer in commit messages and PR bodies, plus a `task_id` on every session. A revert commit that names the reverted commit's trailer links back. **[M]** on the git convention for revert messages (`This reverts commit <sha>`).
- DORA metrics (change failure rate, time to restore) give the standard definitions for rework/failure outcomes **[M]** https://dora.dev/ . Use DORA definitions where the brief's labels overlap.
- Bulk labelling UIs: Label Studio and Argilla (review queues, bulk select, label sets, inter-annotator checks). **[M]** Not fetched.

### 5.5 Record schema (proposal)

Per decision:
- `decision_id`, `created_at`, `task_id`, `caller_session`, `callee_session`, `decision_type`, `schema_version`, `prompt_version` + date (brief §9).
- `pool_id`, `route_chain` (models and pools tried, with failure class per step), `final_model`, `final_pool`, `fallback_used` (bool), `default_used` (bool).
- `state_hash`, `state_tokens`, `questions` (ids + types), `answers_raw`, `answers_calibrated`, `calibrator_version`, `repairs` (e.g. renormalized).
- `usage` (tokens in/out/cached), `neurons` or `cost_usd`, `ttft_ms`, `total_ms`, `step_count`.
- `gauges`: four 0–5 values, plus `outcome_labels` (set) and `outcome_source` (PR/issue/epic ids, with the sub-session vs parent vs anyone perspective from brief §9).

---

## 6. Cycle-safe composition (pools referencing pools)

- **Visited-set DFS**: standard. Mark nodes as *in progress* (grey) and *done* (black). Skip *done* nodes. An *in-progress* node is an ancestor; skipping it breaks the cycle without losing its result, because the ancestor finishes later. **[M]** standard graph algorithm (CLRS).
- **Brief's rule** ("if a pool has already been seen, skip it"): correct only if "seen" means *done*. If "seen" also covers in-progress nodes, skipping is still correct for cycles, but the record of what the skipped pool produced must say *cycle-skipped*, not *failed*. Otherwise the error trail misreports walled providers. This distinction is the main correctness point.
- **Datalog / semi-naive evaluation**: pool-to-pool references as edges, "reached" as facts, evaluated to a least fixpoint. Terminates because the fact set is finite and rules are monotone. **[M]**
- **Nix module fixpoint**: `lib.fix f = let x = f x; in x` with lazy attributes. Cycles are only safe if laziness never forces the cycle. Cyclic references that are forced yield "infinite recursion". So laziness does not solve cycles in control flow; use explicit marks instead. **[M]** nixpkgs `lib/fixed-points.nix`.
- **Kubernetes ownerReferences**: the garbage collector walks owner graphs, and cycle handling is designed around it **[M]**. Not verified in this pass.
- **Recommendation:** implement the pool traversal as DFS with grey/black marks and a `cycle-skipped` outcome. Keep the Nix-style fixpoint as a later option for mutual references that must produce values (e.g. cascaded weights), not for failover order.

---

## 7. Analysis framework: native vs cheap LLM, per provider

### 7.1 Decision rule

Choose native when, on our labelled decisions:
1. **Cost per correct decision** (price ÷ accuracy, at matched coverage) is lower than the best LLM option, and
2. Calibration after recalibration is within tolerance (ECE ≤ 0.05, Brier not worse than the LLM after calibration), and
3. Invalid-output rate is ~0 (native outputs do not need retries), and
4. Order sensitivity and repeat variance are small.

Choose cheap LLM when the LLM's **accuracy is materially higher** (≥ 5 points on the decision type) at ≤ 2× the native per-correct cost, or when native is unavailable, over context, or fails residency.

### 7.2 Measurements needed per provider

| # | Measurement | Method | Needed for |
|---|---|---|---|
| M1 | Accuracy vs outcome label | Label ~300–500 historical decisions from PR/issue outcomes | cost per correct |
| M2 | ECE (10 bins), Brier, reliability diagram, selective risk | After Platt/temperature fit (cross-validated) | calibration |
| M3 | Order sensitivity | Permute choice options 3×; flip rate of argmax | bias |
| M4 | Repeat variance | 10 repeats per item; std of probability | stability |
| M5 | Invalid rate, second-try success | Schema validation log | retry policy |
| M6 | TTFT and total latency P50/P95 | Our own timing, not vendor claims | executor timeouts |
| M7 | Realized $ per 1k | Provider usage fields; neurons from dashboard | budget |
| M8 | Free-allowance consumption | UTC daily counter | disengage rule |
| M9 | Context fit | Distribution of decision state tokens vs context limits (Clef-flash 24,576; Clef 65,536; Jev 64k/32k; Haiku 100k tier; DeepSeek 1M) | routing |
| M10 | Residency / ZDR | Vendor terms (Clef and Jev docs state ZDR for enterprise) | policy |

### 7.3 Provisional answer per provider (from public prices and docs only)

| Provider / model | Class | Provisional answer | Why | What would change it |
|---|---|---|---|---|
| Cloudflare Clef-flash | native | **Worth it, default tier while in the free allowance** | $0.076/1k; output free; ~1.4k/day free | M1 accuracy below LLM on our decisions; 24k context too small for long states |
| Cloudflare Clef 27B | native | **Worth testing for hard decisions** | 65k context; $0.48/1k | If its accuracy does not beat Clef-flash enough to justify 6× price |
| TypeSafe Jev 1.14 | native | **Worth it provisionally; the cheapest paid native option** | $0.084/1k; 64k/32k context; multi-question packing | Calibration claim fails M2; order-sensitivity fails M3; vendor concentration |
| OpenAI Decisions API | native (unpublished) | **Wait** | No schema, no price, limited preview | Public contract and price |
| OpenAI gpt-6-luna / gpt-5-nano (strict schema) | LLM | **Last-resort decider, measure against native** | $0.14–0.25/1k; strict schema makes invalid output near-zero | Beats native on M1 at ≤2× cost |
| Claude Haiku 5.5 | LLM | **Only if Anthropic-only constraint; otherwise comparator** | $0.25/1k; tool-forced JSON; no native probabilities | Same as above |
| DeepSeek flash (off-peak) | LLM | **Cheap-LLM tier for non-urgent batch decisions** | $0.36/1k; off-peak halves price; 1M context; cache hits $0.066/1k | Off-peak schedule conflicts with latency needs |
| Gemini 3.5 Flash-Lite | LLM | **Free-tier candidate; paid comparator** | Free tier shown; $0.85/1k paid | Free-tier rate limits and data-use terms (unverified) |
| Groq / Cerebras small fast models | LLM (very fast) | **"Super-fast free" candidate per brief §4; validate JSON** | Third-party prices and free-tier reports | Catalogue churn (reported); JSON validity |
| Local classifier (ONNX / embeddings + LR / SetFit) | local | **Not yet; build after 300+ labels** | Zero marginal cost; no labels now | Labels exist; latency and privacy wins |
| Cohere Rerank | reranker | **Retrieval ranking only, not decisions** | Not a multi-question decision model | Price unverified |
| OpenAI moderation | fixed classifier | **Safety gate only** | Free; fixed categories; no injection category | — |

### 7.4 Answer to the brief's question

Is a native decision model worth it, or is a cheap LLM (DeepSeek flash class) the better buy?

- **On price alone, native wins**, by about 2–4× per decision against the cheapest comparable LLM (gpt-5-nano $0.14/1k, DeepSeek off-peak $0.36/1k), and by more with multi-question packing.
- **On quality, unknown.** The decision depends on M1–M3 on our own labels. The public evidence is vendor claims and blog posts.
- **Provisional answer:** use native (Clef-flash in the free allowance, Jev as paid default) and keep one cheap LLM (DeepSeek off-peak, or gpt-5-nano) as the second tier, with luna/Haiku as last resort. Then measure. This matches the brief's pool order (free → plan → PAYG) only if free native capacity is large enough; it is not, at 1.4k decisions/day.

---

## 8. Design options catalog

Each option lists placement (V2/V3/V4), cost and main risk. "Pool" means decision pool; "executor" is the decision skill/runbook/mcpx tool.

| # | Option | Covers | Placement | Cost / effort | Main risk |
|---|---|---|---|---|---|
| O1 | Adopt systemone door as the native tier; route `{state, questions}` only; compile clauses before the call | Endpoint, executor | V2 | Low; door exists | Door is Clef-only; Jev access path to confirm |
| O2 | Ordered pool: native free (Clef-flash, Workers AI allowance) → native paid (Jev) → LLM free (Gemini, Groq/Cerebras) → LLM PAYG (DeepSeek, gpt-5-nano) → last resort (luna/Haiku) → default | Decision pool | V2 | Medium | Free allowances are small; pool order must follow measured cost per correct |
| O3 | Daily neuron/token budget counter per provider, disengage at budget, not only at 429 | Pool, executor | V2 | Low | Paid Workers AI has no hard stop; counter must be ours |
| O4 | Executor skill with strict input/output schema (choice, score, noul), returns validated probabilities | Executor | V2 | Medium | Two input formats in brief; need normalizer |
| O5 | Invalid-JSON path: class failures; repair sums locally; retry 2× with schema + errors + hint; then next route; then default | Executor, retry | V2 | Low | Retry cost in tokens; thresholds are guesses |
| O6 | Constrained decoding where available (OpenAI strict, llama.cpp grammar, XGrammar for self-hosted) | Executor | V2 for APIs; V3 for local | Low for API | Reasoning quality on constrained paths (CRANE claim, unverified) |
| O7 | Per-decision ID with 4 numeric gauges (caller/callee correct/wrong, 0–5) in our table; export to Langfuse | Feedback record | V2 | Low | Gauge meaning drift; need score configs |
| O8 | Full probability vector logged + prompt version + model version (no argmax-only logs) | Feedback record | V2 | Low | Storage growth (brief §3 already says measure it) |
| O9 | Outcome labels as a set in our table; Langfuse Boolean per label for dashboards | Feedback record | V2 | Low–medium | Label taxonomy churn |
| O10 | `Decision-Id:` git trailer + task link so PR/revert outcomes join by key | Feedback, attribution | V2 | Low | Discipline in every session; needs convention in PR skill |
| O11 | Conditional layer as CEL effect expressions over answers; effects are log-odds boosts, defaults, narrowing | Conditional layer | V3 | Medium | Rule stacking; need caps and tests |
| O12 | Conditional layer as JsonLogic (JSON-only portable form) | Conditional layer | V3 (alt) | Medium | Verbose; weaker tooling than CEL |
| O13 | DMN-style decision tables for small, human-reviewed combinations (≤ 8 rows) | Conditional layer | V3 | Medium | Explosion beyond a few inputs |
| O14 | `x-clauses` envelope key compiled by the executor; endpoint contract unchanged | Conditional layer, protocol | V2 for the envelope; V3 for clause features | Low | Keep `x-` naming; check JSON Schema annotation rule |
| O15 | Dependent questions as sequential calls with answers in `state`; CPT for ≤3-parent groups | Dependent probabilities | V3 | Medium | Error compounding; use probabilities as soft evidence |
| O16 | Calibration layer: Platt or temperature per decision type and model, versioned | Probabilities | V3 (after labels) | Medium; needs labels | Label scarcity; drift |
| O17 | Permutation test on choice order before trusting a model in a pool | Model evaluation | V2 | Low | Cost of 3× calls for evaluation only |
| O18 | Offline replay of logged decisions against candidate models (counterfactual via stored inputs and probability vectors) | Evaluation | V3 | Medium | Needs stored state (retention policy) |
| O19 | Exploration on near-tie pools to create real propensities for IPS | Learning | V4 | Medium | Costs quality on exploration; needs guardrails |
| O20 | Cycle-safe DFS with grey/black marks and `cycle-skipped` outcome for pools | Pool traversal | V2 | Low | Mislabelled outcomes if skipping is recorded as failure |
| O21 | Nix-style fixpoint for mutual references that produce values | Pool/weights | V4 | High | Infinite recursion risk; not needed for failover |
| O22 | Local classifier (ONNX / embeddings + LR / SetFit) trained on our labels as a free tier | Pool | V3–V4 | Medium–high | Needs 300+ labels; maintenance |
| O23 | Human bulk-review UI (review worked items, sets, labels, reprioritise queue) | Feedback capture | V3 | High | Scope creep; start with CLI/label file |
| O24 | Use the OpenAI Decisions API as a pool member once it has a public contract | Pool | V4 | Low once published | Unknown contract and price; preview access |

Recommended first slice (V2): O1, O2, O3, O4, O5, O7, O8, O10, O14, O17, O20. These are low-effort and make the data needed for the V3 decisions.

---

## 9. Top recommendations

1. **Measure before choosing a default.** Label 300–500 historical decisions from PR/issue outcomes, then run M1–M5 on Clef-flash, Jev, gpt-5-nano and DeepSeek off-peak. The price table shows native is cheapest; only measurement shows whether it is best per correct decision.
2. **Keep the endpoint standard and put clauses in an `x-` envelope compiled by the executor.** The door drops unknown keys, so this must happen before the call. Use CEL for conditions and effects; templates only for rendered text.
3. **Log full probability vectors, prompt and model versions, and a decision ID with four numeric gauges.** Counterfactual evaluation needs the vectors, not the argmax; attribution needs a stable key such as a `Decision-Id:` git trailer.
4. **Make the free-tier budget ours.** Cloudflare's included amount is 10k neurons/day (about $0.11/day), not $10, and Paid has no hard stop. Count neurons per UTC day and disengage at a budget you set.
5. **Use failure classes and a 2-try rule, and treat cycles as a separate outcome.** Refusal, truncation, schema-invalid and business-invalid need different handling; cycle-skipped pools must not be recorded as failed.

---

## 10. Open questions (recorded with defaults; detached run, no question tool)

- **Q: What does "Luna 100k" mean?** Default used: OpenAI `gpt-6-luna` as the last-resort decider (the only Luna in the public price list). Unverified. Ask Drew.
- **Q: Which "$10 included" does Drew mean?** Default used: Cloudflare Workers AI 10k neurons/day (≈$0.11/day). Ask Drew whether he means a different allowance.
- **Q: Is Jev 1.14 image input in our pinned version?** Default used: treat as text-only until verified (Firecrawl says text-only; TypeSafe models page says text and images for 1.14).
- **Q: Do Clef responses expose neuron counts?** Default used: no; measure with dashboard deltas.
- **Q: JSONSchemaBench failure rates?** Default used: none quoted. Fetch the results table before citing numbers.
- **Q: Groq/Cerebras official prices and free-tier limits?** Default used: third-party figures, marked [S].
- **Q: Cohere classify price?** Default used: not priced; reranker only.

## 11. Unverified or not fetched in this pass (list)

- Anything tagged [M] in the text.
- Groq and Cerebras official prices; Cohere prices; Hugging Face zero-shot; SetFit; llama.cpp grammar performance.
- BAML, Pydantic AI, Outlines/Guidance benchmark numbers.
- Langfuse-alternatives (LangSmith, Braintrust, Helicone, Phoenix) score schemas.
- Label Studio and Argilla feature claims.
- Microsoft Personalizer docs; Kubernetes GC cycle behaviour; Nix fixpoint details.
- Jev order-sensitivity and calibration claims (vendor and third-party only).
- OpenAI Decisions API details (only third-party reporting of the DevDay recap).
