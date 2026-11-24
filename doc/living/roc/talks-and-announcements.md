---
created: 2026-11-23
last-modified: 2026-11-24
revision: 1
---

# Roc — Talks, Videos, Releases & Announcements

*Compiled 2026-11-23. The public-showcase side of the Roc ecosystem: people
showing off the language, the new (Zig-based) compiler, and what has actually
shipped. Recent and new-compiler material first; landmark older talks a
newcomer should still watch are included and marked.*

## (a) Talks & videos

### New compiler / recent

- **Roc & Zig: A Compiler Rewrite Story** — Anjana Vakil & Richard Feldman,
  GOTO Unscripted (recorded at GOTO Copenhagen; published to YouTube June 5,
  2026). Feldman traces the ground-up rewrite of Roc's ~300K-line compiler from
  Rust to Zig, and how it coincided with the year AI coding assistants went
  from test-porting grunt work to genuine architectural collaborators. Also
  states on the record that "Roc v1" (0.1) is due before the end of 2026.
  **This is the single best watch-first item for the new compiler.**
  https://www.youtube.com/watch?v=XSXGf3oN2yU&list=PLEx5khR4g7PIyc7AhQEUm06yslH_hWM2i

- **Functional Mechanical Sympathy** (keynote) — Richard Feldman, FUNARCH 2026
  (ACM SIGPLAN workshop on Functional Software Architecture, co-located with
  ICFP 2026, Indianapolis, August 2026). Feldman, now listed with Zed
  Industries, argues functional programs can be made fast by architecting with
  the machine in mind — the performance philosophy behind Roc, presented as a
  research-community keynote rather than a product pitch.
  https://www.conference-publishing.com/toc/ICFPWS26FUNARCH/abs

### Richard Feldman conference talks (landmarks and Roc-adjacent)

- **Outperforming Imperative with Pure Functional Languages** — Richard
  Feldman, Strange Loop 2021. *The* landmark Roc talk (~98K views): how a pure
  functional language can match or beat imperative performance via
  opportunistic in-place mutation, static reference counting, and unboxed
  values. Still the clearest introduction to why Roc exists.
  https://hacker-tube.org/watch/vzfy4EKwG_Y

- **Simple Functional Effects with Tag Unions** — Richard Feldman, YOW! 2022.
  A design talk on Roc's approach to effects: managed effects expressed with
  tag unions instead of monads or effect systems, so effectful code stays
  readable and fully inferred. Listed in GOTO's Functional Programming
  playlist (item 73).
  https://www.youtube.com/playlist?list=PLEx5khR4g7PIyc7AhQEUm06yslH_hWM2i

