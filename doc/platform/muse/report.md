# Meta "Muse" Personal AI Agent — Comprehensive Research Report
Research date: 2026-09-30 / 2026-10-01 (UTC). Mission dated Sept 30, 2026.

**Label key:** **[OFFICIAL]** Meta statements/docs (verified live where noted) · **[PRESS]** journalism/reporting · **[RUMOR]** unverified community claims.

## Summary
Meta launched **Muse**, its "personal AI agent," on **September 8, 2026** (US only, iOS/Android/muse.ai/WhatsApp; 18+), powered by the **Muse Spark** model family, running on a per-user cloud computer ("Muse Secure VM") with a separate "Sentinel" agent gating internet access. At launch Meta's stated vision: a first step toward **personal superintelligence**. Connect 2026 (Sept 23) expanded it: Mac computer use, agent email address, Realtime Voice + Realtime Avatar, AI-glasses access, Muse Charm keychain device, and a connectors/developer platform (opened ~Sept 18). Pricing: free (100M tokens/week per Zuckerberg's own post — not in official docs), Power $20/mo (500M tokens/week), Maximum $100/mo (3B/week). No paid-plugin marketplace, no developer revenue share, and no announced plan to merge Muse Code (CLI) with the consumer app. Personalization files (SOUL.md etc.) exist in the app but are not officially documented; the OpenClaw resemblance was publicly admitted as "heavily inspired" by Meta's Nat Friedman.

---

## 1. RELEASE & CREATION

**[OFFICIAL — verified live]** Meta Newsroom, "Introducing Muse: The World's First Personal AI Agent Built for Everyone," September 8, 2026 — https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/
- "secure, private personal AI agent that proactively helps with people's goals and suggests ideas"
- Runs on "Muse Secure VM, a dedicated, virtual machine (VM) that houses both the agent and a person's data"
- Usable "in the Muse app or directly in WhatsApp"
- Powered by "Muse Spark, Meta's most capable model to date, built for real-world agentic work"
- Keeps working after people close the app; returns when something changes or approval is needed; approval before email sends/purchases; full audit trail
- Payments: Link by Stripe (first AI agent covered by Link purchase protections); Shop Pay "coming soon"; 1Password support "coming soon"
- Sentinel: "A separate Sentinel agent runs on that same machine, kept apart from Muse at the system level. Nothing Muse does reaches the internet unless the Sentinel approves it"
- No visibility into passwords/payment methods; opt-out of training; conversations/VM data not shared with ad systems; tell it to "forget" specifics
- **Roadmap stated at launch:** "Later this year, Meta will introduce Muse Confidential VM, where the whole VM, including a person's data and conversations with Muse, is encrypted with a key only they hold, so not even Meta can access it"
- **Vision:** "Meta thinks personal superintelligence will be one of the most transformative technologies of a lifetime. Muse is a first step: an agent that takes on more of the work so people can focus on what matters to them."
- Availability: "rolling out in the US on iOS, Android, and muse.ai, and coming soon to AI glasses. It's free for most of what people need, with subscription plans for people who want to do more."

