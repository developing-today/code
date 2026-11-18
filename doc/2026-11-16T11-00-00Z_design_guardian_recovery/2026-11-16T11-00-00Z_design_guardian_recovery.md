# Design: guardian social recovery — record formats, successor-key authority, threshold signing (#2382)

**Date:** 2026-11-16 · **Issue:** [#2382](https://github.com/developing-today/code/issues/2382) · **Parent epic:** #2056
**Status:** Proposed (fleet design default, in the form of the #2380 keyset ADR)
**Inputs:** #2382 proposal; the #2380 keyset / authorization log ADR
(`doc/2026-11-12T19-10-07Z_design_keyset_authorization_log/`), whose
`GuardianSetUpdated` entry this record gives its semantics; the #2060
key-recovery investigation
(`doc/2026-10-29T11-28-18Z_investigation_key_recovery_hosted/`), in
particular §4.4 (rotation and continuity), §6.3 (why reconstruction
defeats the veto), §7 (the two recovery-proof rules), and §8.5.4 (the
threshold-signing gap this record closes with Decision 5).

## Problem

#2382 asks for last-resort, non-custodial recovery: a user who has lost
every device and any backup names n guardians and a threshold m;
m guardian approvals install a new key after a delay window in which
the old key can cancel. The first implementation attempt (task 8554)
packet-defected because the record formats and the security semantics
— successor-key authority, revocation, threshold signing — were
unspecified. This record specifies them, as decided positions, against
the #2380 keyset log. It is a design record only: implementing the
recovery flow is a later packet, after this record and after the
#2380 substrate lands (§8).

Two investigation findings bound the whole design and are adopted as
axioms:

- **Approvals never reconstruct the existing key** (#2060 §6.3). Any
  scheme that hands guardians shares of key material (Shamir splitting
  of the identity or its unwrap key) gives k colluding guardians the
  _existing_ signing key immediately, before any delay or veto can
  run, and epoch binding does not revoke that. Shamir splitting is
  therefore excluded, as #2382 itself directs. Guardians authorise a
  **successor** key; the existing key is never reconstructed,
  escrowed, or derivable from anything a guardian holds.
- **The server is relay/timestamp only** (#2382 acceptance). The
  directory relays records and attests first-seen times. No server
  action can forge an approval, complete a rotation, or shorten a
  delay (Decision 4 makes the delay math server-proof).

## Decision 1 — where the records live: keyset chain + recovery annex

The guardian set lives in the identity's #2380 keyset log, in the
entry the ADR already reserved. Recovery _process_ records cannot be
ordinary chain entries: a chain record is valid only if its author
was authorised at its position (ADR Decision 4 rule 3), and a
recovery request is authored by a brand-new successor key that holds
no authority yet. Recovery therefore uses two homes with one envelope:

- **Chain entries** (the existing `KeysetRecord` envelope, ADR
  Decision 2): `GuardianSetUpdated`, and the terminal
  `RecoveryRotation` entry of Decision 3, which the fold accepts
  under a dedicated rule (Decision 3, rule R) rather than the
  admin-author rule.
- **Annex records**: `RecoveryRequest`, `RecoveryApproval`,
  `RecoveryCancellation` (Decision 2). Annex records use the same
  envelope fields (`author`, `lamport` is replaced by the referenced
  keyset head, `prev_hash`, `hash`, `signature`) but are **not**
  folded into the admin chain and confer no authority by existing.
  They are stored and relayed by the directory, are addressed by
  `request_id`, and are consumed by verifiers when evaluating a
  `RecoveryRotation`. An annex record's `prev_hash` field carries
  the `keyset_head_hash` it was written against, pinning every
  recovery act to one folded state.

All bodies use the ADR's canonicalisation: the signed body is the
canonical `serde_json` encoding of `{ author, prev_hash, entry }`
(for annex records, with `request_id` inside `entry`), `hash` is the
SHA-256 of those bytes, and the signature is over those bytes. One
canonicalisation story, one set of test vectors, shared with
`directory_log` and the keyset log.

## Decision 2 — record formats

Guardian identity references are identity identifiers in the ADR
Decision 1 form (hex Ed25519 public key of the guardian identity's
genesis admin key). A guardian's approval is valid only if it is
signed by a key that is an **Active admin key in the guardian
identity's own current fold** — a guardian acts with their admin
authority, never with a device key, and never with a Seitan/invite
key (#2060 §6.1: the Seitan key is single-use and is not an
identity). Guardian identities are learned at invite acceptance and
confirmed out of band by comparison of the **full** account key
(§6.1; the 24-bit petname fingerprint authenticates nothing).

```rust
// Chain entry — already reserved by the #2380 ADR, semantics fixed here.
GuardianSetUpdated {
    /// Identity identifiers of the guardians; order is canonical
    /// (sorted) so equal sets hash equally.
    guardians: Vec<String>,
    /// m: approvals required, 1 <= threshold <= guardians.len().
    /// Recommended default 3-of-5 for high-value identities
    /// (#2060 §6.3 mitigation iii).
    threshold: u16,
}

// Annex record — authored by the successor admin key (self-signed,
// proof of possession of the successor secret). Creating a request
// requires no existing authority; authority comes from approvals
// plus delay, never from the request signature.
RecoveryRequest {
    /// The identity being recovered (Decision 1 identifier).
    identity_id: String,
    /// Keyset head hash the request is evaluated against. The
    /// guardian set, admin keys, and device keys are all read from
    /// the fold at this head — later changes do not move the target.
    keyset_head_hash: String,
    /// Successor keys the approvals will install.
    successor_admin_key: String,   // hex Ed25519 pubkey
    successor_device_key: String,  // hex Ed25519 pubkey
    /// Successor-signed unix time (seconds) of request creation.
    created_at: u64,
    /// Free-text reason, shown in every notification. Not verified.
    reason: String,
}
// request_id = hex SHA-256 of the canonical RecoveryRequest body.
// Single-use: a request_id is consumed by a rotation or a
// cancellation and can never be reused or replayed (#2060 §7 rule 1;
// persisted by the directory, not an in-memory cache).

// Annex record — one per guardian per request.
RecoveryApproval {
    identity_id: String,
    request_id: String,            // binds successor keys + head hash
    /// The guardian identity identifier (Decision 1 form).
    guardian: String,
    /// Guardian-signed unix time of approval.
    approved_at: u64,
}
// author = a guardian admin key; the approval counts once per
// distinct guardian identity, however many keys that identity has.

// Annex record — one valid cancellation ends the request.
RecoveryCancellation {
    identity_id: String,
    request_id: String,
    /// "cancelled-by-surviving-key" | "cancelled-by-guardian-process"
    /// (see Decision 4 for who may cancel and how).
    basis: String,
}
// author = any key that is, in the fold at the request's
// keyset_head_hash, an Active admin key OR an enrolled, unrevoked
// device key of the identity. One signature cancels (#2382).
```

## Decision 3 — the rotation record and successor-key authority

```rust
// Chain entry — terminal record of a recovery.
RecoveryRotation {
    request_id: String,
    /// Hashes of >= threshold distinct-guardian RecoveryApproval
    /// records, sorted canonically.
    approval_hashes: Vec<String>,
    successor_admin_key: String,
    successor_device_key: String,
}
```

**Rule R (fold extension to ADR Decision 4).** A `RecoveryRotation`
record is a valid chain continuation of the head named by its
request's `keyset_head_hash` — not of any later head — if and only
if all of the following hold, evaluated from the fold at that head
and the annex:

1. The request exists, is unconsumed, and its `successor_*` fields
   equal the rotation's.
2. `approval_hashes` resolve to approvals for this `request_id`
   from at least `threshold` **distinct** guardian identities in
   the guardian set folded at that head, each signed by an Active
   admin key of that guardian identity.
3. The delay has elapsed (Decision 4) and no valid
   `RecoveryCancellation` for the request exists with a directory
   first-seen time before the rotation's.
4. The rotation is signed by the successor admin key.

When a `RecoveryRotation` folds, the folded state changes exactly
thus: the successor admin key becomes an Active admin key; **every
prior admin key and device key is marked Revoked (superseded by
recovery)**; the successor device key is enrolled; the guardian set
is carried over unchanged (changing it is a normal admin act for the
successor, after recovery). The identity identifier does not change
(ADR Decision 1: it is a name, not an authority).

**Successor authority is therefore derivative and bounded:** it
exists only because the fold says the guardian quorum, pinned to a
pre-loss state, authorised this exact successor after a delay nobody
could shorten. The old keys' revocation is a fold fact, so every
verifier that folds the log rejects the old keys from the rotation
onward — this is the "replacement of the identity key and its
revocation across verifiers" that #2060 §6.3 requires, and it is
what Shamir reconstruction could never provide.

**Revocation semantics outside recovery** are unchanged from the
ADR: admin-signed `DeviceRevoked` / `AdminKeyRotated` at their chain
positions, judged at position, with revocation-priority fork choice
and checkpoint finality (ADR Decision 4 rule 5). A recovery in flight
does not block a surviving admin key from rotating or revoking in
the normal way; if the admin chain advances past the request's head
by a rotation that revokes the keys the recovery would supersede,
verifiers treat the request as moot (its head is no longer an
ancestor of the canonical head) and the directory marks it consumed.

## Decision 4 — delay, cancel, notify; the server cannot shorten the delay

- **Delay.** `RECOVERY_DELAY_SECONDS = 259_200` (72h, the #2060
  §6.2 example, adopted as the constant). A rotation is timely only
  if `rotation_first_seen >= request_first_seen + DELAY`, where
  `*_first_seen` are directory-attested first-seen timestamps.
- **Server-proofing.** The request body carries the successor-signed
  `created_at`. A directory first-seen time **earlier** than
  `created_at` is impossible for an honest relay, so verifiers
  reject any attestation chain with `request_first_seen <
created_at`, and compute the delay from
  `max(created_at, request_first_seen)`. A malicious or compromised
  relay can therefore only _lengthen_ the delay (by withholding) or
  refuse service — availability attacks, not safety attacks. It
  cannot forge an approval (signatures), cannot mint a cancellation
  (signatures), and cannot backdate (this rule).
- **Cancel.** One signature from any then-authorised admin or
  device key (Decision 2) cancels, at any time before the rotation
  folds. Cancellation is itself an annex record relayed and
  timestamped by the directory; a cancellation first seen after a
  rotation has already folded is void (the rotation is chain
  history; the remedy for a folded hostile rotation is a fresh
  recovery or admin action by the successor's opponents using the
  carried-over guardian set — stated plainly: after the delay, the
  window is closed).
- **Notify.** The directory MUST push a notification on request,
  on each approval, on cancellation, and on rotation, to every
  device key and guardian identity in the fold at the request head
  (#2060 §7 rule 2: any identity change is announced to every bound
  device before it takes effect). Notification is best-effort
  delivery but a hard relay duty: a relay that suppresses
  notifications is detectable after the fact because all four
  record kinds are public annex data any replica can audit.
- **Guardian-process cancellation.** If a guardian learns out of
  band that a request is hostile, they cannot cancel unilaterally
  (their authority is approval, not veto). They withhold approval
  and notify the user through the contact channel used at setup.
  The `cancelled-by-guardian-process` basis is reserved for a
  future threshold-objection rule and MUST NOT be accepted by
  verifiers in v1 — it is named now so the field is not reinvented
  later.

## Decision 5 — threshold-signing crypto dependency: none for v1

#2060 §8.5.4 left threshold signing undecided ("requires a new
crypto dependency"). **Decision: v1 uses no threshold-signature
scheme and adds no crypto dependency.** The "threshold signature"
of #2382 is a **threshold of signatures**: m distinct guardians
each produce an ordinary Ed25519 signature (the primitive `pkgs/id`
already verifies everywhere, via the same stack as
`directory_log`), and Rule R counts them against the folded
guardian set. Authority lives in the fold rule, not in a joint key.

Rationale, against the alternative (a FROST-style threshold
identity, #2060 §4.4 option 2, where the identity key itself is a
threshold key and recovery signs under the _same_ public key):

- §8.5.4's gap is specifically about **no-successor-needed
  rotation** — keeping one public key across recovery. #2382 does
  not ask for that; it asks for successor-key rotation with delay
  and cancel, which the investigation's own §6.3 names as the
  property that closes the reconstruction hole ("shares authorise
  a _new_ key and the existing signing key is never
  reconstructed"). A threshold-of-signatures delivers exactly
  that property with zero new primitives.
- A threshold identity key changes key _generation_ for every
  account (distributed key generation among guardians, resharing
  on guardian churn — the liveness problem §8.5.5 flags as
  unsolved), not just the recovery path. That is a programme-level
  change to the identity substrate, disproportionate to this
  slice, and it would block recovery on the hardest sub-problem.
- Counting signatures keeps every artefact independently
  auditable: each approval is a standalone signed record a third
  party can verify without ceremony transcripts.

Deferred, explicitly: if a future slice wants the same-public-key
property, FROST (RFC 9591) over Ed25519 is the named candidate and
the key format already reserves an algorithm/version tag beside
the public key (#2060 §4.4 recommendation). Adopting it later does
not change any record in this design: approvals would upgrade from
counted signatures to shares of one signature, behind the same
Rule R interface.

## Decision 6 — the guardian-eviction attack case

Attack (#2382 acceptance): an attacker who obtains a surviving
admin key (or the user under coercion) first removes the honest
guardians and adds sockpuppets, then runs a recovery with the
captured set. The design defeats the fast version and bounds the
slow one:

1. **Adding is instant; removing is delayed.** A
   `GuardianSetUpdated` that only adds guardians or raises the
   threshold folds immediately. One that **removes a guardian or
   lowers the threshold** is a _delayed change_: the fold records
   it as pending with the same 72h delay (directory timestamps,
   Decision 4 math), the pre-change set remains the effective set
   until the delay elapses, and any admin key can cancel the
   pending change with one signature (a chain-signed
   `GuardianSetUpdated` restoring the prior set, or an annex
   cancellation naming the pending record's hash).
2. **Requests pin their set.** A recovery request is evaluated
   against the guardian set at its `keyset_head_hash` (Decision
   3). Guardians added after that head never count for it;
   guardians removed-but-pending still count. Evict-then-attack
   inside one delay window therefore recovers nothing: the
   approvals must come from the honest, pre-eviction set.
3. **Everyone hears about it.** Pending removals are notified to
   all guardians and devices (Decision 4 notify duty covers
   guardian-set changes as well as recovery records). A removed
   guardian who is notified can warn the user out of band, and a
   surviving device cancels the pending change or the recovery.
4. **Bounded residual.** An attacker who holds an admin key
   _undetected for longer than the delay_ can complete an
   eviction. No record format prevents that; the mitigations are
   the delay, the notifications, and checkpoint finality (a
   checkpoint-final head cannot be displaced by a forked
   eviction branch, ADR rule 5). This residual is stated, not
   hidden: guardian recovery assumes compromise is noticed within
   72 hours.

## 7. Acceptance-test list for the implementation

Implementation MUST land tests with these names (or clearly mapped
equivalents) in `pkgs/id` test suites, in the `directory_log` /
`keyset.rs` test-vector discipline:

1. `guardian_set_update_folds_and_pins_threshold` — set + threshold
   fold; invalid thresholds (0, m > n) rejected.
2. `recovery_request_id_is_content_addressed_and_single_use` —
   replayed request_id after rotation/cancellation is rejected.
3. `approvals_below_threshold_do_not_rotate` — m−1 approvals,
   rotation record rejected by Rule R.
4. `duplicate_guardian_approvals_count_once` — two approvals from
   keys of one guardian identity count once.
5. `approval_from_non_guardian_or_device_key_rejected` — signatures
   from device keys, Seitan keys, and non-guardians do not count.
6. `recovery_rotation_revokes_all_prior_keys` — after rotation,
   old admin/device signatures verify as history but authorise
   nothing new; successor admin authorises.
7. `cancellation_by_single_device_key_aborts_recovery` — one
   device-key cancellation before the delay ends the request.
8. `rotation_before_delay_rejected` — first-seen arithmetic,
   including a backdated relay attestation (`first_seen <
created_at`) rejected.
9. `guardian_eviction_attack_fails_within_delay` — remove honest
   guardians + add sockpuppets + request recovery inside the
   window: rotation rejected (set pinned at request head;
   removal still pending).
10. `pending_guardian_removal_takes_effect_after_delay` — the
    slow path folds once the delay elapses uncancelled.
11. `server_cannot_forge_approval_or_shorten_delay` — relay-side
    tampering cases (forged approval body, altered timestamps)
    all rejected by a verifier folding from records alone.
12. `recovery_notifications_cover_all_bound_devices_and_guardians`
    — the four notification points enumerate the fold-at-head set.
13. `shamir_material_is_never_accepted` — no record type carries
    key shares; a body smuggling share-shaped fields fails
    schema validation (the exclusion is tested, not just stated).

## 8. Implementation split — what waits on #2380

#2380 is an **implementation** prerequisite, not a design
prerequisite; this record is written against its ADR's reserved
entry and fold rules, and #2380's own acceptance keeps it open
until its packets land. In order:

1. **Waits on #2380 packet 1** (`pkgs/id/src/keyset.rs` substrate:
   envelope, fold, `KeysetState`): the `GuardianSetUpdated` fold
   semantics of Decisions 2/6 and the folded `guardian_set` query.
2. **Waits on #2380 packet 5** (revocation-priority fork choice +
   checkpoint finality): Rule R's fork interaction — a recovery
   rotation is an admin-authority change and MUST require the
   checkpoint-finality treatment of ADR rule 5 wherever the log
   has forked.
3. **Independent of #2380 packets 2–4** (pairing surfaces,
   encryption sessions, genesis migration CLI) and can proceed in
   parallel once (1) exists: annex record types + directory
   relay/timestamp/notify, the recovery client flow (request →
   collect approvals → wait → rotate), and the §7 tests.
4. **Explicitly not in any packet:** Shamir/share material of any
   kind; server-side approval custody; FROST or any new crypto
   dependency (Decision 5); guardian-process cancellation
   (Decision 4, reserved only).

When (1)–(3) land, #2382's remaining acceptance item (delay +
cancel + notify implemented with tests, including the eviction
case) is discharged by tests 7–10 above; this record discharges
the specification items.

**Implementation status (2026-11-18).** Records and Rule R:
`pkgs/id/crates/id-directory/src/recovery.rs` (`check_rotation`,
`apply_rotation`, `notify_targets`). Item 2: `RecoveryEvidence` is a
`keyset::RecoveryRule`, and `keyset::fold_with_recovery` folds a
rotation that passes Rule R into the chain. At a fork it never
displaces a checkpoint-final head, and it takes no revocation
priority: an ordinary continuation of the pinned head wins, which is
Decision 3's "request is moot" case. A checkpoint taken after the
rotation (`checkpoint_final_heads_with_recovery`) makes it final, so a
late surviving-key sibling cannot undo it. Still open: delayed guardian
removals (Decision 6, needs timestamps in the fold), the directory
relay and the client flow.
