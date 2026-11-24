# How OpenAI's usage surfaces interact: Codex, Chat, Work, API, Dots, Astra, Ultrafast

Researched 2026-10-20 for Drew (ChatGPT Pro $100/mo "prolite" plan, heavy Codex user).
Load-bearing facts (pool structure, multipliers, credit rates) are from OpenAI's own
pricing page and Help Center, fetched directly. Prices are linked per item. Where a
claim rests only on third-party reporting, it is marked **[reported]**; where it is
my inference, **[inference]**.

Key official sources:

- OpenAI Codex pricing page (ChatGPT Learn): https://developers.openai.com/codex/pricing
- Help Center, "Using Codex with your ChatGPT plan": https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- Help Center, "Managing usage with GPT-6 Astra in Work and Codex": https://help-lb.openai.com/en/articles/20001516-managing-usage-with-gpt-6-astra-in-work-and-codex

---

## 1. The pools: what shares a wallet and what doesn't

There are effectively **three separate wallets**, plus two small side-meters.

### Wallet A — the shared "agentic" allowance (your subscription pool)

One allowance + one purchased-credit pool covers **all** of these, per OpenAI's
Help Center: "Codex, ChatGPT Work, ChatGPT for Excel, and Workspace Agents use a
shared allowance and credit pool when those features are available on your plan."
The pricing page states it flatly: **"ChatGPT Work and Codex share usage."**

In the pool:

- **Codex local** (CLI, IDE extension, desktop app) and **Codex Cloud** tasks —
  "Local messages and cloud chats share your plan's usage allowance" (pricing
  page). Cloud tasks can draw *more* per task than local messages.
- **ChatGPT Work** — the agent surface for longer, multi-step, deliverable-producing
  work (launched July 9, 2026). Same allowance, same credit rates, same windows as
  Codex. Work is not a separate plan or a separate meter.
- **ChatGPT for Excel / Word / PowerPoint** agentic features, and Workspace Agents.
- Task work started through **Voice** in Work/Codex on desktop (voice itself is
  metered at $0.05/min against the Codex budget for Plus/Pro credit spend).
- **Image generation inside Codex/Work** — counts against the same limits and
  burns them ~3–5× faster than a comparable text turn.
- **Dots' delegated tasks** (see §1.5) — a Dot chatting with you is free, but any
  Work/Codex task it starts draws from this pool as usual.

Metering structure:

- **Windows:** on Plus and Business Standard, a **5-hour window** (starts at your
  first Work/Codex message after the previous window ends) **plus a weekly limit**
  on top; you need allowance left in both. **Pro 100, Pro 200 and Pro 500
  currently have NO five-hour limit** in Work/Codex (Help Center, Astra usage
  article) — but Pro still has an included allowance (weekly), "no five-hour
  limit" ≠ unlimited.
- **Per-model burn rates** differ enormously. OpenAI's published Plus estimates
  for local messages per 5-hour window: GPT-6 Astra **5–45**, GPT-6.1 Sol
  **15–160**, GPT-6 Sol **15–150**, GPT-6 Luna **350–3,000**. These are
  estimates, not fixed caps — task size, context, reasoning effort, tools and
  speed mode all change consumption.
