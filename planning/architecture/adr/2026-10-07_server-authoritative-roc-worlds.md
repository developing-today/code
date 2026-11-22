# ADR: Run Roc worlds as server-authoritative, mediated programs

- **Date:** 2026-10-07
- **Status:** accepted
- **Supersedes:** none
- **Related:** [Roc-Defined Multiplayer Worlds](../../../doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md); [Attenuated capabilities for world access](../../../doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md)

## Context

Multiplayer worlds need shared, reproducible state without granting guest
programs ambient access to the host. The source record contrasts server
authority with peer consensus and revises its initial stateless transition
sketch. As an ADR synthesis, the alternatives are client-authoritative
execution, which moves authority to peers; consensus-based execution, which
adds peer coordination; and unrestricted native code, which makes an
untrusted program indistinguishable from host code. This ADR records the
settled per-world Store+Instance design rather than the initial sketch.

## Decision

The server is authoritative for each world. World programs run through a
versioned Roc/Wasm boundary in a per-world Store+Instance, with no ambient
filesystem, network, clock, randomness, or process access. Host effects are
explicit mediated capabilities, constrained by administrator-set limits and
the program's trust tier; a node identity is not itself authorization.
Unpinned or peer-supplied code remains in the Wasm tier and is never promoted
to native execution. Persist world changes in the durable event-sourced
`.id-worlds/` journal; retain the protocol's versioning and provenance rules.

## Consequences

The server owns ordering and world state, and guests remain portable across
transports and constrained by a declared host interface. The host must maintain the ABI, capability mediation, execution limits, and
durable journal semantics. The native tier is implemented as an administrator opt-in worker isolated
with OS controls, including Landlock, seccomp, rlimits, and a per-call
deadline. A dead worker poisons the program; after a server restart, the
worker and journal state are restored from stored artifacts. Native execution
is not claimed to be resource-safe in-process. It requires explicit trusted provenance rather than language or
peer identity; distributed consensus and general replicated writes are
outside this decision. Follow-up: preserve the worker isolation and
admin-configured resource limits ([source record](../../../doc/2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)).
