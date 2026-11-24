# Roc Platforms Deep-Dive + A Composable Platform Family for ID Apps

**Date:** 2026-11-23 · **Lane:** ROC-PLATFORMS-DESIGN
**Companions:** `ecosystem-2026-11-23.md`, `blog-catalogue-2026-11-23.md` (§4 cadence, F2/F4),
`talks-and-announcements.md`, `zulip-digest-2026-11-23.md` — all same-day, same folder.
**Method:** primary sources read 2026-11-23 — repo READMEs/release pages (via GitHub),
the RocRay manual (lukewilliamboswell.github.io/roc-ray, v0.12.3), Boswell's
multi-entrypoint post, and the platforms' own comparison docs. Corrections to the
earlier reports are flagged **[CORRECTION]** where they occur.

**The question Drew posed:** what does each of these platforms actually *do* and
*allow* — and could we build our own family of platforms, in whatever languages,
that treats their capabilities as a mix-and-match catalogue, so the host running
an id app decides which capabilities that app gets — including a system-level
wrapper that gives every app one D1, one R2, one KV and meters its bytes, and an
Android permission model over native capabilities?

Short answer: **yes, and Roc is unusually well-shaped for it — because in Roc a
platform is already a capability contract.** The app can do *nothing* except
through functions the platform chooses to expose. The work is not inventing the
security model; it is (1) standardising the catalogue, (2) putting provisioning
and metering in the host where the app cannot reach it, and (3) accepting that
composition happens in *our* platform layer, never in the app's platform slot —
upstream has made "platforms do not compose" permanent policy (FAQ, catalogue F2).

---

# Part 1 — The platforms, one by one

## 1.1 RocRay (`roc-ray`) — 2D games and visual tools over raylib

- **Repo / author / licence:** lukewilliamboswell/roc-ray, Luke Boswell.
  UPL-1.0. Created 2024-01-13; 1,579 commits; **51 releases**, current manual
  0.12.3 (roc-lang.org example pins 0.12.1, nightly-2026-11-12).
- **Host language:** Zig, wrapping raylib (C). Targets macOS (Intel + Apple
  silicon), Linux x86-64, Windows x86-64.
- **What it does:** a host-owned frame loop for programs that draw a fresh
  picture many times a second — arcade/puzzle games, simulations, live plots,
  generative art, small editors. Explicitly *not* a GUI toolkit (no widgets),
  *not* a large engine (no editor, scene graph, physics engine, asset pipeline),
  and 2D only today (3D models/cameras/lighting planned, not present; no raw
  OpenGL/`rlgl` escape hatch).
- **App model / entrypoints:** the app supplies `init!`, `update!`, `render!`
  plus a startup `config`. The host runs the loop and enforces *where* effects
  may happen: drawing only in `render!`; file reads and other waiting work only
  in `init!` or a task; everything else in `update!`. Input is a **value** —
  `update!` receives this cycle's keys, mouse, gamepads, elapsed time and window
  state — rather than polled global state. Resources (textures, sounds, fonts,
  shaders) are released when the model stops referring to them; there is no
  `Unload*`.
- **Capability surface the host grants:**
  - *Always, no declaration:* window, drawing (shapes, images, text, 2D
    cameras, shaders, offscreen render targets, tilemaps), keyboard/mouse/
    gamepad input, audio playback (sounds + streamed music), clocks/timers,
    entropy, terminal output, the read-only directory beside the executable,
    in-memory SQLite, and — once the app names itself with `with_app_id` — its
    own private data/config/cache directories.
  - *Only if declared:* see gating below. Behind declarations sit files and
    directory handles, HTTP, UDP, subprocess commands, environment variables,
    clipboard, screen capture/recording (screenshot + GIF), and file-backed
    SQLite databases.
  - *User-mediated, no declaration:* a file the user drops on the window or
    names on the command line — the user's act of choosing it *is* the grant.
    There is deliberately **no file-open dialog**.
- **The app author's view:** `frame.rectangle!(…)`, `frame.texture!(…)`,
  `Audio.load_sound!(store, "jump.wav")`, `sound.play!()`,
  `dir.read_text!(path)` — methods on opaque handles (`frame`, `store`, `dir`),
  never raw paths or globals. Collision/geometry is pure code in `Math`;
  gravity and collision response are the app's own `update!` code.
- **Capability gating — the most important precedent in this whole report.**
  RocRay is the one platform here with a real, designed permission system
  (manual chapter "Permissions: what your app can access"):
  - The app declares each outside-the-window access in its startup config:
    `.with_permission(Directory("assets", ReadOnly))`,
    `.with_permission(HttpOrigin("https://api.example.com"))`,
    `.with_permission(EnvVar("WEATHER_TOKEN"))`, or `with_permissions([…])`.
  - **"That statement is a declaration, and it is the only permission there
    is: nobody is prompted, and there are no launch flags."** Access is
    visible in the app's source and can be changed only by changing its
    source. Undeclared access is refused by the host.
  - Files are reached through **directory handles, not paths** — a handle is
    minted only for a declared directory (or a user-chosen file), and all
    reads/writes are relative beneath it. This is capability-style scoping,
    not string-path checking.
  - Honest limit (the manual states what it does *not* protect against): the
    declaration is authored by the app itself. It constrains a *well-behaved
    or compromised-library* app and makes its reach auditable at a glance; it
    is not a sandbox against a malicious app author, and there is no
    operator-side grant file separate from the source — yet. That gap is
    exactly where our design (§2.2) adds a host-side grant the app cannot edit.
