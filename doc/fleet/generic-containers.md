# Generic containers for ID and fleet purposes (#6076)

Status: inventory + image definitions landed; proof runs recorded on
issue #6076. Parent: issue #6066. Audit context: issue #5771 (its own
items — the mcpx Containerfile, the duplicate lab-ops Dockerfile, the
amd-crd definition-or-waiver — stay on issue #5771; this doc only
records what they imply for what "generic" must not duplicate).

All images are Nix builds in this repo (`dockerTools.buildLayeredImage`,
the pattern the service images already use). No hand-maintained
Dockerfiles.

## 1. Inventory — what "generic" must cover

Derived from the three service-specific Nix images that exist today:

| Existing image                                                          | Runtime                                  | Userspace beyond the service      | CA / tz                                                    | Writable dirs          | Ports / notes                                                                                                       |
| ----------------------------------------------------------------------- | ---------------------------------------- | --------------------------------- | ---------------------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `mcpx` (`pkgs/mcpx/container-image.nix`)                                | bun (JS)                                 | none — deliberately no shell      | cacert via `SSL_CERT_FILE`                                 | `/work`, `/tmp` (1777) | runs as `1000:1000`, `HOME=/tmp`, state/cache under `/tmp`                                                          |
| `identity` (`pkgs/identity/container-image.nix`)                        | none (compiled Go binary, `charm serve`) | none                              | cacert via `SSL_CERT_FILE`                                 | `/data` (volume)       | ports 35353–35356                                                                                                   |
| `lab-ops-hub-crawlercontainer` (`pkgs/lab-ops-hub/container-image.nix`) | python3 + `idna`                         | bash, coreutils, `/bin/sh` linked | cacert via `SSL_CERT_FILE` (Nix python ships no CA config) | `/app`                 | port 8080; absolute interpreter path + explicit `PATH` (shape lesson from Cloudflare's 2026-10-20 platform changes) |

What the inventory says generic must cover:

- **Three runtime classes, no more.** (a) A compiled/static binary
  needs no runtime at all — just a userspace to run in. (b) JS
  services run on bun. (c) Python services run on python3 with a
  small, explicit library set. No service in the estate needs a
  general-purpose distro base, Node, or Deno inside the image.
- **The same base userspace everywhere:** bash + coreutils with
  `/bin/sh` linked (the lab-ops shape lesson: conventionally-shaped
  images keep starting), cacert wired through `SSL_CERT_FILE`,
  tzdata, writable `/tmp` and a working directory, non-root
  `1000:1000`, `HOME=/tmp`.
- **What generic must NOT duplicate (#5771):** the service images
  stay canonical for their services. Generic images are the runtime
  layer a service artifact is layered on or mounted into — they do
  not repackage mcpx, identity, or the crawler, and they are not a
  second build path for any of them. Third-party software stays
  digest-pinned per #5771 and is out of scope here.

## 2. The generic image set

Defined in `pkgs/generic-containers/`, wired as flake packages:

| Flake package              | Image                       | Contents                                                                                                                            |
| -------------------------- | --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `.#generic-runtime`        | `generic-runtime:v1`        | bash, coreutils, cacert, tzdata — the base userspace for compiled-binary (ID) services                                              |
| `.#generic-js-runtime`     | `generic-js-runtime:v1`     | generic base + bun — for bun-based ID/fleet services                                                                                |
| `.#generic-python-runtime` | `generic-python-runtime:v1` | generic base + python3 (default library set: the crawler's `idna`; override with `extraPythonPackages`) — for Python fleet services |

The `container-images` workflow builds all three alongside the
existing service images, whose build set is unchanged.

## 3. Registry / publishing — decision

- **Registry: GHCR** (`ghcr.io`), the registry this repo already
  publishes to and consumes from: the mcpx release workflow pushes
  `ghcr.io/developing-today/code/mcpx`, and the org CI runner image
  is `ghcr.io/developing-today/fleet-build:full`. The lab-ops
  crawler's Cloudflare-registry push stays as-is — it is a
  Cloudflare Containers deploy target, not a generic image.
- **Naming:** `ghcr.io/developing-today/generic-runtime`,
  `ghcr.io/developing-today/generic-js-runtime`,
  `ghcr.io/developing-today/generic-python-runtime` — one repository
  per image, named after the image, not after any service.
- **Tags:** the image's major tag (`v1`, bumped in the `.nix`
  definition when the contents contract changes) plus the commit
  SHA of the build for traceability. No `latest` for deploy use.
- **Publishing trigger:** push to `main` touching
  `pkgs/generic-containers/**` (the same paths the build workflow
  watches). PR builds verify only; they never publish.
- **Retention:** keep the current major tag and the last 10 SHA
  tags per image; older SHA tags are pruned. Generic images are
  reproducible from the flake at any retained SHA, so pruning loses
  nothing.

## 4. Proof runs

One ID service and one fleet service run from the generic images,
with the service artifact supplied at run time (the generic image
provides the entire runtime/userspace):

- **ID (fleet identity):** the `identity` package's `charm` binary
  run inside `generic-runtime`.
- **Fleet:** the Lab Ops Hub crawler (`pkgs/lab-ops-hub/container/app.py`)
  run inside `generic-python-runtime`.

Results (2026-11-11, laptop, docker 29.9.2; full record on
issue #6076):

- **Fleet — full live run.** `generic-python-runtime:v1` running the
  crawler's `app.py` answered `POST /crawl {"url":"https://example.com"}`
  with HTTP 200 and the extracted page (`"title": "Example Domain"`,
  `"status_code": 200`) — the service, its `idna` dependency, and
  outbound HTTPS through the image's CA bundle all work.
- **ID — runtime-equivalence run.** `generic-runtime:v1` running the
  identity package's `charm serve` booted the full service (DI
  injector, all services, "Starting charm server", DB open at
  `/tmp/data/db/charm_sqlite.db`) and then stopped at the same point
  the service's own canonical image stops at when run bare: a panic
  on the empty DB URL scheme, reproduced identically from
  `identity:0.1.0` built from `pkgs/identity/container-image.nix`
  (path `/data/db/charm_sqlite.db`). The DB URL is deployment-supplied
  configuration (the deployed identity pins it via its env file), not
  part of either image — so the generic base runs the ID service
  exactly as far as its own image does, and a deployment that works
  on the canonical image works on the generic one by supplying the
  same env.
- **JS runtime sanity:** `generic-js-runtime:v1` runs `bun --version`
  (1.4.4).
- **No regression:** the existing image set still builds from this
  flake — `mcpx-container` and `identity-image` built alongside the
  generic packages, and the lab-ops image is untouched.
