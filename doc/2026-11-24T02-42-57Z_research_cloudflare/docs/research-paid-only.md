# Cloudflare Paid-Only & Enterprise Features — Research for Reference Document

**Date:** 2026-10-20  
**Scope:** Research task 3 of 3 — what is *not* free, split by affordability for a solo developer already on Workers Paid ($5/mo).  
**Budget lens used in this file:** The user wants to stay at the $5 already spent, and not add another $5 / $10 / $20 lightly. Section A is therefore the $5–$50/mo “could buy if it solves a real problem” set. Section B is custom-contract / enterprise-only and not realistic on a personal budget.  
**Method:** Official Cloudflare sources first — `cloudflare.com/plans`, `cloudflare.com/plans/application-services/`, and per-product pricing pages on `developers.cloudflare.com` — checked 2026-10-20. Where Cloudflare does not publish a price, this file says **“custom pricing, not published”** and flags it **UNVERIFIED** for any third-party estimate. Third-party estimates are never presented as Cloudflare list price.

> **Important change found in Oct 2026 research:** Logpush was historically Enterprise-only. As of the 2 Oct 2026 Observability launch it is now self-serve on Free / Pro / Business with usage pricing. See Section A.14. Anything in older blog posts calling Logpush “Enterprise-only” is now out of date for the base export feature, though Enterprise Log Share / long-retention contracts remain enterprise. Sources: https://blog.cloudflare.com/one-observability-platform/ and https://developers.cloudflare.com/logs/logpush/pricing/

---

## A) AFFORDABLE PAID-ONLY / ADD-ON FEATURES — $5–$50/mo range for a solo dev

### A.0 Baseline: Zone plans that gate most add-ons

#### Pro plan — $20/mo billed annually, $25/mo billed monthly, per zone
- **What it is:** First paid zone plan. Adds managed WAF rulesets + OWASP, 20 custom WAF rules (155 Cloudflare Rules total in current plans table), Polish lossless image optimisation, Mirage / mobile optimisation, APO included, DDoS alerts, ticket support.
- **Exact price:** $20/mo annual / $25/mo monthly, per domain/zone — not per account.
- **Solo verdict:** **Borderline — the single most plausible zone upgrade.** It is 4× the current Workers Paid spend for one zone. Worth it only if that zone needs WAF managed rules, Polish, or APO included. For a pure Workers / R2 / D1 API with no origin website, it buys almost nothing.
- **Source:** https://www.cloudflare.com/plans/

#### Business plan — $200/mo billed annually, $250/mo billed monthly, per zone — OUT OF THE $5–$50 RANGE
- **What it is:** Adds custom SSL upload, 100% uptime SLA (1× credits), 100 custom WAF rules (310 Rules total), Cache Bypass on Cookie, PCI / SOC 2 positioning, Super Bot Fight Mode + Bot Analytics, Waiting Room (1 room, see A.17), chat support.
- **Exact price:** $200 annual / $250 monthly, per zone.
- **Solo verdict:** **Wouldn’t buy on a personal budget** — listed here only because several features below (Waiting Room, Advanced Rate Limiting inclusion, custom certs) are gated to Business, so the reader understands the gate is $200, not $20. 40× the Workers Paid base.
- **Source:** https://www.cloudflare.com/plans/

#### Workers Paid overages — the plan already owned
- **What it is:** Not a new purchase. $5/mo minimum includes 10M requests/mo + 30M CPU-ms/mo. This line covers what happens *past* included usage, which is the realistic “extra Workers Paid usage” the brief asked about.
- **Exact price:** +$0.30 per additional million requests, +$0.02 per additional million CPU-ms. Workers Logs: 20M events included, +$0.60 / million. Workers KV (Paid): 10M reads included +$0.50/M, 1M writes included +$5.00/M, 1 GB stored included +$0.50/GB-mo. Queues: 1M ops included +$0.40/M. Durable Objects and D1 detailed in A.19–A.20.
- **Solo verdict:** **Keep — this is the best-value paid line in the whole catalog.** Example in official docs: 15M requests at 7 ms avg = $8.00 total. 100M requests at 7 ms = $45.40 total. Costs only bite with KV writes in a hot path ($5/M) or always-on Durable Objects.
- **Source:** https://developers.cloudflare.com/workers/platform/pricing/

---

### A.1 Smart Shield + Argo Smart Routing (incl. Tiered Cache)
- **What it is:** Uses Cloudflare’s real-time network congestion data to route origin fetches / dynamic requests around slow paths. Bundle on the plans page is sold as “Smart Shield + Argo” and explicitly *includes Smart Routing and Tiered Caching*.
- **Exact price:** **Starting at $5/mo per zone** on the official plans page. Usage beyond the first GB is metered — Cloudflare’s long-published rate is **$0.10/GB of Argo-routed traffic**. The $0.10/GB figure is confirmed in Cloudflare billing / launch material summarised in search, but the plans page itself only shows the $5 starting price, so budget as $5 + $0.10/GB and verify in dashboard before enabling. At 100 GB = ~$15; at 1 TB = ~$105 — it stops being “affordable” fast.
- **Solo verdict:** **Wouldn’t buy for a cached static / Workers site** (requests never hit origin, so there is nothing to smart-route). Plausible only for a dynamic origin API with global users where 30–50 ms matters. Tiered Cache alone (see A.10) is now available without Argo on paid plans and is the part most solo devs actually wanted.
- **Sources:** https://www.cloudflare.com/plans/ and https://www.cloudflare.com/plans/application-services/

