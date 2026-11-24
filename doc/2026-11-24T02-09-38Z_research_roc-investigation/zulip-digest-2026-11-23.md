# Roc Zulip digest — 2026-10-24 → 2026-11-23

*Compiled 2026-11-23 from roc.zulipchat.com via the Zulip REST API, using Drew's
own account (Drewry Pope, user 747334, member since 2024-08-20).*

**Access used:** an existing `zuliprc` on the laptop — `/home/user/zuliprc`
(mode 600; `[api]` section, site `https://roc.zulipchat.com`, key stored under
the `key=` field) — verified live against `GET /api/v1/users/me`. The key was
used in place on the laptop only and appears nowhere in this document. Also
present on the laptop: a Zulip desktop-app profile at `~/.config/Zulip`
(Electron app data). Chromium's Default profile has no roc.zulipchat.com
history, so the browser route was not needed.

**Coverage:** all 19 public streams in the realm were fetched for the full
30-day window — 2,355 messages, 2026-10-24 09:15 UTC → 2026-11-23 21:12 UTC.
Drew is subscribed to 11 of the 19; the other 8 are public and were included
because two of them (compiler development, platform development) are central.
Five streams were simply quiet (0 messages): API design, book – software
design by example in roc, editor, roctoberfest, software unscripted podcast.
Not covered: direct messages and any private streams Drew is not a member of.
Method note: every message was aggregated by stream/topic/reaction count;
announcements, compiler development and platform development were read in
full; the highest-traffic and highest-reaction topics elsewhere were read in
depth; routine Q&A (most of #beginners) is summarised at topic level.

**Drew mentions:** none. Drew posted 0 messages and was mentioned 0 times in
the window — nothing is waiting for him personally.

---

## TL;DR — the five things that matter

1. **Roc 0.1.0-RC1 shipped on 2026-11-21** — the first release candidate in
   Roc's history, from the new (Zig) compiler. As of 2026-11-23, *no blocker
   bugs have been reported*, and Feldman says RC1 may be promoted directly to
   0.1.0. An RC2 is still his base expectation.
2. **The new compiler is simply "the compiler" now.** Nightlies, the RC, and
   package ports all target it; design discussion assumes it.
3. **Linear types: Feldman said no** (2026-11-23), with a detailed
   "cure worse than the disease" argument. FBIP is Roc's answer.
4. **A live community controversy over LLM-generated code** — a long-time
   contributor's "Goodbye" post (2026-11-19) and a 125-message thread about
   Zig's LLM policies. No policy decision yet; Feldman is explicitly still
   thinking about it.
5. **The ecosystem is visibly shipping**: an official VS Code extension,
   signed macOS nightlies, Homebrew install, the Exercism track on RC1, a
   game engine hitting v0.10.0 with web builds, reactive notebooks, a
   Convex-style platform — and a community showcase being assembled for the
   0.1.0 blog post.

---

## 1. Roc 0.1.0-RC1 — the release candidate (announcements, 2026-11-21)

Richard Feldman's announcement (47 messages, 71 reactions — the most-reacted
topic of the month):
[link](https://roc.zulipchat.com/#narrow/channel/397893-announcements/topic/Release%20candidate%3A%20Roc%200.1.0-RC1/near/642546747)

- RC1 is commit `15177e91f6`; binaries on GitHub. This is Roc's **first-ever
  RC process** — Feldman started it partly to establish the process itself.
- **The rules between RC1 and 0.1.0:** the existing feature freeze stays; no
  breaking API changes to stdlib or the host boundary unless unavoidable for
  an important bug; **no performance improvements at all** (compiler or
  runtime) until 0.1.1, which is planned "shortly after" 0.1.0.
- **Bugfix bar for 0.1.0:** only bugs that (a) block real-world Roc programs
  (hobby projects and learning materials count) *and* (b) can't be worked
  around. Everything else — even compiler crashes — defaults to 0.1.1, with a
  deliberately high bar for exceptions.
- **Status as of 2026-11-23:** Feldman, surprised: *"has anyone actually
  encountered a blocker bug in RC1? It seems … RC1 is eligible to be promoted
  directly to 0.1.0."* Community reports agree: packages and platforms updated
  without issue (Niclas Ahden; kaza's roxum; pixelsprout's rocco-engine;
  Bryce Miller's Tigerbeetle client and roc-pg). Roux builds except
  [roc#13382](https://github.com/roc-lang/roc/issues/13382), which Zak Kohler
  worked around. Two non-blockers noted: a basic-cli `Cmd.exec!`
  process-replacement quirk when running via `roc <file>` (issue to be
  filed), and an investigated cold-`roc check` slowdown on one small example
  (0.05 s CPU on the 2026-11-03 nightly vs 1.00 s on RC1 on a noisy machine;
  Steve Howell's write-up hedges heavily on measurement noise).
- **RC1 build-time benchmarks** (Feldman, on a dozen showcase projects):
  `roc check` and `roc build --opt=dev` are sub-second even cold; warm
  rebuilds under 100 ms. `--opt=dev` was the 0.1.0 priority; speeding up
  `--opt=speed`/`--opt=size` gets a priority bump after 0.1.0 (discussed at
  the Nov 22 online meetup).

## 2. The new compiler — state and decisions

- **It's the shipping compiler.** Community language has flipped: packages are
  "ported over to the modern compiler" (plume 0.5.0), rocup was fixed "for
  the new zig compiler", and the Exercism track runs on 0.1.0-RC1.
- **Memory management** (#compiler development, "remove morphic?", Nov 13–18):
  the new compiler does **not** use Morphic — it implements the successor
  algorithm from the Morphic team's own paper
  ([ACM](https://dl.acm.org/doi/pdf/10.1145/3798221)), described by Feldman as
  "a more powerful alternative" to both Morphic and Perceus-style refcounting.
  A long perf thread in the same topic ended well: PR
  [roc#13090](https://github.com/roc-lang/roc/pull/13090) brought unmodified
  functional-style code to within ~3% more instructions than a hand-written C
  port (a `.ok_or` default was defeating in-place buffer reuse; `List.update`
  also lacked the in-place-mutation eligibility check `List.set` has — being
  fixed). GordonBGood confirmed on the Nov 18 nightly that functional style
  no longer carries an execution-time penalty.
- **Breaking change announced 2026-11-12** (#announcements, "breaking changes:
  operator desugaring and shadowing", 16 msgs): a default-number-resolution
  bug forced changes to operator desugaring — operators no longer require
  both operands and the result to share one type, enabling e.g.
  `my_duration * 12` (Duration × U64) instead of forcing the literal to be a
  Duration.
- **Regression found and filed:** unwrapping an `Ok` in `main!` stopped
  compiling (works at top level) — Niclas Ahden filed
  [roc#13281](https://github.com/roc-lang/roc/issues/13281) at Feldman's
  request (Nov 20).
- **Zig toolchain:** a **Zig 0.17 arena double-allocation bug** "can plausibly
  bloat Roc programs" (#bugs, Oct 26). Anton started the **Zig 0.17.1
  upgrade** on Nov 5. Roc was also submitted to **Compiler Explorer**
  (godbolt) via a community PR (Nov 16).

## 3. Tooling and distribution

- **Official VS Code extension** (announced Nov 5, Anton): syntax
  highlighting (including Roc in doc comments/Markdown), diagnostics as you
  type, formatting (format-on-save default), hover with inferred types.
  Publisher verification is pending on Microsoft's side. Luke Boswell's fork
  inspired "vital components".
- **Signed macOS nightly binaries** (Nov 20, 18 reactions): Intel and Apple
  Silicon nightlies are now officially signed — no more Gatekeeper warnings.
- **Homebrew install** announced Nov 21; Anton will also make a PR to add Roc
  to **mise**.
- **Exercism:** the Roc track is now running on 0.1.0-RC1 (Nov 22).
- **ReleaseSafe nightlies** are now published alongside regular nightlies
  (Anton, in the LSP-bug thread) "for people who want that extra security".
- **LSP outage fixed:** the month's biggest bug thread — "LSP does not work
  after latest nightly" (#bugs, 79 messages, Oct 25 → Nov 14) — was confirmed
  fixed on Nov 13–14 ("yes, now it works, thanks!"). The thread also ranged
  into LSP/compiler-memory security trade-offs.

## 4. Language-design conversations and decisions

- **Linear types — decided against** (#ideas, 48 msgs, Nov 22–23). Steve
  Howell asked for linear types to enforce resource cleanup (auto-close) in
  his roc-chat server. Feldman's answer, in detail: verifying compile-time
  uniqueness requires either a very restrictive memory model (e.g. Goose) or
  "approximately the same amount of type-system complexity and annotation
  verbosity as Rust" — *"the cure is worse than the disease."* Roc's answer
  remains **FBIP** (Functional But In-Place), which Feldman notes came from
  Koka via Folkert's thesis adviser. Expect this request to keep recurring;
  the reasoning is now on record.
- **Reflection / reification — the biggest design conversation of the month**
  (#ideas): "SurrealDB query builder based on reifying" (270 msgs) and
  "Reflection — typed function bodies as Roc values (reifying)" (194 msgs).
  Karakatiza maintains a compiler fork with working reification —
  [github.com/Karakatiza666/roc/tree/reify](https://github.com/Karakatiza666/roc/tree/reify)
  — with design docs and examples, used to build type-safe query builders.
  This is experimental-fork work, not a mainline commitment, but it's where
  the typed-reflection design is being hashed out. Related: "Type reflection
  — Zig `@typeInfo()` equivalent" and a CBOR-serialization design thread
  (39 msgs).
- **Compiler API / introspection — wanted, not planned** (#platform
  development, "Compiler API?", Nov 22): Jack Scully's Convex-style platform
  needs an app's exported endpoints with resolved types; today that means
  hand-rolled parsers or `--introspect` hacks. Luke Boswell pointed to an
  old tooling gist; Feldman is cautious — in the linear-types thread he
  generalised: exposing IR/APIs makes them unchangeable ("a main reason Atom
  got surpassed by VS Code"), so *"be very careful about what APIs to
  expose."*
- **Smaller design threads:** "Reworking Rust Glue" (19 msgs — Karl's
  RustGlue.roc may replace the repo's glue scripts as the recommended
  starting point); "requiring more type annotations?" (18 msgs); "In file
  evaluation/repl" (17 msgs — Kilian Vounckx, returning after a long absence,
  proposed REPL-style in-file evaluation; nothing decided).

## 5. The documentation push (#contributing)

The "Docathon?" thread (144 msgs, **73 reactions — the most-reacted topic
overall**) became a month-long docs effort ahead of 0.1.0:

- **15 new code examples on the website** (Aurélien Geron, Oct 27):
  AdvancedParsing, Loops, NestedTypes, NominalTypes, OpenRecords, Paths,
  PatternAliases, Polymorphism, RecursiveTypes, RocRay, Sets, Streams,
  StringPatterns, Unicode, WrappingArithmetic — reviewed by Anton and Luke
  Boswell.
- The mini-tutorial **will be replaced before 0.1** (Anton).
- Langref expectations set explicitly to "after 0.1.0" (PR #12758) — Feldman:
  "no need to promise things unnecessarily."
- **New website search is live** (Nov 1).
- Steve Howell is writing **"a little Roc book"** — early draft at
  [roc.lynrummy.com/tutorial](https://roc.lynrummy.com/tutorial/introduction.html).
- A community **Roc style guide** (Steve Howell, LLM-drafted, human review
  requested) drew concrete state-machine idiom rules from notramo in
  #beginners (96-msgs thread).

## 6. The controversy: LLM-generated code and community culture

The month's most emotionally charged conversation, worth knowing about
because it may produce policy:

- **"Goodbye"** (#off topic, 52 msgs, 65 reactions, Nov 19–21): Hannes — an
  early, multi-year contributor (he built
  [hasnep.github.io/roc-packages](https://hasnep.github.io/roc-packages)) —
  announced he's leaving. His reason, in his own framing: show-and-tell is
  now dominated by LLM-generated code he learns nothing from ("if I notice
  any interesting decisions in the code I doubt the author will be able to
  explain it"), chat is full of "claudeisms", and he prefers the Gleam
  community, which "seems to value effort over output". He explicitly does
  **not** think Roc should be anti-LLM like Zig.
- A sibling thread — "roc on a unikernel (venting about LLM policies)"
  (#show and tell, 125 msgs) — started from the Zig project auto-closing an
  LLM-disclosed issue with a clear reproducer, and ranged widely over
  upstream-project LLM policies.
- **Feldman's response** (Nov 20): warm goodbye to Hannes; on policy, *"short
  answer: I'm not sure, need to think about it more based on what other
  communities have done"*; every community is going through some version of
  this and compositions are changing either way; his stated personal view is
  to advocate common ground and kindness. **No policy has been adopted.**
- Constructive proposals floated: self-reported "human-code scores" /
  AI-declaration standards for shared projects (removewingman links
  ai-declaration.md). Anton gently pushed back that the team's actual focus
  has been "solving bugs, improving performance and updating the various
  packages and platforms."

## 7. Platforms and projects — what's new or shipping

Platforms:

- **concave** (Jack Scully, Nov 23) — Convex-inspired reactive web platform
  on 0.1.0-RC1: serverless, scale-to-zero, front+back reactivity, deploys to
  Google Cloud Run + serverless Postgres.
- **roxum** (kaza, Nov 7) — web platform bringing axum's style/safety to Roc
  ([github.com/KouYuume/roxum](https://github.com/KouYuume/roxum)).
- **roc-platform-patcher** (Scott Campbell, Oct 30) — "alter & tailor
  platforms": inject middleware into a platform to observe/alter effects,
  monitor allocations, stub or hot-swap effect implementations.
- **DST platform thesis** (Jan Rüggeberg, #platform development) — a
  deterministic-simulation-testing platform simulating a basic-cli subset
  for a Raft KV store; Luke Boswell: DST *"would be an excellent pitch for
  Roc"* and may validate compiler determinism.
- **trantor** (Karl) — "an extensible platform system", listed for the 0.1.0
  showcase.
- **Kite** (Karl, Nov 23) — a native GUI platform, just pushed to GitHub,
  feedback wanted.

Projects/apps:

- **Rocco game engine** (pixelsprout) — the month's most active project:
  v0.6.0 (Oct 28: positional audio, PBR/shadow maps, tone mapping, gamepad)
  → v0.10.0 (Nov 4: environments, fog, bloom) → **web builds via Roc→wasm +
  WebGPU** (Nov 16) with an interactive web tour; an "Agent town" experiment
  visualises Claude/Codex tool-call events in-game; the community spent
  Nov 23 on Rocco's branding/identity.
- **Roclet** (Aurélien Geron, Nov 2) — reactive notebooks in Roc running in
  the browser, even offline ([ageron.github.io/roclet](https://ageron.github.io/roclet/)).
- **roc-chat** (Steve Howell, Nov 19) — a chat server ported from his Zig
  server, deployed (rough edges acknowledged).
- **Roc on a unikernel** (Steve Howell) — a Roc app running on
  zig-metal-unikernel + zig-webbox.
- **roc-native-android 0.0.1** (Luke Boswell, Nov 22) — piloting the
  roc-signals reactive engine for native Android UI.

Packages (mostly Luke Boswell's prolific run): **roc-signals 0.3.0**
(reactive engine overhauled), **roc-fuzz 0.7.0** (many fuzz targets in one
binary), **roc-docx 0.1.0-rc1**, **roc-pdf 0.1.0** — and roc-pdf is **in
production**: Niclas Ahden generates real-estate record-sale flyers with it
— **plume 0.5.0/0.6.0** (Plotly), **roc-ray 0.12.0**, **roc-graph-layout
0.3.0-rc3** (renders Mermaid), **roc-gui 0.1.0-rc1**, **roc-playwright
0.13.0**, **roc-isodate 0.8.0**, **roc-regex** ("competitive with Rust's
regex crate" on wide-SIMD machines), **roc-syntax-color**. Note: two
independent packages both named roc-markdown exist (Karl's and Steve
Howell's) — flagged in-chat as a possible confusion.

Also: a **community showcase** for the 0.1.0 announcement blog post is being
assembled in #show and tell (40 msgs, 44 reactions) — Feldman explicitly
wants variety to show Roc's "long tail of domains". And the "How close can
pure FP get to C?" blog post did the rounds: #8 on HN, #4 on Lobsters, #2 on
r/programming at peak.

## 8. Community pulse

- **Most active voices:** notramo (523 msgs), Steve Howell (438), Richard
  Feldman (270), Luke Boswell (235), Krzysztof Skowronek (157), Anton (115).
- **Roc Online Meetup Nov 2026** was held Nov 22 (the Oct meetup was Oct 24);
  meetup discussion fed the `--opt=speed` priority decision.
- **Build-times survey** (#announcements, Oct 24): Niclas Ahden published
  per-project LOC/s numbers on the pre-RC compiler (e.g. a 76k-LOC server at
  ~5.9k LOC/s cold) — useful baseline for the RC1 improvements.
- **Bugs stream otherwise:** recursive-types inference issue (known root
  cause family), a crash on nested derived encoders (Nov 22), the Zig arena
  bug above. Nothing else looks release-blocking.
- New contributors keep arriving (e.g. Jack Scully — formally-verified
  kernels background — who within days was building the concave platform).

---

## 9. Could this be a recurring digest? — feasibility

**Yes, cheaply, with the access already in place.**

- **Access:** the laptop's `/home/user/zuliprc` user API key works for reads
  today. A digest job should run *from the laptop* (via laptop-agent, like
  this run) so the key never has to be copied to the sandbox. Note the key is
  a full account API key, not a read-only credential — a digest job only
  reads, but anything holding the key could post as Drew; keeping it on the
  laptop, in the existing mode-600 file, is the right posture.
- **Cost/limits:** this full 30-day pull was 19 stream queries + paging —
  under a minute, ~2,400 messages. A weekly pull is ~500–600 messages and a
  handful of requests, far inside Zulip's API rate limits (the REST API
  happily serves 1,000-message pages). Zulip also offers event queues
  (`POST /register`) if near-real-time were ever wanted; unnecessary for a
  digest.
- **Retention:** zulipchat.com hosted history is fully API-readable well
  beyond 30 days (the window here was a choice, not a limit), so a missed
  week is recoverable. Edited/deleted messages simply reflect current state.
- **Shape of the job:** a small deterministic fetch+aggregate script on the
  laptop (stream/topic/reaction tables, keyword flags) + an LLM summarisation
  pass — exactly this run's recipe. The only judgement calls to fix in
  advance: which streams to include (all public ones worked well), whether
  DMs stay excluded (recommended: yes), and the window (7 days, with the
  topic tables making slow-burn threads visible across weeks).
- **No cron was created** — this section is the feasibility note only, per
  instructions.

## Appendix — messages per stream (2026-10-24 → 2026-11-23)

| Stream | Msgs | Drew subscribed |
|---|---:|---|
| ideas | 682 | yes |
| show and tell | 458 | yes |
| beginners | 411 | yes |
| contributing | 293 | yes |
| off topic | 148 | yes |
| bugs | 100 | yes |
| announcements | 92 | yes |
| community | 31 | yes |
| gatherings | 27 | yes |
| performance | 27 | yes |
| compiler development | 26 | no (public, included) |
| introductions | 25 | yes |
| advent of code | 18 | no (public, included) |
| platform development | 17 | no (public, included) |
| API design / book / editor / roctoberfest / software unscripted podcast | 0 | no (public, quiet) |
| **Total** | **2,355** | |

*Raw fetched records and the analysis tables remain on the laptop at
`/tmp/roc-zulip/` (messages.jsonl, fetch-stats.json, analysis.txt) — note
/tmp is ephemeral; this digest is the durable copy.*
