# AI Plans — Drew's Fleet

Draft built 2026-10-21 from the tracker set dated 2026-10-20
(`tracker-draft.md`, `ai-plans-2026-10-20.md`, `master-tracker.md`)
and `~/MEMORY.md`. **No secret values — credential locations by
Bitwarden key name only.**

Rules used throughout (same as the tracker):

- Every price/date/quota is copied from a source dated 2026-10-20 or
  earlier. Anything not in a source is marked **UNKNOWN** /
  **UNSOURCED** / **ESTIMATE-MISSING** rather than filled in.
- Conflicts between sources are shown as conflicts, never silently
  resolved. The tracker itself forbids overwriting receipt-backed
  entries with dictated ones until resolved.
- Prices older than ~30 days need re-verification against official
  vendor docs before anyone acts on them (standing rule from the SaaS
  pricing side chat).

## How to read the estimate columns

- **Estimated API-equivalent value** = what the plan's included usage
  would cost at the provider's own API/list rates, *only where a
  source states a multiplier or a metered pool*. Where a plan is a
  flat wall with no published token numbers, this is
  ESTIMATE-MISSING by design — Anthropic and OpenAI publish no
  token caps for consumer plans.
- **Estimated AA tasks** uses Drew's AInventory framing: a "standard
  AA task" costs what AInventory measured per task for a reference
  model at max effort. Reference rows (AInventory workbook
  2026-10-21 copy + `model_value_data.csv` 2026-10-19, via MEMORY):
  - GPT-6 Luna max — II 38 / CAI 41 — **$0.07 per AA task**
    (CSV says II 37; workbook comparison sheet says 38 — known
    source discrepancy, both preserved in AInventory)
  - DeepSeek V4.1 Flash max — II 39 — **$0.27 per AA task**
  - Astra max — II 53 — **$3.26 per AA task**
  - Claude Opus 5.5 max — II 58 — **$5.98 per AA task**
  - Gemini 4 Argon — CAI 64 (no II row) — cost/task **not in sources**
- AA-task numbers are only computed for **dollar-metered pools**
  (where pool ÷ cost-per-task is real arithmetic). They are labelled
  ILLUSTRATIVE: they assume the whole pool is spent on that one model
  at list rates, at max effort. For token-capped or wall-capped plans
  with no tokens-per-task figure in any source, the entry is
  **ESTIMATE-MISSING**.

## Fleet totals (2026-10-20)

- Receipt-confirmed: $21.09/mo + $579.78/yr in annuals ≈ **$69/mo
  equivalent**.
- Dictated, unverified, on top: OpenCode Go $20 + ChatGPT $100 +
  Muse $20 + ClinePass ~$10 + Command Code ~$10 ≈ **+$160/mo** →
  all-in ≈ **$230/mo equivalent** if every dictated item is real and
  distinct. Resolving the Claude/ChatGPT double-count conflicts moves
  this by ±$40.
- Only two plans are verifiably at pace (hitting walls): Claude and
  ChatGPT. Muse is measurably behind pace. Everything else is
  uninstrumented.
- Route-pool doctrine (Drew): burn cheap/API-priced pools first;
  giant subscriptions last, as reserve + burst capacity; harness
  subscriptions are for code access *through their harnesses*
  (pure-API use = ban risk, accepted); API-priced services are the
  ones to wire into business processes and future paid products.

---

## 1. Anthropic — Claude

