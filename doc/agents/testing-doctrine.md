# Testing doctrine

Indexed from [`AGENTS.md`](../../AGENTS.md).

Three tools, used where they fit:

- **PBT (property-based testing):** state invariants that must always hold, then let a generator hammer the code with thousands of random inputs to find counterexamples. Where code has meaningful invariants — parsers, serializers, state machines, replication, ID/crypto formats — state the invariants and cover them with property tests. In Rust here that is `proptest` (already a dev-dependency of `pkgs/id`).
- **MBT (model-based testing):** where a system's behavior has a spec or design model, build the model and generate tests — and their expected results — from it, instead of hand-writing cases only.
- **Oracles:** the decision procedure that tells you whether actual output matches expected — the hard part of automated testing. Every automated test names its oracle: a reference implementation, the model, an invariant check, or a golden corpus. A test with no decidable oracle is a smell to fix, not ship.

Example-based tests remain fine for smoke and acceptance coverage; they do not replace PBT/MBT where those fit.
