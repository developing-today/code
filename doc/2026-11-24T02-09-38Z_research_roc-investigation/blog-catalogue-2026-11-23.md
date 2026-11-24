# Roc — Blog & News Catalogue

**Compiled:** 2026-11-23 · **Lane:** ROC-BLOG · **Companion to:** `ecosystem-2026-11-23.md`, `talks-and-announcements.md`, `zulip-digest-2026-11-23.md` (all same-day; this file flags deltas against them rather than repeating them).

**The single most important structural fact:** **roc-lang.org has no dated blog.** The site
(inventory verified against its source repo, [roc-lang/www.roc-lang.org](https://github.com/roc-lang/www.roc-lang.org),
`website/content/`) consists of undated, living design pages. Roc's dated "blog" voice lives in
three other places: Richard Feldman's personal blog ([rtfeldman.com](https://rtfeldman.com), 4 posts),
Luke Boswell's personal blog ([lukewilliamboswell.github.io](https://lukewilliamboswell.github.io), 8 posts),
and — for actual announcements — the Roc Zulip and GitHub release pages. **The 0.1.0-RC1 release
(2026-11-21) was announced only on Zulip; no website or blog post about it exists yet** (a community
showcase for the eventual 0.1.0 announcement post was being assembled in #show and tell as of 2026-11-23).

**Counts:** 12 dated blog posts (4 Feldman + 8 Boswell) · 11 roc-lang.org content pages (incl. an
18-question FAQ) · talks/podcasts consolidated in §5 · third-party posts in §6.

Kind tags: `release` · `compiler internals` · `language design` · `ecosystem` · `community` · `funding`.

---

## 1. roc-lang.org — the site's own pages (undated living documents)

Author for all: Richard Feldman / the Roc team, unless noted. These pages are the project's
design manifesto; they are revised in place, so "date" is meaningless — what matters is what
they currently commit to.

| Page | Link | Summary | Kind |
|---|---|---|---|
| **Fast** | [roc-lang.org/fast](https://www.roc-lang.org/fast) | What "fast" commits to: outperform mainstream GC languages (Go, C#, Java, JS) as the goal; explicitly a *non-goal* to beat C/C++/Zig/Rust, whose memory unsafety gives them a higher ceiling. Platforms choose memory strategies per domain (skip deallocation for scripts, per-request arenas for web servers, meshing compaction for long-lived state). Build goal: normally under 1s, ideally under the ~100 ms perception threshold, via caching. | language design |
| **Functional** | [roc-lang.org/functional](https://www.roc-lang.org/functional) | Roc as a pure functional language with escape hatches that don't break purity: local `var $x` mutation visible at every use site, `for`/`while`/`return`, and the pure/effectful split in the type system (`->` vs `=>`, `!`-suffixed names). Contains the canonical **Opportunistic Mutation** explanation: semantically immutable values, mutated in place when uniquely referenced. Also confirms the old `Task`-based effect design is gone — effectful functions are now called directly. | language design |
| **Friendly** | [roc-lang.org/friendly](https://www.roc-lang.org/friendly) | The friendliness programme: small unambiguous syntax (no inheritance, no monkey patching, static dispatch), a zero-configuration formatter, Elm-style helpful compiler errors, `expect`-based testing built into the language, and the ability to run a program that has compile errors (crashing only if execution reaches them — noted as partially implemented). Future testing plans named: benchmarking, generative/snapshot tests, simulated I/O, "reproduction replays." | language design |
| **FAQ** (18 questions) | [roc-lang.org/faq](https://www.roc-lang.org/faq) | The project's book of settled refusals and rationales — see the question list below. Highlights: **no `Maybe`/`Option`** (use descriptive tag unions and `Try`); **no linear, dependent, refinement, or uniqueness types — "the plan is for Roc to never have" them**; **the compiler will never be self-hosted** (Zig is the permanent implementation language); no JS/JVM/CLR/BEAM targets, ever; **one platform per app, platforms do not compose** (exclusivity is the point); no wildcard imports, *because they would break parallel name resolution*; no function equality; UPL license rationale; name origin (the mythological bird + the `.roc` extension). | language design |
| **BDFN (Governance)** | [roc-lang.org/bdfn](https://roc-lang.org/bdfn) | Feldman is "Benevolent Dictator For Now": he has final say today, and his stated last act as BDFN will be to hand decision-making to a successor system — expected "well after a 1.0 release." The open problem he names: designing governance that keeps feature growth slow without forcing it to zero, since committee-driven languages accrete complexity by default. | community |
| **Community** | [roc-lang.org/community](https://www.roc-lang.org/community) | Zulip is the hub; monthly online meetups (show-and-tell format); no Roc conference has ever been held. Documents the informal idea → proposal → implementation pipeline for design changes — deliberately no RFC process at the project's current size, with BDFN as tiebreaker. | community |
| **Foundation** | [roc-lang.org/foundation](https://roc-lang.org/foundation) | The Roc Programming Language Foundation: 501(c)(3), board = Feldman (President), Folkert de Vries (VP), Anton Van Moere (Secretary). Public documents posted include the 1023-EZ, the **2025 Form 990-N**, and the IRS determination letter — i.e. the transparency filings are current. | funding |
| **Donate** | [roc-lang.org/donate](https://www.roc-lang.org/donate) | The live ask: **$4,000 USD/month** to fund one longtime contributor full-time, via Every.org, Liberapay, and GitHub Sponsors. Unchanged from what the ecosystem report found on the homepage. | funding |
| **Different Names** | [roc-lang.org/different-names](https://www.roc-lang.org/different-names) | A Rosetta table for newcomers: `List.fold` = foldl/reduce/inject, `List.keep_if` = filter/select, `?` = bind/flatMap/`>>=`, etc. Small page, but it documents the new compiler's snake_case builtin naming (`fold_rev`, `keep_if`, `drop_if`). | language design |
| **Tutorial** | [roc-lang.org/tutorial](https://www.roc-lang.org/tutorial) | The language tutorial (reference, not a post). Per the Zulip digest, the mini-tutorial is slated for replacement before 0.1.0 and the full language reference is expected "after 0.1.0." | language design |
| **Install** (6 OS pages) | [roc-lang.org/install](https://www.roc-lang.org/install) | Per-OS install guides (Linux, macOS, Windows, Nix, other). Distribution reality behind them: nightlies + the RC, Homebrew (announced 2026-11-21), signed macOS binaries (2026-11-20) — all Zulip-announced, per the digest. | release |

**FAQ question index** (for Space builders): name origin · strings/Unicode posture · no wildcard
`exposing` · no Maybe/Option/null · no BigNum/BigDecimal (I128/Dec128 instead) · no higher-kinded
or higher-rank types · no default currying · **advanced types (linear/dependent/refinement/
uniqueness): never** · platforms don't compose · no high-level compilation targets · **compiler
never self-hosted** · UPL license · no function equality.

---

## 2. rtfeldman.com — Richard Feldman's blog (complete: 4 posts)

| Date | Title | Summary | Kind |
|---|---|---|---|
| 2024-04-29 | **Functional Semantics in Imperative Clothing** | [rtfeldman.com/imperative-clothing](https://rtfeldman.com/imperative-clothing) — Introduces the `!` suffix (implemented by Luke Boswell): effectful calls that *look* imperative but desugar to `Task.await` chains over pure functions, motivated by porting a Bash build script for the roc-lang.org site itself. Explains effects-as-values: functions return descriptions of I/O; the platform's runtime interprets them, free to choose sync, async, or io_uring strategies. **Historical caveat:** this documents the *old* compiler's Task-based design. The new compiler kept the `!` sigil but replaced the semantics — effectful functions are now direct (`=>` types), with no `Task` in between. | language design |
| 2024-05-28 | **0.1 + 0.2** | [rtfeldman.com](https://rtfeldman.com) — The roadmap post: what Roc's first two numbered releases would contain and the project's versioning philosophy. Written two years before RC1; the 0.1.0 that finally shipped as an RC in Nov 2026 is recognisably the release scoped here. | release |
| 2026-07-15 | **How Our Rust-to-Zig Rewrite is Going** | [rtfeldman.com/rust-to-zig](https://rtfeldman.com/rust-to-zig) — The milestone retrospective: 487 days, ~300K lines of Rust → Zig, feature parity reached. Incremental builds 10.0s/3.4s (Rust) → 0.035s (Zig `-fincremental`); hot code loading; Rocci Bird ported as the parity proof (31 KB wasm, under half the old output); the compiler running in the browser as a 2.5 MB wasm on the homepage. Safety ledger: 2 Zig use-after-frees (in error-message paths) vs 21 Rust-era memory-corruption reports, all miscompilations. Drew a large HN thread (discussion distilled at [hndistilled.com/story/48933149](https://hndistilled.com/story/48933149/)). | compiler internals / release |
| 2026-10-22 | **How close can pure FP get to C?** | [rtfeldman.com/fp-vs-c](https://rtfeldman.com/fp-vs-c) — The roc-deflate result: Åhdén's pure-Roc DEFLATE (which he runs in paid production), tuned by Feldman, benchmarks a median 2.6% (M1) / 8.9% (Ryzen 3900X) behind libdeflate over 144 Silesia runs, byte-identical output. Also the post that said 0.1.0 was "still weeks away" — the last blog word on the release before RC1. Peaked at #8 on HN, #4 on Lobsters, #2 on r/programming (per the Zulip digest). | compiler internals / ecosystem |

**Cadence read:** 2 posts in 2024, none in 2025, 2 in 2026 — every post is a milestone
retrospective, not a running commentary. Feldman's blog is a *megaphone fired twice a year*.

---

## 3. Luke Boswell's blog — lukewilliamboswell.github.io (complete: 8 posts)

Boswell is the ecosystem's second author and its platform engine (basic-cli, roc-ray, roc-gui,
the native platforms, roc-parser, the awesome list). His blog is where platform internals get explained.

| Date | Title | Summary | Kind |
|---|---|---|---|
| 2023-12-15 | **Fullstack Roc + htmx — an early exploration** | First of a three-part series building a fullstack web app with Roc + htmx on the old compiler — the exploratory work that led toward his later platform tooling. | ecosystem |
| 2024-01-25 | **Building a GUI platform — Action-State using Zig and raylib** | The design seed of what became RocRay: an action-state GUI/platform experiment with a Zig host over raylib. | ecosystem / compiler internals |
| 2024-02-27 | **Fullstack Roc + htmx — Events** | Part 2 of the htmx series: event handling in the fullstack setup (~15 min read). | ecosystem |
| 2024-03-14 | **Building a Text Editor — Terminal UI** | A short build log of a terminal text editor in Roc (~2 min read) — early basic-cli-era experimentation. | ecosystem |
| 2024-06-19 | **Fullstack Roc + htmx — Data Table** | Part 3 of the htmx series: data tables (~15 min read). The last post before a ~14-month gap. | ecosystem |
| 2025-08-30 | **Roc Platforms: Supporting multi-entrypoint hosts** | [lukewilliamboswell.github.io/roc-platforms-multi-entrypoint](https://lukewilliamboswell.github.io/roc-platforms-multi-entrypoint/) — **The most technically significant post in this catalogue that the earlier research had not catalogued.** Platform hosts expect multiple entrypoint symbols (`init!`, `update!`, `render!`); the prebuilt interpreter shim exposes only one generic entrypoint (`roc_entrypoint`). Solution: at runtime the CLI generates a tiny LLVM bitcode adapter (via Zig's `std.zig.llvm.Builder`) defining each host symbol and forwarding to the shim with an `entry_idx`. Result: fast interpreted dev loops for multi-entrypoint platforms — GUIs, games, Android — with no host ABI change. This is the mechanism underneath the whole autumn-2026 platform burst. | compiler internals |
| 2026-11-23 | **Roc platform for native Android apps** | [lukewilliamboswell.github.io](https://lukewilliamboswell.github.io) — Launch post for roc-native-android (repo created 2026-11-22): the roc-signals reactive engine ported onto a Zig JNI bridge + Kotlin host, real Android widgets, no WebView, with a companion `android.roc` CLI that scaffolds and builds APKs; emulator-verified only. Two disclosures worth recording verbatim in substance: Boswell says he has no mobile experience and **"delegated much of the implementation to Opus 5.5"**, putting it together "over a few hours on the weekend" — posted four days after a longtime contributor's Zulip "Goodbye" over LLM-generated code (see §7). Also note: the post links roc-signals under **lukewilliamboswell/**, not rtfeldman/ (see §7, delta D1). | ecosystem / release |

---

## 4. Related systems — state and cadence (blog + repo + Zulip evidence)

The platform layer is where the blog evidence and the release trains meet. (Versions as of
2026-11-23; full detail in `ecosystem-2026-11-23.md` §2–§3.)

| System | What it is | State & cadence |
|---|---|---|
| **basic-cli** (roc-lang) | The default CLI platform (Rust host): files, Cmd, Http, Tcp, Sqlite, Tty, signals. | Official–active. **0.28.1**, needs nightly-2026-11-13+. 0.26→0.28.1 across Oct–Nov 2026 — roughly fortnightly, tracking nightlies within days. |
| **basic-webserver** (roc-lang) | HTTP server platform (Rust, hyper+tokio): Sqlite, SSE, forms, static files. | Official–active, fastest-moving official platform: **0.17 → 0.22.0 in ~six weeks** (0.22.0 on 2026-11-16), pinning nightly-2026-11-15. |
| **roc-signals** | Feldman's reactive-signals UI engine (web + native GUI); the engine inside roc-native-android and roc-gui-era UI work. | Experimental–active. **0.3.0** released in the digest window ("reactive engine overhauled"); native-GUI starter archives shipped as `gui-0.1.0-rc.1/2/3`. **Repo now under lukewilliamboswell** (see delta D1). |
| **roc-native-android** | Native Android apps in pure Roc — real widgets, no WebView; Kotlin shell, no Gradle; arm64. | Experimental/PoC, **created 2026-11-22**, v0.0.1; launch post 2026-11-23 (§3). Examples: counter, todos, stopwatch, a virtualised 10,000-contact list, HTTP, sensor "ports." |
| **roc-native-macos** | Native macOS apps (Boswell). | Experimental; on the current awesome list; not separately audited. |
| **roc-gui** | Desktop GUI platform on GPUI (Zed's framework). | Experimental but substantial: created 2026-09-14, **25 releases in ~10 weeks**, 0.1.0-rc1 in the digest window; README says not for production. Its dev loop depends on the multi-entrypoint mechanism in Boswell's Aug 2025 post. |
| **RocRay / roc-ray** | 2D games/visual tools over raylib (Zig host); ships a 177-page manual (v0.12.3). | Community–active: **51 releases** lifetime, 0.12.x current; the steadiest release cadence in the ecosystem after the official platforms. |
| **Joy** (Niclas Åhdén) | Full-stack isomorphic web framework/platform (Elm architecture both sides). | Community–active; 0.35.0; 5 releases since Dec 2024 — slow cadence, but the ecosystem's only framework with ~2 years of production use behind it (see §5, Åhdén's talk). |
| **roc-cloudflare-workers** (Karakatiza666) | Roc on Cloudflare Workers: wasm32 reactor + JS shim; typed D1/R2/KV/Queues/Workers AI modules. | Experimental, solo: created 2026-10-26, **10 releases in under a month** (v0.0.10) — the fastest cadence in the ecosystem, and single-author. |
| **Rocco** (Braeden Foster / pixelsprout) | 3D engine platform (Odin host, WebGPU). | The Zulip window's most active project: v0.6.0 (Oct 28) → **v0.10.0 (Nov 4)** → web builds via Roc→wasm + WebGPU (Nov 16). |
| **concave** (Jack Scully) | Convex-style reactive web platform on 0.1.0-RC1 (Cloud Run + serverless Postgres). | Announced 2026-11-23 — brand new; its author hit the missing compiler-API wall within days (digest §4). |
| **roxum** (kaza) | axum-style web platform. | Announced 2026-11-07; RC1-compatible per its author's report. |
| **Exercism Roc track** | The learning on-ramp: 110 exercises. | Fully migrated to the new compiler Aug 2026; **running on 0.1.0-RC1** as of 2026-11-22. The clearest adoption signal that the RC is real. |

**Cadence summary:** official platforms release every ~2 weeks against dated nightlies;
community platforms range from RocRay's steady train to the Cloudflare platform's
10-releases-in-a-month solo sprint. Everything pins a *nightly*, not a release — until
0.1.0 lands, nightly pins are the ecosystem's versioning system.

---

## 5. Talks & podcasts (consolidated; Δ = new or corrected vs `talks-and-announcements.md`)

The earlier investigation's talks file is thorough for 2019–2025; it stands. This section is the
watch-first core plus everything newer. **Δ items are additions from this pass.**

**Watch first**
- **GOTO Unscripted, "Roc & Zig: A Compiler Rewrite Story"** (June 5, 2026) — Feldman + Andrew
  Kelley on the rewrite, hot code loading, and incremental compilation.
  [youtube.com/watch?v=FbvtbuhCiDU](https://www.youtube.com/watch?v=FbvtbuhCiDU)
- **FUNARCH 2026 keynote, "Functional Mechanical Sympathy"** (Sept 17, 2026, Utrecht) — Feldman
  on designing a language around how hardware actually works; a full talk-length companion to the
  fp-vs-C post. [conf.researchr.org](https://conf.researchr.org/details/funarch-2026/funarch-2026-papers/2/Functional-Mechanical-Sympathy)
- **Strange Loop 2021, "Outperforming Imperative with Pure Functional Languages"** — the canonical
  Roc talk; ~98K views, still the highest-reach Roc video. Covers opportunistic mutation and the
  performance thesis. [thestrangeloop.com/2021](https://thestrangeloop.com/2021/outperforming-imperative-with-pure-functional-languages.html)

**Δ New since / missing from the talks file**
- **Δ "A Preview of Roc 0.1.0"** — Feldman, hallway.video, **2026-08-13**. A recorded preview of
  the release, two months before RC1. [hallway.video](https://hallway.video) (listing-level; the
  ecosystem report cites it from GitHub, this pass did not re-verify the video page.)
- **Δ "Roc and Joy: Productivity in Production"** — **Niclas Åhdén**, Func Prog Sweden,
  **2026-11-10**. The ecosystem's most experienced production user presenting his framework —
  notable because it is a *non-Feldman* conference talk by a practitioner, the rarest kind in
  this list.

**The Feldman conference canon** (detail in `talks-and-announcements.md`)
- YOW! 2022, "Unwrapped: Roc's memory-safe tag unions" · GOTO CPH 2022, "An Outsider's Guide to
  Roc's Type System" · GOTO Chicago 2023, "Simplicity in Language Design: Live-coding a Roc FFI
  example" · GOTO 2023, "Rust & Zig Combined" · Strange Loop 2023, "Revise, Iterate, Repeat" ·
  YOW! 2023, "Outperforming Imperative with FP (Reprise)" · Craft Conf 2025, "What makes Roc's
  compiler fast" · Code BEAM Europe 2025 (Elm→Roc design lessons) · Systems Distributed '25,
  "Building Distributed Systems with Roc".

**Podcasts** — Rust in Production S05E04 (Nov 13, 2025; the best production-use interview) ·
The Changelog #645 (the Zig rewrite, practitioner detail) · Developer Voices (June 2025) ·
ADSP 157–159 (three-part deep dive) · Elixir Wizards (Jan 2024) · Software Unscripted Ep. 85 ·
Software Engineering Radio Ep. 553.

**Engagement read:** Roc's talks are overwhelmingly Feldman. The two Δ additions are the first
signs of breadth — a release-preview format and a production-user talk — but as of Nov 2026 there
is still no second regular conference voice.

---

## 6. Notable third-party posts about Roc

| When | Post | Read |
|---|---|---|
| 2022-12 | **JesterXL, "Roc Lang for Advent of Code 2022"** ([dev.to](https://dev.to/jesterxl/roc-lang-for-advent-of-code-2022-2c9b)) | A hands-on experience report from the old-compiler era — historically interesting as a newcomer's-eye view of pre-rewrite Roc. |
| 2025-06 | **"Roc func-prog-lang, vs Elm, Go, Rust, Zig"** ([blog.dragansr.com](https://blog.dragansr.com/2025/06/roc-func-prog-lang-vs-elm-go-rust-zig.html)) | A structured comparison from a polyglot blogger; neutral-to-positive; written entirely from the old compiler's world. |
| n.d. | **Test Double, "Super Performant New ROC Language"** (Q&A with consultant Eric Newbury) ([testdouble.com](https://testdouble.com/insights/super-performant-new-roc-language)) | Practitioner Q&A from a consultancy blog — one of the few non-community-insider evaluations. |
| 2026-03-29 | **"Roc Has a Free Fast Functional Language…"** ([dev.to](https://dev.to/gabrielanhaia/roc-has-a-free-fast-functional-language-but-nobody-knows-it-55k2)) | Shallow overview ("nobody knows it" is the thesis); useful mainly as evidence of Roc's discoverability problem. |
| 2026-07-03 | **AlopexDB, "Getting Started with Roc"** ([asopi.tech](https://asopi.tech/posts/roc-lang-intro/)) | An *evaluation* post (SQL-parser project) — but **stale**: describes Roc as alpha/pre-1.0 with old-compiler characteristics weeks after the parity post landed. The clearest exhibit that third-party coverage has not caught up with the new compiler. Cite with care (the earlier investigation flags its version numbers as unreliable). |
| 2026-10 | **"Roc vs. C: Performance Evaluation"** ([dev.to](https://dev.to/bryce78666/roc-vs-c-performance-evaluation-of-the-deflate-algorithm-2k8a)) | Derivative of Feldman's fp-vs-C post; low independent value. |
| ongoing | **Steve Howell, roc.lynrummy.com** — a book + tutorial for the new compiler, apps built through agentic (Claude-assisted) development, and Wasm demos of Luke's roc-ray examples running in the browser. | The most substantial third-party *learning material* for the new compiler; also another data point for AI-assisted Roc development (§7, F5). |
| ongoing | **HN discussion of the rewrite post** ([hndistilled.com/story/48933149](https://hndistilled.com/story/48933149/)) | The substantive external critique surface: Rust-vs-Zig safety/tooling debate, skepticism about bolt-on borrow checking, build-speed evidence. Where the informed outside argument about Roc actually happened in 2026. |

**Absence worth naming:** no "why we left Roc" post, no major-outlet review, and no independent
benchmark suite coverage surfaced. External writing about Roc in 2026 is mostly *reactions to
Feldman's own posts*.

---

## 7. Findings — the interesting and the load-bearing

**F1. Roc has no blog; it has a manifesto site, two personal blogs, and a chat.**
Release news lives on Zulip and GitHub releases. The website's README still framed 0.1 as not
ready while RC1 was already public (noted in the ecosystem report), and the 0.1.0 announcement
post is being assembled after the fact. Anyone (or any census) watching roc-lang.org for news
will systematically lag reality by weeks. **This is the pass's main methodological finding.**

**F2. The site is a record of deliberate, permanent refusals — and it just hardened one.**
The FAQ now states in writing: never linear/dependent/refinement/uniqueness types, never a
self-hosted compiler, never JS/JVM/CLR/BEAM targets, never platform composition, no Maybe.
Feldman's 2026-11-23 Zulip answer on linear types ("the cure is worse than the disease") is not
an offhand remark — it is the site policy verbatim. Roc's design identity is defined as much by
this refusal list as by its features.

**F3. The `!` sigil has a documented lineage — and its founding document is now obsolete.**
The April 2024 post introduced `!` as sugar desugaring to `Task.await` chains. The new compiler
kept the surface syntax and replaced the machinery (direct effectful functions, `=>` types, no
Task). The blog post was never updated to say so; the /functional page carries the current design.
Readers landing on the 2024 post from search will learn a semantics that no longer exists.

**F4. The multi-entrypoint post is the missing manual for the platform burst.**
Boswell's Aug 2025 interpreter-shim/LLVM-bitcode-adapter post (§3) explains *how* roc-gui,
roc-native-android and friends get fast dev loops despite multi-symbol host ABIs. It predates
those platforms' public launches and reads, in hindsight, as their enabling infrastructure.
It was absent from the earlier research's catalogue (it was cited in the Zulip digest only as
the roc dev REPL root cause).

**F5. The ecosystem's LLM-code question has a concrete, current data point — on the record.**
Four days after a longtime contributor's "Goodbye" over LLM-generated code (Zulip, 2026-11-19;
no policy adopted, per the digest), the project's most prolific platform author published a
platform launch post stating he "delegated much of the implementation to Opus 5.5" and built it
in a weekend — openly, on his own blog, with no evident pushback. A third-party book author is
simultaneously showcasing Claude-built Roc apps. Whatever the community decides, the practice is
already normalised at the centre of the ecosystem; the controversy is about *disclosure and
review norms*, not existence.

**F6. The RC1 → 0.1.0 trajectory, as the public record tells it.**
Blog record: roadmap scoped 0.1 in May 2024 → parity post (Jul 2026) says "later this year" →
fp-vs-C post (Oct 22) says "weeks away" → silence. The actual RC1 announcement (Nov 21), the
zero-blocker status, and Feldman's floated option of promoting RC1 directly to 0.1.0 all exist
**only on Zulip**. The blog trail corroborates the census's trajectory but is not where its
facts live; nothing on any blog surface post-dates or contradicts the 2026-11-23 investigation.

**F7. Engagement is real but narrow.**
Feldman's two 2026 posts are the ecosystem's entire mass-media surface: HN front-page threads,
#8 HN / #4 Lobsters / #2 r/programming for the deflate post. Third-party coverage is thin,
partly stale (AlopexDB), partly derivative (dev.to). Roc's reputation is currently carried by
its founder's writing — effective, and a single point of failure.

**F8. Ecosystem health, as the blog itself supports it.**
Posting cadence is low (Feldman: 2 posts/yr, both milestone retrospectives; Boswell: 2 posts in
15 months after a quiet stretch) but every post marks something shipped. The blog surface is
100% Feldman + Boswell — the same ~5-person concentration the ecosystem report measured in code
is also true of the *writing*. The honest read: a small project whose public narrative is
disciplined and milestone-driven, whose real activity (releases, platforms, community projects)
is substantially larger than its blog footprint suggests — visible only if you read Zulip and
the release pages.

### Census deltas vs the 2026-11-23 investigation

- **D1 (correction): roc-signals ownership.** The ecosystem report lists roc-signals as
  Feldman's (`github.com/rtfeldman/roc-signals`). Boswell's 2026-11-23 post links it as
  `github.com/lukewilliamboswell/roc-signals` and describes the engine as his experiment.
  Current evidence: the signals engine is Boswell-driven and the repo lives under his account.
- **D2 (addition): Boswell's multi-entrypoint post (2025-08-30)** — a substantive compiler-
  internals document missing from the prior catalogue; now §3/F4.
- **D3 (addition): Feldman's "Functional Semantics in Imperative Clothing" (2024-04-29)** — the
  fourth rtfeldman post; prior research covered 3 of 4. Its content is superseded (F3).
- **D4 (addition): two talks** — "A Preview of Roc 0.1.0" (2026-08-13) and Åhdén's "Roc and
  Joy" (2026-11-10) — now §5.
- **D5 (corroboration, not contradiction):** the FAQ's "never" on linear types hardens the
  digest's same-day Zulip report into site policy (F2).
- **No contradictions found.** Nothing on roc-lang.org, rtfeldman.com, or Boswell's blog
  post-dates the investigation (all research is same-day, 2026-11-23) or conflicts with its
  claims about RC1, platform versions, or ecosystem concentration. Where the surfaces disagree
  in *freshness* (website vs Zulip), Zulip is ahead — see F1/F6.

---

## 8. Front-page picks for the Roc Space

Six items, chosen to mirror the Iroh Space's shape (headline state → deep dives → people →
primary sources):

1. **0.1.0-RC1: where the release actually stands** — Zulip-announced Nov 21, zero blockers,
   RC1 may be promoted directly to 0.1.0; the website announcement doesn't exist yet. (The
   Space's news card; sources: Zulip digest + §7/F6.)
2. **"How Our Rust-to-Zig Rewrite is Going"** (Feldman, Jul 2026) — the 487-day rewrite
   retrospective; the single best entry point to modern Roc.
3. **"How close can pure FP get to C?"** (Feldman, Oct 2026) — roc-deflate vs libdeflate;
   the performance thesis with production receipts.
4. **"Roc Platforms: Supporting multi-entrypoint hosts"** (Boswell, Aug 2025) — the compiler
   internals underneath the GUI/Android platform wave; the catalogue's best hidden gem.
5. **"Roc platform for native Android apps"** (Boswell, Nov 2026) — the newest platform and
   the signals engine's proving ground; also the ecosystem's LLM-use data point.
6. **The settled refusals** (FAQ + BDFN) — never linear types, never self-hosted, one platform
   per app, BDFN succession-by-design. Roc's design identity in one card.

---

*Method note: roc-lang.org inventory verified against the site's source repo file tree
(2026-11-23); page texts read from source markdown. rtfeldman.com index fetched 2026-11-23;
all four posts accounted for. Boswell's blog enumerated from its source repo content tree;
all eight posts titled/dated from source, the two most recent read in full or near-full.
One fetch (Boswell's homepage, rendered) was rate-limited by the browser service and routed
around via the repo source instead. Items marked "listing-level" were not opened in full.*
