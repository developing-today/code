# Site Access by User-Agent / Crawler

> **Mirror.** The maintained copy of this file lives in
> **[`developing-today/hardware-doc`](https://github.com/developing-today/hardware-doc)**
> (`doc/hardware/ai-crawler-site-access-table.md` once the sibling checkout exists), because that
> is where the research runs and where new findings are discovered.
>
> This copy exists so the retrieval guidance in [`AGENTS.md`](AGENTS.md) still works when
> `hardware-doc` has not been cloned. **Record new findings in `hardware-doc` and copy them here**,
> not the other way round — otherwise the two drift and neither is trustworthy.


> **Legend:** ✓ real content · ◐ partial access — `200 OK` but only the empty JavaScript shell, no actual data · ✗ blocked (HTTP code shown)
>
> Reconstructed from an image of the original table. Cells marked `?` were ambiguous in the source image.

| Site            | Claude    | GPTBot    | Perplexity | "Google"  | xAI       | OpenAI FD | Chrome   | curl      | Googlebot |
| --------------- | --------- | --------- | ---------- | --------- | --------- | --------- | -------- | --------- | --------- |
| NYTimes         | ✗ 403     | ✗ 403     | ✗ 403      | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| New Yorker      | ✗ 403     | ✓         | ✗ 403      | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| The Atlantic    | ✗ 402     | ✓         | ✗ 402      | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Bloomberg       | ✗ 403     | ✗ 403     | ✗ 403      | ✓         | ✓         | ✓         | ✓        | ✗ 403     | ✗ 403     |
| Washington Post | ✗ 403     | ✗ 403     | ✗ 403      | ✗ drop    | ✗ drop    | ✗ drop    | ✗ drop   | ✗ drop    | ✗ drop    |
| WSJ             | ✗ 401     | ✗ 401     | ✗ 401      | ✗ 401     | ✗ 401     | ✗ 401     | ✗ 401    | ✗ 401     | ✗ 401     |
| FT              | ✗ 403     | ✗ 403     | ✗ 403      | ✗ 403     | ✗ 403     | ✗ 403     | ✗ 403    | ✗ 403     | ✗ 403     |
| Medium          | ✗ 403     | ✗ 403     | ✗ 403      | ✗ 403     | ✗ 403     | ✗ 403     | ✗ 403    | ✗ 403     | ✗ 403     |
| Wired           | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| The Athletic    | ✗ 403     | ✗ 403     | ✗ 403      | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Wikipedia       | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✗ 403     |
| X – tweet       | ✗ 402     | ✗ 402     | ✗ 402      | ✓         | ✓         | ✓         | ✓        | ✓         | ✗ 404     |
| X – profile     | ✗ 402     | ✗ 402     | ✗ 402      | ◐         | ◐         | ◐         | ◐        | ?         | ✗ 404     |
| Facebook        | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✗ 400    | ✓         | ✓         |
| Threads         | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ◐        | ✓         | ✓         |
| Instagram       | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ◐        | ✓         | ✓         |
| LinkedIn¹       | ✓         | ✗ 999     | ✗ 999      | ✓         | ✗ 999     | ✗ 999     | ✓        | wall      | ✗ 999     |
| Pinterest²      | ✓         | ✗ 403     | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| IMDb            | ✗ 403     | ✗ 403     | ✗ 403      | ◐ 202     | ◐ 202     | ◐ 202     | ✗ 403    | ✗ 403     | ◐ 202     |
| Quora           | ✗ 403     | ✗ 403     | ✗ 403      | ✗ 403     | ✗ 403     | ✗ 403     | ✗ 403    | ✗ 403     | ✗ 403     |
| StackOverflow   | ✗ 403     | ✗ 403     | ✗ 403      | ✗ 403     | ✗ 403     | ✗ 403     | ✗ 403    | ✗ 403     | ✗ 403     |
| Reddit          | ◐         | ✗ 403     | ◐          | ◐         | ◐         | ◐         | ◐        | ◐         | ✗ 403     |
| TikTok          | ◐         | ◐         | ◐          | ◐         | ◐         | ◐         | ◐        | ◐         | ✗ 403     |
| YouTube         | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Twitch          | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| GitHub          | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| HN              | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Substack        | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Tumblr          | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Bluesky         | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Mastodon        | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Goodreads       | ✓         | ◐ 202     | ◐ 202      | ◐ 202     | ◐ 202     | ◐ 202     | ◐ 202    | ◐ 202     | ◐ 202     |
| SoundCloud      | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Flickr          | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |
| Imgur           | ✓         | ✓         | ✓          | ✓         | ✓         | ✓         | ✓        | ✓         | ✓         |

## Notes

- **¹ LinkedIn** — superscript footnote present in the original (text not visible in the screenshot).
- **² Pinterest** — superscript footnote present in the original (text not visible in the screenshot).
- **"Google"** column is quoted in the original header, suggesting it refers to a Google-branded fetcher/user-agent rather than Googlebot proper (which has its own column).
- **OpenAI FD** — "OpenAI File Downloader": an on-demand file/page download fetcher in the OpenAI crawler family (listed in Dark Visitors / ai.robots.txt-derived lists as an unnamed or unclear-purpose OpenAI fetcher).
- **Washington Post** returns `✗ drop` — the connection is dropped rather than answered with an HTTP status.
- **LinkedIn** shows `✗ wall` for curl — likely a login-wall/paywall page rather than an HTTP error code.
- Status codes shown: 400 (Facebook/Chrome), 401 (WSJ), 402 (Atlantic/X/Claude+Perplexity), 403 (common block), 404 (X/Googlebot), 202 (IMDb/Goodreads shell responses), 999 (LinkedIn's non-standard anti-bot status).

## User Agents

Use this section as the working reference for which user agents to try when plain requests fail. The **Quick list** below is sorted alphabetically — pick from it, then check the agent's own subsection (under *Per-agent notes*) or the *Sites* section for known findings about the domain you are hitting. Anyone may update this doc at any time with new findings (which agents worked/failed on which domains, mirrors, rate-limit behaviour), regardless of what the current session is about; include updates in your next commit. Failures are as valuable as successes.

### Quick list (sorted)

Here is the list of agents to use. See the per-agent notes below this list, and the site access matrix at the top of this file, for details on what worked where.

1. `Amazonbot` — Amazon/Alexa fetcher; mostly allowed
2. `Applebot` — Siri/Spotlight; almost never blocked
3. `Bingbot` — Bing/Copilot; almost never blocked
4. `ChatGPT-User` — user-triggered ChatGPT fetching; recommended always-allow
5. `Chrome` — modern browser baseline; try second after plain request
6. `ClaudeBot` / `Claude-SearchBot` / `Claude-User` — Anthropic family; Search/User variants rarely blocked, ClaudeBot (training) often blocked
7. `curl` — plain client; blocked by many news/social sites
8. `DuckAssistBot` — DuckDuckGo AI assist; low volume, typically allowed
9. `Googlebot` / `GoogleOther` / `Google-Agent` — Google family; broadly allowed
10. `meta-externalagent` — Meta AI training; mixed reception
11. `MistralAI-User` — user-initiated citation fetcher; new, rarely blocked
12. `OAI-SearchBot` — ChatGPT search index; recommended always-allow
13. `OpenAI File Downloader` — on-demand OpenAI file/page fetcher
14. `PerplexityBot` / `Perplexity-User` — Perplexity retrieval and user-triggered fetch
15. `WhatsApp` — link-preview fetcher; rarely blocked, slips through many CDNs
16. `XaiImageApiFetch/1.0` — xAI SDK image fetcher
17. `YouBot` — You.com search; low volume

### Reference: exact strings, IP ranges, confidence

| Column | User-Agent token | Full UA string | IP ranges published? | Confidence / source |
| --- | --- | --- | --- | --- |
| Claude | `ClaudeBot` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com` | **Yes** — `https://claude.com/crawling/bots.json` (covers ClaudeBot, Claude-User, Claude-SearchBot) | High — Anthropic's official crawling docs |
| GPTBot | `GPTBot` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; GPTBot/1.4; +https://openai.com/gptbot` (version may change; `robots.txt` marker variant exists) | **Yes** — `https://openai.com/gptbot.json` | High — developers.openai.com/api/docs/bots |
| Perplexity | `PerplexityBot` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; PerplexityBot/1.0; +https://perplexity.ai/perplexitybot` | **Yes** — `https://perplexity.ai/perplexitybot.json` | High — docs.perplexity.ai/guides/bots |
| "Google" | ? | Not certain. Candidates: `GoogleOther`, a Gemini/AI-Mode fetcher (`Google-Agent`), or `AppEngine-Google`. Quoted in the original header, implying it is *not* plain Googlebot. Google-Extended sends no HTTP requests at all. | Google family: **Yes** — Google publishes crawler IPs as JSON and recommends reverse DNS (`googlebot.com`) | Low — inferred |
| xAI | `XaiImageApiFetch` | `XaiImageApiFetch/1.0` — the UA the official xAI SDK uses when fetching image URLs for API requests (5-second timeout, 10 MiB max). Documented Grok crawler tokens also exist (`GrokBot/1.0`, `xAI-Grok/1.0`) but real Grok retrieval traffic usually spoofs browser UAs. | **No** — xAI publishes no IP ranges or first-party crawler doc page | Confirmed — github.com/xai-org/xai-sdk-python (`src/xai_sdk/chat.py`) |
| OpenAI FD | OpenAI File Downloader | On-demand file/page downloader in OpenAI's crawler family. Listed in Dark Visitors / ai.robots.txt-derived lists among OpenAI's unclear-purpose fetchers. Related documented UAs from the same family: `ChatGPT-User` (`... compatible; ChatGPT-User/1.0; +https://openai.com/bot`) and `OAI-SearchBot`. | OpenAI family: **Yes** — `chatgpt-user.json`, `searchbot.json`; File Downloader specifically not confirmed to have its own range file | Medium — name per user-provided reference; string not found verbatim online |
| Chrome | — | `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36` | n/a | High |
| curl | — | `curl/8.14.1` (sites often block on the `curl/` prefix alone) | n/a | High |
| Googlebot | `Googlebot` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Chrome/125.0.0.0 Safari/537.36` | **Yes** — `googlebot.json` + reverse DNS verification (resolve to `googlebot.com`/`google.com` and back) | High |
| WhatsApp *(extra)* | `WhatsApp` | `WhatsApp/2.23.20.0` (older variants like `WhatsApp/2.19.315 A` also seen; some fetches arrive as `facebookexternalhit/1.1 whatsapp`) | No dedicated published list — fetches originate from Meta/Facebook IP space | Medium — widely reported link-preview fetcher |

**Verification tip:** UA strings are trivially spoofable — verify by operator-published IP ranges or reverse DNS where available. Note that user-triggered fetchers (ChatGPT-User, Perplexity-User) publish ranges but their requests are user-initiated, so robots.txt may not apply.

### Per-agent notes

Each agent below has a synopsis derived from the access matrix at the top of this file. **Append new findings (specific domains that worked or failed, rate limits, quirks) directly into the agent's *Findings* list at any time**, regardless of the current session topic, and include the update in your next commit.

#### ClaudeBot (`Claude`)

- String: `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com`
- IP ranges: yes — `claude.com/crawling/bots.json`
- Synopsis: works on most of the web but is the single most-blocked AI training crawler in the matrix. Known failures: NYTimes 403, New Yorker 403, The Atlantic 402, Bloomberg 403, Washington Post 403, WSJ 401, FT 403, Medium 403, The Athletic 403, X/Twitter 402, IMDb 403, Quora 403, StackOverflow 403. Notably it is the only agent that gets real content from Goodreads.
- Findings:
  - *(add findings here)*

#### GPTBot

- String: `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; GPTBot/1.4; +https://openai.com/gptbot`
- IP ranges: yes — `openai.com/gptbot.json`
- Synopsis: blocked by nearly every major paywalled publisher (NYTimes, Bloomberg, WaPo, WSJ, FT, Medium) and most Q&A/social sites (Quora, StackOverflow, Reddit). Allowed on New Yorker, Atlantic, Wired, Wikipedia, and all developer/media platforms (GitHub, HN, Substack, YouTube, etc.).
- Findings:
  - *(add findings here)*

#### PerplexityBot (`Perplexity`)

- String: `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; PerplexityBot/1.0; +https://perplexity.ai/perplexitybot`
- IP ranges: yes — `perplexity.ai/perplexitybot.json`
- Synopsis: tracks ClaudeBot's block list almost exactly (NYT, New Yorker, Atlantic 402, Bloomberg, WaPo, WSJ, FT, Medium, Athletic, X, IMDb, Quora, StackOverflow). Works on Wikipedia, Wired, Facebook family, and all developer platforms.
- Findings:
  - *(add findings here)*

#### "Google" (non-Googlebot Google fetchers)

- Candidates: `GoogleOther`, `Google-Agent`, `AppEngine-Google` (exact string used in the original test unknown)
- IP ranges: covered by Google's published crawler JSONs + reverse DNS
- Synopsis: blocked/dropped by the strict publishers (WaPo drop, WSJ 401, FT/Medium/Quora/StackOverflow 403) and gets shell-only responses from X profiles, Reddit, TikTok, IMDb, Goodreads. Works fine everywhere else.
- Findings:
  - *(add findings here)*

#### XaiImageApiFetch (`xAI`)

- String: `XaiImageApiFetch/1.0` — official xAI SDK image fetcher (5 s timeout, 10 MiB max)
- IP ranges: none published
- Synopsis: mirrors the OpenAI FD column in the matrix — blocked by strict publishers (WSJ 401, FT/Medium 403), LinkedIn 999, Goodreads 202-shell; allowed on news sites like NYTimes/New Yorker/Bloomberg and most social/dev platforms.
- Findings:
  - *(add findings here)*

#### OpenAI File Downloader (`OpenAI FD`)

- On-demand file/page downloader in OpenAI's crawler family (Dark Visitors / ai.robots.txt list an unnamed OpenAI fetcher; exact string not confirmed online)
- IP ranges: OpenAI publishes ranges for its documented bots (`chatgpt-user.json`, `searchbot.json`); no separate file confirmed for this one
- Synopsis: same profile as xAI above — good on news sites, blocked on LinkedIn (999), Goodreads (202 shell), strict publishers.
- Findings:
  - *(add findings here)*

#### Chrome

- String: `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36`
- Synopsis: the baseline browser UA. Gets real content almost everywhere except Quora/StackOverflow/IMDb/Bloomberg (403s), Facebook (400), Washington Post (drop). Gets JS-shell-only responses (◐) from Threads, Instagram, X profiles, Reddit, TikTok, Goodreads — for those, a bot UA can paradoxically work better than a browser.
- Findings:
  - *(add findings here)*

#### curl

- String: `curl/8.x` — blocked on prefix alone at many sites
- Synopsis: worst overall performer among non-AI clients: Bloomberg/WSJ/FT/Medium/Quora/StackOverflow/IMDb hard-block it, LinkedIn serves a login wall, Reddit/TikTok serve shells. Fine on developer sites (GitHub, HN), Wikimedia, and most long-tail media.
- Findings:
  - *(add findings here)*

#### Googlebot

- String: `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Chrome/W.X.Y.Z Safari/537.36`
- IP ranges: yes — `googlebot.json` + reverse DNS
- Synopsis: broadly accepted, with odd exceptions: Wikipedia returns 403 (!), Bloomberg 403, X 404, LinkedIn 999, Reddit/TikTok 403, IMDb 202-shell.
- Findings:
  - *(add findings here)*

#### WhatsApp

- String: `WhatsApp/2.23.20.0` (variants: `WhatsApp/2.19.315 A`, sometimes wrapped as `facebookexternalhit/1.1 whatsapp`)
- IP ranges: no dedicated published list (fetches come from Meta IP space)
- Synopsis: link-preview fetcher; rarely blocked and often slips through CDNs that target AI bots and generic scrapers. Not part of the original matrix — added later as a practical fallback.
- Findings:
  - *(add findings here)*

## Other user-agents likely to be allowed (from the docs reviewed)

Based on the sources above plus the Cloudflare robots.txt analysis and Dark Visitors / ai.robots.txt data, these agents are rarely blocked and generally safe for retrieval:

| Bot | Full UA string | IP ranges published? | Why it's likely unblocked |
| --- | --- | --- | --- |
| `Bingbot` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm) Chrome/125.0.0.0 Safari/537.36` | No list — reverse DNS via `bingbot.microsoft.com` | Only ~2.8% of top sites disallow it; blocking risks Bing/Copilot invisibility |
| `Applebot` | `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15 (Applebot/0.1; +http://www.apple.com/go/applebot)` | No public list — reverse DNS via `applebot.apple.com` | Powers Siri/Spotlight; overwhelmingly allowed |
| `OAI-SearchBot` | `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36; compatible; OAI-SearchBot/1.4; +https://openai.com/searchbot` | Yes — `openai.com/searchbot.json` | Search-indexing bot; publishers told to allow it for ChatGPT citations |
| `ChatGPT-User` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot` | Yes — `openai.com/chatgpt-user.json` (user-initiated; robots.txt may not apply) | Real-time ChatGPT fetching; recommended always-allow |
| `Claude-User` | `Mozilla/5.0 (compatible; Claude-User/1.0; +claudebot@anthropic.com)` | Yes — included in `claude.com/crawling/bots.json` | User-initiated fetch; recommended always-allow |
| `Claude-SearchBot` | `Mozilla/5.0 (compatible; Claude-SearchBot/1.0; +claudebot@anthropic.com)` | Yes — included in `claude.com/crawling/bots.json` | Retrieval index; recommended always-allow |
| `Perplexity-User` | `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; Perplexity-User/1.0; +https://perplexity.ai/perplexity-user` | Yes — `perplexity.ai/perplexity-user.json` ("generally ignores robots.txt") | User-triggered fetch; recommended always-allow |
| `DuckAssistBot` | `Mozilla/5.0 (compatible; DuckAssistBot/1.0; +http://duckduckgo.com/duckassistbot.html)` | No published list | DuckDuckGo AI assist; low volume, typically allowed |
| `MistralAI-User` | `Mozilla/5.0 (compatible; MistralAI-User/1.0; +https://mistral.ai)` | No fixed ranges — user-triggered fetcher | User-initiated citation fetcher; new and rarely blocked |
| `GoogleOther` / `Google-Agent` | `Mozilla/5.0 (compatible) GoogleOther` · `Google-Agent` (added Mar 2026, covers Project Mariner etc.) | Covered under Google's published crawler JSONs + reverse DNS | Treated like Googlebot by most WAFs |
| `Amazonbot` | `Mozilla/5.0 (compatible; Amazonbot/0.1; +https://developer.amazon.com/support/amazonbot)` | No public list — reverse DNS via `amazonbot.com` | Mixed purpose; mostly allowed outside news publishers |
| `YouBot` | `Mozilla/5.0 (compatible; YouBot/1.0; +https://about.you.com/youbot/)` | No published list | You.com search; low volume |
| `meta-externalagent` | `Mozilla/5.0 (compatible; meta-externalagent/1.1; +https://developers.facebook.com/docs/sharing/webmasters/crawler)` | No published list — reverse DNS via `facebot`; Meta states it "may bypass robots.txt" | Blocked by some social platforms (LinkedIn/Medium), generally allowed elsewhere |

**Least-blocked picks:** `Bingbot`, `Applebot`, and the search/user-fetch class (`OAI-SearchBot`, `ChatGPT-User`, `Claude-User/Claude-SearchBot`, `Perplexity-User`) are consistently recommended as always-allow across every source reviewed. The training crawlers (`GPTBot`, `ClaudeBot`, `CCBot`, `Bytespider`) and stealth fetchers (Perplexity's undeclared Chrome-impersonation UA documented by Cloudflare) are the ones most likely to hit 403s.

**IP-range summary:** Operators publishing verifiable ranges: OpenAI (all three documented bots), Anthropic (bots.json), Perplexity (both bots), Google (JSONs + reverse DNS). No published ranges (reverse-DNS only or nothing): Microsoft/Bing, Apple, Amazon, Meta, DuckDuckGo, Mistral (user-triggered), You.com, xAI.

## Sites

Per-domain findings. Each site below has a synopsis from the access matrix; **append new findings at any time** — including things beyond user agents (redirects, moved/deleted resources, rate limits, mirrors). Include updates in your next commit.

### News / paywalled

#### nytimes.com
Never works with AI training bots (Claude/GPTBot/Perplexity 403) but almost any other UA works, including curl.
- Findings:
  - *(add findings here)*

#### newyorker.com
Blocks Claude and Perplexity (403); GPTBot and everything else fine.
- Findings:
  - *(add findings here)*

#### theatlantic.com
Claude and Perplexity get 402 (Cloudflare pay-per-crawl signal); GPTBot allowed.
- Findings:
  - *(add findings here)*

#### bloomberg.com
Hard blocker: 403 for Claude/GPTBot/Perplexity, curl, and Googlebot. Browser-like and other AI fetchers pass.
- Findings:
  - *(add findings here)*

#### washingtonpost.com
Drops connections entirely (no HTTP response) for most non-AI clients; AI bots get 403. Plain web requests essentially never work.
- Findings:
  - *(add findings here)*

#### wsj.com
401 for every client tested. Never works with plain requests.
- Findings:
  - *(add findings here)*

#### ft.com
403 for every client tested.
- Findings:
  - *(add findings here)*

#### medium.com
403 for every client tested.
- Findings:
  - *(add findings here)*

#### wired.com
Works with every agent tested.
- Findings:
  - *(add findings here)*

#### theathletic.com
Blocks the three main AI training bots only; browsers/curl/Googlebot fine.
- Findings:
  - *(add findings here)*

### Social / platforms

#### x.com (Twitter)
AI training bots get 402 on tweets; profiles are JS-shell-only for browser-like clients; Googlebot gets 404. Individual tweets are fetchable with Chrome/curl-style UAs.
- Findings:
  - *(add findings here)*

#### facebook.com / threads.net / instagram.com
Fine for most UAs, but Chrome gets shell-only or 400 responses — bot UAs can work better than browser UAs here.
- Findings:
  - *(add findings here)*

#### linkedin.com
Aggressive: 999 for GPTBot/xAI/OpenAI/Googlebot; curl hits an auth wall; Claude/Chrome/"Google" pass. Auth walls mean much content is unreachable regardless of UA.
- Findings:
  - *(add findings here)*

#### pinterest.com
Only GPTBot is blocked (403).
- Findings:
  - *(add findings here)*

#### reddit.com / tiktok.com
Shell-only (◐) for nearly all clients; GPTBot/Googlebot hard-blocked (403). Reddit's JSON endpoints (`old.reddit.com/.json`, etc.) are often a better path than HTML.
- Findings:
  - **2026-08-24 — `www.reddit.com/search.json` is now 403 for *every* UA tried; `old.reddit.com` still works.** Confirmed while researching the Seeed XIAO ESP32S3 Sense.
    - `https://www.reddit.com/search.json?q=...` returned **403** for: Chrome 131 desktop UA, `WhatsApp/2.23.20.0`, `ChatGPT-User/1.0`, `curl/8.5.0`. `ClaudeBot/1.0` returned **429** (rate-limited, i.e. reached a different layer).
    - The 403 body is a full HTML block page (~190 KB), not JSON — so **check the status code, not just whether bytes came back**. `json.load()` failing with an HTML snippet is the tell.
    - **Working path:** `https://old.reddit.com/search.json?q=<query>&limit=50&sort=relevance&t=all` with `-A "WhatsApp/2.23.20.0"` → **200**, valid JSON.
    - Thread contents also work: `https://old.reddit.com<permalink>.json?limit=40` with the same UA. Returns a 2-element array — `[0]` is the post, `[1]` is the comment tree (recurse `data.children[].data.replies`).
    - Be gentle: ~2 s between requests was sufficient for ~10 requests with no throttling. `ClaudeBot`'s 429 suggests the domain tracks per-agent quota.
    - Takeaway: **prefer `old.reddit.com` over `www.reddit.com` for all JSON access**, and rotate to `WhatsApp/2.23.20.0` when a browser UA is refused.

### Q&A / reference

#### wikipedia.org
Almost always works with any of these user agents — the lone oddity is Googlebot getting a 403.
- Findings:
  - *(add findings here)*

#### quora.com / stackoverflow.com
403 for every single client tested. StackOverflow content is widely mirrored (archive.org, third-party scrapes); prefer those.
- Findings:
  - *(add findings here)*

#### goodreads.com
202 shell-only for everyone except ClaudeBot, which gets real content.
- Findings:
  - *(add findings here)*

### Electronics distributors / parts catalogues

#### datasheet.lcsc.com / www.lcsc.com  — **datasheet PDFs now blocked (2026-08-24)**
- **Every** `datasheet.lcsc.com/lcsc/<id>_<part>.pdf` and `www.lcsc.com/datasheet/<C-code>.pdf` request returns **HTTP 200 with `text/html`**, not a PDF. The body is a Vue SPA shell.
- Tried and all identical: Chrome 131 (Win + Linux), `curl/8.5.0`, `WhatsApp/2.23.20.0`, `ClaudeBot/1.0`. **UA rotation does not help** — it is not UA-gated.
- **Silent-corruption hazard:** the response is 200 with a `.pdf` URL, so naive scripts save an HTML page as `something.pdf`. **Always check the `%PDF` magic**, not the status code.
- The HTML returned is frequently **for a different part than requested** (a resistor page came back for a microphone query), so the URL is not even being honoured.
- Useful side effect: the returned page usually contains the real CDN link, e.g. `https://datasheet.lcsc.com/datasheet/pdf/<hash>.pdf`. Extracting and fetching that may work — untested.
- Historical note: a URL of this form cited in an older record of ours (`1811081617_MEMSensing-MSM261D4030H1CPM_C74250.pdf`) **used to work** and now does not, so previously-captured LCSC links should be treated as dead.
- `wmsc.lcsc.com/wmsc/search/global?keyword=` returns `{"code":404,"msg":"The static resource is unavailable..."}`.

##### ✅ What *does* work on LCSC — the product-detail page (confirmed 2026-08-24)
Re-tested while resolving Espressif `C328062`. **The `www.lcsc.com/product-detail/<C-code>.html` page is open and is the reliable route.**
- `https://www.lcsc.com/product-detail/C328062.html` → **HTTP 200, 378,243 bytes**, with a plain **Chrome 131** UA. **`WhatsApp/2.23.20.0` returned the identical byte count**, so the page is *not* UA-gated — the earlier `datasheet.lcsc.com` failures are specific to the PDF endpoints, not to the whole domain.
- The useful data is **embedded JSON in the HTML**, not in the DOM. Grep the raw page for:
  `"productModel"` (the real manufacturer ordering code — this is what resolves an ambiguous C-code), `"brandNameEn"`, `"encapStandard"` (package + body size), `"catalogName"`, `"productIntroEn"`, and `"pdfUrl"`.
- `"pdfUrl"` yields the CDN form `https://datasheet.lcsc.com/datasheet/pdf/<hash>.pdf?productCode=<C-code>` — this is the "untested" extraction suggested above, and **it is the right lead to follow** when you need the mirrored datasheet.
- **Failed on the same lookup, for the record:** `jlcpcb.com/api/luxService/getComponentDetailByComponentCode?componentCode=…` → HTTP 200 with an **empty body**; `cart.jlcpcb.com/api/overseas-shop-cart/v1/shopCartSearch/searchComponentList?keyword=…` → 200, empty body; `wmsc.lcsc.com/wmsc/product/detail?productCode=…` → `{"code":404}`; `wmsc.lcsc.com/ftps/wm/search/global?keyword=…` → **Akamai "Access Denied"**; `yaqwsx.github.io/jlcparts/data/` → 404 (that mirror has moved).
- **Rule of thumb:** for LCSC/JLCPCB, scrape `www.lcsc.com/product-detail/<C-code>.html` and parse the embedded JSON. Do not bother with the `wmsc.*` or `jlcpcb.com/api/*` hosts.

#### memsensing.com
- `403` to every UA tried (Chrome, curl, WhatsApp, ClaudeBot). No datasheets retrievable.


Tested 2026-08-21 while researching the Espressif ESP32-U4WDH.

| Site | Chrome UA | WhatsApp UA | ChatGPT-User / ClaudeBot | Notes |
|---|---|---|---|---|
| `lcsc.com` | ✅ 200 | — | — | Fully accessible. **Price ladder and stock are not in the rendered HTML — they are in embedded JSON.** Grep the raw page for `"productPriceList"`, `"stockNumber"`, `"minPacketNumber"`, `"encapStandard"`. Far more reliable than scraping the DOM |
| `jlcpcb.com` | ✅ 200 | — | — | Product data is JS-loaded; raw HTML is large but useful fields are sparse. Note a part can appear under **more than one** LCSC/JLC code |
| `mouser.com` | ❌ 403 "Access Denied" bot-wall | ❌ 403 | ❌ connection reset (`HTTP/2 INTERNAL_ERROR`) | No working UA found. Returns a ~13.9 KB denial page with a `Reference-ID`. Mouser publishes a **Search API** — that is the intended path |
| `digikey.com`, `digikey.ca`, `digikey.com.br` | ❌ 403 | ❌ 403 | ❌ 403 | No working UA found on any regional domain |
| `octopart.com`, `findchips.com`, `trustedparts.com` | ❌ 403 | — | — | Aggregators uniformly hostile. Octopart returns a 52 KB 403 body that superficially looks like content — **check the status code, not the byte count** |
| `digi-electronics.com` | ✅ 200 | — | — | **Useful workaround for Digi-Key:** mirrors Digi-Key's parametric table, part numbers, MSL, ECCN, packaging. ⚠ Its long-form prose is **AI-generated and factually wrong** (misstated a chip's operating temperature by 20 °C against the manufacturer datasheet). Use the tables, discard the paragraphs |

- Findings:
  - General pattern: **Chinese/Asian distributors (LCSC, JLCPCB) are open; US/EU distributors and aggregators (Mouser, Digi-Key, Octopart, Findchips, TrustedParts) are uniformly closed** to every UA tried. When you need US-distributor parametrics, look for a catalogue mirror rather than rotating UAs — rotation did not help on any of them.
  - Beware AI-generated parts content on distributor-adjacent sites. Cross-check every number against the manufacturer datasheet before recording it.

### Vendor documentation portals

| Site | Chrome UA | Notes |
|---|---|---|
| `documentation.espressif.com` | ✅ 200 | Espressif's current doc platform. **Serves a soft-404: HTTP 200, `text/html`, exactly 13,745 bytes, final URL `/404`.** Always validate `%PDF` magic bytes rather than trusting the status code. Also serves **HTML builds** of datasheets at `<slug>.html` alongside `<slug>.pdf` |
| `www.espressif.com/sites/default/files/…` | ✅ 200 | Legacy static store. Mostly redirects into the above, **but `pcn_downloads/` still holds PDFs that the new platform only exposes as HTML** — worth trying when a PCN appears to be HTML-only |

> **Espressif withdraws documents for EOL/NRND modules, and the withdrawal looks exactly like a typo** (HTTP 200, 13,745-byte shell). Confirmed 2026-08-24: `esp32-wrover_datasheet_en.pdf` and `esp32-wrover-i_datasheet_en.pdf` are gone from the live store while `esp32-wrover-b_datasheet_en.pdf` (also NRND) is still served. **Query the Wayback availability API before concluding a document never existed** — `https://archive.org/wayback/available?url=<url>` is fast, unauthenticated, and returned a live 2025-10-15 snapshot of the WROVER datasheet, which fetched as a genuine 470,491-byte PDF via the `…/web/<timestamp>if_/<url>` form. `esp32-wrover-i` had **no** snapshot, which is itself good evidence that no such standalone document ever existed.
>
> **Espressif's naming convention explains most "missing" module datasheets:** the external-antenna variant is documented *inside* its PCB-antenna sibling's file. There is no `esp32-wroom-32u_datasheet_en.pdf`; it is `esp32-wroom-32d_esp32-wroom-32u_datasheet_en.pdf`. Search for the **pair**.
| `www.espressif.com/en/support/documents/{pcns,advisories}` | ✅ 200 | **The reliable enumeration path.** Plain server-rendered Drupal HTML listing every PCN / advisory PDF by absolute URL. Note `grep` reports these pages as binary — use `grep -a` |
| `esp32.com` (phpBB forum) | ✅ 200 | Fully accessible. Thread URLs are `viewtopic.php?t=<id>`; add `&start=10` for later pages |
| `docs.espressif.com` (esp-docs) | ✅ 200 | Many landing pages are JS-rendered shells with no links — you cannot enumerate a project's PDFs by scraping them |
| `www.wch-ic.com` / `www.wch.cn` (WCH / Nanjing Qinheng) | ✅ 200 | **Vue SPA. Returns HTTP 200 + a ~4,305-byte app shell (4,271 on `wch.cn`) for _every_ unrecognised path**, including all `/downloads/<NAME>_PDF.html` document URLs and all `/downloads/file/<id>.html` pages. A naive `curl` of a datasheet URL therefore looks like a success and yields HTML. **Use the JSON API instead — see the WCH finding below.** No UA substitution needed |
| `www.nxp.com` | ❌ **404** | ⚠️ **Inverted block — the default `curl` UA works and browser UAs do not.** Tested 2026-08-24: Chrome 131, Safari 17, WhatsApp, ChatGPT-User and ClaudeBot UAs **all returned HTTP 404** on `https://www.nxp.com/`; bare `curl/8.5.0` returned **200**. Same inversion on `https://www.nxp.com/docs/en/data-sheet/PN7150.pdf` (404 browser / 200 curl). **Do not apply the standard UA-substitution fallback here — it makes things worse.** Datasheet path pattern is `nxp.com/docs/en/data-sheet/<PART>.pdf`; not byte-validated for `%PDF` **Re-confirmed 2026-08-26:** `https://www.nxp.com/products/PCF85063A` returned **HTTP 200** (231,674 B) with bare `curl` and no `User-Agent` header. |
| `www.aosmd.com` (Alpha & Omega Semiconductor) | ✅ 200 | **Best-behaved vendor host tested.** `res/data_sheets/<PART>.pdf` is directly constructible — `AO3400A.pdf` → 200, 317,848 bytes, `%PDF`. Negative control `BOGUS999.pdf` → **real 404**, so status codes are trustworthy |
| `www.ti.com` | ✅ 200 | `lit/ds/symlink/<part-lowercase>.pdf` works; negative control → **real 404**. **Strip package suffixes**: `tlv62569` works, `tlv62569dbvt` 404s. Search and parametric pages are JS apps |
| `www.sg-micro.com` (SGMicro) | ✅ 200 | ⚠️ **Soft-404s.** `product/SGM2036` → 200 / 590,614 bytes; nonexistent `product/BOGUS999` → **200 / 69,130 bytes**. Distinguish by body size, never by status code |
| `www.bosch-sensortec.com` | ✅ 200 | `media/boschsensortec/downloads/datasheets/bst-<part>-ds<NNN>.pdf` verified (`bst-bme280-ds002.pdf` → 200, 1,663,806 bytes, `%PDF`). The `ds<NNN>` revision counter is not guessable — scrape it |
| `www.goodix.com` | ✅ 200 | `/en/document/center` is a real portal (200, 115,480 bytes). `/en/product` **404s**; the category segment is required (`/en/product/touch` → 200) |
| `www.hynitron.com` | ✅ 200 (**HTTP only**) | ⚠️ **No HTTPS.** `https://www.hynitron.com/` and `https://hynitron.com/` both **fail to connect**; `http://www.hynitron.com/` returns 200. Tooling that force-upgrades to HTTPS will report the site as dead when it is not. Chinese-only |
| `www.liteon.com`, `optoelectronics.liteon.com` | ❌ no connection | **All hosts unreachable 2026-08-24** (connection failure, no HTTP status) with a Chrome UA. `www.lite-on.com` resolved but returned an **empty body**. **UA rotation was not attempted** — try that before concluding the sites are gone |
| `www.capellamicro.com` | ✅ 200 | Degraded legacy static site (`<TITLE>New Products</TITLE>`, references `old.css`). `capellamicro.com.tw` **does not resolve**. Company was acquired by Vishay — go there instead |
| `www.xlsemi.com`, `www.chipsemicorp.com`, `www.qstcorp.com`, `www.memsensing.com`, `www.sitronix.com.tw` | ✅ 200 | All reachable, all **Chinese-first**. Guessed product/category paths 404'd on each — the URL schemes were not determined. For these vendors **LCSC's datasheet mirror is the better first attempt** |
| `zerowriter.ink` | ❌ **403** | ⚠️ **Partial block, and the site root lies.** Tested 2026-08-24: `https://zerowriter.ink/` returns **200** to a Chrome 131 UA, but `/pages/*` and `/products/*` return **403**. The **ClaudeBot UA returns 200** on those same paths. A reachability check against the root therefore passes while every documentation page fails. Shopify-hosted, so real product URLs are `/products/<handle>` with handles not derivable from menu labels — fetch the root and extract links rather than constructing them |
| `zerowriter.com` | — | **Not the project.** Domain-for-sale parking page (Spaceship.com, $7,500). The Crowd Supply campaign and most press link here. The live site is `zerowriter.ink` |
| `reddit.com`, `old.reddit.com` | ❌ **403** | ⚠️ Tested 2026-08-24: Chrome 131 UA → **403**; bare `curl/8.5.0` → **403**; Googlebot UA → **403**; ClaudeBot → **429**. **`WhatsApp/2.23.20.0` → 200.** Use `old.reddit.com/r/<sub>/top.json` and `/search.json?q=…&restrict_sr=1`; append `.json` to any permalink for a full comment tree. Throttle — repeated calls draw 429s |
| `kickstarter.com` | ❌ **403** | **Blocks every UA tried** (Chrome, ClaudeBot, WhatsApp, ChatGPT-User, facebookexternalhit). Note the 403 body is ~294 KB and contains project prose, so a size check is not a success check — the `<title>` is `You're not allowed to do that (403 Forbidden)`. **Use the Wayback Machine**; snapshots exist |
| `soldered.com`, `github.com/SolderedElectronics` | ✅ 200 | Product-page slugs churn — `.../soldered-inkplate-5-9-7-e-paper-board-copy/` returned **404 with a 561 KB body**, so validate on status code, not size. **GitHub is the better source**: `Soldered-<Product>-hardware-design` repos carry KiCad sources, BOM CSVs, schematic PDFs, gerbers, 3D STEP *and* CE/UKCA compliance certificates |
| `www.eink.com` (E Ink Holdings) | ⚠️ **200 for literally everything** | ⚠️ **Textbook class-(c) uniform SPA shell.** Tested 2026-08-24 with a Chrome 131 UA: `/`, `/products`, `/product/ED052TC4`, `/brand.html`, `/brand.html?type=carta`, `/tech.html?type=electronicink` **all returned HTTP 200 with byte-identical 204,139-byte bodies.** A 200 from this host is worthless as evidence a page exists. Worse, a text scrape finds **no part numbers at all** — a regex for `ED\d{3}[A-Z]{2}\d` over the products page returns **zero** matches. E Ink genuinely publishes no part catalogue |
| `www.e-ink.com` (hyphenated) | ❌ no connection | **Times out with no HTTP status**, with and without a browser UA (2026-08-24). Not a block — the domain simply is not the company's site. The live host is `eink.com` |
| `shopkits.eink.com` | ✅ 200 | ⚠️ **The only E Ink property that emits real data.** E Ink's own kit store: per-part specification tables (resolution, active area, pixel pitch, outline, weight, grey levels, ppi, backplane) plus downloadable user manuals. Patterns: `/en/product`, `/en/product?cate=Black%26White`, `/en/product/detail/<desc>(<PART>)`, `/en/download/<opaque-id>/<file>.pdf`, `/en/product/detail/CompatibilityTable`. **~91 SKUs, evaluation kits only** — most production AMEPD panels are absent. Business accounts only for purchase |
| `www.panelook.com` | ❌ **slider CAPTCHA** | Tested 2026-08-24, with and without a Chrome 131 UA: both a direct parameter URL and `modelsearch.php?keyword=` returned **HTTP 200 with an 11,210-byte drag-slider challenge page** ("Please hold down the left slider and drag it to the right"). ⚠ **200 + a plausible size, so a naive scraper records success.** This is the database that indexes E Ink/LCD panel part numbers with parameters, so its loss is expensive — no workaround found |
| `inkplate.readthedocs.io` | ✅ 200 (**useless**) | ⚠️ **Deprecated — every page is a JavaScript redirect shim** to `soldered.com/documentation/inkplate`. `curl` returns 200 and almost no content. **Use the `.rst` source** in `github.com/SolderedElectronics/Inkplate-documentation` (`source/*.rst`, ~10,000 lines) instead |

- Findings:
  - **2026-08-24 — always probe a deliberately bogus path as a negative control before trusting a 200.** Vendor hosts fall into three classes, and you cannot tell which from a single successful request: (a) **honest** — real 404s, so a 200 means the document exists (TI, AOS, Bosch); (b) **soft-404** — 200 with a smaller body for missing pages (SGMicro: 590 KB real vs 69 KB missing); (c) **uniform SPA shell** — byte-identical 200 for every path, existing or not (WCH: 4,305 bytes). Class (c) will make you record "datasheet retrieved" for a file you never got. One bogus-path request settles it.
  - **2026-08-24 — the standard UA-substitution fallback is not universally safe.** `nxp.com` inverts it: the default `curl` UA is allowed and every browser and bot UA tried returns 404. When a *major* vendor 404s its own homepage, suspect UA filtering in **either** direction and try the bare default before escalating.
  - **2026-08-24 — some manufacturers publish no part data at all, and their website will not tell you so.** `eink.com` returns a byte-identical 204 KB SPA shell for every path including invented part URLs, which reads as "the page exists, my extractor is broken." It is neither. E Ink distributes panel specifications only under NDA — their own leaked spec documents say so on page 1 — and the corporate site contains **zero** part numbers. When a vendor's site looks like it should have a catalogue and does not, consider that the absence is the business model, and go to (a) the **board vendor's open design files** for the connector pinout, (b) the **board vendor's driver source** for the protocol, and (c) the manufacturer's own **evaluation-kit store** (`shopkits.eink.com`), which is often the one place real numbers are published.
  - **2026-08-24 — check `archive.org/wayback/available` BEFORE submitting a save.** `web.archive.org/save/<url>` timed out at 120 s and then returned **HTTP 503** on retry, which reads as "archival failed." But the one-line JSON API `https://archive.org/wayback/available?url=<url>` answered instantly and showed a snapshot **already existed** — and fetching it returned a **byte-identical** file (same SHA-256) to the live one. The availability API is fast, unauthenticated and never rate-limited in the way `save/` is. Query it first; only submit a save if it comes back empty.
  - **2026-08-24 — public web search is currently unusable from this environment for obscure part numbers.** DuckDuckGo's `html/` and `lite/` endpoints both return a bot-challenge page ("select all squares containing a duck") with and without a browser UA. Bing returned **entirely unrelated German CRM-software results** for a quoted `"ED052TC4"` query — not zero results, *wrong* results, which is more dangerous. **Do not treat a search-engine miss as evidence a document does not exist**; probe candidate hosts directly instead.
  - **2026-08-24 — a 200 on a site's root does not mean the site is accessible.** `zerowriter.ink` serves the root to a Chrome UA but 403s every documentation and product path; the ClaudeBot UA gets 200 on all of them. Probe an actual content page, not just the homepage, before concluding a site works.
  - **2026-08-24 — the WhatsApp UA is the one that works on Reddit.** Chrome, bare `curl` and Googlebot all 403; ClaudeBot gets 429. `WhatsApp/2.23.20.0` returns 200 on `old.reddit.com` JSON endpoints. Reddit's JSON API needs no key: `/r/<sub>/top.json?t=all&limit=N`, `/r/<sub>/search.json?q=…&restrict_sr=1`, and `<permalink>.json` for a whole thread with nested comments.
  - **2026-08-24 — a large 403 body can look like a successful fetch.** Kickstarter returns **294 KB** of HTML with a 403 status, including real project prose, so any check based on response size will pass. Check the status code and the `<title>`.
  - **2026-08-24 — when a vendor's website looks dead, grep their repository for the domain.** `zerowriter.com` is a parked for-sale page and is what the Crowd Supply campaign and every press article link to, which reads as an abandoned project. The live site (`zerowriter.ink`) was found only in a URL inside `firmware_releases/updates-readme.txt` in their GitHub repo. Release notes, installers, `howto.txt` files and firmware readmes routinely carry the current domain long after marketing pages go stale.
  - **2026-08-24 — GitHub's licence detector reports TAPR OHL as `NOASSERTION`.** Every Soldered Electronics hardware repo shows `NOASSERTION` or `None` via the API while carrying an unambiguous **TAPR Open Hardware License v1.0** in `LICENSE.md`. Do not treat the API field as evidence a hardware repo is unlicensed — open the licence file.
  - **2026-08-24 — GitHub org listings need pagination or you will miss most of a vendor's repos.** `SolderedElectronics` has 200+ repositories; `?per_page=100&page=1` returned **no** Inkplate 5 hardware repos at all. They surfaced only via `/search/repositories?q=inkplate+5+in:name`. Paginate, and cross-check with search.
  - **Chinese component vendors: go to LCSC first, not the manufacturer.** `datasheet.lcsc.com/lcsc/<id>_<Manufacturer>-<Part>_C<NNNNN>.pdf` embeds manufacturer, part number and the stable LCSC `C`-code, giving unusually good provenance for a mirror. This is how the MEMSensing microphone datasheet was obtained; the manufacturer's own site yielded nothing. Applies to MEMSensing, QST, XLSEMI, Chipsemi and similar.
  - **Vendor documentation for display and touch controllers is usually obtained from someone else.** Sitronix's ST77916 spec came from `dl.espressif.com` (the silicon vendor needed it to write a driver); Hynitron's CST816D datasheet came from `files.waveshare.com` (a board vendor mirroring it for customers). Neither manufacturer publishes a public register map. Search the *downstream* vendors, not the IC maker.
  - **Espressif PDFs often have deliberately obfuscated text layers.** Two distinct schemes were seen in the same document set: some PDFs (PCNs, some user guides) use a **uniform +29 code-point offset** — add 29 to each extracted byte and plain ASCII including digits falls out. Others (the ESP32 series datasheet) use a **non-uniform glyph-subset map** that no simple offset recovers.
  - **The reliable workaround is the HTML build, not better PDF tooling:** `documentation.espressif.com/<slug>.html` served the full, cleanly extractable datasheet text when the PDF of the *same document* was unreadable. Try HTML first for any Espressif document you need to quote or search.
  - Slugs are unforgiving: `esp32-mini-1_datasheet_en.pdf` works, `esp32-mini-1_esp32-mini-1u_datasheet_en.pdf` soft-404s, even though the document covers both parts.
  - **Update 2026-08-21 — `documentation.espressif.com` is now a JavaScript SPA, and the 13,745-byte fingerprint above is no longer a 404 test.** *Every* `.html` slug returns that same app shell, including slugs that exist (e.g. `esp32_chip_revision_v3_0_user_guide_en.html` returns 13,745 bytes with the correct `<title>` and `<meta name="description">` in the shell, but no body content). Only `.pdf` slugs return real bytes. The datasheet HTML build that worked previously is still worth trying, but expect the shell.
  - **To enumerate Espressif PCNs and advisories, scrape the old Drupal index pages, not the doc platform and not a search engine.** `https://www.espressif.com/en/support/documents/pcns` and `https://www.espressif.com/en/support/documents/advisories` are still plain server-rendered HTML; `grep -o 'href="[^"]*\.pdf"'` yields the complete list of `sites/default/files/pcn_downloads/…` and `…/advisory_downloads/…` URLs. **PCN and advisory filenames cannot be guessed** — they embed the full human title, sometimes with double spaces (`AR2022-005 Compatibility Advisory for Chip Revision Numbering  Scheme.pdf`) and sometimes in Chinese. Direct URL guessing failed on every attempt; index enumeration succeeded first try.
  - **`docs.espressif.com/projects/esp-chip-errata/` is a real Sphinx site and extracts perfectly**, unlike the errata PDF (which uses the obfuscated text layer). Chapter slugs under `/en/latest/esp32/`: `01-chip-identification/`, `02-errata-summary/`, `03-errata-description/`, `revision-history/`. Prefer it over the PDF for anything you need to quote.
  - **WCH (`wch-ic.com`) — 2026-08-21: use the JSON API, never the `_PDF.html` URLs.** Two endpoints, both fine with a plain default User-Agent:
    1. `https://www.wch-ic.com/api/official/website/files/getFilesById?id=<N>` → JSON metadata: `name`, `version`, `uploadTime`, `size`, `content`, and crucially **`scope`**, a comma-separated list of every part number the document covers. Enumerate `N` over roughly **1–800**.
    2. `https://www.wch-ic.com/download/file?id=<N>` → the actual PDF. Validate `%PDF` magic.
    **Throttle to ≤ ~24 concurrent requests** — above that the API times out silently and you get partial enumeration (246/800 responses in one run).
  - **WCH publishes one document per _family_, named after the family's first member.** `CH445P` has no document of its own; it lives in `CH440DS1.PDF` (`scope` = "CH440G, CH440P, CH440R, CH442E, CH442Q, CH443K, CH443L, CH444G, CH444P, **CH445P**, CH448F, CH448U"). **Search the `scope` field, not document titles** — a part can be absent from every title and still be fully documented. This one behaviour caused a part to be recorded as "no datasheet exists" in an earlier pass.
  - **Some WCH PDFs are encrypted (owner password, empty user password).** `CH440DS1.PDF` and `CH9102DS1.PDF` use `/Filter /Standard /V 2 /R 3` (RC4-128); `CH340DS1.PDF` and `CH343DS1.PDF` use the newer AES-256 handler (`/Perms` present). Raw stream inflation gives nothing. `poppler-utils`' `pdftotext -layout` handles all four transparently — reach for it rather than writing a decryptor. **If `pdftotext` is unavailable** (confirmed absent on this machine 2026-08-24, along with `mutool`, `qpdf` and `gs`), the pure-Python fallback is `python3 -m venv … && pip install pypdf cryptography`. **`cryptography` is mandatory, not optional** — without it pypdf fails on the AES-256 files with `DependencyError: cryptography>=3.1 is required for AES algorithm`, and it fails at `PdfReader()` construction, which reads as "corrupt file" rather than "missing dependency".
  - **Re-verified 2026-08-24 (CH340 pass):** the API and download endpoints still work with a **plain default UA**. A full enumeration of ids **1–400 at 12-way `xargs -P 12`** completed with no silent truncation, consistent with the ≤ ~24 ceiling above. `id=79` = `CH340DS1.PDF` v3.4, uploaded 2025-03-12, `scope` = `CH340G, CH340C, CH340B, CH340N, CH340K, CH340E, CH340X, CH340T, CH340R`. Note the **version mismatch that is not an error**: the API reports version `3.4` while the PDF's own cover page reads `Version: 3D`. Both name the same file — do not "correct" a local filename on the strength of one or the other.
  - **Vendor *driver bundles* can be primary sources.** WCH's macOS `CH34xVCPDriver` embeds an `IOKitPersonalities` dictionary listing every VID/PID the vendor claims — better evidence than any web page. Extraction recipe (pure Python, no `hdiutil`): `bsdtar` the `.7z`; the `.dmg` is **UDZO**, so scan for `0x78 0x01/0x9c/0xda` zlib chunk starts and decompress each with `zlib.decompressobj()`, using `.unused_data` to locate the next chunk; then find `bplist00` markers in the concatenated output and brute-force `plistlib.loads()` over increasing end offsets. The sibling `.pkg` is a `xar` archive whose payload `bsdtar` rejects with "Decompressed size error" — **use the `.dmg`**. The same idea generalises: Windows `.inf` files, Linux kernel driver sources and macOS `Info.plist`s are all vendor-authored ID tables.
  - **Caveat on the +29 offset trick:** it is not as clean as recorded above. The shift is applied to letters, digits and punctuation but the extractor emits *some* characters unshifted, so shifted ranges collide with literal ones (a raw `3` is either the digit `3` or an encoded `P`). Mechanical decoding yields human-legible but machine-useless text like `bpmPO peries po` brrataK` for *"ESP32 Series SoC Errata."* Fine for reading an 8-page guide by eye; do not build a parser on it.

### Developer / media (all ✓)

github.com, news.ycombinator.com, substack.com, tumblr.com, bsky.app, joinmastodon.org instances, youtube.com, twitch.tv, soundcloud.com, flickr.com, imgur.com — worked with **every** UA tested. Treat these as safe defaults; if one of them starts failing, suspect rate limiting rather than UA blocking.
- Findings:
  - *(add findings here)*

## General retrieval tips

Tips for acquiring data are not strictly limited to user agents — whatever works when the normal path doesn't is worth documenting here.

- **Rate limiting:** if the first couple of downloads succeed and then requests start failing, don't switch strategies immediately — slow down, wait a bit, and retry a single download. If that works, update this doc: note that the site has a rate limiter, which UA eventually worked, and any inferred limits (requests per minute, seconds between downloads, burst size). Even a rough guess ("~10 docs/min then 429s") is useful for future sessions.
- **Mirrors and archives:** some sites move or delete resources, so a dead URL isn't always a block. If a URL is unreachable entirely (down, moved, deleted) — not just bot-blocked — someone may already have captured it. See *Finding copies of inaccessible URLs* below before giving up. GitHub repos/mirrors also often hold copies of documentation and datasets.
- **Finding copies of inaccessible URLs** (dead, moved, or blocked):
  - **Wayback capture list:** `https://web.archive.org/web/*/<url>*` shows the capture timeline in the UI. Programmatically use the availability API: `https://archive.org/wayback/available?url=<url>` (returns the closest snapshot as JSON).
  - **Wayback CDX API** — the power tool. Lists every capture with status codes, timestamps, and original URLs, supporting wildcards: e.g. `https://web.archive.org/cdx/search/cdx?url=example.com/docs/&matchType=prefix&filter=statuscode:200&collapse=urlkey&output=json`. Use it to find captures of moved paths (`matchType=prefix` over the old directory), to pick the best snapshot (200s only), or to discover sibling URLs you didn't know existed. Add `&limit=` and paginate.
  - **Other archives:** `archive.today` (mirrors at archive.ph etc.) often has captures Wayback lacks, especially JS-heavy pages; **Memento Aggregator** (`http://timetravel.mementoweb.org/api/json/<timestamp>/<url>`) searches many archives (Wayback, Arquivo, etc.) at once.
  - **Software Heritage** (`softwareheritage.org`) — archival store for source code; if a repo vanished, its origins/directory tree may be preserved there. Wayback rarely preserves whole repos well.
  - **Exact-string search:** take a distinctive phrase, error message, or function name from the content and search it verbatim (quoted) in a general search engine, plus `grep.app`, Sourcegraph, and `gh search code` / `gh api` (higher rate limit than anonymous). Unique strings find forks, mirrors, and cached copies that URL-based search misses.
  - **Alternate hosting of papers/PDFs:** search the exact title plus `filetype:pdf`; Google Scholar's "All versions" link surfaces author homepages, university mirrors, and preprint servers hosting the same document. Check **arXiv** (`arxiv.org`, plus `ar5iv.labs.arxiv.org` for HTML renderings) and **ResearchGate** first — authors frequently self-archive there, so they act as legitimate mirrors of record. Beyond those, large document-library sites such as Library Genesis and Anna's Archive maintain enormous libraries of papers and books; these documents were previously available online elsewhere, so such sites function as archives/mirrors of that public material and _may_ hold what you're looking for when the original host is gone.
  - **Redirect archaeology:** if a domain was restructured, historical DNS/WHOIS services (e.g. SecurityTrails) can reveal what a hostname pointed to previously; combined with CDX prefix queries this usually relocates moved content.
- **Searching GitHub:** GitHub's web/code search is weak for this, but you have better options: `git clone` repos locally and grep them yourself, or use the `gh` CLI (`gh search code ...`, `gh api ...`) which gets a higher, authenticated rate limit than anonymous API calls.
- **Document working paths:** there are pages you simply cannot fetch with plain curl but that work via some combination of UA + endpoint variant + pacing. When you find a working path, record it in the relevant *Sites* subsection (and the *User Agents* section if it's UA-specific).
- **Preserve scarce sources:** when a URL was genuinely useful and hard to acquire — few copies online, or hosted somewhere unlikely to persist — submit it to `https://web.archive.org/save/<url>` (unauthenticated, rate-limited to a few per minute; don't hammer). Worth archiving: datasheets/whitepaper/manual PDFs, technical docs on fragile personal/CMS-hosted pages, demo/example projects that exist only to illustrate how code works, hard-to-find header files or source snippets not in any repo, one-off benchmark posts. Not worth archiving: widely-mirrored content (Wikipedia, MDN, popular repos) or anything already covered elsewhere — you can't archive every page you visit.
  - **Rare case:** if you're confident a site is still up but blocked/rate-limited yourself, and you already know from another source that the URL is valuable, you may submit it unseen and record here that it wasn't directly accessible but was submitted on `<date>` — so future agents know to check `web.archive.org` for the capture. Only do this when there's a concrete reason the page matters.
- **Search-engine HTML endpoints are increasingly CAPTCHA-walled — 2026-08-21.** `https://html.duckduckgo.com/html/?q=…` and `https://lite.duckduckgo.com/lite/?q=…` both returned **HTTP 202** with an anti-bot challenge page ("Select all squares containing a duck") under a normal Chrome UA. Note the status is 202, not 403, so a naive `%{http_code}` check reports success; detect it by grepping the body for `challenge`/`Unfortunately, bots use DuckDuckGo too`. **When looking for a vendor's own document, skip search engines entirely and enumerate the vendor's index page** — that was faster and more complete than search would have been (see the Espressif entry under *Vendor documentation portals*).
- **Verify what you can't explain:** if a request fails in a way that doesn't look like bot-blocking (weird status codes, empty bodies, TLS errors), check whether it also fails for ordinary clients before blaming the UA.

## Sources

Official operator documentation:
- [OpenAI crawlers overview](https://developers.openai.com/api/docs/bots) — exact UA strings, version-change caveat, `robots.txt` marker, IP-range JSONs
- [Anthropic crawling docs](https://claude.com/crawling/bots.json) / ClaudeBot help article — UA strings and combined bots.json
- [Perplexity bots guide](https://docs.perplexity.ai/guides/bots) — PerplexityBot/Perplexity-User UAs and JSON endpoints
- [xAI Python SDK source](https://github.com/xai-org/xai-sdk-python/blob/main/src/xai_sdk/chat.py) — confirms `XaiImageApiFetch/1.0`
- Google: crawler verification via `googlebot.json` and reverse DNS ([Verify Googlebot](https://support.google.com/webmasters/answer/80591))

Aggregator / community lists:
- [ai.robots.txt](https://github.com/ai-robots-txt/ai.robots.txt) + [table-of-bot-metrics.md](https://github.com/ai-robots-txt/ai.robots.txt/blob/main/table-of-bot-metrics.md) — includes the unnamed "OpenAI" fetcher entries; sourced partly from Dark Visitors
- [Crawloria: four classes of AI bots](https://www.crawloria.com/blog/four-classes-of-ai-bots) — GPTBot/OAI-SearchBot/ChatGPT-User exact strings, IP-range files, robots.txt compliance classes
- [Paul Kinlan: ua-tracer](https://paul.kinlan.me/ua-tracer/) — behavioral comparison of GPTBot vs OAI-SearchBot vs ClaudeBot, IP verification notes
- [Cloudflare: Perplexity stealth crawlers](https://blog.cloudflare.com/perplexity-is-using-stealth-undeclared-crawlers-to-evade-website-no-crawl-directives) — undeclared Chrome-impersonation UA

Provenance studies with similar methodology:
- [Vidern: We Tested the World's Top 1,000 Websites](https://vidern.com/blog/top-1000-websites-ai-crawler-study)
- [HasData: The AI Crawler Block Index](https://hasdata.com/blog/ai-crawler-block-index)
- [agentmarkup: 500-company AI-crawler audit](https://agentmarkup.dev/blog/ai-crawler-audit-500-companies/)
