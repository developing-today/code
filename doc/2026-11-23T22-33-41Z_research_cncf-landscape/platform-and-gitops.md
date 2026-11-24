# Platform, GitOps & Control Planes — the CNCF slice that builds clouds

_Lane CNCF-PLATFORM, 2026-11-23. Scope: platform frameworks, GitOps/deployment
reconciliation, infrastructure control planes, cluster provisioning, and the
durable-workflow neighbours. Every project carries its CNCF status
(graduated / incubating / sandbox / not CNCF) and a maturity verdict.
"What it is" is kept separate from "should we use it."_

**Our estate, for the "should we" judgements:** NixOS hosts today, Talos
Kubernetes planned, images built with Nix, CI already on GitHub Actions, and a
fleet queue/dispatch system (queue broker, placement, agent lanes) that we
already built and operate. Several famous projects below compete with things
we own. That is said out loud where it is true.

**Corrections to the premises this lane was handed** (checked 2026-11-23):

- Crossplane is no longer incubating — it **graduated on 6 November 2025**.
- Cadence is no longer foundation-less — Uber donated it to the **CNCF
  Sandbox in 2025**.
- Atlantis is a **CNCF Sandbox** project (accepted 18 June 2024), not a
  non-CNCF project.
- OpenTofu is Linux Foundation governed **and** was accepted as a CNCF
  Sandbox project in April 2025 (per multiple 2026 sources; the LF TSC still
  sets its roadmap).

---

## 1. Cozystack — the platform framework Drew is already circling

**Status: CNCF Sandbox (since 2025-02-28); an Incubating application is in
due diligence. Maturity verdict: real, in production at multiple operators,
moving fast (v1.6.3 was the latest stable in its September 2026 technical
review; v1.7 alphas exist) — but still Sandbox-stage governance, and the
release cadence is brisk enough that upgrades are a recurring event.**

### What it is