- **Maturity:** the steadiest community platform in the ecosystem — shipped
  example programs (Breakout, Pong, Snake, Pixel Workshop, Spark Run, Cave
  Climb, Particles, Live Plot, Postcard Studio), a 177-page manual, a
  performance observatory, headless smoke-test mode. Still pre-1.0 Roc: the
  README warns the API may change as Roc changes.

## 1.2 roc-gui — native desktop GUI on GPUI

- **Repo / author / licence:** lukewilliamboswell/roc-gui, Luke Boswell.
  UPL-1.0 (with GPUI's own licence notices vendored). **[CORRECTION:** an
  automated metadata pass reported MIT; the repo page and LICENSE say
  UPL-1.0.**]** Created 2026-09-14; 733 commits; **25 releases in ~10 weeks**,
  0.1.0 line.
- **Host language:** **Rust** (`crates/host`, Cargo workspace) hosting **GPUI**,
  Zed's GPU-accelerated UI framework. Cross-platform native UI: macOS, Linux
  (Wayland), Windows.
- **What it does / allows:** state-driven *widget* applications — the thing
  RocRay refuses to be. The gallery is the capability statement: HTTP
  Workbench, Database Browser, File Explorer, Terminal Workspace, Image
  Library, Music Player, Clipboard History, System Monitor, Settings Center,
  Device Configurator, Animation Studio, Redis Explorer. In other words the
  intended surface is a full desktop-workbench surface: windows/layouts/text
  input, filesystem browsing, outbound HTTP, database/Redis clients, terminal
  hosting, clipboard, media playback, system metrics.
- **App author's view:** a state-driven declarative UI in Roc over GPUI's
  rendering; the host owns the event loop, GPU drawing, and platform widgets.
  Multi-entrypoint host (`init!`/`update!`/`render!`-style symbols) — enabled
  by Boswell's interpreter-shim mechanism (§1.8).
- **Ships with two facilities no other platform here has:** a built-in
  behavioural/performance **test harness** (S-expression scenario format) and
  a **performance observatory streaming to SQLite**. For our purposes that
  observatory is a proof that a Roc host can emit a structured per-app
  telemetry stream as a platform feature — the shape our metering wants (§2.3).
- **Capability gating today:** none documented. The platform's surface is
  effectively all-or-nothing: an app on roc-gui gets the workbench surface.
  No per-app subsetting, no declaration system comparable to RocRay's. This
  is the norm for young platforms, and the reason gating must be *our*
  platform layer's job, not something we wait for upstream to add.
- **Maturity:** README in bold: "still an early-stage platform. **Do not use
  it in production.**" Substantial and fast-moving, but the Roc API is
  explicitly still being developed.

## 1.3 roc-signals — the reactive UI engine underneath the GUI wave

- **Repo / author / licence:** **lukewilliamboswell/roc-signals**, UPL-1.0.
  **[CORRECTION to the ecosystem report, per catalogue delta D1:** the repo
  lives under Boswell, not rtfeldman; Feldman originated the signals design,
  Boswell is the current driver.**]** This incarnation created 2026-06-29;
  1,868 commits; 35 tags. Versioning is two-layered: the *engine package* is
  at **0.8.0-rc1** (the URL Android apps pin), the web platform release line
  reached **0.3.0** in the digest window ("reactive engine overhauled").
- **What it is:** not a host surface but an **engine written in Roc** that
  compiles *with* the app. A platform for interactive browser apps: you write
  the UI in pure Roc; the engine compiles to WebAssembly alongside it and
  updates **exactly the DOM that depends on what changed**. There is **no
  virtual DOM**.
- **The model:** a *signal* is a read-only value whose state dependencies are
  declared when it is built. State lives in three places: a shared **store**
  of cells readable at any depth of the view; **tables** of rows edited by key
  and drawn through sorted/filtered windows; and **local state** belonging to
  one piece of view. Shared state is written only by **decisions** — pure
  functions that *return their writes* — so the engine learns what changed
  from the write itself, never by diffing values. Slow work runs as typed
  **worker tasks** in Web Workers instantiating the same module. Effects are
  described as data and performed by a small JavaScript runtime; **ports**
  (`Port`/`PortJson`) are typed messages out to host-language services.
- **Why it matters here:** roc-signals is the shared engine inside
  roc-native-android (§1.4) and the UI approach of the GPUI-era work. It is
  the clearest example in the ecosystem of a *capability-shaped library*: the
  view runs once, every write declares its footprint, and the host learns the
  app's effects as data before performing them. A host that receives effects
  as data is a host that can **gate, meter, and log** them — §2.2 leans on
  exactly this.
- **Contrast the author himself draws (README, "Roc Signals and Joy"):**
  Joy = one Model, one `update`, view rebuilt per event and diffed by the
  host, `lazy` to skip unchanged subtrees, and server rendering of the same
  views. Signals = fine-grained updates, more to learn, client-only. Both:
  pure Roc UI → wasm, effects as data, small JS runtime, nightly-pinned,
  APIs expected to change.
- **Capability gating:** the engine has no permission system; gating would
  live in the host that performs its requested effects and answers its ports.
  Its contribution to gating is architectural: effects-as-data makes host
  mediation *natural* rather than bolted-on.
- **Maturity:** experimental–active; js-framework-benchmark entries, `.scm`
  behaviour specs that run natively in milliseconds and replay in headless
  Chrome, fuzz targets on its own codecs/lifecycle. Not a settled API.

