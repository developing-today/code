# Design: delegation, transfer, and cross-signed identity ceremony

Issue: [#9132](https://github.com/developing-today/code/issues/9132). Parent epic: [#9107](https://github.com/developing-today/code/issues/9107) (Part A, ID core). Program: [#9106](https://github.com/developing-today/code/issues/9106). Statements S5.56–S5.61 and S5.63–S5.64 of `doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md` are the requirement set.

Builds on §11 (and the address profile in §10, invariants in §13) of `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md`, which this record makes concrete as a ceremony. Invite delegation (a different thing: who may admit a joiner to a group or world) is in `doc/2026-11-14T18-50-00Z_design_id_invite_delegation/`; this record does not change it.

## Decision summary

1. **Three distinct operations.** *Delegation* grants a narrow scope to another key and leaves the original controller in place. *Transfer* replaces the controlling key of a space. *Identity equivalence* is a data assertion that two keys are linked; it grants nothing by itself. Each has its own signed record and its own ceremony.
2. **Delegation is published, not claimed.** A grant takes effect only when it is listed in the space's well-known delegation location and the verifier checks the grantor's signature. A grantee cannot make itself a delegate by asserting it.
3. **Transfer needs both controllers.** The old controller signs an offer; the new controller signs an acceptance over the same bytes. Neither signature alone moves control.
4. **Identity triples need both sides.** A `sameAs`-style triple is honoured as a link only when both keys sign the same challenge bytes. A unilateral triple is stored as `pending` and grants nothing.
5. **Unilateral claims are inert.** Pending claims can be displayed, cannot authorize anything, expire, and can be withdrawn by their issuer.

## 1. Operations and their records

All records use the shared signed-record profile: a detached signature over domain prefix + deterministic CBOR of the body, as `versioned-design.md` §3.1 and `pkgs/id/crates/id-core/src/signed_envelope.rs` (`canonical_cbor`) define it. Timestamps in the signed body are UTC Unix seconds. The transport codec (#5674 postcard) never forms signed bytes.

**Delegation record** (signed by the grantor):

`{ space, grantor_key, grantee_key, scope, permissions, not_before, expires, nonce, predecessor?, status }`

- `scope` holds exact typed array-address prefixes, per the address profile in `versioned-design.md` §10 (e.g. `[typed_text("id"), typed_text("space-control"), typed_object_ref(space_id), …]`). No wildcards in v1.
- `permissions` enumerates `read | append | edit | moderate | delegate`. `delegate` is never implied; a grantee without it cannot create further grants.
- A delegation may not exceed the grantor's own authority in the verifier's frontier.
- `predecessor` names the delegation this one replaces, if any, so re-grants form a chain rather than a pile.

**Transfer record** (two signatures over one body):

`{ transfer_id, space, old_controller, new_controller, scope, nonce, domain, expires }`

- The old controller signs the **offer**; the new controller signs the **acceptance** over the identical body bytes.
- The transfer replaces the controlling key for the space. It never mutates the immutable public-key address binding of the space; a successor either proves a signed chain at the original address or publishes an explicit signed redirect from old to new (per `versioned-design.md` §10).
- Transfer does not revoke the old controller's existing delegations automatically. Delegations the old controller made stay as recorded and are re-verified against the new controller's frontier; the new controller may revoke them.

**Revocation record** (signed by the grantor or the current root controller): references the grant or transfer ID and takes effect at the latest verified authority frontier. After a transfer, the current root controller is the new controller, so it can revoke inherited grants directly.

**Identity-link record** (cross-signed): `{ key_a, key_b, purpose, scope, nonce, expires, domain }`, signed by **both** keys over the same challenge bytes. The challenge binds the ordered key pair, the purpose and domain string, the exact scope, the nonce and the expiry. Unrelated unilateral statements cannot be paired because the challenge differs.

## 2. The delegation ceremony

The grantor publishes; the grantee does not need to do anything except be reachable for acceptance if the scope requires one. Steps:

1. **Grantor mints.** The grantor's client builds the delegation record with a fresh nonce and signs it.
2. **Grantor publishes.** The record is appended to the space's well-known delegation location (`[typed_text("id"), typed_text("space-control"), typed_object_ref(space_id), typed_text("delegations")]`). Publishing is an append like any other, subject to the space's own admission rules.
3. **Verifier checks.** A verifier, client or server, accepts the delegation only if all of these hold at one authority frontier:
   - the signature verifies against `grantor_key`, and the grantor holds `delegate` (or is the root controller) for every permission granted;
   - `not_before <= verifier_utc + 300s` and `expires >= verifier_utc - 300s`, using the verifier's trusted clock;
   - the nonce is unique for `(grantor, domain, nonce)` at that frontier. An identical envelope is idempotent; the same nonce with a different body is `ReplayConflict`;
   - the chain back to the root controller has no cycle and is within the operator's depth budget (suggested maximum 16). Over budget is `DelegationChainTooDeep`, never partial authority;
   - no revocation for the record is visible at that frontier.
4. **Acceptance is implicit for scoped read/append; explicit for `moderate` and `edit`.** The grantee's client shows the grant and the grantee signs an acknowledgement, which is recorded but does not change validity. This keeps the grantor's publication sufficient for verification while letting the grantee see what they hold.

Revocation is a signed record checked the same way. Freshness: the initial deployment profile caps revocation staleness at **24 hours for all delegated writes**. Beyond that, writes under a delegation fail closed. Old reads may be shown as stale, not current. Offline clients surface frontier age so the user can see when this is about to bite.

## 3. The transfer ceremony

1. The old controller signs the offer over `transfer_id, space, old_controller, new_controller, scope, nonce, domain, expires`.
2. The offer is published to the well-known location, the same as delegations.
3. The new controller reviews the offer and signs the acceptance over the **same body bytes**. Mismatched bodies are rejected.
4. Verification requires both signatures, the expiry window, and that the transfer ID is unused. The new key then controls the space from the transfer's frontier forward.
5. If the old key is unavailable, **no recovery is inferred**. The space either has an explicit namespace recovery policy in its own signed profile, or the owner creates a new space and the old one is recorded as unresolved. A transfer cannot be forced by anyone holding only the new key.

Transfer is distinct from adding a delegate: adding a delegate never moves control, and transfer never leaves an implicit delegate behind.

## 4. Identity equivalence: cross-signing

- Both keys sign the **same challenge bytes**. Each side publishes its signature toward the other; the link exists only when both are present in verifiable form.
- A link with one signature is **pending**. A verifier reports `LinkClaim { status: pending }` and nothing else.
- A cross-signed link is evidence that both keys approved that statement. It does **not** prove the keys belong to one person, and it does not share privileges. Authorization still comes only from delegations and transfers.
- `sameAs` triples are data claims. They create no authentication, no authorization and no SSO equivalence. (SSO equivalence is owned by #9077.)
- Identity resolution never collapses keys by name, account, endpoint, tenant or profile similarity. A client shows the trust level and its source separately from the key.

## 5. Pending and unverified claims

| Property | Rule |
|---|---|
| What a pending claim may do | Be displayed, with its source and signature state, and be counted in review queues. |
| What it may not do | Authorize reads, writes, delegation, transfer, or identity merging. It is never treated as a grant. |
| Expiry | Every pending claim carries `expires`. After expiry plus 300 s it is discarded from active views but its bytes remain in audit. |
| Withdrawal | The issuer may publish a signed withdrawal referencing the claim ID. A withdrawn claim is shown as withdrawn, never silently removed. |
| Promotion | A pending claim becomes a link only by the counterparty's signature over the same challenge bytes before expiry. Nothing else promotes it. |
| Verifier checks before honouring a delegation | Signature, authority at frontier, time window, nonce uniqueness, chain depth, no revocation (§2 step 3). |
| Verifier checks before honouring an identity equivalence | Both signatures over identical challenge bytes, unexpired, purpose and scope match the request. |

## 6. Open questions and chosen defaults

Every question below is decided here. None is left operative as TBD.

| # | Question | Chosen default |
|---|---|---|
| Q1 | Should delegations be listed in the space's well-known location or pushed to grantees? | **Listed in the well-known location.** Verification must not depend on delivery to the grantee. |
| Q2 | Can a delegation be revoked by someone other than its grantor or the root controller? | **No.** Only the grantor or the current root controller revokes. After a transfer the new controller is the current root controller, so it can revoke inherited grants by ID directly, without re-granting them first. |
| Q3 | Does transfer revoke the old controller's delegations? | **No, not automatically.** They are re-verified against the new frontier; the new controller revokes what it does not want. This avoids silently changing access on transfer. |
| Q4 | Is grantee acceptance required for a delegation to take effect? | **No for `read` and `append`; yes-as-acknowledgement for `moderate` and `edit`.** The ack is recorded but does not gate validity, so an unreachable grantee cannot block a grant. |
| Q5 | Recovery when the old controller key is lost. | **Explicit recovery policy in the signed space profile if the owner set one; otherwise not inferred.** The space is preserved as unresolved, and a new space is created to move on. |
| Q6 | Maximum delegation chain depth. | **16 as the operator default**, not a protocol limit. Over budget fails closed. |
| Q7 | Revocation staleness for delegated writes. | **24 hours**, operator may lower. Beyond that, delegated writes fail closed. |
| Q8 | Lifetime of an unpaired identity claim. | **Explicit `expires`; default 30 days** if the issuer sets none. Withdrawal is always available. |
| Q9 | Does a cross-signed link ever imply shared privileges? | **No.** Authorization comes only from delegations and transfers. |

## 7. Implementation issues

Implementable pieces, to be filed and cross-linked from #9132 and #9107 once this record is merged. Each names its oracle, per the testing doctrine in `AGENTS.md`.

1. **Delegation record codec and verifier.** Canonical encode/decode round-trip; signature check; `ReplayConflict`; `DelegationChainTooDeep`. Oracle: independent signature verifier and the model state machine and invariants in `versioned-design.md` §13. Property tests for replay and depth.
2. **Transfer offer/acceptance pair.** Same body bytes required; both signatures; `transfer_id` uniqueness. Oracle: a reference implementation that rejects any mismatched body.
3. **Identity-link challenge and cross-signature verifier.** Challenge binds ordered key pair, purpose, scope, nonce, expiry. Oracle: a golden corpus of paired and unpaired statements.
4. **Pending-claim lifecycle.** Expiry, withdrawal, promotion-only-by-counterparty. Oracle: model state machine over claim states.
5. **Revocation at the frontier and the 24-hour staleness rule.** Oracle: fail-closed model test at the boundary.

## Sources

- [#9132](https://github.com/developing-today/code/issues/9132), [#9107](https://github.com/developing-today/code/issues/9107), [#9106](https://github.com/developing-today/code/issues/9106)
- `doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`, section 5 statements S5.56–S5.64
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/versioned-design.md`, §10 and §11
- `doc/2026-11-14T18-50-00Z_design_id_invite_delegation/` (invite delegation, a separate concern)
