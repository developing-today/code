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