## 1.4 roc-native-android — native Android apps in pure Roc

- **Repo / author / licence:** lukewilliamboswell/roc-native-android, Luke
  Boswell, UPL-1.0. Created **2026-11-22**; v0.0.1; 20 commits. Self-described
  "early proof of concept… written for a bit of fun, and to help with the
  development of the reactive engine." The launch post discloses Boswell has
  no mobile background and delegated much implementation to an LLM over a
  weekend; emulator-verified only.
- **Host shape:** a small **Kotlin shell** (no Gradle project) + a **Zig JNI
  bridge**. The Roc app is built `roc build --target=arm64musl` into
  `libapp.so`; a companion CLI written in Roc (`android.roc`) scaffolds,
  packages (aapt2/zipalign/apksigner), installs and runs the APK. **arm64
  only**, API 26+.
- **What it allows:** *real Android widgets, no WebView.* The app builds its
  screen from `pf.Ui` builders — columns/rows/cards, lists, scroll views with
  virtualised windows (the contacts example scrolls 10,000 contacts drawing
  only what's on screen), text, buttons, links, text fields, checkboxes,
  switches, sliders, a system file picker. Tapping "+" in the counter updates
  exactly one text view — the roc-signals engine (§1.3) doing fine-grained
  updates against native views instead of the DOM.
- **Effect surface (README "Limits", i.e. the honest list):** timers and
  intervals; **HTTP** via `Engine.http` (HTTPS only in practice — plain
  `http://` is blocked by Android's default network policy; no cookie store
  or HTTP cache); **local storage** persisted across restarts and **session
  storage** for the process lifetime; **device facts** — online status,
  visibility, languages, time zone, `prefers-color-scheme` /
  `prefers-reduced-motion` media queries; **files** — choose via the system
  chooser, read, digest (SHA-256), save, upload, download; and **ports**.
  Workers are *not* supported: an app that asks for one stops with
  "unsupported effect kind". No gestures (drag/long-press/swipe) yet.
- **Ports — the extensibility mechanism, and the seed of a permission
  model:** an app reaches the rest of Android (sensors, notifications,
  camera) through **typed messages to services written in Kotlin**. The
  app's activity supplies services by overriding
  `RocActivity.portServices` (see `DevicePorts.kt`); the shipped `ports`
  example calls device info, Toast, battery level, and the accelerometer
  this way. Crucial limit: an app packaged by `android.roc` **cannot add
  Kotlin services yet** — ports currently work only in the repo's own Gradle
  build. The mechanism exists; the packaging path for third-party services
  does not.
- **Capability gating today:** two layers exist, neither is a per-app
  permission system. (1) *Android's own*: the generated manifest and Android's
  runtime permissions bound what the shell process may do at all — but every
  Roc app inside the shell shares that one grant set. (2) *The engine's
  effect list*: the host supports a fixed set of effect kinds and refuses
  others ("unsupported effect kind") — a coarse, platform-wide gate, not a
  per-app one. There is no manifest of *Roc-level* capabilities, no way to
  give app A the accelerometer port but not the file port. §2.4 designs it.
- **Maturity:** brand-new PoC. Its value to us is the *shape*: signals engine
  + real widgets + typed ports to host services is precisely the shape an id
  Android platform wants; almost everything about it would need hardening.

## 1.5 Joy — full-stack Elm-architecture web framework (Åhdén)

- **Repo / author / licence:** niclas-ahden/joy, Niclas Åhdén. **Apache-2.0**
  **[CORRECTION: the ecosystem report's MIT attribution was wrong; repo and
  `gh` metadata both say Apache-2.0.]** Created 2024-12-10; 256 commits; 5
  releases; current **Joy 0.35.0 + joy-html 0.18.0**.
- **Host language:** Rust (with a `wasm32-unknown-unknown` client build and a
  small JS runtime — the compressed bundle is ~27.5 KB total, of which the
  wasm is ~22 KB and the minified JS runtime ~5 KB).
- **What it does:** *isomorphic* full-stack apps in one language. The app
  exports `Model, Msg, init, update, view, subscriptions` — the Elm
  architecture, on both sides. The same `view` renders to HTML **on the
  server** (via joy-html) and hydrates/runs **on the client**; effects are
  values (`pf.Effect`) returned alongside the model; the roadmap goal is
  convenient front↔back communication in the style of Leptos Server Functions
  / message-passing over SSE, websockets, or plain requests. Its surrounding
  suite is production-shaped: **roc-pg** (PostgreSQL with compile-time-checked
  SQL), roc-openapi, roc-vcr, roc-spec, roc-playwright, roc-sendgrid.
- **Performance evidence:** js-framework-benchmark keyed/non-keyed CPU
  geomean 29.6/26.7 ms — in Elm/Leptos/Solid territory, well ahead of React
  (40.7) in the author's local run; memory 3.6 MB.
- **Capability gating:** none as a permission system — a Joy app's reach is
  whatever its server host process can do (database, network, mail). The
  Elm-architecture discipline does give one audit property: every effect is
  an explicit `Effect(Msg)` value produced by `update`, so the *set* of
  effect kinds an app can emit is enumerable from its types. Nobody has
  built the host-side gate that observation makes possible.
- **Maturity — the paradox to state plainly:** README says "early
  development, not production-ready… expect breaking changes," *and* its
  author has run it in production for ~2 years across real client web apps
  (Func Prog Sweden, 2026-11-10: "Roc and Joy: Productivity in Production").
  Both are true: it is battle-tested by exactly one expert operator, who is
  also its maintainer. Bus factor 1.

## 1.6 roc-cloudflare-workers — Roc on Cloudflare Workers, typed bindings

- **Repo / author / licence:** Karakatiza666/roc-cloudflare-workers, single
  author, UPL-1.0. Created **2026-10-26**; 216 commits; **10 releases in under
  a month**, current 0.0.10 (pinned nightly-2026-11-06). Zero stars; solo.
- **Host shape:** the app compiles to a **wasm32 reactor**; the host is in
  **Zig**; a **JS shim** (`makeHandler`, generated `worker.js`) runs it inside
  workerd. Effects are *direct-style* hosted functions that await JS promises
  through **JSPI** (JavaScript Promise Integration) — the new compiler's
  direct effects (§1.8) doing real work, no Task layer.
- **What it allows — a typed module per Workers API:** the app imports
  `pf.Worker`, `pf.Request`, `pf.Response`, and one module per binding:
  **D1** (SQL, sessions/bookmarks), **R2** (objects, multipart), **KV**
  (with metadata), **Cache API**, **Durable Object SQL** storage (SQLite-
  backed classes, RPC interfaces between objects and across Workers via
  service bindings), **Queues**, **Workers AI**, Vectorize, Email, Tcp,
  Hyperdrive config, rate-limit bindings, cron triggers, tail events.
  The app is a single `event!` entrypoint dispatching through
  `Worker.new({ fetch!, scheduled!, queue!, … , objects: […], bindings: […],
  crons: […] })`; a handler kind with no handler is `Unhandled`.
- **The binding discipline — the second key precedent in this report.**
  A binding is named in Roc: `D1.binding("DB")`, `Kv`/`R2` likewise,
  `Object.namespace("ROOMS", { class: room })`. The app *declares* the
  bindings it uses in `Worker.new({ bindings: [db.declare(), …] })`, which
  lands in the **wasm manifest**. Two independent checkers consume that
  declaration: the shim verifies **at the first event** that `env` actually
  contains each declared binding *with the right kind*, and the companion CLI
  (`roc-cfw sync`) statically checks the Wrangler config against the manifest
  and prints the exact config fragments to add. Cross-Worker RPC interfaces
  are checked the same way (`Rpc.iface("Ledger")` name-matching).
  In other words: **declare-in-code → manifest-in-artifact → verify-at-load →
  verify-in-config**, all typed. That is a capability manifest system, built
  by one person in a month, in Roc, today.
- **Metering hooks already present:** the test helper can *count calls per
  binding* (`count: ["DB"]`; a D1 call = one round trip — `all/run/first/
  raw/batch/exec`; KV `get/getWithMetadata/put/delete/list`; R2 likewise),
  the shim has a `wrapEnv` interception point (its counting is built on it),
  and `makeHandler(module, { stats: true })` emits an `x-roc-stats` header
  with allocator/request numbers. None of this is a quota system — but the
  interception point a quota system needs (`wrapEnv`: every binding access
  passes through one shim function) already exists and is already used for
  counting in tests.
- **Known gaps (author's own list):** no Roc-authored Workflows or
  HTMLRewriter (they need JS→Roc calls), no explicit Durable Object
  transactions, KV-backed DO classes refused, Images/Browser Rendering/mTLS
  unmodule'd (reachable only via a raw `Js` escape program — note: an
  *escape hatch exists*, which any gating design must reckon with), several
  surfaces tested only against Miniflare/workerd locally, not the edge.
- **Capability gating today:** Cloudflare's own model — a binding exists in
  `env` or it doesn't; the platform adds declaration + verification, not
  restriction. Per-app subsetting *within* a Worker isn't a concept (one
  Worker = one app). But composed with one-Worker-per-app deployment, the
  binding set **is** the app's capability set, enforced by the operator who
  writes the Wrangler config. That is Drew's model, already operational in
  miniature (§2.3).

