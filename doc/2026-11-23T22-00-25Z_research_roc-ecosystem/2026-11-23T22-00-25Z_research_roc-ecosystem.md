# Roc ecosystem — research record (2026-11-23)

A snapshot of the Roc programming language ecosystem as of 2026-11-23: the new
(Zig) compiler and its first release candidate, the platform layer, the package
ecosystem, real projects and production use, what the community has been
deciding and shipping, and the talks/releases/showcases worth watching.

Compiled from primary sources read 2026-11-23: [roc-lang.org](https://www.roc-lang.org),
Richard Feldman's posts on [rtfeldman.com](https://rtfeldman.com), GitHub repos
and release pages, the community awesome list
([lukewilliamboswell/roc-awesome](https://github.com/lukewilliamboswell/roc-awesome)),
and a 30-day read of the Roc Zulip (2026-10-24 → 2026-11-23, 2,355 messages
across all 19 public streams) summarised in our own words.

## The headline facts

- **Roc 0.1.0-RC1 shipped on 2026-11-21** — the first release candidate in
  Roc's history, built from the new Zig compiler. As of 2026-11-23 no blocker
  bugs had been reported, and Feldman has said RC1 may be promoted directly to
  0.1.0 (an RC2 remains his base expectation). See
  [the new compiler](the-new-compiler.md).
- The compiler rewrite from Rust to Zig took 487 days, reached **feature
  parity in July 2026**, and the new compiler is now simply "the compiler":
  both official platforms, the Exercism track, and essentially every active
  community project target it, pinned to dated nightlies.
- The platform layer is small but genuinely active: two official platforms
  ([basic-cli](https://github.com/roc-lang/basic-cli),
  [basic-webserver](https://github.com/roc-lang/basic-webserver)) shipping
  frequent releases, plus an autumn-2026 burst of community platforms — a
  Cloudflare Workers platform, a native Android platform created 2026-11-22, a
  GPUI desktop GUI, a raylib games platform, and a 3D engine. See
  [platforms](platforms.md).
- There is real production use, but it is concentrated: principally one
  consultant (Niclas Åhdén) running Roc + his Joy framework in production for
  ~2 years, a pure-Roc deflate implementation in paid production within ~9% of
  libdeflate's speed, and one very large body of new-compiler application code
  by Steve Howell. See [real projects](real-projects-and-production-use.md).
- The language changed in the rewrite: effectful functions are suffixed `!`,
  `Result` became `Try`, methods landed, and new builtins (`Iter`/`Stream`)
  exist. Old-compiler code does not carry over unchanged.
- Two community decisions worth knowing: **linear types were decided against**
  (FBIP is Roc's answer), and a live, unresolved conversation about
  LLM-generated code and community culture is under way with **no policy
  adopted yet**. See [community and Zulip digest](community-and-zulip-digest.md).

## Pages

- [Overview and state of Roc](overview-and-state-of-roc.md) — what Roc is,
  where the project stands, who carries it, and the honest caveats.
- [The new compiler](the-new-compiler.md) — the Rust-to-Zig rewrite, feature
  parity, language changes, what's still missing, and the RC1 → 0.1.0
  promotion question.
- [Platforms](platforms.md) — official and community platforms, with maturity
  labels, plus the legacy-compiler platforms to treat as superseded.
- [Libraries and packages](libraries-and-packages.md) — the new-compiler
  package canon: parsing, data formats, databases, the two big author suites,
  tooling, and the superseded list.
- [Real projects and production use](real-projects-and-production-use.md) —
  production, substantial hobby, and demo projects with honest scale labels.
- [Community and Zulip digest](community-and-zulip-digest.md) — the last 30
  days of developments and decisions: RC1 reaction, the linear-types
  decision, the LLM-code conversation stated neutrally, the documentation
  push, and tooling/distribution progress.
- [Talks, releases and showcases](talks-releases-showcases.md) — videos and
  talks to watch first, written announcements, and the demos people are
  showing off.

## Maturity labels used in these pages

- **Official–active** — maintained by the roc-lang org, tracking current nightlies.
- **Community–active** — third-party, releasing against the new compiler in recent weeks.
- **Experimental / PoC** — real and runnable, but self-described as early or proof-of-concept.
- **Legacy / superseded** — built for the old Rust compiler; treat as abandoned unless re-verified.

## Source quality note

Aggregator/overview articles about Roc are unreliable (stale version numbers
were found and discarded during this research). Every version number in these
pages comes from the project's own release page or README as read on
2026-11-23, and Zulip material is summarised in our own words with links to
the topics rather than quoted at length.
