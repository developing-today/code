# Change the embedding model

Source: `pkgs/atlas-search/crates/atlas-core/src/types.rs`, `pkgs/atlas-search/crates/atlas-core/src/shards/mod.rs`, `pkgs/atlas-search/crates/atlas-core/src/stage.rs`, `pkgs/atlas-search/crates/atlas-embed/src/lib.rs`, `pkgs/atlas-search/worker/src/space.ts` and `pkgs/atlas-search/eval/fixtures/golden-311.json` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/types.rs — provenance only; this chapter is complete without following it.

An **embedding model** turns text into a vector — a list of numbers —
such that similar texts land near each other; the vector branch of
search is distance in that space. The load-bearing consequence: vectors
from different models are **not comparable**. They do not live in the
same space, distances between them are meaningless, and no normalisation
fixes that. Almost every rule in this chapter is that sentence applied.
The current model, its space and the parity contract are covered in
[Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md);
this chapter is the migration discipline for replacing it.

## The identity of a model: three spellings, one model

One model is named in three places, in two spellings, and no code
converts between them. A model change must update all three, in step:

| Where                                                                                                                                                            | Constant             | Current value                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------- | ----------------------------------------------------------------------------- |
| atlas-core's types (and atlas-proto's default, and the CLI's `--model` default)                                                                                  | `DEFAULT_MODEL`      | `Snowflake/snowflake-arctic-embed-m-v2.0` — the HuggingFace org/name spelling |
| atlas-core's shard module — the value actually **stamped into shard manifests and staging-database manifests**, and compared at discovery, fan-out and promotion | `EMBEDDING_MODEL_ID` | `snowflake-arctic-embed-m-v2.0` — no org, lowercase                           |
| The Worker's pinned space (`worker/src/space.ts`)                                                                                                                | `EMBED_MODEL`        | `Snowflake/snowflake-arctic-embed-m-v2.0` — the HuggingFace spelling again    |

Around them sit the rest of the pinned identity, all of which moves with
a model change: the pinned revision (`95c2741480856aa9666782eb4afe11959938017f`
today — recorded in `fetch-model`'s file list, the fixture provenance and
the Worker's `MODEL_REV`), the output width (`EMBED_DIM = 768` in
atlas-embed; `EMBED_DIMS` in the Worker), and the vector table name
(below). A shard stamped by the aggregator can never string-equal
`DEFAULT_MODEL` — the two spellings are compared only within their own
subsystems — so "I updated the model constant" is a claim that must name
**which** constants, all of them.

## Vectors are tagged, and spaces are separate tables

Every stored vector set belongs to exactly one model, and the tag is
structural, not a comment:

- Vectors live in a **per-model table**, named for its space:
  `vec_snowflake_m_v2_768_f32` for the current model (768 dimensions,
  f32), `vec_bge_small_384_f32` for the dormant one (below). A new model
  gets a **new table** — a new vector space is never written into an
  existing model's table, and a table is never re-interpreted under a
  new model.
- Shards and staging databases carry the identity in their manifests
  (`atlas_manifest.embedding_model`), merge manifests record the model
  that produced them, and query traces record the producing shard's
  model per result — two or more distinct models among one search's
  candidates set `embedding_model_breach = true` on the trace.
- The enforcement points refuse mixtures, fatally: shard discovery
  refuses a shard whose manifest names a different model (and, in
  strict mode, one that names none); `fanout_scan` re-checks the scanned
  set itself and fails a mixed merge; the M6 promotion gate refuses a
  unit whose recorded model is not the mandated one. See
  [Add an ingest source](add-a-source.md).

## The migration procedure

Migration is additive-first, in this order. No step is skippable, and
the order is the safety property.

1. **Pin the new model exactly.** Repository, revision, and the sha256
   of every file `atlas-search fetch-model` will install
   (`tokenizer.json`, `onnx/model.onnx`), recorded in the repo. The
   model is data, provisioned into the state directory
   (`~/.local/state/atlas-search/model/<rev>/`) by `fetch-model` with
   hash verification and an atomic rename into place — never a nix
   store artifact, never a download at search time. A missing or
   mismatched model fails loudly, and FTS-only search and `get-chunk`
   keep working with no model present.