## 1.7 The officials, briefly — basic-cli and basic-webserver

- **basic-cli** (roc-lang, Rust host, **0.28.1**, nightly-2026-11-13+): the
  default general-purpose platform. Surface: Stdout/Stdin (streaming),
  File/Path (typed writer API, file permissions, symlinks), Dir, Cmd, Http,
  Tcp, UnixSocket, **Sqlite**, Tty raw mode, Signal (INT/TERM/USR1/USR2/
  WINCH), Env, Locale, Utc/time. Gating: none — a basic-cli app has the
  whole surface; the process's OS permissions are the only boundary.
  Significance for us: it is the *reference catalogue* of what a mature Roc
  platform module list looks like, and the module set our server platform
  would start from.
- **basic-webserver** (roc-lang, Rust host on hyper + tokio, **0.22.0**,
  nightly-2026-11-15; 0.17→0.22 in ~six weeks): HTTP request/response plus
  **typed server-sent events**, pooled Sqlite, pooled outbound HTTP,
  bounded request bodies, forms/static files, server lifecycle. Two design
  choices worth stealing: (1) **finite defaults as a runtime contract** —
  admitted SSE responses (256), framed SSE event size (1 MiB), body bounds,
  with deliberate overload behaviour instead of unbounded queues; (2) a
  **host-owned observation point** — opt-in access logging/metrics configured
  in `init!`, where the host observes each request through exactly one
  terminal event (response end / failure / drop), covering Roc responses,
  native files, overload and disconnect uniformly. That terminal-event
  discipline is precisely the accounting point a metering host needs.
  Gating: none per-app; the listener defaults to `127.0.0.1:8000` and durable
  state is pushed to SQLite/external services by design.