| Field | Value |
|---|---|
| Plans held | **Pro (annual)** receipt-confirmed; a dictated **$20/mo tier** that is probably the same plan described two ways (CONFLICT, unresolved) |
| Cost | Pro: $216.00/yr list, **$227.88/yr charged** incl. tax, via Google Play (Mastercard …4126). List equivalents: $20/mo monthly or $17/mo annual ($200/yr list) |
| Last charge / renewal | 2026-10-04 / **renews 2027-10-04** |
| Quota | Rolling **5-hour session window** (Pro = at least 5× Free per session) + **weekly cap across all models**, fixed reset per account. Chat + Claude Code share ONE pool. **No token/message numbers published.** |
| % used | **Hitting limits** (Drew, self-reported). The 5-hour window is his biggest flow problem — worse than the weekly cap |
| Reset / early reset / expiry | Weekly reset datetime **UNKNOWN** (get from Settings → Usage). No early reset: block/wait, or opt-in usage credits at API rates. Annual plan expires/renews 2027-10-04 |
| Overage | WALL + optional API-rate usage credits (Sonnet-class ~$2 in / $10 out per M, Oct-2026 reporting) |
| Models | Claude models in app + Claude Code. OpenCode config IDs: `claude-code/claude-haiku-5-5`; via gateway: `anthropic/claude-opus-5.5`, `anthropic/claude-sonnet-4.5`, `anthropic/claude-sonnet-5.5` |
| API-equivalent value | ESTIMATE-MISSING (no published caps). Ladder for comparison: Max 5x $100/mo, Max 20x $200/mo — linear $/unit, no volume discount |
| AA tasks | ESTIMATE-MISSING (wall-capped, no metered pool). Reference: Opus 5.5 max = $5.98/AA task at API rates |
| Credentials | Google account sign-in (Play). No Anthropic API key in Bitwarden. Open question: is there also a direct Anthropic API account? No invoice/key found |

Future candidate (not purchased): **Claude $200 (Max 20x)** — Drew is
weighing it because he hits limits. Also wondered aloud about
$250/$500 tiers; no such purchased plan in sources.

## 2. OpenAI — ChatGPT / Codex

