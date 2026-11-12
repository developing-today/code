# Design: Atlas secret-tier embedding — one model, local execution, no-egress

- **Date:** 2026-11-08
- **Issue:** [#6059](https://github.com/developing-today/code/issues/6059)
- **Parent:** [#6019](https://github.com/developing-today/code/issues/6019),
  semi-automated ingestion operations.
- **Related decisions and work:** [#6060](https://github.com/developing-today/code/issues/6060)
  (closed by merged PR #7649; one-model mandate), [#6046](https://github.com/developing-today/code/issues/6046)
  (open; golden-set builder), [#6021](https://github.com/developing-today/code/issues/6021)
  (classification revalidation), [#6027](https://github.com/developing-today/code/issues/6027)
  (open; query traces), [#5351](https://github.com/developing-today/code/issues/5351)
  (closed; Gate A parity).
- **Status:** design only. This document does not change the embedder,
  ingestion, classification, or deployment code.
- **Scope:** decide how secret-tier chunks use the mandated local model,
  specify the no-egress and failure invariants, and define the hardware
  evaluation and operational shape required before enabling ingestion.

## 1. Requirement in one paragraph

Secret-tier text is embedded only by the locally executed, version-pinned
Atlas model and pipeline. Neither routine inference nor any failure path may
send the text, tokens, or a retry to a hosted embedding API. If the local
model, runtime, host, or storage boundary is unavailable or cannot be
verified, the batch remains unpromoted and is queued or failed explicitly;
it is never rerouted to a different provider or model. The secret tier uses
the same model identity as every other Atlas tier, as required by the
completed #6060 decision. Before activation, a local benchmark must show
that the selected batch shape fits the real burst workload and resource
budget, and the #6046 evaluation must show acceptable retrieval quality
without exposing secret examples outside the fleet.

## 2. Decision consumed: one model for all tiers

The model choice is settled by the decision record
[`2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability`](../2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability/2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability.md):
every tier uses `snowflake-arctic-embed-m-v2.0` through the same
`atlas-embed` pipeline. PR #7649 merged that decision and closed #6060 while
this design task was being claimed. Therefore:

1. **No distinct secret-tier model is selected.** Secret-tier execution is
   local execution of the existing model, not a model fork. A different
   model requires amending #6060 with contrary evaluation evidence and
   treating the change as a corpus-wide migration; this design does not
   authorize it.
2. The local model and pipeline identity is the full tuple of pinned weights,
   tokenizer, ONNX graph/runtime behavior, pooling, normalization, and output
   width—not just a display name. The in-repo contract is the fp32 ONNX
   `Snowflake/snowflake-arctic-embed-m-v2.0` snapshot, SentencePiece
   tokenizer, no truncation through 8192 tokens, CLS pooling, 768 dimensions,
   and f32 L2 normalization (`pkgs/atlas-search/crates/atlas-embed/src/lib.rs:11-39,48-57`).
   Gate A verifies bit-parity with the Python corpus pipeline (#5351).
3. #6046 remains the quality gate, not permission to pick a per-tier model.
   When its versioned golden set and harness are available, run them against
   the mandated model and the intended secret-tier corpus shape. If quality
   is unacceptable, secret-tier retrieval stays disabled pending a decision
   amendment; do not substitute a hosted or tier-specific model.
4. The golden set is currently an open work item; no #6046 artifact was
   found in this repository at the time of this design. The existing
   `atlas-eval` runner is not itself a golden set, and the `#6039` provisional
   overlap fixture is not the #6046 corpus. No quality result is claimed here.

## 3. Security boundary and no-egress invariant

### 3.1 Inference invariant

For a chunk classified `secret-bearing` / `Tier::Secret`, the only allowed
inference path is the pinned local ONNX model running on an approved fleet
execution node. The raw text and token IDs must not be sent to a third-party
embedding provider, hosted model endpoint, public telemetry service, or
external fallback—on success, timeout, load failure, runtime error, overload,
or retry alike. There is no “best effort” alternate provider. Model
provisioning is a separate, controlled operation: it may fetch the pinned
public model artifact and ONNX Runtime before processing secret data, but
must not include corpus text, token IDs, or content-derived request metadata.
The inference worker runs from local artifacts and needs no model-provider
network access. Enforce that property in the service sandbox, not only by
convention: run the inference worker in a systemd network namespace
(`PrivateNetwork=yes`) with `RestrictAddressFamilies=AF_UNIX`, no network
capabilities, and no IP allow-list. The worker uses local files and Unix
sockets only. If a deployment needs IP networking for any reason, secret
ingestion stays disabled until an equivalent outbound-deny policy is
independently verified. Model provisioning runs as a separate unit with
network access but no mount or read permission for secret staging/corpus
paths; it installs and hash-verifies artifacts before the inference unit is
started.

The local output vector is also secret-tier data: it stays in the secret
staging/promotion path and secret vector store, retains the same tier
credentials and search ceiling, and is not copied to lower tiers or included
in ordinary logs. The comparability contract never weakens #6021's
classification boundary or the existing pre-merge tier filter.

### 3.2 Data handling requirements

- Process raw secret chunks only on a host inside the fleet boundary, from a
  fleet-local source/staging location. The staging foundation currently
  provisions a local libSQL/SQLite file; hosted Turso creation is named as a
  later deployment surface, not as a property of the current implementation
  (`pkgs/atlas-search/crates/atlas-core/src/stage.rs:21-26`). Do not put
  secret chunks in hosted staging as part of this design. If the future
  deployment changes that boundary, #6019/#6021 must explicitly approve and
  document it before secret ingestion is enabled.
- Do not log request bodies, text, token IDs, or content-bearing exception
  details. Emit only opaque unit/job IDs, counts, model identity, timings,
  and error classes. Disable core dumps for the worker; keep vectors and
  staging files under the secret tier's access and retention controls.
- Keep model fetch/provisioning out of the inference process. Pin the
  Hugging Face repository revision and artifact hashes recorded by the
  parity fixtures; verify them before deployment, then load from local
  paths. No runtime model download or “latest” resolution.
- A secret chunk may be submitted to an internal service only if that
  service is inside the fleet trust boundary and the caller is authorized
  for the secret tier. Prefer a local library/worker call over the generic
  HTTP embedding route.
- Verify the egress control as part of the shadow gate: from the worker's
  actual service namespace, attempts to open IPv4/IPv6 sockets, resolve a
  public hostname, and connect to an external canary must fail. Verify
  local file and Unix-socket operation still works. A host firewall rule
  alone is insufficient if the worker can be reconfigured to bypass it;
  if isolation cannot be demonstrated, do not enable secret jobs.

### 3.3 Existing boundary to preserve / close before using the HTTP route

The current `POST /v1/embed` route authenticates a client but explicitly
does not apply the client's tier ceiling (`pkgs/atlas-search/crates/atlas-search/src/server.rs:2055-2068`;
comment references #6788). That route currently executes the local model,
not a hosted embedding provider, but it is not by itself a secret-tier
authorization boundary. The secret ingestion path must therefore call the
local worker/library directly, or a separate implementation must add and
test secret-tier authorization before this endpoint is used for secret
text. This design does not change #6788's general-query contract. No raw
secret text is to be passed through a less-trusted client merely because
the inference compute is local.

## 4. Local embedder and ingestion contract

`atlas-embed` already has the needed local inference primitive. It loads a
caller-supplied model directory and dynamically loaded ONNX Runtime library;
it does not download either during `Embedder::load` (`src/lib.rs:122-172`).
It accepts arbitrary batch lengths, pads each batch to its longest input,
rejects empty or over-context entries, and returns a whole-batch error if
inference fails (`src/lib.rs:99-119,184-235`). Default session threading is
8 intra-op / 2 inter-op threads (`src/lib.rs:127-165`).

The ingestion worker should:

1. Resolve unit sensitivity and secret-tier authorization before reading
   text; reject an unknown or downgraded classification rather than
   defaulting to internal.
2. Read a bounded set of chunks from fleet-local secret staging, tokenize
   locally, and batch by similar token length to reduce padding waste. Use
   both a maximum item count and a maximum total-token budget; do not use
   one unbounded batch for an arbitrarily large unit. Inputs over 8192
   tokens fail loudly and return to chunking/triage, not silent truncation.
3. Reuse one resident model session per worker process. Do not load a model
   per request or per batch: the current core integration notes an
   approximately 1.6-GB-RSS loaded session and serializes its mutable ONNX
   session (`pkgs/atlas-search/crates/atlas-core/src/embed.rs:61-89`).
4. Write vectors and provenance to a private staging generation. Promote
   atomically only after every chunk succeeds, dimensions/model identity
   are checked, and the tier/classification is still valid. No partial
   generation becomes searchable.
5. On failure, leave the source unit in a retryable local state (or mark it
   failed with a structured error and operator-visible job ID). Retry only
   the same pinned local model on an approved host. If it remains
   unavailable, keep the unit unpromoted. Never route it to a hosted API,
   change model, silently drop chunks, or publish a partial index.

The end-to-end staging aggregator and merge-on-close promotion are not yet
implemented in the staging foundation (`stage.rs:21-26`). This design
specifies the contract for that follow-on; it does not imply that the full
secret ingest path already exists.

## 5. Fleet hardware evaluation

### 5.1 Current reference host and evidence limits

Use the current execution node, **amd-6**, as the first operational
candidate: the machine-onboarding standard identifies it as the laptop
running the fleet today (`doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1.md:29-33`).
The current session's live host report on 2026-11-08 identified hostname
`amd`, an AMD Ryzen 7 7840U (8 cores / 16 threads), 123 GiB visible RAM,
and Radeon 780M integrated graphics. The NVIDIA driver was unavailable
and `rocm-smi` was not installed; therefore the baseline is **CPU ONNX**
with no assumed GPU acceleration. This is an observed host snapshot, not a
fleet-wide inventory or a committed service guarantee. The basement-cluster
machines are not an alternative today: the status record says they have no
OS installed (`doc/2026-10-27T03-14-08Z_status_basement-cluster/`).

There are no recorded atlas-embed throughput or peak-batch RSS results.
`crates/atlas-embed/examples/timing.rs` instruments load, single/warm
inference, a long input, and batch 32, but labels itself a measurement
example rather than a benchmark suite. The approximately 1.6-GB RSS note
is a loaded-session estimate, not a measured batch peak. Model artifact byte
size is also not recorded. These are measurements to produce, not numbers
to infer from model dimensions.

### 5.2 Benchmark protocol

Run the benchmark locally on amd-6 with the exact pinned model, ONNX Runtime,
Nix/runtime build, thread settings, and power profile intended for service.
Record the machine, OS/kernel, runtime and model hashes, temperature/power
mode, and background load. Do not upload raw secret samples or benchmark
traces. Use synthetic inputs matching the approved secret chunk-length
distribution, or run representative secret data locally and export only
length/count aggregates. Never put secret text into a CI artifact.

Measure the following, separating cold start from steady state:

| Dimension | Cases / output |
|---|---|
| Batch shape | 1, 8, and 32 items (32 is already exercised by the timing example); test the observed p50/p95 token lengths plus a long-tail case and the 8192-token limit. Include both length-bucketed and mixed-length batches to quantify padding. |
| Workload | One typical delta and one p95-sized burst from #6019's ingest ledger, plus a 2× replay of the observed p95 burst. If the ledger has not produced these distributions, label the workload synthetic and do not enable automatic secret ingestion on that evidence alone. |
| Throughput | Chunks/s and tokens/s per batch and over the full burst; queue wait, batch latency p50/p95, drain time, and padding ratio. Separate tokenization and ONNX time where practical. |
| Memory | Process RSS before load, after model load, and peak during each batch/burst; include model-load peak and the largest accepted sequence/batch. Record system available memory and swap activity. |
| Stability | At least five repeated cold starts and warm runs, then a 30-minute burst/idle soak; record failures, retries, queue depth, and temperature. Do not use secret content in diagnostics. |

Select the largest batch/token budget that stays inside the measured memory
and latency envelope with at least 20% headroom; batch by token length rather
than increasing concurrency first. Begin with one resident worker and the
existing 8/2 intra/inter-op defaults. Only test a second session or other
thread settings if the single-session result misses the ledger-derived
target, since each loaded session carries a significant RSS cost.

### 5.3 Proposed activation gate

Before enabling the secret-tier worker, the benchmark report must show that
the chosen single-worker configuration sustains at least **2× the observed
95th-percentile delta arrival rate**, drains a 95th-percentile burst within
the ingest freshness target agreed under #6019, and remains below its
measured memory budget with 20% headroom. The run must complete without
hosted inference, raw-text logging, partial promotion, or tier-boundary
violations. If arrival data or the freshness target is not yet available,
record the gap and leave scheduled secret ingestion disabled; do not invent
a capacity number from the 1.6-GB session estimate.

The #6046 quality evaluation is a separate gate: run the versioned suite
with the mandated model and capture version/hash, ranking metrics, and
per-tier results. If the golden set contains secret-tier examples, execute
and retain that evaluation on the fleet host only; publish aggregate metrics
and non-sensitive identifiers, never the examples. Quality failure blocks
secret-tier retrieval and is escalated through #6060's decision record,
not “fixed” by a different model in this tier. Use #6046's published metric
definitions, baseline, sample requirements, and numeric regression
thresholds for the pass/fail decision; do not substitute a subjective
“acceptable” judgment or invent a competing threshold here. If #6046 has not
yet settled those criteria, that is an activation blocker: the golden-set
work must settle them before secret retrieval is enabled.

## 6. Operational home and service shape

**Initial home:** amd-6, conditional on the benchmark and its availability
as a fleet execution node. No basement node or cloud model service is
assumed. Keep the embed worker colocated with the authorized staging reader
and secret-tier writer on this host; use the `atlas-embed` library in a
dedicated, least-privilege process with one resident session. A local
process boundary is preferred over a general-purpose HTTP endpoint because
it avoids additional copies of secret text and bypasses the current
`/v1/embed` ceiling mismatch. If a separate local service is later
necessary, bind it only to a private fleet interface, authenticate the
caller for `Tier::Secret`, and prohibit public ingress and external
inference egress.

The worker consumes bounded local jobs and writes to an isolated secret
staging generation; it does not directly mutate a searchable shard while
embedding. On process restart, a leased job returns to the local queue and
the whole generation is retried or discarded. The host service manager
should restart a failed worker, but restart is not a fallback: until a
healthy local model is ready, jobs wait or fail closed. Set process memory
limits only after the benchmark establishes a safe peak plus headroom; do
not use a speculative limit that turns large but valid batches into
repeated crash loops.

The embedding worker emits only job ID, tier, pinned model identity,
chunk/token counts, duration, queue depth, and structured error class.
Raw text and tokens are excluded from logs, traces, crash artifacts, and
metrics labels. Vectors remain secret-tier material and are written only
to the secret generation. Existing tier credentials, discovery ceiling,
and pre-merge filtering remain mandatory.

## 7. Rollout and rollback

1. **Provision:** install the pinned model snapshot and ONNX Runtime from
   controlled artifacts; verify fixture hashes and run the local Gate A
   parity probes. Provisioning completes before any secret input is read.
2. **Evaluate:** run §5's hardware protocol and the available #6046 golden
   suite on the approved machine; publish only metadata and aggregate
   results. Check #6019's plan/spend and storage guardrails.
3. **Shadow locally:** process a bounded secret-tier batch into a private,
   non-searchable generation. Verify no egress, no body logging, correct
   model provenance, and all-or-nothing failure behavior. Do not expose
   shadow vectors to ordinary-tier fan-out.
4. **Enable narrowly:** promote only after the benchmark, quality, security,
   and classification gates pass. Keep the worker paused by default when
   host health or local model availability is uncertain.
5. **Rollback:** stop new secret jobs, preserve their local source units,
   and discard or retain the incomplete generation under the secret-tier
   policy. Do not re-embed through a hosted service. Resume only after the
   same pinned local pipeline passes parity and health checks.

## 8. Follow-on implementation and evidence

This design PR creates no runtime behavior. Follow-ons should be split so
that the security boundary and operational evidence are independently
reviewable:

1. Implement the secret-aware local staging aggregator and all-or-nothing
   generation promotion; require explicit `Tier::Secret` authorization and
   preserve the #6021 classification boundary. At promotion, record the
   mandated `embedding_model` identity (model ID, pipeline version, and
   dimensions) in the shard manifest and refuse a missing or mismatched
   identity. This is the secret-ingest portion of #6060's M2/M6 contract:
   coordinate with its manifest/consumer and promotion follow-ons (#7646
   and #7648), rather than creating a second provenance scheme. Query traces
   must carry the same per-candidate model provenance under #6027 / #6060 M5
   (follow-on #7647). Until the promotion assertion is in place, no secret
   generation is promoted or made searchable.
2. Add a reproducible local benchmark/report harness for token-length
   buckets, batch sizes, throughput, RSS, and burst drain. Keep raw secret
   inputs and outputs out of CI and public artifacts.
3. Complete #6046's versioned golden set and evaluate the mandated model
   under the secret-tier data boundary; publish only aggregate results.
4. Add operational service configuration on the approved execution host,
   with pinned local artifacts, bounded jobs, body-log suppression, health
   checks, fail-closed pause/retry behavior, and a systemd network sandbox
   that denies IP networking to the inference worker. Keep network-enabled
   provisioning separate and unable to read secret staging/corpus paths.

The exact implementation issue split, service memory limit, final batch
budget, and numeric ingest-freshness target are intentionally deferred
until #6019's ledger and §5's measurements exist. A future change to model
identity must amend the #6060 decision and re-evaluate all tiers; it is not
a local optimization.

## 9. Explicit non-goals

- Selecting or introducing a secret-only model, hosted or otherwise.
- Changing the public/internal model, vector metric, cross-tier merge, or
  #6060's one-space contract.
- Implementing the aggregator, new HTTP authorization, service deployment,
  or the #6046 golden-set builder in this design-only change.
- Claiming throughput, RSS, retrieval quality, or production readiness
  without the measurements and artifacts specified above.

## 10. Open questions / activation blockers

1. What exact #6019 ingest-freshness target and 95th-percentile burst
   distribution should the benchmark use? Until the ledger answers, use
   synthetic shape tests only and keep automatic secret ingestion off.
2. When will #6046 publish its versioned golden set and harness, and will
   any of its examples be secret-tier data? Its storage and execution
   location must honor this design's boundary.
3. Is amd-6's measured host capacity and uptime sufficient for this worker,
   or should a dedicated execution node be provisioned? Do not assume the
   unfinished basement cluster is available.
4. The current generic `/v1/embed` route accepts authenticated callers
   without enforcing their tier ceiling (#6788). The secret ingest
   implementation must avoid that route or land an independently tested
   authorization change before using it.