## 1.8 The two cross-cutting mechanisms

**Multi-entrypoint hosts (Boswell, 2025-08-30).** GUI/game/Android hosts need
several entrypoint symbols (`init!`, `update!`, `render!`); Roc's prebuilt
interpreter shim exposed only one generic `roc_entrypoint`. Boswell's fix:
at runtime the CLI generates a tiny LLVM-bitcode adapter (via Zig's
`std.zig.llvm.Builder`) defining each host symbol and forwarding to the shim
with an `entry_idx`. No host ABI change; fast interpreted dev loops for
multi-entrypoint platforms. This post is the enabling infrastructure under
the entire autumn-2026 platform burst — and, for us, the proof that a host
can present *many* narrow entrypoints (per-capability surfaces) without
compiler surgery.

**Direct effects in the new compiler.** The old design returned `Task`
values the platform interpreted. The new compiler kept the `!` sigil and
replaced the semantics: effectful functions are called **directly**
(`=>` in types), the platform's hosted functions *are* the effects. The
Cloudflare platform's JSPI-awaited hosted functions and basic-webserver's
model both run on this. Consequences for gating are §2.2's subject: direct
effects remove the "effects as inspectable data" property Task had — except
where an engine (roc-signals) deliberately reintroduces it above the
platform layer.

### Gating summary (Part 1's bottom line)

