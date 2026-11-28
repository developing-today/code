# Community and Zulip digest — 2026-10-24 → 2026-11-23

The Roc Zulip ([roc.zulipchat.com](https://roc.zulipchat.com)) is the
project's centre of gravity. This page summarises 30 days of it — 2,355
messages across all 19 public streams — in our own words: developments,
decisions, and the conversations that matter, with links into the topics.
Routine Q&A (most of #beginners) is not covered. The public export is also
browsable at [roc-lang.github.io/zulip-export](https://roc-lang.github.io/zulip-export).

Most active voices in the window: notramo (523 messages), Steve Howell
(438), Richard Feldman (270), Luke Boswell (235), Krzysztof Skowronek (157),
Anton (115). The Roc Online Meetup for November was held Nov 22 (the October
meetup was Oct 24); meetup discussion fed the `--opt=speed` priority decision
recorded on [the new compiler](the-new-compiler.md) page.

## Roc 0.1.0-RC1 — reaction and status

Feldman's announcement (47 messages, 71 reactions — the most-reacted topic of
the month):
[Release candidate: Roc 0.1.0-RC1](https://roc.zulipchat.com/#narrow/channel/397893-announcements/topic/Release%20candidate%3A%20Roc%200.1.0-RC1/near/642546747).

- RC1 shipped 2026-11-21; the rules, bugfix bar, benchmarks, and the open
  promotion question are laid out on [the new compiler](the-new-compiler.md)
  page.
- Community reports were uniformly smooth: packages and platforms updated
  without issue (Niclas Åhdén; kaza's roxum; pixelsprout's Rocco engine;
  Bryce Miller's Tigerbeetle client and roc-pg). Roux builds except
  [roc#13382](https://github.com/roc-lang/roc/issues/13382), which Zak Kohler
  worked around.
- Two non-blockers noted: a basic-cli `Cmd.exec!` process-replacement quirk
  when running via `roc <file>` (issue to be filed), and an investigated
  cold-`roc check` slowdown on one small example (0.05 s CPU on the
  2026-11-03 nightly vs 1.00 s on RC1 on a noisy machine; the write-up hedges
  heavily on measurement noise).
- The "How close can pure FP get to C?" blog post did the rounds the same
  month: #8 on Hacker News, #4 on Lobsters, #2 on r/programming at peak.

## Language-design decisions

### Linear types — decided against (Nov 22–23)

Steve Howell asked for linear types to enforce resource cleanup (auto-close)
in his roc-chat server (#ideas, 48 messages). Feldman's answer, in detail:
verifying compile-time uniqueness requires either a very restrictive memory
model (e.g. Goose) or roughly the same type-system complexity and annotation
verbosity as Rust — in his phrase, the cure is worse than the disease. Roc's
answer remains **FBIP** (Functional But In-Place), which Feldman notes came
from Koka via Folkert's thesis adviser. Expect this request to keep
recurring; the reasoning is now on record.

### Reflection / reification — the biggest design conversation of the month

Two large #ideas threads — "SurrealDB query builder based on reifying" (270
messages) and "Reflection — typed function bodies as Roc values (reifying)"
(194 messages). Karakatiza maintains a compiler fork with working
reification —
[github.com/Karakatiza666/roc/tree/reify](https://github.com/Karakatiza666/roc/tree/reify)
— with design docs and examples, used to build type-safe query builders.
This is experimental-fork work, not a mainline commitment, but it's where the
typed-reflection design is being hashed out. Related threads: "Type
reflection — Zig `@typeInfo()` equivalent" and a CBOR-serialization design
thread (39 messages).

### Compiler API — wanted, not planned

Jack Scully's Convex-style platform needs an app's exported endpoints with
resolved types; today that means hand-rolled parsers or `--introspect` hacks
(#platform development, "Compiler API?", Nov 22). Luke Boswell pointed to an
old tooling gist. Feldman's caution, generalised from the linear-types
thread: exposing IR/APIs makes them unchangeable (his example: a main reason
Atom got surpassed by VS Code), so the project intends to be very careful
about what APIs to expose.

### Smaller design threads

- "Reworking Rust Glue" (19 messages — Karl's RustGlue.roc may replace the
  repo's glue scripts as the recommended starting point).
- "Requiring more type annotations?" (18 messages).
- "In file evaluation/repl" (17 messages — Kilian Vounckx, returning after a
  long absence, proposed REPL-style in-file evaluation; nothing decided).
- A regression was found and filed: unwrapping an `Ok` in `main!` stopped
  compiling (works at top level) — Niclas Åhdén filed
  [roc#13281](https://github.com/roc-lang/roc/issues/13281) at Feldman's
  request (Nov 20).

## The LLM-generated code conversation — stated neutrally

The month's most emotionally charged conversation, recorded here because it
may produce policy:

- **"Goodbye"** (#off topic, 52 messages, 65 reactions, Nov 19–21): Hannes —
  an early, multi-year contributor who built the roc-packages index
  ([hasnep.github.io/roc-packages](https://hasnep.github.io/roc-packages)) —
  announced he is leaving. In his framing, show-and-tell is now dominated by
  LLM-generated code he learns nothing from, chat contains many
  LLM-isms, and he prefers the Gleam community, which in his view values
  effort over output. He explicitly does **not** think Roc should be anti-LLM
  like Zig.
- A sibling thread — "roc on a unikernel (venting about LLM policies)" (#show
  and tell, 125 messages) — started from the Zig project auto-closing an
  LLM-disclosed issue with a clear reproducer, and ranged widely over
  upstream-project LLM policies.
- **Feldman's response** (Nov 20): a warm goodbye to Hannes; on policy, in
  short, he is not sure and needs to think more, based on what other
  communities have done; every community is going through some version of
  this and compositions are changing either way; his stated personal view is
  to advocate common ground and kindness. **No policy has been adopted.**
- Constructive proposals floated: self-reported "human-code scores" /
  AI-declaration standards for shared projects (removewingman linked
  ai-declaration.md). Anton gently pushed back that the team's actual focus
  has been solving bugs, improving performance, and updating packages and
  platforms.
- For calibration, the same window's biggest body of new application code
  (Steve Howell's portfolio, see the projects page) is openly described by
  its author as built through agentic development with Claude, and it was
  received as a showcase contribution. Both facts are part of the record.

## The documentation push (#contributing)

The "Docathon?" thread (144 messages, **73 reactions — the most-reacted
topic overall**) became a month-long docs effort ahead of 0.1.0:

- **15 new code examples on the website** (Aurélien Geron, Oct 27):
  AdvancedParsing, Loops, NestedTypes, NominalTypes, OpenRecords, Paths,
  PatternAliases, Polymorphism, RecursiveTypes, RocRay, Sets, Streams,
  StringPatterns, Unicode, WrappingArithmetic — reviewed by Anton and Luke
  Boswell.
- The mini-tutorial **will be replaced before 0.1** (Anton).
- Langref expectations set explicitly to "after 0.1.0" (PR #12758) — the
  project's line is not to promise things unnecessarily.
- **New website search is live** (Nov 1).
- Steve Howell is writing "a little Roc book" — early draft at
  [roc.lynrummy.com/tutorial](https://roc.lynrummy.com/tutorial/introduction.html).
- A community **Roc style guide** (Steve Howell, LLM-drafted, human review
  requested) drew concrete state-machine idiom rules from notramo in a
  96-message #beginners thread.

## Tooling and distribution progress

- **Official VS Code extension** (announced Nov 5, Anton): syntax
  highlighting (including Roc in doc comments/Markdown), diagnostics as you
  type, formatting (format-on-save default), hover with inferred types.
  Publisher verification is pending on Microsoft's side. Luke Boswell's fork
  inspired vital components of it.
- **Signed macOS nightly binaries** (Nov 20, 18 reactions): Intel and Apple
  Silicon nightlies are now officially signed — no more Gatekeeper warnings.
- **Homebrew install** announced Nov 21; Anton will also make a PR to add Roc
  to **mise**.
- **Exercism:** the Roc track is now running on 0.1.0-RC1 (Nov 22).
- **ReleaseSafe nightlies** are now published alongside regular nightlies
  (Anton, in the LSP-bug thread) for people who want that extra security.
- **LSP outage fixed:** the month's biggest bug thread — "LSP does not work
  after latest nightly" (#bugs, 79 messages, Oct 25 → Nov 14) — was confirmed
  fixed on Nov 13–14. The thread also ranged into LSP/compiler-memory
  security trade-offs.
- **Build-times survey** (#announcements, Oct 24): Niclas Åhdén published
  per-project LOC/s numbers on the pre-RC compiler (e.g. a 76k-LOC server at
  ~5.9k LOC/s cold) — a useful baseline for the RC1 improvements.

## What's shipping — platforms, projects, packages

Covered in full on the [platforms](platforms.md),
[libraries](libraries-and-packages.md), and
[projects](real-projects-and-production-use.md) pages. The November-window
headlines: the Rocco game engine's run from v0.6.0 to v0.10.0 plus web
builds; the concave, roxum, Kite, and trantor platform announcements;
Roclet reactive notebooks; roc-chat deployed; Roc on a unikernel;
roc-native-android 0.0.1 piloting the roc-signals engine; and a heavy release
cadence from Luke Boswell's package suite (roc-signals 0.3.0, roc-fuzz 0.7.0,
roc-docx 0.1.0-rc1, roc-pdf 0.1.0, plume 0.5.0/0.6.0, roc-ray 0.12.0,
roc-graph-layout 0.3.0-rc3, roc-gui 0.1.0-rc1) plus roc-playwright 0.13.0 and
roc-isodate 0.8.0. New contributors keep arriving — e.g. Jack Scully
(formally-verified kernels background), who within days was building the
concave platform.

## Bugs stream, otherwise

A recursive-types inference issue (known root-cause family), a crash on
nested derived encoders (Nov 22), and the Zig 0.17 arena bug noted on the
compiler page. Nothing else in the window looked release-blocking — which is
consistent with the RC1 status above.
