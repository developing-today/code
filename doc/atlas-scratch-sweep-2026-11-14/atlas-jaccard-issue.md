# Atlas search: Jaccard as a third signal (rescore + ingest dedup); SPLADE as a triggered follow-on

Context: the atlas hybrid design settled 2026-11-04 (see the shard-restore work under #5669 and the spike verdict on hatch-personal#20): retrieval = **dense cosine scan + FTS/BM25**, one fan-out per shard carrying both scorers, merged with reciprocal rank fusion. Dot product adds no independent signal on normalized embeddings. Drew (2026-11-04) asked where Jaccard fits in atlas specifically — this issue records the answer as a design task.

## Proposal: Jaccard as a THIRD signal, at rescore — not at retrieval

- Each chunk gains a stored **token-shingle set** (hashed word shingles) alongside its embedding and text.
- Retrieval stays two-scorer (cosine + BM25). Jaccard is computed **only over the candidate union** (the top ~100 merged candidates), as a rescore-stage feature:
  - **Near-exact boost**: a candidate whose shingle overlap with the query is very high is probably _the_ text the user pasted or quoted (an error message, a passage, a title) — cosine alone ranks it "topically similar among many"; Jaccard identifies it as _the match_. Boost it decisively.
  - **Tie-breaker / confirmation**: among semantically-equal candidates, prefer higher literal overlap when the query contains rare exact tokens.
- Why not a full-corpus third scorer: set intersection per row across every shard is real scan cost for a signal that only matters near the top. Rescore-stage keeps it ~free.

## Second use: ingest / promotion dedup

- At ingest (and at staging→core promotion, #5669), compute shingle overlap of a new chunk against recent / same-source chunks: **≥ ~0.9 Jaccard = near-duplicate** — flag or suppress instead of silently double-indexing. The corpora most at risk: brain-dump explosions, repeatedly-versioned design docs, re-ingested repos.
- Dedup findings are data, not deletions: record the edge (chunk, near-dup-of, score) and let promotion policy decide; never auto-delete on a similarity score alone.

## SPLADE — investigated only on a trigger (Drew, 2026-11-04: "no reason to investigate SPLADE?")

There IS a reason — quality, not speed: learned sparse retrieval can beat BM25 where vocabulary mismatch hurts (the model expands terms with learned weights), and it would collapse hybrid retrieval into a single vector scan (TursoDB even has an experimental sparse-IVF index). The costs are also real: a SPLADE encoder must run at **ingest and at query time**, a model must be hosted, and our corpora are technical text where exact terms dominate — BM25's home turf.

So: **triggered follow-on, not a standing task.** Investigate SPLADE only if the shipped hybrid's held-out evaluation shows the lexical half is the weak side (e.g. recall@10 on the eval set improves materially when an oracle lexical ranker is substituted). Bar for adoption: a measured recall@10 gain over the BM25 half on the held-out query set, large enough to justify a permanent inference dependency at ingest + query time. Until the trigger fires, this stays a named paragraph, not a project.

## Acceptance shape

- Design note covering: shingle construction (size, hashing, storage cost per chunk), the rescore formula (how the Jaccard feature enters the fused ranking), dedup thresholds + the promotion-policy hook, and the eval-set methodology that would (a) prove the rescore signal helps and (b) arm the SPLADE trigger.
