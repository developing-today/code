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
  rate/auth/context classification oracle, provenance oracle, serving-leaf
  event parsing, strict argv rejection, and a no-network inventory CLI run.
- Dry-run manifest builds one deterministic payload shape per leaf with
  explicit cost controls (`--max-probes`, `--timeout-ms`,
  `--max-input-tokens`, one attempt per leaf, zero network calls).
- Live mode is single-leaf only (`--live` requires exact `--leaf` and
  `--max-probes 1`), enforces the token cap before spawning, records
  requested leaf, actual serving leaf, input shape, token usage, latency,
  and failure reason, and never logs credentials or raw error bodies.

## Live probes run in this slice

None with provider credentials. The authorized-host live path is
implemented (`--live` shells `opencode run --format json --model <leaf>`
with a strict timeout and the sentinel oracle) but this lane did not
execute credentialed probes; no context value is promoted.

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
