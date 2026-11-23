# Tutorial: stage, classify and promote a source

This tutorial walks one tiny toy source — three invented notes about model sailboats — through the ingest path that exists today for staged content: a staging unit (a per-unit staging database registered in the staging registry), classification declared at staging time, inspection, and promotion through two distinct gates into a new core scan shard (a searchable shard of embedded chunks). The concepts are in [Ingest today: staging, classification, promotion](../concepts/ingest.md); the verb reference is [atlas-stage](../reference/atlas-stage.md). Everything here is toy content invented for this tutorial; no real corpus material is used.

Two gates will be named, and they are not the same gate:

- The **M6 model gate** (#7648, part M6 of the #6060 one-model decision): a unit promotes only when its recorded embedding model is the mandated model, `snowflake-arctic-embed-m-v2.0`.
- The **batch gate** (#6057 design, #7709 record and state machine): five ordered stages — write, classify, dedup, provenance check, publish — judged per batch, with a durable audit record.

Names used throughout: unit `toy-sailboats-001`, owner `tutorial`, registry file `toy-registry.db`, staging database `toy-sailboats.db`, shard directory `toy-shards/`. All `atlas-stage` output is one JSON object per line on stdout; errors print `atlas-stage: &lt;message&gt;` on stderr and exit 1 (usage errors exit 2).

## Step 1: write the toy source

1. Create three small note files. Keep each well under 32 KiB — at or below that size an artifact stages as one whole-file unit.

   ```sh
   mkdir -p toy-notes
   printf 'Toy note one: a model sailboat stays upright because its ballast sits low in the hull.\n' > toy-notes/ballast.md
   printf 'Toy note two: a loose rudder makes the sailboat wander off course on the pond.\n' > toy-notes/rudder.md
   printf 'Toy note three: waxing the hull before a race is a superstition, not a technique.\n' > toy-notes/hull-wax.md
   ```

## Step 2: create the stage unit

2. Provision the staging database, apply the v1 staging schema, and register the unit as `open`:

   ```sh
   atlas-stage provision --db toy-sailboats.db --unit toy-sailboats-001 \
     --owner tutorial --registry toy-registry.db
   ```

   The default physical sensitivity tier is `internal` (the staging tier vocabulary is `public` | `internal` | `secret` — not the four-level sensitivity ladder). The command prints the unit record: `unit_id`, `db_uri`, `owner`, `sensitivity`, `state` (`open`), `schema_version` (1), and timestamps. Staging databases are vector-free by design: they hold `items` and raw, un-embedded `chunks_raw`, never vectors.

## Step 3: add the items

3. Stage each note as an item. Every ingest names the artifact's origin — repository, path inside it, the local file, and the commit the bytes were read at:

   ```sh
   atlas-stage ingest --db toy-sailboats.db --unit toy-sailboats-001 \
     --repo toy/sailboats --path notes/ballast.md --file toy-notes/ballast.md \
     --commit 0000000000000000000000000000000000000001 --kind prose
   ```

   Repeat for `rudder.md` and `hull-wax.md`. Each prints `{"item_id": …, "unit": "toy-sailboats-001", "source": "toy/sailboats:notes/ballast.md@0000…"}`. Writes at this stage are unmediated — sensitivity is recorded, not enforced — because nothing staged is searchable yet. Enforcement happens at classification and at the gates.

## Step 4: run classification

Classification in this surface is _declared by the producer and composed by staging_: you supply the source-native classification segments — byte ranges of the artifact, each with a sensitivity from the four-level ladder — and staging splits the artifact at those boundaries and persists one classification record per chunk. Classification may only change at a declared boundary; undeclared or uncertain text fails closed.

4. Write a segments file for the ballast note. This toy file declares its whole text as one `personal` segment (byte offsets are illustrative; a real producer computes them from its own boundary detection):

   ```json
   [{ "start": 0, "end": 86, "sensitivity": "personal", "certain": true }]
   ```

   Save it as `ballast.segments.json`. Segment files are parsed strictly: a missing or mistyped field, or an unknown sensitivity name, is an error — never a silent default.

5. Re-stage the note as a classified artifact by adding `--segments`:

   ```sh
   atlas-stage ingest --db toy-sailboats.db --unit toy-sailboats-001 \
     --repo toy/sailboats --path notes/ballast.md --file toy-notes/ballast.md \
     --commit 0000000000000000000000000000000000000001 --kind prose \
     --segments ballast.segments.json
   ```

   The chunks are now boundary-safe — they partition the source exactly and never cross the boundary the classification was cut on — and each chunk carries its persisted classification record (actor `atlas-stage`, method `boundary-split`). A record that later reads back malformed or unrecognized resolves to `secret-bearing`: fail closed, every time. The corpus-wide remainder sweep that classifies _unclassified_ content in an existing corpus database is a different tool — [atlas-classify](../reference/atlas-classify.md) — and is not run against a staging database.

## Step 5: inspect what is staged

6. List the unit's items:

   ```sh
   atlas-stage items --db toy-sailboats.db --unit toy-sailboats-001
   ```

   One JSON object per item: `id`, `unit_id`, `kind`, `source`, `sensitivity`, `chunks`, `meta`. Check the chunk counts and sensitivities are what you declared before going further — promotion is the point of no return for this unit.

7. Check the registry's view, and which units are still open:

   ```sh
   atlas-stage lookup --unit toy-sailboats-001 --registry toy-registry.db
   atlas-stage units --registry toy-registry.db --state open
   ```

   An unknown unit in `lookup` is an error (`no staging unit "&lt;unit&gt;" in the registry`, exit 1) — check the spelling against `units` output rather than re-provisioning.

## Step 6: promote through the gates

Promotion is the aggregator's embed-at-promotion step, and it is gated twice over.

8. Run the promote verb. It needs the embedding model and the ONNX runtime, resolved exactly as `atlas-search` resolves them (`ATLAS_MODEL_DIR` / `ATLAS_ORT_LIB`); a missing model or runtime is a loud exit-1 error, before anything is written:

   ```sh
   atlas-stage promote --registry toy-registry.db --unit toy-sailboats-001 \
     --shard-dir toy-shards --topic codeconfig
   ```

   What happens, in order: the unit's staged chunks are read in staging order; oversize chunks are split at line boundaries into context-fitting pieces that partition the text exactly (identical piece texts are embedded once); every piece is embedded document-side, prompt-free, with the mandated model; a new core scan shard is written into `toy-shards/` with its self-describing manifest stamped with the embedding-model identity; and the unit's own database is stamped with that provenance. Default `--topic` is `codeconfig`; pass your own for a notes batch.

9. Understand the **M6 model gate**, which the registry transition then applies: the unit promotes only when its database's recorded `atlas_manifest.embedding_model` equals the mandated model (`EMBEDDING_MODEL_ID` = `snowflake-arctic-embed-m-v2.0`). This is the one-model rule — vectors from different embedding models are not comparable, so a shard of unknown or wrong-model provenance must never join the searchable set. On success the registry writes the merge manifest (rows in, rows promoted, content hash, embedding model, timestamp — the durable audit record), branches the staging database as the unit's archive, and moves the unit to `merged`. The promote verb prints `{"unit": …, "shard": …, "shard_path": …, "items": …, "chunks": …, "dims": …}` — expect `dims` 768.

10. Understand the **batch gate**, the second gate, which judges a batch (a staging unit, or one increment of a long-lived unit) through five ordered stages: **write → classify → dedup → provenance check → publish**. Its record — persisted in the registry's `gate_records` table — holds per-stage verdicts with input hashes, row counts, and row ids, never content. The rules that matter here:

    - A stage with no recorded verdict has not passed, and verdicts are recorded strictly in order — no stage can be judged before its predecessor passed.
    - Each failure carries exactly one enumerated reason code, bound to its stage: `write_invalid` / `write_bypass` (write), `classify_unresolved` (classify — this is where undeclared or contradictory classification stops a batch), `dedup_undispositioned` (dedup), `provenance_unresolvable` (provenance check), `publish_partial` (publish). A `skip` verdict exists only where the design allows it (write on an empty batch; publish with zero survivors and nothing pending), always with its own reason, and never yields a publish manifest.
    - Batch states run `open → closing → gated → published`, or `quarantined`. The same reason code three times (`STRIKE_LIMIT` = 3) marks the batch `needs_review` and stops automatic retries. A retry resumes at the failing stage — except a write failure, which re-enters at write — and if an earlier stage's input changed, every verdict from the earliest changed stage on is discarded: changed bytes never inherit a verdict obtained for different content.

## Step 7: verify the content is searchable

11. Confirm the unit left the active states:

    ```sh
    atlas-stage units --registry toy-registry.db --state merged
    ```

    The unit appears here, and no longer under `--state open`. Promotion also closed the staging database to further writes: the provenance stamp adds the manifest table, so the database no longer passes the exact five-table v1 schema check — staging into it again is refused by design.

12. Search for the toy content against the new shard. The shard fan-out is experimental and off by default; point `atlas-search` at the shard directory explicitly, in vector mode (the fan-out serves the vector branch from the shards; full-text stays on the monolith backend, which has never seen these notes):

    ```sh
    atlas-search search 'how does a toy sailboat stay upright' \
      --shards dir:toy-shards --mode vector --k 5
    ```

    The ballast note's chunk should be the top hit. In production the promoted shard joins the service's discovered shard set by existing — discovery reads each candidate shard's manifest and cross-checks its model identity — after which the same query through the ordinary surfaces finds it too. See [Tutorial: search the corpus three ways](search-the-corpus.md).

## What a refused promotion looks like

A refused promotion is loud and leaves the unit unpromoted — there is no partial state in which some chunks became searchable. `atlas-stage promote` exits 1 with `atlas-stage: &lt;message&gt;` on stderr. The M6 refusals are: a wrong model recorded (the message is the comparability rule — vectors from different embedding models are not comparable), no model recorded (provenance unknown), and an empty unit. A batch-gate refusal instead leaves a gate record: the batch sits in `quarantined` with the failing stage's reason code recorded, its staging database untouched (quarantine is a state, not a deletion), and after three strikes on one reason code it is flagged `needs_review` for a human. Fix the cause — re-declare the classification, repair the provenance, re-stage the content — and retry; the gate resumes at the failing stage.

## What dead letters look like

Dead letters belong to the sweep surface (`atlas-sweep`), not to `atlas-stage`: in the periodic source sweep, a candidate that fails three times (`DEAD_LETTER_STRIKES` = 3) is dead-lettered rather than retried forever. They are operated on directly:

```sh
atlas-sweep --state-dir &lt;state-dir&gt; dead-letter list
atlas-sweep --state-dir &lt;state-dir&gt; dead-letter retry &lt;ref&gt;
atlas-sweep --state-dir &lt;state-dir&gt; dead-letter drop &lt;ref&gt;
```

`list` prints one reference per line (an empty store prints nothing and exits 0); `retry` and `drop` are silent on success. The distinction to keep: a gate quarantine is a batch held _before_ publish with its audit record attached, resumable at the failing stage; a sweep dead letter is a source candidate set aside _by the scheduler_ after repeated failure, resumable only by an explicit `retry`. Neither deletes anything.
