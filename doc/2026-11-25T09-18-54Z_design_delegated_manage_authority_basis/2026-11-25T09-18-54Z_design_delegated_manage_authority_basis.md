# Delegated Manage membership authority

## Request and verified problem

Issue [#171](https://github.com/developing-today/code/issues/171) asks a group member with a valid delegated Manage grant to add and remove members even when their membership role is below Manage. On `origin/main` at `cb1b80c`, `groups/fold.rs::member` refuses these operations based on membership role before `manage_chain` checks the delegation. Removing only that refusal is insufficient: `id-core::Membership` passes the signed author to p2panda-auth, whose group processor independently enforces the author's Manage access.

The design record for groups says an action is valid through a causal grant chain from a root (§7.2, [delegation design](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md)). p2panda-auth also uses `Operation::author()` for StrongRemove filtering. Treating the manager root as the operation author would change which concurrent events are removed.

## Decision

Keep the signed actor separate from the authority that permits a membership action. The group fold validates the actor's Manage chain at the action's causal position. It then gives id-core a verified authorization marker for that membership operation. p2panda-auth uses the marker only to validate and apply Add/Remove membership changes; `Operation::author()` remains the signed actor for attribution and StrongRemove. Self-removal continues to use the signed actor and requires no delegated authority.

`AuthorizationActor::Verified` is a caller attestation, not a cryptographic proof. In this application only the group fold may set it, after checking the complete chain, scope, grantor standing, removals, expiry and vetoes. The membership layer preserves it in its replay history. Event-store resolution recomputes it from the signed event set and the group projection; it is not stored as a durable fact on the event.

Admin-specific policy remains in the group fold: delegated Manage cannot add an Admin or remove an Admin. The p2panda Verified path is limited to Add/Remove; it does not authorize Promote or Demote. A cut or invalid grant produces no verified marker and the membership change is refused.

## Rollout slice

The first slice adds a separate p2panda authorization actor API, while preserving the signer for resolver rules. Its regression oracle covers a Read member whose delegated Manage add/remove applies, a caller-supplied member basis that lacks Manage being rejected, self-removal remaining a leave, and StrongRemove filtering the delegate's concurrent action by the delegate's identity. The code-repository slice then derives `Verified` only from the event-fold's causal authorization check and carries it into both the in-memory and event-store membership paths.

The action-level root may be a current manager or a re-anchored group root. Both are evaluated by the same fold authorization check; neither is converted into a synthetic membership or signer identity.

## Verified when

- `cargo test -p p2panda-auth delegated_authorization_keeps_signer_identity_for_strong_remove` passes, and the complete `p2panda-auth` test suite passes remotely.
- `cargo test -p id-events groups::tests::test_delegated_manage_can_add_and_remove_members` passes remotely.
- The event-store replay test observes the same Add/Remove outcome as the group fold, while a cut chain remains refused and the Admin guards remain unchanged.