| Field | Value |
|---|---|
| Plans held | **Plus (monthly)** receipt-confirmed **plus** a dictated **$100/mo tier** (CONFLICT, unresolved — separate direct-billed plan on top of Play Plus, a replacement, or a double-count?) |
| Cost | Plus: $19.99/mo list, **$21.09/mo charged** incl. tax, via Google Play (Mastercard …4126). Dictated tier: $100/mo (no receipt yet) |
| Last charge / renewal | Plus: 2026-09-25 / **renews 2026-10-25**. $100 tier: UNKNOWN. Unexplained separate ChatGPT charge: $16.46 on Discover …3416, 2026-10-19 — API platform billing is the leading candidate, unconfirmed |
| Quota | Ladder (Oct 2026): Free $0, Go $8, Plus $20 (=1x), **Pro $100 = 5x Plus usage + 5x Codex, NO 5-hour window**, Pro $200 = 10x for new subscribers (was 20x; Sep 22–29 subscribers grandfathered at 20x through 2026-10-29), Pro $500 = 25x + Astra Ultrafast. API usage is never included in any consumer plan |
| % used | **Hitting limits** on the $100 tier (Drew) |
| Reset / early reset / expiry | Monthly cycle; Plus renews Oct 25. No early reset: block/wait or ChatGPT credits if explicitly enabled for apps. Per-app weekly limits can be set for OAuth/third-party app use (counts toward plan usage) |
| Overage | WALL (+ credits if enabled). No PAYG overflow by default |
| Models | ChatGPT models in app; OpenCode routes via `openai` (built-in provider, OAuth — verified working on this sandbox 2026-10-20 via ChatGPT plan) and `openai-codex` (Drew's custom provider → local `codex-proxy` bridge on his machines, not the sandbox). Config IDs include gpt-4o, gpt-4o-mini, gpt-5-mini, gpt-5.4/5.5/5.6/6.x families. Models the Codex backend accepted for his account (2026-10-20): gpt-6.1-sol, gpt-6-astra, gpt-6-sol, gpt-reserve, gpt-6-luna, gpt-5.6-sol/terra/luna, codex-auto-review. **gpt-5.5 NOT accepted** with a ChatGPT account (400) |
| API-equivalent value | ESTIMATE-MISSING (multiples of Plus are relative, not dollar pools). Every tier is $20 per 1x of Plus — no bulk discount |
| AA tasks | ESTIMATE-MISSING (wall-capped). References: Luna max $0.07/task, Astra max $3.26/task at API rates |
| Credentials | Google account (Play) + OpenAI OAuth into OpenCode (connected 2026-10-05). No OpenAI API key in Bitwarden |

## 3. Google — Google AI Pro (Google One, 5 TB)

| Field | Value |
|---|---|
| Plan held | **Google AI Pro (5 TB)**, annual |
| Cost | **$199.99/yr charged** (no tax on receipt), via Google Play (Discover …3416). List $19.99/mo or $199.99/yr |
| Last charge / renewal | 2026-10-02 / **renews 2027-10-02** |
| Quota | 5 TB storage; **4x Free access** to Gemini; expanded Gemini / Deep Research; expanded limits in AI Studio, Antigravity, and Jules; family sharing ×5; YouTube Premium Lite; Google Home Premium Standard (~$10/mo value); Health Premium; **$10/mo Google Cloud credits** (Developer Program) |
| % used | UNKNOWN per feature (Google exposes no single meter). Verdict in master tracker: behind — the GCP credits are likely unspent |
| Reset / early reset / expiry | Billing annual. **The $10/mo GCP credits expire monthly — use-it-or-lose-it** ($120/yr of the plan's value). Per-feature caps are daily block/wait; no early reset |
| Overage | FLAT per-feature caps. Storage over 5 TB = buy more / next tier (Ultra $99.99/mo = 5x Pro; $199.99/mo = 20x Pro, 20–30 TB) |
| Models | Gemini family via `google` provider + Antigravity (`antigravity-agy`, `antigravity-ls`). Config IDs include gemma-4-26b-a4b-it, gemma-4-31b-it, lyria-3 previews, gemini-3.x/4 experimental lines (see §13) |
| API-equivalent value | ESTIMATE-MISSING for AI usage; the only hard dollar component in sources is the $10/mo expiring GCP credit |
| AA tasks | ESTIMATE-MISSING. Reference: Gemini 4 Argon = CAI 64, cost/task not in sources |
| Credentials | Google account drewrypope@gmail.com |

### 3a. Google Jules (bundled, not separate)

- Plan/cost: **not stated in sources** — no receipt; Jules is bundled
  with Google AI tiers; which tier/quota applies is an open question.
- Models: Jules coding agent (Gemini-based); runs in GitHub workflows
  in developing-today/code as `google-labs-jules[bot]`.
- Credentials (names only): Bitwarden `google.jules-api-key`,
  `google.jules-oauth-creds`. OAuth completed 2026-10-18.
- % used / reset: UNKNOWN.

### 3b. Google Antigravity

- Providers `antigravity-agy` / `antigravity-ls` are configured in
  OpenCode. Open question: covered by Google AI Pro, or a separate
  entitlement? Expanded Antigravity limits are listed as part of AI
  Pro (above), but per-workspace metering is UNKNOWN.

## 4. Muse (Meta) — consumer subscription

| Field | Value |
|---|---|
| Plan held | **Power** (confirmed via subscription status 2026-10-21; also observed 2026-10-20). Price UNVERIFIED — no receipt in Gmail or Plaid. Drew's believed "$15 tier" matches nothing current in the consumer product (see Conflicts) |
| Cost (official list, verified 2026-09-30) | Free $0; **Power $20/mo = 500M tokens/week**; **Maximum $100/mo = 3B tokens/week** (6x tokens for 5x price — the only bulk discount in the fleet) |
| Quota | Weekly token limit. Free plan's weekly number is unpublished in official help (a "100M/wk" figure circulates from a Threads post — NOT official) |
| % used | 36% observed 2026-10-20; **52% on 2026-10-21**, plus 1B additional tokens untouched (subscription-status check by the parent agent, 2026-10-21) |
| Reset / expiry | **Weekly reset Fri Oct 24, 2026, 08:48 CDT.** No early reset — the allowance refreshes on its scheduled date only. Behind pace on 2026-10-20 (~50% of window gone at 36% used) |
| Overage | WALL |
| Models | This agent runs **Muse Wonder**. Free plan runs Muse Spark. Paid Muse plans (incl. trial) run Spark + Wonder; Wonder runs the main conversation, artifacts, scheduled runs, subagents |
| API-equivalent value | ESTIMATE-MISSING — token mix (in/out/cache) per week is not in any source, so a dollar conversion would be invented. Formula when the mix is known: tokens × Meta API rates in §5 |
| AA tasks | ESTIMATE-MISSING (no tokens-per-AA-task figure in sources) |
| Credentials | Meta account drewrypope@gmail.com (password in Secure Vault). Bitwarden names: `muse.sdk-token`, `meta.api-key` (see §5) |

## 5. Meta — Model API + Muse Code / Muse CLI (separate products!)

Three different Meta things are routinely conflated in the sources;
the tracker now separates them:

### 5a. Meta Model API (PAYG)

- Key exists: Bitwarden `meta.api-key`, **verified live** 2026-10-20
  against `https://api.meta.ai/v1` (models list returned). Inference
  returns **402 Payment Required** — the API is pay-as-you-go with its
  *own* billing; the Muse subscription does **not** fund it, and no
  card is on the developer account. Drew: will "probably put some
  cash on the Muse API" eventually — a timing decision, not a no.
- Rates (developer pricing via Oct-13 launch reporting; also in
  `~/docs/muse.md`): **Muse Wonder $4.00 in / $20.00 out per M
  tokens, cached input $0.20/M.** Muse Spark ~$1.25 in / $4.25 out
  per M (third-party docs figure in tracker addendum — treat as
  less firmly sourced).
- Models listed by the API (2026-10-20): muse-wonder-1.0,
  muse-spark-1.3 / 1.2 / 1.1, muse-image-1.0, sam-3.1,
  muse-voice-transcribe-1.0.
- Overage: pure METER — 2x usage = exactly 2x cost. Spend today: $0
  because it is unfunded/blocked, not because it is free.
- Sandbox wiring: a `meta` provider exists in the deployed OpenCode
  config (via @ai-sdk/openai); through the SDK the 402 surfaces as a
  silent multi-minute stream hang, not an error.
- AA tasks: ESTIMATE-MISSING (no per-task token counts in sources).

### 5b. Muse Code (Meta's CLI) — tiers

Two source sets disagree on the bottom tier; both are recorded:

- Tracker "Verified" section (2026-10-20, sub-agent research
  spot-verified): Everyday **$5/mo**, **High $15/mo**, Power $50/mo,
  metered as **prompt allowances per 5-hour window**. Drew's $15 maps
  to **High**. CLI v1.4.11 installed + checksum-verified on the
  sandbox 2026-10-20; sign-in is Meta-account OAuth (device code), a
  *different* credential system from the Model API key. Note from the
  binary: "META_API_KEY always takes priority over the account
  login."
- `~/docs/muse.md` (Meta product doc): Muse Code offers Muse
  Wonder; **Power Usage $50/mo (20x)** and **Maximum Usage $200/mo
  (80x)** are the only route to **Muse Wonder Full**. Everyday is
  described as free there — **CONFLICT with the $5 figure above,
  unresolved.**
- Drew's position: he pays for Muse CLI *and* the Muse subscription
  *and* his Android agent has its own plan (three separate things).
  Wanted: an OpenCode provider that rides the Muse CLI's own
  subscription credentials (claude-code pattern); none exists
  built-in — plan is a `muse-proxy.mjs` in the codex-proxy pattern
  over `muse exec --json`. Pending: Drew approves a login code.
- % used / reset for the CLI plan: UNKNOWN (5-hour window metering
  only). AA tasks: ESTIMATE-MISSING.

## 6. OpenCode Go — 2 of 3 workspaces

| Field | Value |
|---|---|
| Plans held | **3 workspaces, ONE login (his GitHub)**. 2 workspaces on **OpenCode Go** (explicitly NOT Go Plus), $10/mo each = $20/mo (dictated; $5 first month per teardown). Third workspace's plan: UNKNOWN |
| Quota (official) | **Dollar-metered, stacked windows per workspace: $12 per rolling 5h, $30 per week, $60 per month** at each model's list per-token rate (~6x multiplier on the $10 price). Per-model monthly caps: premium models (Kimi K3, GPT-6 Luna, Claude Haiku 5.5, Grok) ~$15 each; standard open-weight coders up to the full $60. Promos temporarily boost some models (official table at check time: DeepSeek V4.1 Flash at $80 cap, Gemini 4 Argon 2x limited-time) |
| % used / reset | UNKNOWN per workspace — check opencode.ai console Overview. Windows are rolling (5h) / weekly / monthly; no single reset date in sources |
| Overage | At a cap: fall back to free models, or draw the Zen PAYG balance if "Extra usage" is on — EXCESS-only, never a forced upgrade. Windows are per-workspace: doubling on one workspace walls at its own caps unless routed across both |
| API-equivalent value | **~$60/mo metered pool per workspace** (~6x the $10 price); two workspaces = $120 pool |
| AA tasks (ILLUSTRATIVE, pool ÷ AInventory cost/task) | Per workspace per month, whole $60 pool on one model: Luna max ≈ **857 tasks** (but Luna is premium-capped at ~$15/mo → cap-bound ≈ **214 tasks**); DeepSeek V4.1 Flash max ≈ **222 tasks**; Opus 5.5 max ≈ **10 tasks**. Promotional caps change these |
| Models | Go key works in any agent, not just OpenCode. Providers configured in `~/git/devtoday-code/.opencode/opencode.jsonc`: claude-code, cloudflare-gateway, cloudflare-workers-ai, cloudflare-ai-gateway, google, antigravity-agy, antigravity-ls, openai, openai-codex, opendesign, meta (sandbox) |
| Credentials | OpenCode sign-in per workspace (one GitHub login). No OpenCode credential in Bitwarden yet. Receipts/billing confirmation for the two Go plans: still missing |

## 7. ClinePass ("ClineCode")

| Field | Value |
|---|---|
| Plan held | Dictated: "ClineCode" 1-year membership at ~$7–8/mo equivalent (UNVERIFIED). Official product found: **ClinePass, flat $9.99/mo** — the dictated annual figure matches no official monthly price (CONFLICT: annual promo or a different Cline product?) |
| Quota (official) | Open-weight models only (GLM, Kimi, DeepSeek, MiMo, Qwen, MiniMax, Muse Spark Contributor) — **no frontier Claude/GPT**. Quota = **2–5x what the same spend buys at API rates**, measured over **5h rolling + weekly + monthly** windows |
| % used / reset | UNKNOWN — Cline dashboard. A "Cline Bot Inc." charge was found Oct 5, 2026, amount unknown — likely this |
| Overage | WALL → Cline usage-billing credits or BYOK |
| API-equivalent value | 2–5x the $9.99 spend at API rates (official framing) ≈ $20–50/mo equivalent |
| AA tasks | ESTIMATE-MISSING (quota is a value multiple, not a stated dollar pool) |
| Credentials | Not recorded yet (no key location in Bitwarden noted) |

## 8. CommandCode (commandcode.ai)

| Field | Value |
|---|---|
| Plan held | **GO, $1/mo, ACTIVE — VERIFIED live 2026-10-20** (signed in via GitHub as dezren39). Includes **$10 in credits**. Oct 2, 2026 invoice: $1.36, Paid. (This supersedes both the dictated "annual ~$10/mo" guess and the teardown's GOAT assumption for *his* plan) |
| Official ladder (for context) | Go $1 ($10 credits), GOAT $10 → $70 credits/mo, Pro $20 → $80, Provider $15 PAYG, Max 10x $100, Max 20x $200, Team Pro $40. GOAT windows: $14/5h, $35/wk. Top-up credits are bought at model cost and **never expire**. API access on every plan except $1 Go per the teardown — note tension with his verified Go + API keys below; treat the teardown line as UNSOURCED for his account |
| % used / reset | **68% of monthly limit used; resets Nov 2, 2026.** Extra-credits balance $0.00 |
| Overage | WALL → top-up credits (EXCESS). Past a limit, paid models pause, free models keep working unless credits cover it |
| API-equivalent value | $10 credits included on the $1 plan (per his account page) |
| AA tasks (ILLUSTRATIVE, $10 credits ÷ cost/task) | Luna max ≈ **143 tasks**; DeepSeek V4.1 Flash max ≈ **37 tasks**; Opus 5.5 max ≈ **1–2 tasks** |
| Credentials | API keys on the account: `laptop-framework` (pre-existing, value not retrievable) and `muse-sandbox` (created 2026-10-20; **value was never captured** — truncated display only; recovery = paste retry or revoke + CLI login, awaiting Drew's pick). Names only here |

## 9. Open Design (open-design.ai) — formerly mis-tracked as "Design Code"

Correction (Drew, 2026-10-20): "It wasn't code design. It was open
design." All earlier "Design Code" entries (§16 of the tracker draft,
the ~$10/mo annual dictated item) are this service.

| Field | Value |
|---|---|
| Plan held | **Open Design Go — $84.00 paid 2026-10-12** (Stripe receipt, card …3571), term **Oct 12, 2026 – Oct 12, 2027**. List: $10/mo, $84 billed yearly ($7/mo effective) |
| What it is | AI design agent (open-source Apache-2.0 core) that drives a coding agent Drew already runs (Claude Code / Codex / OpenCode + 14 others) through design workflows |
| Quota | Go includes **up to $120/mo of "Design Plan" model credits PLUS API access** to those plan models. Windows (live addendum): **$10 per 5-hour window, $30 per 7-day window**, up to $120/mo. PAYG balance $1.00, auto-recharge OFF |
| Credit burn multipliers | GPT-6 Luna **4x**, Claude Haiku 5.5 **2x**, GLM-5.3 Flash-X **2x**, DeepSeek V4.1 Flash **0.5x** (**0.25x promo through Oct 27, 2026**), most others 1x |
| % used / reset | UNKNOWN %; windows are rolling 5h / 7-day as above. Annual term expires/renews Oct 12, 2027 |
| Models (12, via API) | claude-haiku-5.5, deepseek-v4-flash, deepseek-v4-flash-vision-exp, deepseek-v4-pro, deepseek-v4.1-flash, glm-5.3-flash, glm-5.3-flashx, gpt-6-luna, hy4-preview, kimi-k2.7-code, mimo-v2.6-flash, mimo-v2.6-pro |
| API-equivalent value | Up to $120/mo of plan model credits for $7/mo effective — before burn multipliers (a 4x model consumes the pool 4x as fast) |
| AA tasks | ESTIMATE-MISSING — burn multipliers are per-model and not convertible to AInventory's per-task dollar costs without a mapping no source provides |
| Credentials | API key `muse-sandbox` acquired + live-verified 2026-10-20; wired into sandbox OpenCode as provider `opendesign`. Dual-stored: Bitwarden `opendesign.api-key`, SOPS in developing-today/code PR #453. Endpoint: `https://amr-link.open-design.ai/v1` (Chat Completions works; the Responses path hangs through the AI SDK) |

## 10. DeepSeek API (PAYG — "DevSec" in early dictation, resolved)

- Identity: "DevSec" was DeepSeek all along (Drew, confirmed
  2026-10-20).
- Plan: none — pure API usage. Key existence/location: **not recorded
  anywhere yet** (campaign item #12). Spend: UNKNOWN.
- Rate card (since Aug/Sep 2026 restructure; the old flat $0.14/$0.28
  is superseded):
  - **V4.1 Flash:** peak $0.30 in / $1.20 out per M; off-peak half
    ($0.15 / $0.60); cache-hit input $0.006 peak / $0.003 off-peak
  - **V4 Pro:** peak $1.32 / $3.96 per M; off-peak $0.66 / $1.98;
    cache-hit $0.044 / $0.022
  - **Context: 1M tokens, both models** (the only context windows
    stated in the source files — see §13)
  - Peak = 01:00–04:00 and 06:00–10:00 UTC Mon–Fri (excl. Chinese
    holidays) = **20:00–23:00 and 01:00–05:00 CDT**. US daytime batch
    work is automatically off-peak (half price). Cache hits bill at
    ~2% of miss price — the single biggest lever.
- Overage: none — pure METER. 2x usage = exactly 2x cost.
- Role in doctrine: heavy off-peak volume in the steady state;
  API-priced, so safe to wire into business processes/products.
- AA tasks: cost/task is itself an AInventory row — DeepSeek V4.1
  Flash max = $0.27/task, i.e. ≈ **37 tasks per $10** of API spend at
  max effort (by definition of the reference row, not a plan pool).

## 11. Cloudflare — Workers AI / AI Gateway (usage-based)

- Plan/cost: no invoice found; usage-based on the Cloudflare account.
  (Platform subscriptions verified separately 2026-10-20: Workers
  Paid $5/mo + R2 Paid $0 base — see the Cloudflare plan docs, not
  this file.)
- Models: routes to Anthropic, Google, and OpenAI families via
  OpenCode providers `cloudflare-gateway`, `cloudflare-workers-ai`,
  `cloudflare-ai-gateway`.
- Credentials (names only): Bitwarden `cloudflare.api-token-ai-inference`
  (AI Gateway Read+Run + Workers AI, minted 2026-10-07);
  `cloudflare.api-token-day-to-day`; `cloudflare.api-token-account-wide`
  (admin copy, not for routine use).
- Quotas / % used / spend: UNKNOWN in the tracker sources. AA tasks:
  ESTIMATE-MISSING.

## 12. Accounts with no confirmed paid plan + watchlist

| Service | State in sources |
|---|---|
| Cursor | Account exists (Team mail through 2025-12-23, "Year in Code" recap). No receipt/invoice/renewal in ~13 months of Gmail. Paid or free: UNKNOWN |
| Perplexity | Account/workspace exists (Enterprise Pro upsell mail 2026-10-19). No receipt. Free or Pro: UNKNOWN |
| Midjourney | No mail found at all — confirm whether an account exists (Discord-billed subs would not email receipts here) |
| Wispr Flow | **Flow Pro (annual) — paid, confirmed:** $143.99/yr list, **$151.91/yr charged** incl. tax (Discover …3416), last charge 2026-10-09, **renews 2027-10-09**. Dictation + Notetaker. Quota: UNKNOWN in all sources. (Not a model plan, but it is his speech-to-text layer — mishearings are Wispr, not typos) |
| Xiaomi MiMo (watchlist, not purchased) | Drew's pick for the most honest pricing he has seen: pay a stated amount, get a stated number of billions of tokens. Rate card in teardown: Flash $0.14/$0.28 per M flat, Pro $0.435/$0.87, cached ~$0.003, Batch API half price again. Verify at purchase time. AA tasks: ESTIMATE-MISSING (no MiMo cost/task row in AInventory sources read) |
| Claude $200 / Gemini $15 (watchlist) | Future candidates Drew named. Gemini $15's relationship to the Google AI Pro plan in §3 is UNKNOWN |
| Amazon RxPass $5/mo, Porkbun domains | In the master tracker only for completeness — not AI plans |

---

## 13. Model details — what the files do and do not contain

Honest inventory: the source files are **plan** documents, not a model
catalog. Most model-level fields the library asked for are simply not
in them, and are marked MISSING rather than reconstructed from
general knowledge.

### What IS sourced

| Model / family | Release timing (relative) | Context | In / Out / Cache (per M tokens) | Notes |
|---|---|---|---|---|
| DeepSeek V4.1 Flash | Rate-card restructure Aug/Sep 2026 | **1M** | Peak $0.30 / $1.20, cache-hit $0.006; off-peak half, cache-hit $0.003 | AInventory: II 39, $0.27/AA task (max) |
| DeepSeek V4 Pro | Same restructure | **1M** | Peak $1.32 / $3.96, cache-hit $0.044; off-peak $0.66 / $1.98, cache-hit $0.022 | |
| Muse Wonder | Announced Oct 12, 2026; launch reporting Oct 13, 2026 | MISSING | **$4.00 / $20.00 / cached $0.20** (Meta developer pricing) | Runs paid Muse; Wonder Full is Muse-Code-only (§5b) |
| Muse Spark 1.3 | Current version Sep 2026; Spark first launched Apr 8, 2026 | MISSING | ~$1.25 / $4.25 / cache MISSING (third-party docs figure) | Runs free Muse. Older versions 1.2 / 1.1 still listed by the Meta API |
| GPT-6 Luna | In OpenCode Go premium list + Open Design (4x burn) | MISSING | MISSING (subscription-routed) | AInventory: II 38 (CSV: 37) / CAI 41, **$0.07/AA task** (max) |
| GPT-6 Astra | ChatGPT Pro $500 tier mentions "Astra Ultrafast" | MISSING | MISSING | AInventory: II 53, $3.26/AA task (max), 56 t/s |
| Claude Opus 5.5 | Routed as `anthropic/claude-opus-5.5` | MISSING | MISSING | AInventory: **II 58 (top)**, $5.98/AA task (max), 92 t/s |
| Gemini 4 Argon | OpenCode Go promo: 2x, limited-time | MISSING | MISSING | AInventory: CAI 64, no II row, cost/task MISSING |
| Xiaomi MiMo Flash / Pro | Watchlist | MISSING | $0.14 / $0.28; Pro $0.435 / $0.87; cached ~$0.003; Batch = half | Flat, no peak games |
| Embedding models (for Turso Atlas, from pro-month plan) | — | — | Local/API, model TBD in readiness sprint | bge-small 384d, bge-base 768d, bge-large/voyage-class 1024d, OpenAI ada-class 1536d — dims + bytes/vector only; no prices in sources |

### Short-term / beta / preview models visible in Drew's configs

These names appear in the OpenCode config (`opencode.jsonc`,
2026-10-20) or the Open Design model list. Listing = the name exists
in his routing; no source gives them release dates, context windows,
or prices — all MISSING per model.

- Open Design: `deepseek-v4-flash-vision-exp` (experimental),
  `hy4-preview` (preview)
- Google routing: `lyria-3-clip-preview`, `lyria-3-pro-preview`,
  `deep-research-preview-04-2026`, `deep-research-max-preview-04-2026`,
  `gemini-3-flash-preview`, `gemini-3.1-pro-preview` (+ `-customtools`),
  `gemini-3.1-flash-lite-preview`, `gemini-3.1-flash-image-preview`,
  `gemini-3.1-live-preview`-family, `gemini-2.5-computer-use-preview-10-2025`,
  assorted `-tts-preview` / `-image-preview` variants
- Promotions with expiry (the closest thing to scheduled change in
  the files): OpenCode Go's DeepSeek V4.1 Flash $80 cap + Gemini 4
  Argon 2x are "limited-time"; Open Design's DeepSeek V4.1 Flash
  0.25x burn promo runs **through Oct 27, 2026**; ChatGPT Pro $200's
  old 20x multiple is grandfathered **through Oct 29, 2026** for
  Sep 22–29 subscribers only.

### What is MISSING for essentially every model

- **Context windows** — only DeepSeek V4.1 Flash / V4 Pro (1M) are
  stated anywhere in the source files.
- **Scheduled deprecations / sunsets** — *none* found in any source
  file. The closest items are superseded *rate cards* (DeepSeek's old
  flat $0.14/$0.28; Turso-style plan reshuffles are in the Turso doc)
  and the expiring promos above. Any deprecation calendar must come
  from a later vendor-docs pass.
- **Relative release dates** — only the Muse family (Spark Apr 2026 /
  1.3 Sep 2026; Wonder Oct 2026) and the DeepSeek restructure window
  (Aug/Sep 2026) are dated in sources. GPT-6 / Claude 5.5 / Gemini 4
  generation ordering is implied by version numbers only.
- **Per-model in/out/cache pricing** for all subscription-routed
  models (Claude, GPT, Gemini via plans) — by design unpublished;
  AInventory cost-per-task is Drew's substitute metric.

## Conflicts register (carried, unresolved)

1. **ChatGPT:** Play Plus $21.09/mo vs dictated $100/mo tier — separate,
   replacement, or double-count? Plus the unexplained $16.46 charge.
2. **Claude:** annual Pro $227.88/yr vs dictated $20/mo tier — the
   teardown's judgement is "almost certainly the same plan," but the
   tracker forbids merging them until a second billing source appears.
3. **Muse $15:** Drew's believed $15 CLI tier vs consumer Power $20
   list vs Muse Code High $15 (§5b) — the $15 most plausibly maps to
   Muse Code High, *not* to the consumer subscription he also holds.
   Muse Code's bottom tier is itself conflicted ($5 vs free).
4. **ClinePass:** dictated annual ~$7–8/mo vs official $9.99/mo flat.
5. **CommandCode:** resolved for his *current* plan by the live check
   (GO $1, §8); the teardown's GOAT figures remain useful only as
   ladder context.
6. **"Design Code" = Open Design:** RESOLVED 2026-10-20 (Drew +
   receipt + live account). It is a *different* service from
   CommandCode — that earlier conflation is also resolved.

## Sources for this draft

- `~/workspace/ai-subscriptions/tracker-draft.md` — §§1–17, Conflicts,
  Limits & resets, Route-pool doctrine, Unknowns, plus the
  2026-10-20 addenda (CommandCode verified, Muse API 402, Open
  Design verified, Muse Code tiers, corrections).
- `~/workspace/ai-subscriptions/ai-plans-2026-10-20.md` — per-plan
  teardown with official-page sourcing (ladders, windows, overage).
- `~/workspace/ai-subscriptions/master-tracker.md` — one-row-per-plan
  allotment / % used / pace / overage table + totals.
- `~/MEMORY.md` — AInventory key rows; Muse Power usage/reset;
  OpenCode server + Codex OAuth verification notes (TOOLS.md via
  MEMORY cross-refs); fleet + doctrine summary.
- `~/docs/muse.md` — Muse model/plan facts (Wonder vs Spark, Wonder
  Full via Muse Code $50/$200, Meta API list prices).
- Parent-agent live check 2026-10-21: Muse subscription status
  (Power, 52% weekly, reset Oct 24 08:48 CDT, 1B additional tokens).
