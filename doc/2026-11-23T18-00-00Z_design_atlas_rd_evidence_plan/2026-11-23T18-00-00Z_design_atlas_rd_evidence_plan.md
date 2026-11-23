# Atlas R&D evidence plan

Decision record for [#13541](https://github.com/developing-today/code/issues/13541), a bounded publication under [#13241](https://github.com/developing-today/code/issues/13241). The standing implementation plan is maintained in [Implementation programme and evidence gates](../../pkgs/atlas-search/book/design/implementation-programme.md).

The programme should establish a source-to-reader freshness receipt and a branch-level authorization oracle before adopting any planner, index or ranker. The reason is causal attribution. A stale publication and a weak retrieval strategy can both produce a miss. An index that loses eligible neighbors can look faster while changing the underlying problem. Hold the eligible corpus and published revision constant to distinguish those failures.

The book chapter specifies the experimental arms, source-level retry and deletion checks, published evaluation baselines, producer-versus-consumer proofs, and release/rollback evidence. The roadmap remains the issue ledger. The existing same-change documentation rule and combined Pages deploy continue to govern the book.

## Decisions and their acceptance receipts

The follow-on [workstream protocol issue](https://github.com/developing-today/code/issues/13592) puts acceptance criteria beside the interfaces they govern. Use these experiment boundaries when reporting a result.

| Workstream | Decision unit | Evidence needed |
|---|---|---|
| [A. Ingest](../../pkgs/atlas-search/book/design/automatic-ingest.md) | One adapter and its ordered publication | A duplicate, delayed update, partial enumeration, interrupted capture and revoked occurrence converge to the expected eligible state. Record independent capture, publication and reconciliation watermarks. |
| [B. Planner](../../pkgs/atlas-search/book/design/query-planning.md) | One transform for one query family | Restricted context stays restricted, original query fallback is reproducible, and a bounded classifier plus end-to-end request meets its own measured budget. |
| [C. Retrieval](../../pkgs/atlas-search/book/design/layered-search.md) | Resolution, ANN and rescore as three independent changes | Check physical and query eligibility before retrieval. Report exact-neighbor overlap, judged evidence recall, small eligible cohorts and resource costs. |
| [D. Reranking](../../pkgs/atlas-search/book/design/reranking.md) | Reorder an identified eligible candidate pool | Prove the ranker only permutes or truncates its input. Report paired ordering gain, tail latency and timeout fallback against the original ordering. |
| [E. Recommendations](../../pkgs/atlas-search/book/design/recommender.md) | Candidate union, ranking and signed paging as separate stages | Use temporal holdout, cold-start baseline, invariant hidden-only changes, and idempotent replay after a restart. |
| [F. Fleet and ID](../../pkgs/atlas-search/book/design/fleet-and-id-integration.md) | One projection direction or scoped replica lease | Native edits and return projections retain distinct origin. An older event cannot resurrect deleted data. Offline reads stop when their authorization lease expires. |
| [G. Access and caches](../../pkgs/atlas-search/book/design/access-evolution.md) | One representation, current scope and absolute validity window | Compare warm and cold reads, expired cursor and ETag, denied hydration, and idempotent expiry cleanup. Record D1 and Durable Object outcomes independently. |
| [H. Evaluation](../../pkgs/atlas-search/book/design/evaluation-programme.md) | One preregistered decision on matched eligible populations | Keep scored, unscored and inconclusive arms distinct. Disclose coverage, paired uncertainty, measured latency, release threshold and policy-preserving rollback. |

The archive describes the decision boundaries. The issue map and implementation chapter provide current owners, ordering and linked implementation evidence; raw evaluation inputs and private corpus records remain local.

Implementation receipts should be attached to the corresponding decision. Track the [bounded census reader](https://github.com/developing-today/code/pull/13567), [scored development-arm requirement](https://github.com/developing-today/code/pull/13554) and [negotiated MCP output schema](https://github.com/developing-today/code/pull/13562) independently. A merged schema or scorer guard establishes its own contract. Production ranking, remote corpus freshness and client interoperability each need an additional end-to-end receipt.