| Platform | Per-app subsetting today? | Mechanism present |
|---|---|---|
| RocRay | **Partial — the real thing, app-authored** | Source-level declarations, host-enforced, handle-scoped files |
| Cloudflare Workers | **Operator-side, by deployment** | Typed binding declarations → manifest → load-time + config verification; `wrapEnv` interception; per-binding call counting in tests |
| Android | Coarse only | Android OS permissions (per-shell, not per-app); fixed host effect-kind list |
| roc-gui | No | All-or-nothing surface; (observatory proves telemetry is buildable) |
| roc-signals | No (by design — it's an engine) | Effects-as-data *enables* a host gate |
| Joy | No | Effect values enumerable by type; no host gate built |
| basic-cli / basic-webserver | No | OS process boundary; finite-limit contracts; host observation points |

Nobody ships "operator grants a subset to a third-party app." Two platforms
ship the two halves of it: RocRay ships *declaration + host refusal*;
Cloudflare ships *declaration → manifest → operator-side verification +
interception*. Our design is those two halves joined, with the grant held by
the operator instead of written by the app author.

---

# Part 2 — The design: an id platform family

## 2.1 The composition problem, and why it doesn't kill the vision

Roc's rule is absolute and now written into the FAQ as a permanent refusal:
**an app binds to exactly one platform; platforms do not compose.** Drew's
vision — mix-and-match capability functions, chosen per deployment — appears
to ask for the forbidden thing. It survives by relocating the composition:

- **Composition happens at platform-construction time, in our layer, not at
  app-binding time.** We ship *one id platform per target class* —
  `id-server`, `id-android`, `id-desktop` (and later `id-edge` on the
  Cloudflare shape) — each a single ordinary Roc platform. Inside each
  platform, the host is assembled from a **shared capability catalogue**:
  a versioned list of capability modules (`cap.files`, `cap.http.client`,
  `cap.http.server`, `cap.sql.d1`, `cap.objects.r2`, `cap.kv`,
  `cap.queue`, `cap.ui.widgets`, `cap.render2d`, `cap.sensors`, …), each
  with a typed Roc API surface, a host implementation, a declaration syntax,
  and a metering schema. Two deployments of `id-server` can enable
  *different catalogue subsets* — the platform binary is built (or
  configured) per deployment profile, the way the Cloudflare shim is
  generated per app manifest.
- **The app declares; the operator grants; the host enforces.** The app
  states the subset it uses (RocRay-style declarations, generalised). The
  operator's deployment config states the subset it *grants* — and the
  grant, not the declaration, is authoritative: an app that declares
  `cap.sensors.camera` on a deployment that grants only
  `[cap.kv, cap.sql.d1]` fails at load with the ungranted names listed,
  exactly as `roc-cfw sync` fails a Wrangler config that doesn't match the
  manifest. Declaration-without-grant is a build/deploy error, never a
  runtime surprise.
- **What is portable across targets is the catalogue, not the binary.** The
  same capability *name, Roc API, declaration and metering schema* appear in
  `id-server` (backed by real D1), in `id-android` (backed by local SQLite /
  on-device storage), and in a test platform (backed by in-memory fakes —
  the Cloudflare test helper's seeded-Miniflare pattern). An id app's pure
  logic and capability calls stay identical; only the platform line in its
  header changes. This is platform-switching, which Roc has always
  supported, used as the composition seam.

**Where direct effects help and hinder.** *Help:* hosted functions are just
functions — a host can implement a capability function as a **checking
wrapper** (grant check → meter → delegate) with no interpreter to build,
and JSPI/native async composes normally; per-call overhead is one branch
and one counter. *Hinder:* with Task gone, effects are no longer data the
host can inspect *before* performing them; you cannot statically enumerate
"what will this app do?" from a Task chain, because there is no chain.
Mitigations, in order of strength: (1) **the declaration is the static
statement** — the compiler already guarantees the app can only *name*
capability functions its platform exposes, so a platform built from a
granted subset makes ungranted capabilities **unnameable**, which is
stronger than any runtime check; (2) where effects-as-data is wanted
(UI surfaces), keep the roc-signals pattern — decisions return writes,
the host applies them — recovering inspectability exactly where the
attack/abuse surface justifies it; (3) runtime wrapper checks as
defence-in-depth for capabilities granted but *scoped* (this directory,
that origin, this byte budget), where the scope can't be expressed by
omission.

The honest summary: **omission is the gate, wrappers are the fence,
declarations are the audit.** Roc's one-platform rule forces the gate to
be omission — which happens to be the strongest form.

## 2.2 The capability catalogue (the artefact we would actually maintain)

Each catalogue entry is a contract with five parts:

1. **Name + version** (`cap.sql.d1@1`) — appears in app declarations,
   operator grants, manifests, and metering records under one string.
2. **Roc API surface** — the module the app imports (`Id.D1` with
   `binding`, `rows!`, `run!`, …), modelled on the Cloudflare platform's
   typed modules, including its error tags (`BadParam`, …) so failures are
   values, not crashes.
3. **Declaration + grant syntax** — RocRay-shaped: app-side
   `.with_permission(...)`-equivalents generalised to scopes
   (`Directory("data", ReadWrite)`, `HttpOrigin("https://…")`,
   `D1Binding("DB", [Read])`); operator-side grant files per deployment.
4. **Host implementation(s)** — one per target class, in whatever language
   fits the host (Zig for edge/Android bridge, Rust for server/desktop,
   Kotlin for Android services, JS for the Workers shim). The contract is
   the Roc API + the metering schema; the language is an implementation
   detail (§2.5).
5. **Metering schema** — the countable events and their units (§2.3):
   for storage caps: bytes-read, bytes-written, rows-read, rows-written,
   objects, keys; for network: requests, bytes in/out, origins; for
   compute-ish caps: calls, wall time. Defined *in the catalogue*, so
   every host implementation meters identically and bills/limits are
   comparable across targets.

Existing platforms map onto the catalogue as prior art, per capability:
files/directories from RocRay (handle-scoped) and basic-cli (typed writer
API); HTTP server + SSE + finite limits from basic-webserver; D1/R2/KV/
Queues/Durable Objects from roc-cloudflare-workers; widgets from roc-gui
and roc-native-android's `Ui`; 2D rendering from RocRay; reactive state
from roc-signals; ports from roc-native-android. We would be *cataloguing
and re-hosting* proven surfaces, not inventing APIs from a blank page.

## 2.3 The provisioning/metering wrapper (Drew's D1/R2/KV sketch, designed)

**The sketch:** system-level apps wrap the capabilities so that *every app
gets one D1 database, one R2 bucket/prefix, one KV namespace*; apps import
the same libraries; *we* limit the bytes each app may read and write.

**Where things live.** Three distinct roles, and the design's first rule is
not to blur them:

- **The provisioner** (a system app / operator tool, *not* in the app's
  process): on app registration, creates-or-assigns the app's D1 database,
  its R2 bucket (or a dedicated prefix in a shared bucket — bucket-per-app
  is cleaner for lifecycle and deletion; prefix-per-app is cheaper at
  thousands of apps; the catalogue contract should hide which), and its KV
  namespace. It writes the app's **grant record**: app id, capability set,
  binding names → real resource ids, and **budgets** (below). This is the
  Wrangler-config role, generalised: the Cloudflare platform proves the
  declare→manifest→verify loop; the provisioner adds *create* to the front
  of it and *budgets* to the record.
- **The host shim** (in the serving process, *outside* the app's reach):
  every capability call passes through the host's hosted functions — there
  is nowhere else for it to pass; that is what a Roc platform *is*. The
  shim resolves the app's binding names to its provisioned resources
  (the app literally cannot name another app's database: its `D1.binding
  ("DB")` resolves through the provisioner's map, and the underlying
  resource id never appears in app code), checks the grant, **meters**,
  and delegates. The Cloudflare platform's `wrapEnv` is this interception
  point in embryo; its test helper's per-binding call counting is the
  metering loop in embryo.
- **The app**: imports `Id.D1`, `Id.R2`, `Id.Kv` and writes ordinary code.
  It holds no credentials, no resource ids, no quota logic.

**Metering.** Count in the shim, at the same granularity the Cloudflare
test helper already counts (per binding, per operation kind), extended
with the byte dimensions Drew named:

- **D1:** rows read / rows written (D1's own billing dimensions) **plus**
  bytes: request SQL + parameters in, result rows out (serialised size).
  Byte-accurate result sizes are available at the shim because every row
  crosses it (the platform's CBOR layer already measures what it encodes).
- **R2:** bytes read (GET/HEAD responses), bytes written (PUT/multipart
  parts), object count, storage byte-days if we want a holding cost.
- **KV:** reads, writes, deletes, lists; bytes in values in/out.
- Counters live in a **metering store the app cannot write** — in our
  estate's terms, a row per (app, capability, window) in the operator's
  own D1/Turso plane, flushed by the host, never by the app. Windows:
  per-minute (burst), per-day and per-month (budget), aligned with the
  Turso→D1 audit's D1 plane: the wrapper's natural home is the same D1
  estate G1–G3 migrate the PM/dashboard plane into, and its counters are
  exactly the kind of telemetry-mirror workload that audit classified
  MOVE-CLEAN. Per-scope metering walls already exist in our route layer
  (the `openai-payg-free` wall, #13839); this is the same pattern at the
  app-platform layer: **count locally, wall at the scope, reset on the
  window.**

**Enforcement: deny, don't throttle — with one exception.** When a budget
is exhausted the shim returns the capability's ordinary error tag
(`Err(QuotaExceeded)` in the module's error union) rather than slowing
down: (1) errors-as-values is Roc's idiom and the app's `Try` handling
already has a path for it; (2) a throttled storage layer creates
partial-write states that are far harder to reason about than a clean
refusal; (3) denial is *auditable* — the refusal is itself a metered
event. The exception is **burst shaping on shared-fate resources**
(inbound request admission, SSE connections, outbound HTTP): there,
basic-webserver's finite-defaults contract is the model — bounded queues,
deliberate overload behaviour — because the resource being protected is
the host, not the app's budget. Budgets get **soft/hard pairs**: a soft
line that emits an operator event (and, in the fleet's idiom, a PM-stack
notification) and a hard line that denies. First offence shape: warn at
80%, deny at 100%, reset at window close; all three numbers are grant-
record fields, not code.

**What the Cloudflare platform proves is already buildable in Roc today:**
typed per-API modules over D1/R2/KV with Roc-idiomatic errors; a binding
declaration that compiles into a wasm **manifest**; load-time verification
of that manifest against the live environment; config verification as a
CLI; per-binding call counting; an env-interception point in the shim;
and apps that run against seeded local fakes (Miniflare) with the *same*
manifest. Missing from it, and ours to build: provisioning (it assumes
Wrangler/an operator made the resources), byte-level metering (it counts
calls), budget enforcement, and multi-app tenancy (it is one-app-per-
Worker by construction). None of the missing four requires a Roc language
change; all four live in the shim/provisioner layer. **That is the
buildable-today core of Drew's sketch.**

**Mapping onto our estate:** id apps are the tenants; the fleet's broker/
PM plane is the operator; D1 is the provisioned store (with Turso staying
the broker-core/embedding store per the audit's STAY-TURSO verdict — an
app that needs vector search gets a `cap.vector.turso` catalogue entry
rather than a D1 pretence). Grant records are natural broker-side state;
the deny-path error is a natural fit for the id crate's existing
capability/ceiling vocabulary (Meadowcap-style attenuation: a grant *is*
an attenuated capability set, and sub-grants to sub-tasks are attenuation
with smaller budgets — the P2P synthesis's composition A/C language
applies verbatim at this layer).

## 2.4 The Android permission model

**Starting point (§1.4):** roc-native-android gives us real widgets, the
signals engine, a fixed effect-kind list, system-mediated file picking,
and **ports** to Kotlin services — inside one Kotlin shell whose Android
manifest permissions every hosted Roc app currently shares.

**The model we would layer in, in four strata:**

1. **Android's own permissions (unchanged, outermost).** The shell's
   manifest declares the union of permissions any hosted app *could* be
   granted (INTERNET, sensors, notifications, …), and Android's runtime
   permission flow remains the user's OS-level consent. Nothing we build
   replaces this; we build *inside* it.
2. **The id-app manifest (ours, per app).** Each id app ships a Roc-level
   capability declaration — the same catalogue names as everywhere else:
   `cap.fs.appdata`, `cap.fs.picker`, `cap.http`, `cap.net.info`,
   `cap.sensors.accelerometer`, `cap.sensors.battery`, `cap.notify.toast`,
   `cap.ui.widgets`, `cap.render2d`, `cap.storage.local`, `cap.clock`, …
   The declaration is compiled into the app artifact (the Cloudflare
   manifest pattern) and is *also* rendered for humans: an install screen
   that says, in plain words, what this app may do — RocRay's "what your
   app can access" chapter, turned into an install-time document.
3. **The operator/user grant (authoritative).** The device-side host
   holds the grant: declaration ∩ policy. Policy sources, in precedence
   order: the fleet's device management (for our own devices), the
   installing user's toggles (per-capability, revocable at runtime —
   Android's settings idiom), and the app's declaration as the ceiling
   (a grant can never exceed a declaration; an app cannot talk its way
   into a capability it didn't declare, because — §2.1 — in a platform
   built from the granted subset the ungranted capability is
   *unnameable* in its API surface).
4. **Host enforcement (innermost, mechanical).** Three enforcement
   points, all host-side: (a) **ports are registered per grant** — the
   Kotlin shell only registers a `portServices` entry for a granted
   capability, so an ungranted sensor port *does not exist* for that
   app (today's `RocActivity.portServices` override becomes a
   grant-driven registry); (b) **effect kinds are checked per app** —
   today's platform-wide "unsupported effect kind" refusal becomes
   "ungranted effect kind" for this app, same code path, finer key;
   (c) **filesystem scope is handle-based** — app-data directory by
   default; anything wider only via the system picker (user's act =
   grant for that file, RocRay's rule) or a declared+granted directory
   handle; raw paths never cross the API.
   **Rendering as a grant** falls out naturally: `cap.ui.widgets` (the
   `Ui` builders) and `cap.render2d` (a RocRay-style canvas surface) are
   different catalogue entries with different host entrypoints; a
   background/sensor app holds neither and *cannot draw*; a game holds
   `render2d` but not, say, `fs` beyond its app-data handle. Because the
   signals engine updates views through the host, even a granted UI
   capability is mediated — the host sees every view mutation the app
   requests.

**What roc-native-android would need to grow to carry this:** (i) the
packaging path for third-party Kotlin port services (today ports work
only in the repo's Gradle build — the single biggest gap); (ii) a
per-app (not per-shell) service registry and effect-kind table;
(iii) a manifest step in `android.roc` that embeds the capability
declaration into the APK and generates the human-readable grant screen;
(iv) multi-app hosting or a disciplined one-shell-per-app packaging so
grants can't bleed across apps sharing a process; (v) gestures and the
remaining widget surface, for product completeness rather than for the
permission model. Items (i)–(iv) are host/tooling work in Kotlin/Zig/Roc
CLI code — none needs upstream Roc changes, but all of it is *ours to
own* if we build on a 20-commit PoC (§2.5).

## 2.5 Honest limits

- **Pre-1.0 churn is the tax on everything.** Roc is at 0.1.0-RC1
  (2026-11-21, Zulip-announced, zero blockers); every platform pins a
  *nightly*, APIs are explicitly unstable, and the Cloudflare platform
  shipped breaking changes in most of its ten releases. A platform family
  built now must budget a standing "nightly bump" chore per platform —
  small per bump (the ecosystem does it fortnightly), fatal if ignored
  for a quarter.
- **The ecosystem is ~5 people.** Boswell alone is basic-cli, RocRay,
  roc-gui, roc-native-android, roc-signals' current driver, roc-parser
  and the awesome list. Åhdén is Joy + roc-pg + the production suite.
  Karakatiza666 is the Cloudflare platform + most data-format packages.
  Feldman is the compiler. Consuming any of these is a dependency on a
  person; the project's own $4k/month fundraising goal is the tell.
- **Fork vs consume, priced honestly:**
  - *Consume* (pin a release URL, track upstream): cheapest now; you
    inherit their roadmap and their breaking changes; viable for
    RocRay-shaped and Cloudflare-shaped surfaces where our changes are
    additive (catalogue wrappers *around* their modules).
  - *Fork* (copy the platform, own it): you own the nightly-bump chore,
    the host bugs, and the divergence — forever. Justified only where
    the platform *is* our product surface: the id-server and id-android
    hosts, where grant enforcement and metering must be in code we
    control. Even there, fork the *shape* and track upstream's API
    evolution socially (Zulip) rather than merging code.
  - *Re-host* (our host, their API): the middle path the catalogue
    enables — keep the Roc API surface compatible with upstream's so
    apps and examples port, implement the host ourselves where
    enforcement must live. This is the recommended default for the
    storage caps, and it is what "platform-as-capability-contract"
    means in practice.
- **The multi-language point (Drew's "may write in different
  languages"):** nothing in the design requires our hosts to be Roc or
  to share a language with each other. The portable artefact is the
  **contract**: catalogue name, Roc API module, declaration/grant
  syntax, manifest format, metering schema. Hosts already demonstrate
  the spread — Zig (RocRay, Cloudflare, Android bridge), Rust
  (basic-cli, basic-webserver, roc-gui, Joy), Kotlin (Android shell),
  JS (Workers shim, signals runtime). Roc's platform ABI is a C-shaped
  boundary; a host in Go or Odin (Rocco's engine is Odin) is ordinary.
  What must *not* fork is the contract: two hosts implementing
  `cap.sql.d1@1` with different error tags or different metering units
  would silently break the family's one promise — same app code, same
  limits, any target.
- **What this does not give us:** a sandbox against malicious *native*
  code (Roc apps are compiled code in the host process; the capability
  model governs what the platform *offers*, and Roc's purity makes the
  offer meaningful — an app cannot syscall around its platform in pure
  Roc — but a compromised compiler/host is out of scope, as it is for
  RocRay's own stated threat model); upstream platform composition
  (never coming, by policy); and a stable 1.0 to build on (RC1 is
  close, not landed).

## 2.6 The build order this implies (smallest first proof)

1. **Catalogue v0** — names + Roc APIs + metering schemas for the
   storage trio (`cap.sql.d1`, `cap.objects.r2`, `cap.kv`) and
   `cap.http.server`, lifted from the Cloudflare modules and
   basic-webserver's limits contract.
2. **A shim prototype on the Cloudflare shape** — one app, provisioner-
   created D1/R2/KV, `wrapEnv`-style interception, call+byte counters
   into an operator D1, hard-deny at a tiny budget, proven by the
   platform's own test-helper pattern (seeded fakes, counted calls).
3. **Declaration→grant verification** — RocRay's declaration syntax +
   `roc-cfw sync`'s manifest checking, with the operator grant file as
   the authority.
4. **Android strata 2–4** on top of roc-native-android's ports, in the
   repo-build form first (where ports work today).
5. Only then: desktop (`roc-gui` surface) and `render2d` as grantable
   caps, and the second target class that proves the catalogue is
   actually portable.

---

*Sources: repo pages and READMEs for roc-ray, roc-gui, roc-signals,
roc-native-android, joy, roc-cloudflare-workers, basic-cli,
basic-webserver (read 2026-11-23); RocRay manual v0.12.3, esp.
"Permissions: what your app can access", "Files, assets, and databases",
"Coming from raylib"; roc-cloudflare-workers README/MODULES summaries
incl. binding manifest, `sync` verification, test-helper counting and
`wrapEnv`; roc-signals README incl. its Joy comparison; roc-native-android
README incl. Limits and Port sections; Boswell, "Roc Platforms:
Supporting multi-entrypoint hosts" (2025-08-30); roc-lang.org FAQ
(platform composition refusal). Version/star/commit figures as of
2026-11-23 and will drift.*
