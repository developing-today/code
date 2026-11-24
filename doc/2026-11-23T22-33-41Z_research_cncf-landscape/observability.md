# Observability & Operations — CNCF Landscape Research

*Lane CNCF-OBSERVE · researched 2026-11-23 · sources linked inline throughout*

**Scope.** The observability slice of the cloud-native landscape: metrics, logs, traces,
profiles, dashboards, cost, feature flags, chaos, and the AI-SRE newcomer — what each
project is, what state it is actually in, and whether it belongs in *our* estate.

**Our estate, for the "should we" verdicts.** NixOS hosts today; Talos-based Kubernetes
planned; Cozystack as the platform layer on top of that Kubernetes; the fleet / id /
atlas / mcpx projects as the workloads. Small team, self-hosted, performance-sensitive,
no enterprise-compliance requirements. One fact shapes half the verdicts below:
**Cozystack already ships an observability stack** — VictoriaMetrics (cluster) +
VictoriaLogs + Grafana + Fluent Bit + Alerta at platform and tenant level, with optional
VictoriaTraces and a per-tenant OpenTelemetry Collector for tracing
([Cozystack monitoring architecture](https://deepwiki.com/cozystack/cozystack/8-development-and-deployment);
[Cozystack introduction](https://cozystack.io/blog/2024/02/introducing-cozystack-a-free-paas-platform-based-on-kubernetes/)).
Where Cozystack has already decided, the honest question is not "which tool is best"
but "is the built-in choice good enough that fighting it would be a mistake."

**How to read this doc.** Each project gets: *what it is* → *current state* (with CNCF
status labelled: graduated / incubating / sandbox / not CNCF) → *should we use it*.
A maturity verdict is given per project. Claims are linked; vendor benchmark numbers
are flagged as vendor numbers where that matters.

**Projects covered: 23** — Prometheus, VictoriaMetrics, Grafana, Grafana Alloy,
Grafana Mimir, Grafana Loki, Grafana Tempo, Grafana Pyroscope, SigNoz, OpenTelemetry,
OpenTracing, Jaeger, Pixie, Trickster, HolmesGPT, OpenCost, OpenFeature (+ flagd),
Chaos Mesh, Thanos, Cortex, Fluent Bit, Perses, Kepler.

---

## The three headline answers

### 1. Should we still use Prometheus? — Yes, but mostly as a *language and a model*, not always as the server

Prometheus the project is healthy and still the centre of gravity: CNCF graduated,
on the 3.x line (3.15 in September 2026, 3.16 current in late November), with
designated LTS lines (3.5, then 3.13) — a maturity signal the 2.x era never had
([Prometheus 3.x release state](https://sharpskill.dev/en/blog/devops/prometheus-vs-grafana-vs-datadog-monitoring-devops-interview);
[3.x notes and LTS](https://www.devopsandinfra.com/tool/prometheus/)).
Prometheus 3.0 (Nov 2024) added UTF-8 metric/label names, Remote Write 2.0, and a
built-in OTLP receiver; native histograms became **stable in 3.8** (Dec 2025), though
scraping them is opt-in per job via `scrape_native_histograms`
([Prometheus 3.0 changes](https://www.greptime.com/blogs/2026-05-09-prometheus-3-and-greptimedb)).
The Q3 2026 CNCF Technology Radar places Prometheus and OpenTelemetry alone in
"Adopt" for observability
([CNCF Tech Radar Q3 2026](https://www.cncf.io/wp-content/uploads/2026/11/CNCF-Tech-Radar-survey-W5-261020-3.pdf)).

But the *server* is no longer the automatic choice for storage. VictoriaMetrics beats
it on RAM and disk at every measured scale (details below), and Cozystack already runs
VictoriaMetrics. So the estate answer is split:

- **Inside Cozystack:** VictoriaMetrics is the metrics store; Prometheus-format
  exporters, PromQL-style queries (MetricsQL is a near-superset), and Prometheus
  alerting rules remain the working surface. Don't rip that out to run vanilla
  Prometheus — you'd pay more RAM for less retention to win an argument nobody is having.
- **Outside Kubernetes** (NixOS hosts, the fleet's own infrastructure, small standalone
  installs): a single Prometheus 3.x with local TSDB is still the simplest correct
  thing. Its 15-day default retention is a default, not a cap, and 3.x's OTLP receiver
  means OTel-instrumented services can push straight into it without a collector.

### 2. Grafana — still the default dashboard layer, and it's not close; but know what it is now

Grafana remains the default visualisation and alerting UI for self-hosted stacks, and
nothing in this research displaces it for us — Cozystack ships it, every backend we
care about has a datasource for it, and the dashboard ecosystem (thousands of published
dashboards) has no rival. Two posture facts to hold clearly:

- **Grafana is not a CNCF project, and neither are Loki, Tempo, or Mimir.** The "LGTM
  stack" is a *vendor* stack from Grafana Labs that happens to be open source. Grafana,
  Loki, Tempo, Mimir, and Pyroscope are AGPL-3.0; Alloy is Apache-2.0
  ([LGTM component licences](https://docs.synedrix.io/grafana-lgtm/)). AGPL is fine for
  self-hosting; it matters if we ever embed or resell.
- **Grafana Labs runs an open-core business.** The OSS builds are genuinely complete
  for our purposes, but the company's gravity is toward Grafana Cloud and Enterprise,
  and the maintenance signals show it: Grafana Agent reached end of support in
  December 2025 (Alloy is the replacement), and in early 2026 the Helm charts for
  Grafana, Tempo, and Loki moved out of Grafana's own chart repos into a
  `grafana-community` organisation
  ([LGTM stack changes 2025–2026](https://kb.ekarisky.com/observability/lgtm/)).
  None of that blocks adoption; it does mean "the community maintains the packaging"
  is increasingly the real support model for the self-hosted path.

The CNCF's own candidate for Grafana's slot is Perses (below) — dashboards-as-code,
Apache-2.0, and architecturally much more "us" — but it is a sandbox project at 0.5x
and is a watch item, not a replacement today.

### 3. "What's the open-source Datadog?" — There isn't one. There are two honest approximations

Datadog is one product: one agent, one backend, one UI, every signal, plus APM, RUM,
profiling, security monitoring, and a bill. No open-source project replicates that.
What exists:

- **The assembled stack** (what most self-hosters actually run): OpenTelemetry for
  instrumentation and collection, Prometheus or VictoriaMetrics for metrics, Loki or
  VictoriaLogs for logs, Tempo or Jaeger for traces, Grafana on top. This is more
  pieces to operate than Datadog, but each piece is best-of-breed and replaceable, and
  the instrumentation (OTel) is portable if we ever change our minds.
- **SigNoz** — the closest single-product attempt: one UI over metrics/logs/traces/APM
  stored in ClickHouse, OpenTelemetry-native from day one, and the project that most
  deliberately copies the Datadog experience
  ([SigNoz positioning](https://www.opensourcehunter.com/datadog-alternatives/)).
  Evaluated in its own section below; the short version is *promising, genuinely
  usable, but pre-1.0 open-core from a small VC-backed company* — a different risk
  shape than the assembled stack, not a strictly better one.
- **Pixie** was the other single-product attempt (eBPF auto-everything, below). It
  stalled; the zero-instrumentation idea survived but moved into OpenTelemetry itself.

Anyone who tells Drew there is a drop-in open-source Datadog is selling something.

---

## The estate stack this research implies

| Layer | Pick | Why |
|---|---|---|
| Instrumentation | OpenTelemetry SDKs + semantic conventions | Graduated, vendor-neutral, every backend accepts OTLP |
| Collection (in-cluster) | Cozystack's per-tenant OTel Collector; Fluent Bit for logs | Already shipped by the platform; Fluent Bit is the default log shipper everywhere |
| Metrics store | VictoriaMetrics (via Cozystack); Prometheus 3.x standalone outside K8s | VM wins on resources; Prometheus wins on simplicity at small scale |
| Logs | VictoriaLogs (via Cozystack); Loki if we ever leave Cozystack | Follows the metrics decision |
| Traces | Jaeger v2 or Tempo — see the traces section; Cozystack offers VictoriaTraces | Either is fine; pick by which UI we standardise on |
| Dashboards/alerts | Grafana (+ Perses on watch) | Default, shipped, ecosystem |
| Cost visibility | OpenCost, once multi-tenant clusters make cost allocation a real question | Prometheus-native, exports into the stack above |
| Feature flags | OpenFeature API in code; flagd as the backend | Cheap insurance against vendor lock-in; flags-in-Git fits our workflow |
| Chaos | Chaos Mesh — later, in a staging cluster, maybe | A practice, not a product we need yet |
| AI incident help | HolmesGPT — trial only, read-only, local models if possible | Interesting, sandbox-grade, newly acquired |

---

## Metrics

### Prometheus — CNCF **graduated** · verdict: **adopt (as model + small-scale server)**

**What it is.** The pull-based metrics system that defined cloud-native monitoring:
instrumentation libraries expose a `/metrics` endpoint, the server scrapes it, stores
samples in a local time-series database, and answers PromQL queries; Alertmanager
handles alert routing. Its exposition format and PromQL are the de-facto standard
every other tool in this doc either implements or interoperates with.

**Current state.** 3.x is a substantial modernisation, not a re-badge:

- **Native histograms stable** in 3.8 (Dec 2025): a whole distribution in one series
  with automatically-sized exponential buckets — higher resolution at a fraction of
  the cardinality of classic per-bucket series. Scraping them is opt-in
  (`scrape_native_histograms: true`); the old feature flag became a no-op in 3.9, which
  silently stops ingestion if you upgrade carelessly
  ([native histogram state](https://sharpskill.dev/en/blog/devops/prometheus-vs-grafana-vs-datadog-monitoring-devops-interview)).
- **OTLP receiver built in**: `--web.enable-otlp-receiver` exposes
  `/api/v1/otlp/v1/metrics`, so OTel SDKs and Collectors can push metrics directly —
  no translation sidecar. Metrics only; traces and logs still need real backends
  ([Prometheus 3.0 OTLP ingestion](https://www.greptime.com/blogs/2026-05-09-prometheus-3-and-greptimedb)).
- **Remote Write 2.0** carries metadata, exemplars, and native histograms to long-term
  stores; UTF-8 names mean OTel's dotted names survive translation; newer 3.x added
  stable XOR2 chunk encoding and OpenMetrics 2.0 scraping
  ([3.15 changes](https://www.mertozen.com/en/blog/prometheus-promql-rate-histogram-quantile-recording-rule)).
- **LTS releases exist now** (3.5.x, 3.13) with defined support windows — the project
  is being run like infrastructure, which is what it is
  ([Prometheus versions and LTS](https://www.devopsandinfra.com/tool/prometheus/)).

Known limits, unchanged: single-node storage (HA = run two and dedupe downstream),
no built-in auth (put it behind a proxy/mesh), and RAM hunger at high cardinality —
which is the entire reason the next entry exists.

**Should we use it.** Yes — as the instrumentation contract everywhere (exporters,
PromQL, alert rules), as the actual server for standalone/NixOS-side monitoring, and
as the scrape agent even where VictoriaMetrics is the store. No — as the long-term
in-cluster store, where Cozystack's VictoriaMetrics is already better for us.

### VictoriaMetrics — **not CNCF** (independent, Apache-2.0 core) · verdict: **adopt (already adopted via Cozystack)**

**What it is.** A time-series database built to be a faster, cheaper Prometheus:
accepts Prometheus scrape configs (via vmagent), remote write, and OTLP; answers
MetricsQL, a PromQL superset; runs as a single binary or a cluster
(vminsert/vmselect/vmstorage). It is *not* a CNCF project — it's a company-led open
source project (VictoriaMetrics Inc.) — and Drew asked for it anyway because it is
the obvious comparison. Cozystack's monitoring is built on it
([Cozystack monitoring](https://deepwiki.com/cozystack/cozystack/8-development-and-deployment)).

**Current state / how it compares.** Independent measurements on identical small
hosts found VictoriaMetrics using roughly **2.2–2.6× less RAM** than Prometheus at
~30k active series, with the gap widening under churn because Prometheus's head block
retains churned series longer
([measured small-host comparison](https://pardha.dev/posts/prometheus-vs-victoriametrics-lab/)).
At large scale the gap is bigger: a petabyte-class comparison measured ~0.4–0.9
bytes/sample stored vs Prometheus/Thanos's ~2, and order-of-magnitude faster
long-range queries
([VM vs Prometheus vs Thanos at scale](https://dev.to/grfdsasdvef/victoriametrics-vs-prometheus-vs-thanos-petabyte-scale-metrics-storage-query-tco-222e))
— treat the exact figures as workload-dependent, but the direction is consistent
across every comparison we found, including neutral ones.

The honest caveats, because they are real:

- **MetricsQL is not PromQL.** It is a superset with subtly different semantics in
  places: one drill found the same counter returning `increase()` of 543.27 on
  Prometheus/Thanos/Mimir but 550 on VictoriaMetrics, because MetricsQL doesn't
  extrapolate the same way
  ([long-term storage revisit](https://revisit.tech/blog/prometheus-long-term-storage-revisit-thanos-mimir-and-victoriametrics/)).
  Dashboards mostly port; alert math should be re-checked, not assumed identical.
- **Some features are enterprise-tier** — notably downsampling in the community
  edition's absence; the cluster version is Apache-2.0 but the company sells an
  enterprise line, so the open-core gravity exists here too
  ([licence/feature split](https://revisit.tech/blog/prometheus-long-term-storage-revisit-thanos-mimir-and-victoriametrics/)).
- Remote Write 2.0 interop is still rough — in the same drill, Prometheus 3.15
  sending RW 2.0 succeeded against Mimir and **failed against VictoriaMetrics
  1.153**. RW 1.0 is fine; just don't assume the new protocol works everywhere yet.

**Should we use it.** We already do, via Cozystack, and this research says that's the
right call for in-cluster metrics: at our scale either would work, but VM leaves RAM
and disk headroom that a performance-sensitive small team will feel. Keep Prometheus
semantics knowledge current — it's the portable skill; VM is the storage detail.

### Long-term Prometheus storage: Thanos vs Cortex vs Mimir

Only relevant if we run Prometheus servers at a scale/lifetime where local TSDB
isn't enough *outside* the Cozystack/VictoriaMetrics world. Summary of a fast-moving
three-way:

- **Thanos** — CNCF **incubating**. Sidecars bolt onto existing Prometheus servers,
  ship TSDB blocks to object storage, and a querier fans out across sidecars + store
  gateways for a global PromQL view. Additive: Prometheus stays exactly as it is.
  Current (0.42.x, mid-2026). The default choice for "many existing Prometheus
  servers, want retention + global query, no re-architecture"
  ([Thanos vs Cortex vs Mimir](https://nodique.com/comparisons/thanos-vs-cortex-vs-mimir)).
- **Cortex** — CNCF **incubating**. The original push-based multi-tenant backend:
  Prometheus remote-writes to distributor/ingester/querier components. Historically
  important, currently hard to recommend for anything new: Grafana Labs forked it in
  2022 to create Mimir, taking ~87% of the commit activity with it, and Cortex's
  release cadence has slowed accordingly — it continues (1.21.x in 2026) but as a
  legacy line
  ([Cortex state](https://revisit.tech/blog/prometheus-long-term-storage-revisit-thanos-mimir-and-victoriametrics/)).
  CNCF status says "incubating"; reality says "maintained, superseded."
- **Grafana Mimir** — **not CNCF** (Grafana Labs, AGPL-3.0). Cortex's architecture
  continued with full-time backing: horizontally scalable, multi-tenant, ingests
  Prometheus remote write and OTLP metrics natively. Mimir 3.0 (Nov 2025) made the
  Kafka-based ingest-storage architecture the default and its new query engine
  standard — more capable, and noticeably heavier to run than the Mimir 2 mental
  model; a small team should notice that the "simple scalable metrics backend" now
  wants a Kafka-compatible queue in the write path
  ([Mimir 3 changes](https://kb.ekarisky.com/observability/lgtm/)).

**Should we use any of them.** No, not on the current plan: VictoriaMetrics covers
the in-cluster case and standalone Prometheus covers the rest. If a future
multi-cluster design needs a global metrics view outside Cozystack, Thanos is the
least-regret bolt-on; Mimir only if multi-tenancy with hard per-tenant limits
becomes a real requirement.

---

## Dashboards and the Grafana stack

### Grafana — **not CNCF** (Grafana Labs, AGPL-3.0) · verdict: **adopt (default UI)**

**What it is.** The dashboard and alerting front-end for essentially every
self-hosted observability stack: datasources for Prometheus/VictoriaMetrics, Loki,
Tempo, Jaeger, SQL databases, and dozens more; dashboards as JSON, provisionable
from Git; unified alerting across sources. It stores no telemetry itself.

**Current state.** Still the unchallenged default (Grafana 13.x as of late 2026).
What changed is the *posture around it*, covered in headline answer 2 above:
open-core vendor, AGPL core, Cloud/Enterprise gravity, community taking over the
Helm packaging. The OSS build remains fully functional for dashboarding and
alerting, and Cozystack ships it as the visualisation layer over VictoriaMetrics
and VictoriaLogs
([Cozystack monitoring](https://deepwiki.com/cozystack/cozystack/8-development-and-deployment)).

**Should we use it.** Yes. The realistic risk isn't Grafana disappearing; it's
slow divergence between the OSS and commercial experiences. Mitigation is cheap
and we should do it anyway: keep dashboards in Git (provisioned, not hand-clicked),
which also makes a future Perses migration a translation job instead of an
archaeology project.

### Grafana Alloy — **not CNCF** (Grafana Labs, Apache-2.0) · verdict: **optional — prefer upstream OTel Collector**

**What it is.** Grafana's distribution of the OpenTelemetry Collector, extended
with native Prometheus scrape/remote-write pipelines and first-class pipes into
Loki/Tempo/Mimir/Pyroscope. It replaced Grafana Agent, which reached end of support
in December 2025
([Alloy's role in the LGTM stack](https://www.ramguthula.com/articles/sre-10-build-your-own-lgtm-stack.html);
[Agent end of support](https://kb.ekarisky.com/observability/lgtm/)).

**Should we use it.** Only if we were all-in on Grafana backends. We aren't —
Cozystack runs the upstream OTel Collector per tenant, and upstream keeps us
vendor-neutral with the same pipelines. Alloy is a fine tool; adopting a vendor's
collector distribution buys integration polish we'd mostly not use and gives up
the neutrality that is OTel's whole point.

### Loki, Tempo, Pyroscope — **not CNCF** (Grafana Labs, AGPL-3.0) · verdict: **situational**

The three Grafana backends, briefly, since they arrive as a set:

- **Loki** (logs): indexes labels, not log content; chunks go to object storage.
  Cheap to run, deliberately limited query power (LogQL). Fine product; for us it's
  the fallback if we ever leave Cozystack's VictoriaLogs, not a target.
- **Tempo** (traces): object-storage-only trace backend with TraceQL, queried
  through Grafana. Genuinely good and the cheapest way to keep lots of traces;
  Tempo 3.x (2026) moved to a Kafka-decoupled write path in microservices mode,
  same heaviness trend as Mimir 3
  ([LGTM versions and changes](https://kb.ekarisky.com/observability/lgtm/)).
  Compared head-to-head with Jaeger below.
- **Pyroscope** (profiles): continuous profiling backend (the fourth signal's
  current practical home while OTel Profiles is alpha — see OpenTelemetry).
  Worth knowing exists; not a day-one need.

**Should we use them.** Tempo is the only one with a live decision attached (traces,
next section). Loki/Pyroscope: no independent adoption; if a Grafana-shaped stack
ever replaces Cozystack's, they come with it.

### Perses — CNCF **sandbox** · verdict: **watch closely; pilot for new dashboards-as-code work**

**What it is.** The CNCF's dashboard project: an open dashboard *specification*
plus a tool that renders it. Born at Amadeus, driven now by Red Hat, Amadeus, and
Prometheus-community contributors. Dashboards are schema-validated data (CUE),
written via SDKs (Go/CUE) or a UI, applied with a CLI (`percli`), statically
validated in CI, and migratable from Grafana JSON on a best-effort basis. Datasources
cover Prometheus, Tempo, Loki, Pyroscope, and a growing plugin set (Jaeger added in
0.54). Apache-2.0 throughout. It has no storage and no alerting engine — it
displays; Prometheus/Alertmanager still alert
([Perses overview](https://www.mrvitej.in/devops/cncf-perses/);
[Perses 0.54](http://stackgems.com/gems/perses)).

**Current state.** Sandbox since August 2024; 0.55 in October 2026 — still 0.x,
data model still moving (0.55 broke several things). The strongest adoption signal:
kube-prometheus (the Prometheus community's own Kubernetes stack) now ships a
Perses addon that replaces Grafana, importing community-mixin dashboards as Perses
CRs via a Perses operator
([kube-prometheus Perses addon](https://prometheus-operator.dev/kube-prometheus/kube/perses/)).
Red Hat embedding it in OpenShift's console ecosystem points the same direction:
Perses is being built as embeddable infrastructure, not just an app.

**Should we use it.** Not as our Grafana replacement today — plugin coverage,
polish, and the 0.x model churn are real costs, and Grafana is already in the
platform. But Perses is the most "us"-shaped project in this whole doc:
declarative, validatable, Nix/GitOps-friendly dashboard definitions under an
Apache licence with neutral governance. Recommended posture: any *new* dashboard
estate we build by hand (fleet/id/atlas/mcpx service dashboards) is a legitimate
Perses pilot; if it proves out, it becomes the long-term hedge against Grafana's
open-core drift.

---

## The all-in-one attempt

### SigNoz — **not CNCF** (SigNoz Inc., MIT core + commercial `ee/`) · verdict: **credible alternative stack; not our default**

**What it is.** An OpenTelemetry-native observability platform: a SigNoz collector
distribution ingests OTLP, everything (metrics, logs, traces, exceptions) lands in
**ClickHouse**, and one UI serves APM service pages, dashboards, alerts, log
exploration, and trace views. It is the project that most deliberately sets out to
be "the open-source Datadog," including a Datadog dashboard migration tool
([SigNoz as Datadog alternative](https://signoz.io/blog/open-source-datadog-alternative/)).

**Current state.** Moving fast and still pre-1.0: v0.147 shipped October 2026 on a
roughly weekly release cadence, with recent additions like LLM/AI observability on
OTel GenAI conventions and an MCP server. The licence shape is open-core: MIT for
the core, a separate commercial licence for the `ee/` directory (fine-grained
access control and other enterprise features), and the company sells SigNoz Cloud
plus enterprise self-hosting on usage-based pricing
([SigNoz facts](https://kb.ekarisky.com/observability/signoz/);
[licence and version check](https://www.opensourcehunter.com/datadog-alternatives/)).
Funding is modest (~$6.5M raised, last round 2023), which cuts both ways: no
giant-vendor gravity well, and no giant-vendor runway either.

**Honest evaluation.** The architecture is sound — ClickHouse is a genuinely good
single store for high-cardinality telemetry, and OTel-only ingestion is the right
bet (it means adopting SigNoz never traps your instrumentation). The costs are
operational and structural: self-hosting means running ClickHouse (plus its
keeper/coordination) well; pre-1.0 weekly releases mean upgrade discipline; and
the single-product convenience is exactly the coupling the assembled stack avoids —
if SigNoz-the-company pivots, you keep your OTel instrumentation but lose your UI,
alerts, and dashboards in one move.

**Should we use it.** Not as the platform default — Cozystack's stack is already
assembled and shipped, and swapping it for SigNoz would trade four boring parts
for one exciting one. It *is* the right answer to keep in the drawer for a
greenfield, single-team deployment that wants one UI and can accept ClickHouse
operations — e.g., if atlas or mcpx ever needs a standalone observability box
independent of the platform. Re-evaluate at its 1.0.

---

## Instrumentation and tracing

### OpenTelemetry — CNCF **graduated** (May 2026) · verdict: **adopt unconditionally — this is the standard**

**What it is.** The vendor-neutral instrumentation layer: APIs and SDKs per
language, the OTLP wire protocol, semantic conventions for attribute/metric names,
and the Collector — a pipeline binary (receivers → processors → exporters) that
receives, transforms, samples, and fans out telemetry to any backend. It is not a
backend and stores nothing.

**Current state.** The headline most people have wrong: **OpenTelemetry graduated
from the CNCF in May 2026**, alongside Kubernetes and Prometheus at the top tier,
with 12,000+ contributors from 2,800+ companies and the second-highest velocity in
the CNCF after Kubernetes itself
([OTel graduation](https://www.cncf.io/blog/2026/08/31/opentelemetry-has-graduated-now-what-2/)).
Signal by signal:

- **Traces, metrics, logs: stable** at API, SDK, and wire level (OTLP 1.11).
  Semantic conventions are maturing area by area — HTTP stable since 2023,
  database conventions stabilised in 2025, Kubernetes resource attributes stable
  in semconv 1.42 (June 2026); messaging/RPC and GenAI conventions still in
  development
  ([OTel status reference](https://kb.ekarisky.com/observability/opentelemetry/reference/)).
- **Profiles: public alpha** since 26 March 2026 — an OTLP profiles format ~40%
  smaller on the wire than the earlier draft, Collector support from v0.148, and
  the eBPF profiler (the agent Elastic pledged) as an official component. Explicitly
  "not for critical production workloads" yet
  ([profiles alpha and collector state](https://www.ajayk.sh/blog/opentelemetry-ebpf-observability-cost-2026/)).
- **The Collector is the asterisk.** Graduation is a project-level judgement; the
  Collector binary is still versioned v0.x (v0.162 in autumn 2026) with per-component
  maturity labelled "mixed" — core pipelines are stable, the contrib long tail
  varies. Pin versions, read component READMEs, don't treat "the Collector" as one
  maturity level
  ([collector versioning reality](https://www.ajayk.sh/blog/opentelemetry-ebpf-observability-cost-2026/)).
- **eBPF auto-instrumentation (OBI)** — Grafana's Beyla, donated to OTel in 2025 —
  is at v0.14 and officially "in Development": promising, not yet a 1.0. This, not
  Pixie, is where zero-instrumentation is consolidating (see Pixie).

**Should we use it.** Yes, everywhere, as the only instrumentation standard for
fleet/id/atlas/mcpx code. It's the one decision in this doc with no serious
counter-argument: instrument once with OTel and every backend choice in this
document stays reversible.

### OpenTracing — CNCF **archived** · verdict: **do not use; migrate off if encountered**

**What it is / was.** The 2016 vendor-neutral tracing API. It merged with
OpenCensus in 2019 to form OpenTelemetry, was archived by the CNCF in January
2022, and in March 2026 the OTel specification formally deprecated its
OpenTracing-compatibility requirements (removal no earlier than March 2027);
existing shims may linger but no new work should build on them
([OpenTracing deprecation](https://opentelemetry.io/blog/2026/deprecating-opentracing-compatibility/);
[archival dates](https://www.cncf.io/blog/2026/08/31/opentelemetry-has-graduated-now-what-2/)).
Mentioned here only so nobody has to wonder: if a dependency still exposes
OpenTracing, treat it as legacy and route around it via the OTel bridge.

### Jaeger vs "Spiffy" — a disambiguation first

Drew's question paired "Spiffy" with Jaeger. **SPIFFE** (which "Spiffy" almost
certainly means) is a workload-identity framework — a different layer entirely,
covered by the security lane, not this one. **Jaeger** is the tracing backend, and
it's covered properly here.

### Jaeger — CNCF **graduated** · verdict: **adopt-able default trace backend; pick vs Tempo by UI gravity**

**What it is.** The distributed-tracing backend open-sourced by Uber in 2016: store
spans, query them, render the waterfall. One of the CNCF's earliest graduations.

**Current state.** Jaeger v2 (Nov 2024) rebuilt the whole system **on the
OpenTelemetry Collector framework**: one binary configured with Collector YAML,
native OTLP receivers on 4317/4318, Jaeger's own storage/query components as
Collector extensions, and backends including Elasticsearch/OpenSearch, Cassandra,
Badger (single-node), and experimental ClickHouse. The agent and Jaeger's own
client libraries are gone/archived — instrumentation is OTel SDKs now. **Jaeger v1
reached end-of-life on 31 December 2025**; v2 is at 2.23 (November 2026) on a
roughly monthly cadence
([Jaeger release/EOL state](http://eosl.date/eol/product/jaeger/);
[Jaeger v2 architecture](https://www.k8s.guide/ecosystem/jaeger/)).

**Jaeger or Tempo?** The real decision. Tempo wins on storage economics (object
storage only, no database to run) and on TraceQL's ability to answer structural
queries ("slow DB calls under /pay") in one shot; Jaeger wins on having its own
purpose-built UI, Apache-2.0 licensing, neutral governance, and not requiring
Grafana. A November 2026 hands-on comparison found both correct on retrieval,
with Tempo's query model clearly stronger for aggregate questions
([Jaeger vs Tempo hands-on](https://techloghub.com/compare/jaeger-vs-tempo)).

**Should we use it.** If traces live inside the Grafana-centric Cozystack world,
follow the platform (VictoriaTraces/Tempo-shaped options) and don't add Jaeger as
a second UI. If we stand up tracing for our own services outside that gravity —
atlas/mcpx debugging across the fleet — Jaeger v2 all-in-one is the lowest-friction
correct choice: one binary, OTLP in, done. Either way the instrumentation is OTel,
so this decision is cheap to reverse, which is exactly how it should be.

---

## eBPF visibility, caches, and AI help

### Pixie — CNCF **sandbox** (since June 2021) · verdict: **do not adopt; the idea won, the project didn't**

(And yes — "Percy" in Drew's question is almost certainly Pixie. There is no
observability project called Percy.)

**What it is.** New Relic's eBPF auto-instrumentation platform for Kubernetes:
deploy it and, with zero code changes, you get full-body HTTP/gRPC request capture,
database query visibility (Postgres, MySQL, Redis, Kafka, and more), network and
resource metrics, and always-on CPU profiles — queried live with its PxL scripting
language. Its distinguishing design choice: **data stays in the cluster** (in-memory,
short retention — it's a live debugger, not a warehouse; long-term storage means
exporting via its OTel plugin). Documented cost is under 5% of cluster CPU,
typically under 2%
([Pixie overview](https://devtoollab.com/blog/ebpf-observability-tools);
[Pixie security self-assessment](https://tag-security.cncf.io/community/assessments/projects/pixie/self-assessment/)).

**Current state — why it stalled.** Pixie has sat at sandbox since June 2021 —
five-plus years without progressing. The CNCF's own project page shows a health
score of **32**, total contributors **down 40% year over year**, and recent activity
arriving as component pre-releases rather than unified versions; the fairest
summary in the current literature is "maintained, not accelerating"
([Pixie CNCF project page](https://www.cncf.io/projects/pixie/);
[eBPF tools comparison](https://devtoollab.com/blog/ebpf-observability-tools)).
The strategic story matters more than the numbers: New Relic donated Pixie and
then the industry's zero-instrumentation energy consolidated elsewhere — Grafana
donated Beyla to OpenTelemetry (now OBI), backed by Grafana, Splunk, Coralogix,
and Odigos contributors. Pixie is a standalone platform with its own query
language and data model; OBI is a component that emits plain OTel into whatever
stack you already have. The component ate the platform.

**Should we use it.** No. If we want eBPF-derived visibility, the path is OTel's
OBI when it reaches 1.0 (and Cilium/Tetragon-class tooling from the network/security
lanes in the meantime). Pixie remains genuinely impressive technology and a fine
ad-hoc debugging install in a throwaway cluster — it is not a foundation to build
the estate's observability on.

### Trickster — CNCF **sandbox** (since March 2021) · verdict: **niche tool, solves a problem we don't have**

**What it is.** A reverse-proxy cache for time-series query APIs, built at Comcast
to make Grafana dashboards stop hammering Prometheus. Its trick is **Delta Proxy
Caching**: it inspects a dashboard query's time range, serves the already-cached
portion, and asks the backend only for the missing delta — plus step-boundary
normalisation and a "fast forward" for the most recent point. It fronts Prometheus,
InfluxDB, ClickHouse, and others, with memory/bbolt/Redis/filesystem cache backends
([Trickster overview](http://rawkode.academy/technology/trickster);
[Trickster CNCF page](https://www.cncf.io/projects/trickster/)).

**Does it replace Datadog?** No — it doesn't collect, store, or alert on anything.
It makes *dashboards over an existing TSDB* load faster when many viewers load the
same heavy panels. It is a cache, full stop.

**Should we use it.** Only if we ever run a Prometheus/VictoriaMetrics that is
visibly suffering under dashboard load — at our scale, unlikely, and both
VictoriaMetrics' query performance and Grafana's own caching attack the same
problem earlier in the chain. File under "know it exists."

### HolmesGPT — CNCF **sandbox** (since October 2025) · verdict: **worth a bounded trial; not infrastructure yet**

**What it is.** Robusta's open-source AI SRE agent: point it at an alert or ask it
a question, and an agentic loop queries live observability sources (Prometheus,
Kubernetes, logs, cloud APIs, 50+ integrations), correlates changes, and produces
a root-cause analysis with a suggested fix. It is read-only by design and respects
RBAC. A newer "operator mode" runs it continuously, watching for problems and
reporting to Slack. It works with any LLM provider (OpenAI, Anthropic, Bedrock,
Gemini, local models) and does not require Kubernetes
([HolmesGPT repository](https://github.com/robusta-dev/holmesgpt)).

**Current state.** Very new to the CNCF (sandbox, October 2025) and moving quickly
(0.24.x releases). The material event: **Freshworks acquired Robusta in October
2026**, folding the team and technology toward Freshservice/FireHydrant
([Robusta joins Freshworks](https://home.robusta.dev/blog/robusta-joins-freshworks);
[acquisition coverage](https://techgig.com/news/ai/freshworks-acquires-robusta-dev-for-ai-powered-it-outage-investigation/135339126)).
The project remains Apache-2.0 in the CNCF, and the founders say little changes
today — but an acquired sandbox project's centre of gravity reliably drifts
toward the acquirer's product over time. Price that in.

**Should we use it.** Worth looking at — genuinely. For a small team, an agent
that does the first 15 minutes of alert triage (gather the graphs, the recent
deploys, the obvious correlation) attacks a real cost, and its read-only posture
makes the trial safe. Our shape of trial: run it against our own stack with our
own model routes (we have the fleet's model infrastructure; HolmesGPT is
provider-agnostic), on real alerts, scored against what the human found. What it
is not: a reason to change anything about the observability stack underneath —
it consumes Prometheus/Grafana-class sources, which we already have.

---

## Operations-adjacent projects

### OpenCost — CNCF **incubating** · verdict: **adopt when multi-tenancy makes "who spent what" a real question**

**What it is.** A vendor-neutral specification and implementation for Kubernetes
cost allocation: it joins live resource usage (Kubernetes API + Prometheus) with
cloud pricing (or a custom price list) and attributes cost to namespace,
deployment, pod, container, and labels. Born at Kubecost, in the CNCF since 2022,
incubating since October 2024, aligned with the FinOps Foundation's model
([OpenCost CNCF page](https://www.cncf.io/projects/opencost/);
[incubation announcement](https://opencost.io/blog/cncf-incubation/)).
It is a *data layer*: it computes and exposes numbers (API + Prometheus metrics);
recommendations, budgets, and optimisation are left to you or to Kubecost on top.
Recent work even extends it to LLM inference cost per token via vLLM metrics
([OpenCost 1.121 inference costing](https://www.cncf.io/blog/2026/08/05/opencost-1-121-0-first-of-a-kind-kubernetes-inference-cost-tracking/)).

**Should we use it.** On cloud Kubernetes, yes almost by default — it's the
standard answer and it feeds our existing Prometheus/Grafana. On our bare-metal
Talos/Cozystack estate the value is different but real: with a custom price list
(hardware amortisation + power), OpenCost answers "what does each tenant/project
cost to run" in the same units finance thinks in, which matters once atlas/id/
mcpx tenants share clusters. Adopt it when the platform goes multi-tenant in
earnest; before that it's a dashboard nobody's decisions depend on. Note the AKS
cost add-on is literally built on OpenCost — this is the mainstream choice, not
a bet.

### OpenFeature + flagd — CNCF **incubating** · verdict: **adopt the API in our code; flagd is the right-sized backend**

**What it is.** A standard *evaluation API* for feature flags: application code
calls `getBooleanValue("flag", default, context)` through an SDK, and a pluggable
**provider** behind it talks to whatever actually holds the flags — a vendor
(LaunchDarkly, Flagsmith, Unleash…), an in-house system, or **flagd**, the
project's own reference daemon. It deliberately standardises only the asking, not
flag management: no UI, targeting model, or audit log comes with the standard
([what OpenFeature standardises](https://github.com/mchittineni/ultimate-platform-engineering-guide/blob/HEAD/progressive-delivery-and-feature-flags/what-is-openfeature-and-why-does-a-vendor-neutral-flag-api-matter.md)).
Spec v0.9.0 (August 2026) marked the evaluation/provider sections stable; OFREP
standardises remote evaluation; flag evaluations have OTel semantic conventions,
so flag state shows up in traces for free
([OpenFeature state](https://feature-flag.com/blog/openfeature-explained/)).

**flagd** is the piece that makes this concrete for us: a small Go daemon (or
in-process evaluator) reading flag definitions from JSON/YAML in Git, a
ConfigMap, or CRs via the OpenFeature Operator, hot-reloading on change. Flags
in Git, reviewed like code, no SaaS.

**vs "just using a flags library."** A library hard-codes one backend's API into
every service; the day the backend changes (or the vendor reprices, or we
self-host), every call site is a migration. OpenFeature moves that to one
provider swap. The cost is one indirection layer and an SDK dependency — cheap.
The analogy is exact: it's OTel's argument, applied to flags, and it won the same
way (even Datadog's own feature flags are built on OpenFeature now).

**Should we use it.** Yes for any service where flags are more than a config
value — mcpx and id especially, where gradual rollout and kill-switches are
operational tools. Use flagd with definitions in Git; skip vendor flag platforms
entirely unless experimentation/A-B analytics ever becomes a product need.

### Chaos Mesh — CNCF **incubating** · verdict: **later, in staging; a practice purchase, not a platform need**

**What it is.** Kubernetes-native chaos engineering from PingCAP (built to break
TiDB on purpose): faults declared as CRs — pod kills, network latency/loss,
clock skew, IO faults, JVM faults, cloud API failures — injected by a privileged
per-node daemon, orchestrated as experiments and cron-like schedules, with a
workflow engine for multi-step scenarios
([Chaos Mesh overview](https://cloudparts.dev/technologies/chaos-mesh/);
[Chaos Mesh CNCF page](https://presentations.cncf.io/projects/chaos-mesh/)).
Incubating since February 2022; actively releasing (2.8.x in late 2026); also
exists in a second flavour, LitmusChaos, likewise incubating — the CNCF hosts
both and picks no winner.

**The security footnote that matters.** The chaos daemon is privileged on every
node, so a compromised Chaos Mesh is a compromised cluster: in 2025 JFrog found
four CVEs (three rated 9.8, "Chaotic Deputy") letting anyone with in-cluster
network access execute commands through its GraphQL endpoint; fixed in 2.7.3
([Chaos Mesh CVE history](https://cloudparts.dev/technologies/chaos-mesh/)).
This is inherent to the category, not a scandal — but it dictates deployment
shape: staging clusters, network-restricted, never ambient in production.

**Should we use it.** Not yet, and maybe not for a while. Chaos tooling pays off
when a system is already reliable enough that *unknown* failure modes are the
risk — our platform is at the stage where known failure modes (backups, upgrades,
node loss) still need basic game-day discipline, which a calendar and a script
handle. When Cozystack tenants start depending on our SLOs, Chaos Mesh in a
staging cluster is the right tool and it's the category standard. Buying it
earlier would be theatre.

### Fluent Bit — CNCF **graduated** (under the Fluentd umbrella) · verdict: **adopt — the default log shipper**

**What it is.** A small C binary that collects, parses, filters, and forwards
logs (and now metrics/traces) from anything to anything — 70+ plugins, SQL stream
processing, native OTLP support, tiny footprint. It is the Fluentd ecosystem's
edge agent (Fluentd itself is graduated; Fluent Bit rides under that umbrella).
Adoption is enormous — billions of downloads, deployed millions of times daily —
and the CNCF's Q3 2026 Technology Radar found it the *most recommended*
observability tool surveyed, treated by users as simply "the default choice for
log collection"
([CNCF Tech Radar Q3 2026](https://www.cncf.io/wp-content/uploads/2026/11/CNCF-Tech-Radar-survey-W5-261020-3.pdf);
[Fluent Bit releases](https://endoflife.date/fluent-bit)).

**Current state.** Fluent Bit 5.2 (November 2026), fast release cadence, no drama.
Cozystack already ships it for log collection into VictoriaLogs.

**Should we use it.** Yes — it's already in the platform, and it's also the right
answer on plain NixOS hosts for getting logs into whatever backend. This is the
least interesting verdict in the document because the industry already reached it.

### Kepler — CNCF **sandbox** · verdict: **optional garnish; interesting on bare metal, skip for now**

**What it is.** The Kubernetes-based Efficient Power Level Exporter: a Prometheus
exporter that reads hardware energy counters (RAPL on bare metal, plus hwmon,
Redfish, GPU/DCGM paths) and attributes node energy to pods and namespaces, so
"watts per workload" becomes a metric next to all the others
([Kepler CNCF page](https://www.cncf.io/projects/kepler/)).

**Current state.** Sandbox since May 2023, still releasing (v0.13, November 2026).
One correction to the common description: **current Kepler does not require
eBPF** — the eBPF/model-server design is legacy (≤0.9); today's architecture is
procfs/sysfs CPU-time attribution over measured node power. Which also means the
honest caveat printed in its own community's material: per-pod watts are
*attributed estimates*, not measurements — only the node total is metered
([Kepler v0.13 state](https://github.com/depeche-io/kcd-2026/blob/HEAD/feel-the-breeze/slides/slides.md)).

**Should we use it.** Our estate is unusually Kepler-shaped — bare metal, owned
hardware, power is a real cost line — and it would feed OpenCost a real power
price. But estimates-per-pod is a weak foundation for decisions, and nothing we
currently decide needs energy attribution. Revisit alongside OpenCost if
per-tenant costing becomes real; the node-level power numbers alone (via RAPL →
Prometheus) are a one-exporter job if curiosity strikes sooner.

---

## What contradicts the obvious consensus

Five findings from this pass that cut against what "everyone knows":

1. **OpenTelemetry already graduated** (May 2026). Most practitioners still
   describe it as incubating and hedge accordingly. The hedge is out of date —
   but its mirror image is also true: the *Collector binary is still v0.x* with
   per-component maturity. "Graduated project, unfinished distribution" is the
   accurate one-liner, and both halves matter for planning.

2. **The eBPF auto-instrumentation war is over and Pixie lost it** — not on
   technology (its capture is still the richest) but on shape: the industry chose
   a *component* that emits OTel (OBI, ex-Beyla) over a *platform* with its own
   query language and data silo. Sandbox since 2021, health score 32,
   contributors down 40% YoY. "CNCF project" on the tin concealed a stalled
   project; status labels lag reality by years, in both directions (see also
   Cortex: incubating, and largely superseded by its own fork).

3. **Prometheus's strongest moat was never performance** — VictoriaMetrics beats
   it on RAM, disk, and long-range query speed in every neutral measurement we
   found, and it *still* isn't a clean replacement, because MetricsQL's subtle
   semantic differences (e.g. `increase()` extrapolation) make "drop-in" a claim
   to verify per-dashboard, not assume. Ecosystem gravity and semantic exactness,
   not benchmarks, are what keep Prometheus central. Relatedly, Prometheus 3.x's
   built-in OTLP receiver and stable native histograms quietly invalidated the
   standard 2.x-era advice that you need a collector tier and a separate
   long-term store from day one — at small scale, one Prometheus is again a
   complete answer.

4. **The "simple open-source Grafana stack" got heavier while nobody watched.**
   Mimir 3's default write path wants Kafka; Tempo 3's microservices mode is
   Kafka-decoupled too; Grafana Agent is dead; the Helm charts moved to a
   community org. None of this is bad engineering — but the LGTM stack's
   operational weight in 2026 is much closer to "run a small data platform" than
   its laptop-demo reputation suggests. Our platform (Cozystack) having already
   chosen the VictoriaMetrics family — single-purpose binaries, no Kafka in the
   metrics path — looks better in this light than it did two years ago.

5. **The "open-source Datadog" is two companies' open-core products wearing a
   trench coat.** SigNoz (MIT core + commercial `ee/`) and the Grafana stack
   (AGPL + Cloud/Enterprise) are both vendor funnels; the genuinely neutral
   assembled stack (OTel + Prometheus/VM + Jaeger + Grafana-or-Perses) is the
   only version where no single company's pricing decision can reach into our
   infrastructure. That's not an argument against either product — we recommend
   Grafana ourselves — but "open source" in this category now means "open core
   with a sales motion," and the estate should hold its dashboards-as-code and
   OTel-instrumentation discipline precisely so that stays a tolerable fact
   rather than a lock-in.

---

## Bottom line for the estate

Instrument everything with **OpenTelemetry**. Let **Cozystack's stack stand**
(VictoriaMetrics + VictoriaLogs + Grafana + Fluent Bit) rather than relitigating
it; run **Prometheus 3.x** where we need standalone monitoring outside Kubernetes.
Keep **Grafana** as the UI while holding dashboards in Git and piloting **Perses**
for new hand-built estates. Pick **Jaeger v2 or Tempo** for traces by UI gravity,
knowing the choice is reversible. Add **OpenCost** when multi-tenancy makes cost
allocation a decision input, **OpenFeature + flagd** in services now, and give
**HolmesGPT** a bounded, read-only trial against real alerts. Skip **Pixie**,
**Trickster**, and **Chaos Mesh** for now — one stalled, one niche, one premature —
and treat **SigNoz** as the standing alternative if a future greenfield deployment
wants one product instead of a stack.