### A.2 Load Balancing
- **What it is:** Global / local load balancer in DNS / Traffic: pools of origins, health monitors, failover, geo-steering and (higher plans) session affinity. Protects against a single origin / VPS dying.
- **Exact price:** **Starting at $5/mo** on official plans page. Dashboard billing example in developers docs shows “Basic Load Balancing — $5.00” as a flat add-on line. Included starter allocation historically: 2 origins / pools and 500,000 DNS queries/mo; **+$5/mo per additional origin** beyond included — that per-origin detail is from Cloudflare’s add-on breakdowns, verify in dashboard as allocations change by plan. Queries beyond included are metered in dashboard at subscription time.
- **Solo verdict:** **Reasonable $5 buy *if* you have 2+ origins.** For a single-origin or Workers-only deployment it does nothing. One of the few add-ons with a clean, predictable $5 entry.
- **Sources:** https://www.cloudflare.com/plans/ , https://developers.cloudflare.com/billing/understand/how-billing-works/ , https://blog.cloudflare.com/introducing-load-balancing-intelligent-failover-with-cloudflare

### A.3 Advanced Certificate Manager (ACM) + Total TLS
- **What it is:** ACM lets you choose CA (Let’s Encrypt / Google / SSL.com etc.), validity period, cipher suites, ECDSA/RSA, multi-level wildcard (`*.api.example.com`), and issue advanced cert packs per hostname. **Total TLS** is the ACM feature that automatically issues individual certs for every proxied hostname / subdomain so no subdomain goes uncovered — it requires ACM, it is not a separate SKU.
- **Exact price:** **$10/mo per zone** — “Starting at $10 per month” on official plans page, confirmed as $10/mo/zone in ACM docs/blog.
- **Solo verdict:** **Wouldn’t buy unless** you need a second-level wildcard, a specific CA, or Total TLS for many subdomains. Universal SSL (free) already covers apex + first-level wildcard. This is a compliance / multi-tenant feature, not a personal-site need.
- **Sources:** https://www.cloudflare.com/plans/ , https://www.cloudflare.com/plans/application-services/

### A.4 Automatic Platform Optimization (APO) for WordPress
- **What it is:** One-click WordPress edge caching + optimisation plugin: caches HTML at edge, bypasses for logged-in users, optimises fonts / third-party JS.
- **Exact price:** **$5/mo on Free plan; included on Pro / Business / Contract.** Stated verbatim on plans page.
- **Solo verdict:** **Only if you run WordPress.** Irrelevant to a Workers / Astro / static stack. If you do run WP on a Free zone, $5 APO is cheaper than jumping to $20 Pro just for APO.
- **Source:** https://www.cloudflare.com/plans/

### A.5 Cache Reserve
- **What it is:** Persistent cache on top of R2. Normally edge cache evicts under pressure; Cache Reserve keeps cacheable assets (TTL ≥10 hours, with Content-Length) in R2 as an ultimate upper tier for up to 30 days (retention resets on request), shielding origin and saving origin egress.
- **Exact price:** **Pure usage, no flat fee, but requires a paid plan.** Storage $0.015/GB-mo, Class A writes $4.50/million, Class B reads $0.36/million — identical to R2 Standard. Worked example in official docs: 1,000 × 1 GB assets, 1,000 reads each = $19.86/mo ($15 storage + $0.36 reads + $4.50 writes pro-rated).
- **Solo verdict:** **Cheap and sensible for image/file-heavy sites with an origin that charges egress.** For Workers + R2 origin there is no origin egress to save, so it adds cost for little gain. Watch Class A churn — daily-expiring assets rewritten daily can push a 1M-asset example to $171.60/mo in docs.
- **Source:** https://developers.cloudflare.com/cache/advanced-configuration/cache-reserve/

### A.6 Tiered Cache (Smart Tiered Cache)
- **What it is:** Hierarchy of Cloudflare caches: lower-tier PoPs fill from a designated upper-tier PoP instead of every PoP independently fetching origin. Dramatically cuts origin fetches and improves hit ratio. Variants: Generic Global, Smart (topology-aware, recommended).
- **Exact price:** **No separate line-item in current pricing.** Historically bundled with Argo; the current Smart Shield + Argo bundle lists Tiered Caching as included, and Tiered Cache itself is enable-able on paid plans at no usage fee. **Flag:** If dashboard shows Tiered Cache under Argo for your zone, do not assume enabling Smart Tiered Cache alone triggers the $0.10/GB Argo meter — verify the toggle is “Tiered Cache” not “Argo Smart Routing”.
- **Solo verdict:** **Enable it — it’s the free win in this section.** No downside for most sites except slightly higher latency on a true miss via upper tier.
- **Source:** https://www.cloudflare.com/plans/ (Argo bundle includes Tiered Caching)

### A.7 Cloudflare Images (Paid)
- **What it is:** Store originals in Images, serve resized / AVIF/WebP variants, or transform remote / R2 images via URL or Workers binding. Different from free Polish (Pro zone feature).
- **Exact price:** Free: 5,000 unique transformations/mo, no storage. **Paid: Images Transformed $0.50 per 1,000 unique transformations beyond 5,000 included; Images Stored $5 per 100,000 images/mo; Images Delivered $1 per 100,000 delivered/mo.** Storage sold in $5 blocks.
- **Solo verdict:** **Good $5 buy if you serve many image sizes.** For a handful of images, R2 + free transformations (5k/mo) is enough and costs $0. Note transformations are billed *per unique combination per month* — a responsive site with 6 sizes × 2,000 images = 12,000 transformations = $3.50 overage.
- **Source:** https://developers.cloudflare.com/images/pricing/

