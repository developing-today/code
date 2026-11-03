# Explicit group removal modes

- Date: 2026-11-02 (UTC 04:44)
- Scope: `pkgs/id` group `remove()` behavior and its replicated event path.
- Status: implementation in progress; this records the project owner's settled decision, not a new policy choice.
- Source: [issue #170](https://github.com/developing-today/code/issues/170), especially the owner decision and refinements in comments 6488949304, 6489076460, 6489453634, and settled compatibility note 6568031916.
- Related design: [groups and delegation](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), whose earlier three-policy framing predates this decision.

## Intent

Make the fate of a removed member's delegated grants an explicit, policy-validated choice. The current optional successor silently makes a successor-less transfer revoke grants. A literal cascade fallback is not part of this change: for a removed grantor it has the same effective result as revoke.

## Settled behavior

`remove()` requires a `RemoveMode`:

- `Revoke`: the removed member's grants end.
- `Transfer(TransferOpts { successor })`: reissue grants to a permitted `Member`, `Admin`, `Designated` account, or the removing `Actor`.
- `KeepPrior`: preserve prior grants under their original grantor and expose them in group grant listings as retained.
- `ReassignToGroup`: re-anchor grants at the group while preserving each grant's scope and `redelegate` flag; this adds neither membership nor Manage authority.
- `ServerDefault`: resolve through group policy before an event is emitted. Only a concrete deterministic mode is persisted and replayed.

Policy controls allowed modes, allowed successor kinds, and its default. Reject a disallowed mode or successor at the action/apply boundary; do not silently degrade it. Existing transfer-to-member behavior remains unchanged.

The replicated `MemberOp` event changes to carry a concrete mode. Deserialization of legacy events without `mode` remains deterministic: legacy `successor: Some(key)` maps to `Transfer(Member(key))`; `None` maps to `Revoke`. The upstream signed `GroupAction::Remove { member }` bridge remains unchanged; grant disposition is represented by the groups-layer event, not that membership/access bridge.

## Initial implementation plan

1. Add serializable mode/successor types and policy fields, preserving old event decoding.
2. Require a mode on the remove API; resolve `ServerDefault` and validate policy before event emission.
3. Apply concrete mode semantics in the deterministic group fold/delegation view, including retained-grant enumeration and group re-anchoring.
4. Update remove-path docs and add tests for each mode, policy rejection, replay compatibility, scope/redelegation preservation, and refusal of Manage-gated actions by a recipient of a re-anchored grant.
5. Run targeted group tests and relevant `pkgs/id` checks.

The work remains scoped to the remove path; sibling groups APIs and broader membership/policy redesign are excluded.

---

## 2026-11-02T04-52-00Z Update: initial server-policy defaults

The initial `RemovePolicy::default()` allows the four concrete modes and all four successor categories, while selecting `Revoke` for `ServerDefault`. This preserves existing explicit `Transfer(Member)` use while making an unspecified removal least-authority and behavior-compatible with the current outcome for a removed grantor. A group's policy can narrow the allowed lists; the old group-wide `Delegation` setting does not silently override an explicit removal mode.
