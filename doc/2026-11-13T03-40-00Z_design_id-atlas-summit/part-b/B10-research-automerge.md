# #9143 research slice B10 — Automerge

**Research scope:** `crdt-automerge`. Research-only; no adoption implied. Evidence checked 2026-11-12; this is a scoped synthesis of the source ledger in `research.md`.

## Evaluation criteria

Data model/history; convergence; scale/performance limits; signed-history compatibility; cross-language/format interoperability; authority boundary. Compare immediate adoption with a later adapter and with no adoption.

## Evidence and fit

Automerge documents are histories of changes; deterministic merge rules retain inspectable concurrent conflicts. The transport-agnostic sync protocol is per document and its Rust API documents reliable in-order stream assumptions. Structured collaborative documents are a plausible fit; ID arrays/triples and generic federation are not equivalent data models. Actor identifiers do not establish authenticated signer identity; signed envelopes must remain an independent layer.

## Sources

- [Automerge Concepts](https://automerge.org/docs/reference/concepts/)
- [Automerge merge rules](https://automerge.org/docs/reference/under-the-hood/merge-rules/)
- [Automerge Rust sync API](https://automerge.org/automerge/automerge/sync/index.html)

## Options and disposition

1. Adopt now: rejected because no measured workload/scale case and signatures/authority remain external.
2. Isolated adapter after V2 conflict reports and workload benchmark: conditional candidate.
3. Do not adopt: valid result if measurements do not justify history/merge overhead.

**Conclusion:** preserve conflict-report-first; revisit only with representative structured-document workload, memory/CPU/wire benchmarks, and signed-origin interoperability tests. No merge code is in scope.