- **Credits** (purchased top-ups): when included usage runs out, Plus/Pro users
  can buy **ChatGPT credits** that feed this same pool. Credits are metered per
  million tokens at model-specific rates (Standard speed): Astra 250 in /
  25 cached / 1,250 out; GPT-6.1 Sol 50 / 2.5 / 250; GPT-6 Sol 50 / 5 / 250;
  GPT-6 Luna 2.5 / 0.25 / 12.5. Cross-checking OpenAI's credit rates against its
  API prices gives **1 credit ≈ $0.04** for Sol/Luna/Terra (third-party
  arithmetic on OpenAI's published numbers:
  https://inventivehq.com/blog/codex-subscription-options-guide).
- **Freebies inside the pool:** when signed in with a ChatGPT account, Codex
  **auto-review safety checks are free** and don't count toward limits
  (Help Center). `/status` and local shell commands don't consume quota either
  (the model reading tool output back does, via context).

Your plan specifically: Pro 100 is reported at **5× the Plus allowance**
(BGR via https://spiketoday.com/openai-dots-agents-explained-price-and-plans/);
Pro 200 dropped from 20× to **10× Plus** for new subscribers on Oct 30, 2026
(existing subscribers grandfathered through Oct 29); Pro 500 is **25× Plus**.

### Wallet B — regular ChatGPT Chat (separate)

Ordinary Chat conversations are **not** in the agentic pool. OpenAI's Help
Center is explicit: "Regular Chat usage isn't included" (in the Work/Codex
credit accounting), and "Regular Chat Voice has separate limits and doesn't
consume Codex usage." Chat has its own per-model message caps and reset
periods. Chatting, brainstorming and asking questions in regular Chat never
touches your Codex/Work allowance, and heavy Codex use never locks you out of
Chat. Images, voice and file uploads in Chat can have their own separate
limits again.

### Wallet C — the API platform (fully separate, both directions)

- API usage (platform.openai.com, your API keys, prepaid balance / pay-as-you-go)
  is billed per token at published API rates and governed by API rate limits
  (requests/tokens per minute by usage tier) — never by plan windows.
- OpenAI's ChatGPT Plus help page, as quoted in third-party coverage: **"API
  usage is separate and billed independently."** A ChatGPT plan never pays for
  API-key calls; an API balance never buys subscription usage; and **ChatGPT
  credits are not API credits** — the two credit systems do not convert
  (https://blog.laozhang.ai/en/posts/codex-api-key-vs-subscription).
- **Codex itself can run on either wallet.** Signed in with ChatGPT → Wallet A.
  Run with an `OPENAI_API_KEY` → Wallet C at API rates, and you **lose the
  cloud features**: no Codex Cloud, no GitHub code review, no Slack
  integration, and some curated plugins are unavailable. OpenAI positions the
  API-key route for CI/automation (pricing page, "API Key" section).

### Side-meter 1 — Luna Reserve (`gpt-reserve`)

The `gpt-reserve` slug on your backend is this. Per OpenAI's help article (as
summarized at https://justinmckelvey.com/blog/codex-usage-limits): selected
personal Plus and Pro accounts get **extra GPT-5.6 Luna usage after regular
usage runs out**. It has its **own meter**, does not refill the 5-hour or weekly
allowance, does not unlock stronger models, is not offered to Business/
Enterprise workspaces, and OpenAI does not publish its size. Think of it as a
small free overflow tank of the cheapest model.

### Side-meter 2 — Dots (your "dot usage")

"Dot usage" almost certainly refers to **Dots**, OpenAI's always-on agents
launched at DevDay on **September 29, 2026** — the dictation timing fits, and
Dots were the third headline of the same event that launched Pro 500 and
Ultrafast (https://www.metirai.com/blog/openai-devday-2026-dots-agents-private-intelligence-pro-500).

- A Dot is a persistent personal agent living in ChatGPT: runs on **GPT-6
  Astra**, gets its own cloud computer and browser, connects to 4,000+ apps
  via plugins, works on ongoing goals between conversations, and reports back
  via ChatGPT, Slack, Teams or voice (https://spiketoday.com/openai-dots-agents-explained-price-and-plans/).
- **Eligibility [reported, conflicting]:** Pro (100/200/500), Business Premium,
  and admin-enabled Enterprise/Edu/Healthcare; adults only; **not** in the EEA,
  UK or Switzerland at launch; gradual rollout. One report (BGR) said the $100
  Pro tier was *not* included yet while others list all Pro tiers — treat
  Pro 100 eligibility as unconfirmed until it appears in your account.
- **Cost/usage:** your **first Dot is included** in the plan; extra Dots are
  planned but **unpriced** as of this writing. Critically: **conversations
  with your Dot do not count toward your ChatGPT usage limits, but tasks it
  runs in Codex or ChatGPT Work draw from Wallet A as usual** (multiple
  sources, incl. https://findskill.ai/blog/openai-dots-always-on-agents-explained-2026/).
  So a Dot is free to talk to and potentially expensive to let work: an
  always-on Astra-driven agent delegating into Codex can drain the weekly
  agentic allowance unattended.
- Not the same as the old "Agent mode," which the Help Center says is
  discontinued in favour of Work.

---

## 2. Astra: what it is and how it counts

**GPT-6 Astra is a model — the flagship of the GPT-6 family** (GPT-6 launched
September 3, 2026). The family ladder:

| Model | Role | API price (Standard, ≤272K ctx) | Codex credit rate (in / cached / out per 1M) |
|---|---|---|---|
| GPT-6 **Astra** | Flagship; strongest reasoning + computer use | **$10 in / $50 out** (cached $1, cache-write $12.50) | 250 / 25 / 1,250 |
| GPT-6.1 **Sol** | Near-Astra quality, built for coding/computer use/office work (Sep 29, 2026) | **$2 / $10** (cached $0.10) | 50 / 2.5 / 250 |
| GPT-6 **Sol** | Previous-gen mid tier | $2 / $10 class | 50 / 5 / 250 |
| GPT-6 **Luna** | Cheap/fast volume model | ~$0.25–$1 class (credit rate 2.5/0.25/12.5) | 2.5 / 0.25 / 12.5 |

API prices: OpenAI pricing page as checked Oct 13, 2026 by
https://astrapricingcalculator.com/ and corroborated by VentureBeat-cited
figures in https://www.taskade.com/blog/codex-pricing-explained. Credit rates:
OpenAI pricing page, fetched directly (table above in §1).

How Astra is separate from the others:

- **It is the same wallet, at a much higher burn rate.** Astra costs 5× Sol's
  credit rate on input and output, and 100× Luna's output rate. In message
  terms on Plus: 5–45 local messages per 5 hours vs Luna's 350–3,000.
- **Naming split causes confusion [reported]:** in regular Chat, the
  Astra-powered option is labelled **"GPT-6 Pro"** (Pro/Business/Enterprise);
  in Work and Codex it is named **GPT-6 Astra** directly. Plus gets limited
  Astra in Work/Codex only. A higher-capability variant, **GPT-6 Astra Pro**,
  is reported for Pro/Business/Enterprise plans
  (https://www.sgaitools.com/articles/gpt-6-astra-explained.html — single
  source, treat as reported).
- **Capability pitch:** computer use is the headline — Astra operates software
  (cursor, clicks, typing) and scored 72.6% on OSWorld 2.0 vs 65.7% for
  GPT-5.6 Sol, ~47% faster per task [OpenAI numbers via sgaitools].
- **Safety overhead [reported]:** Astra is the first model rated **Critical**
  for cybersecurity capability under OpenAI's Preparedness Framework, so extra
  safeguard checks run alongside it and can **slow, pause or stop tasks —
  including benign ones** — with a review prompt in Chat/Codex
  (https://www.notebookcheck.net/OpenAI-Astra-new-safeguards-can-stop-your-ChatGPT-task-mid-run.1387259.0.html).
- **Behaviour reports:** users describe Astra (and Sol) as more autonomous and
  token-hungry than GPT-5.5 — longer self-directed runs, ~2.25× the tokens per
  session in one long-time user's logs
  (https://www.vincentschmalbach.com/gpt-5-5-retirement-codex/).
- **Field burn-rate report:** a five-person team sharing a Pro 20× login ran
  Astra Extra High for three workdays and consumed **92% of the weekly
  allowance**; the same team on Sol Extra High normally finished the week with
  30–40% left. Quality felt slightly better with fewer fix passes, not enough
  to justify the burn; they switched back to Sol
  (https://trlabs.my/resources/codex-astra-extra-high-allowance-three-days).
  Single anecdote, self-described as uncontrolled — but directionally
  consistent with the published rate card.

Bottom line on Astra: not a mode, not a separate pool — the flagship model,
metered ~5× Sol / ~100× Luna (output) inside the same Wallet A (or at
$10/$50 per 1M on Wallet C).

---

## 3. Ultrafast: what it is, what it costs, what users say

### What it is

**Ultrafast is a speed (service) tier, not a model and not a plan add-on.**
Same weights, served faster — OpenAI claims **up to 8× faster token generation
in Codex (~300 tokens/second)** and **up to 6× in the API**. It launched at
DevDay (Sep 29, 2026) for **GPT-6 Astra**; **GPT-6.1 Sol Ultrafast followed on
October 8, 2026** (https://rohitai.com/blog/openai-gpt-6-1-sol-ultrafast-api-codex-guide).
It sits on top of a tier ladder: Batch/Flex (0.5× price) → Standard (1×) →
**Fast** (2×, formerly "Priority processing", renamed July 30, 2026) →
**Ultrafast** (6×) (https://www.orcarouter.ai/blog/gpt-6-1-sol-ultrafast-rollout).

**[Reported, unconfirmed for Astra]** the speed is widely attributed to
Cerebras hardware (OpenAI signed a $10B+ Cerebras deal in Jan 2026; Cerebras
confirmed serving a GPT-5.6 Sol Ultrafast preview at 750 tok/s in August — a
different, unpriced preview). OpenAI's own materials don't name a vendor for
the Astra tier (https://vallettasoftware.com/blog/post/chatgpt-pro-max).

### What it costs

In the API — **6× the model's Standard price** (OpenAI pricing page figures,
corroborated by multiple trackers):

| Model | Standard (in / out per 1M) | Ultrafast (in / cached / cache-write / out per 1M) |
|---|---|---|
| GPT-6 Astra | $10 / $50 | **$60 / $6 / $75 / $300** |
| GPT-6.1 Sol | $2 / $10 | **$12 / $0.60 / $15 / $60** |

- Long-context (>272K input tokens) Astra Ultrafast: $120 / $12 / $150 / $450.
- API access: available to all API users at **low rate limits** (Astra:
  500K TPM at tiers 1–3, 1M at tier 4, 5M at tier 5), US/global processing
  only; higher limits via an account team
  (https://mixed-news.com/en/gpt-6-astra-ultrafast-tier-300-million-output-tokens/).
- OpenAI recommends persistent WebSocket connections for agentic use, warning
  that plain HTTP overhead can eat the latency gains.

Inside a subscription (OpenAI pricing page, fetched directly):

- Ultrafast burns **included subscription usage at 8×** the Standard rate for
  the same model, and **purchased credits / Enterprise PAYG at 6×**.
- **In ChatGPT Work/Codex it is available ONLY on Pro 500 ($500/mo) and
  eligible Enterprise/Edu plans.** Pro 100 and Pro 200 cannot unlock it — not
  even by buying credits (Help Center / pricing page feature matrix).
- For comparison, **Fast mode** — which your Pro 100 *does* include — burns
  included usage at **2.5×** and credits at **2×**, for roughly 2× speed.

### What would make it worth it

The honest arithmetic (https://www.orcarouter.ai/blog/gpt-6-1-sol-ultrafast-rollout):
you pay 6× per token to remove up to 7/8 of the *generation* time. That pays
only when **someone or something is blocked waiting on the tokens**:

- Interactive pair-programming / tight steer-fix loops where the human is the
  bottleneck and latency changes whether the tool feels usable.
- Long sequential agent chains where each step waits on the previous output.
- Live demos, customer-facing latency-sensitive features.

It does **not** pay for: batch/background jobs, overnight runs, CI, bulk
refactors nobody watches — there the half-price Batch/Flex lanes are the
correct answer, and note that 8× *token generation* ≠ 8× *task completion*:
tool calls, test runs, file I/O and approvals don't speed up
(https://aistify.com/openai-devday-2026-biggest-announcements/). On a
subscription it is doubly expensive: the 8× allowance burn means a Pro 500's
25×-Plus allowance holds only ~3×-Plus worth of Astra Ultrafast work
**[inference from published multipliers]**.

### User reports (marketing separated from experience)

Hands-on reports are **thin** — Pro 500 is three weeks old and $500/mo, so
the reviewer pool is small. What exists:

- **Skeptical on delivered speed:** a forum user measuring Astra Ultrafast in
  real use reported **~70 tok/s** rather than the 300 headline, with others
  noting the figure depends heavily on context length and that OpenAI's *Fast*
  mode often benchmarks slower than competitors' standard modes
  (https://vibingtalk.com/index.php?threads/ultrafast-mode-for-astra-6-is-70tps-for-300-m-output.1586591/).
  Single-thread anecdote, unverified methodology — but it's the only
  measured-user number I found, and it's ~4× under the marketing ceiling
  ("up to" is doing work in OpenAI's claim).
- **Allowance-shock reports:** a widely shared post from a heavy Codex user
  claimed they **emptied a full week of Pro 500 allowance in under 30 minutes**
  of Astra Ultrafast. The blog relaying it could not verify the account but
  noted the 8× burn rate makes it arithmetically plausible
  (https://vallettasoftware.com/blog/post/chatgpt-pro-max). Treat as
  unverified, directionally consistent with the rate card.
- **Stage marketing:** Altman at DevDay: "an incredible 300 tokens per
  second… you know what, it's worth it" — a sales line, delivered to laughter,
  alongside a rehearsed side-by-side demo
  (https://dev.to/axrisi/openai-devday-2026-every-announcement-with-prices-and-availability-1mbh).
- **Power-user sentiment:** Harrison Kinsley (Sentdex) on X: "the API prices
  are the real prices and the gap will continue to close" — i.e., expect
  subscription allowances to keep converging toward API economics (via
  vallettasoftware, above).
- **Analyst consensus** (TechRepublic, aicatchup, orcarouter): worth it only
  for interactive Astra/Sol work where a human waits; a poor buy for
  throughput. No independent benchmark of end-to-end task speedup exists yet.

---

## 4. Practical synthesis for a Pro 100 ($100) heavy Codex user

**Burn order inside Wallet A (your subscription):**

1. **Luna first** for volume work: scaffolding, boilerplate, simple
   automations, test generation, bulk edits. Its output credit rate is 1/100th
   of Astra's; on Plus-equivalent metering it's 350–3,000 messages per window.
   When the plan allowance is exhausted, the **Luna Reserve** (`gpt-reserve`)
   gives you a further small tank of it for free.
2. **GPT-6.1 Sol as the default driver.** OpenAI positions it as near-Astra on
   coding/computer-use at **1/5 the price** ($2/$10 API; 50/250 credits), and
   independent cost-per-task arithmetic from DevDay coverage put a 50-step
   agent session at ~$1.37 on 6.1 Sol vs ~$8.20 on Astra (API list). Field
   reports back Sol as the sustainable all-day choice.
3. **Astra sparingly:** hardest problems, architecture, computer-use/spatial/
   design work, final review of Sol's output. Expect ~5× Sol's burn and budget
   for its safeguard system occasionally pausing tasks for review.
4. **Work vs Codex is a false choice** — same wallet. Don't run research in
   Work imagining it's free while Codex is metered; both drain the one
   allowance. Do thinking/discussion in **regular Chat** instead — it's a
   genuinely separate meter.
5. **Fast mode is your speed lever** (2.5× included burn, ~2× speed): flip it
   on for interactive sessions where you're actively waiting; leave batch
   work on Standard.

**Wallet C (API) uses:** CI, scripted/automated runs, and overflow when the
subscription is exhausted — remembering API-key Codex loses Cloud/GitHub
review/Slack. For unattended bulk work use Batch/Flex at 0.5× Standard.

**Ultrafast verdict for you specifically:**

- **You cannot buy it on Pro 100 at any price** — in Work/Codex it's
  Pro 500/Enterprise only, and credits don't unlock it. The only door open to
  you today is the API at 6× ($60/$300 Astra; $12/$60 Sol).
- **Pro 500 (+$400/mo) is not defensible** on current evidence: it buys 25×
  Plus allowance plus a speed tier whose real-world delivery is unverified
  (~70 tok/s in the only user measurement found vs 300 claimed) and which
  burns that allowance at 8×. It becomes defensible only if you are
  *routinely* exhausting Pro 100's allowance AND your bottleneck is your own
  waiting time on interactive frontier-model work.
- **API Ultrafast has a narrow defensible case:** latency-critical interactive
  loops (e.g. a live coding session or demo on Sol Ultrafast at $12/$60) where
  minutes saved are worth 6× token cost. For everything else, Standard — or
  Batch at half price — wins.

**Dots, if they appear on your Pro 100:** free to talk to, but supervise what
they delegate — every Work/Codex task a Dot starts bills Wallet A at Astra
rates by default.

---

## Sources

Official (fetched directly):

- Codex pricing / ChatGPT Learn — https://developers.openai.com/codex/pricing
- Help Center: Using Codex with your ChatGPT plan — https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
- Help Center: Managing usage with GPT-6 Astra in Work and Codex — https://help-lb.openai.com/en/articles/20001516-managing-usage-with-gpt-6-astra-in-work-and-codex

Third-party reporting and user reports:

- Codex usage limits explainer (Luna Reserve, windows) — https://justinmckelvey.com/blog/codex-usage-limits
- API key vs subscription comparison — https://blog.laozhang.ai/en/posts/codex-api-key-vs-subscription
- Credit-value arithmetic — https://inventivehq.com/blog/codex-subscription-options-guide
- Plan/allowance tracker — https://www.morphllm.com/codex-pricing
- Usage limits by plan — https://muvon.io/blog/codex-usage-limits
- DevDay pricing roundup (Taskade) — https://www.taskade.com/blog/codex-pricing-explained
- DevDay analysis (AI Signal) — https://ai-signal.sisqo.dev/articles/openai-devday-2026-codex-cloud-agents-api
- Ultrafast vs Standard rates — https://nerdschalk.com/gpt-6-astra-ultrafast-vs-standard-speed-and-usage-rates/
- DevDay notes — https://andrewlauchner.com/notes/openai-devday-2026
- DevDay announcements with prices — https://dev.to/axrisi/openai-devday-2026-every-announcement-with-prices-and-availability-1mbh
- Sol Ultrafast launch — https://medium.com/@aiindustrytoday/openai-brings-near-instant-steering-to-codex-adds-gpt-6-1-sol-ultrafast-mode-63383d76360f
- Dots / Pro 500 / Ultrafast — https://www.metirai.com/blog/openai-devday-2026-dots-agents-private-intelligence-pro-500
- Dots explained (SpikeToday) — https://spiketoday.com/openai-dots-agents-explained-price-and-plans/
- Dots explained (FindSkill) — https://findskill.ai/blog/openai-dots-always-on-agents-explained-2026/
- Dots guide — https://dev.to/theglitchfixer/openai-dots-how-to-set-up-use-chatgpts-always-on-ai-agent-2026-2g4p
- What is ChatGPT Work — https://madrobot.blog/2026/10/03/what-is-chatgpt-work-vs-chat-codex-claude-cowork-cost-limits/
- ChatGPT Plus limits & credits — https://blog.laozhang.ai/en/posts/chatgpt-plus-usage-limits
- Plus Codex limits & credits — https://blog.laozhang.ai/en/posts/chatgpt-plus-codex-weekly-limit-credits
- Chat/Codex pool analysis — https://www.ai.joaoqueiros.com/blog/chatgpt-codex-merge-devday-interface-usage-limits
- Usage dashboard walkthrough — https://www.jdhodges.com/blog/how-to-check-codex-usage-chatgpt-plus/
- Sol Ultrafast pricing analysis — https://www.orcarouter.ai/blog/gpt-6-1-sol-ultrafast-rollout
- Astra Ultrafast API tier details — https://mixed-news.com/en/gpt-6-astra-ultrafast-tier-300-million-output-tokens/
- Astra pricing calculator (checked Oct 13, 2026) — https://astrapricingcalculator.com/
- Sol Ultrafast worth-it analysis — https://myaifactbook.com/gpt-6-1-sol-ultrafast-pricing-guide/
- Sol Ultrafast guide — https://rohitai.com/blog/openai-gpt-6-1-sol-ultrafast-api-codex-guide
- Pro 500 vs Pro 200 (AI Catchup) — https://aicatchup.com/news/chatgpt-pro-500-astra-ultrafast
- Pro 500 launch coverage — https://techbytes.app/posts/openai-pro-500-plan-chatgpt-pro-limits-cut/
- Pro 500 / allowance analysis — https://vallettasoftware.com/blog/post/chatgpt-pro-max
- Pro 500 vs 200 (TechRepublic) — https://www.techrepublic.com/article/news-chatgpt-pro-500-vs-200/
- DevDay biggest announcements — https://aistify.com/openai-devday-2026-biggest-announcements/
- Astra explainer — https://www.sgaitools.com/articles/gpt-6-astra-explained.html
- Astra access by surface — https://trendquotient.com/technology/ai-productivity/gpt-6-astra-chatgpt-access/
- Astra safeguards (Notebookcheck) — https://www.notebookcheck.net/OpenAI-Astra-new-safeguards-can-stop-your-ChatGPT-task-mid-run.1387259.0.html
- GPT-5.5 retirement / model behaviour field notes — https://www.vincentschmalbach.com/gpt-5-5-retirement-codex/
- Astra Extra High allowance field report — https://trlabs.my/resources/codex-astra-extra-high-allowance-three-days
- Ultrafast user-speed thread — https://vibingtalk.com/index.php?threads/ultrafast-mode-for-astra-6-is-70tps-for-300-m-output.1586591/
