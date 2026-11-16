# Source 05 — Tiers/pricing (PRESS + OFFICIAL help page, verified live 2026-10-01)
- Meta Help Center "About Muse subscriptions": https://www.meta.com/help/subscriptions/1021145227643680/ (read live)
  - Free: "available for free with a usage limit. If you reach your free usage limit and want more usage, you can upgrade to a paid subscription or wait until your free usage limit refreshes." NO numeric figure on the page.
  - Power: $20/month → 500M Muse tokens/week
  - Maximum: $100/month → 3B Muse tokens/week
  - Subscribe via Meta account, mobile app or muse.ai; 18+; supported country; auto-renews monthly
- 100M tokens/week free figure: NOT in launch post, NOT in help center. Source: Mark Zuckerberg's own Threads post on launch day (per drawpie adversarial re-check, 2026-09-23; also Bloomberg Sept 8 via Techmeme). CNET also reported. [PRESS, first-party CEO statement]
- TechCrunch/Axios launch-day reporting: three tiers — free, Power $20/mo, Maximum $100/mo. [PRESS]
- The Information (August 2026) had reported Meta considering up to $199.99/mo premium tier — discussed, not shipped. [PRESS rumor pre-launch]
- Usage meter in app warns before free allowance runs out; no overage billing — hard throttle (tokenomy.ai). [PRESS analysis]
- Referral program (allblogthings, 2026-09-30): invite codes grant bonus tokens (reportedly 1B tokens per referral); user's own code + up to 20 referrals = up to 21B tokens total; stacks on weekly allowance; doesn't expire on upgrade/downgrade. Community-reported, not official. [COMMUNITY]
- Payment card required at signup even for free tier (TechCrunch, shattered.io; contested by layer3labs which says no mandatory payment for free baseline — sources disagree). [PRESS, conflicting]
- No future tiers announced.

# Source 06 — Developer access (PRESS + community, 2026-09-30)
- Muse connector platform opened ~Sept 18-19, 2026: Zuckerberg X post (@finkd, status 2101084678640066765): "You bring the API." Developers submit at muse.ai/platform; Meta reviews for functional/security/legal; listed in Muse Connector Directory. [PRESS]
- Flow: describe product → Meta tests + feedback → approved connectors listed. No published SDK, API spec, review timeline, fees, or revenue share (zeniteq, shipwithmuse, the-ai-corner). [PRESS analysis]
- Early connectors: Gmail, Google Calendar, Spotify, OpenTable at launch (Meta-provided); Notion, Granola first third-party; Connect added Walmart, Best Buy, Gap, Sephora, Ulta, Wayfair, Expedia, Instacart, GitHub, Notion, PayPal, Shop Pay. Muse for Small Business (Sept 29): Asana, Box, Canva, Dropbox, Figma, Granola, HighLevel, QuickBooks, Klaviyo, Lovable, Slack, Stripe, Zoom + FB/IG business accounts. [OFFICIAL Meta newsroom + PRESS]
- Custom connectors: any user can have Muse build a custom connector from a public API/CLI; Meta does NOT review custom connectors (venturebeat, shipwithmuse). [PRESS]
- >1,500 developers applied within first week (Benzinga via TheStreet). [PRESS]
- No paid-plugin marketplace, no developer revenue share published. Meta's own monetization: small fee from transactions (merchants), per Zuckerberg at Connect Sept 23. [PRESS]
- Muse Code (separate CLI product): experimental plugin system (muse plugins marketplace add/install/approve), skills (SKILL.md), MCP, SDK at github.com/meta-models/muse-code-sdk; own subscriptions $5/$15/$50. [OFFICIAL dev.meta.ai docs]
- No announced plan to merge Muse Code with consumer app. No public API/SDK for the consumer agent platform itself — only Meta Model API for the underlying Muse Spark models (muse-spark-1.3 at $1.25/$4.25 per M tokens, 1M context). [PRESS/OFFICIAL dev.meta.ai]

