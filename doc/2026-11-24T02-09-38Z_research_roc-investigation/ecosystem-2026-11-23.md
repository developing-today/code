# The Roc Ecosystem — November 2026

**Report date:** 2026-11-23 · **Scope:** platforms, libraries/packages, the new (Zig) compiler, real projects, and the last ~6 months of movement.
**Method:** primary sources only where possible — [roc-lang.org](https://www.roc-lang.org), Richard Feldman's posts on [rtfeldman.com](https://rtfeldman.com), GitHub repos and release pages, and the community awesome list ([lukewilliamboswell/roc-awesome](https://github.com/lukewilliamboswell/roc-awesome)). All web sources were read on 2026-11-23. Aggregator/overview articles were checked against primary sources and, where they conflicted, discarded (see §7).

**Maturity labels used throughout:**

- **Official–active** — maintained by the roc-lang org, tracking current nightlies.
- **Community–active** — third-party, releasing against the new compiler in recent weeks.
- **Experimental / PoC** — real and runnable, but self-described as early or proof-of-concept.
- **Legacy / superseded** — built for the old Rust compiler; not on the current (new-compiler) awesome list; treat as abandoned unless re-verified.

---

## TL;DR

- Roc's compiler was rewritten from Rust to Zig. **Feature parity with the old compiler was reached in July 2026**, and the new compiler is now the only one that matters: both official platforms, the Exercism track, and essentially every active community project target it, pinned to dated nightlies.
- **Roc 0.1.0 — the language's first numbered release in eight years — had not shipped as of 2026-11-23.** Feldman (2026-10-22) said it was "still weeks away." Downstream projects and the nixpkgs update already talk about a "0.1.0 release candidate," so an RC is imminent or quietly circulating, but the [roc repo README](https://github.com/roc-lang/roc) still says Roc is "not ready for a 0.1 release yet." **Nightlies are the real distribution.**
- The platform layer is small but genuinely active: two official platforms (basic-cli, basic-webserver) shipping frequent releases, plus a burst of new community platforms in autumn 2026 — a Cloudflare Workers platform (Oct 2026), a native Android platform (created 2026-11-22), a GPUI-based desktop GUI, a raylib games platform with 51 releases, and a 3D engine.
- There is real production use, but it is concentrated: essentially one prolific consultant (Niclas Åhdén) running Roc + his Joy framework in production for ~2 years, plus one very large body of new-compiler application code by Steve Howell (a live group chat and ~33k lines of Roc). A pure-Roc deflate implementation runs in paid production within ~9% of libdeflate's speed.
- The language itself changed in the rewrite: effectful functions are suffixed `!`, `Result` became `Try`, methods (`list.map(...)`) landed, and new builtins (`Iter`/`Stream`) exist. Old-compiler code does not carry over unchanged.

---

## 1. The new compiler (Zig rewrite)

### What happened

- The rewrite of the compiler from Rust to Zig was announced in February 2025 and took **487 days**. The codebase went from ~300K lines of Rust to **464K lines of Zig**; the [roc repo](https://github.com/roc-lang/roc) is now ~92.9% Zig (6,207 stars, ~56K commits, 430 forks, read 2026-11-23).
- **Feature parity was reached in July 2026.** The primary source is Feldman's retrospective, ["How Our Rust-to-Zig Rewrite is Going"](https://rtfeldman.com/rust-to-zig) (2026-07-15), which is also the best single document on the rewrite's rationale and results.
- The legacy Rust compiler line ended at alpha1 (2025-01-29) through alpha4-rolling (2025-08-26) and is superseded. Distribution of the new compiler is via **nightlies** (`nightly-YYYY-MM-DD-<hash>`, [roc-lang/nightlies](https://github.com/roc-lang/nightlies)); the [setup-roc](https://github.com/roc-lang/setup-roc) GitHub Action supports `nightly-new-compiler`, and Nix users have the [roc-lang/roc-overlay](https://github.com/roc-lang/roc-overlay) flake. nixpkgs itself moved its roc package from alpha4 to `nightly-2026-11-09-9e85de7` in November 2026, noting a 0.1.0 release candidate was expected "in the coming weeks" ([maintainermode.org PR #564994](https://maintainermode.org/pr/564994)).

### What the new compiler does that the old one didn't

From [Feldman's rewrite retrospective](https://rtfeldman.com/rust-to-zig) and [roc-lang.org](https://www.roc-lang.org):

- **Build speed:** incremental builds went from 10.0s (Rust 1.85) / 3.4s (Rust 1.97) to **0.035s** on Zig 0.17 with `-fincremental`. Caveat: stable Zig 0.16.0 has a bug that breaks `-fincremental` on this codebase (fix landed upstream; the team is waiting on the next stable), and `-fincremental` is currently x86-64 only.
- **Hot code loading in dev:** `roc server.roc` picks up source edits per-request without a restart.
- **Reproducible cross-compilation:** `roc build --target=x64musl` produces byte-identical output across hosts.
- **Zero-parse caching:** `roc check` / `roc test` write an on-disk cache (structure-of-arrays layout, 32-bit indices) that is read back without parsing.
- **New internals:** a new parser (Anthony Bullard, Sam Mohr), a new type checker (Jared Ramirez), and new lambda-set resolution (Ayaz Hafiz, prototyped in OCaml — polymorphic defunctionalization).
- **Language-level additions:** string interpolation inside patterns; compile-time execution of pure functions (demonstrated as a zero-allocation HTTP router); an in-browser build of the compiler shipped as the "echo" platform (2.5 MB wasm, embedded on the roc-lang.org homepage, by Stephan).
- **Memory-safety retrospective:** the post counts 2 Zig-compiler use-after-free bugs found in the new compiler's error-message paths, versus 21 Rust-era memory-corruption reports — all of which were miscompilations, none in compiler logic. Feldman's conclusion is that the rewrite did not trade away safety in practice.

### Language changes in the new compiler

Per the current [roc-lang.org examples](https://www.roc-lang.org/examples) and release notes:

- Effectful functions are suffixed with `!` (`main!`, `echo!`); pure vs effectful is visible in every signature.
- `Result` is now **`Try`**.
- **Methods** exist: `list.map(...)`, `n.to_str()`.
- New builtins **`Iter`** and **`Stream`**; `for!` loops; local mutable variables via `var $x`.

### What's still missing / not done

- **0.1.0 itself.** Feldman's 2026-10-22 post ["How close can pure FP get to C?"](https://rtfeldman.com/fp-vs-c) says Roc is "still weeks away from its first numbered release, 0.1.0, eight years in the making." His talk ["A Preview of Roc 0.1.0"](https://hallway.video) (2026-08-13) previews it. Some downstream READMEs (e.g. roc-native-android, §2) already say "the 0.1.0 release candidate or a recent nightly," but no numbered release was present on the roc repo as of 2026-11-23.
- Documentation is the acknowledged gap: Feldman, in the rewrite retrospective's closing section: "in its current state you can still expect a variety of bugs, incomplete features, and unfinished docs. I have a lot of documentation to write between now and then!"
- The project is still fundraising for sustainability: the roc-lang.org homepage (read 2026-11-23) seeks **$4,000 USD/month** to fund one longtime contributor full-time. The Roc Programming Language Foundation is a 501(c)(3) nonprofit ([rtfeldman.com/rust-to-zig](https://rtfeldman.com/rust-to-zig)). Current corporate sponsors per the repo README: Lambda Class, ohne-makler, Decem (earlier sponsors named in Feldman's post: rwx, Vendr, NoRedInk, Tweede Golf, Martian).

### Who has moved to the new compiler

- **Both official platforms** (basic-cli, basic-webserver) — new-compiler only, tracking nightlies within days (§2).
- **The Exercism Roc track** — fully migrated August 2026 (110 exercises; maintainer Aurélien Geron hand-updated 108 beginner exercises; [Exercism forum thread 68377](https://forum.exercism.org), [track](https://exercism.org/tracks/roc)).
- **Essentially the entire current awesome list** — every platform and package in §2–§3 that is labelled Community–active pins a November 2026 nightly.
- **Rocci Bird** (the WASM-4 game, §4) was ported as the feature-parity proof.
- Old-compiler projects that did not migrate are listed as Legacy/superseded in §2–§3.

---

## 2. Platforms

A Roc *platform* is the host program (in Rust, Zig, Go, …) that provides an app its effects — files, network, UI. Platforms are where most ecosystem action is.

### Official (roc-lang org)

| Platform | What it's for | Status |
|---|---|---|
| **[basic-cli](https://github.com/roc-lang/basic-cli)** | Command-line apps: the default general-purpose platform. Host in Rust. Modules include Stdout/Stdin (streaming lines and chunks), File/Path (writer API, permissions such as OwnerOnly/PublicRead, symlinks), Cmd, Http, Tcp, UnixSocket, Sqlite, Tty (raw mode), Signal (SIGINT/SIGTERM/SIGUSR1/SIGUSR2/SIGWINCH), Env, Locale. | **Official–active.** Latest **0.28.1**, requiring nightly-2026-11-13 or newer. 0.26.0 was a large consistency release (typed `Duration`/`Utc`/`Monotonic`, unified `Err(EndOfInput)`, one error tag per module, mimalloc on Linux static hosts). Maintained mainly by Luke Boswell, with release automation by Anton-4. [Releases](https://github.com/roc-lang/basic-cli/releases) |
| **[basic-webserver](https://github.com/roc-lang/basic-webserver)** | HTTP servers. Host in Rust (hyper + tokio). Ships Sqlite, server-sent events, HTML/forms/static-file support. | **Official–active, very fast-moving.** Latest **0.22.0**, pinning nightly-2026-11-15. It shipped 0.17 → 0.22 across roughly six weeks in Oct–Nov 2026. [Releases](https://github.com/roc-lang/basic-webserver/releases) |

### Community — new compiler

| Platform | What it's for | Status |
|---|---|---|
| **[RocRay / roc-ray](https://github.com/lukewilliamboswell/roc-ray)** (Luke Boswell) | 2D games and visual tools over raylib; Zig host. Drawing, audio, keyboard/mouse/gamepad, files, networking, screen recording; macOS/Linux/Windows x64. Explicitly 2D-only (3D planned) and not a GUI toolkit. Ships real example programs: Breakout, Pong, Snake, Pixel Workshop, Spark Run, Cave Climb, Particles, Live Plot, Postcard Studio. | **Community–active.** 80 stars, 1,579 commits, **51 releases** (created 2024-01-13). The roc-lang.org example pins release **0.12.1** (nightly-2026-11-12); the [manual](https://lukewilliamboswell.github.io/roc-ray/roc-ray.pdf) is at 0.12.3. |
| **[roc-gui](https://github.com/lukewilliamboswell/roc-gui)** (Luke Boswell) | Native desktop GUI on **GPUI** (Zed's UI framework); cross-platform (macOS, Linux/Wayland, Windows). Ships a behavioural/performance test harness and a performance observatory streaming to SQLite; the repo gallery shows a dozen demo apps (HTTP Workbench, Database Browser, Terminal Workspace, Redis Explorer, …). | **Experimental, but substantial.** Created 2026-09-14; 733 commits, 25 releases in ~10 weeks. README: "still an early-stage platform. Do not use it in production." |
| **[roc-signals](https://github.com/rtfeldman/roc-signals)** (Richard Feldman) | Reactive-signals UI engine, for the web and for native GUI. Its native-GUI docs ship starter archives as release candidates (`gui-0.1.0-rc.1/2/3` per OS) pinned to nightly-2026-09-04 ([docs](https://github.com/rtfeldman/roc-signals/blob/HEAD/www/content/docs/native-gui.md)). The same signals engine powers roc-native-android (below). | **Experimental–active.** Feldman's own current UI project; the engine is at 0.8.0-rc1 per the awesome list. |
| **[roc-native-android](https://github.com/lukewilliamboswell/roc-native-android)** (Luke Boswell) | Native Android apps in pure Roc — real Android widgets, **no WebView**. Reactive engine from roc-signals; arm64 only; Kotlin shell, no Gradle. Examples: counter, todos, stopwatch, contacts (a virtualised 10,000-contact list), http, and "ports" (battery/accelerometer exposed from Kotlin). | **Experimental / PoC — brand new.** Created **2026-11-22** (the day before this report); v0.0.1, 20 commits; self-described "early proof of concept." Luke's launch post, "Roc platform for native Android apps," is dated 23 Nov 2026 on [lukewilliamboswell.github.io](https://lukewilliamboswell.github.io). Its README already references "the 0.1.0 release candidate or a recent nightly." |
| **[roc-native-macos](https://github.com/lukewilliamboswell/roc-native-macos)** (Luke Boswell) | Native macOS apps. | **Experimental.** Listed on the current awesome list; not separately audited for this report. |
| **[Joy](https://github.com/niclas-ahden/joy)** (Niclas Åhdén) | Full-stack / isomorphic web framework + platform: Elm-architecture client and server in one language. | **Community–active; the ecosystem's most battle-tested framework.** 66 stars, 256 commits, 5 releases (created 2024-12-10); current README pins Joy **0.35.0** + joy-html 0.18.0. README self-label: "early development, not production-ready… expect breaking changes" — yet its author runs it in production (§4), and its js-framework-benchmark numbers are competitive with Elm/Leptos/Solid. |
| **[roc-cloudflare-workers](https://github.com/Karakatiza666/roc-cloudflare-workers)** (Karakatiza666) | Roc on **Cloudflare Workers**: apps compile to a wasm32 reactor behind a JS shim. Typed modules for fetch, D1, R2, KV, Cache API, Durable Object SQL, Queues, and Workers AI; Zig host; effects await JS promises via JSPI. Companion CLI: `roc-cloudflare-workers-cli`. | **Experimental — brand new and solo.** Created **2026-10-26**; already at **v0.0.10** (10 releases in under a month, 216 commits, 0 stars). |
| **[roc-tower-platform](https://github.com/grayrest/roc-tower-platform)** (grayrest) | Web servers with pure handlers on Rust hyper/Tower + SQLite. | **Community–active** (current awesome list). |
| **[basic-ssg](https://github.com/lukewilliamboswell/basic-ssg)** (Luke Boswell) | Static site generation. | **Community–active** (current awesome list). |
| **[roc-fuzz](https://github.com/lukewilliamboswell/roc-fuzz)** (Luke Boswell; originally bhansconnect) | Fuzz-testing platform. | **Community–active** (current awesome list). |
| **[roc-wasm4](https://github.com/lukewilliamboswell/roc-wasm4)** (Luke Boswell) | Games for the WASM-4 fantasy console. Its game Rocci Bird was the new compiler's feature-parity demo (§4). | **Community–active** (current awesome list). |
| **[Rocco](https://github.com/BraedenFoster/rocco)** (Braeden Foster) | Experimental **3D engine** platform: engine in Odin, WebGPU rendering; a game is a model plus five pure functions producing a `Scene` value. glTF, PBR, shadows, bloom; macOS/Linux/Windows/web builds. | **Experimental.** (Via Steve Howell's site, §4, read 2026-11-23.) |
| **[roc-screensaver](https://github.com/showell/roc-screensaver)** (Steve Howell) | Zig platform with its own software rasterizer: one Roc app runs both as a Windows screensaver and as a web page. Three screensavers ship on it, including "Lanterns, a sunrise made for Roc's 0.1.0 launch"; ~8,155 lines of Roc apps. | **Experimental–active** (single-author, shipping). |
| **[restricted-roc](https://codeberg.org/removewingman/restricted-roc)** (removewingman, Codeberg) | CLI platform that can sandbox itself — restricts syscalls and filesystem access. | **Experimental** (current awesome list). |
| **Platform templates** (Luke Boswell) | Scaffolding for writing new platforms in [Go](https://github.com/lukewilliamboswell/roc-platform-template-go), [Rust](https://github.com/lukewilliamboswell/roc-platform-template-rust), [Zig](https://github.com/lukewilliamboswell/roc-platform-template-zig). | **Community–active.** Worth noting: the "write your own platform" path is a supported, templated workflow, which is how the long tail above exists at all. |

### Legacy-compiler platforms (not on the current awesome list)

Treat as **legacy / superseded** unless re-verified: roc-aws-lambda and roc-aws-lambda-runtime (isaacvando), roc-iced (tarkah), roc-microbit (bhansconnect, embedded), static-site-gen (isaacvando), roc-ecs (ageron), roc-parallel demo (ageron). Aurélien Geron (ageron) also maintains **roclet** (reactive Roc notebooks in the browser) and new-compiler-updated forks of basic-cli/basic-webserver (updated 2026-10-05).

---

## 3. Libraries and packages

Roc packages are distributed as bundles via GitHub releases (there is no central registry like crates.io). The list below is the **new-compiler** canon per the current [roc-awesome](https://github.com/lukewilliamboswell/roc-awesome) README (read 2026-11-23).

### Parsing and text

- **[roc-parser](https://github.com/lukewilliamboswell/roc-parser)** (Luke Boswell) — parser combinators plus ready-made parsers for CSV, JSON, YAML, XML, Markdown, AsciiDoc, HTTP, and Roc source itself. The ecosystem's flagship parsing package.
- **[roc-regex](https://github.com/grayrest/roc-regex)** (grayrest) — regular expressions.
- **[roc-ascii](https://github.com/hasnep/roc-ascii)** (Hasnep) — ASCII text handling.
- **[unicode](https://github.com/roc-lang/unicode)** (roc-lang) — graphemes and code points.
- **[path](https://github.com/roc-lang/path)** (roc-lang) — path types.
- **[http](https://github.com/roc-lang/http)** (roc-lang) — HTTP client types, v2.0.0.

### Data and serialisation (mostly Karakatiza666)

- **[roc-cbor](https://github.com/Karakatiza666/roc-cbor)** — CBOR with derived readers/writers (0.5.0).
- **[roc-toml](https://github.com/Karakatiza666/roc-toml)** — TOML 1.0 with comment-preserving edits.
- **[roc-jwt](https://github.com/Karakatiza666/roc-jwt)** — JWT decode + claims (bring-your-own crypto).
- **[roc-semver](https://github.com/Karakatiza666/roc-semver)**, **[roc-tar](https://github.com/Karakatiza666/roc-tar)**, **[roc-base58](https://github.com/Karakatiza666/roc-base58)**, **[roc-s2-geometry](https://github.com/Karakatiza666/roc-s2-geometry)** (port of Google's S2 geometry), **[roc-websocket](https://github.com/Karakatiza666/roc-websocket)** (RFC 6455, platform-agnostic), **[roc-app-header](https://github.com/Karakatiza666/roc-app-header)** (parse/edit Roc app headers, MVS version selection).
- **[roc-rvn](https://git.sr.ht/~jwoudenberg/roc-rvn)** (Jasper Woudenberg, SourceHut) — a Roc-like serialisation format ("Roc Value Notation").

### Dates and time

- **[roc-time](https://github.com/lukewilliamboswell/roc-time)** (Luke Boswell) — date/time, 0.3 line, microsecond profile.
- **[roc-isodate](https://github.com/imclerran/roc-isodate)** (imclerran) — ISO date parsing/formatting.
- **[roc-gregorian](https://github.com/jwoudenberg/roc-gregorian)** (Jasper Woudenberg) — Gregorian calendar math.

### Databases and network services

- **[roc-pg](https://github.com/niclas-ahden/roc-pg)** (Niclas Åhdén) — PostgreSQL client with **compile-time checked SQL**. (Supersedes the old-compiler agu-z/roc-pg.)
- **[roc-redis](https://github.com/jaredramirez/roc-redis)** (Jared Ramirez) — Redis client, binary-safe, platform-agnostic.

### The Åhdén suite (Niclas Åhdén — all new-compiler, production-driven)

One author accounts for a large share of the serious package ecosystem, written for his production work (§4): [roc-deflate](https://github.com/niclas-ahden/roc-deflate) (§4), roc-base64, roc-crc32, roc-md5, roc-hmac, roc-scram, [roc-openapi](https://github.com/niclas-ahden/roc-openapi) (validate request bodies against OpenAPI 3.x), [roc-playwright](https://github.com/niclas-ahden/roc-playwright) (browser automation), **[roc-spec](https://github.com/niclas-ahden/roc-spec)** (parallel test runner with isolated environments), **[roc-vcr](https://github.com/niclas-ahden/roc-vcr)** (record/replay HTTP in tests), roc-sendgrid, roc-url, roc-xlsx, roc-zip, roc-xml, roc-prng, roc-database-url, roc-human-readable-file-size, roc-maybe, roc-non-zero, joy-html.

### The Boswell suite (Luke Boswell)

- **[weaver](https://github.com/lukewilliamboswell/weaver)** — ergonomic CLI argument parser (originally by smores56).
- **[roc-docx](https://github.com/lukewilliamboswell/roc-docx)** — read/write Word documents, pure Roc.
- **[roc-font](https://github.com/lukewilliamboswell/roc-font)** — TTF/OTF parsing and text shaping, pure Roc.
- **[roc-graph-layout](https://github.com/lukewilliamboswell/roc-graph-layout)** — graph layout plus Mermaid parsing/rendering.
- **[roc-pdf](https://github.com/lukewilliamboswell/roc-pdf)**, **[roc-pandoc](https://github.com/lukewilliamboswell/roc-pandoc)**, roc-ansi, **[plume](https://github.com/lukewilliamboswell/plume)** (interactive charts via Plotly.js), roc-time (above).

### Other packages

- **[roc-random](https://github.com/kili-ilo/roc-random)** (kili-ilo) — random generation (successor to jancvanb/roc-random).
- **[rocsat](https://github.com/mpizenberg/rocsat)** (Matthieu Pizenberg) — a SAT solver in Roc.
- **[terrocotta](https://github.com/obust/terrocotta)** (obust) — GUI library with Clay-style layout + MVU, rendering via roc-ray.

### Tooling

- **[roc-syntax-mcp](https://github.com/Karakatiza666/roc-syntax-mcp)** (Karakatiza666, on npm) — an MCP server that teaches coding agents Roc: it indexes the language reference, builtins, basic-cli, basic-webserver, roc-parser, roc-random, and third-party platforms such as roc-ray; every topic app passes `roc check` against its pinned release. A sign of the times: the ecosystem is already building tooling *for AI agents* writing Roc.
- **[tree-sitter-roc](https://github.com/faldor20/tree-sitter-roc)** (faldor20) — grammar updated for the new syntax; powers Helix/Neovim.
- **[zed-roc](https://github.com/h2000/zed-roc)**, **[roc-vscode](https://github.com/roc-lang/roc-vscode)** — editor support.
- **[setup-roc](https://github.com/roc-lang/setup-roc)** (roc-lang) — GitHub Action; **[release-package](https://github.com/roc-lang/release-package)** (roc-lang) — package publishing workflow.
- **[bundle-roc-library](https://github.com/hasnep/bundle-roc-library)** + **pre-commit-roc** (Hasnep); **[roc-just](https://github.com/grayrest/roc-just)** (just task-runner port); **[roc-blueprint](https://github.com/lukewilliamboswell/roc-blueprint)** (typed dev-environment/task declarations in Roc — roc-gui's build is driven by it); **[roc2nix](https://github.com/jrmurr/roc2nix)**; **[roc-start](https://github.com/imclerran/roc-start)** (scaffolder; older listing, status unverified).

### Superseded / abandoned (old-compiler era, absent from the current list)

lukewilliamboswell/roc-json (built-in JSON plus roc-parser cover this now), Hasnep/roc-html (joy-html is the live one), Hasnep/roc-math, quelgar/roc-utils, isaacvando/rtl (template language), isaacvando/gob (a stack language written in Roc).

---

## 4. Real projects

Honest scale labels: **Production** = someone depends on it / it earns money · **Substantial hobby** = large, finished, used by its author/community, not commercial · **Demo/toy** = exists to demonstrate the language or a platform.

### Production

- **Niclas Åhdén's production Roc (Joy framework)** — *Production, small-team/consulting scale.* Åhdén (instateam.se) has used Roc in production for ~2 years; Feldman calls him "Roc's most prolific production user." His Func Prog Sweden talk (2026-11-10), "Roc and Joy: Productivity in Production," states Joy "already powers real web apps in the wild." His package suite (§3) is the by-product of that work. Sources: [rtfeldman.com/fp-vs-c](https://rtfeldman.com/fp-vs-c), [github.com/niclas-ahden](https://github.com/niclas-ahden).
- **roc-deflate** — *Production, single-package.* Åhdén's pure-Roc DEFLATE implementation runs in **paid production**. Feldman spent weeks optimising it and benchmarked it against libdeflate over 144 Silesia-corpus runs (levels L1–L12): median **8.9% slower than libdeflate on a Ryzen 9 3900X, 2.6% slower on M1**, worst case 18% slower, best case ~18% faster, byte-identical output. Honest caveats from the same post: roc-deflate's APIs are non-streaming only, and one heavily optimised package is not the general case. Source: [rtfeldman.com/fp-vs-c](https://rtfeldman.com/fp-vs-c) (2026-10-22).

### Substantial hobby / community infrastructure

- **Steve Howell's [roc.lynrummy.com](https://roc.lynrummy.com) portfolio** — *Substantial hobby, production-adjacent.* The largest public body of new-compiler Roc application code found in this investigation, all UPL-1.0 and, by the author's own account, built "through agentic development, with Claude"; the apps also run in-browser as wasm.
  - **roc-chat** — a live production group chat at chat.lynrummy.com that replaced the group's earlier Zig chat: **~33,000 lines of Roc in 171 modules** (+ ~10,000 lines in 45 spec modules, ~2,300 `expect`s) on a custom Zig-host platform (29 hosted functions, the filesystem as the database, SSE live updates, password/GitHub/device-code sign-in; AI agents are chat members with their own keys).
  - **roc-markdown** — 5,780 lines; "finished… in production"; renders every roc-chat message; held to 314 frozen CommonMark/GFM spec examples.
  - **roc-syntax-color** — 5,242 lines; syntax highlighting for 7 languages.
  - **Safari Ride** — 14,268 lines, 98 modules, 816 expects; plus a 39-chapter online book, *Safari Ride: a tour of a real Roc program* (early draft).
  - **A BASIC interpreter** — 6,733 lines; ECMA-55 Minimal BASIC plus the 1978 Creative Computing dialect; graded against 208 NBS test programs and 99 games from *BASIC Computer Games*.
  - **Roc on Shale** — 25 Roc programs, ~25,700 lines: code-check/report scripts for his Shale agent scripting language.
  - **Seattle Delivery** (4,704 lines; vehicle-routing solver, Clarke–Wright + local search, bit-for-bit reproducible native vs wasm), **Fast Track** (4,166 lines; board game with search AI), **Dog Park** (3,688 lines; herding game), **Safari Walk** (1,666 lines; built on the Rocco 3D engine), a shared 1,439-line chess kit behind Knight's Tour / Eight Queens, the roc-screensaver platform (§2), and animated shorts (*Trick or Treat*, *Welcome to Roc*, *Robot Thanksgiving*).

### Demos and smaller projects

- **Rocci Bird** (bhansconnect, 2024; art by Luke DeVault) — *Demo, but a load-bearing one.* A WASM-4 Flappy-Bird-like, ported to the new compiler as the feature-parity proof: `roc build --opt=size` produces a **31 KB wasm**, less than half the old compiler's output. Playable on itch.io and embedded in Feldman's rewrite post. Source: [rtfeldman.com/rust-to-zig](https://rtfeldman.com/rust-to-zig).
- **roc-ray example games/tools** (with the platform, §2) — Breakout, Spark Run, Cave Climb, Pixel Workshop, Particles, Live Plot, Postcard Studio. *Small-but-complete.*
- **kai** ([thebrandonlucas/kai](https://github.com/thebrandonlucas/kai)) — a CLI frontend for Nix, listed as a showcase app on the awesome list.
- **roc-gui demo apps** (§2) — a dozen GPUI demo applications (HTTP Workbench, Database Browser, Redis Explorer, …) shipped as the platform's gallery.

### Learning / puzzle scenes

- **Exercism Roc track** — 110 exercises, fully migrated to the new compiler in August 2026. This is the main on-ramp for new users and a real signal of new-compiler adoption. [exercism.org/tracks/roc](https://exercism.org/tracks/roc)
- **Advent of Code** — [lukewilliamboswell/aoc](https://github.com/lukewilliamboswell/aoc) (puzzle solutions) is on the current awesome list's showcases. Roc has an AoC presence through individual solvers rather than an organised scene.
- **Game jams** — no distinct Roc game-jam institution was found in this pass; the games energy is concentrated in roc-ray / roc-wasm4 / Rocco examples instead. (Stated plainly rather than padded.)
- **Community hub** — the Roc Zulip is the project's centre of gravity (linked from every official page); a public export is browsable at [roc-lang.github.io/zulip-export](https://roc-lang.github.io/zulip-export). Announcements there were corroborated against the web sources above rather than scraped for this report.

---

## 5. What's new in the last ~6 months (May – Nov 2026)

| When | What |
|---|---|
| 2026-07-15 | **Feature parity** for the Zig rewrite + Feldman's retrospective, ["How Our Rust-to-Zig Rewrite is Going"](https://rtfeldman.com/rust-to-zig). |
| Aug 2026 | **Exercism Roc track fully migrated** to the new compiler (110 exercises). |
| 2026-08-13 | Feldman's talk **"A Preview of Roc 0.1.0"** ([hallway.video](https://hallway.video)). |
| 2026-09-14 | **roc-gui** created (GPUI desktop GUI platform); 25 releases by late November. |
| Oct–Nov 2026 | **basic-cli 0.26 → 0.28.1** and **basic-webserver 0.17 → 0.22.0** release trains, tracking November nightlies. |
| 2026-10-22 | ["How close can pure FP get to C?"](https://rtfeldman.com/fp-vs-c) — the roc-deflate vs libdeflate result; "weeks away" from 0.1.0. |
| 2026-10-26 | **roc-cloudflare-workers** platform created; iterated to v0.0.10 within a month. |
| Nov 2026 | **nixpkgs** moves roc from alpha4 to a new-compiler nightly; roc-lang.org's examples section rebuilt around the new compiler (examples validated against nightlies of 2026-11-12/15, including a terminal Snake on basic-cli 0.28.0 and a RocRay example). |
| 2026-11-10 | Åhdén's Func Prog Sweden talk, "Roc and Joy: Productivity in Production." |
| 2026-11-22/23 | **roc-native-android** created (2026-11-22) and announced (2026-11-23). |
| Ongoing | **roc-syntax-mcp** — agent-facing tooling for the new compiler; Steve Howell's entire lynrummy portfolio is a 2026, agent-developed body of work. |

---

## 6. Exists / New / Actually used — the separation Drew asked for

- **Exists (and is current):** the new compiler (nightlies), basic-cli, basic-webserver, roc-ray, Joy, roc-parser, roc-pg, roc-signals, the Åhdén and Boswell package suites, Exercism track.
- **New (last ~6 months):** feature parity itself; roc-gui (Sep 2026); roc-cloudflare-workers (Oct 2026); roc-native-android (Nov 2026); the basic-webserver 0.17→0.22 velocity; roc-syntax-mcp; language changes (`Try`, methods, `Iter`/`Stream`) becoming the documented norm.
- **Actually used (evidence of real use, not just existence):** roc-deflate in paid production; Joy powering production web apps for ~2 years; roc-chat as a live group chat; Exercism learners on 110 exercises; roc-ray's shipped example tools; Howell's Shale tooling scripts (25 programs used on his own project).
- **Exists but legacy:** everything in the "Legacy/superseded" lists in §2–§3 — old-compiler code that has not been ported and should not be cited as current ecosystem.

---

## 7. Caveats and source quality

- **Pre-release language.** Everything above sits on nightly compilers. Pins move weekly; a package that is "current" today names a specific November 2026 nightly in its header. Expect breakage until 0.1.0 lands.
- **Small numbers, honestly.** The ecosystem's centre of mass is a handful of people: Feldman (compiler, roc-signals), Luke Boswell (basic-cli, roc-ray, roc-gui, the native platforms, roc-parser, the awesome list itself), Niclas Åhdén (Joy + the production package suite), Karakatiza666 (data formats + Cloudflare), Steve Howell (applications). Bus factor is the ecosystem's biggest risk, and the $4k/month fundraising goal (§1) is the project's own acknowledgement of it.
- **Aggregator articles are unreliable here.** A July 2026 overview on asopi.tech, for example, cites platform versions (basic-cli 0.20.0 / basic-webserver 0.13.1) that were already stale against the release pages, and a "basic-graphics" platform appears in dev.to-era material but in no current list. Every version number in this report comes from the project's own release page or README as read on 2026-11-23.
- **Zulip** is where announcements happen first; this report corroborated them via the public web (repos, blogs, talks) and the [public Zulip export](https://roc-lang.github.io/zulip-export) exists for anyone who wants the primary feed.

---

*Compiled 2026-11-23 from primary sources read the same day.*
