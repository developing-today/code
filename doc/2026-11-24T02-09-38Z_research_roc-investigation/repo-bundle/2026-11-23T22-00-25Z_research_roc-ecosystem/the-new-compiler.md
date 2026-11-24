# The new compiler (Zig rewrite) and the road to 0.1.0

## What happened

- The rewrite of Roc's compiler from Rust to Zig was announced in February
  2025 (Feldman's decision write-up:
  [gist](https://gist.github.com/rtfeldman/77fb430ee57b42f5f2ca973a3992532f.pibb))
  and took **487 days**. The codebase went from ~300K lines of Rust to **464K
  lines of Zig**.
- **Feature parity was reached in July 2026.** The primary source is Feldman's
  retrospective, ["How Our Rust-to-Zig Rewrite is Going"](https://rtfeldman.com/rust-to-zig)
  (2026-07-15) — also the best single document on the rewrite's rationale and
  results, and the best watch/read-first item alongside the GOTO talk
  ["Roc & Zig: A Compiler Rewrite Story"](https://www.youtube.com/watch?v=XSXGf3oN2yU&list=PLEx5khR4g7PIyc7AhQEUm06yslH_hWM2i).
- The legacy Rust compiler line ended at alpha1 (2025-01-29) through
  alpha4-rolling (2025-08-26) and is superseded. Distribution of the new
  compiler is via **nightlies** (`nightly-YYYY-MM-DD-<hash>`,
  [roc-lang/nightlies](https://github.com/roc-lang/nightlies)); the
  [setup-roc](https://github.com/roc-lang/setup-roc) GitHub Action supports
  `nightly-new-compiler`, and Nix users have the
  [roc-lang/roc-overlay](https://github.com/roc-lang/roc-overlay) flake.
  nixpkgs itself moved its roc package from alpha4 to
  `nightly-2026-11-09-9e85de7` in November 2026
  ([maintainermode.org PR #564994](https://maintainermode.org/pr/564994)).

## RC1 shipped — reconciling two snapshots

Two of this record's sources were written days apart and read differently;
the fresher one wins, and the difference is worth stating explicitly:

- The ecosystem survey (web sources, read 2026-11-23) found **0.1.0 not yet
  shipped**: Feldman (2026-10-22) had said it was "still weeks away," the roc
  repo README still said Roc is "not ready for a 0.1 release yet," and
  downstream projects were already talking about a "0.1.0 release candidate."
- The Zulip record — the community's own channel — shows that state was
  already superseded: **Roc 0.1.0-RC1 shipped on 2026-11-21**
  ([announcement topic](https://roc.zulipchat.com/#narrow/channel/397893-announcements/topic/Release%20candidate%3A%20Roc%200.1.0-RC1/near/642546747)).
  RC1 is commit `15177e91f6`, binaries on GitHub. This is Roc's **first-ever
  RC process**, started partly to establish the process itself.

**So: parity landed in July 2026, and RC1 has shipped. What remains open is
the promotion question** — whether RC1 becomes 0.1.0 directly or an RC2
comes first.

### The RC1 → 0.1.0 rules and status

From Feldman's announcement and the following days' discussion (summarised
in [community and Zulip digest](community-and-zulip-digest.md)):

- The existing feature freeze stays. No breaking API changes to stdlib or the
  host boundary unless unavoidable for an important bug.
- **No performance improvements at all** (compiler or runtime) until 0.1.1,
  which is planned "shortly after" 0.1.0.
- **Bugfix bar for 0.1.0:** only bugs that (a) block real-world Roc programs
  (hobby projects and learning materials count) *and* (b) can't be worked
  around. Everything else — even compiler crashes — defaults to 0.1.1, with a
  deliberately high bar for exceptions.
- **Status as of 2026-11-23:** no blocker bugs reported. Feldman noted,
  surprised, that RC1 appears eligible to be promoted directly to 0.1.0;
  an RC2 is still his base expectation. Community reports agree: packages and
  platforms updated to RC1 without issue.
- **RC1 build-time benchmarks** (Feldman, on a dozen showcase projects):
  `roc check` and `roc build --opt=dev` are sub-second even cold; warm
  rebuilds under 100 ms. `--opt=dev` was the 0.1.0 priority; speeding up
  `--opt=speed`/`--opt=size` gets a priority bump after 0.1.0.

## What the new compiler does that the old one didn't

From [Feldman's rewrite retrospective](https://rtfeldman.com/rust-to-zig) and
[roc-lang.org](https://www.roc-lang.org):

- **Build speed:** incremental builds went from 10.0s (Rust 1.85) / 3.4s
  (Rust 1.97) to **0.035s** on Zig 0.17 with `-fincremental`. Caveat: stable
  Zig 0.16.0 has a bug that breaks `-fincremental` on this codebase (fix
  landed upstream; the team is waiting on the next stable), and
  `-fincremental` is currently x86-64 only.
- **Hot code loading in dev:** `roc server.roc` picks up source edits
  per-request without a restart.
- **Reproducible cross-compilation:** `roc build --target=x64musl` produces
  byte-identical output across hosts.
- **Zero-parse caching:** `roc check` / `roc test` write an on-disk cache
  (structure-of-arrays layout, 32-bit indices) read back without parsing.
- **New internals:** a new parser (Anthony Bullard, Sam Mohr), a new type
  checker (Jared Ramirez), and new lambda-set resolution (Ayaz Hafiz,
  prototyped in OCaml — polymorphic defunctionalization; the academic lineage
  is Hafiz's WITS 2024 paper on type-directed defunctionalization).
- **Memory management:** the new compiler does **not** use Morphic — it
  implements the successor algorithm from the Morphic team's own paper
  ([ACM](https://dl.acm.org/doi/pdf/10.1145/3798221)), described by Feldman as
  a more powerful alternative to both Morphic and Perceus-style refcounting.
  A November performance thread ended with PR
  [roc#13090](https://github.com/roc-lang/roc/pull/13090) bringing unmodified
  functional-style code to within ~3% more instructions than a hand-written C
  port, and confirmation that functional style no longer carries an
  execution-time penalty.
- **Language-level additions:** string interpolation inside patterns;
  compile-time execution of pure functions (demonstrated as a zero-allocation
  HTTP router); an in-browser build of the compiler shipped as the "echo"
  platform (2.5 MB wasm, embedded on the roc-lang.org homepage, by Stephan).
- **Memory-safety retrospective:** the post counts 2 Zig-compiler
  use-after-free bugs found in the new compiler's error-message paths, versus
  21 Rust-era memory-corruption reports — all of which were miscompilations,
  none in compiler logic. Feldman's conclusion is that the rewrite did not
  trade away safety in practice.
- **Toolchain notes:** a Zig 0.17 arena double-allocation bug "can plausibly
  bloat Roc programs" (reported in #bugs, Oct 26); the Zig 0.17.1 upgrade was
  started Nov 5. Roc was also submitted to Compiler Explorer (godbolt) via a
  community PR (Nov 16).

## Language changes in the new compiler

Per the current [roc-lang.org examples](https://www.roc-lang.org/examples) and
release notes:

- Effectful functions are suffixed with `!` (`main!`, `echo!`); pure vs
  effectful is visible in every signature.
- `Result` is now **`Try`**.
- **Methods** exist: `list.map(...)`, `n.to_str()`.
- New builtins **`Iter`** and **`Stream`**; `for!` loops; local mutable
  variables via `var $x`.
- A breaking change announced 2026-11-12 (operator desugaring and shadowing):
  a default-number-resolution bug forced changes to operator desugaring —
  operators no longer require both operands and the result to share one type,
  enabling e.g. `my_duration * 12` (Duration × U64) instead of forcing the
  literal to be a Duration.

## Who has moved to the new compiler

- **Both official platforms** (basic-cli, basic-webserver) — new-compiler
  only, tracking nightlies within days (see [platforms](platforms.md)).
- **The Exercism Roc track** — fully migrated August 2026 (110 exercises;
  maintainer Aurélien Geron hand-updated 108 beginner exercises;
  [Exercism forum thread 68377](https://forum.exercism.org)), and running on
  0.1.0-RC1 as of Nov 22.
- **Essentially the entire current awesome list** — every platform and package
  labelled Community–active in these pages pins a November 2026 nightly.
- **Rocci Bird** (the WASM-4 game, see
  [talks, releases and showcases](talks-releases-showcases.md)) was ported as
  the feature-parity proof.
- Community language has flipped: packages are "ported over to the modern
  compiler" (plume 0.5.0), rocup was fixed "for the new zig compiler."

## What's still missing / not done

- **0.1.0 itself** — the promotion decision above. Feldman's talk
  ["A Preview of Roc 0.1.0"](https://hallway.video) (2026-08-13) previews it.
- Documentation — the acknowledged gap (see the overview page); the
  mini-tutorial will be replaced before 0.1 and langref expectations are set
  explicitly to "after 0.1.0."
- Performance work on `--opt=speed`/`--opt=size` is deliberately deferred
  until after 0.1.0 by the RC rules.
