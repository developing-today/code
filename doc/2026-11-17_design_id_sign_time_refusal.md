# Sign only what receipt would accept — the sign-time refusal contract for `pkgs/id`

Design record for issue #6487 (design-architect pass 60, 2026-11-06;
audit at origin/main `252d43085`). This record commits the contract to
the repo exactly as the issue states it — nothing here is re-decided.
All seven signer filings the contract names have landed (status table
below, verified fresh 2026-11-17); what was missing, and what this
record supplies, is the contract itself as a committed design record.

> Source-of-record note: issue #6487 points at a full design doc at
> `files/designs/code-id-sign-time-refusal.md` in the
> keep-the-four-repos-healthy goal workspace. That path does not exist
> on the laptop and the document could not be read, so the issue body
> is the source of record for everything below. The open questions the
> issue defers to that design doc (revoke rule set vs #4071;
> verify-side defense in depth for #6447/#4346; validate() author
> rule; extension beyond `pkgs/id`) remain open questions of record on
> #6487 and are not settled here.

## Problem

`pkgs/id` states its validity rules on the receipt side (fold, viewer,
verifier, accept) and most of its signing entry points never consult
them, so a caller can mint a properly signed, signature-valid object
that no consumer will ever count — creation reports success,
consumption silently ignores. The crate has been fixing this one
signer at a time (#5623 artifacts, landed; #6441 attestation outcome
domain, landed 2026-11-06 via PR #6442). Seven filings in five days
are the remaining signers: #6422 (currency `sign_authorisation` never
calls `validate`), #6424 (attestation signers never check the signer
is a player), #6434 (match_audit `sign_header`/`sign_footer` check
nothing `verify_integrity` enforces), #6443 (directory_auth
`sign_request` signs any nonce, infallibly), #6447 (`sign_move`:
blank world / empty input), #6451 (`sign_rotation`: routine
self-rotation the fold can never count), #6469
(`SignedReplicationEnvelope::sign`: eight envelope-local `accept`
rules unchecked).

## The contract

**Sign only what receipt would accept.**

1. Mirror at sign time every receipt rule decidable from the signer's
   own inputs (key, object, existing arguments).
2. The decidability line is part of the contract: (a) intrinsic —
   object alone; (b) signer-relative — key vs object; (c)
   seam-extended — decidable only with one more input the seam
   conceptually has: the public seam changes, it never silently
   skips; (d) receiver-state — stays receipt-only, named with its
   reason in the closing inventory.
3. Refusal speaks the module's own vocabulary, naming the rule;
   wire formats and receipt verdicts unchanged.
4. A test bypass, where needed, is crate-private and named
   `*_unchecked` (the artifact pattern) — never a second public
   unchecked signer.

## Pins the phases carry

- #6422: `sign_authorisation` calls `RateAuthorisation::validate`
  for the signing key's issuer; refusal via `SignError`.
  `sign_revoke` unchanged (no revoke validator exists — Phase 4
  disposition).
- #6443: `sign_request` becomes **fallible** (pin: not a validated
  nonce newtype — all callers are in-crate tests/helpers); the
  nonce-domain predicate is defined once, shared by `sign_request`
  and `verify_signed`.
- #6424: `sign_genesis` refuses a non-player signer
  (`SignError::NotAPlayer`). `sign_claim` is class (c): **pin the
  artifact pattern** — a public checked entry point taking the
  agreement/player set; bare `sign_claim` becomes crate-private
  `sign_claim_unchecked`. #6441's intrinsic outcome check applies
  in both.
- #6434: `sign_header` refuses wrong version, invalid roster,
  non-roster signer; `sign_footer` refuses non-roster signer and
  a `seed_reveal` not digesting to `header.seed_commitment` — with
  the `AuditErrorV0` reasons. Log-context checks stay in
  `verify_integrity`.
- #6447: `sign_move` refuses blank `world_id` (core.rs:291 rule)
  and empty `input` (core.rs:1938). Input length stays
  receipt-only (per-world limit unknown at the seam).
