# ADR: keyset / authorization log for `id` identity (#2380)

**Date:** 2026-11-12 · **Issue:** [#2380](https://github.com/developing-today/code/issues/2380) · **Parent epic:** #2056
**Status:** Proposed (fleet design default; supersedes the "needs a Drew format decision" block on task 8502 — the 2026-11-12 queue cleanup released that escalation because ADR record-format/signing design is fleet design work)
**Inputs:** #2380 proposal; #2060 key-recovery investigation
(`doc/2026-10-29T11-28-18Z_investigation_key_recovery_hosted/`); #2044 §4.3
option 1 (hosted IdP claim bindings, implemented as `BindingRecord` in
`pkgs/id/src/hosted_binding.rs`); the existing signed-log pattern in
`pkgs/id/src/directory_log.rs`.

## Problem

`pkgs/id` identity is a single Iroh Ed25519 secret key (`src/local.rs`):
the public key IS the account, invites name it directly (`src/invite.rs`),
and there is no indirection for device keys, revocation, IdP bindings, or
rotation. A lost key is a lost identity; a second device is a second
account. #2380 asks for a signed keyset/authorization log as the identity
substrate. This ADR decides the record formats, the fold/verification
rules, and the identity identifier derivation — acceptance item 1 of
#2380. Implementation is scoped as follow-up packets (§7).

## Decision 1 — identity identifier derivation

**The identity identifier is the hex Ed25519 public key of the genesis
admin key, and it never changes.**

- For a migrated single-key account, the genesis admin key IS the
  existing key, so the identifier equals the public key that invites,
  directory entries (`AccountKeyed { id, name }`, `directory.rs`), and
  group records already name. Nothing that references an identity today
  needs to change.
- For a new account, the key that signs the genesis record as admin
  becomes the identifier.
- The identifier is a name, not an authority: after an admin-key
  rotation or revocation, the genesis key may hold no power at all.
  Authority is always computed by folding the log (§4), never from
  possession of the identifier key alone.

Rejected alternative: `SHA-256(genesis record)` as the identifier. It
decouples the name from any key, but it breaks every existing reference
(invites and directory records name public keys), forces a translation
layer at every surface, and buys nothing the fold does not already
provide — a rotated-away genesis key is powerless either way.

## Decision 2 — record envelope (extends `directory_log::LogRecord`)

The keyset log deliberately reuses the envelope shape, canonicalisation,
and merge order of `pkgs/id/src/directory_log.rs`, adding a chain link:

```rust
pub struct KeysetRecord {
    /// Hex Ed25519 public key of the signer.
    pub author: String,
    /// Lamport counter, as in `directory_log` (merge order:
    /// `(lamport, author, hash)`).
    pub lamport: u64,
    /// Hex SHA-256 of the previous record in this identity's chain;
    /// empty string on the genesis record.
    pub prev_hash: String,
    /// Hex SHA-256 over the signed body.
    pub hash: String,
    /// The keyset change.
    pub entry: KeysetEntry,
    /// Hex Ed25519 signature over the signed body.
    pub signature: String,
    /// For `DeviceEnrolled` only: hex Ed25519 signature by the device
    /// key being enrolled, over the same body (proof of possession).
    pub countersignature: Option<String>,
}
```

The signed body is the canonical `serde_json` encoding of
`{ author, lamport, prev_hash, entry }` — byte-for-byte the
`directory_log::body_bytes` discipline, so the two logs share one
canonicalisation story and one set of test vectors. `hash` is the
SHA-256 of those bytes; the signature is over those bytes.

## Decision 3 — entry types (`KeysetEntry`)

| Variant | Fields | Admin-only? |
| --- | --- | --- |
| `Genesis { admin_key, device_key, label }` | hex pubkeys; first record, `prev_hash = ""`, `author == admin_key` | self-signed |
| `DeviceEnrolled { device_key, label }` | hex pubkey, human label; requires `countersignature` by `device_key` | yes |
| `DeviceRevoked { device_key, reason }` | hex pubkey, free-text reason | yes |
| `AdminKeyRotated { new_admin_key }` | hex pubkey; signed by an admin key authorised at the parent state; the new key countersigns (proof of possession) via `countersignature` | yes |
| `IdpBindingBound { binding_id }` | references a `hosted_binding::BindingRecord` by its immutable `binding_id`; the binding record itself stays the artefact connecting an IdP `sub` to this identity (#2044 §4.3 option 1) | yes |
| `IdpBindingRevoked { binding_id }` | as above | yes |
| `GuardianSetUpdated { guardians, threshold }` | guardian identity identifiers (Decision 1 form), k-of-n threshold; **record type reserved now**, semantics land with the social-recovery follow-up so device enrolment never blocks on it | yes |
| `Checkpoint { state_hash, record_count }` | hex SHA-256 of the canonical folded state (§4); lets a replica verify a snapshot without replaying from genesis | no (any authorised device key) |

Everyday signing (messages, group records, blob-gate membership) is
NOT a keyset entry: it is authorised by presenting a signature from any
device key the fold currently shows as enrolled and not revoked.

## Decision 4 — fold and verification rules

Folding is deterministic and position-based, mirroring `directory_log`:
a `Log`-style container holds only records that verify, merge is set
union keyed by `(lamport, author, hash)`, and every replica holding the
same records reaches the same state.

1. **Chain order.** Records of one identity form a chain via
   `prev_hash`. The fold replays the canonical chain starting at the
   unique `Genesis` record. A record whose `prev_hash` does not match
   the current head is not yet foldable (it belongs to a fork or is
   out of order — see rule 5).
2. **Per-record verification.** `hash` recomputes from the canonical
   body; the signature verifies under `author`; where a
   countersignature is required (`DeviceEnrolled`,
   `AdminKeyRotated`) it verifies under the enrolled/rotated key.
   Any failure excludes the record and everything chained on it.
3. **Authorisation is judged at the record's position.** An admin
   entry is valid only if `author` was an authorised admin key in the
   state folded from all preceding records. A device key revoked at
   position *n* cannot authorise records after *n*, but records it
   signed before *n* remain valid history. Wall-clock time plays no
   part in validity.
4. **Folded state.** `{ identity_id, admin_keys: {key → Active|Rotated|Revoked}, devices: {key → {label, enrolled_at, revoked}}, idp_bindings: {binding_id → Bound|Revoked}, guardian_set: Option<{guardians, threshold}>, head_hash, records_folded }`.
   Derived queries: `is_device(key)`, `is_admin(key)`,
   `state_hash()` (SHA-256 of the canonical state encoding, the value
   `Checkpoint` records carry).
5. **Forks.** Two valid records extending the same head are a fork.
   The canonical continuation is the one with the smaller merge key
   `(lamport, author, hash)`; the other branch is retained as evidence
   but never folded. Because admin authority is judged at position,
   a fork created by a key that the canonical branch later revokes
   cannot resurrect itself: its records still fold only if authorised
   at their position on the canonical chain — and they are not on it.
6. **Checkpoints.** A replica may start from a `Checkpoint` whose
   `state_hash` it has verified against a full fold once, then fold
   only the suffix. A checkpoint never authorises anything by itself.

## Decision 5 — migration of existing single-key accounts

An existing account migrates by appending a `Genesis` record signed by
its existing key, enrolling that same key as both the genesis admin key
and the first device key. Identity identifier = that key's public key
(Decision 1), so migration is invisible to invites, the directory, and
group membership. Migration is lazy (on first keyset-aware use) and
idempotent: a second genesis for the same identity is a fork under
rule 5 and loses deterministically to the first folded one.

## Decision 6 — scope boundaries

- Keyset-admin operations are narrower than everyday signing: only
  admin keys enrol/revoke devices, rotate admin keys, change guardian
  sets, or bind/revoke IdP claims. Device keys never gain admin power
  by being enrolled.
- SSO/IdP state never mints or rebinds keys by itself (the #2060 rule):
  an `IdpBindingBound` entry is an admin-signed pointer to a
  `BindingRecord`; the hosted binding ceremony's own rebind path
  (`RebindRecord`, signed by a surviving key) is unchanged.
- Per-device group-encryption sessions (`src/encryption/`) consume the
  folded device set; they do not define their own enrolment.

## 7. Implementation follow-ups (separate packets, in order)

1. `pkgs/id/src/keyset.rs`: `KeysetRecord`/`KeysetEntry`, sign/verify,
   fold, `KeysetState` + queries, unit/property tests against the
   `directory_log` test-vector discipline (fork determinism,
   revocation-at-position, countersignature enforcement).
2. Enrolment/revocation of a second device key through the existing
   `src/qr.rs` / `src/invite.rs` pairing surfaces (QR payload carries
   the enrolment body for the countersignature).
3. `src/encryption/` per-device sessions read the folded device set
   instead of ad-hoc key lists.
4. Genesis migration for single-key accounts (lazy, §5) plus CLI
   surface (`id keyset show|enrol|revoke|rotate`).

Acceptance items 2–4 of #2380 map to packets 2, 3, and 4 above;
packet 1 is the substrate they share. #2380 stays open until all four
land.
