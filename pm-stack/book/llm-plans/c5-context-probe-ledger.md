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