- **Why Static Typing Came Back** — Richard Feldman, GOTO Copenhagen 2022.
  Not Roc-specific, but the talk that frames Roc's design bet: why dynamic
  languages dominated the 1990s and why modern inference-based static typing
  (Roc's included) reclaimed their advantages.
  https://www.hallway.video/watch/Tml94je2edk

- **Calling Functions Across Languages** — Richard Feldman, GOTO Chicago 2023.
  On FFI and calling across languages in-process; directly relevant to how
  Roc platforms (written in Rust/Zig/etc.) host Roc applications, and to
  Feldman's platform-design thinking.
  https://www.hallway.video/watch/ZOvxa9aKCCg

- **Rust & Zig Combined** — Richard Feldman, GOTO 2023. How the *original* Roc
  compiler combined a Rust codebase with Zig (for its surgical linker and
  LLVM bitcode work) — the prequel that makes the later full Zig rewrite much
  less surprising. Listed in GOTO's Functional Programming playlist
  (item 30).
  https://www.youtube.com/playlist?list=PLEx5khR4g7PIyc7AhQEUm06yslH_hWM2i

- **Intro to Roc & Innovation in Functional Programming** — Richard Feldman &
  James Lewis, GOTO Unscripted interview (recorded GOTO Copenhagen, released
  April 2024). A relaxed long-form intro: Roc's origins in Elm, platform
  architecture, parsing strategy, and where Feldman thinks functional
  programming is heading.
  https://play.fountain.fm/episode/zdpbCUHdYtpjynPqmIdg

### Other contributors

- **Type-directed Defunctionalization in the Presence of Type Classes and
  Monomorphization** — Ayaz Hafiz, paper presented at WITS 2024 (Workshop on
  the Implementation of Type Systems, London, January 2024). The academic
  write-up of Roc's lambda-set specialization — the compiler technique that
  lets higher-order Roc code compile to optimal closure-free machine code.
  Hafiz also built the new compiler's lambda-set resolution system, so this is
  the design lineage of a core piece of the rewrite.
  https://roc.zulipchat.com/user_uploads/22008/HudpAdrSn4Rys-i9pv8ZyGBu/ACM-SIGPLAN-Proceedings-Template.pdf

### Podcast appearances

- **Roc with Richard Feldman** — Rust in Production, Season 5 Episode 4
  (corrode), November 13, 2025. The most production-focused recent interview:
  why the team moved the compiler from Rust to Zig, what Rust can learn from
  that decision, and Roc's existing production use (e.g. at Vendr).
  https://corrode.dev/podcast/s05e04-roc/

- **The Roc Programming Language with Richard Feldman** — The Changelog
  Interviews #645 (2025). A wide-ranging 90-minute conversation: static
  dispatch, opportunistic mutation, Roc vs Go speed, why Roc will never
  self-host, and Roc in the LLM era. Good single-episode overview of the
  whole project.
  https://changelog.com/podcast/645

- **Roc — A Functional Language Looking for Those Software Sweetspots** —
  Richard Feldman on Developer Voices (Kris Jenkins), June 2025. Focuses on
  what a language makes *harder* as a design tool, and where Roc is hunting
  for its niche. (Episode listing page; scroll to the Roc entry.)
  https://www.developervoices.com/episodes/

- **The Roc Programming Language with Richard Feldman, Parts 1–3** — ADSP:
  Algorithms + Data Structures = Programs, Episodes 157–159 (Nov–Dec 2023).
  The deepest technical interview series: what "functional" means, type
  inference, continuation-passing style, and compiler internals, with Conor
  Hoekstra and Bryce Adelstein Lelbach.
  https://adspthepodcast.com/2023/11/24/Episode-157.html

- **Creating a Language: Elixir vs. Roc** — José Valim & Richard Feldman,
  Software Unscripted, cross-published by Elixir Wizards (January 11, 2024).
  Two language creators comparing notes: how goals shape design tradeoffs,
  and Roc's focus on single-threaded performance.
  http://www.smartlogic.io/podcast/elixir-wizards/s11-e13-software-unscripted-jose-valim-richard-feldman-elixir-roc/

- **Lambda Set Defunctionalization with Ayaz Hafiz** — Software Unscripted
  Ep. 85 (January 25, 2024). Feldman interviews core contributor Ayaz Hafiz
  on the Roc compiler's most distinctive technique — how lambda sets make
  defunctionalization a typed, whole-program optimization.
  https://shows.acast.com/software-unscripted/episodes/664fde448c77cc0013b3338a

## (b) Written announcements & releases

- **How Our Rust-to-Zig Rewrite is Going** — Richard Feldman, rtfeldman.com,
  July 15, 2026. The milestone post: after 487 days, the Zig rewrite of the
  300K-line compiler **passed feature parity** with the original. Includes
  build-time comparisons (incremental rebuilds down to 0.035s on Zig 0.17),
  hot code loading demos, and the roadmap: version 0.1.0 — Roc's first-ever
  numbered release — targeted for later in 2026.
  https://rtfeldman.com/rust-to-zig

- **How Close Can Pure FP Get to C?** — Richard Feldman, rtfeldman.com,
  October 22, 2026. The roc-deflate showcase write-up (see demos below):
  a pure-Roc DEFLATE implementation benchmarked a median 2.6% (Apple M1) /
  8.9% (Ryzen 3900X) behind libdeflate, the hand-tuned SIMD C library —
  with memory safety, purity, and no build scripts. Feldman notes 0.1.0 is
  "weeks away" as of this post.
  https://rtfeldman.com/fp-vs-c

- **Rewriting Roc's Compiler in Zig** (decision write-up) — Richard Feldman,
  February 2025. The original announcement of the rewrite: why almost every
  part of the Rust compiler was due for a rewrite anyway, why Zig over Rust
  (compile times, allocator style, direct LLVM bitcode generation), and why
  Roc will still never self-host.
  https://gist.github.com/rtfeldman/77fb430ee57b42f5f2ca973a3992532f.pibb

- **0.1 + 0.2** — Richard Feldman, rtfeldman.com, May 28, 2024. The roadmap
  post that first laid out what Roc's first two numbered releases would
  contain and the project's versioning philosophy.
  (Listed at https://rtfeldman.com)

- **Roc 0.0.0-alpha1 released** — roc-lang/roc, January 29, 2025. The start
  of Roc's permanent alpha release series (alpha1–alpha4 "rolling" releases
  followed, latest alpha4 in August 2025), replacing the use-a-nightly
  free-for-all with pinned, permanently available builds.
  https://communick.news/post/2449578/4355026

- **Roc's New Compiler for Advent of Code 2025** — Richard Feldman, December
  2025. The hastily-assembled guide that first put the new compiler in the
  public's hands: nightly builds, the revised language design, and what
  hadn't been ported yet (e.g. exhaustiveness checking). A primary-source
  snapshot of the new compiler's state at the end of 2025.
  https://gist.github.com/rtfeldman/f46bcbfe5132d62c4095dfa687bb9aa4

- **basic-cli releases (0.20.0 → 0.23.0-rc1)** — roc-lang/basic-cli, August
  2025 onward. The flagship CLI platform kept shipping through the compiler
  transition: 0.20.0 (Aug 2025) overhauled the Cmd API and massively expanded
  test coverage, and the 0.21–0.23 release candidates retarget the platform
  line at the new compiler.
  https://github.com/roc-lang/basic-cli/releases

- **basic-webserver releases (0.13.x → 0.14.0-rc1)** — roc-lang/basic-webserver,
  2025–2026. The HTTP server platform (Rust host on Hyper/Tokio) added
  multipart/form-data parsing, typed server-sent events, and SQLite support;
  its README now states plainly that `main` and the 0.14 release candidates
  target Roc's new Zig-based compiler and the old compiler is not supported.
  A 0.22.0 release was recorded on November 16, 2026.
  https://github.com/roc-lang/basic-webserver/releases

## (c) Demos & showcases

- **Rocci Bird, rebuilt on the new compiler** — Brendan Hansknecht's WASM-4
  Flappy Bird clone (art by Luke DeVault), ported by Luke Boswell, playable
  in the browser via WebAssembly and embedded in the July 2026 rewrite post.
  Under 1,000 lines of Roc; `roc build --opt=size` now produces a 31KB wasm
  binary — less than half the old compiler's output. The proof-of-life demo
  for feature parity.
  https://rtfeldman.com/rust-to-zig

- **The roc-lang.org homepage REPL** — the new compiler itself, compiled to a
  2.5MB WebAssembly binary, running the "echo" platform in the browser so
  anyone can write and run basic Roc programs on the homepage (built by
  Stephan). The most accessible hands-on demo of the new compiler — no
  install required.
  https://rtfeldman.com/rust-to-zig

- **Hot code loading demo** — video embedded in the rewrite post showing a
  running Roc web server and a 2D game picking up code changes on the next
  request/frame, with no restart. A headline developer-experience feature of
  the new compiler that the old one never had.
  https://rtfeldman.com/rust-to-zig

- **roc-deflate vs libdeflate** — Niclas Åhdén's DEFLATE package (which he
  runs in paid production work), optimization-tuned by Feldman until it
  trailed the best C implementation by only single-digit percentages across
  144 Silesia-corpus benchmarks. The strongest "pure functional can be fast"
  evidence the project has produced.
  https://rtfeldman.com/fp-vs-c

- **Advent of Code 2025 on the new compiler** — the community spent December
  2025 solving Advent of Code with the bleeding-edge compiler, with the core
  team actively supporting them on Roc Zulip. Not one demo but dozens of
  small real programs — the new compiler's first public stress test.
  https://gist.github.com/rtfeldman/f46bcbfe5132d62c4095dfa687bb9aa4

---

*Further pointer, unverified recording link: Feldman also gave a keynote,
"The Essence of Functional Programming," at Functional Conf (India,
presented remotely), discussed in ADSP Episode 157.*


## Change log

- r1 (2026-11-24): Migrated from hatch-personal workspace/roc-investigation/ under #13877; content unchanged.
