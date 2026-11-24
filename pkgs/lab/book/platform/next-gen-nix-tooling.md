# Nix apps V2 and next-gen tooling

An honest opening: **"Nix apps V2" as a named programme is
absent.** No document or issue the census found uses that name;
the flake-apps mechanism of record is the justfile generation in
[NixOS flake apps](flake-apps.md). What does exist is the
tooling-generation question — which Nix, and which surrounding
tools, come next — and its record is one standing scan. Status:
**researched-only; no adoption decision**. Read by the platform
census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

`doc/nix-ecosystem-scan-2026-08.md` §§4–5 is the record:

- **Upstream Nix 2.30–2.34 highlights:** an evaluation profiler,
  WAL SQLite caches, `nix flake prefetch-inputs`, HTTP
  binary-cache compression, `external-builders`,
  `nix store roots-daemon`, and an unprivileged daemon via
  `nix-nswrapper`. The estate's own system runs a 2.35
  pre-release line (the scan, and `doc/platform/toolchains/rust.md`).
- **Lix** 2.95 — a fork of CppNix 2.18: no lazy trees, a
  CA-derivations rewrite planned, and 8–20% faster than 2.18 per
  its own materials, as cited by the scan.
- **Determinate Nix** — the validated downstream. The estate
  already depends on Determinate's _installer and cache action_
  in CI (see [CI/CD: Nix and GitHub Actions](ci-cd.md)), while
  the `determinate` flake input in `flake.nix` sits **commented
  out**.
- **snix/Tvix** — the Rust reimplementation lineage, present in
  the cache benchmarks through its nar-bridge (see
  [The Nix cache worker and its alternatives](nix-caches.md)).
- **Also in the input graph**, each marked "TODO: use this?":
  `omnix`, `nix-output-monitor` (in use), `nix-topology`, and
  `nix-search`.

## What the record plans

Nothing adopted. The scan is a watch-brief: it tracks the field
so a decision can be made from current facts when one is needed.

## Gaps

- No evaluation or adoption record exists for Lix or Determinate
  Nix _as the system Nix_. The book presents this section as a
  research summary with an open decision, because that is what
  the record supports.

## Sources

Platform census §15 and supplements S2, 2026-11-23, landed as
PR #13959.
