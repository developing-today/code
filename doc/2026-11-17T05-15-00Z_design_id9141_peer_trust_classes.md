# Design (#9141): peer trust classes + signed-data acceptance rule

Status: design section for issue #9141 (program #9106, parent epic #9108).
Author: fleet lane any-j, task 20333, 2026-11-17.
Basis: Drew directive 2026-11-12 18:29 CST, program #9106 §1
(statements S1.1–S1.25 as mapped in the #9106 statement map).
Settled neighbours this section consolidates, and does not override:

- The #9106 ID/Atlas summit design, `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md`
  (wire/data model; configuration invariants; key/revocation posture).
- Slice B31, implemented and merged (PR #10344, issue #10186):
  `pkgs/id/crates/id-sync/src/federation/slices/b31_peer_classes.rs`
  plus its golden test `pkgs/id/crates/id-sync/tests/federation_b31.rs`.
  Where this section and B31's matrix could disagree, B31's tested
  matrix is the conformance oracle and this section must be amended.
- Slice B32 (issue #10233, open): the mandatory universal
  signed-and-validated acceptance gate at every ingress path.

## 1. The acceptance rule (non-negotiable)

Data is accepted into local accepted state **only** when it is
cryptographically signed and validated, regardless of source —
including from nominally trusted peers, from loopback, and from the
local node's own producers (a local producer signs before the same
gate; there is no unsigned local shortcut).

"Validated" is the full gate, all four checks, for every inbound
object (B31 `GateEvidence`):

1. `signature_valid` — the signature verifies under the claimed
   signer's key.
2. `signer_authorized` — the signer is the peer (or an identity bound
   to it) and is authorized for this kind of object (B05, issue
   #10208, landed: signer identity binding is distinct from mere
   signature presence).
3. `semantically_valid` — the object passes semantic validation for
   its type.
4. `scope_authorized` — the object is inside the scope the peer is
   configured for.

A trust class is a **directional policy label, never a cryptographic
fact and never a gate bypass**. Class membership decides *whether an
operation may be attempted*; the gate decides *whether an inbound
object may be accepted*. Both must pass, in that order of evaluation:
class/policy first (cheap), gate second (per object).

## 2. The three classes, plus the default

| Class | Meaning |
|---|---|
| `fully_trusted` | Broad configured operations in both directions, subject to the peer's configured operation set and — inbound — the full gate. Never a validation bypass. |
| `send_only` | We send to this peer; we never accept peer-originated data from it. Inbound acceptance is class-forbidden, even for perfectly signed objects. |
| `accept_signed_from` | Inbound acceptance only, and only for objects passing the full gate. Grants no outbound sends by itself. |
| `unknown` (default) | No configured trust. The restrictive default for any peer with no policy, an expired policy, or a policy not yet in force: deny everything. |

Class changes are **versioned per peer** (`policy_version`, monotonic):
a demotion or revocation is a new policy version, never an in-place
edit, and takes effect as soon as evaluation uses the new snapshot.
A policy may be time-bounded (`valid_from` / `expires_at`); outside
its window the peer is `unknown`.

## 3. Per-operation semantics

Operations are directional. Reads, writes, and relays from the
directive map onto the matrix operations as follows: a **write** to
us is inbound acceptance; a **read** by a peer from us is an outbound
send of already-accepted data (reading never admits new data, so no
gate applies on the read path — the data passed the gate when it was
accepted); a **relay** moves a third party's data or queries and is
never implied by any class.

| Operation (direction) | fully_trusted | send_only | accept_signed_from | unknown |
|---|---|---|---|---|
| Accept data — write in (inbound) | Allow iff configured **and** full gate passes | Deny (`ClassForbids`) | Allow iff configured **and** full gate passes | Deny (`UnknownPeer`) |
| Send data — read out (outbound) | Allow iff configured | Allow iff configured | Deny (`ClassForbids`) | Deny (`UnknownPeer`) |
| Query relay (outbound) | Deny unless a separate relay grant exists (`NotGrantedByClass`) | Deny unless a separate relay grant exists | Deny unless a separate relay grant exists | Deny |
| Completeness authority (inbound claims) | Never granted by class | Never granted by class | Never granted by class | Deny |
| Cross-tenant access | Never granted by class | Never granted by class | Never granted by class | Deny |

Rules that fall out of the table, stated once:

- An operation absent from a peer's configured operation set is
  denied (`NotConfigured`) **even for `fully_trusted`**. The class
  is a ceiling, the configuration is the actual grant.
- Inbound acceptance without all four gate checks is denied
  (`GateNotPassed`) for **every** class, `fully_trusted` included.
- No class grants relay, completeness authority, or cross-tenant
  access by itself. Relay requires a separate, explicit grant
  recorded on the policy (`relay_granted`); completeness and
  cross-tenant access require their own designs and are out of
  scope here.
- Evaluating an operation in a direction it does not have is a
  denial (`WrongDirection`), not a silent no-op.

## 4. Configuration per peer

One policy snapshot per peer (`PeerPolicy` in B31): `peer_id`,
`class`, the configured `operations` set, the separate
`relay_granted` flag, the `scope` the policy applies to,
`policy_version`, and optional `valid_from` / `expires_at`.

Configuration **cannot** (summit part-b invariant, restated):
disable the gate, widen a peer beyond its class ceiling, grant
relay/completeness/cross-tenant rights through the class field, or
make an unknown peer acceptable. Emergency peer disable and key
revocation are always available and take effect at the next
evaluation against the new policy version; revocation posture for
previously accepted data is owned by issue #10204, not this section.

A hosted service peer is an ordinary configured peer: it gets a
class and an operation set like any other, and its data passes the
same gate.

## 5. Worked defaults for the open questions

Recorded as chosen defaults (changeable by amending this section;
silence is not a decision):

1. **Does `fully_trusted` imply relay?** No. Default: relay is a
   separate grant for every class (§3).
2. **A `send_only` peer's signed object arrives via a third-party
   relay.** Default: acceptance is judged against the *originator's*
   class (the signer), not the carrier's; the carrier needs its own
   relay grant for carrying it. A `send_only` originator's object is
   therefore still not accepted.
3. **Class change mid-session.** Default: no session grandfathering.
   Each operation is evaluated against the current policy snapshot;
   a demotion denies the next operation, it does not abort data
   already accepted under the earlier version (that data passed the
   gate when accepted; revocation effects are #10204's).
4. **Expired policy.** Default: the peer becomes `unknown` at
   `expires_at`; there is no grace window and no auto-renewal.
5. **Outbound sends and the gate.** Default: the gate is an inbound
   (acceptance) concept. Outbound objects are signed by the local
   producer before they are admitted locally, so a send carries
   already-gated data; the class matrix still decides whether the
   send may happen at all.

## 6. Implementation status and follow-on

Already landed: B31 peer-class matrix (#10186, PR #10344, golden
matrix test over class × operation × direction × gate), B05 signer
binding (#10208), key lifecycle (#10183), replay/freshness (#10206),
signature-suite agility (#10207). Open slices that build on this
section: B32 universal gate (#10233), rejection observability
(#10247), capability negotiation / downgrade rejection (#10195),
config schema + hard invariants (#10178), plane independence
(#10194), revocation effects on accepted data (#10204). No new
implementation issues are filed by this section: the acceptance
criteria's "granular issues for anything implementable" are the
slices above, filed by the #9106 mill.

Acceptance mapping for #9141: design section merged under `doc/`
(this file); open questions recorded with chosen defaults (§5);
granular implementation issues filed (§6).
