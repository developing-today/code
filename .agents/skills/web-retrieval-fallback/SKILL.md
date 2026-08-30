---
name: web-retrieval-fallback
description: Fetch public web content that fails with plain requests (403/429/bot-block pages) by rotating user agents and using documented fallback paths. Use when curl/wget/HTTP fetches get blocked, when a site returns bot-block pages, or when documenting a working retrieval path for future sessions.
---

# Web Retrieval Fallback

Escalation ladder for retrieving public web content when the normal request fails due to automated-client filtering. The canonical instructions live in [`AGENTS.md` → Web retrieval fallback](../../../AGENTS.md#web-retrieval-fallback); this skill operationalizes them.

## References

- **Instructions:** `AGENTS.md` — ["Web retrieval fallback"](../../../AGENTS.md#web-retrieval-fallback) section
- **Full data:** [`ai-crawler-site-access-table.md`](../../../ai-crawler-site-access-table.md) — sorted user-agent list ([`#user-agents`](../../../ai-crawler-site-access-table.md#user-agents)), per-agent notes, per-domain findings, and general retrieval tips

Read both before retrying if you haven't already in this session — prior findings about the specific domain or agent may already exist.

## Ladder

1. **Default request** with normal headers.
2. On 403, 429, bot-challenge page, or obvious filtering: retry with a modern browser UA:
   `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36`
3. Still failing: rotate through other well-known agents (different sites block different agents). Good next tries:
   - `WhatsApp/2.23.20.0` (link-preview fetcher; slips through many news/paywall CDNs)
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot`
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com`
4. Check the doc's *Sites* section for the domain you are hitting — there may be a known working path, mirror, archive link, or rate-limit note.
5. If content still won't come via direct fetch: try mirrors (`web.archive.org`, GitHub mirrors/repos), alternate endpoints (`.json` APIs, `old.reddit.com/.json` style variants), or clone-and-search locally.

## Rules

- Do not use altered User-Agent headers when testing/debugging actual HTTP behaviour of an app, API, auth flow, or client — it hides genuine access-control problems.
- A success via alternate UA is not evidence the resource works normally for ordinary clients.
- Never use UA substitution to bypass authentication, account permissions, or explicit access controls.

## Document findings

When a fallback works (or fails interestingly), update `ai-crawler-site-access-table.md` regardless of session topic:

- Agent-specific finding → its subsection under **Per-agent notes**
- Domain-specific finding → the domain's entry under **Sites** (include mirrors, rate limits, quirks)
- General technique → **General retrieval tips**

Include the update in your next commit. Record failures too ("UA X has known failures on example.com, but UA Y worked"). If early downloads succeed then fail, slow down and retry once before switching strategies — if pacing fixes it, document the inferred rate limit.

## Preserve scarce sources

If a URL was genuinely useful and hard to acquire — very few copies online, or hosted somewhere unlikely to persist — submit it to `https://web.archive.org/save/<url>` (works unauthenticated). Worth archiving: datasheets/whitepaper/manual PDFs, technical docs on fragile personal or CMS-hosted pages, demo/example projects that exist only to illustrate how some code works, hard-to-find header files or source snippets not in any repo or package manager, one-off benchmark posts. Not worth archiving: widely-mirrored content (Wikipedia, MDN, popular repos) or anything already covered elsewhere — you can't and shouldn't archive every page you visit. Anonymous saves are rate-limited to roughly a few per minute; don't hammer it.

Rare case: if you're confident the site is still up but blocked/rate-limited yourself, and you already know from another source that the URL is valuable (a citation, an API doc you need, task-relevant source code), you may submit it unseen and record in the doc's *Sites* section that it wasn't directly accessible but was submitted on `<date>`, so a future agent can check `web.archive.org` for the capture. Only do this when there's a concrete reason the page matters, not speculatively.
