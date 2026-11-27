# C9138-01 — second-embedder research matrix

Issue: [#10157](https://github.com/developing-today/code/issues/10157). Sources retrieved **2026-11-27**; research-only checkpoint. **Decision: undecided; no second embedder enabled.** No inference, recall run, or vendor benchmark scores were produced or imported. No Cortex substitution.

## Decision and scope

Proceed with the independently useful cost/self-hostability matrix and validator rather than parking all research on #9092. Keep the existing Snowflake model and separate vector spaces. This is an agent's reversible scope decision, not an owner selection. #9113 is closed (GitHub checked 2026-11-27); #9092 remains open. Neither status supplies missing quality evidence.

The machine-readable record is `embedder-candidates-research-report.json`. It contains exactly the five issue-named identities; Voyage, OpenAI text-embedding-3 and Workers AI are comparison families/services, **not deployable exact pipeline identities**. Concrete variants below are examples for costing, not second-model selections.

## Source-backed comparison

All prices below are USD per **million input tokens**, standard hosted list prices before additional runtime/storage/retry costs, after any free allowances. These are not equal-corpus dollar measurements: tokenizers, truncation, corpus volume, re-embedding frequency and serving demand differ. No throughput/hardware run establishes self-hosted TCO.

| Exact comparison identity | Cost basis | Self-hostability | Mixed-corpus / integration boundary |
| --- | --- | --- | --- |
| Snowflake arctic-embed-m-v2.0 | Commercial model use free; infrastructure unmeasured [S1] | Apache-2.0 weights, local example [S1] | Multilingual text, 768-D, 8192 tokens; preserve current pin and query prefix. Not Cortex. |
| BGE-M3 | Own hardware/operations unmeasured, not free compute [S2] | MIT weights, local example [S2] | More than 100 languages; dense 1024-D, sparse and multi-vector modes; 8192 tokens. Extra modes need their own index contract. |
| Voyage | voyage-4-lite $0.02; voyage-4 $0.06; voyage-4-large/code-4 $0.12 [S4] | Hosted family also includes a distinct open-weight voyage-4-nano [S3]; nano license/sizing not evaluated | General/multilingual vs code-specialized choices; 4-family defaults 1024-D, optional 256/512/2048; 32000 context [S3]. Choose exact variant before integration. |
| OpenAI text-embedding-3 | small $0.02; large $0.13 [S6] | Retrieved guide establishes API hosting, not downloadable weights for these exact models [S5] | Text/code-search examples; small/large default 1536/3072-D, shortening available; 8192 max input [S5]. No corpus-specific quality conclusion. |
| Workers AI | Hosted @cf/baai/bge-m3 $0.0118 [S7] | Cloudflare-hosted service; self-hosting upstream BAAI weights is a different serving identity | Hosted page says 60000-token context [S7], upstream says 8192 [S2]. Preserve this discrepancy; validate endpoint semantics in #9092 before relying on either as a shared pipeline limit. |

**Quality for every row: unmeasured on the repository's mixed corpus.** Multilingual/code capabilities above are vendor descriptions, not a ranking. Equal dimensions, related weights, or vendor claims of within-family compatibility never authorize mixing these rows with existing Snowflake vectors.

## Actual dependency interfaces, not issue-state gates

Inspected at base `57ce9d543f8` (2026-11-27):

- `pkgs/atlas-search/worker/src/space.ts:19-22`: current concrete model `Snowflake/snowflake-arctic-embed-m-v2.0`, revision `95c2741480856aa9666782eb4afe11959938017f`, 768-D table `vec_snowflake_m_v2_768_f32`. The research display identity does not replace this production pin.
- `pkgs/atlas-search/crates/atlas-core/src/shards/mod.rs:393-421` and `stage.rs:115-139`: manifest has a single optional embedding model and staging rejects incompatible model identity. These are not a completed mixed-embedder pipeline registry.
- `part-a-addressing/versioned-design.md:25-36`: #9113 typed profile dispatch is an identity boundary, not proof of vector pipeline implementation.
- `part-c/versioned-design.md:39-41` and `implementation-slices.json:919-959`: downstream C9138-02 requires exact pipeline identity and idempotency. This slice does not implement it. Exact model/revision/preprocess/dimensions plus source identity/version must qualify any future vectors; ingest remains asynchronous and authority/signature/tenant checks precede indexing.
- `pkgs/atlas-search/book/design/evaluation-programme.md:66-72` names the standing golden set and evaluator. `crates/atlas-core/src/eval.rs:604-613` defines an `EvalReport` with metrics but **without corpus/eval digest fields**. Existing eval fixtures are not a #9092-authorized comparative artifact.
- `part-c/benchmark-plan.json:34-39,131` specifies a possible owner-artifact link contract; it is not an artifact. No #9092-produced corpus/eval-digest-bearing quality report was found in the inspected Part C / Atlas evidence paths. Thus `quality_artifact`, `corpus_sha256`, and `eval_set_sha256` are explicitly null. No claim of exhaustive absence outside the inspected repository.
- `crosscut/research-results/atlas-content-hash-producer.md` and `baseline/seam-map.md` preserve the Atlas observed content hash / typed-ID separation. A quality corpus digest must not be invented from a model pin, golden filename, or a content-ID algorithm.

## Validator contract and evidence boundary

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/validate_embedder_research.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/embedder-candidates-research-report.json
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/test_validate_embedder_research.py
```

Oracle: **research-matrix** (#10157, `implementation-slices.json:890-914`). Stdlib only. Required `--input PATH` / `--input=PATH` exactly once; argparse rejects unknown, abbreviated, missing-value and stray arguments before reads. No subprocess/network/inference in the validator. Closed report/candidate schemas reject score fields, substituted/duplicate/missing candidates, missing source links, fabricated decisions and enabled second-model flags. All 120 candidate permutations must preserve validity; generated deletion/duplication/substitution cases must reject. The validator checks structural evidence, not the semantic truth of prose or live vendor prices.

Optional quality import uses a **proposed local validation envelope**, not a claimed #9092 production format. A future `quality_artifact` object has relative `path`, byte `sha256`, and `owner_ref` pointing to #9092. Referenced JSON must declare `owner_issue: 9092`, `corpus_sha256`, and `eval_set_sha256` matching independently supplied report baselines. Those digests are compared, never recalculated as recall. Artifact metrics remain opaque; this report schema accepts no quality scores. Integrate the actual #9092 interface when published rather than assuming its evaluator currently emits this envelope.

Byte hashes establish integrity and digest equality, **not provenance authorization or corpus truth**. An `owner_issue` field and issue URL can be forged. The cited #9092 publication, baseline origins and artifact authorization require independent review; the validator does not fetch or authenticate them. Synthetic unit-test metadata exercises this proposed interface only and is not a quality artifact.

## Acceptance state / next slice

- Covered: all five identities, dated cost/self-hostability/capability sources, unchanged current model, explicit undecided status, absent quality evidence, strict structural oracle and rejection tests.
- Unmeasured: mixed-corpus retrieval quality, self-hosted throughput/TCO and exact cost per corpus.
- Unresolved: actual #9092 artifact schema/digests/publication, concrete family variants, hosted Workers context discrepancy, downstream mixed-pipeline implementation. No rollout or second-model choice is justified here.
- Next slice: after #9092 publishes an authorized artifact and independently anchored corpus/eval digests, adapt the import envelope and show that the same validator accepts the matching artifact and rejects either digest mismatch **without running recall**. This is an evidence dependency, not a request for an owner decision.
- Rollback: remove these research-only files or mark prices stale; existing Snowflake serving is untouched.

## Retrieved primary sources

- [S1]: https://huggingface.co/Snowflake/snowflake-arctic-embed-m-v2.0 — license, dimensions, local usage, context and query prefix.
- [S2]: https://huggingface.co/BAAI/bge-m3 — MIT, dimensions/context, local use, multilingual/dense/sparse/multi-vector capabilities.
- [S3]: https://docs.voyageai.com/docs/embeddings — hosted families and open-weight nano, variant dimensions/context/input_type. Page metadata updated 2026-08-10; retrieved 2026-11-27.
- [S4]: https://docs.voyageai.com/docs/pricing — current hosted standard token rates and separate batch/allowance rules.
- [S5]: https://developers.openai.com/api/docs/guides/embeddings — API usage, dimensions/shortening, maximum input and code-search examples.
- [S6]: https://developers.openai.com/api/docs/pricing — Specialized models / Standard embedding input rates.
- [S7]: https://developers.cloudflare.com/workers-ai/models/bge-m3/ — hosted model ID, unit rate and advertised context.

Every URL was retrieved successfully with the default read-only fetch on 2026-11-27. Sources are mutable vendor documentation, not pinned model/runtime contracts. Vendor benchmark scores visible on these pages were deliberately excluded.
