# Troubleshooting

Source: `pkgs/atlas-search/crates/atlas-core/src/turso.rs`, `pkgs/atlas-search/crates/atlas-core/src/stage.rs` and `gate.rs`, `pkgs/atlas-search/crates/atlas-search/src/service.rs`, `pkgs/fleet/crates/fleet-search/src/cli.rs`, and `pkgs/atlas-search/worker/src/` (`index.ts`, `search.ts`, `service.ts`, `cache_do.ts`, `hot_cache.ts`, `space.ts`) under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ — provenance only; this chapter is complete without following it.

Every entry here is a recorded failure shape, not a hypothetical: what you see, why it happens, what to check, and what actually fixes it. Two distinctions resolve most confusion before you start — the service temp store (in memory, 10 minutes) is not the Worker cache (Durable Objects, indefinite), and standalone is not the service route. [The service and the Worker](../concepts/service-and-worker.md) draws both; [Running the service and the Worker](services.md) is the operator companion to this chapter.

## A search takes about four seconds

**What you see.** Hybrid and vector searches take roughly 4 seconds; FTS-only searches are fast by comparison.

**Why.** That is the serving physics of the current store, not a fault. The production vectors are stored as plain BLOBs — `vec_snowflake_m_v2_768_f32` holding a `F32_BLOB(768)` per chunk — and **`vector_top_k` does not work on them**. Semantic search is therefore an **exact cosine scan** (`--vector-backend scan`, the default): every allowed chunk's distance is computed, which the programme record measures at about 4 seconds for a plain scan, and a _filtered_ scan is 6–15 seconds of server-side work — which is why the service bounds concurrent branch executions and answers saturation with `429` rather than queueing without limit. Earlier measurements agree in scale: a pool-100 scan branch at 6.18 seconds wall on the Python path, and the Rust scan arm at 18.5–21.9 seconds end-to-end for golden queries under the rewrite-idle conditions of 2026-11-04.

**Check.** `GET /health` on the service: a high `queue_depth` or `active_branch_executions` means you are waiting behind other scans, not stuck. `--mode fts` on the same query confirms the split — fast FTS, slow vector is the signature of this entry.

**Fix.** There is no operator fix today; replacing the exact scan with an approximate-nearest-neighbour path is programme work, stated in the investigation record as "fix the serving physics". The `index-fixed` and `index-fill` vector backends exist in the CLI, but they call `vector_top_k` against the libsql vector index — the mechanism that does not work on the plain-BLOB vectors above — so switching to them is not a workaround for this store. Do not file the 4 seconds as an outage.

## A via-service search falls back to standalone — loudly

**What you see.** The search succeeds, but stderr carries a warning of the shape `fleet search: warning: --via-service: &lt;detail&gt;; falling back to standalone` (the `atlas-search` client prints its own equivalent), and the result envelope's `notes` carry the fallback note. Exit code is 0.

**Why.** The fallback is contractual, not silent, by design: any service failure under `--via-service` degrades to a standalone search with the reason recorded in both places. The recorded causes, in the order to check them:

- **The service is down or unreachable** — the unit is stopped, failed, or the port is taken by something else. Check `GET /health` on `127.0.0.1:18760` first.
- **The seat has no usable token.** A via-service client presents a token from the environment variable named `ATLAS_SEARCH_TOKEN`, or from `~/.config/atlas-search/client.json` (mode 0600). A seat with no client config **cannot use the service route** and falls back. A token the service does not know, or whose identity was removed from `~/.local/state/atlas-search/service.json`, fails the same way.
- **The request disqualifies the service route.** `--backend local` and a non-default `--vec-table` each force the loud standalone fallback with an explanatory note; any `--shards` specification skips the service attempt entirely, with no note. None of these three appears in any help text — this is source behaviour.
- **`get-chunk` always does this.** Chunk reads are job-scoped in the protocol, so `get-chunk` always runs standalone, with the loud note when via-service was in effect. That is its normal shape, not a failure.

