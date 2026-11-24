# Artifact and binary repositories

The Artifactory question — where do built artefacts live, and
should the estate run a repository manager of its own — has a
crisp current answer and a deliberately open future one. Status:
**in use** (GHCR for containers; the Nix cache plane for Nix
artefacts); the self-host decision is **researched, with a named
trigger, left open where the record leaves it**. Read by the
platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **Containers live on GHCR.** The registry decision of record
  (`doc/fleet/generic-containers.md` §3) is GHCR — the registry
  this repository already uses. The published estate:
  `ghcr.io/developing-today/code/mcpx`, the generic runtime
  images, and `ghcr.io/developing-today/fleet-build:full` (see
  [Containers and Docker images](containers-and-images.md)).
  The CNCF registries census records the structural point: there
  is no cost pressure on this choice today. (Pricing detail
  belongs to the Project Management book.)
- **Nix artefacts live in the binary-cache plane** — the
  proven-but-unwired cache worker or a future attic-class
  self-host; there is no separate artifact repository for Nix
  outputs, by design of the ecosystem. See
  [The Nix cache worker and its alternatives](nix-caches.md).
- **Everything else** — npm, cargo, generic file artefacts —
  flows from upstream registries and the repository itself. **No
  universal artifact repository is in use or recorded**, and the
  record does not treat that as a problem.

## The researched verdict

From the CNCF registries census (landed in-repo,
`registries-and-builds.md`): **"GHCR is the right registry
today, and the trigger for self-hosting is friction, not
price."** The self-host triggers of record: air-gapped or
LAN-local clusters, egress at scale, sovereignty, and
pull-through caching.

- **Zot** (CNCF Sandbox; a single Go binary, OCI-native, no
  database — the storage directory *is* the registry) is the
  named **first self-host step**: the registry to stand up the
  day GHCR friction appears, including as a LAN pull-through
  mirror.
- **Harbor** (CNCF Graduated) is the governance-heavy answer the
  census explicitly sizes as too much platform for a
  one-operator estate today.
- **Dragonfly** (Graduated 2025-10-28) is not a registry at all —
  a peer-to-peer distribution layer for later, at much larger
  node counts.
- **JFrog Artifactory** itself: commercial, multi-format, and
  "not our shape" per the census.

## The decision, as the record leaves it

GHCR now for containers, plus a Nix binary cache for Nix
artefacts; Zot-on-LAN as the pre-decided first step when the
triggers fire; Harbor and Artifactory-class systems rejected for
now on fit. The Zot step and the cache-plane choice are
**decisions the record leaves open** — queued decisions with
triggers, not settled plans — and this book presents them
exactly that way.

## Sources

Platform census §17 and supplements S12, 2026-11-23, landed as
PR #13959.