2. **Port the embedder, and pass Gate A.** atlas-embed must reproduce
   the new model's reference pipeline bit-identically on a fixed probe
   set — token ids equal, output vectors delta 0.0, single and batched —
   with the probes recorded by a generator the way
   `fixtures/embed-probes.json` was. Gate A verdicts are
   version-matched: the gate reads the ONNX Runtime version from the
   library and refuses to judge against a different one (a different
   runtime produces ~1e-7 deltas and the gate reports NOT APPLICABLE,
   not a pass).
3. **Build the new space beside the old one — the shadow index.** Embed
   the corpus into the new table and the new shards, from the same
   chunk text, while production keeps serving the old space untouched.
   The index is derived and disposable; the corpus is neither. The
   shadow space is evaluated as its own shard set — never merged with
   the old set for a "combined" look, because a mixed fan-out is
   exactly what the machinery refuses.
4. **Evaluate on the golden set.** Score the shadow space against the
   golden set (golden-311: 311 judged queries over the frozen corpus)
   through the evaluation harness, against the production space's
   recorded row for the same set — the current production hybrid's best
   measured row is 189 hits@10 / MRR 0.3894 at RRF k=10, pool 100 per
   branch. Parameter selection happens on the **dev** split only; the
   harness's `select_best_signal` refuses a test-split report, and the
   test split is scored once, as the final pass. Gate B is the tie
   requirement against the reference implementation's production path;
   the programme is [Evaluation: golden sets and gates](../concepts/evaluation.md).
5. **Cut over on the evaluation result — not on completion.** Finishing
   the embedding pass is not the decision. The decision is the gate
   result, and Gate C states it as a rule: the A/B winner is the
   implemented default. If the new space wins, cutover is a defaults
   change, not a data migration: the three identity spellings, the
   revision, the dimension, `DEFAULT_VEC_TABLE`, the service
   configuration's `vec_table`, and the Worker's space pins move
   together, and the service restarts onto the new space (see
   [Release and rollout](release-and-rollout.md)). If it does not win,
   the shadow space is deleted or kept as a recorded experiment — and
   production never moved.
6. **Mind the caches; they mostly mind themselves.** A query-embedding
   cache key is a pure function of (normalized query text, model id,
   model revision), so a model change changes every key — no TTL, no
   invalidation pass. The Worker cache key likewise carries the model
   revision and vector table, so new-space entries are new entries and
   old ones are orphaned, never served across spaces. A re-shard
   changes the shard set's snapshot id, which invalidates every cached
   service result at once. Verify these three properties hold for the
   new model before cutover; do not add a manual purge step in their
   place.

## What happens to the old space

The old space is **kept, dormant, at first** — the migration phases are
additive-first and nothing is removed before its replacement is proven.
But dormancy needs a retirement decision with a date, or it becomes
permanent. The cautionary example is already in the corpus:

**The BGE-small dormancy.** The previous space,
`vec_bge_small_384_f32` (model `BAAI/bge-small-en-v1.5`, 384 dimensions),
remains in the publish database as a dormant fallback to this day. Its
afterlife, in full: the string naming its table appears **only** in the
Python reference — no Rust code names it, so no Rust surface can be
pointed at it deliberately; the Worker explicitly refuses its vectors
(a 384-dimension BGE-small vector "is not comparable with the corpus"
under the pinned-space check); its side-store files still sit in the
laptop working tree; and the local backend can still be configured to
read whichever vector table it is told to, so the dormant space is one
configuration value away from being served by accident. Nothing about
it is broken, and nothing about it is decided — it occupies the publish,
invites confusion about which space is real, and has no owner for its
removal.

So the last step of a model migration, scheduled at cutover time, is
the old space's disposition, recorded in the change: retained as a
named fallback with the surfaces that may use it stated, or removed —
table, side stores, configuration references and book mentions
together. "Left in place, undecided" is the one outcome this chapter
exists to prevent.
