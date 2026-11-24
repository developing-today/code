# FREE-CENSUS — Free Models & Free Tokens Report
**Date: 2026-11-23.** Research only; no config changes were made.
Threads: A = live config inventory (developing-today/code `origin/main`, `.opencode/opencode.jsonc` post-PR-#13648, 246,541 bytes, read via GitHub API; vault names via `fleet secrets list`, 232 names, names only). B = external census of our-orbit providers from their own APIs/docs, fetched 2026-11-23. C1/C2 = full models.dev universe walk (326 providers, split at `meganova`/`melious`), each provider checked against its own docs/pricing.

**Verification caveat (Thread A):** the running server's effective config could NOT be read this run — `GET 127.0.0.1:13420/api/config` via laptop-agent returned `UnauthorizedError` (server password exists in the vault by name but was not retrieved). "Enabled/serving" below rests on the file's switchboard rule (a provider is ON unless listed in `disabled_providers`) plus live-probe comments recorded in the config itself (dated 2026-11-14…16). Treat live-serving as strongly evidenced, not freshly read back.

---

## 1. HEADLINE ANSWERS

### 1a. Google AI Studio free pool — are all the free-pool models running for us?

**Mostly yes — the pool is enabled, keyed, and routed. Three qualifications and four untapped items.**

- The carrier is **not** the bare `google` provider id — that id is a deliberately **disabled, definitionless tombstone** (so the built-in catalog doesn't auto-activate it). The actual AI Studio free tier rides **four per-project aliases**, all `instanceOf: "google"`, all pointing at `generativelanguage.googleapis.com/v1beta`, **none in `disabled_providers`** (i.e. all enabled):
  - `google.muse` (key: `google.gemini-api-key`), `google.gen-lang` (`google.aistudio.free-gen-lang-api-key`), `google.personal` (`google.aistudio.free-personal-api-key`), `google.drew` (`google.aistudio.free-drew-api-key`) — all four key names present in the vault.
- They are grouped by the **`google-free` round-robin provider route** into **`gemini-free-plan`**, seated in the `background` tier; `gemini-4-hydrogen` also fronts `gemini-hydrogen-plan`. Because free quota is per-model **per project**, the four projects multiply the daily buckets — the config seats each model at its top effort variant deliberately ("the free tier meters requests per day, not tokens").
- **Wired (5 models):** `gemini-4-hydrogen` (#xhigh), `gemini-3.8-flash`, `gemini-3.5-flash`, `gemini-3.5-flash-lite`, `gemini-3.1-flash-lite` (all #high). Config-recorded live probes (2026-11-16) show the three Flash/Lite models answering 200 on every project; Hydrogen answering 200 on 3 of 4 projects (entitled, 503 high-demand on the fourth); 3.8 Flash entitled everywhere (503 high-demand at probe times — a capacity answer, not a quota/auth refusal).
- **Not wired, correctly:** `gemini-4-carbon`, `gemini-4-argon` (429 — no free-tier quota; vendor-side block, not config), the `gemini-2.5` line (config probe: 404, retired for new users; externally still listed free but deprecated, rerouting to 3.8 Flash from 2026-12-01).
- **Free at Google but NOT tapped by us:** (1) **Gemini Decision 1** — 200 RPM / 50K TPM / **10,000 RPD free** (official rate-limits page) — a structured-decision API (yes/no, choice, rubric scores), not a chat model; our decision-model pool (`systemone`, Zen `jev-1.13-free`) doesn't include it. (2) **Gemini Embedding 2 / Embedding 001** — free, not wired anywhere. (3) **Gemma 4** — free on AI Studio; no Gemma is named for the Google aliases (Gemma 4 31B reaches us via the NVIDIA door instead). (4) The free audio/live/TTS/transcribe models — not wired (not chat-relevant).
- **Catches:** free-tier content is marked "Used to improve our products: Yes" on Google's pricing page for every free model. The separate PAYG project (`google.developing-today`, route `google-payg`) is seated nowhere and its prepay balance is depleted (402) — that's the paid door, not the free pool.
- **Bottom line:** every chat model Google actually gives our projects free quota for is wired and multiplied ×4 projects. The gaps are Decision 1 and the embeddings, not the chat pool.

### 1b. Is there free tokens we aren't tapping?

**Yes — a lot, in three buckets:** (i) doors we already have but under-use (NVIDIA's 83-model free catalog — we seat 5; OpenRouter stuck at the 50/day tier a one-time $10 unlocks to 1,000/day; Groq's non-chat free shelf), (ii) keys with no door or doors switched off (SambaNova key in the vault with **no provider in the config at all**; `_commandcode` disabled with 5 free models catalogued; Cloudflare Workers AI disabled with its vault token **revoked**), and (iii) whole providers not in our orbit with real recurring volume (LongCat 500K tokens/day, Kilo 200 req/hr, ModelScope 2,000 calls/day, SenseNova's rolling credit pools, OpenAI's data-sharing tokens). Full ranked shortlist in §3.

---

## 2. MASTER TABLE — free offers vs. our wiring

Verified 2026-11-23. "Wired" is from Thread A (file state on origin/main). Full per-provider detail in §5–§6; the complete 326-provider appendix is §7.

| Provider | Free models / allowance available now (own-source) | Wired for us? | Size / capability note |
|---|---|---|---|
| OpenCode Zen (`opencode`) | 11 free chat models, unquantified quota, "limited time": DeepSeek V4.1 Flash Free, MiMo V2.6 Flash Free, Nemotron 3.5 Super VL/Lightning Free, Big Pickle, Beam, Iceman, Flint 40B, Hy4 Flash, Tidepool, Jev 1.13 (decision) — https://opencode.ai/docs/zen/ + live models API | **Yes** — `zen-plans` route feeds 11 free pools (flash-free, inkling, glm-5-3-flash-zen, flint, big-pickle, nemotron-vl, nemotron-lightning, mimo-flash, systemone…) | Small/mid models, limited context — seated as last-resort legs by design |
| OpenCode Go (`opencode-go`) | 3 free models on the plan surface: Beam Free, LongCat 2.5 Free, Iceman Contributor Free (probe-verified 2026-11-14) | **Yes** — `opencode-go-plans` route; all 3 seated in scout | Free legs ride the paid Go subscription accounts (×4) |
| Google AI Studio (`google`) | Gemini 4 Hydrogen (1M ctx), 3.8 Flash, 3.6 Flash, 3 Flash Preview, 3.5/3.1 Flash-Lite, 2.5 Pro (deprecated), Gemma 4, Decision 1 (10K RPD), Embedding 2/001 — per-model per-project rate limits — https://ai.google.dev/gemini-api/docs/pricing | **Yes** — 5 chat models × 4 projects via `google-free` → `gemini-free-plan` (see §1a) | Hydrogen is flagship-class Flash, 1M ctx; free-tier data trains Google products |
| NVIDIA NIM (`nvidia`) | 40 RPM, no published daily/token cap, no per-token billing; 83-model free catalog incl. Nemotron-3-Ultra 550B, Mistral Large 4 (1T-class), Kimi K3.1 Pro, DeepSeek V4.1 Flash, Gemma 4 31B — https://build.nvidia.com/ FAQ + live `/v1/models` | **Partially** — `nvidia-free-plan` seats 5 of 83; key claimed in config comments but **no `nvidia` name in `fleet secrets list`** (likely credential store) | The largest uncapped free allowance found; prototype framing, data recorded per trial terms |
| Groq (`groq`) | Free plan, per-model-per-org, stackable: gpt-oss-120b / gpt-oss-20b / Qwen3.8-27B at 200K TPD each (~600K TPD total), Whisper 28.8K audio-sec/day, prompt-guard ~500K TPD each — https://console.groq.com/docs/rate-limits | **Partially** — `groq-free-plan` seats the 3 chat models but the plan is in **no tier route** (TPM caps 7–8K < a session's ~26K first request; tiny-prompt seats only). 8 catalogued models unrouted (Whisper ×2, Orpheus ×2, prompt-guard ×2, Allam, safeguard) | Very fast decode; 8K TPM is the binding choke |
| OpenRouter (`openrouter`) | 15 models at $0/$0 (incl. Nemotron-3-Ultra 550B :free, ERNIE-5.5-Flash preview :free 1M ctx, Gemma 4 26B/31B :free); :free cap 50 req/day, **1,000/day after one-time $10 lifetime top-up** — https://openrouter.ai/api/v1/models + /docs/limits | **Yes, at the low tier** — `openrouter-free-plan` seats 4 (Ling 3.2 Flash, Nemotron Ultra, Flint, Cohere North Mini Code); no top-up recorded; **no `openrouter` name in `fleet secrets list`** | 550B free is real but capped by platform req/day; upstream 429s common |
| Z.ai (`zai`) | GLM-4.7-Flash, GLM-4.5-Flash, GLM-4.6V-Flash at $0 in/out permanently; no published numeric cap — https://docs.z.ai/guides/overview/pricing | **Yes** — `glm-flash-free-plan` seats 4.7 + 4.5 (4.6V vision not seated) | 200K ctx Flash models; limits unpublished |
| Cloudflare Workers AI (`cloudflare-workers-ai`) | **10,000 Neurons/day** on Free AND Paid plans (~160–550 LLM calls/day depending on model); catalog incl. gpt-oss-120b, GLM-5.3/5.2, Kimi K2.x, Nemotron-3-120B, DeepSeek V4, Llama 3.3 70B — https://developers.cloudflare.com/workers-ai/platform/pricing/ | **No** — provider is a disabled tombstone, no definition; vault AI-inference token is **`cloudflare.api-token-ai-inference.REVOKED`** | Small absolute volume (~$0.11/day list) but daily, permanent, already included in any Workers account |
| SambaNova (`sambanova`) | Free tier (no payment method): ~20 RPD × 200K TPD per model (secondary trackers; official page didn't render numbers) — gpt-oss-120b, Llama 3.3 70B, DeepSeek-V3.1, Gemma 4 31B — https://docs.sambanova.ai/docs/en/models/rate-limits | **No — key without a door:** `sambanova.api-key` is in the vault; **no sambanova provider/catalog/pool/comment exists in the config at all** | 20 req/day/model is demo-scale per model but stacks across models; all models classed Preview |
| Cohere (`cohere`) | Trial keys: 1,000 API calls/month, 20 RPM chat (Command A family, R7B, Embed, Rerank) — https://docs.cohere.com/docs/rate-limits | **Catalogued, seated nowhere** — provider enabled, 7 models defined, "direct use only"; `cohere.api-key` in vault | Evaluation-only per Cohere terms; small volume regardless |
| Command Code (`_commandcode`) | 5 free-named catalog models: Laguna S 2.1 Free, Ling 3.0/3.1 Flash :free, Beam :free, Glyph Cluster :free | **No** — provider **disabled** (its 3 paid DeepSeek/Muse legs in deepseek pools are inert while disabled); `commandcode.api-key` in vault | Re-enabling is one flip but also reactivates its paid pool legs |
| EXAONE (`exaone`) | `k-exaone_v2.5` free on EXAONE API Platform | **Yes** — `exaone-free-plan` | ⚠️ **Key expires 2026-12-16** (30-day platform keys, issued 2026-11-16) — rotate or lose the leg |
| LongCat (`longcat`, Meituan) | **500,000 tokens/day default, increasable to 5M/day on application**, 1M-ctx LongCat-2.x, OpenAI- + Anthropic-compatible — https://longcat.chat/platform/docs/ | **No** (not a config provider; LongCat 2.5 Free reaches us only via Go/Zen plan legs — the direct platform quota is separate and untapped) | Flagship-grade recurring volume; no card mentioned |
| Kilo (`kilo`) | `:free` models + Auto Free router at $0, **200 requests/hour per IP**, incl. Nemotron Ultra-class — https://kilo.ai | **No** | Generous request cap; free routes may log prompts; roster shifts with upstream promos |
| ModelScope (`modelscope`) | API-Inference: **2,000 calls/day per user, 200/model/day**, Qwen/DeepSeek/GLM/MiniMax — https://modelscope.cn/docs/model-service/API-Inference/intro | **No** | Requires account bound to a real-name-verified Alibaba Cloud account — practical barrier outside China |
| SenseNova (`sensenova`) | Token Plan **Free public beta ¥0**: 60,000 credits per rolling 5h per pool (~1,500 calls/5h), DeepSeek V4 Flash / GLM-5.2 / own Flash-Lite lanes — https://platform.sensenova.cn/docs | **No** | Beta — paid tiers announced, no end date; mainland China phone signup |
| SiliconFlow (`siliconflow`) | $0 models (Qwen3-8B, R1-Distill-7B, GLM-4-9B, BGE embed/rerank…) with high fixed limits (~1K–10K RPM, 50K–5M TPM by size) + $1 new-user credit — https://cloud.siliconflow.com/models | **No** | Small models + embeddings stack; identity verification required |
| OpenAI (`openai`) | **Data-sharing complimentary tokens: 250K–1M flagship + 2.5M–10M small-model tokens/day** (GPT-5.x/4.1/4o/o-series + mini/nano), recurring; free Moderation endpoint — https://platform.openai.com/docs/models + Help Center data-sharing terms | **No** — we have `openai-payg` (key in vault) but no data-sharing opt-in recorded | Biggest frontier free volume in the universe walk — paid for with your traffic data; needs $5 prepaid minimum + positive balance |
| Requesty (`requesty`) | Free tier on free models, full gateway: **200 requests/day, no expiry, no card** — https://www.requesty.ai/free-models | **No** | Free-model roster rotates with upstreams |
| UnoRouter (`unorouter`) | Aggregates ~50 upstream free tiers: **130+ `:free` models behind one key**, own cap 1 req/min/model — https://unorouter.com/en/blog/free-models-aggregated | **No** | Breadth not reliability: shared pools drain (429/503), part community/web-chat adapters |
| IO.net (`io-net`) | Free Standard plan: daily-refreshed credits, email-only signup, no card; **amount not published** — https://io.net/docs/guides/payment/io-intelligence-payments | **No** | Unquantified — needs a measurement signup |
| IBM watsonx (`watsonx`) | Lite plan, always free: **300K tokens/month** + 20 CUH runtime — https://www.ibm.com/docs/en/watsonx/saas | **No** | Steady low volume; IBM Cloud account (card for identity verification) |
| Mistral (`mistral`) | ⚠️ **Free API tier effectively gone** — today's docs: Free mode "doesn't include API access"; API requires billing activation (pay-as-you-go). Free plan = Vibe/Studio usage only, unquantified — https://docs.mistral.ai/admin/billing-usage/usage-limits | Provider enabled as a **paid reserve leg** (`mistral-plan`), key in vault (`mistral.api-key`); not a free leg | Do not count Mistral as free anymore (note: Thread C2's fast pass repeated the older "Experiment free tier" description; Thread B's verbatim doc quote supersedes it) |
| Cerebras (`cerebras`) | **No permanent free tier.** One-time $5 promo credit, 30-day expiry, payment method required to receive it; trial ceilings up to 1M TPD on gpt-oss-120b — https://www.cerebras.ai/pricing | **No** — no provider, no key name in vault | A sprint, not a supply |
| Hugging Face (`huggingface`) | Free accounts: **$0 inference credits** (pricing table: Monthly Credits: None). PRO $9/mo includes $2/mo credits — https://huggingface.co/docs/inference-providers/pricing | **No** — disabled tombstone, no definition, no key name | The old free serverless allowance no longer exists |
| GitHub Models (`github-models`) | **RETIRED 2026-07-30** — playground, catalog, API, BYOK all gone — https://docs.github.com/en/github-models/use-github-models/prototyping-with-ai-models | N/A — not even in models.dev anymore; config carries only the separate `github-copilot` tombstone | Copilot Free (2,000 completions + 50 chat/mo) still exists but is an IDE product, not a serving API |
| Together (`togetherai`) | No free tier/trial; $15 minimum first credit purchase — official billing docs | No provider in config | — |
| Fireworks (`fireworks-ai`) | $1 one-time starter credit (secondary); 10 RPM without payment method | No provider in config | Negligible |
| DeepSeek official (`deepseek`) | Paid only, no free allowance — but note our config's temporary PAYG tail uses it (PR #13648) | Enabled (paid) | Cheap ≠ free |
| xAI (`xai`) | No free tier/credit published | No provider in config | — |


---

## 3. UNTAPPED FREE TOKENS — ranked shortlist (full universe)

Ranked by usable volume × model usefulness. Tags: **[R]** recurring / **[1×]** one-time · card requirement · the one action that taps it.

### Recurring

1. **NVIDIA NIM — the other ~78 models of the catalog** · [R] · no card · Already tapped for 5 models; the same door's free catalog (40 RPM, no published cap) holds Kimi K3.1 Pro, Mistral Large 4, Nemotron 3.5 Super VL 120B, GLM-5.3-Flash and dozens more. **Action:** probe + seat more of the 83 (config comment warns `kimi-k2.6` and `nemotron-3-super-120b` answered 404 for our account — probe per model first). First resolve where the key lives: no `nvidia` name in `fleet secrets list` despite the config claiming a stored key.
2. **LongCat direct platform** · [R] · no card mentioned · 500K tokens/day (→5M on application), 1M-ctx frontier model, dual-compatible endpoints. **Action:** create account, issue key, add provider + plan; the Go/Zen LongCat legs we already seat are a different, smaller shelf.
3. **OpenAI data-sharing tokens** · [R] · $5 prepaid minimum · 250K–1M flagship + 2.5M–10M small tokens/day on GPT-5.x-class models — the largest frontier free volume found anywhere. **Action:** a *decision*, not just a tap: opt the `openai-payg` org into data sharing (inputs/outputs shared with OpenAI; ZDR/Enterprise orgs excluded). Flagged for Drew rather than assumed.
4. **Kilo Gateway :free** · [R] · no card · 200 req/hr/IP on $0 models incl. Nemotron Ultra class. **Action:** free account + key, add provider. Catch: free-route prompts may be logged — route non-sensitive volume.
5. **OpenRouter $10 unlock** · [R after 1× $10] · $10 lifetime top-up (one-time) lifts :free from 50 → 1,000 req/day permanently. **Action:** top up the account behind our existing key (also resolve the key's vault name — none found in `fleet secrets list`).
6. **SambaNova** · [R] · no card (adding one ends the free tier) · ~200K TPD/model × several models (gpt-oss-120b, Llama 3.3 70B, DeepSeek-V3.1). **Action:** the key is already in the vault (`sambanova.api-key`) — just add the provider definition, catalog, and a plan. The single cheapest config-only tap in this report.
7. **`_commandcode` free shelf** · [R] · key already in vault · 5 free models catalogued (Laguna S 2.1 Free, Ling 3.0/3.1 Flash :free, Beam :free, Glyph :free). **Action:** remove `_commandcode` from `disabled_providers` + seat the free models — noting the same flip reactivates its 3 paid legs inside the DeepSeek pools, so do it as one deliberate change.
8. **Cloudflare Workers AI** · [R] · no card · 10,000 Neurons/day (~160–550 calls/day), already included in any Workers account we hold. **Action:** mint a fresh Workers AI token (the vault one is revoked by name), restore a provider definition, catalogue the small models (GLM-4.7-Flash is the neuron-cheapest useful one).
9. **Groq non-chat shelf** · [R] · key exists (location unresolved, see #1 note) · Whisper large-v3 + turbo (2,000 RPD, 28.8K audio-sec/day), prompt-guard pair (~500K TPD each), Orpheus TTS — all catalogued, none routed. **Action:** seat them for transcription/guard/TTS micro-seats; the chat pool stays tiny-prompt-only because free TPM (7–8K) can't carry a session's first request.
10. **ModelScope API-Inference** · [R] · no card, but real-name Alibaba binding · 2,000 calls/day across Qwen/DeepSeek/GLM/MiniMax. **Action:** only if a real-name-verified Alibaba account is available; otherwise skip — friction, not config, is the blocker.
11. **SenseNova Token Plan (beta)** · [R while beta] · mainland phone, no card · ~60K credits/5h per pool, DeepSeek V4 Flash / GLM-5.2 lanes. **Action:** signup if a mainland number is available; treat as ephemeral (paid tiers announced).
12. **SiliconFlow free models** · [R] · identity verification, no card · $0 Qwen3-8B / R1-Distill-7B / GLM-4-9B + BGE embeddings/rerank at high TPM ceilings. **Action:** account + verification; best untapped *embeddings* freebie in the walk.
13. **Requesty free tier** · [R] · no card · 200 req/day gateway on free models, no expiry. **Action:** account + key; roster rotates.
14. **UnoRouter** · [R] · no card (GitHub/Discord sign-in) · 130+ `:free` models, 1 req/min/model. **Action:** one key; breadth for scout-class calls, expect 429/503 churn.
15. **IO.net free Standard** · [R] · email only · daily credits, amount unpublished. **Action:** one measurement signup; promote only if the daily pool proves ≥100K-token-equivalent.
16. **Google Decision 1 + Embeddings** · [R] · key already in vault (×4 projects) · Decision 1 at 10K RPD/project is the largest single free quota at Google; Embedding 2/001 free. **Action:** catalogue both on the existing `google.*` aliases; Decision 1 fits decision/classification seats (alongside `systemone`), embeddings fit atlas/id ingest — neither is a chat seat.
17. **Cohere trial shelf** · [R] · key in vault · 1,000 calls/month, already catalogued, seated nowhere. **Action:** none needed for access (direct use works today); seating it buys little — evaluation-only terms and small volume. Listed for completeness.
18. **IBM watsonx Lite** · [R] · card for identity check only · 300K tokens/month forever. **Action:** IBM Cloud account; steady trickle, below the volume flag threshold.

### One-time (recorded, not recurring)

| Source | Amount | Friction | Note |
|---|---|---|---|
| Tencent TokenHub | **1M free tokens per model** × many models, valid 1 year | Tencent real-name verification; ~$10 balance freeze reported | Largest one-time aggregate if verification is feasible |
| Alibaba Model Studio | ~1M tokens/model, 90 days | Real-name verification (China) / region-locked | Frontier Qwen eval |
| Snowflake Cortex trial | $400, 30 days | **No card to start** | Warehouse-resident LLM work |
| OCI (Oracle) | $300, 30 days | Card/debit verification | GenAI sprint; nothing recurring after |
| Vultr promo | up to $300, ~30 days | Card/PayPal ($5 min deposit via PayPal) | Coupon-dependent — verify in-account |
| Azure free account | $200, 30 days | Card + phone | Spendable on Azure OpenAI/Foundry |
| AWS free tier | $100 + up to $100 onboarding, 6 months | Payment method may be requested | Spendable on Bedrock |
| OVHcloud | $200, 1 month | Account + project | EU-sovereign trial |
| Scaleway | 1M tokens + 60 min audio | Account | Real tokens across Qwen/Llama/Mistral/gpt-oss |
| Volcengine Ark | 500K tokens per text model | China platform, real-name | Per-model multiplication |
| Meta Model API | $20 signup credit | US developers, no card stated | Muse Spark frontier testing (~4–5M blended tokens) |
| Cerebras | $5, 30 days | **Payment method required** | 1M TPD ceiling on gpt-oss-120b while it lasts |
| Sarvam | ₹100 (~$1.20), never expires | Verified account | ~4–10M tokens of its own cheap Indic models |
| Fireworks | $1 | — | Negligible |

**Anti-entry — Google Vertex $300 trial:** as of **2026-11-16** the GCP trial credit no longer covers Vertex/Gemini platform services for new/upgraded accounts. Do not plan on free Vertex for a new signup.

### ⭐ Untapped larger / frontier-class free (subsection)

- **NVIDIA NIM (already our door):** Nemotron-3-Ultra **550B**, Mistral Large 4 (**1T-class MoE**), DeepSeek V4.1 Flash (552B MoE), Kimi K3.1 Pro, Nemotron 3.5 Super VL **120B** — all at the same 40 RPM free limit. The single biggest untapped frontier pool, and it's behind a key we already have.
- **OpenRouter :free:** Nemotron-3-Ultra 550B :free (1M ctx) and ERNIE-5.5-Flash-Preview :free (1M ctx) — gated by the 50→1,000 req/day platform cap (see shortlist #5).
- **OpenAI data-sharing:** GPT-5.x-class flagship tokens at 250K–1M/day — frontier volume, priced in data (shortlist #3).
- **Kilo / UnoRouter free routes:** Nemotron Ultra-class models at $0 through gateways (shortlist #4, #14).
- **Cloudflare catalog (neuron-expensive but present):** GLM-5.3/5.2, Kimi K2.5–K2.7, Nemotron-3-120B, DeepSeek V4 Pro — drawable against the 10K neurons/day if ever worth the burn rate.
- **Google:** Gemini 4 Hydrogen (flagship-class, 1M ctx) is already wired; Gemini 2.5 Pro free until its 2026-12-01 reroute — but our projects get 404 on the 2.5 line, so it's not actionable for us.

---

## 4. STALE / DISAPPEARED / AT-RISK FREE LEGS WE STILL CARRY

| Item | State today | Evidence |
|---|---|---|
| **EXAONE free key** | **Dies 2026-12-16** — leg goes dark unless rotated | Config comment: platform keys expire 30 days after issuance (issued 2026-11-16); rotate the vault item + credential-store row |
| **Mistral free API tier** | **Gone** — Free mode no longer includes API access (today's docs, verbatim). Our `mistral` provider is a paid reserve leg so nothing breaks, but no free Mistral allowance exists to tap; `_vibe`'s Mistral models ride the separate Vibe sign-in key | Thread B, docs.mistral.ai billing/usage-limits fetched 2026-11-23 |
| **OpenRouter provider comment** | **Stale comment in our own config**: the provider block still says "OFF via disabled_providers… refresh the model list when re-enabling" — false; the provider is enabled and `openrouter-free-plan` is routed (2026-11-15 pool). Harmless but misleading | Thread A, config line 1312 vs switchboard |
| **Groq free Llama models** | `llama-3.3-70b-versatile` / `llama-3.1-8b-instant` are **Enterprise-only now**; already removed from our provider block (endpoint stopped serving them) — no stale leg remains, noted so nobody re-adds them | Thread B (Groq models page) + Thread A comment |
| **Zen free roster rotation** | Roster rotates without notice: Nemotron 3 Ultra Free was dropped from Zen 2026-11-22 (secondary changelog tracking). Our Zen seats use other models, and Ultra reaches us via NVIDIA/OpenRouter — but every Zen free leg is inherently perishable | Thread B, Zen section |
| **MiniMax free leg** | `minimax-flash-plan` already deleted 2026-11-16 after Zen delisted `minimax-m3.1-flash-free` — handled, recorded here so the absence isn't mistaken for a gap | Thread A |
| **Cloudflare AI token** | Vault name is literally `cloudflare.api-token-ai-inference.REVOKED` — a dead credential is the only AI-inference token we hold | Thread A, vault list |
| **GitHub Models** | Retired 2026-07-30. We carry no leg (only the unrelated `github-copilot` tombstone) — nothing to clean, listed to close the question | Thread B |
| **HF free serverless** | Free-account inference credits are $0 now. Our `huggingface` entry is a definitionless tombstone — nothing live to clean | Thread B + Thread A |
| **Google PAYG project** | `google.developing-today` prepay depleted (402), route seated nowhere — a dead paid door sitting next to the healthy free pool; fund or forget deliberately | Thread A, config lines 1208–1216 |
| **Key-location gaps** | `groq`, `nvidia`, `openrouter` have **no matching names in `fleet secrets list`** although config comments claim stored, probe-verified keys. Presumed to live in the credential store under other names — resolve before concluding anything is missing | Thread A, §5 |

---

## 5. WHAT WE ALREADY RUN — free pools in the live config (Thread A detail)

Free pools (`experimental.model_routes`), all enabled unless noted:

- **Go + Zen plan legs:** `beam-plan` (Beam Free), `longcat-plan` (LongCat 2.5 Free), `iceman-plan` (Iceman Free), `hy4-flash-plan` (Hy4 Flash Free) — each served by both `opencode-go-plans` and `zen-plans` routes (4 accounts each).
- **Zen-only:** `flash-free-plan` (DeepSeek V4.1 Flash Free), `inkling-plan` (Inkling 2 Small Free), `glm-5-3-flash-zen-plan`, `flint-plan` (Flint 40B), `big-pickle-plan`, `nemotron-vl-plan`, `nemotron-lightning-plan`, `mimo-flash-plan` (MiMo V2.6 Flash Free), `systemone` (Jev 1.13 — decision models only, never a chat seat).
- **Direct doors:** `groq-free-plan` (3 chat models; in no tier route — tiny-prompt only), `nvidia-free-plan` (5 models), `openrouter-free-plan` (4 models), `glm-flash-free-plan` (Z.ai: GLM-4.7-Flash w3, GLM-4.5-Flash w1), `gemini-free-plan` (Google: 5 models × 4 projects), `exaone-free-plan` (1 model).
- **Tier placement:** `scout` leads with beam/longcat/iceman/inkling/openrouter-free before falling to paid legs. `background` runs the full free cluster (flash-free, gemini-free, exaone, hy4, glm zen + z.ai, flint, big-pickle, nemotron ×2, nvidia, openrouter) before `workhorse`. Zen free models sit last-resort wherever they fit (limited context, per config comments).
- **Disabled providers (verbatim, current):** `_commandcode`, `google` (tombstone — aliases carry the real service), `google-vertex`, `cloudflare-ai-gateway`, `vercel`, `huggingface`, `github-copilot`, `cloudflare-workers-ai`, `meta` (PAYG door; same models serve via `_muse`/plans), `_antigravity` (excluded from this census per brief).
- **Catalogued but unrouted free:** Cohere's 7 trial models; Groq's 8 non-chat models; `_commandcode`'s 5 free models (provider disabled); Google's Carbon/Argon (no quota) — see §2/§4.
- **Vault-only, no config provider at all:** `sambanova.api-key` (the standout), plus `baseten.api-key`, `modal.token-id`/`modal.token-secret` (Modal's $30/mo Starter credit is compute-only — its own pricing page excludes hosted Model API tokens, so this is not a token freebie).


---

## 6. APPENDIX — full provider-by-provider walk (models.dev universe, 326 providers)

Universe: `https://models.dev/api.json` fetched 2026-11-23 — 326 provider ids. Split at index 162/163 (`meganova` | `melious`). Local runtimes excluded as not cloud APIs: `atomic-chat`, `lmstudio`, `lynkr`. Method note (C1): long-tail negatives are "no free offer found on the provider's own docs/pricing/model list as listed in models.dev" — checked to that depth. models.dev $0 model rows alone were treated as leads, not proof (most are subscription-included plan models).

### 6a. First half (302ai → meganova) — free-offer providers (Thread C1)

| id | offer | R/1× | amount | friction |
|---|---|---|---|---|
| alibaba | Model Studio new-user free quota | 1× | ~1M tokens/model, 90 days | Account; real-name verification (China site); region-locked |
| alibaba-cn | Same, China deployment | 1× | ~1M tokens/model, 90 days | Real-name verification |
| amazon-bedrock | AWS Free Tier credits spendable on Bedrock | 1× | $100 + up to $100 onboarding, 6 months | AWS account; payment method may be requested |
| anthropic | Small Console test credit (amount unpublished; ~$5 per third parties, unverified) | 1× | Small | Console account |
| azure | Free account credit (Azure OpenAI/Foundry) | 1× | $200, 30 days | Card + phone, ~$1 temp hold |
| azure-cognitive-services | Same Azure credit | 1× | $200, 30 days | Card + phone |
| google-vertex | GCP $300 trial — **since 2026-11-16 no longer covers Vertex/Gemini for new accounts** | 1× | $300, 90 days (grandfathered only for Vertex) | Card ($1 temp auth) |
| google-vertex-anthropic | Same GCP trial situation | 1× | $300, 90 days, same exclusion | Card |
| cloudflare-workers-ai | Daily Neuron allocation (also in §2) | R | 10,000 Neurons/day | No card |
| github-copilot | Copilot Free (IDE product, not an API) | R | 2,000 completions + 50 chat/agent req/mo | GitHub account |
| io-net | Free Standard: daily-refreshed credits | R | Amount unpublished | Email only |
| kilo | `:free` models + Auto Free router | R | $0 models, 200 req/hr/IP | Free account, no card |
| longcat | Platform free daily quota | R | **500K tokens/day, →5M on application** | Signup, no card mentioned |
| clarifai | Community plan | R | 1,000 inputs/mo + 1,000 search ops/mo | Signup |
| aimlapi | Free Developer tier + trial credits | R + 1× | 10 req/hr free tier; credit amount unpublished | No card per listings |
| mallard | Signup credit + no-account test key | 1× (+micro R) | £1 credit; test key 5 req/hr | Email confirmation |
| hopper | Signup credit | 1× | $5 | **Card required** |
| jzstoken | Promo: one model $0 until 2026-11-30 | 1× promo | Unlimited calls to that model during promo | Signup |

**$0-catalog leads with NO published free allowance on the provider's own docs (unverified — not counted as free tiers):** `agnes` (3 $0 models), `aihubmix` (18 `-free` models, docs say pay-as-you-go), `amd` (11 $0, China dev portal), `bothub` (2 `:free`), `commandcode` (4 `:free` — but see §2: our own config catalogues 5 free models on this door with a key in the vault), `empiriolabs` (4 $0 incl. GLM Flash/Gemma 3 27B), `freemodel` (no verifiable limits), `hetzner` (2 $0 Qwen, experimental), `inferall` (6 $0 incl. GLM-5.3/Nemotron Ultra), `inferx` (12 $0 incl. DeepSeek V4 Flash/Qwen3-Coder-Next), `jad-ai` (3 $0), `kenari` (9 `:free`).

**Subscription-included, not free (checked):** `alibaba-coding-plan`, `alibaba-coding-plan-cn`, `alibaba-token-plan`, `alibaba-token-plan-cn`, `kimi-code-plan-cn`, `kimi-code-plan-global`, `kuae-cloud-coding-plan`, `cline-pass`, `gitlab`. `cloudflare-ai-gateway` — free gateway plan exists but provides no free inference tokens.

**Notable verified negatives:** `chutes` — own FAQ: "We do not offer a free tier at this time" (former 200 req/day retired 2026-03-15). `deepinfra` — pricing page lists only paid rates; card/pre-pay required before use.

**Checked, none (first half)** — provider's own doc/pricing surface as listed in models.dev, checked 2026-11-23:
302ai, abacus, ablilayeratedllm→abliteratedllm, abliteration-ai, above, adaption, adzuki, agentrouter, ai-router, ai21, aiand, aigrow, ainetcafe, aipg, aistupidlevel, aixy, aki-io, ambient, anonrouter, anyapi, apiko, arcee, augure, auriko, autarcho, bailing, baseten, bee, berget, blueclaw, boundless, bumo, byteplus, cedarllm, cheapaimodel, cheapest-token, cheapllm, claudinio, cloudcode, cloudferro-sherlock, concentrate, confidential-ai, coralbricks, cortecs, crossmodel, crusoe, daoxe, databricks, digitalocean, dinference, dothoosh, doubleword, drun, ebcloud, echo, edenai, engy, entrima→entrim, evroc, ezkielyna, fastrouter, fiftyx, fortytwo, frequency, friendli, frogbot, gisoo, gmicloud, graphn, greenpt, helicone, hostinger-router, hpc-ai, huawei-maas, hubris, humain, hyper, impossibl, inception, inceptron, inco, infer, inference, inference-coop, inferenceapis, inferris, infersia, infomaniak, isoquant, iteracompute, jalapeno, jiekou, joingonka, keln, kitani, klavo, klokintegration, korouter, kosmik, kunavo, liquid, lithosai, llmgateway, llmgateway-providers, llmtech, llmtr, logits, lowrouter, lucidquery, lyceum, meganova. (Full per-provider source URLs are in the Thread C1 working report; each was the provider doc URL from models.dev.)
Sibling-covered in this half, confirmed present: cerebras, cloudflare-workers-ai, cohere, deepseek, fireworks-ai, google, groq, huggingface. `github-models` is absent from models.dev entirely (consistent with its retirement).

### 6b. Second half (melious → zro) — free-offer providers (Thread C2)

| id | offer | R/1× | amount | friction |
|---|---|---|---|---|
| meta | Meta Model API signup credit (Muse Spark) | 1× | $20/new account | US developers, no card stated |
| mixlayer | `qwen/qwen3.5-4b-free` | R | 1 small model, no published cap | No card stated |
| modal | Starter included compute credit — **excludes hosted Model API tokens** (own pricing page, from 2026-09-01) | R | $30/mo compute credit | No card stated |
| modelscope | API-Inference free tier | R | **2,000 calls/day, 200/model/day** | Real-name-verified Alibaba account binding |
| moonshotai | Voucher after $5 cumulative top-up | 1× conditional | $5 voucher | $1 min top-up; $5 paid to unlock |
| moonshotai-cn | Registration voucher | 1× | ¥15, 3 months | Mainland China mobile |
| morph | Free tier | R | 200 requests/month | Account |
| nebius | Token Factory trial | 1× | $1, 30 days | **Card required** |
| novita-ai | Rotating $0 open models (+$100 Sandbox promo credit, explicitly NOT Model API credit) | R / 1× | Rotating set, no published cap | No card |
| oci | Oracle Cloud trial → OCI Generative AI | 1× | $300, 30 days | Card/debit verification |
| openai | **Data-sharing complimentary tokens** + free Moderation | R | 250K–1M flagship + 2.5M–10M small tokens/day | **$5 prepaid minimum**; data-sharing opt-in |
| opencode-go | Go plan `*-free` models | R | 9 free-listed models at $0 marginal | Go subscription surface |
| orcarouter | Own Fusion/free routes at $0 + BYOK | R | No published cap found | No card stated |
| ovhcloud | Public Cloud trial → AI Endpoints | 1× | $200, 1 month | Account + project (card per trial terms) |
| poe | Free plan compute points via API | R | ~300 points/day (approximate — no longer published by Poe) | No card |
| requesty | Free tier on free models, full gateway | R | **200 requests/day, no expiry** | **No card** |
| sarvam | Signup credit, never expires | 1× | ₹100 (~$1.20) | Verified account |
| scaleway | Generative APIs free tier | 1× | 1M tokens + 60 min audio | Account |
| sensenova | Token Plan Free public beta ¥0 | R (beta) | 60K credits/5h per pool, ~1,500 calls/5h/model | Mainland phone + SMS, no card |
| siliconflow | $0 models + new-user credit | R | $0/token within fixed per-model limits (to ~5M TPM) + $1 credit | Identity verification |
| siliconflow-cn | Same scheme, China site | R | $0 rows + ¥16 credit reported | Real-name verification (CN/HK/TW/Macau docs) |
| snowflake-cortex | Free trial → Cortex AI | 1× | $400, 30 days | **No card to start** |
| tencent-tokenhub | New-user free trial | 1× | **1M tokens per model**, valid 1 year | Tencent real-name verification |
| unorouter | Aggregated upstream free tiers, 130+ `:free` models | R | $0/token; own cap 1 req/min/model | No card (Discord/GitHub sign-in) |
| vercel | AI Gateway free tier — **buying any credits permanently ends it** | R conditional | $5 credits/month | Team account, no card |
| volcengine | Ark new-user quota per model | 1× | 500K tokens per text model | China platform, real-name |
| vultr | New-customer promo credit → Serverless Inference | 1× promo | Up to $300, ~30 days | Card/PayPal link ($5 min deposit via PayPal) |
| watsonx | Lite plan, always free | R | 300K tokens/month + 20 CUH | IBM Cloud account (card for identity verification) |
| zenmux | 2 standing free models (GLM-4.7-Flash-Free, GLM-4.6V-Flash-Free) + promo deal board | R | $0/token, limits unpublished | No card stated |
| zhipuai | China twin of `zai`: same 3 Flash models at ¥0 | R | ¥0/token, unpublished caps | China platform account |

Fast-pass sibling-covered in this half: mistral (see the correction in §2), nvidia, opencode (Zen), openrouter, sambanova, togetherai, xai, zai.

**Checked, none (second half)** — provider doc/pricing surface from models.dev, checked 2026-11-23:
melious, meragpt, merge-gateway, minimax, minimax-cn, minimax-cn-coding-plan, minimax-coding-plan, moark, model-oracle-ai, modelis, modular, moneo, munito, munito-eu, nan, nano-gpt (free web chat only, no free API), nearai, neon, neosmith, neuralwatt, nodecode, nokeep, nova, oathbreaker, obscurify, ofox, ollama-cloud ($0 plan is pay-as-you-go with no included usage), openreason, opper, outerlabs, paraloncloud, pareto, pendra, perplexity (old $5/mo Pro API credit cancelled Feb 2026), perplexity-agent, phala, phase, pioneer, poolside, powerspring, primalabs, prime, privatemode-ai (local product), proxystacks, qihang-ai, qiniu-ai, qvac (local SDK), rayline, reflection, refuseless, regolo-ai, routing-run, ruapi, runcrate, runinfra, sagg, sakana, salad-cloud, sandbase, sap-ai-core, scnet-token-plan, scx-ai, sokkan, sparrowcodes, stackit, standardcompute, steadyrouter, stepfun (15-day trial promo ended 2026-08-24), stepfun-ai, stepfun-ai-step-plan, stepfun-step-plan, stepping-stone, subconscious, submodel, superbot-gg, synthetic, synthorai, teamorouter, teapot-router, tencent-coding-plan, tencent-token-plan, tensorx, tenstorrent, the-grid-ai, therouter, thinkingmachines, tinfoil, toapis, tokengo, tokenify, tokenlab, tokenrouter ($0 catalog row but no published free terms), tokenwatt, trustedrouter, trustedtokens, umans-ai, umans-ai-coding-plan, unbiased, uncensuredai, unrestricted, uplinkswitch, upstage (startup program is application-gated, not an API tier), vancine, venice, verda, vispark, vivgrid, volcengine-coding-plan, wafer.ai, wallaby, wally, wandb, watt, xiaomi (MiMo API paid; free MiMo lives on Zen), xiaomi-token-plan-ams, xiaomi-token-plan-cn, xiaomi-token-plan-sgp, xpersona, xxapi, zai-coding-plan, zeldoc, zenifra, zerogpu, zhipuai-coding-plan, zro. (Count check by C2: 39 free-offer rows + 124 checked-none = 163. Per-provider source URLs are in the Thread C2 working report.)

---

## 7. METHOD + CAVEATS

- External facts were verified 2026-11-23 from each provider's own API/docs; items a worker could only corroborate via a secondary source reading the official page are marked ⚠️secondary in the thread reports (notably: SambaNova's numeric free limits, Poe's ~300 points/day, NVIDIA phone-verification reports, Fireworks' $1 credit).
- Thread B and Thread C2 disagreed on Mistral (C2's fast pass repeated the legacy "Experiment free tier" description; Thread B fetched Mistral's current billing docs stating Free mode includes **no API access**). This report follows Thread B.
- Config facts are from origin/main at read time (post-PR-#13648). The live server's effective config was not readable (auth) — see the caveat at the top.
- Nothing in this report changes any config. Every "action" in §3 is a proposal for a follow-up task.