- #6451: `sign_rotation` refuses a Routine rotation whose
  `successor_key` equals the signing key's issuer id. Recovery
  analogue stays fold-only (inputs don't name the replaced key).
- #6469: `SignedReplicationEnvelope::sign` returns `None` for
  every envelope-local `accept` rule (the origin's eight rows);
  its doc comment states the full refusal contract. Receiver-state
  rules stay in `accept`.

## Receipt-only inventory (class (d), named with reasons)

The rules the contract deliberately leaves on the receipt side:

- `sign_revoke` (#6422 pin): unchanged — no revoke validator exists;
  its disposition belongs to the Phase 4 closing inventory.
- #6447 input length: receipt-only — the per-world limit is unknown
  at the signing seam.
- #6451 recovery analogue: fold-only — the signer's inputs don't
  name the replaced key.
- #6469 receiver-state rules: stay in `accept` — they depend on
  receiver state the signer does not have.
- #6434 log-context checks: stay in `verify_integrity` — they are
  properties of the log context, not of the signed header/footer
  alone.

Phase 4 (child issue #6488, the closing sign-surface inventory)
dispositions every public signing entry point in `pkgs/id` against
this contract — enforced / deliberately receipt-only (class-(d)
reason named) / gap-filed — with required dispositions including
`sign_revoke`, `directory::sign_up`/`sign_up_keyed`, the #3231
builder symmetry, and `sign_unchecked`; the contract statement lands
in the crate's documentation there.

## Phases (adopt the origins — no parallels)

- **Phase 1 — call the validator that exists:** #6422 + #6443.
- **Phase 2 — signer membership:** #6424 + #6434. Independent of Phase 1.
- **Phase 3 — world and rotation signers:** #6447 + #6451 + #6469.
  Independent of Phases 1–2.
- **Phase 4 — closing sign-surface inventory + contract statement**
  (child issue #6488, gated on Phases 1–3).

## Filing status (verified fresh 2026-11-17)

| Filing | Signer                                    | Closing PR | Merged (UTC)         | Merge commit                               | Issue state |
| ------ | ----------------------------------------- | ---------- | -------------------- | ------------------------------------------ | ----------- |
| #6422  | currency `sign_authorisation`             | #8677      | 2026-11-11T14:15:04Z | `a222c693b7309f817987830f2173847074ab106d` | CLOSED      |
| #6424  | attestation `sign_genesis` / `sign_claim` | #6559      | 2026-11-11T16:17:05Z | `f998d270d3a5994a6346fc553e651ad721a89bbb` | CLOSED      |
| #6434  | match_audit `sign_header` / `sign_footer` | #6497      | 2026-11-06T10:24:09Z | `c0ce60b173f8520d5a52b49da3ec7b5cda1180c7` | CLOSED      |
| #6443  | directory_auth `sign_request`             | #6539      | 2026-11-09T01:05:53Z | `3bd39e6f2ab4eae168904b1da6b48f628b0d74cb` | CLOSED      |
| #6447  | `sign_move`                               | #6544      | 2026-11-11T03:07:31Z | `ef3f1c10d2c46a68d36431009e983719254d9082` | CLOSED      |
| #6451  | `sign_rotation`                           | #6500      | 2026-11-06T11:20:19Z | `85c435ec6c59950f59b773776785ee78046ab69c` | CLOSED      |
| #6469  | `SignedReplicationEnvelope::sign`         | #8011      | 2026-11-09T08:55:13Z | `6c5af5b86a4704bfd2448e7833b6db0e7d8bdc48` | CLOSED      |

## Acceptance criteria (from #6487)

- Every public signing entry point in pkgs/id refuses at sign time exactly the objects the receipt side (fold, viewer, verifier, accept) would not count, under one shared refusal contract — no signer-local validity rules remain.
- Sign-time refusal errors name the violated receipt rule, so a caller can fix the object before signing.
- The closing sign-surface inventory (child issue #6488) shows every public signing entry point dispositioned against the contract, and property tests assert sign-accept ⇒ receipt-accept for generated objects.
