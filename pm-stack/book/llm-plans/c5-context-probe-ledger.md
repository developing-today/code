# C5 empirical context-probe ledger (task 23761, first runnable slice)

Status: runnable bounded measurement slice landed; no live context ceiling is claimed.
Recommendation-only boundary holds: no routing configuration was changed.

## Exact wired free leaf inventory (derived, not hand-copied)

Source of truth: `bin/free-context-probe.mjs --inventory-only` reads
`.opencode/opencode.jsonc` (`experimental.model_routes` plus
`experimental.provider_routes` alias expansion).

- Curated plan ids: 20 (`FREE_PLAN_IDS` in the harness).
- Direct provider/model leaves on 2026-11-25: **119** (56 configured
  targets expanded through shared provider routes such as `google-free`,
  `zen-plans`, and `opencode-go-plans`).
- Every leaf is tagged `contextSource: provider-published` until a live
  sentinel pass promotes exactly that leaf to `empirically-tested`.
  Catalog `limit.context` values are not test results.

Regenerate: `node bin/free-context-probe.mjs --inventory-only --out target/fleet-output/free-context-inventory.json`

## What this slice proves

- Offline suite `node --test bin/free-context-probe.test.mjs` (7 tests):
  inventory oracle (configured plan targets, alias expansion, no
  `opencode-route/*` wrappers), deterministic sentinel payload oracle,
  narrowed rate/auth/context classification oracle (HTTP 413 alone is not
  a context proof; only explicit context signals classify as
  `fail-context`), provenance oracle, strict argv rejection (unknown,
  duplicate, and mutually exclusive flags), and a no-network inventory
  CLI run.
- Dry-run manifest builds one deterministic payload shape per leaf with
  explicit cost controls (`--max-probes`, `--timeout-ms`,
  `--max-input-tokens`, one attempt per leaf, zero network calls).
  The chars/4 input figure is labeled a rough preflight estimate, not a
  provider tokenizer oracle.
- Live mode is one keyless Kilo call only (`--live` requires exact
  `--leaf kilo/kilo-auto/free` and `--max-probes 1`, fixed 8-block
  payload, 64-token output cap, bounded body read), records requested
  leaf, response model / serving leaf, input shape, server-reported
  usage, latency, and failure reason, and never logs credentials or raw
  error bodies. Exit status keys on HTTP 200, not on the sentinel
  alone. All other leaves remain dry-run.

## Live probes run in this slice

One keyless Kilo smoke probe, 2026-11-25
(`node bin/free-context-probe.mjs --live --leaf kilo/kilo-auto/free
--max-probes 1`; record:
`pm-stack/book/llm-plans/evidence/c5-kilo-free-smoke-2026-11-25.json`):

- HTTP 200, sentinel returned (`pass-sentinel`), ~1 s latency,
  response model `reflection/beam-free`
  (serving leaf `kilo/reflection/beam-free`, upstream provider not
  reported by the gateway), server-reported usage 271 in / 60 out.
- This is a small sentinel smoke probe only (fixed 8-block payload,
  64-token output cap), not a context-ceiling test. No context value is
  promoted: the record stays `contextSource: provider-published`.

## Remaining work (explicit ledger)

1. On the authorized host, run one `--live` probe per representative leaf
   (one per plan first: 20 probes, `--max-probes 1`, `--timeout-ms 30000`,
   small `--max-input-tokens`), capture the JSON records, and file the
   serving-leaf attribution table (requested vs served, usage, reason).
2. Binary-search each wired leaf per the payload family: highest
   repeatable pass and first repeatable fail with the end sentinel;
   repeat the boundary once to reject transient provider errors.
3. Controlled routing comparison with stated conditions: same prompt set
   through `openrouter-free-plan` vs direct leaves and through
   ordered vs weighted vs round-robin free clusters, recording per-leg
   attempts, rate-limit walls, and fall-through behavior.
4. Promote only passing leaves to `empirically-tested` with date, request
   shape, accepted input tokens, server-reported usage, and outcome; keep
   every other leaf at `provider-published`.

## Safety notes

- Kilo `kilo-auto/free` measures the router, not a stable model: key
  results by actual serving leaf.
- OpenRouter `:free` needs serving-leaf telemetry; route success alone
  does not prove an OpenRouter completion (B6 precedent).
- SambaNova stays excluded (402 payment-required); Cloudflare stays
  excluded (revoked credential) until fresh successful probes exist.

## Slice 3 (2026-11-27, task 25259) — representative dry-run + Kilo repeat

Recommendation-only boundary holds: no routing configuration changed.
Harness reused as-is from PR #15006; no harness code changed.