**[PRESS]** Launch context:
- Development led under chief AI officer Alexandr Wang (Scale AI founder; Meta bought 49% of Scale AI for $14.3B in June 2025); Muse Spark first introduced April 2026 (Axios via Wikipedia: https://en.wikipedia.org/wiki/Muse_(AI_agent))
- Reuters reported Meta delayed the original April launch to harden security after internal tests exposed sensitive data (iCloud photos), via https://www.webpronews.com/metas-muse-ai-agent-takes-on-your-to-do-list-a-personal-ai-that-acts-not-just-answers/
- TechCrunch launch coverage (Sept 8): https://techcrunch.com (via dev.to citations) — tiers reported as free/$20/$100; payment card required at signup; VP of AI products Vishal Shah told Engadget the agent "isn't really limited in what it can do" because it can write its own software
- Downloads: ~730K in first 5 days (Sensor Tower via CNBC); #1 US iPhone free app Sept 18; ~2.5M+ by Sept 21; ~5M by Sept 30 (Sensor Tower estimates via aistockwire); Evercore's Mark Mahaney expects 100M users in 6–12 months (CNBC via freshblognews)

---

## 2. RELATIONSHIP TO OTHER META OFFERINGS

**[OFFICIAL]** The consumer Muse agent and Meta AI assistant are **separate products**:
- Meta's July 24, 2026 newsroom post "Meta AI Doesn't Just Think, It Acts" (https://about.fb.com/news/category/product-news/) covers Meta AI as its own product line
- Engadget: "Muse is separate from the existing Meta AI app with its own iOS and Android apps and website" — https://www.engadget.com/2256577/how-to-get-started-with-meta-s-new-ai-agent-muse/
- dev.to 13-tests guide: "Not the same as Meta AI: The Meta AI assistant inside Instagram and Messenger is a separate product" — https://dev.to/hao_kang_82922526dfe5d934/meta-muse-in-13-real-world-tests-what-to-delegate-what-to-verify-3hlh

**[PRESS]** Shared foundation, distinct products (via https://memeburn.com/meta-has-five-products-called-muse-and-only-one-of-them-runs-your-life/):
- **Muse Spark** (April 2026): model family that "replaced Llama across Meta's consumer products" — i.e., the Meta AI app/assistant now runs on Muse Spark. Shared *model*, not the same product.
- **Muse Image** (July 2026): image-generation model in Meta AI/Instagram/WhatsApp; **Muse Video** (July 2026, preview)
- **Muse Code** (Aug 2026): terminal coding agent on Muse Spark 1.2 (macOS/Linux) — separate developer product with its own docs, plugin system, and SDK; same model family, separate product
- **Muse** (Sept 8, 2026): the consumer personal agent
- Naming confusion is real: Microsoft also has a research model called Muse (2025, Xbox/Ninja Theory), and muse.ai was previously an independent video-search company.

No official statement found that Meta will merge or retire the Meta AI app into Muse — they remain separate, though both draw on Muse Spark models.

---

## 3. ROADMAP & INTENTIONS

**[OFFICIAL]** Stated by Meta:
1. **Muse Confidential VM** — "later this year": whole VM encrypted with user-held key; not even Meta can access (launch post)
2. **AI glasses** — Muse coming to Meta AI glasses ("coming months"; wake word = say the agent's name; vision-enabled, acts on what wearer sees) — https://www.meta.com/blog/muse-personal-agent-ai-glasses/ and Connect coverage
3. **Mac computer use** — now live in Muse for Mac app (drive any app with permission; continues after user walks away) — Connect announcement (https://www.ai-generative.org/news/meta-expands-muse-with-mac-computer-use-agent-email-and-planned-glasses-access)
4. **Agent email address** — forthcoming (no date): agent gets its own email; users can forward/copy it into threads
5. **Realtime Voice + Realtime Avatar** — long conversations while Muse works in background; customizable voice by description; expressive video avatar (https://techcrunch.com/2026/09/23/everything-new-coming-to-metas-ai-agent-muse/)
6. **Muse Charm** — pocket/keychain device for the agent, "expected in coming months"/December per coverage, pricing TBD
7. **Muse for Small Business** (announced Sept 29) — business skills + connectors (Shopify, QuickBooks, Stripe, Slack, etc.); **Meta Enterprise Platform** (Sept 28, newsroom) — a Muse agent, business agent, and coding tool, led by ex-MongoDB CEO CJ Desai
8. **Business model:** Zuckerberg at Connect: "We believe that Muse will make you money… we will profit by taking a small fee from transactions" (from merchants, not users); no ads inside Muse — https://www.thestreet.com/technology/mark-zuckerberg-meta-muse-ai-revenue-potential
9. Early access program opened Sept 25: users ask Muse "Can you let the Muse team know I want to be part of the Muse early access program?" (TechCrunch: https://techcrunch.com/2026/09/25/meta-opens-early-access-program-for-new-muse-features/)
10. Zuckerberg's Connect keynote: "In the coming years, I expect that Muse is going to grow into the personal superintelligence that billions of people around the world are going to use" (TechCrunch Connect article)

---

## 4. RUMORS (all unverified unless noted)

- **[RUMOR]** Pre-launch screenshot leaks (Sept 7, @testingcatalog via https://www.techedt.com/metas-muse-ai-agent-moves-closer-to-handling-everyday-tasks): field-trip permission-slip auto-completion from email; a Goals interface (dinner reservations, marathon prep, saving for a car). Largely confirmed by the actual launch.
- **[RUMOR]** testingcatalog (Sept 26, https://www.testingcatalog.com/meta-prepares-screen-viewing-and-web-voice-calls-for-muse/): recent Muse web build contains an unreleased **screen-view tab** (watch the agent's virtual computer) and a **web voice-call UI** ("phone call" style). In-build, not shipped.
- **[RUMOR]** Community referral reports (https://www.allblogthings.com/2026/09/how-to-get-free-muse-ai-app-credits-1b-tokens-referral-code-guide.html): invite codes grant bonus tokens (reportedly ~1B tokens/referral, max 20 referrals = 21B total), stacking on weekly allowance, no expiry. Not confirmed by Meta.
- **[RUMOR]** The Information (August, pre-launch): Meta considered a premium tier as high as **$199.99/mo**. Never shipped (via https://aistockwire.com/blog/meta-launches-muse-personal-ai-agent-secure-vm-sentinel-pricing-september-2026).
- **[RUMOR]** OpenClaw resemblance: X threads (Sept 13) — Peter Steinberger (OpenClaw creator) and Ansh Nanda ("Muse is LITERALLY OpenClaw for normies") noted near-identical SOUL.md/IDENTITY.md/MEMORY.md/AGENTS.md/TOOLS.md files. Nat Friedman publicly replied: "heavily inspired as a product by OpenClaw" but built from scratch — making the *existence* of the similarity **[PRESS-verified]** while exact file-identity claims remain community-reported (https://www.webpronews.com/metas-muse-borrowed-openclaws-soul-and-doesnt-deny-it/).
- **[PRESS, reported by Reuters/404 Media]** Outbound phone-calling beta (Sept 16, Meta engineer Ryan Fox on X) routed some calls to **trained human agents** without disclosure; Meta VP acknowledged "was a miss" and rolled the feature back. Not a rumor — reported by Reuters; disclosure failure confirmed by Meta.

---

## 5. DEVELOPER ACCESS

- **[PRESS]** Muse Connectors opened to third-party developers ~Sept 18–19, 2026 via Zuckerberg's X post (@finkd): "You bring the API" — https://x.com/finkd/status/2101084678640066765; submission at **muse.ai/platform**; Meta reviews for functional/security/legal requirements; approved connectors appear in the **Muse Connector Directory** (https://runtimewire.com/article/meta-opens-muse-connectors-developers).
- **[PRESS]** The submission process is a curated funnel, not self-service: "no SDK, no API spec, no developer terms, no disclosed fees or revenue share" (https://www.zeniteq.com/ai-connectors-could-make-muse-the-new-app-store-chjm9q; https://shipwithmuse.live/blog/muse-developer-platform). >1,500 developers applied in the first week (Benzinga via TheStreet).
- **Custom connectors:** any user can have Muse build a custom connector from a public API/CLI; Meta does NOT review custom connectors (https://www.zeniteq.com/…, community templates e.g. https://github.com/AstroxNetwork/muse-connector-template).
- **Paid plug-ins / marketplace monetization:** None found. No paid-plugin marketplace, no developer revenue share, no published fee structure. Meta's own monetization is a small transaction fee from merchants (Section 3). [No official statement — gap, not a denial.]
- **Public API/SDK for the consumer agent:** None. Meta sells the underlying *models* via Meta Model API (`muse-spark-1.3` at $1.25/$4.25 per M tokens, 1M context) — https://dev.meta.ai/docs/cookbook/long-context — but the agent platform (Secure VM, Sentinel, connectors) is a product, not a platform, per community analysis (https://medium.com/@mansi.more943/metas-muse-c90cadbffa4a).
- **Muse Code (CLI) relationship:** separate developer product (Aug 2026) with its own experimental plugin system (`muse plugins marketplace add/install/approve`), SKILL.md skills, MCP support, SDK at https://github.com/meta-models/muse-code-sdk, and its own subscriptions ($5/$15/$50/mo) — [OFFICIAL: https://dev.meta.ai/docs/muse-code/configuration, https://dev.meta.ai/help/subscriptions/what-is-a-muse-code-subscription]. No announced plan to merge Muse Code with the consumer Muse app.

---

## 6. ADVANCED / POWER USE

**[OFFICIAL]** Meta's showcased capabilities: recipe reels → grocery list; bill negotiation; selling a car; training-plan adjustments; unprompted suggestions from remembered details (launch post). Zuckerberg: Civilization strategy guide for his daughter, with offered history tab (dev.to relay). Wang at Connect: Muse negotiated a cable bill down **$85/mo** via support chat; insurance savings; found unclaimed refunds (https://www.medianama.com/2026/09/223-signals-meta-connect-muse/).

**[OFFICIAL docs]** Scheduled automation is a supported primitive: one-time, location-based, and **recurring tasks** ("Every Monday at 9am…"), delivered as chat messages, viewable via "What reminders do I have?" / Upcoming tab, continuing until cancelled — https://www.meta.com/help/artificial-intelligence/1484325780075655/

**[COMMUNITY]** Concrete power-use examples:
- dev.to Ying Liao walkthrough: connectors + scheduled tasks as an automation runtime — e.g., weekday 7am digest jobs that pull Gmail/Calendar and report back (http://dev.to/ying_liao_0a481102ff971b4/automating-real-work-with-muse-connectors-scheduled-tasks-58co)
- dev.to Hao Kang: 13 real-world tests — inbox triage with read-only Gmail, travel booking, long forms (https://dev.to/hao_kang_82922526dfe5d934/meta-muse-in-13-real-world-tests-what-to-delegate-what-to-verify-3hlh)
- Maya Farah (Meta employee, launch day): her agent "Foo" **with a subagent** manages inbox, tracks projects, books travel, flags priorities — evidence subagents exist in the consumer app (https://www.linkedin.com/posts/mayafarah_meet-muse-today-we-launched-muse-metas-activity-7503354841973280768-_GdF)
- WSJ hands-on (Sept 30): Stripe-Link checkout flow, Marketplace negotiation, childcare-platform booking with manual login, read-only Gmail bill triage (https://www.wsj.com/tech/personal-tech/meta-muse-ai-agent-review-ab956101)
- aicostcheck's seven copyable workflows: inbox-to-calendar triage, travel planning/purchasing with approval gates (https://aicostcheck.com/blog/meta-launched-muse-a-us-only-personal-ai-agent-available-thr)

**Caveats:** Formal multi-agent orchestration tooling (subagent_spawn, cron, goals) is documented for **Muse Code CLI**, not verified for the consumer app. Community reports of VM terminal/browser/filesystem use are consistent with Meta's architecture (each user gets a Linux VM), but Meta has not published a power-user API. Incidents show limits: Wardle's Mac zero-day (Sept 21, patched Sept 22); The Verge's VM filesystem export (Meta: intended behavior); the Marketplace "Allow Always" address-leak incident (Sept 29–30) showing approval-grant nuance.

---

## 7. VOICE / AUDIO

- **Current status at launch:** text/chat interface only (app, web, WhatsApp) — [OFFICIAL launch post].
- **[OFFICIAL announcement, Connect Sept 23]** **Muse Realtime Voice**: hold long conversations while Muse works in the background; customizable voice by description (pace, accent). **Muse Realtime Avatar**: embodiment tech generating synchronized voice + lip movement + expression from a shared stream; can animate a reference image (photo, illustration, animal, object). Demoed by Alexandr Wang on X (Sept 24); Axios/TechCrunch reported video chat added; Meta's research team published a technical account Sept 23 (https://runtimewire.com/article/meta-muse-live-video-chat-voice). No firm GA date given — treat as announced/demoed, rolling out.
- **[RUMOR]** testingcatalog (Sept 26): web build contains a "phone call"-style web voice UI in preparation — unreleased (https://www.testingcatalog.com/meta-prepares-screen-viewing-and-web-voice-calls-for-muse/).
- **[OFFICIAL]** Glasses: ongoing voice conversations via wake word ("say its name") coming in "coming weeks" per Connect (https://transcriptdaily.com/2026/09/24/meta-platforms-unveils-muse-ai-agent-expands-smart-glasses-and-vr-push.html).
- **[PRESS]** Separately, an outbound **phone-calling beta** (Sept 16) was rolled back after Reuters/404 Media found calls routed to undisclosed human agents.

---

## 8. TIERS

**[OFFICIAL — verified live on Meta's Help Center, 2026-10-01]** https://www.meta.com/help/subscriptions/1021145227643680/
| Tier | Price | Weekly allowance |
|---|---|---|
| Free | $0 | Numeric figure NOT published; "available for free with a usage limit… upgrade to a paid subscription or wait until your free usage limit refreshes" |
| Power | $20/mo | 500M Muse tokens/week |
| Maximum | $100/mo | 3B Muse tokens/week |

- Requirements: Meta account, 18+/age of majority, supported country (US; Canada added ~Sept 18 per iPhone in Canada/press). Auto-renews monthly; subscribe via app or muse.ai.
- **[FIRST-PARTY CEO]** Free-tier number: Zuckerberg's launch-day Threads post — "free to use for up to **100M tokens per week**" (drawpie's adversarial re-check traced it directly to Zuckerberg; Bloomberg Sept 8 carried the same figure; CNET reported it). Not in launch post or help docs.
- **[PRESS]** TechCrunch/Axios (launch day): tiers named free, Power ($20), Maximum ($100). Payment card reportedly required even for free tier (TechCrunch; contested by one secondary source — conflicting).
- **[PRESS analysis]** No overage billing — hard throttle, weekly refresh; app shows a usage meter. "Muse token" unit undefined by Meta (https://tokenomy.ai/blog/muse-vs-instinct-cost-economics).
- **No future tiers announced.** Pre-launch rumor of a $199.99/mo tier (The Information, Aug) never shipped. **[RUMOR]** community referral program claims ~1B tokens per invite (max ~21B) — unverified.

---

## 9. AVATARS & IMAGE GENERATION

- **[OFFICIAL]** Meta Help Center (verified live): "Emoji and identity: Muse has a visual identity including an **avatar and tagline** that you can update" — https://www.meta.com/help/artificial-intelligence/995796179982326/
- **[PRESS]** At launch onboarding, users name the agent, pick an avatar, set communication style (techrepublic: https://www.techrepublic.com/article/news-meta-muse-ai-agent-us-launch/). The physical avatar Meta showed at Connect was named **"Jolly"** (TechCrunch: https://techcrunch.com/2026/09/23/everything-new-coming-to-metas-ai-agent-muse/).
- **[OFFICIAL announcement]** Muse Realtime Avatar (Connect): animated video presence for calls; voice-design by description. No availability date.
- **Deeper customization (custom image generation, animations):** No official plans found. Community has not documented anything beyond avatar picking + Realtime Avatar.
- Note: **Muse Image** (July 2026) is a separate image-generation model for Meta AI/Instagram/WhatsApp/advertisers — not the agent's avatar system.

---

## 10. PERSONALIZATION FILES

- **[OFFICIAL]** Meta Help Center (verified live): "The file-based memory system" is listed as one of four ways Muse builds its understanding of you (conversations, observed patterns, connector context, file-based memory). Customizable: name, personality/tone, communication style, memories, avatar+tagline. Revert via "Go back to your default personality." Also: import memory from other assistants (Settings → Data Controls → Import memory, upload .zip) — https://www.meta.com/help/artificial-intelligence/995796179982326/
- **[RUMOR/COMMUNITY]** The app ships markdown files **SOUL.md, IDENTITY.md, MEMORY.md, AGENTS.md, TOOLS.md** (spotted Sept 13 by OpenClaw creator Peter Steinberger; amplified by Ansh Nanda on X). Not officially documented by Meta (explainx.ai: https://www.explainx.ai/blog/what-is-soul-md-meta-muse-persona-file-2026).
- **[FIRST-PARTY exec, via press]** Nat Friedman: "We built Muse from scratch, but it is definitely heavily inspired as a product by OpenClaw" (https://www.webpronews.com/metas-muse-borrowed-openclaws-soul-and-doesnt-deny-it/).
- **[COMMUNITY convention, inherited from OpenClaw]** SOUL.md = agent persona, values, tone, boundaries; USER.md = facts about the user; MEMORY.md = durable self-maintained long-term facts; IDENTITY.md = name/role/avatar identity card. Users can ask Muse to edit these files in conversation.
- No official Meta documentation of how each file affects behavior — behavior details come from community reverse-engineering, not Meta docs.

---

## 11. CONVERSATION / CONTEXT MANAGEMENT

- **[OFFICIAL]** One ongoing conversation surface (like messaging a person); memory/customization carries across sessions; users can tell Muse to "forget" specifics (launch post; help center).
- **[PRESS]** Memory architecture is "layered, not scored": a self-maintained curated file of durable facts/preferences/commitments, written immediately when something important is shared; long conversations are compacted into summaries ("the summary decides what survives, and I don't get a vote after the fact" — Muse's own description in the Stark Insider experiment, https://www.starkinsider.com/2026/09/meta-muse-vs-openclaw-personal-ai-agent.html).
- **[PRESS]** Beyond chat: Artifacts (itineraries, dashboards), activity feed under the avatar, and a **Goals tab** for long-running jobs; proactive suggestions from remembered context (https://dev.to/hao_kang_82922526dfe5d934/zuckerbergs-muse-does-not-start-from-chat-it-starts-from-what-it-is-allowed-to-know-10e).
- **Side chats:** no evidence in the consumer app. `/side`, `/compact`, `/fork`, `/rewind` are **Muse Code CLI** commands (official dev docs: https://dev.meta.ai/docs/muse-code/interactive) — do not conflate with the consumer product.
- **[OFFICIAL docs]** Scheduled tasks persist and run in background independent of conversation state; delivered as messages (help article).

---

## Could not verify / Open questions
1. Exact free-tier token number in official docs — Zuckerberg's 100M/week figure never appears in Meta's help center; treat help-center language ("usage limit… refreshes") as canonical, 100M/week as CEO-stated.
2. Whether a payment card is truly required for the free tier — press reports conflict.
3. GA dates for: agent email address, Realtime Avatar/video chat, AI-glasses integration, Muse Charm pricing/markets, Confidential VM ("later this year").
4. No evidence of: paid plug-in marketplace, developer revenue share, or plans to merge Muse Code with the consumer app.
5. Reddit-specific rumor threads did not surface in the search index; X posts and forum-adjacent blogs (testingcatalog, allblogthings, dev.to) supplied the rumor corpus.
6. How each personalization file (SOUL.md etc.) precisely affects behavior — Meta has not documented this; community descriptions are the only source.

## Sources
- [OFFICIAL, verified live 2026-10-01] Meta Newsroom launch post (Sept 8, 2026): https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/
- [OFFICIAL, verified live 2026-10-01] Meta Help Center — subscriptions: https://www.meta.com/help/subscriptions/1021145227643680/
- [OFFICIAL, verified live 2026-10-01] Meta Help Center — personality & memories: https://www.meta.com/help/artificial-intelligence/995796179982326/
- [OFFICIAL, verified live 2026-10-01] Meta Help Center — reminders/scheduled tasks: https://www.meta.com/help/artificial-intelligence/1484325780075655/
- [OFFICIAL, verified live 2026-10-01] Meta blog — Muse on AI glasses: https://www.meta.com/blog/muse-personal-agent-ai-glasses/
- [OFFICIAL docs] Muse Code interactive/session docs: https://dev.meta.ai/docs/muse-code/interactive ; config: https://dev.meta.ai/docs/muse-code/configuration
- [PRESS] TechCrunch Connect 2026 Muse roundup (Sept 23): https://techcrunch.com/2026/09/23/everything-new-coming-to-metas-ai-agent-muse/
- [PRESS] TechCrunch early access program (Sept 25): https://techcrunch.com/2026/09/25/meta-opens-early-access-program-for-new-muse-features/
- [PRESS] Above Avalon launch analysis w/ press-release quotes (Sept 22): https://www.aboveavalon.com/notes/2026/9/22/meta-launches-muse-a-solution-in-search-of-a-problem
- [PRESS] memeburn — the five Muses (Sept 10): https://memeburn.com/meta-has-five-products-called-muse-and-only-one-of-them-runs-your-life/
- [PRESS] webpronews — OpenClaw/SOUL.md story (Sept 22): https://www.webpronews.com/metas-muse-borrowed-openclaws-soul-and-doesnt-deny-it/
- [PRESS] explainx.ai — SOUL.md explainer (Sept 24): https://www.explainx.ai/blog/what-is-soul-md-meta-muse-persona-file-2026
- [PRESS] starkinsider — Muse vs OpenClaw memory (Sept 14): https://www.starkinsider.com/2026/09/meta-muse-vs-openclaw-personal-ai-agent.html
- [PRESS] runtimewire — connectors opening (Sept 18): https://runtimewire.com/article/meta-opens-muse-connectors-developers
- [PRESS] zeniteq — connectors analysis: https://www.zeniteq.com/ai-connectors-could-make-muse-the-new-app-store-chjm9q
- [PRESS] shipwithmuse.live — developer platform (Sept 27): https://shipwithmuse.live/blog/muse-developer-platform
- [PRESS] tokenomy.ai — cost economics/tiers (Sept 27): https://tokenomy.ai/blog/muse-vs-instinct-cost-economics
- [PRESS] drawpie — "What is Meta Muse AI?" w/ verification notes (Sept 22): https://drawpie.com/blog/what-is-meta-muse-ai/
- [PRESS] WSJ review (Sept 29–30): https://www.wsj.com/tech/personal-tech/meta-muse-ai-agent-review-ab956101
- [PRESS] runtimewire — video chat/voice (Sept 23): https://runtimewire.com/article/meta-muse-live-video-chat-voice
- [PRESS] testingcatalog — unreleased web voice/screen view (Sept 26): https://www.testingcatalog.com/meta-prepares-screen-viewing-and-web-voice-calls-for-muse/
- [PRESS] mixed-news — glasses/email/avatar (Sept 23): https://mixed-news.com/en/meta-muse-agent-ai-glasses-connect-2026-email-address/
- [PRESS] TheStreet — Zuckerberg monetization (Sept 23): https://www.thestreet.com/technology/mark-zuckerberg-meta-muse-ai-revenue-potential
- [PRESS] Engadget getting-started guide: https://www.engadget.com/2256577/how-to-get-started-with-meta-s-new-ai-agent-muse/
- [PRESS] VentureBeat — zero-day/visibility (Sept 21): https://venturebeat.com/security/meta-patched-muses-zero-day-but-security-teams-still-lack-visibility-into-what-the-agent-can-access
- [PRESS] unite.ai — Small Business connectors (Sept 29): https://www.unite.ai/meta-adds-small-business-skills-and-app-connectors-to-muse-ai-agent/
- [COMMUNITY] dev.to — 13 real-world tests: https://dev.to/hao_kang_82922526dfe5d934/meta-muse-in-13-real-world-tests-what-to-delegate-what-to-verify-3hlh
- [COMMUNITY] dev.to — scheduled tasks + connectors: http://dev.to/ying_liao_0a481102ff971b4/automating-real-work-with-muse-connectors-scheduled-tasks-58co
- [COMMUNITY] dev.to — memory/UX deep dive: https://dev.to/hao_kang_82922526dfe5d934/zuckerbergs-muse-does-not-start-from-chat-it-starts-from-what-it-is-allowed-to-know-10e
- [COMMUNITY] allblogthings — referral credits: https://www.allblogthings.com/2026/09/how-to-get-free-muse-ai-app-credits-1b-tokens-referral-code-guide.html
- [FIRST-PARTY employee] Maya Farah LinkedIn (launch day): https://www.linkedin.com/posts/mayafarah_meet-muse-today-we-launched-muse-metas-activity-7503354841973280768-_GdF
- Wikipedia background (tertiary): https://en.wikipedia.org/wiki/Muse_(AI_agent)
