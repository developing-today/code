# Part C judge repair disposition

**Scope:** `review/part-c-judge.md` findings 1–6. Repairs are limited to Part C artifacts; no cloud credentials/spend, benchmarks, Git operations, issue filing, or additional review loops.

1. **Oracle semantics:** replaced shape-only checks with a finite in-memory reference store executing 11 put/get/metadata/reference/tombstone/resurrection transitions. It computes SHA256 over exact fixture profile bytes (`abc`), checks the resulting digest against the fixture, compares every transition to expected output, and checks all 44 operation/engine capability labels (including R2 metadata scan unsupported). Output now makes only those claims; it does not assert production race safety or provider correctness.
2. **Laptop evidence run:** added `C9133-LOCAL`, a proposed redb+Turso runner/result slice. It pins identical corpus bytes/digest and requires runner, revision, hardware, OS/runtime, cache/storage-shard and dataset metadata plus result-v1 validation. This is a follow-up only and was not run. Cloud evidence remains separate.
3. **Recall/shard ownership:** removed recall acceptance/recommendations from #9133. Search recall@10 and Atlas search shard decisions belong entirely to #9092/#8627. #9133 may optionally link a #9092 artifact after schema and corpus/eval digest validation; it never reruns recall or recommends shards. Storage partition count is explicitly `storage_shard_count`.
4. **Cloud validator ownership:** `C9133-CLOUD` owns new `pkgs/id/benchmarks/validate_results.py`, its strict CLI test, and cloud result record. CLI contract is explicit: offline validator requires `--input`, optionally accepts `--output` and `--owner-search-result`, rejects unknown/malformed arguments before network/lock/write/spend; benchmark runner is a distinct gated entry point.
5. **Research file collision:** C9135-01 now owns proposed `part-c/store-placement-research.md`; C9138-01 owns `part-c/embedder-candidates-research.md`. Neither shares a research file with the other. Existing `research.md` remains this design's consolidated record, not their implementation-owned path.
6. **Recommendation total order:** snapshot materialization orders score descending, immutable recommendation ID ascending. C9139-01's finite model oracle now covers ties, post-cutoff arrivals, restart, duplicate replay, replay/cursor expiry, terminal/empty-nonterminal pages, and signed derived output.

## Reinstated requirements

- Hot cache: tenant-tier/compute-usage admission, per-tenant/global byte bounds, deterministic LRU/age eviction, and fail-closed immediate policy revocation.
- Untrusted shared bucket: co-tenants see ciphertext payload only; canonical `ContentId`/`PayloadId` remain distinct from opaque physical locator.
- GC: atomic #9084 generation fence remains a hard prerequisite; deletion is disabled without it, large reclaimable files are prioritized, chunks reclaimed independently, whole-file identity stays stable.
- Search: configured much/all accepted ingest is asynchronously auto-queued for embedding/backfill; ingest does not block; tenant-authorized built-in ID search is a platform contract. Existing model Snowflake arctic-embed-m-v2.0 remains distinct from vendor choice; second embedder remains owner-undecided.

## Remaining empirical gaps and defaults

- No actual laptop redb/Turso timing run or D1/R2 cloud run was performed. No latency winner, cost winner, placement recommendation, search recall result, or shard recommendation is available. Conservative default: do not recommend a storage placement; retain current per-surface authority until evidence exists.
- Cloud comparison needs approved credentials, spend cap, corpus snapshot/digest, region parity and native billing result. Local results are separate and cannot be generalized to cloud.
- Atlas #9092 producer source/config for the observed SHA256-of-UTF8-text content hash remains unlocated; the data observation is not a producer contract.
- #9084 must still provide the transactional reference-generation fence. If it cannot, GC deletion remains disabled.
- Immediate revocation requires a proven policy-epoch push path or synchronous authorization; absent either, cache serving is disabled.
