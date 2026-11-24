# Platforms

A Roc *platform* is the host program (in Rust, Zig, Go, …) that provides an
app its effects — files, network, UI. Platforms are where most ecosystem
action is. Maturity labels are defined in the
[index](2026-11-23T22-00-25Z_research_roc-ecosystem.md).

## Official (roc-lang org)

| Platform | What it's for | Status |
|---|---|---|
| **[basic-cli](https://github.com/roc-lang/basic-cli)** | Command-line apps: the default general-purpose platform. Host in Rust. Modules include Stdout/Stdin (streaming lines and chunks), File/Path (writer API, permissions such as OwnerOnly/PublicRead, symlinks), Cmd, Http, Tcp, UnixSocket, Sqlite, Tty (raw mode), Signal (SIGINT/SIGTERM/SIGUSR1/SIGUSR2/SIGWINCH), Env, Locale. | **Official–active.** Latest **0.28.1**, requiring nightly-2026-11-13 or newer. 0.26.0 was a large consistency release (typed `Duration`/`Utc`/`Monotonic`, unified `Err(EndOfInput)`, one error tag per module, mimalloc on Linux static hosts). Maintained mainly by Luke Boswell, with release automation by Anton-4. [Releases](https://github.com/roc-lang/basic-cli/releases) |
| **[basic-webserver](https://github.com/roc-lang/basic-webserver)** | HTTP servers. Host in Rust (hyper + tokio). Ships Sqlite, server-sent events, HTML/forms/static-file support. | **Official–active, very fast-moving.** Latest **0.22.0**, pinning nightly-2026-11-15. It shipped 0.17 → 0.22 across roughly six weeks in Oct–Nov 2026. [Releases](https://github.com/roc-lang/basic-webserver/releases) |

## Community — new compiler

| Platform | What it's for | Status |
|---|---|---|
| **[RocRay / roc-ray](https://github.com/lukewilliamboswell/roc-ray)** (Luke Boswell) | 2D games and visual tools over raylib; Zig host. Drawing, audio, keyboard/mouse/gamepad, files, networking, screen recording; macOS/Linux/Windows x64. Explicitly 2D-only (3D planned) and not a GUI toolkit. Ships real example programs: Breakout, Pong, Snake, Pixel Workshop, Spark Run, Cave Climb, Particles, Live Plot, Postcard Studio. | **Community–active.** 80 stars, 1,579 commits, **51 releases** (created 2024-01-13). The roc-lang.org example pins release **0.12.1** (nightly-2026-11-12); the [manual](https://lukewilliamboswell.github.io/roc-ray/roc-ray.pdf) is at 0.12.3. |
| **[roc-gui](https://github.com/lukewilliamboswell/roc-gui)** (Luke Boswell) | Native desktop GUI on **GPUI** (Zed's UI framework); cross-platform (macOS, Linux/Wayland, Windows). Ships a behavioural/performance test harness and a performance observatory streaming to SQLite; the repo gallery shows a dozen demo apps (HTTP Workbench, Database Browser, Terminal Workspace, Redis Explorer, …). | **Experimental, but substantial.** Created 2026-09-14; 733 commits, 25 releases in ~10 weeks. README: "still an early-stage platform. Do not use it in production." A 0.1.0-rc1 was cut during the November Zulip window. |
| **[roc-signals](https://github.com/rtfeldman/roc-signals)** (Richard Feldman) | Reactive-signals UI engine, for the web and for native GUI. Its native-GUI docs ship starter archives as release candidates pinned to nightly-2026-09-04 ([docs](https://github.com/rtfeldman/roc-signals/blob/HEAD/www/content/docs/native-gui.md)). The same signals engine powers roc-native-android (below). | **Experimental–active.** Feldman's own current UI project; roc-signals 0.3.0 overhauled the reactive engine during November 2026. |
| **[roc-native-android](https://github.com/lukewilliamboswell/roc-native-android)** (Luke Boswell) | Native Android apps in pure Roc — real Android widgets, **no WebView**. Reactive engine from roc-signals; arm64 only; Kotlin shell, no Gradle. Examples: counter, todos, stopwatch, contacts (a virtualised 10,000-contact list), http, and "ports" (battery/accelerometer exposed from Kotlin). | **Experimental / PoC — brand new.** Created **2026-11-22**; v0.0.1, 20 commits; self-described "early proof of concept." Luke's launch post, "Roc platform for native Android apps," is dated 23 Nov 2026 on [lukewilliamboswell.github.io](https://lukewilliamboswell.github.io). |
| **[roc-native-macos](https://github.com/lukewilliamboswell/roc-native-macos)** (Luke Boswell) | Native macOS apps. | **Experimental.** Listed on the current awesome list; not separately audited for this record. |
| **[Joy](https://github.com/niclas-ahden/joy)** (Niclas Åhdén) | Full-stack / isomorphic web framework + platform: Elm-architecture client and server in one language. | **Community–active; the ecosystem's most battle-tested framework.** 66 stars, 256 commits, 5 releases (created 2024-12-10); current README pins Joy **0.35.0** + joy-html 0.18.0. README self-label: "early development, not production-ready… expect breaking changes" — yet its author runs it in production (see the projects page), and its js-framework-benchmark numbers are competitive with Elm/Leptos/Solid. |
| **[roc-cloudflare-workers](https://github.com/Karakatiza666/roc-cloudflare-workers)** (Karakatiza666) | Roc on **Cloudflare Workers**: apps compile to a wasm32 reactor behind a JS shim. Typed modules for fetch, D1, R2, KV, Cache API, Durable Object SQL, Queues, and Workers AI; Zig host; effects await JS promises via JSPI. Companion CLI: `roc-cloudflare-workers-cli`. | **Experimental — brand new and solo.** Created **2026-10-26**; already at **v0.0.10** (10 releases in under a month, 216 commits, 0 stars). |
| **[roc-tower-platform](https://github.com/grayrest/roc-tower-platform)** (grayrest) | Web servers with pure handlers on Rust hyper/Tower + SQLite. | **Community–active** (current awesome list). |
| **[basic-ssg](https://github.com/lukewilliamboswell/basic-ssg)** (Luke Boswell) | Static site generation. | **Community–active** (current awesome list). |
| **[roc-fuzz](https://github.com/lukewilliamboswell/roc-fuzz)** (Luke Boswell; originally bhansconnect) | Fuzz-testing platform. | **Community–active** (current awesome list). roc-fuzz 0.7.0 (Nov 2026) packs many fuzz targets into one binary. |
| **[roc-wasm4](https://github.com/lukewilliamboswell/roc-wasm4)** (Luke Boswell) | Games for the WASM-4 fantasy console. Its game Rocci Bird was the new compiler's feature-parity demo. | **Community–active** (current awesome list). |
| **[Rocco](https://github.com/BraedenFoster/rocco)** (Braeden Foster; engine work by pixelsprout) | Experimental **3D engine** platform: engine in Odin, WebGPU rendering; a game is a model plus five pure functions producing a `Scene` value. glTF, PBR, shadows, bloom; macOS/Linux/Windows/web builds. | **Experimental — and November's most active project.** v0.6.0 (Oct 28: positional audio, PBR/shadow maps, tone mapping, gamepad) → v0.10.0 (Nov 4: environments, fog, bloom) → **web builds via Roc→wasm + WebGPU** (Nov 16) with an interactive web tour. |
| **[roc-screensaver](https://github.com/showell/roc-screensaver)** (Steve Howell) | Zig platform with its own software rasterizer: one Roc app runs both as a Windows screensaver and as a web page. Three screensavers ship on it, including "Lanterns, a sunrise made for Roc's 0.1.0 launch"; ~8,155 lines of Roc apps. | **Experimental–active** (single-author, shipping). |
| **[restricted-roc](https://codeberg.org/removewingman/restricted-roc)** (removewingman, Codeberg) | CLI platform that can sandbox itself — restricts syscalls and filesystem access. | **Experimental** (current awesome list). |
| **Platform templates** (Luke Boswell) | Scaffolding for writing new platforms in [Go](https://github.com/lukewilliamboswell/roc-platform-template-go), [Rust](https://github.com/lukewilliamboswell/roc-platform-template-rust), [Zig](https://github.com/lukewilliamboswell/roc-platform-template-zig). | **Community–active.** Worth noting: the "write your own platform" path is a supported, templated workflow, which is how the long tail above exists at all. |

## New platforms announced in the November Zulip window

Summarised from [community and Zulip digest](community-and-zulip-digest.md):

- **concave** (Jack Scully, Nov 23) — Convex-inspired reactive web platform
  on 0.1.0-RC1: serverless, scale-to-zero, front+back reactivity, deploys to
  Google Cloud Run + serverless Postgres.
- **roxum** (kaza, Nov 7) — web platform bringing axum's style/safety to Roc
  ([github.com/KouYuume/roxum](https://github.com/KouYuume/roxum)).
- **roc-platform-patcher** (Scott Campbell, Oct 30) — "alter & tailor
  platforms": inject middleware into a platform to observe/alter effects,
  monitor allocations, stub or hot-swap effect implementations.
- **DST platform thesis** (Jan Rüggeberg) — a deterministic-simulation-testing
  platform simulating a basic-cli subset for a Raft KV store; Luke Boswell
  called DST an excellent pitch for Roc and a possible validation of compiler
  determinism.
- **trantor** (Karl) — "an extensible platform system," listed for the 0.1.0
  showcase.
- **Kite** (Karl, Nov 23) — a native GUI platform, just pushed to GitHub,
  feedback wanted.

One open platform need from the same window: a **compiler API / introspection
surface**. Jack Scully's concave needs an app's exported endpoints with
resolved types; today that means hand-rolled parsers or `--introspect` hacks.
Feldman is cautious about exposing compiler APIs at all — exposing IR/APIs
makes them unchangeable — so this is wanted, not planned.

## Legacy-compiler platforms (not on the current awesome list)

Treat as **legacy / superseded** unless re-verified: roc-aws-lambda and
roc-aws-lambda-runtime (isaacvando), roc-iced (tarkah), roc-microbit
(bhansconnect, embedded), static-site-gen (isaacvando), roc-ecs (ageron),
roc-parallel demo (ageron). Aurélien Geron (ageron) also maintains **roclet**
(reactive Roc notebooks in the browser, running even offline —
[ageron.github.io/roclet](https://ageron.github.io/roclet/)) and
new-compiler-updated forks of basic-cli/basic-webserver (updated 2026-10-05).
