# R&D implementation plan and release gates

Start with a question an operator can answer from two receipts. Which source revisions were promoted, and which generation did this client actually search? The first receipt comes from ingestion. The second comes from the serving path and includes its effective scope. A backup timestamp, database size or successful HTTP response cannot answer both questions. The [roadmap and issue map](roadmap-and-issue-map.md) identifies owners and prerequisite issues. The [program tracker](https://github.com/developing-today/code/issues/13241) carries dated observations and dispositions.

## Work in dependency order

| Deliverable | Existing foundation | Acceptance |
|---|---|---|
| Corpus and consumer census | Canonical base tables, distinct publish files, manifests and CAS registry | Attach aggregate counts to a store role and read interval. Compare a consumer only against a matching scoped publish manifest. Report unknown source watermarks explicitly. |
| Replayable source ingest | Sweep core, gate records, local promotion, occurrence writer, stable source roots | A fixture survives duplicate delivery, a missed hook, a changed paragraph, deletion, and recovery after interruption. Independent reconciliation produces the same visible occurrences and vector input hashes. |
| Effective-scope retrieval | Branch filters, destination disclosure checks, job-scoped reads | A restricted synthetic exact match never displaces the best eligible candidate. Evaluate both independent branches and rendered results. Repeat after scope narrowing on warm cache, neighbor reads and stale replicas. |
| Planner and index experiments | Versioned query parameters, Snowflake exact scan, FTS, fusion and evaluation harness | Classifier runs in shadow first. Compare routed versus explicit filters, exact 768d versus 256d, and then ANN versus eligible exact search. Record cold and warm latency and per-class quality. |
| Ranking and personalization | Fused lists, feedback capture, optional Jaccard and snapshot paging | Separate candidate recall from ranking. Evaluate sparse, late interaction and cross-encoder arms individually. Test single-owner recommendation candidates against time-separated judgments before enabling a feed. |
| Fleet, ID and MCP | Fleet search, Worker search, reference/occurrence machinery and ID publication design | Demonstrate four distinct harness identities and a scoped search-to-read chain. Test ID publication and incoming ID object vectorization separately. Bind cache hits and subsequent pages to current grants. |

## Three acceptance fixtures

**Edited source with two rights domains.** Stage two synthetic documents containing identical bytes under distinct grants. Edit one occurrence and interrupt promotion before publication. Repeat the same delivery and reconcile from a complete source snapshot. The oracle independently reconstructs the visible revisions, verifies byte spans and counts changed embedding inputs. Revoke one occurrence and restore a pre-revocation snapshot. Its text and provenance must remain inaccessible after recovery. The surviving occurrence still resolves under its own grant. [Design: automatic ingest of all sources](automatic-ingest.md)

**A perfect match outside a destination's scope.** Build a small corpus with a restricted exact term/vector match, an eligible weaker match, and an eligible distractor. Request a pool of one through each branch. Verify the eligible winner against an exhaustive scalar reference over the allowed set. Repeat through each surface and a cached response, inspect branch admission and check the job-scoped read. A larger oversampling constant can conceal this failure on a particular corpus. It cannot establish the invariant. [Design: layered search characteristics on Snowflake](layered-search.md) · [Design: MCP and programmatic access, next steps](access-evolution.md)

**A recommendation page after a policy change.** Fix a parent snapshot from synthetic eligible candidates with ties. Fetch a page twice, restart the accumulator and fetch the next page. Check stable tie order and cursor binding. Then retire a candidate. Revalidate before replaying a page or serving a cached neighbor. Reject or rebuild the stale snapshot without disclosing the retired reference. Separately compare content-only and co-occurrence candidate recall with a recency baseline. [Design: the single-user recommender](recommender.md)

## Evidence that permits rollout

For each experiment record source SHA, corpus and policy generation, model and chunker fingerprints, candidate budget, baseline configuration, judgments, client scope and execution route. Publish a CI/Fleet receipt for synthetic inputs. Restricted evaluation remains on authorized infrastructure. Compare per-class relevance, no-answer behavior, candidate overlap, end-to-end latency and storage. Break out the denominator for each metric. Keep the Jaccard near-exact regression set separate from a new sealed holdout. [Design: the standing evaluation programme](evaluation-programme.md)

After promotion, verify a real authorized request against the new generation. For a failed arm, retain the old serving choice and record which requirement failed. During rollback preserve the latest revocation floor even when restoring an older index. The program's [dated design record](https://github.com/developing-today/code/blob/main/doc/2026-11-23T18-03-00Z_design_atlas_research_acceptance/README.md) provides the source adapter and experiment matrices.