# Source 07 — Personalization files / OpenClaw story (PRESS + community, 2026-09-30)
- Sept 13: OpenClaw creator Peter Steinberger (@steipete) found Muse ships markdown files SOUL.md, IDENTITY.md, MEMORY.md, AGENTS.md, TOOLS.md — similar to OpenClaw. Ansh Nanda X post: "Muse is LITERALLY OpenClaw for normies." [RUMOR/community, X]
- Nat Friedman (head of product, Meta Superintelligence Labs) responded on X: "We built Muse from scratch, but it is definitely heavily inspired as a product by OpenClaw" — tried OpenClaw in January, bought hundreds of Mac minis for team; called creator Peter Steinberger "a genius." [PRESS reporting of first-party exec statement — webpronews, TechCrunch]
- explainx.ai (2026-09-24): SOUL.md "configures the AI companion's persona/identity"; NOT officially documented by Meta beyond existence. [PRESS analysis]
- Official help article (meta.com/help): customizes name/personality/tone/communication style/memories/avatar+tagline; "file-based memory system" listed as one of the four ways Muse builds understanding of the user. No SOUL.md/USER.md names published by Meta. [OFFICIAL]
- Community convention (from OpenClaw heritage): SOUL.md = personality/values/tone/boundaries; USER.md = facts about the user; MEMORY.md = durable long-term facts, self-maintained; IDENTITY.md = agent identity card (name, role, avatar). Users can ask Muse to edit these. [COMMUNITY]

# Source 08 — Voice/audio (OFFICIAL announcements + PRESS)
- At launch: text/chat interface (app, web, WhatsApp). No voice at launch per launch post. [OFFICIAL]
- Connect Sept 23, 2026: Meta announced real-time voice conversations — "Muse Realtime Voice" — with customizable voice (describe desired pace/accent); users can "hold a long conversation while Muse works in the background" (meta.com glasses blog; TechCrunch; mixed-news). [OFFICIAL announcement]
- Muse Realtime Avatar (Sept 23): embodiment tech turning Realtime Voice into animated video avatars; model generates synchronized speech/lip movement/expression; can animate a reference image (photo, illustration, animal, object); demoed by Alexandr Wang on X Sept 24; Axios/TechCrunch reported live video chat added; Meta research team published technical account Sept 23 (runtimewire). [PRESS, announced/demoed — no firm GA date per ai-generative]
- testingcatalog (Sept 26): Muse web build shows web-based voice calls ("phone call" UI) and persistent screen-view tab in preparation — unreleased, found in build. [PRESS/analysis, unreleased]
- Glasses: ongoing voice conversations with Muse coming "in the coming weeks" via Meta Glasses wake-word ("say its name"), per Connect. [OFFICIAL announcement]
- Separately: outbound phone-calling beta (Sept 16, Meta principal engineer Ryan Fox X post) — routed calls to trained HUMAN agents without disclosure; Reuters + 404 Media reported Sept 22; Meta VP acknowledged "was a miss," rolled back the feature. [PRESS, verified by Reuters]

# Source 09 — Avatars & image generation (OFFICIAL + PRESS)
- At launch: users name the agent, pick an avatar, set communication style during onboarding (techrepublic; Business Today via tech-insider). [PRESS]
- Help center (official): "Emoji and identity: Muse has a visual identity including an avatar and tagline that you can update." [OFFICIAL]
- The physical avatar shown at Connect was named "Jolly" by Zuckerberg (TechCrunch; Bloomberg via freshblognews). [PRESS]
- Realtime Avatar model: expressive video presence for calls (see voice notes). No firm availability date. [OFFICIAL announcement]
- Custom image generation for avatars / animations: no official statement found. No plans announced beyond Realtime Avatar.
- Separate: "Muse Image" (July 2026) is Meta's image generation model used in Meta AI/Instagram/WhatsApp/Advantage+ creative — NOT the personal agent's avatar system. [OFFICIAL via adsuploader newsroom summary; PRESS]

