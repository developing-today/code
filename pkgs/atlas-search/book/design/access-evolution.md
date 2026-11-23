# Design: MCP and programmatic access, next steps

> **Design — not shipped behaviour.** This chapter describes work that is designed and tracked, not behaviour that works today. The current MCP server exposes exactly one tool, and the current Worker cache has no TTL or eviction; both facts are stated plainly in [What exists today](#what-exists-today) at the end. Tracking: master prompt §2G in the Atlas programme investigation; [#8625](https://github.com/developing-today/code/issues/8625) (MCP surface and its cross-client acceptance remainder), [#6805](https://github.com/developing-today/code/issues/6805) (Worker cache eviction and invalidation), [#9134](https://github.com/developing-today/code/issues/9134) (D1 hot cache), and [#12900](https://github.com/developing-today/code/issues/12900) thread 2 (the multi-client question, which is Drew's open decision — see the end of this chapter).

Source: `pkgs/atlas-search/worker/src/index.ts`, `pkgs/atlas-search/worker/src/mcp.ts`, `pkgs/atlas-search/worker/src/search.ts`, `pkgs/atlas-search/worker/src/cache_do.ts`, `pkgs/atlas-search/worker/src/space.ts`, `pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/mcp.ts — provenance only; this chapter is complete without following it.

Atlas has three access surfaces: MCP (the Model Context Protocol, by which agent clients call tools) for agents, REST for services, and the CLI for humans. The design principle for their evolution is stated once and applied everywhere: one retrieval core, three surfaces, and tier enforcement inside the core. Sensitivity ceilings and corpus scoping are enforced inside the retrieval core's branch queries — the queries that gather full-text and vector candidates — never post-ranking and never by client goodwill. Every surface therefore inherits the same enforcement, and a new surface cannot accidentally widen access by forgetting a filter. The background is in [Sensitivity tiers and client ceilings](../concepts/sensitivity.md); the current surfaces are documented in [The Atlas MCP server](../reference/mcp.md), [The Worker HTTP API](../reference/worker-api.md), and [The service HTTP API](../reference/service-api.md).

## The MCP growth path

### The two-step shape: search, then read

The copyable access pattern for agents is a two-step shape: `search` returns references and snippets, and a second step reads the full content of a chosen reference. The first step is deliberately light — result identifiers, provenance (corpus and `source_ref`), a snippet sufficient to judge relevance, and the effective ceiling the search ran under. The second step fetches one identified chunk or object in full, re-checking the caller's ceiling at read time, so a reference obtained under one ceiling cannot be redeemed under another.

What the MCP surface must never do is resource enumeration at corpus scale: listing or paging the corpus itself as MCP resources. The corpus is on the order of a hundred thousand chunks; an enumeration surface would be a bulk-export surface wearing a protocol's clothes, it would bypass the ranking and ceiling logic that make search safe, and no agent workflow needs it — agents need the two-step shape above. Search is the enumeration boundary: what search cannot return under a caller's ceiling, enumeration must not return either.

### Growing the single `search` tool deliberately

The current server exposes a single `search` tool — see [What exists today](#what-exists-today). The design grows it deliberately, one tool at a time, each with the same identity-stamping, ceiling, and audit treatment as `search`:

- **search** — the existing tool: hybrid search over the unified index, returning references and snippets.
- **read** — the second step of the two-step shape: fetch one referenced chunk or object in full, ceiling re-checked at read time.
- **related / recommend** — related results beside a known item, and recommendations from the recommender workstream (see [Design: the single-user recommender](recommender.md)): more-like-this over the embedding index and the stored recommendation objects of [#9109](https://github.com/developing-today/code/issues/9109) Part C. One surface, two entry points: related-to-an-item and recommended-for-the-user.
- **corpus status** — a read-only status tool reporting what the corpus holds and how current it is: the index version, the ingest watermark, and per-corpus counts. This tool exists because freshness is currently invisible to consumers (counts frozen at a publish date, with no live signal); an agent that can read corpus status can say how stale its own search results may be.

Deliberately not tools, now or in this design: cache purge (a write; it stays an operator surface) and cache inspect (an operator and audit surface). Both exist today as gated REST routes and stay there.

Each addition lands behind the evaluation and acceptance gates of its workstream. Growing the tool list is not a version bump; a tool ships when its acceptance — including the cross-client acceptance below — passes.

## REST: signal provenance with `explain`

The REST surface serves services rather than agents, and its designed addition is signal provenance: an `explain` facility on search results that states, per result, which signals produced it and how they combined. Atlas retrieval is hybrid — a full-text (FTS/BM25) branch and a vector branch fused with reciprocal rank fusion (RRF) — with further stages designed in [Design: layered search characteristics on Snowflake](layered-search.md) and [Design: the reranking stage](reranking.md). A bare score tells a calling service nothing it can act on; a provenance record — which branches returned the chunk, at what ranks, with what per-signal weights, and what any reranking stage did to it — lets the service debug its own queries, lets the evaluation programme attribute failures to a stage, and makes the query-as-data record (`atlas-query-trace/v1`) complete.

The design returns `explain` on request, not by default: provenance is per-result bulk that interactive callers do not need. When requested, it is part of the versioned contract below, so MCP, REST, and CLI renderings of the same search agree on what the signals were.

## One versioned contract: growing `atlas-proto`

All three surfaces currently honour, to different degrees, the seed contract in the `atlas-proto` crate: `query-parameters-v1.json`, the versioned query-parameter set (today: result depth, candidate pool, expansion, the RRF constant, and per-signal weights, with defaults every surface shares). The seed is described in [atlas-proto: the wire contract](../library/atlas-proto.md).

The design grows that seed into the versioned API contract all three surfaces honour: query parameters, the result envelope, the `explain` provenance shape, error shapes, and the corpus-status shape, each versioned, each change additive within a version and a new version when it is not. A surface that cannot honour a contract field says so as a validation error — the existing precedent is the CLI's planner and spell flags, which are present in the contract, accepted by the parser, and refused as not implemented rather than silently ignored. The contract is the mechanism that keeps "one retrieval core" true in practice: if the surfaces share the parameters and the envelope, they cannot drift into three different searches that happen to share a name.

## Closing #8625's remainder: cross-client acceptance

[#8625](https://github.com/developing-today/code/issues/8625) — the MCP query surface over the unified index, as an mcpx namespace — is partly landed: the hosted MCP endpoint is live on the Worker and wired into the laptop's mcpx daemon as its remote `atlas` server. What remains open, and what keeps #8625 open, is cross-client acceptance.

The acceptance is per client, for the four consumers named in the issue: **OpenClaw, OpenCode, OpenChamber, and Muse lanes**. Each client must be shown querying the unified index through the namespace with:

- **identity stamped** — the query carries the calling agent's own identity, not a shared hosted identity;
- **ceiling and corpus scoping enforced in branch queries** — the client's sensitivity ceiling and its permitted corpora are applied inside the retrieval core's branch queries, before ranking, so an out-of-scope chunk cannot surface, be counted, or leak its existence through a result total;
- **audit-logged** — every query is written to the audit log with its identity, ceiling, corpus scope, and trace identifier.

Today all hosted MCP callers share the one identity `hosted-per-user` at the `personal` ceiling; per-agent identity and scoping are #8625's later slices and are deliberately not improvised in the current Worker code. Closing the remainder means replacing that shared identity with per-client identities on the hosted path and passing the four-client acceptance above. Until then, #8625 stays open and this chapter describes the acceptance as a requirement, not a result.

## The Worker cache fix (#6805)

The Worker's per-query Durable Object cache currently retains every result indefinitely: no TTL, no eviction, no invalidation — the `cache_do.ts` header states this is v1 behaviour, and [#6805](https://github.com/developing-today/code/issues/6805) tracks the fix. Indefinite retention is not a caching strategy: the corpus changes, embedding spaces are pinned and revised, and a cache that never expires serves a frozen corpus forever while reporting itself as a hit.

The designed fix has three parts, consistent with the deferred triggers already recorded in the Worker README:

- **A TTL and eviction policy.** Entries expire on a stated time-to-live and are evicted least-recently-used under a stated size bound, replacing retention-by-default with retention-by-policy.
- **Invalidation on corpus change.** The ingest watermark is the corpus-change signal: when ingest advances the corpus past a stated threshold — the recorded trigger is the first same-space re-ingest changing more than 5% of chunks, or a stale-result complaint — the cache key gains the corpus generation, orphaning prior entries exactly as the fleet keying design in [Design: search integration for fleet and ID](fleet-and-id-integration.md) does.
- **A sweeper at scale.** At the recorded scale triggers (10,000 cached queries or 1 GB aggregate), an LRU sweeper reclaims orphaned and expired entries rather than letting Durable Object storage grow without bound.

The exact-key purge route that exists today remains as the operator tool; it is not a substitute for the policy above, because no operator purges one query at a time against a corpus-wide ingest. The D1 hot cache (the [#9134](https://github.com/developing-today/code/issues/9134) chain) is a separate store with its own freshness, admission, and invalidation slices; it is not the fix for #6805, and its master flag is off today.

## Open decision: #12900 thread 2 — the multi-client future

[#12900](https://github.com/developing-today/code/issues/12900) thread 2 asks whether Atlas is headed for multi-client use — other clients and fork sessions querying it as a tool, with the service and result-store shape that implies — or whether it stays Drew's personal system with the surfaces it has. Thread 1 of that issue (parallel FTS and vector branches) has landed; thread 3 (the Nix-packaged Rust binary) has substantially landed elsewhere. Thread 2 remains open, and it is **Drew's decision**.

This chapter therefore presents thread 2 as an open decision and designs nothing on top of a multi-tenant answer. The work above — per-client identity, ceilings in the core, a versioned contract — is justified for Drew's own fleet of agents on its own merits and does not presume the answer. If the decision goes toward multi-client use, tenant isolation, per-tenant store placement, and the D1 hot-cache chain become load-bearing and get their own design pass; if it does not, they remain what they are today: a personal system with several clients of one owner. Either answer leaves this chapter's design intact, which is the point of not pre-building for an answer that has not been given.

## What exists today

- The Atlas MCP server lives in the Cloudflare Worker (`worker/src/mcp.ts`). It is a stateless Streamable HTTP endpoint at `POST /mcp` exposing exactly **one tool, `search`**, with input `{query, k?}` (`k` clamped to 1–50, default 10), over the same `executeSearch` path as REST `POST /v1/search` — an MCP search and a REST search for the same normalised query share one Durable Object cache entry. Cache purge and inspect are deliberately not MCP tools. There is no `read`, `related`/`recommend`, or corpus-status tool today.
- The laptop's mcpx connects to that endpoint as its remote `atlas` server. All hosted callers share the identity `hosted-per-user` and the pinned ceiling `personal` (compile-time constants in `worker/src/space.ts`, verified against the service's reported effective ceiling before any result is cached). Per-agent identity stamping, per-agent ceilings and corpus scoping, and a query audit log do not exist on the hosted path today; they are [#8625](https://github.com/developing-today/code/issues/8625)'s open later slices, and #8625 stays open on cross-client acceptance for OpenClaw, OpenCode, OpenChamber, and Muse lanes.
- The Worker holds no database credentials; fan-out, embedding, and merge live in the fleet search service, reached over the `atlas-search.security.cab` tunnel with a Cloudflare Access service token. The Worker's public routes are `GET /health` (open), `POST /v1/search`, `GET /v1/cache/inspect`, and `POST /v1/cache/purge` (all gated), and `POST /mcp` (gated).
- The Worker cache retains entries indefinitely: no TTL, no eviction, no invalidation in v1; the only deletion is the exact-key operator purge. [#6805](https://github.com/developing-today/code/issues/6805) is open.
- The D1 hot-cache binding is present in the Worker configuration with its master flag off (`HOT_CACHE_ENABLED = "false"`), and in the current code nothing in the request path calls the D1 store at all; the storage layer landed under [#13100](https://github.com/developing-today/code/issues/13100) (closed) and its follow-ons are gated or in review — see [Roadmap and issue map](roadmap-and-issue-map.md).
- `atlas-proto`'s `query-parameters-v1.json` exists as the seed contract (`SearchParametersV1`: `k`, pool, expand, RRF `k`, per-signal weights). No `explain` provenance field, no corpus-status surface, and no grown multi-surface contract exist today. [#12900](https://github.com/developing-today/code/issues/12900) thread 2 is open and undecided.
