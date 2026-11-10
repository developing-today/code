# Atlas secret-tier local embedder: model selection, fleet-hardware evaluation, no-egress invariant

Issue: [#6059](https://github.com/developing-today/code/issues/6059)
Parent: [#6019](https://github.com/developing-today/code/issues/6019)
(semi-automated ingestion operations). Research source: prior-art doc
`~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md`
(2026-11-04), Cluster 7. Sibling designs: embedding cost ledger
[#6058](https://github.com/developing-today/code/issues/6058) (landed as
`doc/2026-11-09T01-07-20Z_design_atlas_embedding_cost_ledger`), delta
protocol [#6028](https://github.com/developing-today/code/issues/6028),
per-batch attestation [#6045](https://github.com/developing-today/code/issues/6045).
Status: design only. No implementation is landed or claimed by this
document.

## The invariant (hard, not a preference)

**Secret-tier text is embedded locally or not at all.** No route —
primary, fallback, retry, batch-API discount, or "temporary" capacity
borrow — may send secret-tier text to a hosted embedding API. This is
a classification rule, not a cost rule: a hosted call carrying secret
text is a classification violation at any price, including free. The
internal tier is under no such constraint and uses whichever route
the #6058 cost ledger favors.

Corollaries, all fail-closed:

1. **Routing is decided by the tier tag, in one place.** The embed
   router holds a route table keyed by classification tier. The
   secret tier's table has exactly one entry (the local embedder)
   and no fallback list. A missing local route is an error, never a
   reason to consult another tier's table.
2. **Embedder-down means ingest queues.** When the local embedder is
   unavailable, secret-tier batches wait in the ingest queue with
   their tier tag intact. Nothing reroutes, downgrades, or samples
   the text to a hosted endpoint "to check" — a probe payload is
   still secret text.
3. **Every secret-tier embed is attributable.** The #6045 attestation
   record and the #6058 ledger row for a secret-tier batch name the
   local model id as `model`. A secret-tier ledger row naming a
   hosted model is a stop-the-line audit finding, detectable by a
   ledger scan alone.

## Decision

1. **Candidates, in evaluation order.**
   - **The fleet's current local model family (Snowflake Arctic
     Embed via sentence-transformers)** — already operational in
     atlas-search (`pkgs/atlas-search`, query embedder with
     `prompt_name='query'`), already behind the Gate A parity
     machinery (spike #5351). Incumbency is a real advantage:
     cross-tier comparability (below) is cheapest when the secret
     tier's space is close to a space the fleet already serves.
   - **BGE-M3** — the issue's named alternative: multilingual,
     1024-dim, long-context, self-hosted cost ≈ $0.001/1M tokens at
     high GPU utilization (#6058's verified figures). The fallback
     if the incumbent fails the parity bar.
   - **BGE-small (legacy local set)** — rejected without evaluation:
     it is the dormant fallback table in atlas-search today, kept
     for rollback, and its quality gap against both candidates is
     not worth a third evaluation track.
2. **Selection is by golden-set parity first, throughput second.**
   The golden set (sibling work under #6027's retrieval-statistics
   family) is embedded by (a) the internal tier's hosted model
   (text-embedding-3-small, the mcpx default route) and (b) each
   local candidate. Parity is measured on retrieval agreement —
   top-K overlap and rank correlation on golden queries — because
   the property the system actually needs is **cross-tier
   comparability**: a secret-tier result and an internal-tier result
   must be meaningfully rankable side by side, or the tiers become
   two silos that happen to share a schema. A candidate that misses
   the parity bar is out regardless of speed; among candidates that
   clear it, the faster on fleet hardware wins. Proposed bar, to be
   ratified with the #6027 sibling: top-10 overlap ≥ 0.8 against the
   hosted model on the golden query set, measured per source class.
3. **The evaluation runs on fleet hardware, at delta batch shapes.**
   The benchmark host is the fleet's own ingestion machine (the
   laptop, amd-6) and any GPU host already in the fleet — no cloud
   benchmarks, no rented cards: the number that matters is tokens/s
   and p95 latency on the hardware that will actually run it, at the
   batch sizes the delta protocol (#6028) actually produces. Delta
   workloads are bursty and small (tens to hundreds of new chunks per
   batch, ≈27–54M tokens for a *full* corpus re-embed per #6058), so
   the benchmark reports cold-start + small-batch latency, not just
   peak throughput at saturation. A candidate that needs a warm,
   always-resident GPU to hit acceptable latency fails this section
   by construction (see dead ends).
4. **Operational home: a batch worker on the ingestion host, not a
   daemon.** The secret-tier embedder runs as a queue-driven batch
   worker co-located with ingestion on amd-6: it wakes on queued
   secret-tier batches, loads the model, embeds, writes vectors +
   attestation + ledger rows, and exits. No resident GPU process, no
   new network listener — the worker reads the same ingest queue the
   hosted route reads, which keeps the tier split a routing decision
   (section: the invariant) rather than a second pipeline. If the
   golden-set winner cannot meet small-batch latency on CPU/on-call
   GPU on amd-6, that is an evaluation result to record and escalate,
   not a reason to move the tier off fleet hardware.
5. **Failure posture is queue-and-say-so.** Embedder down, model load
   failure, or parity-gate failure (Gate A drift of the #6663 kind —
   ~1e-7 nondeterminism across ORT versions) all produce the same
   behavior: secret-tier batches stay queued, the queue depth is
   visible in the #6058 ledger's absence (batches stop publishing),
   and an operator-visible error names the local route as down. The
   one forbidden behavior is silent substitution.

## Named dead ends (do not re-propose without new evidence)

- **Always-on GPU for bursty embedding.** ~$1,080/mo spot A100 at
  zero usage against pennies of batch API for the internal tier —
  and for the secret tier the comparison is not even available,
  since the API is forbidden at any price. Bursty work gets a
  batch-shaped worker (Decision 4).
- **Hosted "private" / zero-retention endpoints for secret text.**
  Contractual retention promises do not change the classification
  fact: the text left the fleet. The invariant is about egress, not
  about trusting a vendor's deletion pipeline.
- **Embedding secret text on a borrowed general-purpose session**
  (an LLM session's embedding side-channel, a search worker's remote
  mode). If the process can reach a hosted model, it is a hosted
  route. The route table (Corollary 1) is the only path.

## What this design does not decide

- The internal tier's route (the #6058 ledger's business, incl. the
  #6069 Modal batch question).
- The golden set's construction and the final parity threshold —
  owned by the #6027 sibling; this design names the metric shape
  and a proposed bar.
- Whether Gate A's bit-identity requirement applies to the secret
  tier or the #6663 tolerance finding governs — decided with the
  Gate A owners when the evaluation runs, and recorded as an
  amendment here if it changes Decision 2's procedure.

## Evaluation checklist (the implementation follow-on, in order)

1. Assemble the golden set with the #6027 sibling; freeze it.
2. Embed it via text-embedding-3-small (internal reference) and
   both local candidates; report top-K overlap + rank correlation
   per source class.
3. Benchmark both candidates on amd-6 at delta batch shapes
   (16 / 64 / 256 chunks): cold start, tokens/s, p95 latency.
4. Record the winner + the numbers as an amendment to this design;
   the loser stays named with its numbers (no silent drop).
5. Implement the tier-keyed route table + batch worker behind this
   design; the first secret-tier ledger row naming the local model
   is the acceptance evidence.