# Source 10 — Context management (OFFICIAL + PRESS)
- Official: one ongoing conversation; memory carries across sessions (winek.ai: "Muse's memory is structural and ambient"); help center: customizations "carry over between conversations"; user can say "forget" to remove specifics (launch post). [OFFICIAL/PRESS]
- Stark Insider (Sept 14) experiment: Muse's own description — layered memory; "curated file I maintain myself: durable facts, preferences, commitments... write it down immediately, before I even reply"; long conversations compacted into summaries ("the summary decides what survives"). [PRESS — reported agent self-description]
- Goals tab for long-running jobs; Artifacts (itineraries, dashboards) as structured outputs separate from chat (dev.to). [PRESS]
- "Side chats": no evidence in consumer app. In Muse Code (CLI) there is /side, /compact, /fork, /rewind — developer docs (dev.meta.ai). Do NOT attribute to consumer app. [OFFICIAL docs, different product]

# Source 11 — Power use examples (community + official exec anecdotes)
- Maya Farah (Meta, LinkedIn launch day): her agent "Foo" (with a subagent) manages inbox, tracks projects, books travel, flags what matters. [FIRST-PARTY employee]
- Zuckerberg anecdote (dev.to): agent offered Civilization strategy guide to his daughter, offered to add a history tab. [PRESS relaying exec anecdote]
- Wang at Connect: Muse negotiated cable bill down $85/mo via online support chat; insurance savings; found unclaimed refunds (medianama). [PRESS relaying exec]
- Meta launch examples: recipe reel → grocery list; selling a car; lowering a bill; training plan adjustments. [OFFICIAL]
- dev.to Hao Kang: "13 real-world tests" — inbox triage, travel, forms; notes cost/tiers. [COMMUNITY]
- dev.to Ying Liao: scheduled tasks + connectors walkthrough — morning briefs, digests as cron-like automation. [COMMUNITY]
- WSJ review (Sept 29-30): shopping checkout with Link, gym referral lookup, Marketplace, childcare booking platform, Gmail read-only triage. [PRESS]
- aicostcheck: seven copyable workflows (inbox-to-calendar triage, travel planning/purchasing, etc.). [PRESS analysis]
- Multi-agent: consumer app has subagents (Farah post); formal multi-agent orchestration evidence is from Muse Code CLI tooling (subagent_spawn), not consumer. [MIXED]

# Source 12 — Rumors / unreleased (RUMOR, label unverified)
- Pre-launch screenshots (Sept 7, @testingcatalog via techedt): field-trip permission slip auto-complete; goals interface (dinner reservations, marathon prep, saving for car). [RUMOR, pre-launch — largely confirmed at launch]
- testingcatalog (Sept 26): web build contains screen-view tab + web voice calls in preparation. [RUMOR, in-build findings]
- Referral/early-access: invite codes, 1B-token referral grants (allblogthings); early access program — join by asking Muse ("Can you let the Muse team know I want to be part of the Muse early access program?") announced via X Sept 25 (TechCrunch). Early access is OFFICIAL; referral token figures are community-reported.
- OpenAI "dots" launched Sept 29 at DevDay as direct Muse competitor (Axios via ramaonhealthcare). [PRESS — competitor context]
- The Information (Aug): possible $199.99/mo tier discussed pre-launch, never shipped. [PRESS rumor]
- Muse Charm keychain device: announced at Connect; ships December (markets/pricing unconfirmed — notebookcheck). [OFFICIAL announcement, details TBD]
- Meta Enterprise Platform announced Sept 28 (newsroom): includes Muse agent + business agent + coding tool; CJ Desai (ex-MongoDB CEO) to run it. [OFFICIAL]
- Security incidents: Wardle Mac zero-day (Sept 21, patched Sept 22); Verge VM filesystem export (Sept 24, Meta: intended behavior); YouTuber Robb Marketplace address leak via "Allow Always" grant (Sept 29-30, Verge + PCMag). [PRESS — these are incidents, not roadmap rumors]