### Reproducible commands (run from repo root, origin/main `75996dcc9c`)

- Offline suite: `node --test bin/free-context-probe.test.mjs` → 7/7 pass.
- Fresh inventory: `node bin/free-context-probe.mjs --inventory-only`
  → 119 direct leaves, 20/20 plans present, zero missingPlans.
  Matches the slice-1 baseline (119 leaves).
- Representative dry-run (one leaf per plan, zero network):
  for each of the 20 FREE_PLAN_IDS,
  `node bin/free-context-probe.mjs --dry-run --plan <plan> --max-probes 1 --max-input-tokens 8000`
  → record `pm-stack/book/llm-plans/evidence/c5-representative-dryrun-2026-11-27.json`
  (20 representatives, fixed 8-block sentinel payload, ~128–164 approx tokens each).
- Kilo live re-probe (repeatability check, one keyless call):
  `node bin/free-context-probe.mjs --live --leaf kilo/kilo-auto/free --max-probes 1`
  → record `pm-stack/book/llm-plans/evidence/c5-kilo-free-rerun-2026-11-27.json`.
- Routing-comparison dry-run leg (zero network):
  `node bin/free-context-probe.mjs --dry-run --plan openrouter-free-plan --max-probes 11 --max-input-tokens 8000`
  → 11 same-shape payloads (~128 approx tokens each), staged at
  `target/fleet-output/c5-routing-openrouter-leg.json` (regenerable; not committed).

### Captured outcomes

- Kilo rerun 2026-11-27T00:33:43Z: HTTP 200, outcome `pass` (http-200) but
  NOT `pass-sentinel` — the end-sentinel criterion from 2026-11-25 did not
  reproduce. Serving leaf rotated: `kilo/reflection/beam-free` (11-25) →
  `kilo/nvidia/nemotron-3.5-super-vl:free` (11-27). Usage 245 in / 64 out —
  output exactly at the 64-token cap, consistent with truncation before the
  sentinel (or non-following model). Latency 1634 ms vs 1067 ms prior.
- Ledger note confirmed empirically: `kilo-auto/free` measures the router,
  not a stable model. A single `pass-sentinel` is not a ceiling and does not
  transfer across serving leaves.
- Promotion: none. Every leaf stays `contextSource: provider-published`,
  including Kilo (rerun failed the sentinel criterion).

### Controlled routing comparison — status: live leg BLOCKED, design preserved

- Blocker: the harness live adapter is keyless-Kilo-only by construction
  (`--live` rejects any other `--leaf`). There is no credentialed live
  adapter for `openrouter/*:free` or for direct free leaves, and this slice
  does not add network capability. So no live same-prompt comparison
  (openrouter-free-plan vs direct leaves; ordered vs weighted vs
  round-robin) could run.
- Preserved evidence: the dry-run openrouter leg (11 identical-shape
  payloads) is staged for the future live adapter; direct-leaf counterparts
  are the 20 representatives above. Live comparison design: same sentinel
  payload through each leg, record per-leg attempts, rate-limit walls
  (429s), serving-leaf attribution, and fall-through behavior.
- Notable cross-evidence (observational only): the Kilo router served
  `nvidia/nemotron-3.5-super-vl:free` on 11-27, a model family also present
  on the openrouter-free-plan leg. No routing claim follows; recorded for
  the future comparison.

### Harness gap noted (recommendation, not changed in this slice)

- The live record persists `outcome`/`reason` but not the underlying
  `sentinelReturned` boolean; `pass` (HTTP 200) vs `pass-sentinel` is
  recoverable only by re-reading the outcome string. Recommend persisting
  `sentinelReturned` explicitly in a future harness slice.

### Remaining work (updated)

1. Credentialed live adapters (one per provider family, same cost controls:
   one attempt, strict timeout, sanitized metadata) are the precondition
   for ledger items 1–3 (per-plan live probes, binary-search ceilings,
   live routing comparison). Until then, live work stays Kilo-smoke-only.
2. Binary-search ceilings remain unstarted (need live adapters).
3. Ordered vs weighted vs round-robin cluster comparison remains unstarted
   (needs live adapters + pool-steward coordination; policy only here).

## Slice 4 (2026-11-27, task 26036) — credentialed direct adapters + capability matrix

Recommendation-only boundary holds: no routing configuration changed, no
provider configuration changed, no live call was made by this slice.

### What landed

