# Real projects and production use

Honest scale labels: **Production** = someone depends on it / it earns money ·
**Substantial hobby** = large, finished, used by its author/community, not
commercial · **Demo/toy** = exists to demonstrate the language or a platform.

## Production

- **Niclas Åhdén's production Roc (Joy framework)** — _Production,
  small-team/consulting scale._ Åhdén (instateam.se) has used Roc in
  production for ~2 years; Feldman calls him "Roc's most prolific production
  user." His Func Prog Sweden talk (2026-11-10), "Roc and Joy: Productivity in
  Production," states Joy "already powers real web apps in the wild." His
  package suite (see the libraries page) is the by-product of that work.
  Sources: [rtfeldman.com/fp-vs-c](https://rtfeldman.com/fp-vs-c),
  [github.com/niclas-ahden](https://github.com/niclas-ahden).
- **roc-deflate** — _Production, single-package._ Åhdén's pure-Roc DEFLATE
  implementation runs in **paid production**. Feldman spent weeks optimising
  it and benchmarked it against libdeflate over 144 Silesia-corpus runs
  (levels L1–L12): median **8.9% slower than libdeflate on a Ryzen 9 3900X,
  2.6% slower on M1**, worst case 18% slower, best case ~18% faster,
  byte-identical output. Honest caveats from the same post: roc-deflate's APIs
  are non-streaming only, and one heavily optimised package is not the general
  case. Source: [rtfeldman.com/fp-vs-c](https://rtfeldman.com/fp-vs-c)
  (2026-10-22).
- **roc-pdf in production** — _Production, single-package._ Niclas Åhdén
  generates real-estate record-sale flyers with Luke Boswell's roc-pdf 0.1.0
  (reported in the November Zulip window).

## Substantial hobby / community infrastructure

- **Steve Howell's [roc.lynrummy.com](https://roc.lynrummy.com) portfolio** —
  _Substantial hobby, production-adjacent._ The largest public body of
  new-compiler Roc application code found in this investigation, all UPL-1.0
  and, by the author's own account, built "through agentic development, with
  Claude"; the apps also run in-browser as wasm.
  - **roc-chat** — a live production group chat at chat.lynrummy.com that
    replaced the group's earlier Zig chat: **~33,000 lines of Roc in 171
    modules** (+ ~10,000 lines in 45 spec modules, ~2,300 `expect`s) on a
    custom Zig-host platform (29 hosted functions, the filesystem as the
    database, SSE live updates, password/GitHub/device-code sign-in; AI
    agents are chat members with their own keys).
  - **roc-markdown** — 5,780 lines; "finished… in production"; renders every
    roc-chat message; held to 314 frozen CommonMark/GFM spec examples.
  - **roc-syntax-color** — 5,242 lines; syntax highlighting for 7 languages.
  - **Safari Ride** — 14,268 lines, 98 modules, 816 expects; plus a 39-chapter
    online book, _Safari Ride: a tour of a real Roc program_ (early draft).
  - **A BASIC interpreter** — 6,733 lines; ECMA-55 Minimal BASIC plus the 1978
    Creative Computing dialect; graded against 208 NBS test programs and 99
    games from _BASIC Computer Games_.
  - **Roc on Shale** — 25 Roc programs, ~25,700 lines: code-check/report
    scripts for his Shale agent scripting language.
  - **Seattle Delivery** (4,704 lines; vehicle-routing solver, Clarke–Wright
    - local search, bit-for-bit reproducible native vs wasm), **Fast Track**
      (4,166 lines; board game with search AI), **Dog Park** (3,688 lines;
      herding game), **Safari Walk** (1,666 lines; built on the Rocco 3D
      engine), a shared 1,439-line chess kit behind Knight's Tour / Eight
      Queens, the roc-screensaver platform, and animated shorts (_Trick or
      Treat_, _Welcome to Roc_, _Robot Thanksgiving_).
  - Also from the November window: **Roc on a unikernel** — a Roc app running
    on zig-metal-unikernel + zig-webbox.
  - Howell is also writing "a little Roc book" — early draft at
    [roc.lynrummy.com/tutorial](https://roc.lynrummy.com/tutorial/introduction.html).
- **Roclet** (Aurélien Geron, Nov 2) — reactive notebooks in Roc running in
  the browser, even offline ([ageron.github.io/roclet](https://ageron.github.io/roclet/)).

## Demos and smaller projects

- **Rocci Bird** (bhansconnect, 2024; art by Luke DeVault) — _Demo, but a
  load-bearing one._ A WASM-4 Flappy-Bird-like, ported to the new compiler as
  the feature-parity proof: `roc build --opt=size` produces a **31 KB wasm**,
  less than half the old compiler's output. Playable on itch.io and embedded
  in Feldman's rewrite post. Source:
  [rtfeldman.com/rust-to-zig](https://rtfeldman.com/rust-to-zig).
- **roc-ray example games/tools** (with the platform) — Breakout, Spark Run,
  Cave Climb, Pixel Workshop, Particles, Live Plot, Postcard Studio.
  _Small-but-complete._
- **kai** ([thebrandonlucas/kai](https://github.com/thebrandonlucas/kai)) — a
  CLI frontend for Nix, listed as a showcase app on the awesome list.
- **roc-gui demo apps** — a dozen GPUI demo applications (HTTP Workbench,
  Database Browser, Redis Explorer, …) shipped as the platform's gallery.
- **Rocco "Agent town"** — an experiment visualising Claude/Codex tool-call
  events inside the Rocco 3D engine (November 2026).

## Learning / puzzle scenes

- **Exercism Roc track** — 110 exercises, fully migrated to the new compiler
  in August 2026 and on 0.1.0-RC1 as of Nov 22. This is the main on-ramp for
  new users and a real signal of new-compiler adoption.
  [exercism.org/tracks/roc](https://exercism.org/tracks/roc)
- **Advent of Code** — [lukewilliamboswell/aoc](https://github.com/lukewilliamboswell/aoc)
  (puzzle solutions) is on the current awesome list's showcases. Roc has an
  AoC presence through individual solvers rather than an organised scene. The
  community's December 2025 run on the then-brand-new compiler — dozens of
  small real programs, with the core team actively supporting on Zulip — was
  the new compiler's first public stress test.
- **Game jams** — no distinct Roc game-jam institution was found in this
  pass; the games energy is concentrated in roc-ray / roc-wasm4 / Rocco
  examples instead. (Stated plainly rather than padded.)

## The 0.1.0 showcase

A community showcase for the 0.1.0 announcement blog post is being assembled
in #show and tell (40 messages, 44 reactions in the November window) —
Feldman explicitly wants variety to show Roc's "long tail of domains."
Projects named for it include Karl's trantor platform and the Rocco engine.
