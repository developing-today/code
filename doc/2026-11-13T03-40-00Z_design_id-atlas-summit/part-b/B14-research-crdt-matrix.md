# #9143 research slice B14 — common CRDT/prior-art matrix

**Research scope:** `crdt-common-matrix`. Research-only synthesis of B10–B13; complete outcome may be “adopt none.” Evidence checked 2026-11-12.

## Common criteria and matrix

| Candidate | Data/history | Convergence/sync | Scale evidence | Signed history / interop | Authority boundary |
|---|---|---|---|---|---|
| Automerge | structured document change history | deterministic merge, per-document sync | binary history and project benchmarks; workload-specific | documented formats/bindings; actor ID is not signer auth | no peer authorization |
| Diamond Types | text-oriented Eg-walker operation log | collaborative text merge | paper reports text workload results only | Rust project; agent ID is not signer auth | no federation authority |
| Willow | scoped namespace/subspace entries | semilattice and range reconciliation | bounded/partial-sync proposals | evolving profile, Ed25519/Meadowcap | write authorization tokens, not global peer classes |
| OCapN | object references/handoff, not CRDT | transport-separated object communication | draft; no generic reconciliation claim | living drafts | reference authority; attenuation unverified here |

## Options and revisit rule

Adopt one now, defer behind an adapter, or adopt none. Default: **defer all** until conflict reports exist and representative fleet workloads show merge cost/latency or user-value evidence that justifies adoption. A later choice must preserve signed origin, conflict artifacts, deletion/privacy scope, and cross-implementation vectors. The comparison is not exhaustive: newly discovered prior art gets a new evidence row, not a silent replacement.

## Sources

- [Automerge Concepts](https://automerge.org/docs/reference/concepts/)
- [Diamond Types authors' paper page](https://martin.kleppmann.com/2025/03/30/eg-walker-collaborative-text.html)
- [Willow Data Model](https://willowprotocol.org/specs/data-model/)
- [Willow Confidential Sync status](https://willowprotocol.org/specs/confidential-sync/)
- [OCapN CapTP draft](https://github.com/ocapn/ocapn/blob/main/draft-specifications/CapTP%20Specification.md)

**Conclusion:** no adoption is a complete, passing research outcome until scale evidence earns reconsideration.