Cozystack is a framework that turns a set of bare-metal servers into a
multi-tenant private cloud with a Kubernetes-native API. You declare a
`Postgres`, a `VMInstance`, a `Bucket`, a tenant `Kubernetes` cluster — all
plain Kubernetes resources — and the platform provisions them. Ænix (the
company behind it) describes it as running "virtual machines, containers,
managed databases, S3 object storage, and GPU workloads on bare metal you
own, under one Kubernetes-native control plane with multi-tenant isolation,"
Apache 2.0, no per-core fees ([Ænix's Cozystack page](https://aenix.io/products/cozystack/)).

The real architecture, assembled from the project's own technical review
and independent teardowns:

- **Management cluster on Talos by default.** The recommended path is a full
  Talos-based distribution that owns the nodes; it is _not_ Talos-only — it
  can also install on generic Linux via Ansible (bootstrapping k3s) or onto a
  bring-your-own Kubernetes cluster ([Cozystack General Technical Review](https://github.com/cozystack/cozystack/blob/HEAD/GENERAL_TECHNICAL_REVIEW.md)).
- **Tenant Kubernetes = Kamaji + Cluster API + KubeVirt.** Each tenant gets
  a _real_ cluster, not a namespace: Kamaji runs the tenant's control plane
  (apiserver, controller-manager, scheduler) as pods in the management
  cluster, Cluster API manages lifecycle, and tenant worker nodes are
  KubeVirt VMs ([teardown of the Cluster API design](https://eden-cms-v2.onbex.co/blog/2026/07/11/cozystack-kubevirt-cluster-api-bare-metal-paas)).
- **Storage: LINSTOR/DRBD** replicated block storage (on ZFS) for VMs and
  databases, plus SeaweedFS for S3 object storage ([Ænix case study](https://aenix.io/case-studies/bare-metal-kubernetes-messaging-saas/)).
- **Networking: dual CNI** — Kube-OVN for the overlay/VPC-like tenant
  networking and Cilium for policy and load balancing, with MetalLB
  exposing services ([DeepWiki architecture overview](https://deepwiki.com/cozystack/cozystack/1.1-architecture)).
- **Managed services are operators behind one API:** PostgreSQL via
  CloudNativePG, MariaDB, MongoDB, ClickHouse, Redis/Valkey, Kafka,
  RabbitMQ, NATS, OpenSearch, Qdrant, FoundationDB — exposed to tenants as
  orderable resources ([Ænix's Cozystack page](https://aenix.io/products/cozystack/)).
- **Flux is inside.** Platform "applications" are Helm releases reconciled
  by Flux CD — the Cozystack API layer translates an `Application` resource
  into a Flux `HelmRelease` ([DeepWiki architecture overview](https://deepwiki.com/cozystack/cozystack/1.1-architecture)). This matters for the Flux-vs-Argo question below: adopting Cozystack quietly adopts Flux as the platform's internal reconciler, whatever you choose for your own apps.
- **Tenancy is a first-class CRD** with nested tenants, quotas, RBAC and
  audit — built for the service-provider model, where the operator and the
  customer are different parties ([Ænix's Cozystack page](https://aenix.io/products/cozystack/)).

There is at least one detailed public production story: a messaging-API
SaaS consolidated 13 Proxmox hosts onto one 8-node Cozystack cluster,
running ~25,000 per-customer instances in KubeVirt VMs with managed
MongoDB/PostgreSQL/RabbitMQ, operated day-to-day by effectively one person
([Ænix case study](https://aenix.io/case-studies/bare-metal-kubernetes-messaging-saas/)).

### What adopting it decides for you

This is the honest part. Cozystack is not a component; it is a bundle of
already-made decisions:

1. Your nodes run Talos (or you accept the second-class install paths).
2. Your block storage story is LINSTOR/DRBD — Rook/Ceph becomes the
   _object_-storage side note (SeaweedFS), not the main act.
3. "A VM" and "a database" and "a Kubernetes cluster" are all the same
   kind of request against one API — you give up assembling those pieces
   yourself.
4. Tenant isolation is cluster-grade (Kamaji control planes + VM workers),
   with the overhead that implies.
5. Flux reconciles the platform's own add-ons.
6. You inherit Ænix's release train and its opinions (Kube-OVN _and_
   Cilium, Kamaji, SeaweedFS) — each individually defensible, collectively
   a lot of moving parts that are _theirs_, not yours.

### Should we use it

For Drew's stated direction — bare metal, Talos, DRBD/LINSTOR, KubeVirt
VMs, managed databases, tenant clusters — Cozystack is the closest thing in
the landscape to "the thing you were about to build, already built." The
fit is unusually exact, down to the Talos + LINSTOR pairing. The costs are
the mirror image: it competes directly with the fleet system's own
placement/dispatch ambitions at the platform layer, and adopting it means
the platform's shape is Ænix's, evolving on Ænix's cadence, while the
project is still CNCF Sandbox. A reasonable posture: trial it as the
_tenant-facing cloud layer_ on the Talos hardware while fleet remains the
agent/dispatch layer above it — and go in knowing the storage, tenancy and
GitOps decisions listed above come with it.

---

## 2. GitOps: Flux vs Argo CD, and where GitHub Actions fits

First, the framing that dissolves half the confusion: **GitHub Actions is
CI — it builds, tests and publishes. GitOps tools are deployment
reconciliation — they continuously make the cluster match Git.** They are
not alternatives. Actions keeps its job either way; the question is only
which reconciler runs inside the clusters.

### Flux

**Status: CNCF Graduated (2022). Maturity verdict: production-proven,
lighter, and still healthy after its corporate sponsor (Weaveworks) shut
down in early 2024 — development continues under CNCF governance, v2.9
shipping in 2026.**

What it is: not an application but a _toolkit_ — a set of in-cluster
controllers (source, kustomize, helm, notification, image-automation) that
are ordinary Kubernetes operators speaking CRDs. There is no central
server and no bundled UI; multi-cluster means each cluster reconciles
itself from Git, independently. It leans entirely on Kubernetes RBAC
(service-account impersonation) rather than inventing its own. It decrypts
SOPS secrets natively at apply time. Its progressive-delivery partner is
Flagger ([Portainer's comparison](https://www.portainer.io/blog/argocd-vs-flux); [the boring comparison](https://systhoughts.com/posts/argocd-vs-flux-gitops-comparison)).

### Argo CD

**Status: CNCF Graduated (December 2022, as part of the Argo project).
Maturity verdict: the most widely adopted GitOps tool in the field —
roughly 60% of Kubernetes clusters in the 2025 Argo CD end-user survey —
and the one with the real UI.**

What it is: a standalone _platform_ — API server, repo server, application
controller, its own RBAC with SSO, AppProjects for multi-tenancy,
ApplicationSets for templating apps across clusters, and a genuinely good
web UI showing diffs, health, sync history and resource trees. One central
instance manages many clusters hub-and-spoke. Auto-sync/self-heal are
opt-in rather than default ([Akuity's comparison](https://akuity.io/blog/argo-cd-flux-comparison)).

### The rest of the Argo family (separate tools, one brand)

- **Argo Workflows** — a Kubernetes-native workflow/DAG engine: jobs,
  pipelines and parallel compute expressed as CRs running in pods. CI-like
  and batch-like work _inside_ the cluster. Not a deployer.
- **Argo Rollouts** — progressive delivery: replaces/augments Deployment
  with canary and blue-green strategies driven by metrics.
- **Argo Events** — event-driven triggers (webhooks, queues, schedules)
  that start Workflows or other actions.

All four sit under the single graduated "Argo" CNCF project.

### Headline answer for us

Both are graduated, both pull from Git, both are safe. The difference is
operating model, not capability:

- **Flux** is the better fit for this estate. Drew's clusters will be
  few, self-operated, and managed by someone fluent in Kubernetes; Flux's
  per-cluster controllers have no central control plane to run, no second
  RBAC system to learn, native SOPS decryption (we already use SOPS), and
  it is _already inside Cozystack_ reconciling the platform itself — one
  reconciler to understand instead of two.
- **Argo CD** earns its keep when many teams or many clusters need a
  shared window and guardrails — the UI, AppProjects and central view are
  real value at organisational scale we do not have. If a future
  multi-tenant Cozystack deployment wants a tenant-facing deployment UI,
  Argo CD is the piece to reconsider then.

Verdict: **Flux for our own GitOps; Argo CD only if/when a UI for other
humans becomes the requirement.** Do not run both as reconcilers of the
same state.

---

## 3. Crossplane — the infrastructure control plane

**Status: CNCF Graduated (6 November 2025 — it entered as Sandbox in 2020,
Incubating in 2021, and graduated with 3,000+ contributors from 450+
organisations). Maturity verdict: production-grade and the default answer
to "infrastructure as Kubernetes objects" — with a real learning curve and
a heavy CRD footprint if you install big providers carelessly.**

### What it is

Crossplane extends a Kubernetes cluster so that _external_ infrastructure
becomes Kubernetes objects. Install a **Provider** (AWS, Azure, GCP,
GitHub, Vault, Kubernetes, Helm, Terraform…) and resources like an S3
bucket or an RDS instance become **Managed Resources** — CRs whose
controllers call the provider's API and keep reconciling the real resource
back into line if it drifts ([cloudparts' Crossplane profile](https://cloudparts.dev/technologies/crossplane/)).

The platform-team half is **Compositions**: you define your own API with a
Composite Resource Definition (XRD) — say `ObjectStore` with just a region
and a retention setting — and a Composition (a pipeline of functions)
expands each instance into the real underlying resources. Application
teams consume your small API; they never see the fifty cloud knobs
underneath. Crossplane 2.0 (August 2025) made composite and managed
resources namespaced by default, let compositions include plain Kubernetes
resources alongside cloud ones, dropped the old claim objects, and added
Operations for one-off/scheduled jobs ([cloudparts' Crossplane profile](https://cloudparts.dev/technologies/crossplane/)).

The clean division of labour when combined with GitOps: "ArgoCD answers:
does the cluster contain what Git says? Crossplane answers: does the cloud
contain what the cluster objects say?" ([Medium walkthrough](https://medium.com/@anubhavbhatt/devops-concepts-05-25-crossplane-with-argocd-managing-cloud-infrastructure-through-kubernetes-337668449295)).

### Should we use it

Mostly **no — not now**, and the reason is overlap, not quality:

- Our infrastructure is NixOS/Talos declared in Nix and Git, not cloud
  APIs. Crossplane's providers shine against AWS/Azure/GCP APIs; we have
  almost no such surface.
- Cozystack already _is_ a Kubernetes-native provisioning layer for the
  things we'd compose (databases, buckets, clusters, VMs) with its own
  curated APIs. Running Crossplane beside it means two control planes
  offering self-service infrastructure with two different abstraction
  styles.
- Fleet already owns dispatch/placement for agent work.

Crossplane earns its keep the day we need to provision and continuously
reconcile _external_ resources — cloud buckets, DNS, GitHub org objects,
SaaS tenants — through the same GitOps flow as cluster state, or offer a
composed self-service API to tenants _outside_ what Cozystack provides.
Until that day it is a second control plane we'd be operating for
hypothetical consumers. Revisit if the estate grows a real cloud-API
surface; the project will still be there, and it is graduated now.

---

## 4. "Argonize" — an identification, honestly labelled

**Finding: there is no infrastructure project of any standing named
"Argonize."** Searches for the name against Crossplane/Kubernetes/platform
tooling surface only: `go-argonize`, a tiny Go helper wrapping the Argon2
password-hashing library ([KEINOS' repositories](https://github.com/KEINOS?tab=repositories)) — cryptography plumbing, unrelated to platform engineering.

The nearest real matches for what Drew may have seen or heard:

- **Argonaut** (argonaut.dev, YC S21) — a commercial SaaS that deploys
  apps and infra to AWS/GCP: Kubernetes PaaS + CI pipelines + managed
  Terraform state with prebuilt modules ([Launch HN](https://news.ycombinator.com/item?id=36480230)). Internally it is _built on_ Argo CD for delivery ([Argonaut's own Argo CD write-up](https://community.ops.io/argonaut/argocd-in-action-a-behind-the-scenes-tour-of-argonauts-gitops-approach-3ago)). It is not a Crossplane competitor so much as a hosted platform that _uses_ the Argo/Terraform stack — the comparison to Crossplane is "buy a hosted deploy platform" vs "run an open control plane yourself."
- **Argo** itself — the most likely source of the name in conversation;
  covered in §2.
- An "Argon" security tool could not be verified as a distinct,
  notable project in this pass; treat that branch as **not found**, not
  as disproven.

So the honest answer to "how does Argonize compare to Crossplane" is:
under that name, there is nothing to compare. If it was Argonaut, it is a
commercial hosted product in a different category entirely.

---

## 5. kcp — Kubernetes-like control planes, minus the workloads

**Status: CNCF Sandbox. Maturity verdict: serious building block with
real industrial users (Kubermatic, SAP, IBM's KubeStellar orbit), but a
tool for platform _builders_, not platform _users_.**

### What it is

kcp takes the Kubernetes API machinery — apiserver, CRDs, watches,
reconcile loops, RBAC — and removes the container orchestration. In its
place it offers **workspaces**: fully isolated logical clusters, each with
its own URL, API set and RBAC, "as cheap and quick as a namespace," hosted
on shared shards the way pods are scheduled to nodes. **APIExports** and
**APIBindings** let one workspace publish APIs that thousands of other
workspaces consume — an API-as-a-Service pattern aimed at SaaS providers
([kcp.io](https://www.kcp.io/); [Rawkode's kcp profile](http://rawkode.academy/technology/kcp)). It intentionally has no pods: it "does not replace Kubernetes, but complements it as a backend to host Kubernetes-like APIs as SaaS" ([kcp.io](https://www.kcp.io/)).

Its flagship downstream is Kubermatic's developer platform, which uses
kcp workspaces as the tenancy layer under a service catalog that actually
provisions ([Kubermatic on KDP](https://www.kubermatic.com/blog/what-is-kdp/)).

### Should we use it

Not directly. kcp is what you build on when you are selling API tenancy
to thousands of customers. Our tenancy story, if Cozystack lands, is
Kamaji-hosted real clusters. The one place kcp thinking is relevant:
Cozystack's own API layer and any future fleet control-plane APIs borrow
the same "Kubernetes resource model as the product API" idea — kcp is the
reference implementation of that idea at extreme scale. Watch, don't adopt.

---

## 6. Cluster provisioning: Cluster API, Sveltos, and friends

### Cluster API (CAPI)

**Status: a Kubernetes SIG project (not a standalone CNCF project).
Maturity verdict: the standard way clusters themselves are provisioned
declaratively — Cluster/ Machine/ MachineDeployment objects reconciled by
infrastructure providers (AWS, Azure, vSphere, bare metal, KubeVirt…).**

This is the layer that answers "who creates the clusters the GitOps tool
then manages." It matters to us concretely because **Cozystack already
uses it**: tenant clusters are CAPI clusters whose control planes are
Kamaji pods and whose workers are KubeVirt VMs (§1). If we ever provision
clusters outside Cozystack, CAPI is the default mechanism to reach for.

### Sveltos

**Status: CNCF Sandbox. Maturity verdict: focused and useful at fleet
scale; overkill for a handful of clusters.**

Sveltos is a controller that deploys add-ons — Helm charts, Kustomize
overlays, raw resources — across many clusters from one management
cluster, selecting targets by label so new clusters inherit the right
add-ons automatically and drift gets repaired ([Rawkode's Sveltos profile](http://rawkode.academy/technology/sveltos)). It is the state-distribution
half of Mirantis' k0rdent platform (CAPI + Flux + Sveltos). For us: this
is the tool you adopt when per-cluster GitOps repos become unmanageable
across dozens of clusters. We are not there; Flux per cluster is simpler.

### Kratix

**Status: open-source platform framework by Syntasso; CNCF Sandbox status
could not be verified in this pass — treat as not CNCF until checked.
Maturity verdict: an interesting idea, small ecosystem.**

Kratix's unit is the **Promise**: a platform team publishes a contract
("a database, like this") that bundles the API developers request against
_and_ the workflow that fulfils it, which can drive Terraform, Crossplane,
or plain pipelines underneath. It is Crossplane's composition idea
generalised beyond Kubernetes resources. Notable as a design reference for
platform APIs; not an adoption candidate for an estate of our size.

### OpenChoreo (the project behind the "Choreo" name)

**Status: CNCF Sandbox; shipped its 1.0 alongside acceptance (March
2026). Maturity verdict: a complete, opinionated IDP from WSO2 — the
"buy the whole platform" end of the spectrum.**

OpenChoreo is the open-source evolution of WSO2's Choreo SaaS: a complete
developer platform for Kubernetes with developer abstractions, a
Backstage-powered portal, built-in CI/CD, GitOps and observability in one
cohesive system, explicitly positioned _against_ the assemble-it-yourself
toolkit approach ([OpenChoreo's 1.0 announcement](https://openchoreo.dev/blog/openchoreo-joins-cncf-and-ships-1-0/)). It competes with the
Cozystack-shaped decision, not with any single tool in it: adopting it
means adopting WSO2's platform shape instead of Ænix's. Mentioned here so
the name is placed; Cozystack remains the closer fit for bare-metal/Talos.

---

## 7. K3s — yes, it's the laptop one (with caveats)

**Status: CNCF Sandbox (since August 2020; an incubation application
filed in November 2025 is still open). Maturity verdict: utterly proven in
the field — arguably the most-run Kubernetes distribution on earth at the
small end — despite its nominal Sandbox label.**

Drew's memory is right: K3s is Rancher/SUSE's lightweight Kubernetes —
upstream Kubernetes packaged as a single binary under 100 MB that installs
with one command and bundles containerd, Flannel networking, CoreDNS,
Traefik ingress, a service load balancer, local-path storage and a Helm
controller ([cloudparts' K3s profile](https://cloudparts.dev/technologies/k3s/)). Its signature trick is the datastore:
cluster state lives in SQLite through **Kine** (an etcd-API-to-SQL shim)
by default; embedded etcd or an external Postgres/MySQL are the HA
options ([Build Daily's K3s profile](https://www.builddaily.io/resources/k3s/)).

How it differs from the alternatives in Drew's world:

- **vs kubeadm Kubernetes:** same conformant Kubernetes, a fraction of
  the installation and memory; you give up some configurability and the
  "stock" layout.
- **vs Talos:** they solve different layers. Talos shrinks and hardens
  the _node OS_ (immutable, API-managed, no shell) and runs full
  Kubernetes on it. K3s shrinks _Kubernetes itself_ and runs on a normal
  Linux you still administer. Talos is the more opinionated, more secure
  node story; K3s is the faster, friendlier one.
- **For us:** K3s is the right answer for throwaway/dev/CI clusters, a
  laptop cluster, or a single small box — and it is literally Cozystack's
  bootstrap path on generic Linux. It is not a competitor to the Talos
  plan for the real fleet; it's the complement for everywhere Talos is
  more ceremony than the job deserves.

---

## 8. OpenYurt — Kubernetes stretched to the edge

**Status: CNCF Incubating (accepted July 2025; Sandbox since September
2020; originated at Alibaba Cloud). Maturity verdict: production-used at
Alibaba scale for genuine edge fleets; solves a problem we mostly don't
have.**

### What it is

OpenYurt extends an _unmodified_ upstream Kubernetes cluster so nodes
across unreliable cloud↔edge links behave. Its core piece is **YurtHub**,
an agent on each edge node that proxies kubelet/apiserver traffic and
caches the node's objects on local disk: when the link drops, the node
keeps running its workloads from cache instead of being evicted as dead —
"edge autonomy." Around that sit node pools (geography as the unit of
deployment and upgrade), the Raven networking layer for cross-region
pod traffic, a tunnel for cloud→edge reachability, and EdgeX integration
for IoT devices ([CNCF's incubation announcement](https://www.cncf.io/blog/2025/07/02/openyurt-becomes-a-cncf-incubating-project/); [Rawkode's OpenYurt profile](http://rawkode.academy/technology/openyurt)). The design bet is
explicitly non-invasive: no fork, and a cluster can be converted to
OpenYurt and back.

### Should we use it

Only if the estate grows genuinely remote nodes on bad links — a node at
another site that must keep working through WAN outages. A home/lab plus
a small co-located fleet on a LAN gets nothing from YurtHub's caching
that ordinary Kubernetes doesn't already do, and pays for it in extra
components. File under "know it exists for the day a remote site appears."

---

## 9. The Terraform corner: OpenTofu and Atlantis

### OpenTofu

**Status: Linux Foundation project (Technical Steering Committee
governance); also accepted as a CNCF Sandbox project in April 2025 per
2026 sources. Not a CNCF-graduated tool. Maturity verdict: the fork won —
it is now a separately governed tool with features Terraform's open CLI
lacks, not a protest copy.**

Drew's memory is exactly right: OpenTofu is the fork of Terraform created
in August 2023 when HashiCorp relicensed Terraform from MPL 2.0 to the
Business Source License; the fork started from the last MPL release
(1.5.x), is MPL 2.0, reads the same HCL, uses the same providers and the
same state format, and has since added built-in state encryption,
provider `for_each`, and `-exclude` ([Encore's 2026 comparison](https://encore.dev/articles/opentofu-vs-terraform-2026); [Scalr's OpenTofu guide](https://scalr.com/learning-center/what-is-opentofu)). HashiCorp is now part of IBM; Terraform remains BSL.

**For us:** mostly moot. Nix declares our machines; Talos declares our
clusters; there is no HCL estate to protect. OpenTofu matters at the
margins — consuming a Terraform module someone else wrote, or managing a
cloud resource Nix doesn't reach — and if that day comes, OpenTofu (not
Terraform) is the default on licence grounds alone.

### Atlantis

**Status: CNCF Sandbox (accepted 18 June 2024). Maturity verdict: the
standard self-hosted answer to "run Terraform from pull requests," alive
and releasing steadily (≈17 stable releases in the year to October
2026).**

Atlantis is one Go server that listens for PR webhooks: open a PR
touching Terraform/OpenTofu files and it runs `plan` and posts the real
plan as a PR comment; a permitted human replies `atlantis apply` and it
applies, with credentials that live only on the Atlantis host, never with
developers ([CNCF's Atlantis page](https://www.cncf.io/projects/atlantis/); [detailed Atlantis walkthrough](https://www.mrvitej.in/devops/cncf-atlantis/)). It automates
whichever engine you point it at — Terraform or OpenTofu.

**For us:** Atlantis automates a workflow we don't run. Our PR automation
equivalent already exists in spirit (GitHub Actions + fleet dispatch on
PRs). If an OpenTofu estate ever appears, Atlantis is the obvious PR
front-end for it; until then it's a name to know, not a tool to install.

---

## 10. Cadence (and Temporal) — durable execution, a different layer

**Status: Cadence — CNCF Sandbox (donated by Uber in 2025; Apache 2.0;
150+ companies in its ecosystem). Temporal — not a foundation project;
MIT-licensed, built by Temporal Technologies, the company founded by
Cadence's creators. Maturity verdict for both: production-proven at very
large scale.**

### What durable execution is

Cadence (open-sourced by Uber in 2017) and Temporal (a 2019 fork of
Cadence by its original authors) solve one problem: long-running business
logic written as ordinary code that _cannot lose its place_. The platform
records every step of a workflow in an append-only event history; if the
worker process crashes, deploys, or the machine dies, another worker
replays the history, rebuilds the workflow's exact state, and continues
from the same line. Retries, timers that sleep for days, signals and
timeouts are platform primitives instead of hand-built state tables and
dead-letter queues ([cloudparts' Temporal profile](https://cloudparts.dev/technologies/temporal/); [Uber's CNCF announcement](https://www.uber.com/ie/en/blog/cadence-workflow-joins-the-cloud-native-computing-foundation/)).

### How it compares to our fleet queue — honestly

Different layer, and the difference matters:

- **Fleet's queue** dispatches _units of agent/infrastructure work_ —
  tasks claimed by lanes, placed on seats, retried as whole tasks. Its
  state is a task row; its recovery model is "re-dispatch the task."
- **Cadence/Temporal** make a _single application's own multi-step
  process_ durable from the inside: step 3 of 12 survives a crash and
  resumes as step 3, with the values from steps 1–2 intact, because the
  workflow's history is replayed. A queue cannot give you that; it can
  only restart the whole unit or rely on the application checkpointing
  itself.

So Cadence does not compete with fleet — it is the thing you'd embed
_inside_ a long-running service whose internal process must survive
restarts (an IdP provisioning flow that waits three days for an approval;
an agent run that must resume mid-plan rather than restart). If a future
service has that shape, Temporal is the pragmatic pick (larger ecosystem,
managed cloud option) and Cadence the governance pick (CNCF, Apache 2.0,
Uber still a major user). For fleet itself: no change — task-level
dispatch is the right granularity for agent work, and adding a durable
execution engine underneath it would be a second state machine to operate.

---

## 11. Backstage and the packaging layer

### Backstage

**Status: CNCF Incubating (since March 2022; Spotify's creation).
Maturity verdict: the standard open developer-portal framework — and a
framework is exactly what it is.**

Backstage gives you a software catalog, scaffolding templates, TechDocs
and a plugin system — as a React/Node application _you_ build, extend and
operate; every adopter's Backstage is its own codebase ([iThub's Backstage profile](https://www.ithub.directory/paas-app-deployment/backstage)). It pays off when an
organisation has hundreds of services and dozens of engineers who can't
find them. **For us it is overkill, plainly:** one operator, a fleet
system that already knows what exists, and books/portals we generate
ourselves. A portal becomes interesting if the estate's service count
grows by an order of magnitude _and_ other humans need to browse it.

### Helm vs Kustomize

**Helm: CNCF Graduated. Kustomize: not a CNCF project — it's part of
Kubernetes itself (SIG CLI, built into `kubectl`). Both are mature;
they're complements more than rivals.** Helm is a package manager (charts,
values, versioning, a registry story); Kustomize is a patch/overlay
system over plain YAML with no templating language. In practice: consume
upstream software as Helm charts (Cozystack's own add-ons are Helm
releases under Flux), keep our own manifests as plain YAML + Kustomize
overlays where a base needs per-cluster variation. Both are driven
equally well by Flux, which has native controllers for each.

---

## 12. The AI-platform names: Agent Substrate, AI Runway, kagent

These three are real, recent, and easy to conflate. They sit at three
different layers:

### Agent Substrate

**Status: CNCF Sandbox (donated by Google in November 2026, alongside
gVisor and OpenRL); the open-source project reached 1.0, and Google sells
it managed as "Agent Substrate on GKE." Maturity verdict: weeks old as a
CNCF project but Google-production-shaped; vendor claims, not yet
independently verified.**

Agent Substrate is a runtime for _sandboxed AI agents_ on Kubernetes.
Its core move is decoupling the agent's state from compute: an **Actor**
holds an agent session's memory, files and identity; **Workers** are
reusable sandboxes drawn from a warm pod pool; **Gateways** enforce
access control and inject secrets by actor identity. When an agent waits
on a model turn, its state is snapshotted to storage and the worker
returns to the pool — Google claims restore in under 500 ms, 10× the
density of one-container-per-agent, and 500+ suspend/resume activations
per second ([Google Cloud's announcement](https://cloud.google.com/blog/products/containers-kubernetes/open-infrastructure-for-autonomous-agents-comes-to-cncf); [NVIDIA's technical blog](https://developer.nvidia.com/blog/?p=124728)). Isolation is gVisor or
Cloud Hypervisor microVMs.

_Relevance to us:_ this is the first CNCF project aimed squarely at the
problem our fleet system solves by hand — many agent sessions, mostly
idle, needing isolation and fast resume. It is a **watch closely** item:
if fleet ever needs to run agent sandboxes _inside_ Kubernetes at real
density, Substrate is the substrate to evaluate before building more of
our own. Today our lanes run on seats/laptops, not in-cluster sandboxes,
so it solves a problem one step removed from ours.

### AI Runway

**Status: CNCF Sandbox (accepted 28 September 2026; first commit December
2025 — a young project). Maturity verdict: early; a focused tool, not a
platform.**

AI Runway "provides a web UI and a unified Kubernetes CRD to deploy AI
models across multiple inference engines and providers" ([CNCF's AI Runway page](https://www.cncf.io/projects/ai-runway/)). In plain terms: one
declarative API + console for putting models onto whatever serving engine
(vLLM-style engines, cloud providers) sits underneath. It is model
_deployment_ plumbing. For an estate whose model access is API-based
(opencode routes, provider pools), there is nothing here yet; it becomes
relevant only if we self-host inference on the Talos/GPU hardware.

### kagent

**Status: CNCF Sandbox (accepted 22 May 2025; Solo.io's creation).
Maturity verdict: the most developed of the "agents on Kubernetes"
frameworks; its 1.0 line is a breaking redesign of the 0.x model, so it
is still settling.**

kagent is a framework for running AI agents _as Kubernetes resources_ —
agents, their MCP tools, model configs and egress policies declared as
CRDs, with approval gates for risky tool calls and OpenTelemetry traces
for everything ([detailed kagent profile](https://www.mrvitej.in/devops/cncf-kagent/)). Notably, kagent 1.x
_runs on Agent Substrate_ for its sandboxed sessions — the two projects
compose rather than compete ([detailed kagent profile](https://www.mrvitej.in/devops/cncf-kagent/)). It is aimed at
operations agents (troubleshoot the cluster, query Prometheus, propose
fixes) more than at general agent workloads. For us: conceptually
adjacent to fleet's lanes, architecturally committed to a
Kubernetes-everything world we haven't adopted. Name to know; no action.

---

## 13. Verdict table

| Project                            | CNCF status                            | Verdict for our estate                                                                    |
| ---------------------------------- | -------------------------------------- | ----------------------------------------------------------------------------------------- |
| Cozystack                          | Sandbox (Incubating in due diligence)  | Trial as the tenant-cloud layer on Talos; know it decides storage/tenancy/GitOps for you  |
| Flux                               | Graduated                              | **Adopt** — our GitOps reconciler; already inside Cozystack                               |
| Flagger                            | Graduated                              | Later, if canary deploys matter                                                           |
| Argo CD                            | Graduated                              | Only if a deployment UI for other humans becomes a requirement                            |
| Argo Workflows / Rollouts / Events | Graduated (Argo)                       | Pieces to borrow later; none needed now                                                   |
| Crossplane                         | Graduated (Nov 2025)                   | Not now — Nix + Cozystack + fleet cover its ground; revisit with a real cloud-API surface |
| "Argonize"                         | —                                      | Does not exist as an infra project; nearest match Argonaut is a hosted SaaS on Argo CD    |
| kcp                                | Sandbox                                | Watch; a builder's tool for API tenancy at SaaS scale                                     |
| Cluster API                        | K8s SIG project                        | Already in our future via Cozystack; default cluster provisioner otherwise                |
| Sveltos                            | Sandbox                                | At dozens of clusters, not before                                                         |
| Kratix                             | Not verified as CNCF                   | Design reference only                                                                     |
| OpenChoreo                         | Sandbox                                | The WSO2-shaped alternative to Cozystack; not our shape                                   |
| K3s                                | Sandbox (incubation applied)           | Adopt for dev/CI/laptop/small-box clusters; complements Talos                             |
| OpenYurt                           | Incubating                             | Only with genuinely remote, badly-connected nodes                                         |
| OpenTofu                           | Linux Foundation (+ CNCF Sandbox 2025) | Default _if_ HCL is ever needed; Nix covers us today                                      |
| Atlantis                           | Sandbox                                | The PR front-end for an OpenTofu estate we don't have                                     |
| Cadence                            | Sandbox                                | Durable execution for inside a service; does not compete with fleet's queue               |
| Temporal                           | Not CNCF                               | Same layer as Cadence; pragmatic pick if that need appears                                |
| Backstage                          | Incubating                             | Overkill at our scale — a framework we'd have to staff                                    |
| Helm                               | Graduated                              | Use for consuming upstream charts                                                         |
| Kustomize                          | K8s SIG (not CNCF)                     | Use for our own manifests' per-cluster overlays                                           |
| Agent Substrate                    | Sandbox (Nov 2026)                     | Watch closely — first CNCF answer to in-cluster agent sandboxing at density               |
| AI Runway                          | Sandbox (Sep 2026)                     | Only if we self-host model inference                                                      |
| kagent                             | Sandbox                                | Adjacent to fleet's problem; Kubernetes-committed; no action                              |

_Count: 26 projects/frameworks covered (counting the Argo family as four
and the three AI-platform names individually)._
