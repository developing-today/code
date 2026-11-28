# Delegated Manage membership changes: authorized-basis design

No prior plan file exists for this task; this record is the design-first document required by `pkgs/id/docs/DOCUMENTATION_PROTOCOL.md` before any membership-layer change.

## Request / intent

Issue #171: a delegated Manage holder whose membership role is below Manage is rejected in `Engine::member()` (`pkgs/id/crates/id-events/src/groups/fold.rs`) before `delegation.rs::manage_chain()` can validate the delegation. The task packet (2955, `small_fix`) asks for the narrow authorization correction plus regression coverage, preserving Admin-specific checks and causal-chain refusal behavior.

## Design approach (decided, not yet implemented)

Decided: delegated Manage operations must carry an explicit, causally validated authorization basis into the membership layer. The signed author remains the author; the operation is not modeled as ordinary membership and the signer is not substituted.
Why: `id-events::manage_chain` validates the chain, but `id-core::Membership::decide` hands the operation to p2panda-auth, which checks the signer's own access independently. Removing the fold gate alone fails there (prior remote evidence: `Refused(NotManager)` at fold; removing the gate moves the refusal to `Membership::decide` with `actor lacks sufficient access`). A synthetic roster entry would corrupt the roster and roots; signer substitution would corrupt attribution and StrongRemove behavior.

Concretely, the seam (for an expanded packet, out of `small_fix` scope):

1. `id-events` fold computes the delegation basis at the event's causal position (`authorized()` chain + attenuation + expiry + veto/void/removal-mode disposition + Admin guards unchanged) and passes an opaque `AuthorizationBasis` alongside the `AuthOperation`.
2. `id-core::Membership` admits/processes the operation on that basis: the basis pins the causal position (frontier/dependencies), the chain that was valid there, and the asserted time used for expiry. Replay (canonical order, `rebuild_canonical`, resolver filtering) re-validates or replays the pinned basis deterministically — it must not become a standing permission.
3. p2panda-auth still owns membership state mutation and StrongRemove arbitration; the basis only answers the "may this signer act here" question that p2panda-auth would otherwise answer from roster access alone. Admin-only paths (`Add` with `Admin`, `SetRole`, Admin removal of Admin) keep requiring roster Admin status, never delegation alone.
4. Refusal behavior is preserved: no-chain → `NoChain`, attenuated → `Attenuated`, unknown/voided/cut links stay refused; placeholders/walk-through semantics for descendants unchanged.

## Alternatives considered

- **Fold-gate-only relaxation** (remove `NotManager` checks so `manage_chain` decides): rejected. Prior WIP evidence shows `Membership::decide` then refuses via p2panda-auth. Unsafe and insufficient.
- **Synthetic membership (add delegate as manager in the auth graph)**: rejected. Corrupts roster, roots, `members()`/`managers_at_position`, and downstream writer-set/device reasoning.
- **Signer substitution (re-attribute the op to a manager)**: rejected. Corrupts authorship, `is_effective` leave-vs-remove logic, StrongRemove/mutual-remove arbitration, and auditability.

## Verified when

A delegated holder with below-Manage roster access but a valid, unexpired, unvoided chain to Manage scope at its position gets `Applied` for a member add/remove, takes effect in the settled projection, and replays identically in every arrival order; a holder whose chain is cut/expired/voided/attenuated is refused (`NoChain`/`Attenuated`) with Admin guards intact. Evidence: targeted regression tests in `id-events` groups fold plus `id-core` membership property tests, run via `just check` (or the repo's standard check recipe) and, for any remote run, a Fleet dispatch result.

## Status

Design recorded; **no production implementation** in this `small_fix` pass. The correct seam touches id-core authorization state carried through p2panda-auth validation, state mutation, and StrongRemove replay — explicitly excluded from this packet's scope by its blocker. Resume requires an expanded implementation packet covering id-core + p2panda-auth seam design, replay-safe trusted authorization, signer attribution, and Admin-guard preservation, reusing the red regression as the acceptance oracle.

## References

- `pkgs/id/crates/id-events/src/groups/fold.rs` (`member()`, `Engine::apply`)
- `pkgs/id/crates/id-events/src/groups/delegation.rs` (`manage_chain()`, `authorized()`, `links()`)
- `pkgs/id/crates/id-core/src/membership/layer.rs` (`Membership::decide`, `process`)
- `pkgs/id/crates/id-core/src/membership/gate.rs`, `pkgs/id/crates/id-core/src/membership/mod.rs`
- Issue https://github.com/developing-today/code/issues/171 (open); PR #3694 closed unmerged (not inspected, not touched)