### A.8 Cloudflare Stream
- **What it is:** Video storage, encoding (adaptive bitrate), player, live ingest (RTMP/SRT/WebRTC), and delivery — priced by minutes, not GB, with encoding + bandwidth included.
- **Exact price:** **Storage: $5 per 1,000 minutes stored (prepaid blocks). Delivery: $1 per 1,000 minutes delivered (postpaid).** Pro/Business zones include 100 min stored + 10,000 min delivered/mo. Plans page: “Starting at $5 per month for 1,000 minutes of video stored.”
- **Solo verdict:** **Affordable entry, scales linearly.** 10 hours stored = ~$3 in a $5 block; 1,000 viewers × 10 min = 10,000 delivered min = $10. Cheaper and far simpler than self-hosting video, but for 2–3 embedded clips YouTube/Vimeo free embed or R2 + `<video>` may be $0. Not a trial — this is the ongoing rate.
- **Sources:** https://developers.cloudflare.com/stream/pricing/ , https://www.cloudflare.com/plans/

### A.9 Spectrum — limited self-serve slice (TCP/UDP proxy for non-HTTP)
- **What it is:** Proxy + DDoS protection for arbitrary TCP/UDP: Minecraft, SSH, RDP, game servers, SMTP, custom protocols. HTTP CDN only covers HTTP/S; Spectrum covers the rest.
- **Exact price (self-serve slice):** On the official Spectrum product page, availability table shows **Pro: 5 GB/mo included, $1/GB overage; Business: 10 GB/mo included, $1/GB overage** for a limited set of applications/ports. Full Spectrum (arbitrary ports, all apps) is Enterprise — see B.14.
- **Solo verdict:** **The $1/GB meter is the trap.** 5 GB is a demo allowance. A public game server doing 200 GB/mo would be ~$195 in overage on Pro. For a private service, use Cloudflare Tunnel (free) instead. Only buy the self-serve slice for a tiny, low-bandwidth TCP service you must expose publicly.
- **Source:** https://www.cloudflare.com/application-services/products/cloudflare-spectrum/

### A.10 Rate Limiting — legacy vs Advanced, and WAF rate-limiting rules
- **What it is:** Count requests by IP / header / cookie / path etc. and block / challenge / throttle. Current model is WAF *Rate Limiting Rules* counted in your plan’s rule quota (Free 1, Pro 2, Business 5, Enterprise up to 100). “Advanced Rate Limiting” adds counting by header, cookie, ASN, response code, etc. and is included on Business/Enterprise WAF.
- **Exact price:** **No separate per-rule fee in the current WAF rules model if you stay within plan quota** — cost is the plan upgrade to get more rules. Legacy rate limiting (pre-WAF product, still on some zones) was **$0.05 per 10,000 good (non-blocked) requests matched** — flag as legacy; do not budget it unless dashboard shows “Rate Limiting (legacy)”.
- **Solo verdict:** **Use the 1–2 rules you already have; don’t upgrade for this alone.** Protect `/login` and `/api/*` and you’re done. Advanced counting characteristics are nice but not $180/mo (Pro→Business jump) nice.
- **Sources:** https://www.cloudflare.com/plans/ (rule counts by plan in comparison table)

### A.11 Snippets vs Transform Rules — not a paid add-on, a plan-count difference
- **What it is:** **Transform Rules** = no-code Rules engine actions: rewrite URL, modify request/response headers, rewrite HTTP response. **Snippets** = tiny JavaScript (max 5 ms CPU, 2 MB, 32 KB package) run *by* the Rules engine for logic Transform Rules can’t express (dynamic header from cookie, signed redirects, A/B logic). Snippets run before Workers.
- **Exact price:** **Included at no extra cost, no usage fees, on all plans** — GA blog: “Snippets are included at no extra cost for Pro, Business, and Enterprise plans — with no usage-based fees” and available on Free too. Counts differ: **Free 5, Pro 10, Business 25, Enterprise 50 Snippets per zone.**
- **Solo verdict:** **Use Snippets instead of a Worker for trivial edge tweaks — saves Workers requests.** No purchase decision here; just a limit that grows if you ever buy Pro.
- **Source:** https://blog.cloudflare.com/snippets/

### A.12 Zaraz Paid
- **What it is:** Server-side tag manager: loads GA4, Meta Pixel, etc. from Cloudflare edge instead of browser, with consent management, Managed Components, and ecommerce events. All features are on all plans — Paid is purely a volume tier, not a feature gate.
- **Exact price:** **1,000,000 Zaraz Events/mo free per account. Paid: $5 per additional 1,000,000 events/mo.** One event = one pageview or one `zaraz.track()` call. Without Paid enabled, Zaraz *disables* at 1M until next cycle (emails at 50/80/90%).
- **Solo verdict:** **Free tier covers ~200k–300k pageviews/mo at 3–5 events/view — most solo sites never pay.** Enable Paid ($5) only as an overage safety net if you’re near 1M events and can’t afford tags suddenly stopping.
- **Source:** https://developers.cloudflare.com/zaraz/pricing-info/

