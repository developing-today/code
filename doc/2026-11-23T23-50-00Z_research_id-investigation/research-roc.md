# Roc ecosystem survey — app-platform relevance for the ID project

Research date: 2026-11-23. Method: public web pages fetched and read directly
(blog posts, official Roc docs/examples, Falcor docs, curated lists). Claims
marked **[unverified]** were not confirmed first-hand. The Reddit link could
not be read (see §2).

**ID, for relevance notes:** a distributed, capability-based (OCapN-style),
content-addressed graph platform — signed objects in content-addressed
storage (iroh), CRDT/graph data, apps written once in a pure language and run
across hosts (desktop / Android / web / server) that each supply a standard
capability set, with fuel-metered execution and token/in-app-item economics.

---

## 1. Luke Boswell's blog — all posts, grouped by topic

Source: https://lukewilliamboswell.github.io/ (index read 2026-11-23).
Boswell is a Roc core contributor (author of roc-lang/roc PR #8214, the
platform host shim) and maintainer of the roc-awesome list. Eight posts total:

### Platforms / compiler infrastructure

- **"Roc Platforms: Supporting multi-entrypoint hosts"** (30 Aug 2025) —
  https://lukewilliamboswell.github.io/roc-platforms-multi-entrypoint/
  How the new Roc CLI's pre-built interpreter shim (single generic
  entrypoint) is adapted at runtime to hosts expecting multiple entrypoints
  (`init!`, `update!`, `render!`): the CLI generates a tiny LLVM bitcode
  adapter per platform. Landed via roc-lang/roc PR #8214.
  _Relevance to ID:_ an app platform whose hosts expose several entrypoints
  (init/render/update, or pure-client vs server roles) needs exactly this
  glue; it also shows the fast interpreted dev loop can coexist with
  multi-entrypoint hosts — useful for an ID app SDK.
- Platform templates (not blog posts, but his): `roc-platform-template-zig`
  (actively maintained, release 0.6; also -rust, -go; -c/-wasi/-swift
  archived) — the canonical starting point for a new platform/host.
  _Relevance to ID:_ the template an ID platform would fork; the Rust
  template matters if ID's host is Rust/iroh-based.

### roc-ray / graphics / games

- **"Building a GUI platform — Action-State using zig and raylib"**
  (25 Jan 2024) — https://lukewilliamboswell.github.io/roc-ray-experiment/
  The original roc-ray experiment (see §4).
- **"Ball physics simulation"** (3 Nov 2024) —
  https://lukewilliamboswell.github.io/roc-ray-ball-physics
  A spring/gravity ball simulation on a WIP roc-ray branch, using his
  `roc-pga2d` package (2D projective geometric algebra) for the physics math
  and `roc-random` for generation. The app code shows the newer
  `init!`/`render!` effectful-function style with `Draw.*!` calls.
  _Relevance to ID:_ proof that simple real-time simulations/games
  (the Dwarf-Fortress/Space-Invaders class Drew named) are expressible as
  pure Roc app logic over a drawing capability set; also a data point that
  Roc-side math packages (PGA) can carry simulation math without a BLAS.

### roc-tui

- **"Building a Text Editor — Terminal UI"** (14 Mar 2024) —
  https://lukewilliamboswell.github.io/text-editor-tui
  A text-editor example (open/save/undo/redo/cursor) built on the
  `basic-cli` platform plus his `roc-ansi` package (ANSI escape helpers),
  in the roc-ansi repo — i.e. TUI as a _package_ over a CLI platform, not a
  dedicated platform. (His separate `roc-tui` platform repo is in §5.)
  _Relevance to ID:_ text interfaces (mind-mapping, graph browsing,
  sketching) can ride on the plain CLI capability set + a library — the
  cheapest tier of the "same app, many surfaces" story.

### Android

- **"Roc platform for native Android apps"** (dated 23 Nov 2026 on the
  index — i.e. published today) —
  https://lukewilliamboswell.github.io/roc-native-android/ (see §6).

### Fullstack web (basic-webserver + htmx)

