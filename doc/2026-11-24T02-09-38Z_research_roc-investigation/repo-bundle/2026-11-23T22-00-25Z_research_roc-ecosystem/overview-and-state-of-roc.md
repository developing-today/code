# Overview and state of Roc

Roc is a functional programming language built around a distinctive bargain:
pure functional code with full type inference, compiled to fast native
binaries via opportunistic in-place mutation and static reference counting,
with all side effects supplied by a _platform_ — a host program (written in
Rust, Zig, Go, …) that provides files, network, UI, and so on. The language
grew out of Elm's lineage (Richard Feldman created Roc after years as a core
Elm figure), and its design bet is that a language can be both friendly and
very fast. The landmark talk
["Outperforming Imperative with Pure Functional Languages"](https://hacker-tube.org/watch/vzfy4EKwG_Y)
(Strange Loop 2021) remains the clearest statement of why Roc exists.

## Where the project stands (2026-11-23)

- The compiler was rewritten from Rust to Zig over 487 days; the
  [roc repo](https://github.com/roc-lang/roc) is now ~92.9% Zig (6,207 stars,
  ~56K commits, read 2026-11-23). Feature parity with the old compiler was
  reached in July 2026, and the new compiler is the only one that matters —
  see [the new compiler](the-new-compiler.md).
- **Roc 0.1.0-RC1 shipped 2026-11-21**, the language's first release candidate
  and the run-up to its first numbered release in eight years. Nightlies
  ([roc-lang/nightlies](https://github.com/roc-lang/nightlies)) remain the
  real distribution channel.
- The ecosystem's centre of gravity is the **Roc Zulip**, linked from every
  official page; a public export is browsable at
  [roc-lang.github.io/zulip-export](https://roc-lang.github.io/zulip-export).
  The last 30 days there are summarised in
  [community and Zulip digest](community-and-zulip-digest.md).
- The **Exercism Roc track** — 110 exercises, fully migrated to the new
  compiler in August 2026 and running on 0.1.0-RC1 as of 2026-11-22 — is the
  main on-ramp for new users ([exercism.org/tracks/roc](https://exercism.org/tracks/roc)).

## Who carries it

The centre of mass is a handful of people, and these pages name them because
the ecosystem cannot be understood without them:

- **Richard Feldman** — language creator; compiler, [roc-signals](https://github.com/rtfeldman/roc-signals),
  and the project's public voice ([rtfeldman.com](https://rtfeldman.com)).
- **Luke Boswell** — basic-cli, [roc-ray](https://github.com/lukewilliamboswell/roc-ray),
  [roc-gui](https://github.com/lukewilliamboswell/roc-gui), the native
  Android/macOS platforms, roc-parser, and the awesome list itself.
- **Niclas Åhdén** — the [Joy](https://github.com/niclas-ahden/joy) framework
  and the production-driven package suite; described by Feldman as Roc's most
  prolific production user.
- **Karakatiza666** — data-format packages and the Cloudflare Workers platform.
- **Steve Howell** — the largest public body of new-compiler application code
  ([roc.lynrummy.com](https://roc.lynrummy.com)).

## Sustainability and risk, stated plainly

- The roc-lang.org homepage (read 2026-11-23) seeks **$4,000 USD/month** to
  fund one longtime contributor full-time. The Roc Programming Language
  Foundation is a 501(c)(3) nonprofit. Current corporate sponsors per the repo
  README: Lambda Class, ohne-makler, Decem (earlier sponsors named by Feldman:
  rwx, Vendr, NoRedInk, Tweede Golf, Martian).
- **Bus factor is the ecosystem's biggest risk.** A small number of authors
  account for the compiler, both official platforms, most serious packages,
  and most production use.
- **Everything is pre-1.0 and mostly pre-0.1.** Packages pin specific dated
  nightlies in their headers and pins move weekly; expect breakage until
  0.1.0 lands and some churn after it.
- Documentation is the acknowledged gap — Feldman, closing his rewrite
  retrospective: in its current state you can still expect "a variety of bugs,
  incomplete features, and unfinished docs." A community documentation push
  ahead of 0.1.0 is under way (see the digest page).

## Exists / new / actually used

- **Exists (and is current):** the new compiler (nightlies + RC1), basic-cli,
  basic-webserver, roc-ray, Joy, roc-parser, roc-pg, roc-signals, the Åhdén
  and Boswell package suites, the Exercism track.
- **New (last ~6 months):** feature parity itself; RC1; roc-gui (Sep 2026);
  roc-cloudflare-workers (Oct 2026); roc-native-android (Nov 2026); the
  basic-webserver 0.17 → 0.22 release velocity; agent-facing tooling
  (roc-syntax-mcp); the language changes (`Try`, methods, `Iter`/`Stream`)
  becoming the documented norm.
- **Actually used (evidence of real use, not just existence):** roc-deflate in
  paid production; Joy powering production web apps for ~2 years; roc-chat as
  a live group chat; Exercism learners on 110 exercises; roc-ray's shipped
  example tools; roc-pdf generating real-estate flyers in production.
- **Exists but legacy:** old-compiler platforms and packages listed as
  legacy/superseded in [platforms](platforms.md) and
  [libraries and packages](libraries-and-packages.md) — code that has not been
  ported and should not be cited as current ecosystem.