### A.13 Turnstile — paid? No, for solo use
- **What it is:** Privacy-preserving CAPTCHA alternative (Managed / Non-interactive / Invisible widgets).
- **Exact price:** **Free plan: $0, unlimited challenges, up to 20 widgets, 10 hostnames/widget, 7-day analytics.** There is **no self-serve paid Turnstile tier**. Enterprise Turnstile (see B.15) is “Contact Sales” / custom pricing, not published — **UNVERIFIED** for any dollar figure.
- **Solo verdict:** **Use Free, do not budget anything.** Anyone selling you “Turnstile Pro” is not Cloudflare list pricing.
- **Source:** https://developers.cloudflare.com/turnstile/plans/

### A.14 Logpush (now self-serve) + Workers Logpush + Log Explorer
- **What it is:** **Logpush** streams raw logs (HTTP, DNS, Firewall, Audit, Workers Trace, Zero Trust, etc.) to R2 / S3 / GCS / SIEM. **Transformers** (GA Oct 2026) run SQL to filter/redact/reshape before delivery. **Log Explorer** is Cloudflare-hosted log search/retention in-dashboard. **Workers Logpush** is the Workers Trace Events dataset specifically.
- **Exact prices (all official, Oct 2026 model):**
  - Logpush exports to *internal* destinations (R2, Pipelines): **25 GB/mo included, +$0.03/GB**
  - Logpush exports to *external* destinations: **25 GB/mo included, +$0.10/GB**
  - Logpush Transformers: **1 GB/mo included, +$0.04/GB** processed
  - Workers Logpush: **10M requests/mo included (Workers Paid), +$0.05/million**
  - Log Explorer (plans page): **$1/GB ingested, first 10 GB free**
  - Unified Observability (from 1 Dec 2026, announced Oct 2026): Paid plans include 50 GB ingestion + 10 GB-mo storage, then $0.25/GB ingested + $0.10/GB-mo stored — note this future change if budgeting logs into 2027.
- **Solo verdict:** **25 GB free export to R2 is generous — most solo sites pay $0.** Logpush to an external SIEM is where costs appear, and Log Explorer at $1/GB gets expensive if you ingest everything unsampled. Sample or filter with Transformers first.
- **Sources:** https://developers.cloudflare.com/logs/logpush/pricing/ , https://www.cloudflare.com/plans/ , https://blog.cloudflare.com/one-observability-platform/

### A.15 Client-Side Security (formerly Page Shield paid features)
- **What it is:** Detects malicious / compromised third-party JavaScript, Magecart-style skimmers, and unexpected script changes on your pages.
- **Exact price:** **$0.099 per 1,000 requests** on official plans page.
- **Solo verdict:** **Skip unless you load many third-party scripts on a checkout / login page.** At 1M requests/mo that’s $99/mo — it leaves the $5–$50 range quickly at volume. At 100k requests it’s $9.90 and defensible.
- **Source:** https://www.cloudflare.com/plans/application-services/

### A.16 Waiting Room — gated to Business, Advanced is Enterprise
- **What it is:** Virtual queue page for traffic spikes (launches, ticket drops) — holds excess users at edge and admits at a rate your origin can handle.
- **Exact price:** **No standalone $5 add-on. Basic Waiting Room (1 room) is included only on Business ($200/mo annual) and Enterprise.** Advanced Waiting Room (multiple rooms, custom templates, scheduled activation, analytics, mobile app queue) is an Enterprise add-on — **custom pricing, not published — UNVERIFIED** for any third-party “tens of thousands/yr” estimate.
- **Solo verdict:** **Out of budget by gating, not by its own price.** If you truly need a queue for a one-off drop, a third-party queue or a static “sold out / come back” Worker is cheaper than a $200/mo zone upgrade.
- **Sources:** https://www.cloudflare.com/plans/ (Business table lists “Waiting Room (1)”)

### A.17 R2 Infrequent Access (IA) storage class
- **What it is:** Cheaper storage for cold data (backups, archives, logs) you still want instantly retrievable, in exchange for pricier operations + a retrieval fee + 30-day minimum.
- **Exact price:** **Storage $0.01/GB-mo (vs $0.015 Standard), Class A $9.00/M (vs $4.50), Class B $0.90/M (vs $0.36), Data Retrieval $0.01/GB, Egress free, 30-day minimum duration, NO free tier for IA.** 
- **Solo verdict:** **Only for write-once, read-rarely data >~100 GB** where storage saving beats retrieval cost. Terrible for media you serve often — retrieval at $0.01/GB turns 10 TB of reads into $100. Standard is the default for a reason.
- **Source:** https://developers.cloudflare.com/r2/pricing/

### A.18 D1 Read Replication — no extra charge
- **What it is:** D1 can replicate reads to replicas near users (read replication / Sessions API with bookmarks for read-your-writes consistency). Primary still handles writes.
- **Exact price:** **No extra charge for replicas themselves.** Official Workers pricing footnote: “Read replication does not charge extra for read replicas. You incur the same usage billing based on `rows_read` and `rows_written`.” D1 Paid itself: first 25B rows read/mo + 50M rows written/mo included, then $0.001/M rows read, $1.00/M rows written, first 5 GB storage included + $0.75/GB-mo.
- **Solo verdict:** **Enable if your users are global — it’s a free latency win, you only pay the same row counts you’d pay anyway.** Watch unindexed scans: you’re billed for rows *scanned*, not returned.
- **Source:** https://developers.cloudflare.com/workers/platform/pricing/

