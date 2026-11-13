# #9143 research slice B11 — Diamond Types / Eg-walker

**Research scope:** `crdt-diamond`. Research-only. Diamond Types is a plausible referent for intake “diamonds”; project existence and fit do not prove Drew intended that project. Evidence checked 2026-11-12.

## Evaluation criteria

Data type and history; convergence; measured scale evidence and workload limits; signed-history fit; interoperability; authority boundaries. Compare text-only adapter, generic use, and non-adoption.

## Evidence and fit

The authors' paper identifies an optimized Rust Eg-walker collaborative text CRDT. It describes append-only operation history and reports text-workload performance results. This is useful for a possible text stream, not a generic federation/identity protocol. Agent IDs are identifiers, not authenticated signing identities. Reported benchmark figures must not be generalized beyond the paper's corpus.

## Sources

- [Authors' paper page, 2025-03-30](https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html)
- [Eg-walker paper repository](https://github.com/josephg/egwalker-paper)
- [Diamond Types crate](https://docs.rs/diamond-types/latest/diamond_types/)

## Options and disposition

1. Adopt as generic merge layer: reject; text-specialized data model does not justify this.
2. Benchmark a text-only adapter against Automerge for a measured text workload: conditional candidate.
3. Do not adopt: valid if no text workload demonstrates a meaningful advantage.

**Conclusion:** retain “Diamond Types” as an explicitly uncertain interpretation; no implementation or selection absent workload evidence and a separate signed-operation adapter.
