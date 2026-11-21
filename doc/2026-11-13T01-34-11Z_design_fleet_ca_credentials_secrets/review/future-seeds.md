# Independent re-review: future idea seeds

**Disposition: AGREE.** Re-read the updated SEED-61 packet and verified its changed proposal file hash against the author-provided value:

- `proposals/idea-seeds.json`: `2f95de3fbcd5c2b85769420ad3ef8865ddc85dc409a060e6fc47f9297a35f6d7`
- `research/future-seeds.md` remains `2e4fd28037b2063b63b6a3e991f32e8c4f0c0fade1f50b1cb4ffe48c0a38a9cf`.

SEED-61 now cites spec §3.1, pins the approved tuple and synthetic request fixture to `signature_algorithm=ML-DSA-44`, uses an explicitly pinned independent verifier, and makes unsupported/inconclusive the outcome if that verifier cannot validate the baseline. It forbids substituting another PQ signature. This resolves the remaining blocker from my prior review.

The prior findings remain addressed: air-gapped signing lifecycle has a bounded owner; optional/approval-gated hard dependencies were removed; the public ACME DNS-01 branch is conditional; SEED-31 fixes ML-DSA-44; and the trust-install mapping and per-packet §4.2 metadata/materialization contract are explicit. I found no other blocker in this re-review. No source or proposal file was changed; only this review file records the disposition.