### A.19 Durable Objects / KV / Queues overages (grouped, already in Workers Paid)
- **What it is:** Stateful coordination (DO), global KV, and message queues billed inside Workers Paid.
- **Exact prices:** DO: 1M requests + 400k GB-s included on Paid (per current docs summarised in pricing page continuation), then **$0.15/M requests, $12.50/M GB-s duration, SQL storage $0.20/GB-mo**. KV and Queues as in A.0.
- **Solo verdict:** **Included allowance covers 1 always-on DO (~336k GB-s/mo).** Second always-on DO or non-hibernating WebSockets are the classic surprise bill — use WebSocket Hibernation API.
- **Source:** https://developers.cloudflare.com/workers/platform/pricing/

### A.20 Zero Trust / Access — Pay-as-you-go (solo-relevant slice)
- **What it is:** Access (ZTNA in front of self-hosted / SaaS apps), Gateway (DNS/HTTP filtering, SWG), Tunnel, WARP. Separate billing axis from zone plans — per *user*, not per zone.
- **Exact price:** **Free: $0 forever, up to 50 users. Pay-as-you-go: $7/user/mo billed annually, no user cap, 30-day log retention, 100% uptime SLA. Contract: custom pricing, not published.**
- **Solo verdict:** **Free tier (50 users) already covers a solo dev + family / small team indefinitely.** $7 for 1 user is in-range only if you exceed 50 users or need 30-day logs / SLA — unlikely personally. Do not confuse this $7 seat with the $5 Workers Paid account fee; they are separate subscriptions.
- **Source:** https://www.cloudflare.com/sase/products/access/

### A.21 Cloudflare Registrar + Cloudflare for SaaS (small add-ons worth noting)
- **Registrar:** At-cost, no-markup domain registration/renewal. Plans page: “Starting at $7.85” (varies by TLD, that figure is a .com-class example). **Verdict: Buy — it is literally wholesale; you save vs most registrars, no monthly fee.**
  - Source: https://www.cloudflare.com/plans/
- **Cloudflare for SaaS / SSL for SaaS:** Issue certs + custom hostnames for *your* customers’ domains pointing at your SaaS. Pricing historically: first allocation included by plan, then per-active-hostname (third-party summaries cite $0.10/hostname/mo, first 100 free on Enterprise) — **flag UNVERIFIED from official pricing page in this research; verify in dashboard / SaaS pricing page before quoting to customers.** **Verdict: Only relevant if you build a multi-tenant SaaS; irrelevant to a personal site.**
  - Source for existence/gating: https://www.cloudflare.com/plans/ (Custom Hostnames / SaaS features in comparison table)

### A.22 What is *not* a paid add-on (avoid double-paying)
- **Transform Rules, Cache Rules, Page Rules (legacy), Redirect Rules, Origin Rules, Compression Rules:** Included in zone plan rule quotas (Free 65 Rules total, Pro 155, Business 310 in current table). No per-rule fee beyond plan.
- **Universal SSL, HTTP/3, Brotli, Early Hints, Rocket Loader (Pro+), Polish/Mirage (Pro+):** Plan features, not add-ons.
- **Tunnel (cloudflared):** No separate charge; costs, if any, are the Zero Trust seat tier in A.20.
- **Source:** https://www.cloudflare.com/plans/

**Section A subtotal example — realistic solo stack:** Workers Paid $5 (owned) + Pro zone $20 annual *only if needed* + ACM $0 + Argo $0 + Load Balancing $0–$5 + Images $0–$5 + Stream $0–$5 = **$5/mo as-is, $25–$40/mo if you add Pro + one $5 add-on.** That is the ceiling of sensible personal spend in this catalog.

---

## B) ENTERPRISE-ONLY / CRAZY-EXPENSIVE — NOT realistic on a personal budget

> For every item in this section: **Pricing model is a custom annual Enterprise / Contract, unless a published per-seat list is noted. Where a dollar figure appears from third parties, it is flagged UNVERIFIED — Cloudflare does not publish it.** Do not budget from the estimate; the point is order of magnitude (thousands/mo, not tens).

### B.1 Enterprise / Contract zone plan itself
- **What it is:** Account-level contract replacing per-zone Pro/Business: named account team / Solutions Engineer, 24/7 phone+chat, 100% uptime SLA with 10× (Standard) / 25× (Premium) credits, network prioritisation, role-based account control, unlimited / custom WAF rulesets, SSO support.
- **Pricing model:** **Custom, billed annually — not published.** Plans page lists only “Custom”. Third-party / community reports of $5,000–$8,000/mo entry are **UNVERIFIED**.
- **Why enterprise:** Sold by sales with traffic/commitment negotiation, annual commit, and procurement / security review — not a dashboard toggle.
- **Source:** https://www.cloudflare.com/plans/

### B.2 Magic WAN (now “Cloudflare WAN”) — the MPLS replacement
- **What it is:** Replaces MPLS circuits and hub-and-spoke corporate WAN: connects data centres, branch offices, and clouds via anycast GRE/IPsec tunnels or CNI to Cloudflare’s network, with Magic Firewall policies inline. This is the product meant when people say “setting up MPLS” in Cloudflare terms — you don’t buy MPLS *from* Cloudflare, you buy its replacement.
- **Pricing model:** **Enterprise-only — custom pricing, not published.** Docs header literally: “Enterprise-only”.
- **Why enterprise:** Requires network architecture design, on-ramps per site, BGP, and hardware Connector appliances — a corporate network project, not a website feature.
- **Source:** https://developers.cloudflare.com/cloudflare-wan/

