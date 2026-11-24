# Libraries and packages

Roc packages are distributed as bundles via GitHub releases — there is no
central registry like crates.io. The list below is the **new-compiler** canon
per the current [roc-awesome](https://github.com/lukewilliamboswell/roc-awesome)
README (read 2026-11-23), plus releases announced in the November Zulip
window.

## Parsing and text

- **[roc-parser](https://github.com/lukewilliamboswell/roc-parser)** (Luke Boswell) — parser combinators plus ready-made parsers for CSV, JSON, YAML, XML, Markdown, AsciiDoc, HTTP, and Roc source itself. The ecosystem's flagship parsing package.
- **[roc-regex](https://github.com/grayrest/roc-regex)** (grayrest) — regular expressions; described in November as "competitive with Rust's regex crate" on wide-SIMD machines.
- **[roc-ascii](https://github.com/hasnep/roc-ascii)** (Hasnep) — ASCII text handling.
- **[unicode](https://github.com/roc-lang/unicode)** (roc-lang) — graphemes and code points.
- **[path](https://github.com/roc-lang/path)** (roc-lang) — path types.
- **[http](https://github.com/roc-lang/http)** (roc-lang) — HTTP client types, v2.0.0.

## Data and serialisation (mostly Karakatiza666)

- **[roc-cbor](https://github.com/Karakatiza666/roc-cbor)** — CBOR with derived readers/writers (0.5.0).
- **[roc-toml](https://github.com/Karakatiza666/roc-toml)** — TOML 1.0 with comment-preserving edits.
- **[roc-jwt](https://github.com/Karakatiza666/roc-jwt)** — JWT decode + claims (bring-your-own crypto).
- **[roc-semver](https://github.com/Karakatiza666/roc-semver)**, **[roc-tar](https://github.com/Karakatiza666/roc-tar)**, **[roc-base58](https://github.com/Karakatiza666/roc-base58)**, **[roc-s2-geometry](https://github.com/Karakatiza666/roc-s2-geometry)** (port of Google's S2 geometry), **[roc-websocket](https://github.com/Karakatiza666/roc-websocket)** (RFC 6455, platform-agnostic), **[roc-app-header](https://github.com/Karakatiza666/roc-app-header)** (parse/edit Roc app headers, MVS version selection).
- **[roc-rvn](https://git.sr.ht/~jwoudenberg/roc-rvn)** (Jasper Woudenberg, SourceHut) — a Roc-like serialisation format ("Roc Value Notation").
- Note: two independent packages both named **roc-markdown** exist (Karl's and Steve Howell's) — flagged in-chat as a possible confusion.

## Dates and time

- **[roc-time](https://github.com/lukewilliamboswell/roc-time)** (Luke Boswell) — date/time, 0.3 line, microsecond profile.
- **[roc-isodate](https://github.com/imclerran/roc-isodate)** (imclerran) — ISO date parsing/formatting; 0.8.0 released in November 2026.
- **[roc-gregorian](https://github.com/jwoudenberg/roc-gregorian)** (Jasper Woudenberg) — Gregorian calendar math.

## Databases and network services

- **[roc-pg](https://github.com/niclas-ahden/roc-pg)** (Niclas Åhdén) — PostgreSQL client with **compile-time checked SQL**. (Supersedes the old-compiler agu-z/roc-pg.) Bryce Miller's Tigerbeetle client and roc-pg were among the packages confirmed working on RC1.

## The Åhdén suite (Niclas Åhdén — all new-compiler, production-driven)

One author accounts for a large share of the serious package ecosystem,
written for his production work (see the projects page):
[roc-deflate](https://github.com/niclas-ahden/roc-deflate), roc-base64,
roc-crc32, roc-md5, roc-hmac, roc-scram,
[roc-openapi](https://github.com/niclas-ahden/roc-openapi) (validate request
bodies against OpenAPI 3.x),
[roc-playwright](https://github.com/niclas-ahden/roc-playwright) (browser
automation; 0.13.0 in November),
**[roc-spec](https://github.com/niclas-ahden/roc-spec)** (parallel test runner
with isolated environments),
**[roc-vcr](https://github.com/niclas-ahden/roc-vcr)** (record/replay HTTP in
tests), roc-sendgrid, roc-url, roc-xlsx, roc-zip, roc-xml, roc-prng,
roc-database-url, roc-human-readable-file-size, roc-maybe, roc-non-zero,
joy-html.

## The Boswell suite (Luke Boswell)

- **[weaver](https://github.com/lukewilliamboswell/weaver)** — ergonomic CLI argument parser (originally by smores56).
- **[roc-docx](https://github.com/lukewilliamboswell/roc-docx)** — read/write Word documents, pure Roc; 0.1.0-rc1 in November.
- **[roc-pdf](https://github.com/lukewilliamboswell/roc-pdf)** — 0.1.0 in November, and **in production**: Niclas Åhdén generates real-estate record-sale flyers with it.
- **[roc-font](https://github.com/lukewilliamboswell/roc-font)** — TTF/OTF parsing and text shaping, pure Roc.
- **[roc-graph-layout](https://github.com/lukewilliamboswell/roc-graph-layout)** — graph layout plus Mermaid parsing/rendering; 0.3.0-rc3 in November.
- **[roc-pandoc](https://github.com/lukewilliamboswell/roc-pandoc)**, roc-ansi, **[plume](https://github.com/lukewilliamboswell/plume)** (interactive charts via Plotly.js; 0.5.0 ported to the modern compiler, then 0.6.0, in November), roc-time (above).

## Other packages

- **[roc-random](https://github.com/kili-ilo/roc-random)** (kili-ilo) — random generation (successor to jancvanb/roc-random).
- **[rocsat](https://github.com/mpizenberg/rocsat)** (Matthieu Pizenberg) — a SAT solver in Roc.
- **[terrocotta](https://github.com/obust/terrocotta)** (obust) — GUI library with Clay-style layout + MVU, rendering via roc-ray.
- **[roc-syntax-color](https://github.com/showell/roc-syntax-color)** (Steve Howell) — 5,242 lines; syntax highlighting for 7 languages.

## Tooling

- **[roc-syntax-mcp](https://github.com/Karakatiza666/roc-syntax-mcp)** (Karakatiza666, on npm) — an MCP server that teaches coding agents Roc: it indexes the language reference, builtins, basic-cli, basic-webserver, roc-parser, roc-random, and third-party platforms such as roc-ray; every topic app passes `roc check` against its pinned release. A sign of the times: the ecosystem is already building tooling *for AI agents* writing Roc.
- **[tree-sitter-roc](https://github.com/faldor20/tree-sitter-roc)** (faldor20) — grammar updated for the new syntax; powers Helix/Neovim.
- **[zed-roc](https://github.com/h2000/zed-roc)**, **[roc-vscode](https://github.com/roc-lang/roc-vscode)** — editor support. The **official VS Code extension** was announced Nov 5 (see the digest page): syntax highlighting (including Roc in doc comments/Markdown), diagnostics as you type, formatting, hover with inferred types.
- **[setup-roc](https://github.com/roc-lang/setup-roc)** (roc-lang) — GitHub Action; **[release-package](https://github.com/roc-lang/release-package)** (roc-lang) — package publishing workflow.
- **[bundle-roc-library](https://github.com/hasnep/bundle-roc-library)** + **pre-commit-roc** (Hasnep); **[roc-just](https://github.com/grayrest/roc-just)** (just task-runner port); **[roc-blueprint](https://github.com/lukewilliamboswell/roc-blueprint)** (typed dev-environment/task declarations in Roc — roc-gui's build is driven by it); **[roc2nix](https://github.com/jrmurr/roc2nix)**; **[roc-start](https://github.com/imclerran/roc-start)** (scaffolder; older listing, status unverified).

## Superseded / abandoned (old-compiler era, absent from the current list)

lukewilliamboswell/roc-json (built-in JSON plus roc-parser cover this now),
Hasnep/roc-html (joy-html is the live one), Hasnep/roc-math,
quelgar/roc-utils, isaacvando/rtl (template language), isaacvando/gob (a stack
language written in Roc).
