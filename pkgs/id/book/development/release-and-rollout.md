# Release and rollout

Source: `pkgs/id/flake.nix` (the `packages` set), `.github/workflows/id-check.yml`, `pkgs/id/hosted-worker/wrangler.toml` and `pkgs/id/hosted-worker/README.md` in https://github.com/developing-today/code/blob/main/pkgs/id/flake.nix — provenance only; this chapter is complete without following it.

ID has no versioned release process of its own: no release workflow, no tag-and-publish step and no changelog gate exists for the `id` binary in the repository's workflow set. What ships is the Nix package set, built from `main`, and verified by the checks that run on `main` after each merge. This chapter states what is packaged, how a build reaches a node, and what the hosted edition's rollout status is. The book's own deployment is a separate subject, covered in [Deploying this book](../operations/deploying-this-book.md); this chapter does not repeat it.

## How the `id` binary is packaged

The packages live in the `packages` set of `pkgs/id/flake.nix`. Both are `pkgs.rustPlatform.buildRustPackage` derivations over the whole `pkgs/id` source tree, both are version `0.1.0`, and both install the same single binary as `bin/id`.

| Package | Flake name | Feature set | Notes |
|---|---|---|---|
| `id-web` | `pname = "id"` | `buildFeatures = ["web", "sandbox", "ssh", "world"]` | The primary product and the flake's `default` package. Its build installs the web dependencies offline through the bun2nix hook (`bunRoot = "web"`), builds the web assets in its `preBuild` step, and builds the Roc platform's Wasm host. |
| `id-lib` | `pname = "id-lib"` | `buildNoDefaultFeatures = true`, `buildFeatures = ["sandbox", "world"]` | The library variant: no web UI, no Bun in its build inputs, no `ssh`. Its feature set is the Cargo default set; the split between the two builds is explained in [Building and testing](building.md). |

Build either package from `pkgs/id/`:

```sh
nix build .#id-web
nix build .#id-lib
```

From the repository root, the same packages are reachable through the root flake's sub-flake wiring in `lib/default.nix`, which inherits the sub-flake's packages unchanged. The flake also generates one app per public justfile recipe from `just-recipes.json`; its `default` app runs the default package's `bin/id`.

Both packages run their test phase under the `ci-check` profile, with the `serve_tests` tests skipped — those tests bind and listen on sockets, which the Nix sandbox forbids — and with a CA bundle file supplied for the check phase through `SSL_CERT_FILE`.

## Verification on `main`

Merges land on `main` first and are verified there. The workflow `.github/workflows/id-check.yml` (workflow name `id`) is the gate:

- On a push to `main`, it runs the fast checks as a parallel matrix, one job per check listed in `.github/scripts/id-pr-checks.txt`. A guard job checks that list against the flake first: a new flake check that is neither listed nor explicitly excluded fails the guard by name. The matrix is aggregated by a job that keeps the historical required status name `nix flake check`.
- On its schedule (nightly) and on manual dispatch, it runs the full `nix flake check` for `pkgs/id` instead: the complete check set in [Building and testing](building.md), in the Nix sandbox.
- The Playwright browser tests and the NixOS VM tests are not in the push matrix. They run in `.github/workflows/id-e2e-nightly.yml`, nightly and on manual dispatch.

A build that is red on `main` is not a release candidate. There is no second channel — no beta or stable branch — between `main` and a deployed node.

## Rolling a build out to a node

A node is one `id serve` process running one of the two packages against one data directory. Rolling out a new build is: build or fetch the new package, stop the old process, start the new package's `bin/id serve` with the same flags and the same data directory, and confirm the node answers. The operator's view of that work — flags, data directories and health checks — is [Running a node: serve, worlds and the hosted edition](../operations/services.md).

Three format facts bound what a rollout may assume:

- The event store's on-disk format marker is `id.events.store.v9`, and a `v8` store is accepted. A rollout across that boundary does not need a data migration step.
- The tag registry file carries `REGISTRY_VERSION = 1`; the `id-config` document carries `CONFIG_SCHEMA_VERSION = 1`, and a document with any other schema version is rejected, not migrated.
- Guest modules are content-verified, not version-negotiated: a gallery host verifies every bundle file at startup, and a world host loads the module it is given. Rolling out a new guest is installing the new module, per [Add a Roc guest app](add-a-guest.md).

No automatic update mechanism exists in the binary: `id` does not check for, download or apply a newer build of itself. Every rollout is an operator action.

## The hosted edition: scaffold status, not a live rollout

The hosted edition has no rollout to describe, because it is not deployed. Stating its status plainly, from its own files:

- `pkgs/id/hosted-worker/` is a local-only scaffold. The header of its `wrangler.toml` states: this config is for `wrangler dev` and tests; no account-level deployment, production binding or billable resource is authorised by the package; deployment and the real-account jurisdiction canary are gated on issues #2285 and #3403.
- Run locally, the hosted Worker's application fails closed by design: local workerd does not implement jurisdiction restrictions, so every fully-valid request receives `503` with the error `jurisdiction_unavailable`. That behaviour is the scaffold working as intended, not an outage.
- `pkgs/id/worker-poc/` is a proof of concept for running one example world in a Worker isolate with journal equality against the native host. Its acceptance boxes are checked in its readme, and nothing in it is a production path. Its `d1-journal-poc/` subfolder is a second local-only probe with a placeholder database ID, run only with the local flag.
- The hosted Worker's test suite does run in automation: `.github/workflows/id-hosted-worker-tests.yml` exists in the workflow set for the scaffold's tests. Test coverage of a scaffold is not a deployment.

When the gating issues close and a deployment is authorised, the rollout procedure belongs in this chapter, written in the same change that performs the first deployment, per [Keeping this book current](keeping-this-book-current.md). Until then, any text — in this book or elsewhere — that presents the hosted edition as a running service is wrong, and the concepts chapter keeps the same discipline: [The hosted edition](../concepts/hosted.md).

Two neighbouring Worker workflows are not the hosted edition and do not change this status: `.github/workflows/id-mail-worker-publish.yml` and `.github/workflows/id-mail-worker-promote.yml` concern the separate mail worker. Their existence is not evidence that `id-hosted-worker` is published or promoted.

## This book's own release

This book ships as the `.#id-docs` Nix build. Its build gate runs on `main` in `.github/workflows/id-docs.yml`, which builds only and never deploys; the actual GitHub Pages deployment is performed by a combined deploy that stages every book in the repository together, because one Pages deployment replaces the whole site. Both halves are specified in [Deploying this book](../operations/deploying-this-book.md), and the maintenance contract that keeps the book true between releases is [Keeping this book current](keeping-this-book-current.md).