### B.3 Magic Transit
- **What it is:** L3 DDoS protection + traffic acceleration for entire IP prefixes / on-prem, cloud, and hybrid networks. Announces your /24+ via BGP through Cloudflare, scrubs attacks at edge, returns clean traffic via tunnels/CNI.
- **Pricing model:** **Enterprise-only — custom pricing, not published.** Docs header: “Enterprise-only”.
- **Why enterprise:** You must own a routable prefix, do BGP onboarding with an account team, and commit to bandwidth terms.
- **Source:** https://developers.cloudflare.com/magic-transit/

### B.4 Cloudflare Network Interconnect (CNI)
- **What it is:** Direct physical / partner interconnect (PNI, IX, or platform partner like Megaport/Equinix) from your datacentre to Cloudflare, bypassing public internet for Magic Transit / Magic WAN / CDN traffic.
- **Pricing model:** **CNI ports are $0 to Enterprise customers** per official docs — but **eligibility is determined by your account team, and you pay the datacentre / cross-connect / platform provider separately.** The underlying Enterprise contract is custom pricing, not published.
- **Why enterprise:** You need a cage in a supported facility, a cross-connect order, and a BGP session — $0 port fee does not mean $0 project.
- **Source:** https://developers.cloudflare.com/network-interconnect/get-started/

### B.5 BYOIP (Bring Your Own IP)
- **What it is:** Cloudflare announces *your* IP prefixes from all its locations and binds them to CDN / Spectrum / Magic Transit / Gateway, so traffic keeps your IPs instead of Cloudflare’s.
- **Pricing model:** **Enterprise-only — custom pricing, not published.** Docs header: “Enterprise-only”.
- **Why enterprise:** Only relevant if you own IP space with reputation / allowlist history you cannot abandon; requires RPKI / IRR and onboarding.
- **Source:** https://developers.cloudflare.com/byoip/

### B.6 Workers Audit Logging / Enterprise Log Share & long-retention log delivery
- **What it is:** Three related enterprise log concepts: (1) **Audit Logs v2** — who changed what in the account (retained 18 months for all, UI query 90 days); (2) **Log Share** — a legacy Enterprise role/permission model for granting log access and pushing logs to enterprise SIEMs at scale; (3) **Enterprise Logpush contracts** — pushing Audit Logs *beyond* 18 months and high-volume / all-dataset Logpush under contract terms, plus Workers Trace Events at enterprise volume.
- **Pricing model:** Audit Logs themselves are included; **extended retention / Enterprise Log Share / contract Logpush is custom pricing, not published.** Note the Oct 2026 change (A.14): base Logpush is now self-serve for everyone, so “Logpush = Enterprise-only” is outdated — what remains enterprise is the contract scale/retention/SLA around it.
- **Why enterprise:** Driven by compliance (SIEM, SOX/PCI audit trails, multi-year retention) and procurement, not by a solo dev debugging a Worker — Workers Logs (7-day Paid) + $0 Logpush-to-R2 covers that.
- **Sources:** https://developers.cloudflare.com/fundamentals/account/account-security/audit-logs/ , https://developers.cloudflare.com/logs/logpush/ , https://blog.cloudflare.com/enterprise-for-all-update/

### B.7 Enterprise Support, TAM, and Professional Services
- **What it is:** Named Customer Success / Solutions Engineer, Technical Account Manager, 24/7/365 phone, emergency escalation, SLA credits (10×/25× on Contract vs 1× on Business), onboarding / migration professional services.
- **Pricing model:** **Custom pricing, not published — bundled into Contract, with professional services as paid add-ons to Contract per Zero Trust pricing table.**
- **Why enterprise:** You are buying people and guarantees, not software. A solo dev gets ticket (Pro) / chat (Business) support; there is no self-serve “add TAM for $X” SKU.
- **Sources:** https://www.cloudflare.com/plans/ , https://www.cloudflare.com/sase/products/access/

### B.8 SSO / SCIM at enterprise scale, RBAC & Data Localization Suite
- **What it is:** Dashboard SSO (SAML/OIDC to your IdP), SCIM provisioning/deprovisioning from Okta/Azure AD, granular RBAC at account/zone/resource level, plus **Data Localization Suite**: Regional Services (decrypt/process only in EU/US/etc.), Customer Metadata Boundary, and jurisdiction controls.
- **Pricing model:** Basic SSO support appears in plans comparison for paid plans; **full SCIM, enterprise IdP at scale, and Data Localization Suite / Regional Services are Contract / add-on — custom pricing, not published — UNVERIFIED for any per-seat figure.**
- **Why enterprise:** Driven by a company IdP and data-residency law, not personal use — a solo account uses email + TOTP/passkey + API tokens.
- **Source:** https://www.cloudflare.com/plans/

### B.9 Advanced DDoS, Network Prioritization & Adaptive DDoS
- **What it is:** Beyond unmetered L3/L7 DDoS included on all plans: L7 adaptive DDoS, network prioritisation for Contract traffic, advanced DDoS alerts / analytics, and Magic Transit-grade L3 protection (B.3).
- **Pricing model:** **Custom pricing, not published** for the advanced/contract layer. Base unmetered mitigation is free on all plans — do not conflate the two.
- **Why enterprise:** Prioritisation only matters when you are negotiating how your traffic is treated vs others during a global event, with an SLA behind it.
- **Source:** https://www.cloudflare.com/plans/ (Network Prioritization row: Contract only)