- **"Fullstack Roc + htmx — Early exploration"** (15 Dec 2023)
- **"Fullstack Roc + htmx — Events exploration"** (27 Feb 2024) —
  https://lukewilliamboswell.github.io/roc-htmx-demo-2/
  (adds a SQLite API to basic-webserver, tree views of hierarchical data,
  htmx event-driven re-rendering)
- **"Fullstack Roc + htmx — Data Table"** (19 Jun 2024)
  _Relevance to ID:_ server-side Roc apps rendering graph-shaped data
  (tree view over a nested-set hierarchy in SQL) with the same
  platform/app split — the "server host" end of the write-once story, and
  a pattern for graph visualisation served as HTML.

---

## 2. The Reddit link — could not be read

https://www.reddit.com/r/roc_lang/s/3nfBm2CheC — **reddit.com is blocked
for direct fetch in this environment, and targeted searches did not surface
the post**, so its exact title/content are **[unverified]**. Given the
timing (Boswell's roc-native-android post is dated today, 2026-11-23) it
most plausibly shares/discusses that Android platform release — treat that
as inference, not fact.

---

## 3. Roc platforms generally — the model ID would build on

Sources: official Platforms docs
(https://www.roc-lang.org/docs/main/langref/platforms/), the roc-ray post,
the multi-entrypoint post, roc-lang/roc DeepWiki platform summary.

- **Platform/app split.** Every Roc app is built on exactly one _platform_.
  The platform — not a standard library — supplies **all** I/O primitives.
  A platform = a Roc-side API (modules the app imports) + a _host_ written
  in another language (Rust, Zig, Go, C, Swift…) that implements the actual
  system operations and the `RocOps` runtime interface (alloc/realloc/
  dealloc, panic, etc.). The platform header declares `requires` (what the
  app must provide, e.g. `main!`), `exposes` (API modules for apps), and
  `provides` (entrypoints handed to the host).
- **Pure vs effectful.** Roc functions are pure by default. In the current
  (new-compiler) language, effectful operations are _hosted functions_
  marked with a `!` suffix (`Stdout.line!`, `init!`, `render!`); purity is
  visible in every signature, and errors are `Result` values — no
  exceptions, no null. (The older compiler expressed the same split as
  `Task` + platform `Effect` functions, as seen in the Jan 2024 roc-ray
  post; the `Task`→`!` migration is manual per the Advent 2025 notes.)
- **Capability definition.** Because the platform defines the _entire_
  effect surface an app can touch, a platform **is** a capability set: an
  app can only do what its platform's exposed modules let it do. Define the
  Roc-side API once and implement hosts per target (desktop/Android/web/
  server) and the same app runs everywhere the API is implemented — this is
  the mechanism behind every project in §§4–6, and it maps directly onto
  ID's "standard capabilities tied to libraries, programmed against once,
  implemented per platform" goal, and onto capability security: the host
  can meter, log, or refuse each hosted call (a natural fuel/accounting
  hook).
- **Multi-entrypoint hosts** are now supported by the CLI (see §1), so a
  platform can expose several host entrypoints (init/update/render, or
  client/server roles) without giving up the interpreted fast dev loop.
- **Language/compiler status (risk factor).** Roc is pre-1.0 and explicitly
  "not ready for a 0.1 release yet" (official position as of mid-2026,
  reported at https://asopi.tech/en/blog/20260703_2/). The compiler was
  rewritten in Zig (new compiler usable for Advent of Code 2025; docs on
  roc-lang.org now describe the new design, older material describes the
  old one — expect both dialects in the wild). Hosts are still commonly
  Rust (basic-webserver = Rust + hyper + tokio; basic-cli v0.20 as of
  2025-08). **[Status detail partially secondary-sourced.]**
  _Relevance to ID:_ the platform model is an unusually exact fit for ID's
  architecture, but betting the app layer on Roc means tracking a moving,
  pre-1.0 language — the prompt should treat Roc as the leading candidate
  for the app/capability layer with the churn risk stated, not as settled.

---

## 4. roc-ray (raylib platform)

- **What:** a Roc platform whose Zig host wraps raylib; apps write
  `init!`/`update!`/`render!` callbacks and draw via `Draw`/`RocRay`
  modules (shapes, text, sprites/textures, mouse/keyboard input).
  Repo: https://github.com/lukewilliamboswell/roc-ray
- **Two eras.** (a) The Jan 2024 blog experiment: minimal Action-State GUI
  (Richard Feldman's design draft — `Init`/`Render` returning element
  trees with `Action`s, an alternative to Elm's Model/View/Update),
  a handful of raylib calls, explicitly "not polished". (b) The current
  platform: **release 0.12.1**, featured on the **official Roc examples
  site** (https://www.roc-lang.org/examples/RocRay/README — "Drawing with
  RocRay", validated against Roc nightly-2026-11-12). Release bundles ship
  a precompiled host for **Windows x64, macOS (Intel + Apple Silicon),
  Linux x64 (X11/XWayland)**; app authors need no Zig toolchain. Includes
  a headless smoke-test mode (`--host-headless --host-headless-frames=N`).
- **Maturity:** experimental but actively released and officially
  showcased; still one maintainer's project, API paired to specific Roc
  nightlies.
- _Relevance to ID:_ the strongest existing template for an ID "2D
  surface" capability set (sprites, text, basic games, graph
  visualisation): a proven host pattern (Zig + raylib) and an app API
  simple enough to re-declare as ID capabilities. The Action-State post is
  also the reference for structuring _plugin-like_ UI components with
  isolated state — relevant to ID apps embedding each other's views.

## 5. roc-tui and roc-gui

- **roc-tui** — https://github.com/lukewilliamboswell/roc-tui — "An
  experimental TUI platform for Roc" (listed under Experiments in
  roc-awesome; ~12 stars, last updated Mar 2025 per relatedrepos —
  **[unverified]** star count/date). Distinct from the roc-ansi _package_
  approach in §1. Note there is also an unrelated `mdgriffith/roc-tui`
  (Matthew Griffith, of elm-ui) — **[unverified]** which is more active.
  _Relevance to ID:_ a terminal surface for graph browsing/editing is
  feasible at two levels — ANSI package over a CLI host (cheap, proven by
  the text-editor example) or a dedicated TUI platform (experimental).
- **roc-gui** — https://github.com/lukewilliamboswell/roc-gui — listed in
  roc-awesome's Experiments alongside roc-tui and `roc-graphics-mach`
  (a Mach-engine graphics experiment). No blog post; maturity:
  experimental **[details unverified]**.
  _Relevance to ID:_ evidence the GUI-platform design space is still
  unsettled in Roc-land; ID should expect to define its own UI capability
  surface rather than adopt one.
- **roc-signals** — https://github.com/lukewilliamboswell/roc-signals —
  Boswell's signals-based reactive UI engine, browser-focused (Wasm host;
  modules `Browser`, `Signal`, `Ui`, `Elem`, `Html`, local-storage
  signals). It began as a compiler stress-test and now drives a RealWorld
  (Conduit) example; it is the engine ported to Android in §6. Maturity:
  active experiment, still shaking out compiler bugs (e.g. roc-lang/roc
  issue #10103, an optimised-wasm panic found via roc-signals, Jul 2026).
  _Relevance to ID:_ the leading candidate pattern for ID's _web/pure
  client_ UI — fine-grained reactivity over graph data, and a worked
  example of one engine retargeted across hosts (browser → Android).

## 6. Roc on Android — roc-native-android

- **What:** https://github.com/lukewilliamboswell/roc-native-android — a
  Roc platform for native Android apps: the roc-signals reactive engine
  ported onto a **Zig JNI bridge + Kotlin host**; ships a companion CLI
  (`android.roc`) that scaffolds an app and builds it into an **APK**.
  A Todo app example runs in the emulator.
- **How it runs:** Roc app → (platform API) → Zig native library via JNI →
  Kotlin/Android host providing the UI/runtime surface. Same app model as
  the browser signals platform, different host — the write-once claim
  demonstrated, not just asserted.
- **Limitations (author-stated):** "significant gaps in features"; enough
  for simple apps; built over a weekend, tested only in an emulator on the
  author's MacBook; not competing with mature mobile frameworks.
- _Relevance to ID:_ the existence proof for ID's Android tier — a
  capability-defined Roc app packaged as a native APK with a thin native
  host. Also a sizing data point: one person + an LLM built the host in
  hours, suggesting per-platform hosts for a _narrow_ capability set are
  cheap; a full ID capability set would be proportionally larger.

## 7. Falcor (Netflix) — graph data access pattern

- **What:** Netflix's JavaScript library for efficient data fetching over
  a **JSON Graph**: all data modelled as one virtual JSON object extended
  with References (normalisation — canonical entities live once, e.g.
  `usersById[123]`, and paths elsewhere are refs), Atoms (leaf values with
  metadata/expiry) and Errors. Docs: https://netflix.github.io/falcor/
  (Model docs: https://netflix.github.io/falcor/documentation/model.html).
- **Mechanics:** clients `get`/`set`/`call` by **path sets** against a
  `Model`; the Model keeps an in-memory cache, **batches and dedupes**
  concurrent path requests, uses cached references to optimise/skip
  requests, and fetches only missing fragments from a `DataSource`
  (HTTP to a Falcor Router, or any transport implementing the interface —
  a Model can even wrap another Model via `asDataSource()`, and `deref`
  binds a Model to a graph location so components see only a fragment).
- **Status:** mature, open-sourced 2015, production-proven at Netflix;
  the JS ecosystem around it is quiet now **[activity level unverified]** —
  its value today is the _design_, not the code.
- _Relevance to ID:_ a reference design for ID's client-side graph access:
  path-based retrieval over a normalised graph, aggressive client caching
  with reference-based optimisation, batched fetching of exactly the
  missing fragments — the natural query/cache layer between ID apps and
  the content-addressed store, and a vocabulary (path sets, refs, atoms)
  for the "search/traverse the graph" API. Contrast to note in the
  prompt: Falcor assumes a trusted central server; ID needs the same
  ergonomics over signed, federated, capability-checked data.

## 8. BLAS / linear algebra — for embeddings, search, and simulation math

"BLAS" here = the dense/sparse linear-algebra layer underneath embedding
similarity search and game/simulation math.

### Rust (host-side, mature)

- **faer** — pure-Rust linear algebra (own SIMD via `pulp`, own GEMM);
  fastest Rust option for dense/sparse decompositions, credible against
  OpenBLAS/MKL; converts to/from ndarray/nalgebra via `faer-ext`. No GPU
  story. (v0.24.x; https://github.com/sarah-ek/faer-rs — primary host now
  Codeberg **[per arewelearningyet]**.)
- **nalgebra** — general-purpose linalg with statically-sized types and
  geometry (points, rotations, isometries); the game/graphics-math choice
  (v0.35.x). Pure Rust, no BLAS dependency.
- **ndarray** — the de-facto n-dimensional array (NumPy analogue) most
  Rust ML code is written against; BLAS/LAPACK via `ndarray-linalg`.
  (v0.17.x; single-maintainer pace — noted risk.)
- Supporting cast for embeddings specifically: **fastembed-rs** (local
  embedding generation), **qdrant** / **lancedb** (vector stores),
  **tantivy** (full-text) — the "text, embeddings and search" job is rated
  _Ready_ in Rust by the ml-rust survey
  (https://ml-rust.github.io/state/, dated 23 Nov 2026 — note the survey
  self-describes as AI-written; treat ratings as indicative).
  Overview source: https://www.arewelearningyet.com/scientific-computing/
- _Relevance to ID:_ host-side, this is solved: an ID host can embed
  faer/nalgebra for vector math and fastembed-style models for embedding
  generation, exposing only _results_ (nearest neighbours, similarity) as
  capabilities. ndarray is the safe interchange format; faer the speed
  layer; nalgebra the app/game math layer.

### Roc (app-side, early)

- **Hasnep/roc-linear-algebra** and **Hasnep/roc-math** — pure-Roc math
  packages (listed in roc-awesome / the roc-packages index).
- **mulias/roc-array2d** — 2D arrays.
- **lukewilliamboswell/roc-pga2d** — 2D projective geometric algebra used
  for the physics demo in §1 (points/translators as multivectors) — shows
  elegant small-scale sim math in pure Roc, not a BLAS.
- _Relevance to ID:_ no Roc-side BLAS exists **[no counterexample found]**;
  app-side numeric work in Roc today is small-scale (2D game math, PGA).
  Anything embedding-scale belongs host-side behind a capability — which
  conveniently matches ID's pure-app/impure-host split and fuel model
  (heavy math is host work, metered as a capability call).

## 9. Rust rendering/game libraries (host implementation options)

- **raylib (Rust crate `raylib` / raylib-rs)** — safe bindings to raylib,
  the same C library roc-ray wraps via Zig. _Fit:_ quickest path to a
  Rust-host 2D surface (sprites, text, input) with a proven API shape;
  parity with roc-ray's surface eases cross-checking.
- **macroquad** — raylib-inspired, immediate-mode 2D framework on
  miniquad; same code to desktop, HTML5, Android, iOS; fast compiles.
  _Fit:_ arguably the best single target for "basic 2D game + sprites +
  text" ID hosts, including Android, without an ECS.
- **miniquad** — the minimal cross-platform window/render layer under
  macroquad. _Fit:_ use directly if ID wants its own thin host surface
  with minimal opinions.
- **bevy** — the large ECS-first Rust engine (wgpu renderer, huge plugin
  ecosystem). _Fit:_ the heavy tier — complex simulations needing an ECS,
  exactly the split Drew described (simple surfaces via raylib-class
  hosts; Bevy hosts for ECS simulations). Cost: big API, fast churn.
- **wgpu** — cross-platform GPU API (Vulkan/Metal/DX/WebGPU) underlying
  Bevy and most serious Rust rendering; **vello** (GPU 2D) and **lyon**
  (tessellation) sit on it. _Fit:_ the long-term rendering substrate if
  ID outgrows raylib-class hosts; too low-level as a first surface.

## 10. Roc package/platform ecosystem sources worth citing

- **roc-awesome** — https://github.com/lukewilliamboswell/roc-awesome
  (Boswell-maintained curated list; platforms incl. basic-cli,
  basic-webserver, roc-wasm4 — a WASM-4 fantasy-console game platform,
  roc-aws-lambda; packages incl. roc-pg (Postgres), roc-json, roc-parser,
  roc-svg, roc-html, unicode). Machine-readable mirror:
  https://awesome.ecosyste.ms/lists/lukewilliamboswell%2Froc-awesome
- **roc-packages index** — https://hasnep.github.io/roc-packages
  (Hasnep's browsable package index; Hasnep also authors roc-math,
  roc-linear-algebra, roc-datetimes, roc-html, roc-svg.)
- **Official examples** — https://www.roc-lang.org/examples/ (now includes
  platform examples: basic-cli incl. an effectful Snake game, and the
  RocRay drawing example — a useful maturity yardstick for roc-ray.)
- **roc-wasm4** — Roc platform for the WASM-4 fantasy console; the
  platform roc-ray's host design borrowed from. _Relevance:_ another
  data point that tiny, fixed capability sets make delightful game
  platforms — a model for ID "micro-surfaces".

---

## Bottom line for the prompt

1. Roc's platform model is the closest existing match to ID's "define
   capabilities once, implement per host, pure apps" architecture — and
   Boswell's body of work (roc-ray, roc-signals, roc-native-android,
   roc-tui/ansi, platform templates, multi-entrypoint compiler work) is a
   one-person proof that desktop-GUI, terminal, browser, server and
   Android hosts can all sit under one app model.
2. The honest caveats to carry into the prompt: Roc is pre-1.0 with a
   compiler/language still in flux (two dialects in the wild); most
   platforms above are single-maintainer experiments; there is no Roc-side
   BLAS (heavy numerics belong host-side — faer/nalgebra/ndarray in Rust).
3. Falcor contributes the client graph-access design (path sets,
   normalised refs, batched fragment fetching, client cache) to adapt —
   with signatures and capabilities added — for ID's graph layer.
