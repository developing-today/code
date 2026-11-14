# #9143 research slice B12 — Willow

**Research scope:** `crdt-willow`. Research-only. Sources checked 2026-11-12; proposal status is retained.

## Evaluation criteria

Namespace/subspace data model; authorization; convergence; anti-entropy efficiency; bounded partial sync; signed-only ingest fit; interop maturity and versioning.

## Evidence and fit

Willow entries carry namespace, subspace, path, claimed timestamp, payload digest/length and authorization token; its join semantics support convergence. Willow'25 specifies Ed25519/Meadowcap parameters. Confidential Sync describes scoped, partial and resource-limited reconciliation, but its page calls the work a proposal, paused and outdated. Claimed timestamps are not independent wall-clock evidence. These are useful concepts, not a drop-in ID contract.

## Sources

- [Willow Data Model](https://willowprotocol.org/specs/data-model/)
- [Willow'25 parameters](https://willowprotocol.org/specs/willow25/index.html)
- [Confidential Sync proposal/status](https://willowprotocol.org/specs/confidential-sync/)

## Options and disposition

1. Adopt Willow directly: reject absent compatibility and protocol maturity proof.
2. Borrow scoped range-reconciliation ideas behind ID adapters: conditional research prototype.
3. Retain existing conflict-report-first sync and adopt none: valid until scale data warrants another protocol.

**Conclusion:** test reconciliation only against scoped retention/deletion/tenant cases; preserve signatures and reports independently. No production protocol selection here.