### B.10 Bot Management for Enterprise
- **What it is:** Full 1–99 bot score per request, ML + JA3/JA4 fingerprinting, verified-bots, per-path / per-endpoint policies, Bot Analytics, and fields usable in WAF rules (`cf.bot_management.score`). Lower plans get only Bot Fight Mode (Free, coarse, cannot be skipped by WAF rule) or Super Bot Fight Mode (Pro/Business, categories + limited exceptions, *no* score).
- **Pricing model:** **Enterprise add-on — custom pricing, not published.** Third-party estimates ($5k–$25k/yr add-on) are **UNVERIFIED**.
- **Why enterprise:** Score-based bot policy is an anti-fraud product for login/checkout at scale; Turnstile (free) + Super Bot Fight Mode covers a personal site.
- **Sources:** https://www.cloudflare.com/plans/ , https://developers.cloudflare.com/turnstile/plans/ (Enterprise advanced bot detection context)

### B.11 API Shield (Enterprise API Security)
- **What it is:** API Discovery, Schema Learning/Validation, Sequence Analytics, volumetric abuse detection, mTLS with managed PKI, and per-operation security posture for APIs.
- **Pricing model:** **Enterprise — “To subscribe to API Shield, upgrade to an Enterprise plan and contact your account team.” Custom pricing, not published.** Some base API Shield *limits* exist on Free/Pro/Business (saved endpoints, schema size), but the full product with Log-or-Block, 10,000 endpoints and Sequence Analytics is Enterprise-with-API-Shield.
- **Why enterprise:** Protecting a public API as a product, with schema governance across teams — not a solo Worker with 5 routes and a bearer token.
- **Source:** https://developers.cloudflare.com/api-shield/plans/

### B.12 Enterprise WAF — unlimited managed rulesets, Attack Score, Account-level rulesets
- **What it is:** Unlimited custom WAF rulesets, account-level (not just zone) rulesets, full Managed Ruleset overrides, WAF Attack Score (full, vs “Lite” on Business), exposed-credential checks, and content-scanning at enterprise limits (Free 1 rate rule / Pro 2 / Business 5 / Enterprise up to 100 rate-limiting rules).
- **Pricing model:** **Custom pricing, not published** beyond the Contract plan itself.
- **Why enterprise:** Governance across dozens of zones from one account ruleset is an org problem; a solo dev hits Pro’s 20 custom rules long before needing unlimited.
- **Source:** https://www.cloudflare.com/plans/

### B.13 China Network
- **What it is:** Performance + delivery from data centres *inside* mainland China (via partner network, e.g. JD Cloud), with content compliance handling.
- **Pricing model:** **Enterprise add-on — custom pricing, not published.** Requires a valid **ICP filing/licence per domain** — without it, Cloudflare explicitly serves China visitors from outside China.
- **Why enterprise:** ICP is a regulatory filing a foreign solo dev normally cannot / should not obtain for a personal project, and the add-on is priced as a country network, not a feature toggle.
- **Source:** https://developers.cloudflare.com/support/troubleshooting/general-troubleshooting/geographic-traffic-routing/ (China Network = Enterprise, ICP required — via search result on developers.cloudflare.com)

### B.14 Spectrum for Enterprise (full, arbitrary TCP/UDP)
- **What it is:** The unrestricted version of A.9: any port, any TCP/UDP application, BYOIP support, Magic Transit integration.
- **Pricing model:** **Custom pricing, not published** for Enterprise Spectrum beyond the self-serve GB allowances in A.9. Per-GB enterprise rates cited by third parties ($1/GB first tier) match the self-serve overage but contract tiers are **UNVERIFIED**.
- **Why enterprise:** Running public game / VoIP / financial protocol infrastructure at TB scale through Spectrum is a bandwidth contract negotiation.
- **Source:** https://www.cloudflare.com/application-services/products/cloudflare-spectrum/

### B.15 Turnstile Enterprise
- **What it is:** Unlimited widgets (vs 20), 200 hostnames/widget (vs 10), any-hostname widgets, Ephemeral IDs, off-label (remove Cloudflare branding), 30-day analytics (vs 7).
- **Pricing model:** **“Contact Sales” — custom pricing, not published.**
- **Why enterprise:** Branding removal + ephemeral IDs are compliance/white-label needs for a platform embedding Turnstile for *its* customers.
- **Source:** https://developers.cloudflare.com/turnstile/plans/

### B.16 Zero Trust Contract / Gateway Enterprise & Access at scale
- **What it is:** Contract tier of Cloudflare One: full SASE/SSE — Gateway with DLP (custom data profiles), CASB, Browser Isolation, DEX, device posture at scale, 6-month log retention + Logpush, professional services.
- **Pricing model:** **Per-seat list for self-serve is published ($7/user/mo Pay-as-you-go, A.20); Contract is custom per user/mo billed annually — not published.** At scale the math leaves personal range by headcount alone: 500 seats × $7 = $3,500/mo even *before* Contract uplift, and bundles with Browser Isolation/DLP are quoted higher — any $10–$25/user bundle figure from third parties is **UNVERIFIED** as list.
- **Why enterprise:** You are securing a workforce’s laptops and SaaS access, with DLP/CASB compliance reporting. A solo dev’s “workforce” is one laptop on Free (50 users).
- **Source:** https://www.cloudflare.com/sase/products/access/

