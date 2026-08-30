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

0. **On GitHub, authenticate first** — `-H "Authorization: Bearer $(gh auth token)"`, or use
   `gh api`. The API limit is per-identity, so UA rotation cannot help. See
   [below](#github-authenticate-before-rotating-user-agents).
1. **Default request** with normal headers.
2. On 403, 429, bot-challenge page, or obvious filtering: retry with a modern browser UA:
   `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36`
3. Still failing: rotate through other well-known agents (different sites block different agents). Good next tries:
   - `WhatsApp/2.23.20.0` (link-preview fetcher; slips through many news/paywall CDNs)
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot`
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com`
4. Check the doc's *Sites* section for the domain you are hitting — there may be a known working path, mirror, archive link, or rate-limit note.
5. If content still won't come via direct fetch: try mirrors (`web.archive.org`, GitHub mirrors/repos), alternate endpoints (`.json` APIs, `old.reddit.com/.json` style variants), or clone-and-search locally.

## GitHub: authenticate before rotating user agents

For anything on `api.github.com`, UA rotation is the wrong tool — the limit is **per-identity,
not per-agent**, so no header will help. Authenticate instead:

```bash
TOKEN="$(gh auth token)"
curl -fsSL -H "Authorization: Bearer $TOKEN" https://api.github.com/...
```

| | Unauthenticated | With `gh auth token` |
|---|---|---|
| `api.github.com` core | **60 / hour** | **5 000 / hour** |
| Search API | 10 / min | 30 / min |

Measured, not quoted from docs — check yours with:

```bash
curl -s -H "Authorization: Bearer $(gh auth token)" https://api.github.com/rate_limit
```

Notes:

- 60/hour is easy to exhaust: one recursive tree listing of a large repo can do it in a single
  call, and you will then see 403s with `X-RateLimit-Remaining: 0` that look like bot-blocking
  but are not. **Check the header before reaching for a UA.**
- `raw.githubusercontent.com` is not the API and is far more permissive, so prefer it for
  fetching file contents. The same `Authorization` header works there and is required for
  private repos.
- `gh api` handles auth and pagination itself — `gh api repos/OWNER/REPO/git/trees/BRANCH?recursive=1`
  is usually better than hand-rolling `curl`.
- If `gh auth token` is empty, the user is not logged in. Say so rather than silently falling
  back to unauthenticated requests and blaming rate limits later.
- Never print the token, echo it into a file, or paste it into a commit. Reference it as
  `$(gh auth token)` at the point of use.

The same principle generalises: where a host offers authentication the user already has, use
it before pretending to be a different client.

## Rules

- Do not use altered User-Agent headers when testing/debugging actual HTTP behaviour of an app, API, auth flow, or client — it hides genuine access-control problems.
- A success via alternate UA is not evidence the resource works normally for ordinary clients.
- Never use UA substitution to bypass authentication, account permissions, or explicit access controls.
- Prefer real credentials the user already holds (`gh auth token`) over UA substitution. It is
  not a workaround — it is the supported path, and it raises the GitHub limit 83x.

## Document findings

When a fallback works (or fails interestingly), update `ai-crawler-site-access-table.md` regardless of session topic:

- Agent-specific finding → its subsection under **Per-agent notes**
- Domain-specific finding → the domain's entry under **Sites** (include mirrors, rate limits, quirks)
- General technique → **General retrieval tips**

Include the update in your next commit. Record failures too ("UA X has known failures on example.com, but UA Y worked"). If early downloads succeed then fail, slow down and retry once before switching strategies — if pacing fixes it, document the inferred rate limit.

## Preserve scarce sources

If a URL was genuinely useful and hard to acquire — very few copies online, or hosted somewhere unlikely to persist — submit it to `https://web.archive.org/save/<url>` (works unauthenticated). Worth archiving: datasheets/whitepaper/manual PDFs, technical docs on fragile personal or CMS-hosted pages, demo/example projects that exist only to illustrate how some code works, hard-to-find header files or source snippets not in any repo or package manager, one-off benchmark posts. Not worth archiving: widely-mirrored content (Wikipedia, MDN, popular repos) or anything already covered elsewhere — you can't and shouldn't archive every page you visit. Anonymous saves are rate-limited to roughly a few per minute; don't hammer it.

Rare case: if you're confident the site is still up but blocked/rate-limited yourself, and you already know from another source that the URL is valuable (a citation, an API doc you need, task-relevant source code), you may submit it unseen and record in the doc's *Sites* section that it wasn't directly accessible but was submitted on `<date>`, so a future agent can check `web.archive.org` for the capture. Only do this when there's a concrete reason the page matters, not speculatively.
