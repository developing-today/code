# Research: CDC feasibility at embedding scale (#6040)

**Decision: REJECT content-defined chunking (FastCDC) at the embed layer.
Keep the span-anchored approach of #6028. CDC at the storage/blob layer
remains a separate, still-open hybrid option (this report does not decide
it).**

Experiment: `cdc_experiment.py` (stdlib-only, deterministic; rerun it to
reproduce every number below). Results as run: `experiment-output.txt`.
Oracle: an unchanged document must score reuse == survival == 1.0 for
every chunker (asserted in the script's self-check, and it passes).

## Setup

- Corpus: this repo's own `doc/` + `planning/` Markdown, files >= 4 KiB —
  651 documents, 16,804,740 bytes. (Atlas's real ingestion corpus is
  documents of exactly this kind; the parent design #6028 is about
  ingesting them.)
- Chunk target ~2048 bytes average (~512 tokens at ~4 chars/token — the
  embed-layer scale named on #6040):
  - **fastcdc** — Gear-hash FastCDC, normalized chunking, min 1024 /
    avg 2048 / max 8192 (measured mean chunk 2521 B on this corpus).
  - **fixed** — fixed-size 2048-byte blocks (naive baseline).
  - **span** — span-anchored packing (approximation of #6028): split on
    paragraph boundaries, pack whole spans up to ~2048 B, never split a
    span unless it exceeds 8192 B.
- Revisions simulated per document, deterministically:
  - **insert15** — a 15-byte insertion at 5 spread positions (the
    structural-shift fragility case from the prior-art research);
  - **rewrite** — a ~200-byte region replaced at 3 spread positions.
- Metrics: within-corpus dedup ratio (unique-chunk bytes vs total);
  cross-version byte reuse (v2 bytes covered by chunks already present
  in v1 — under delta-only ingestion those bytes need no re-embed);
  chunk survival (fraction of v1 chunks unchanged in v2).

## Results

Within-corpus dedup ratio at ~2 KB scale: **0.000 for all three
chunkers.** On real document prose there are essentially no repeated
2 KB chunks to dedupe — CDC's headline benefit buys nothing at this
scale, for CDC or anyone else.

Cross-version results, per-document means (small documents dominate —
an insertion in a 5 KB document touches most of its chunks):

| edit | chunker | byte reuse | chunk survival |
|---|---|---|---|
| insert15 | fastcdc | 0.279 | 0.347 |
| insert15 | fixed | 0.000 | 0.000 |
| insert15 | span | 0.346 | 0.417 |
| rewrite | fastcdc | 0.454 | 0.529 |
| rewrite | fixed | 0.549 | 0.590 |
| rewrite | span | 0.554 | 0.607 |

Byte-weighted (pooled across the whole corpus):

| edit | chunker | byte reuse | chunk survival |
|---|---|---|---|
| insert15 | fastcdc | 0.521 | 0.556 |
| insert15 | fixed | 0.000 | 0.000 |
| insert15 | span | 0.546 | 0.593 |
| rewrite | fastcdc | 0.671 | 0.701 |
| rewrite | fixed | 0.755 | 0.764 |
| rewrite | span | 0.754 | 0.778 |

Sanity anchor (one 1,195,710-byte document, a single 15-byte insertion):
fastcdc reuse 0.998 / span 0.999 / fixed 0.003 — the FastCDC
implementation resynchronizes correctly; the corpus means above are a
small-document effect, not an implementation defect, and they are the
regime the embed layer actually lives in (most documents are small).

## Reading

1. **Dedup: nothing to buy.** At ~512-token chunks the corpus contains
   no duplicate chunks, so CDC's reason to exist at backup scale
   (~1 MiB chunks over near-identical snapshots) does not transfer.
2. **Boundary stability: span >= FastCDC >> fixed.** Under insertions
   both content-aware schemes resync and fixed blocks collapse to
   zero, as expected — but the span scheme matches or beats FastCDC
   without any rolling hash, because paragraph boundaries are already
   content boundaries in document prose.
3. **Local rewrites: span ~ fixed > FastCDC.** A rewrite costs each
   scheme the touched chunk; FastCDC additionally pays a resync chunk,
   and gains nothing back.
4. **CDC aligns bytes, not vectors** (the prior-art verdict): even a
   surviving chunk boundary says nothing about embedding drift, while
   span chunks carry semantic units by construction.

## Decision

**Reject FastCDC/CDC at the embed layer; the span-anchored chunking of
#6028 stands.** This confirms, with our own corpus numbers, the
prior-art research verdict the issue was filed to verify. The hybrid
option remains open where it was always plausible: CDC at the
storage/blob layer (backup-scale chunks over blobs), which is a
separate decision for the storage work, not this layer.

— fleet research run for #6040, task 15258, lane code-d, 2026-11-14.
Corpus at repo HEAD f2df19ed32; script + results in this directory.