### B.17 Other Enterprise-only items to name so they are not mistaken for shopping list items
- **Keyless SSL & Dedicated / Custom SSL at scale, Dedicated Egress IPs (formerly Aegis):** Keep private keys on *your* HSM / server while Cloudflare terminates TLS; dedicated egress IP ranges for origin allowlisting. **Enterprise-only — custom pricing, not published.** Docs note dedicated CDN egress is “only available for Enterprise… reach out to your account team.” Source: https://developers.cloudflare.com/byoip/ (service-bindings / egress note)
- **Secondary DNS, DNS Firewall at scale, Foundation DNS:** Enterprise DNS products for operators running DNS as infrastructure. **Custom pricing, not published.**
- **Cloudflare for Platforms / Workers for Platforms at enterprise scale, Dispatch Workers:** Building a platform where *your* customers deploy Workers. Base Workers for Platforms exists self-serve, but enterprise volume/support is contract. **Custom pricing, not published for enterprise tier — UNVERIFIED beyond that.**
- **IBM Cloud / partner resale (e.g. IBM Cloud Internet Services powered by Cloudflare):** Cloudflare capabilities resold and supported through IBM’s enterprise contracts and marketplace, not a Cloudflare self-serve SKU. **Pricing is IBM contract pricing, not Cloudflare list — custom, not published by Cloudflare.** Flagged here because the brief asked for “IBM partnership stuff”: it is a *channel* for enterprise buying, not a feature a solo dev buys from cloudflare.com.
- **Waiting Room Advanced, Rate Limiting Advanced at enterprise limits, Transform / Managed Transforms at account scale:** All follow the same pattern — basic version in a self-serve plan (see Section A), advanced limits/templates in Contract. **Custom pricing, not published.**

---

## Bottom line for the main document

- **Already paying the right amount:** Workers Paid $5/mo is the sweet spot. Everything in the demo app brief (Workers, D1, R2, Durable Objects, KV, Queues) lives inside that $5 + tiny usage overages.
- **Next sensible *optional* dollars, in order:** (1) $0 — enable Tiered Cache, Snippets, Turnstile Free, Zaraz Free, Logpush-to-R2 free allowance. (2) $5 — Load Balancing *or* Images Paid *or* Stream starter, only if a specific project needs it. (3) $20 annual — Pro for one zone, only if that zone is a real website needing WAF/Polish/APO.
- **Do not budget:** Business ($200/zone), ACM ($10) without a wildcard/CA need, Argo ($5 + $0.10/GB) for cached/Workers traffic, Client-Side Security at volume, or anything in Section B. Section B is not “expensive Pro” — it is a different sales motion (annual contract, account team, custom quote) starting three orders of magnitude above $5/mo.

## Sources index (official first)

- Plans & add-on starting prices: https://www.cloudflare.com/plans/ and https://www.cloudflare.com/plans/application-services/
- Workers / D1 / KV / Queues / Durable Objects / D1 read replication: https://developers.cloudflare.com/workers/platform/pricing/
- R2 / R2 IA: https://developers.cloudflare.com/r2/pricing/
- Cache Reserve: https://developers.cloudflare.com/cache/advanced-configuration/cache-reserve/
- Images: https://developers.cloudflare.com/images/pricing/
- Stream: https://developers.cloudflare.com/stream/pricing/
- Zaraz: https://developers.cloudflare.com/zaraz/pricing-info/
- Turnstile plans: https://developers.cloudflare.com/turnstile/plans/
- Logpush pricing (new self-serve model): https://developers.cloudflare.com/logs/logpush/pricing/ and https://developers.cloudflare.com/logs/logpush/
- Logpush GA for all plans (Oct 2026): https://blog.cloudflare.com/one-observability-platform/ and https://blog.cloudflare.com/enterprise-for-all-update/
- Snippets GA / included pricing: https://blog.cloudflare.com/snippets/
- Billing mechanics / add-on line examples: https://developers.cloudflare.com/billing/understand/how-billing-works/
- Load Balancing launch pricing: https://blog.cloudflare.com/introducing-load-balancing-intelligent-failover-with-cloudflare
- Spectrum availability by plan: https://www.cloudflare.com/application-services/products/cloudflare-spectrum/
- Zero Trust / Access pricing: https://www.cloudflare.com/sase/products/access/
- Magic WAN (Cloudflare WAN) — Enterprise-only: https://developers.cloudflare.com/cloudflare-wan/
- Magic Transit — Enterprise-only: https://developers.cloudflare.com/magic-transit/
- BYOIP — Enterprise-only: https://developers.cloudflare.com/byoip/
- CNI — Enterprise eligibility, $0 port: https://developers.cloudflare.com/network-interconnect/get-started/
- Audit Logs v2 / retention: https://developers.cloudflare.com/fundamentals/account/account-security/audit-logs/
- API Shield plans — Enterprise subscription: https://developers.cloudflare.com/api-shield/plans/

*All prices USD, checked 2026-10-20. Cloudflare changes add-on allocations frequently (Logpush in Oct 2026 is the latest example) — re-verify in dashboard before enabling any metered add-on, especially Argo ($/GB), Spectrum ($/GB), and Log Explorer / Observability ($/GB).*