**Fix.** Restore the missing leg — start the unit, provision the seat's client config, or accept the standalone route the request actually asked for. If the standalone artifact itself is missing from PATH, there is nothing to fall back to: `fleet search` exits **3** naming the missing artifact.

## An MCP caller sees only personal-tier results

**What you see.** Through the MCP endpoint (or the Worker REST search), results stop at the `personal` tier no matter what the caller asks for. `local-only` and `secret-bearing` content never appears, and no parameter changes it.

**Why.** The outbound search service identity is still shared and pinned: the Worker sends ceiling `personal` and disclosure context `hosted-per-user`, presenting the service's `cf-worker` client token, whose service-side cap is `personal`. Neither the REST body nor the MCP tool schema can widen that. An optional inbound `AGENT_TOKENS` secret distinguishes MCP callers for audit, but does not yet select a per-agent backend ceiling or corpus scope. After the service responds, the Worker verifies `effective_ceiling` is `personal` **before caching** — a result computed above `personal` is refused with a 502 and can never enter the cache. See #8625 for remaining per-agent backend policy and cross-client verification.

**Fix.** None on that surface — by design. A caller entitled to a higher ceiling uses the laptop service directly, with its own named client identity and cap (the user's CLI is capped at `secret-bearing`), under the rules in [Sensitivity tiers and client ceilings](../concepts/sensitivity.md). The Worker will never be that route: it holds no credential that could search above `personal`, and that absence is the security property. The full surface is in [The Atlas MCP server](../reference/mcp.md).

## Cache answers look stale

**What you see.** A hosted (Worker) search returns the same answer after the corpus underneath it has changed. Repeating the query, waiting, and redeploying the Worker all change nothing.

**Why.** The Worker's Durable Object cache **retains entries indefinitely — no TTL, no eviction in v1, by the owner's order** (#6805). Each (normalized query, clamped `k`, pinned ceiling, pinned context, pinned model revision, pinned vector table) gets one Durable Object holding one row; a complete entry is served verbatim forever, with only its hit counter moving. The deferred triggers for revisiting this exist on the record — add a corpus generation to the key at the first same-space re-ingest changing more than 5% of chunks or on a stale-result complaint, add an LRU sweeper at 10,000 cached queries or 1 GB aggregate — but they are triggers, not code. Do not confuse this store with the service temp store (10 minutes, in memory): the stale answer is the Worker's, and restarting the laptop service cannot touch it.

**Check.** `GET /v1/cache/inspect?query=&lt;query&gt;&amp;k=&lt;k&gt;` on the Worker (gated) returns metadata only — whether the entry exists, whether it has an embedding and a result, its hit count and timings — which proves the answer came from cache without exposing the cached body.

**Fix.** Purge the exact key: `POST /v1/cache/purge` (gated) with body `{ "query": "&lt;query&gt;", "k": &lt;k&gt; }` deletes that one entry's row and returns `{ "key": …, "query": …, "purged": true }`; the next search is a miss and re-runs against the service. Purge is **exact-key only** — there is no prefix or enumeration purge, intentionally: the key is a SHA-256 digest, and enumeration would need a separate index. Note that `k` is part of the key: purging at the wrong `k` purges a different entry and leaves yours intact.

## The D1 flag is on, but nothing changes

**What you see.** `HOT_CACHE_ENABLED` is set to `"true"`, the `HOT_CACHE` D1 binding exists — and no read or write behaviour changes at all.

**Why.** The D1 tenant hot cache is **storage layer only, not wired into the request path** (#13105). Its module provides tested get / put / touch / evict primitives over the `hot_cache` table, but nothing in the Worker's routing, search, MCP, service, or cache code ever calls the store — its only callers are its own tests. The flag's own semantics are narrow (only the literal string `"true"` enables the store constructor, and only when the binding also exists), but even a correctly enabled store is never consulted. Freshness evaluation (#13101) and the admission/eviction sweep (#13103) are likewise follow-ons.

**Fix.** None available, and none needed: leave the flag at its committed `"false"`. When you are asked where a hosted answer was cached, the answer today is always the Durable Object, never D1. Turning the flag on in the hope of changing staleness, latency, or cost will do none of those things — see the stale-cache entry above for the purge that does work.

## A promotion refuses

**What you see.** `atlas-stage promote` (or the registry's promote transition) fails fatally and the staging unit stays unpromoted — it is not left half-merged.

**Why — the model gate.** Promotion is gated on the embedding-model contract (M6 of the one-model decision, #7648 / #6060): a unit promotes **only** when its database's recorded `atlas_manifest.embedding_model` equals the mandated model, `snowflake-arctic-embed-m-v2.0`. The three refusals are all fatal and all leave the unit unpromoted:

- A different model is named — vectors from different embedding models are not comparable.
- No model is recorded — provenance unknown.
- The unit is empty.

**Why — classification.** The batch gate behind promotion runs five ordered stages — write → classify → dedup → provenance-check → publish — and fails closed: a stage with no recorded verdict has not passed, and verdicts are recorded strictly in order. A classification that cannot be resolved (`classify_unresolved`), a malformed or unrecognised per-chunk classification record (which resolves to `secret-bearing` on read-back, never to something more permissive), or a producer's segment declaration with a missing, mistyped, or unknown-sensitivity field (a usage error, never a silent default) will each stop the batch. The same reason code three times (`STRIKE_LIMIT = 3`) marks the batch `needs_review` and refuses further automatic retries; resume is at the failing stage, and changed bytes never inherit a verdict obtained for different content.

**Fix.** Read the refusal literally: stamp or re-embed the unit with the mandated model through the promote verb's embed-at-promotion step (it needs the model and ONNX runtime, resolved like the `atlas-search` binary), or fix the classification input at the producer — segment declaration stays with the producer; staging composes and persists it. Do not edit the recorded model name to match: the gate exists precisely because the name is a claim about vectors that must be true. The full workflow is in [Ingest today: staging, classification, promotion](../concepts/ingest.md).

## Results differ between Python and Rust

**What you see.** The same query through `atlas_search.py` (kept on the laptop as `atlas-search-py`) and through the Rust `atlas-search` returns different results or a different order.

**Why.** The Rust engine is a semantics port of the Python, with named parity breaks — check these before suspecting the corpus:

- **Context default.** Python `search` defaults `--context` to `local-owner` for **both** verbs and does not validate the value (an unknown context fails closed inside the disclosure logic). Rust `search` defaults to `fleet-internal`, Rust `get-chunk` to `local-owner`, and an unknown Rust context is a hard exit-2 error. Different contexts mean different context ceilings, which means different result sets — legitimately. Align `--context` explicitly before comparing anything else.
- **RRF constant.** The shipped production fusion — Rust, and the Python production path's own header — uses reciprocal rank fusion at **k = 10**, pool 100 per branch (`query-parameters-v1.json`). The Python **eval lineage** is a different configuration: `eval3.py` fuses top-30 branches at **k = 60** with truncation to 384 dimensions. Numbers or orderings produced through the eval harness are k = 60, trunc-384 numbers; they are not the production ordering and never were.
- **Smaller named breaks.** The Rust FTS match expression keeps short digit-bearing tokens (`r2`, `s3`) that the Python hybrid rule drops, and the Python baseline additionally removes a 23-word stop list. The Rust topics/projects matching deliberately diverges from the Python's LIKE behaviour (#5375): the Rust escapes the pattern so a value matches only a JSON element exactly equal to it, where the Python has the wildcard bug. Gate A's bit-identity is also version-matched: fixtures were recorded with ONNX Runtime 1.31.1, and a different runtime produces small deltas rather than a parity verdict.

**Fix.** Compare like with like: same context, same ceiling, same mode, the production path on both sides. Where a break above explains the difference, the Rust production behaviour is the shipped behaviour and the book documents the source, per the parity discipline — the standing gates themselves are in [Evaluation: golden sets and gates](../concepts/evaluation.md).

## The Worker returns 502

**What you see.** Hosted searches fail with `502` and the envelope `{ "error": { "code": "query_failed", "message": "…" } }`; MCP callers see the same failure as an `isError: true` result over HTTP 200.

**Why.** The Worker has no fallback: when the laptop service is unreachable, returns a non-JSON or error response, reports an `effective_ceiling` other than `personal`, or returns an embedding outside the pinned space (wrong model, wrong dimensions, a non-finite component), the Worker maps the failure to 502. Any upstream HTTP status is re-mapped to 502; the upstream status survives only inside the message text. Nothing is cached from a failed search beyond a possibly stored embedding — if the embed call itself failed, not even that.

**Fix.** Work the laptop leg: service health on port 18760, the tunnel hostname `atlas-search.security.cab`, and the Access service token's validity (names in [Running the service and the Worker](services.md)). The Worker side needs no clearing afterwards — a failed fill leaves the entry at most in the embedded state, and the next request retries the search without re-embedding.

## Exit-code quick table

Success is always 0. Error lines go to stderr prefixed with the binary's name; no partial JSON is printed on failure paths (the loud via-service fallback is the exception — it succeeds, at 0).

| Surface                         | 0                                                                | 1                                                                                                                       | 2                                                                                                          | Other                                                                                                                                                         |
| ------------------------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `atlas-search`                  | Success, including a loud standalone fallback                    | Backend / transport / embed failures; `get-chunk` not-found                                                             | Usage and validation: clap errors, count / parameter / context validation, `init` / `fetch-model` failures | —                                                                                                                                                             |
| `fleet search`                  | Success, including `--help` and bare `--version`                 | Service-job or backend failure with no fallback available                                                               | Usage and validation; rejected `init` / `fetch-model` verbs                                                | **3** = no service route and the standalone artifact is not on PATH. **69** = domain stub in a build that cannot serve it                                     |
| `fleet serve search`            | Help only — a running service never exits 0                      | Runtime failure: backend token, bind (an occupied 18760 exits 1)                                                        | Usage and config errors                                                                                    | —                                                                                                                                                             |
| `atlas-stage`                   | Success                                                          | All runtime errors: unknown sensitivity or state, missing unit, embedder or model absence, registry and database errors | Clap usage errors only                                                                                     | —                                                                                                                                                             |
| `atlas-classify`, `atlas-sweep` | Success (for `atlas-sweep`, including a `run-once` overlap-skip) | — (no exit-1 path)                                                                                                      | **Everything**: usage and runtime failures alike                                                           | —                                                                                                                                                             |
| `atlas-eval`                    | Success, report printed                                          | Unreadable input, invalid eval data, evaluation failure                                                                 | Wrong arity, unknown split                                                                                 | —                                                                                                                                                             |
| `atlas-github-comments`         | Success, including runs that skipped HTTP-451 repos              | **Everything**, including argument errors                                                                               | — (no exit-2 path)                                                                                         | —                                                                                                                                                             |
| `atlas_search.py`               | Success                                                          | `get-chunk` not-found; unhandled backend failures (as tracebacks)                                                       | Argparse errors; `die()` validation, token-resolution, and backend errors                                  | —                                                                                                                                                             |
| Worker HTTP                     | `200`                                                            | —                                                                                                                       | —                                                                                                          | `400` bad request, `401` gate failure, `404` no route, `405` non-POST to `/mcp`, `413` body over 64 KiB, `502` upstream / space / ceiling failure (see above) |

The inconsistency in the middle rows is real, not a table error: no two hand-rolled binaries in the family share one exit-code philosophy, and only the clap binaries mirror the Python. Check the row for the binary you actually ran before scripting against a code.
