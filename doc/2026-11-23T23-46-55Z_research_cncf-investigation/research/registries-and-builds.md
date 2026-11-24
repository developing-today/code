# Container registries, image distribution, and build/app-definition tooling

Researched 2026-11-23. Scope: whether Drew's estate needs its own registry at all,
which one if so, and how much of the surrounding "application definition and image
build" landscape survives contact with a Nix + Dockerfiles + GitHub workflow.
Every project carries its CNCF status and a maturity verdict; "what it is" and
"should we use it" are separated throughout.

## Drew's questions, answered up front

- **Do we need a container registry? Do we just use GitHub until it costs money?**
  Essentially yes — GHCR is the right registry today, and the trigger for
  self-hosting is friction, not price. GitHub's own billing documentation states
  that container image storage and bandwidth for the Container Registry are
  _currently free_, with advance notice promised before that changes; public
  packages are free outright, and pulls from inside GitHub Actions don't count
  against transfer allowances
  ([pricing analysis with doc citations](https://www.cloudrepo.us/articles/private-docker-registry-hosting-comparison)).
  Self-hosting earns its keep on four other grounds: air-gapped or LAN-local
  clusters (pull speed and survival when the WAN is down), egress costs at real
  scale, sovereignty over the artifact store, and pull-through caching of
  third-party images. None of those bind yet.
- **Harbor or Dragonfly?** They're not alternatives. **Harbor** is a registry —
  the place images live, with RBAC, scanning, and replication. **Dragonfly** is a
  P2P distribution layer that sits _in front of_ a registry (Harbor explicitly
  included) so a thousand nodes don't each pull the same layer from origin.
  Dragonfly graduated from the CNCF on 2025-10-28 (announced 2026-01-14) and is
  not a registry itself
  ([CNCF graduation announcement](https://www.cncf.io/announcements/2026/01/14/cloud-native-computing-foundation-announces-dragonflys-graduation/)).
  For our scale: neither, yet. GHCR now; Zot when a local registry is needed;
  Dragonfly only when pull storms measure out.
- **What is Zot, and why might it beat a big registry for us?** Zot is a
  single-binary, OCI-native registry (CNCF Sandbox) with no database and no
  moving parts beyond a storage directory. For a small estate, Harbor's
  Postgres + Redis + five services buy governance features we don't need; Zot
  buys the 90% case — push, pull, mirror, scan — in one process. Detail in §2.
- **What is Artifact Hub?** A catalogue website, not a registry. It indexes Helm
  charts, operators, OPA policies, and other cloud-native artifacts so they can
  be _discovered_; the artifacts themselves live in their own registries/repos.
  It's where you find the Cozystack-ecosystem charts, not where you push
  anything. CNCF Sandbox. Detail in §3.
- **If we use Nix, Dockerfiles, and GitHub, do we need the app-definition/build
  landscape at all?** Mostly no. Nix already does the thing Helm/Kustomize/Dagger
  are compensating for (reproducible, declarative assembly), and Nix builds OCI
  images directly. The exception is consumption: the Kubernetes ecosystem —
  Cozystack included — ships as Helm charts, so Helm stays as a _client_ for
  other people's software even if we never author a chart. Detail in §4.

---

## 1. The incumbent: GHCR

|                      |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **What it is**       | GitHub's OCI registry (`ghcr.io`), identity and permissions inherited from GitHub repos, `GITHUB_TOKEN` authentication in Actions with no extra credential plumbing.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| **CNCF status**      | Not CNCF — a GitHub product.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| **Pricing reality**  | Public packages: free. Container storage and bandwidth: documented as _currently free_ (a policy, not a contractual tier — GitHub commits to advance notice, reported as at least 30 days for egress charges). Other GitHub Packages formats (npm, Maven…) are metered against small plan quotas — 500 MB/1 GB on Free — but those quotas do not currently apply to container images ([CloudRepo comparison](https://www.cloudrepo.us/articles/private-docker-registry-hosting-comparison), [RunX analysis](https://www.runxbuild.com/blog/github-container-registry-pricing/)). Actions-internal pulls are exempt from transfer accounting, including on self-hosted runners. |
| **Maturity verdict** | Production-grade, and the integration with our existing CI is the feature. The risk to register is policy change, not reliability — and the mitigation is that OCI images are portable: a registry is a hostname change, not a migration project, _if_ image references are kept templated rather than hard-coded.                                                                                                                                                                                                                                                                                                                                                             |

**Should we use it:** yes, as the only registry, until one of the four triggers
fires (air-gap/LAN locality, measured egress pain, sovereignty requirement,
pull-through caching of upstream images). Keep `ghcr.io` references behind a
variable in deployment tooling so the exit stays cheap.

## 2. Self-hosted registries and distribution

| Project                                      | CNCF status                                               | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Maturity verdict                                                                                                                                                                                                                                                               |
| -------------------------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [Harbor](https://goharbor.io/)               | **Graduated** (2020)                                      | The full-featured enterprise registry: projects/RBAC, vulnerability scanning (Trivy), image replication between registries, retention/GC policies, audit logs, OIDC. A deployment is several services plus Postgres and Redis.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | The default answer for organisations with compliance and multi-team governance needs. For a one-operator estate it's a second platform to run so the first platform can pull images.                                                                                           |
| [Zot](https://zotregistry.dev/)              | **Sandbox** (accepted 2022-12-13)                         | An OCI-native registry as a single Go binary: implements the OCI Distribution Spec from scratch (not a Docker-registry fork), stores artifacts in plain OCI image layout on disk or S3 (the storage directory _is_ the registry — back it up and you've backed up the registry), no database. Extensions compiled into the same binary add mirroring/sync, Trivy-backed search, scrub, metrics, UI, and cosign/notation verification ([project overview](https://www.mrvitej.in/devops/cncf-zot/)). Started at Cisco; in production use as edge/CI registries.                                                                                                                                                                  | The right-sized self-host option for us. Sandbox label, but the scope is deliberately small and the failure modes are boring. This is the registry to stand up the day GHCR friction appears — including as a LAN pull-through mirror in front of GHCR.                        |
| [Dragonfly](https://d7y.io/)                 | **Graduated** (2025-10-28)                                | A P2P distribution system for images, OCI artifacts, files, and AI model weights: a scheduler coordinates peers; each node's dfdaemon downloads pieces from other nodes and seeds them onward, so origin bandwidth grows ~logarithmically instead of linearly with fleet size. Integrates as a containerd registry mirror and with Harbor preheating. Its subproject Nydus provides the lazy-loading image format covered in the runtimes file. CNCF reports production use at tens of millions of container launches/day and up to ~90% origin bandwidth savings at Alibaba/Ant/DiDi/Kuaishou scale ([CNCF](https://www.cncf.io/announcements/2026/01/14/cloud-native-computing-foundation-announces-dragonflys-graduation/)). | Excellent at a scale we do not have. P2P distribution pays when many nodes pull the same large images simultaneously; a handful of nodes pulling from a nearby registry never notice the problem it solves. Revisit if the fleet or image sizes grow by an order of magnitude. |
| CNCF Distribution (`registry:2`'s successor) | Not CNCF-listed separately (OCI-adjacent, CNCF landscape) | The reference OCI registry implementation — the minimal push/pull server most people meet as `registry:2`. No auth UI, no scanning, no frills.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Perfectly fine as a disposable cache; Zot supersedes it for anything we'd keep.                                                                                                                                                                                                |
| Artifactory                                  | Not CNCF — JFrog commercial                               | A universal artifact repository (containers plus Maven/npm/NuGet/everything) with enterprise governance.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Solves a multi-format, multi-team problem with a commercial contract. Not our shape; listed because Drew asked — the open-source answer to its container slice is Harbor/Zot.                                                                                                  |

**Why Zot over Harbor, for us specifically:** the registry's job in this estate is
(a) exist on the LAN, (b) mirror upstream so the cluster survives WAN outages,
(c) hold Nix-built images close to the nodes. Zot does all three in one binary
with a JSON config and a storage directory you can tar up and carry into an
air-gapped room — the OCI layout on disk means the data outlives the software.
Harbor's advantages (fine-grained RBAC, project quotas, replication topologies,
audit trails) are answers to "many teams, one registry, auditors" — a question
we are not asking. If that question ever arrives, Harbor is the graduated,
obvious step up, and moving is again a hostname change.

## 3. Artifact Hub — the catalogue, not a warehouse

[Artifact Hub](https://artifacthub.io/) (CNCF **Sandbox**) is a discovery index:
it crawls repositories of Helm charts, OLM operators, OPA/Gatekeeper policies,
Falco rules, KEDA scalers, Backstage plugins, container images, and more, and
presents them with rendered READMEs, values schemas, security reports, and
changelog tracking; `helm search hub` queries it natively
([Helm docs](https://docs.helm.sh/docs/v3/helm/helm_search_hub/), which describe
it as "a web-based application that enables finding, installing, and publishing
packages and configurations for CNCF projects… a Cloud Native Computing
Foundation sandbox project").

**What it is not:** a registry. You cannot push an image or chart _to_ Artifact
Hub; you publish to your own chart repo/registry and register the repository
with the Hub so people can find it.

**Should we use it:** as a consumer, yes, passively — it's the search box for
the Helm ecosystem we'll be consuming from Cozystack's world. Running our own
instance or publishing to it only matters if we start distributing charts
publicly.

## 4. Application definitions and image builds

| Project                                      | CNCF status                                                                                                                                                                                                                                    | What it is                                                                                                                                                       | Verdict for a Nix estate                                                                                                                                                                                                           |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Helm](https://helm.sh/)                     | **Graduated**                                                                                                                                                                                                                                  | The Kubernetes package manager: charts (templated manifests + values) are how nearly all third-party Kubernetes software ships, Cozystack's components included. | **Keep as a client.** We will install other people's charts. Authoring our own charts for Nix-managed services is usually a downgrade — the Nix module/flake can generate the manifests with types and reproducibility Helm can't. |
| [Kustomize](https://kustomize.io/)           | Not CNCF (Kubernetes SIG tooling, built into `kubectl`)                                                                                                                                                                                        | Template-free customisation of YAML by overlay patches.                                                                                                          | Fine for consuming raw manifests with small local deltas. Mostly displaced by Nix generation in our own stack.                                                                                                                     |
| Carvel (`ytt`, `kapp`, imgpkg…)              | Not CNCF (VMware/Broadcom open source)                                                                                                                                                                                                         | A composable toolset: `ytt` templates YAML as data, `kapp` deploys with resource ownership/waiting, `imgpkg` relocates images as OCI artifacts.                  | Genuinely well-designed, small adoption. No pull for us; noted because it appears in VMware-adjacent stacks.                                                                                                                       |
| [Dagger](https://dagger.io/)                 | Not CNCF (company-led)                                                                                                                                                                                                                         | CI pipelines as code (Go/TS/Python SDKs) running on BuildKit, portable across CI providers.                                                                      | Solves "my GitHub Actions YAML is unmaintainable" — a real problem, but our pipelines are Nix/Actions-shaped already and Dagger would be a second build system. No.                                                                |
| [ko](https://ko.build/)                      | Not CNCF (Google → Chainguard stewardship)                                                                                                                                                                                                     | Builds Go binaries into minimal OCI images without Dockerfiles — `ko build ./cmd/app` and the image reference lands in your manifests.                           | The Go-only special case of what Nix does generally. If a Go service ever needs a fast path outside the Nix build, ko is the trick; otherwise redundant.                                                                           |
| [Skaffold](https://skaffold.dev/)            | Not CNCF (Google) — **being retired**: removed from gcloud CLI releases after 2027-01-15, GitHub repository to be archived 2027-01-29, per its own documentation ([Skaffold docs announcement](https://skaffold.dev/docs/?ref=hackernoon.com)) | The dev-loop orchestrator: watch source → build images → deploy → tail logs.                                                                                     | Dead end by its maintainer's decision; do not adopt now. Its dev-loop role is the same niche as Telepresence (runtimes file), which survives.                                                                                      |
| Cloud Native Buildpacks                      | **Incubating**                                                                                                                                                                                                                                 | Heroku-style source-to-image: detect language, build a runnable OCI image without a Dockerfile.                                                                  | A reasonable on-ramp for other people's code; Nix is strictly stronger for ours. OpenFunction uses it under the hood (runtimes file).                                                                                              |
| [BuildKit](https://github.com/moby/buildkit) | Not CNCF (Moby)                                                                                                                                                                                                                                | The Dockerfile build engine (covered in the runtimes file).                                                                                                      | Already present wherever Dockerfiles are built; not a choice.                                                                                                                                                                      |

### Drew's closing question, answered directly

_If I use Nix, Dockerfiles, and GitHub, do I need any of this?_

**For building: no.** The stack already covers the whole pipeline with stronger
guarantees than the landscape offers:

- Nix builds the software reproducibly and can emit OCI images directly
  (`dockerTools.buildLayeredImage` / `streamLayeredImage`) — no Dockerfile, no
  Buildpacks, no ko, no Dagger required. Layered-image output also dedupes
  nicely in any registry.
- Dockerfiles remain the interchange format for _other people's_ software;
  BuildKit builds them, and that's the end of our involvement.
- GitHub Actions is the CI; GHCR is the registry (§1). Nothing in the build
  landscape improves on that until GHCR's four exit triggers fire.

**For deploying: one exception.** Helm, as a consumption client, because the
Kubernetes ecosystem — Cozystack first among equals — distributes software as
charts, and vendoring/forking those charts to avoid Helm would cost more than
Helm does. Everything else in the app-definition category (Kustomize, Carvel,
Skaffold) is optional tooling for problems Nix already answers.

**For distribution: later, and small.** When a local registry is warranted, Zot
(§2) — one binary, OCI-native, mirroring built in. Dragonfly and Harbor stay on
the shelf until scale or governance, respectively, asks for them by name.

## Bottom line for this slice

1. **Registry:** GHCR today, by design and not by accident; keep image references
   templated so exit stays a hostname change. First self-host step is Zot as a
   LAN mirror/cache, not a platform migration.
2. **Distribution:** Dragonfly is graduated, excellent, and unnecessary at our
   node count; Nydus/eStargz lazy pulling (runtimes file) is the nearer-term
   lever if cold starts ever hurt.
3. **Catalogue:** Artifact Hub is a search engine for other people's packages —
   use it to find charts; it stores nothing of ours.
4. **Builds/app-definitions:** Nix + Actions already win this category. Helm
   survives as a client for the Cozystack ecosystem; the rest of the category is
   solving YAML-era problems we don't have.
