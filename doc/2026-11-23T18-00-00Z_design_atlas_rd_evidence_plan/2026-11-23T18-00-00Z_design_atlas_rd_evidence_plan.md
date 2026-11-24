# Atlas R&D evidence plan

Decision record for [#13541](https://github.com/developing-today/code/issues/13541), a bounded publication under [#13241](https://github.com/developing-today/code/issues/13241). The standing implementation plan is maintained in [Implementation programme and evidence gates](../../pkgs/atlas-search/book/design/implementation-programme.md).

The programme should establish a source-to-reader freshness receipt and a branch-level authorization oracle before adopting any planner, index or ranker. The reason is causal attribution. A stale publication and a weak retrieval strategy can both produce a miss. An index that loses eligible neighbors can look faster while changing the underlying problem. Hold the eligible corpus and published revision constant to distinguish those failures.

The book chapter specifies the experimental arms, source-level retry and deletion checks, published evaluation baselines, producer-versus-consumer proofs, and release/rollback evidence. The roadmap remains the issue ledger. The existing same-change documentation rule and combined Pages deploy continue to govern the book.