- `bin/free-context-probe.mjs` gains credentialed direct adapters
  (`DIRECT_ADAPTERS`, `resolveDirectAdapter`, `runDirectProbe`,
  `createProbeBudget`) for three source-backed OpenAI-compatible provider
  families: **groq** (`https://api.groq.com/openai/v1/chat/completions`,
  credential env `GROQ_API_KEY`, Groq OpenAI-compatibility docs),
  **nvidia** (`https://integrate.api.nvidia.com/v1/chat/completions`,
  credential env `NVIDIA_API_KEY`, NVIDIA API Catalog getting started),
  and **openrouter** (`https://openrouter.ai/api/v1/chat/completions`,
  credential env `OPENROUTER_API_KEY`, OpenRouter docs). Kilo stays on
  its separate keyless path.
- Fail-closed contract, all checks BEFORE any network call: unsupported
  provider, exact leaf absent from the wired inventory, output cap above
  the hard cap (64 tokens), named credential env var absent or empty,
  missing explicit call budget, or budget exhausted. Run budgets are
  hard-bounded at 5 live calls (`DIRECT_PROBE_MAX_CALLS`); the CLI live
  path uses a budget of exactly 1. A provider that reports output beyond
  the requested cap invalidates the measurement (`fail-cap`), never a
  pass. Credentials travel only in the Authorization header; records
  carry the env var NAME (`credentialSource: env:<NAME>`), never a value.
- `--live` now accepts an exact direct leaf (e.g.
  `--leaf groq/openai/gpt-oss-120b --max-probes 1`) alongside the Kilo
  leaf. A fail-closed probe writes no record and exits 4.
- Live records now persist `sentinelReturned` explicitly for both
  adapters — the slice-3 harness gap noted above is closed.
- Offline suite: `node --test bin/free-context-probe.test.mjs` → 15/15
  pass (7 pre-existing + 8 new). The new tests name their oracles:
  fail-closed paths leave the mock-fetch call count at zero; the happy
  path keys the serving leaf by the response-reported model, carries the
  64-token cap in the request body, and contains no credential value; a
  CLI spawn with `GROQ_API_KEY` scrubbed exits 4 with
  `missing-credentials` and writes no record.

### Live leg — not run; capability matrix is the deliverable

No non-Kilo credential is provisioned in this lane's environment: an
env-var NAME check on 2026-11-27 found zero provider key names (no
`GROQ_API_KEY`, `NVIDIA_API_KEY`, `OPENROUTER_API_KEY`, or any
`*API_KEY` name; values were never read). Per the task's fallback, this
slice delivers the source-backed capability matrix instead of a live
comparison — full matrix with per-family prerequisites:
`pm-stack/book/llm-plans/evidence/c5-slice4-capability-matrix-2026-11-27.json`.

Summary (representative leaf → prerequisite):

- groq `groq/openai/gpt-oss-120b` → provision a Groq Free Plan key as
  `GROQ_API_KEY` on the probe host, then one bounded `--live` call.
- nvidia `nvidia/nvidia/nemotron-3-ultra-550b-a55b` → generate a free key
  at build.nvidia.com/settings, provision as `NVIDIA_API_KEY`, then one
  bounded `--live` call.
- openrouter `openrouter/inclusionai/ling-3.2-flash:free` → provision
  `OPENROUTER_API_KEY`, then one bounded `--live` call; key results by
  the response-reported model (router caveat, B6 precedent).
- zai, google.muse, _commandcode-free, exaone → no adapter: no
  source-verified direct endpoint/credential name recorded, or the wired
  leaf is an internal route name rather than a direct provider identity.
  Prerequisite per family is in the matrix JSON; endpoints and credential
  names must be source-verified, never guessed.
- opencode zen/go route families (11 of 20 plans) → excluded by design:
  aggregator routes; probing them measures the router.

### Acceptance mapping for #9573

- Advances item 3's precondition: the free-provider inventory now has a
  credentialed exact-leaf probe path for three provider families.
- Item 3 is NOT completed: no live exact-leaf measurement has run (no
  credential provisioned), no context value is promoted (every leaf
  stays `contextSource: provider-published`), no free path is wired in
  front of a production path, and fall prevention is not demonstrated.
- Item 2's controlled routing-mode comparison remains unmeasured; this
  slice does not claim it. Item 1 was delivered by PR #11979.

### Remaining work (updated)

1. Provision ONE named credential above on the probe host and run the
   first bounded direct comparison: the same sentinel payload through a
   direct leaf and through the existing routed path where the same model
   is available (≤5 live calls total, 64-token output cap), recording
   serving-leaf attribution in this ledger.
2. Binary-search ceilings per leaf (needs step 1's live adapters proven).
3. Ordered vs weighted vs round-robin cluster comparison (needs live
   adapters + pool-steward coordination; policy only here).
