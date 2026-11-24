# Build-server alternatives

Asked "what is this estate's build server?", the honest answer
is that the estate does not run one — its **fleet placement
system over GitHub Actions and Blacksmith is the build-server
answer**. The named alternatives are mostly absences, each
searched for. Status: **in use** (placement); **researched**
(the rest). Read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **Fleet placement** dispatches builds with a runner choice
  (`--runner auto|github|blacksmith`, a `--push-nix-cache`
  option, and container/surface options — the fleet book and the
  design register carry the surface). Jobs spill across surfaces
  in the cascade of record: GitHub Actions, then Blacksmith,
  then Codespaces, then queued, never dropped (see
  [GitHub Actions](github-actions.md)).
- **Actions-exec sessions** (#13774) and the **compile
  coordinator design**
  ([#13765](https://github.com/developing-today/code/issues/13765))
  are the ephemeral-capacity leg.
- **The laptop itself is a build host**, with the
  `ghcr.io/developing-today/fleet-build:full` CI runner image and
  a cargo-target-broker for shared Rust build caches. The
  in-cluster build ambition is **planned**, waiting on the
  cluster (lab census, 2026-11-23).

## The alternatives, as the record leaves them

- **Garnix:** never used as a CI or build service. A repo-wide
  search finds only template-repository links in TODO and archive
  lists, and its name in the nix-cache benchmark comparison.
  **Absent as a system.**
- **Hercules CI:** `hercules-ci-effects` is a flake input (with
  flake-parts following it), but no Hercules CI service usage
  record was found. **In use as a library input only**, as far
  as the record shows.
- **Hydra:** a `lib/hydra.nix` file exists in the repository; no
  record of a running Hydra was found. **File present, service
  absent** per the record searched — the census flags this one
  as lightly searched.
- **Cachix** appears as the upstream project caches the flake
  consumes, and as a benchmark comparator for the cache worker —
  not as the estate's own CI cache. That role belongs to
  magic-nix-cache over the GitHub Actions cache today (see
  [The Nix cache worker and its alternatives](nix-caches.md)).

## Gaps

No build-server procurement or migration is recorded or planned;
the placement system is the settled answer, and its scaling
story is
[Automation and automated scaling](automation-and-scaling.md).

## Sources

Platform census §16 and supplements S13, 2026-11-23, landed as
PR #13959.
