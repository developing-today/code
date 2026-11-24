# Containers and Docker images

This estate builds its containers the Nix way: images assembled
purely from Nix store paths, with no Dockerfile and no upstream
base image. Status: **proven and in use**. This chapter joins the
two halves of the record — the from-scratch Nix build pattern,
and the OCI/Docker image estate it publishes into — as read by
the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).
Where the published images live is
[Artifact and binary repositories](artifact-repositories.md).

## The from-scratch pattern

- **Mechanism:** `pkgs.dockerTools.buildLayeredImage`
  throughout. An image is its store-path contents and its config
  — that is the from-scratch pattern here, by construction.
- **Generic runtimes**
  ([#6076](https://github.com/developing-today/code/issues/6076),
  closed): `pkgs/generic-containers/` holds `runtime-base.nix`
  (bash, coreutils, cacert and tzdata only; user `1000:1000`;
  writable `/work` and `/tmp`; the CA bundle wired through
  `SSL_CERT_FILE`), `js-runtime.nix` and `python-runtime.nix`.
  The header states the design: the common denominator of the
  service images, deliberately carrying no service code.
- **Service images:** `pkgs/mcpx/container-image.nix`,
  `pkgs/identity/container-image.nix` plus its
  `provider-image.nix`, `pkgs/lab-ops-hub/container-image.nix`,
  and the mcpx end-to-end test image built inline in
  `lib/default.nix`.
- **The programme rule behind it** (Drew, 2026-11-04): Nix-built
  generic containers for ID and fleet, no hand-maintained
  Dockerfiles. Audit trail: #5765 (deployment epic) and #5771
  (containers Nix-build audit).
- **Cloudflare-side containers:** `pkgs/cf-container-hello` (a
  Cloudflare Containers hello project with its own workflow) and
  the lab-ops-hub container, deployed with its worker, are the
  edge-container leg.

## The pipeline and the image estate

`.github/workflows/container-images.yml` builds all of the above
on the Determinate installer plus magic-nix-cache pattern (see
[CI/CD: Nix and GitHub Actions](ci-cd.md)) and, **on push to main
only**, publishes the generic images to GHCR with v1 and
commit-SHA tags. The ordering and concurrency rationale is
recorded in the workflow itself, from PR #8369 review. Policy
home: `doc/fleet/generic-containers.md` §3.

The published estate of record: `ghcr.io/developing-today/code/mcpx`
(0.1.0 and latest), the generic runtime images
(`generic-runtime`, `generic-js-runtime`, `generic-python-runtime`),
and `ghcr.io/developing-today/fleet-build:full`, the CI runner
image.

## What the record plans

No change to the build pattern is recorded. The open registry
question — when, if ever, to self-host — is a decision with
named triggers, kept open in
[Artifact and binary repositories](artifact-repositories.md).

## Gaps

None for the build story. Runtime and rollout proof for the
generic images rests on #6076's closed record; the census noted
it did not re-verify that record's proof artefacts.

## Sources

Platform census §§9 and 17 and supplements S6, 2026-11-23,
landed as PR #13959.
