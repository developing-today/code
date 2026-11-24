# The Nix cache worker and its alternatives

This chapter carries the platform's central tension, stated
plainly: **the estate has built and proven its own Nix binary
cache — and does not use it by default.** The worker is
**proven** in its own repository; it is **not** among this
flake's substituters. Both halves are load-bearing. Read by the
platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## The worker — what exists

The cache lives in its own repository,
**developing-today/nix-cache** (two workflows of its own).
Its architecture, from the memory topic of 2026-10-05 and the
design-register entry C1 (`pm-stack/design-register-2026-11-11.md`):

- A Cloudflare **Worker** as the front door.
- An **R2** bucket, `nix-cache-nars`, holding the NAR bytes.
- A **KV** namespace, INDEX, as the hot index.
- A **D1** database, `nix-cache-db`.
- A **Queue**, `nix-cache-indexing`, feeding indexing.
- A daily cron orphan sweep.
- Deployed on workers.dev. Uploads go through a secret path
  prefix, because `nix copy --to` cannot send custom headers.
  The signing keypair is named `nix-cache-1`.
- **Deliberately no Durable Objects** — the rationale in its
  README: cache PUTs are atomic and idempotent, and the queue
  gives verified, retried indexing.

## The proof of record

A content-addressed demo derivation was built, signed, pushed,
deleted locally, and then **substituted back from the cache
alone in 1.8 seconds**, against a 93-second build, with only
this substituter and its key trusted. Follow-on work shipped and
was live-verified on 2026-10-07: a zstd push path (a custom
script — `nix copy` hardcodes xz) and presigned R2 upload URLs
for all objects, bypassing the 100 MB Worker body limit. A
benchmark programme against Cachix, Attic and Garnix exists in
that repository (BENCHMARK.md); its figures belong to that repo
and the Project Management book, and are not reproduced here.

## What the estate actually uses today

- This flake's substituters are `cache.nixos.org` plus the yazi
  and hyprland cachix caches — see
  [Nix and the NixOS flake](nix-and-the-flake.md). **The
  nix-cache worker is not among them.**
- CI uses magic-nix-cache backed by the GitHub Actions cache —
  see [CI/CD: Nix and GitHub Actions](ci-cd.md).

## Alternatives and successors — researched, undecided

`doc/nix-ecosystem-scan-2026-08.md` §5 is the standing survey:
Lix's documentation steers away from the perl nix-serve toward
**attic**, **harmonia**, and S3 plus garage; a cache shootout
(April 2026) benchmarked harmonia, nix-serve and nix-serve-ng,
ncps (a proxy cache), attic (sqlite/S3, zstd chunks), the snix
nar-bridge, and plain nginx/S3. Determinate Systems maintains an
attic fork; the original attic project moves slowly. The scan's
action item: a self-hosted cache of attic (or its Determinate
fork) plus S3 is the current best-practice stack, and `n2p` over
iroh is a zero-config LAN/WAN store-sync option worth attention.
A dezren39/nix-serve-ng fork exists in the repo universe.

One recorded dead end: the Attic client comparison died on a
build problem — the atticd server built and ran, but the Attic
client links Nix C++ libraries unavailable for the estate's
Nix 2.35.2 (`doc/platform/toolchains/rust.md`, 2026-10-05).

## Gaps and the open decision

- **No retention or LRU garbage collection exists** — the cron
  sweeps orphans only (filed as developing-today/nix-cache#7).
- What Drew wants to cache for real was never decided, per the
  design register.
- **No successor or migration decision is recorded.** Attic-class
  self-hosting versus the Cloudflare worker versus the status
  quo is an open choice, and this book presents it as open.

## Sources

Platform census §14 and supplements S8, 2026-11-23, landed as
PR #13959.
