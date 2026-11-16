# ID recovery codes (custody variant c) — decision record

**Date:** 2026-11-08 · **Issue:** [#5888](https://github.com/developing-today/code/issues/5888)
**Parent:** [#5684](https://github.com/developing-today/code/issues/5684) (identity, login & delegation)
**Programme:** epic [#2056](https://github.com/developing-today/code/issues/2056)
**Siblings:** #5884 (setup ceremony), #5885 (keyless feasibility), #5886 (variant a),
#5887 (variant b), #5889 (variant d), #5890 (identity management UX)
**Status:** v1.2 decision record (2026-11-09 UTC). This revision makes the encoding,
claim lifecycle, settlement evidence, and custody-specific promises precise in response
to PR review. It does not change the selected code count, entropy, threshold, or recovery
flow. Design only — no implementation is authorised by this record alone, and
implementation timing is a separate decision per the filing.

Verified at `origin/main` `a80d206752` (2026-11-08). Prior art read:
`doc/2026-10-29T11-28-18Z_investigation_key_recovery_hosted/` (#2060),
`pkgs/id/src/issuer_rotation.rs` (#2804 recovery commitment), and
`pkgs/id/src/hosted_binding.rs` (#2046 delayed rebind).

---

## 1. What this record settles, and what it does not

The parent (#5684) already rules the direction: identity is an iroh-style keypair;
simplicity-first is a hard requirement; variant (c) is "10 security codes"; and the
all-codes-lost case must be stated plainly. The parent explicitly **delegates the
threshold and use model** ("design it: any one code? a quorum? single-use each?").

This record settles, for variant (c):

1. the code model — format, entropy, and the form in which a code is stored;
2. the threshold and use model;
3. regeneration — cadence, authorisation, and what happens to the prior set;
4. the unrecoverable-state UX, stated plainly and verbatim;
5. what recovery does and does not restore;
6. the invariants implementation must hold.

It does **not** implement anything, re-decide the parent's direction, redesign the
setup ceremony (#5884), the identity management UX (#5890), the custody variants of
their own variants (#5886/#5887/#5889), or issuer rotation (#2804).

## 2. Chosen shape, in one paragraph

At setup the identity is issued **ten independent, single-use recovery codes**. Each
code is **100 bits** of CSPRNG entropy rendered as **20 Crockford-base32 characters**
in five groups of four. For every code the setup ceremony derives a distinct Ed25519
**recovery key** and publishes only that key's **hash** — the _recovery set_: a signed,
versioned object naming the identity, a monotonically increasing `set_generation`, and
ten slot commitments. **Any one unused code is sufficient to recover**; each code works
once. Redemption is an anchored two-phase signed event — _claim_, a **72-hour** settle
window in which the current key may _cancel_, then _settle_, which installs a **new
identity key** (a rotation, never a rebind of the old key) and issues a **fresh set of
ten codes**, invalidating the old set as a whole. If every code is lost and no other
path exists, **the identity is unrecoverable**: there is no support reset, no operator
override, and no developer backdoor, and the setup screen says so in those words.

## 3. Code model

### 3.1 Format

- **Alphabet:** Crockford base32 `0123456789ABCDEFGHJKMNPQRSTVWXYZ` — no `I`, `L`, `O`,
  `U`, so the characters people confuse (`0/O`, `1/I/L`) or read as accidental
  profanity do not occur.
- **Length:** 20 characters = **100 bits** per code, printed only as five groups of
  four: `XXXX-XXXX-XXXX-XXXX-XXXX`. The canonical serialisation has no separators.
- **Canonical form:** uppercase, no separators, no whitespace. Input normalisation folds
  case, strips ASCII spaces and `-`, and folds `I`/`l` → `1`, `O`/`o` → `0`; any other
  character is refused rather than guessed. Codes are ASCII; under the #3455 plain-output
  contract they carry no `Cc`/`Cf` characters, so a code can never act on a terminal.
- **Index:** each code is labelled with its slot ("Code 3 of 10"). The index is not
  secret — it selects which commitment slot to check — but it is printed so a user can
  tell ten codes from three.

### 3.2 Entropy and key derivation

One code carries 100 bits. Guessing a code is an offline attack against a public
commitment (below), so the per-code figure — not the aggregate of ten — is the one that
matters against a thief who wants to _forge_ a code. At 2^100 derivations, guessing is
not a practical attack on any hardware horizon this design considers; it is also far
stronger than the 64-bit (GitHub) and 26-bit (Google backup codes) conventions it
replaces.

```
seed_bytes = CSPRNG(13 bytes); seed_bytes[0] &= 0x0f
             -> exactly 100 random bits (the four high bits are zero)
seed_i           = OS2IP(seed_bytes)  // big-endian unsigned integer
code_i           = encode_crockford_base32(seed_i, exactly 100 bits) -> 20 chars
identity_id_raw  = the stable 32-byte identity identifier in RecoverySet.identity
info             = ASCII("id-recovery-key-v1") || uint8(slot_i)
recovery_sk_seed = HKDF-SHA256(IKM = seed_bytes, salt = identity_id_raw,
                               info = info, L = 32 bytes)
recovery_pk_i    = Ed25519 public key from the 32-byte RFC 8032 private seed
commitment_i     = SHA-256(the 32 raw bytes of recovery_pk_i)
```

`identity_id_raw` is the stable identity identifier carried by the signed setup
artefact, encoded as its raw 32 bytes — never its hexadecimal/base32 display, a
variable-length key encoding, or the rotating current signing key. The 32-byte HKDF
output is passed unchanged as the Ed25519 private seed; the standard Ed25519 operation
derives/clamps its scalar internally. No caller pre-clamps or re-encodes the seed.

This deterministic vector is for interoperability tests only, not a credential:

| Input / output                           | Hex or text                                                        |
| ---------------------------------------- | ------------------------------------------------------------------ |
| `identity_id_raw`                        | `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` |
| `seed_bytes`                             | `000102030405060708090a0b0c`                                       |
| `slot_i`                                 | `0` (`00`)                                                         |
| printed code                             | `0082-0C20-A1G7-104G-M2RC`                                         |
| HKDF-Extract PRK                         | `79a16fb829d0002342d172baa194f1da47f39ab13c6ae8b5f1d3161cec835765` |
| `recovery_sk_seed` (HKDF output)         | `ff8bb0c3394081cb029d87da441dd3f444ea6e508e6d6c8d1d175a11e210f99a` |
| `recovery_pk_i` (raw Ed25519 public key) | `bf84a3db9047871d529efc602dfef543cf0a620fd3bc0fe944f5fe447b2f513f` |
| `commitment_i`                           | `1177777cf67e33ed69e0746c5569f0a8638d3e4ac661bffc225889c7f8407163` |

HKDF uses RFC 5869 HMAC-SHA-256: `PRK = HMAC-SHA256(salt, IKM)` and
`OKM = HMAC-SHA256(PRK, info || 0x01)` for this one-block, 32-byte output.

**The derivation consumes exactly the bits the code carries.** Generate 13 bytes and
clear the high four bits, leaving exactly 100 random bits. Interpret the result as a
big-endian 100-bit integer; `I2OSP(..., 13)` is its 13-byte big-endian form with those
four high bits fixed to zero. Encode exactly those 100 bits as 20 Crockford symbols.
At redemption, decode the 20 symbols to the same integer and perform the same 13-byte
conversion before HKDF. Thus the same seed reaches HKDF at setup and redemption, and
no bit outside the printed code influences the key. (An earlier draft drew a fresh
128-bit seed and used it for HKDF while printing only 100 bits; the other 28 bits could
not be recovered from the code, so redemption could never match the published
commitment. This revision records the necessary invariant: all derivation entropy must
be carried by the code, and any padding must be fixed, not sampled.)

Domain separation by `slot_i` means a code from slot 3 can never validate against slot 7,
and an HKDF mismatch is a failed recovery, not a silent one.

### 3.3 Storage form — the commitment set

The only recovery material that exists at rest, anywhere, is the **recovery set**:

```
RecoverySet {
  identity:        <identity public key / id>,
  set_generation:  <monotonic u64, predecessor-linked>,
  slots:           [ commitment_0 .. commitment_9 ],   // SHA-256 of each recovery_pk
  created_at:      <signed timestamp, advisory only>,
}
```

It is a signed object, created by the identity key at setup, and replicated exactly like
any other signed ID-graph object. This mirrors `issuer_rotation::recovery_commitment`
(#2804): the recovery credential appears **only as a commitment**, never in the object
and never in the definition. It also follows #2060's rule that the server stores a
digest, never a signing key.

Plaintext codes may exist only in volatile client memory while generated and displayed
during setup, a recovery-settlement ceremony, or an on-request regeneration ceremony,
and on whatever the user saves. They are never sent to the service, written to app state,
logged, included in a sync payload, included in an error report, or derivable from the
identity key, account credential, password, or commitment set. Implementations should
zero temporary buffers before releasing them where the platform permits; no claim is
made that managed runtimes can erase every copy.

### 3.4 Storage guidance given to the user

Printed at setup, in the same screen that hands over the codes:

> Print or write these down. Keep two copies in two different places — somewhere you
> would keep a passport or a spare house key. Don't keep them only on the device that
> holds your ID, and don't keep all ten in one digital note or photo: that is one place.
> A password manager is fine as one of the two copies, not as the only one.

The guidance is guidance, not enforcement: the design deliberately does not refuse to
finish setup because a user saved the codes to a single place. The informed-choice gate
is the transcription check (§7.1), which is real, rather than a nag the user clicks past.

## 4. Threshold and use model

**Any one unused code recovers. Each code is single-use. There is no quorum.**

Why any-one:

- **Simplicity-first is a hard requirement, not a preference.** A quorum asks a
  non-technical user to understand _how many_ codes they need, keep that many in
  separate places, and remember which ones they kept. The realistic failure is a user who
  keeps one code believing one is enough — and then is locked out by the design that was
  supposed to protect them.
- The security a quorum buys — "one stolen code is not enough" — is bought more cheaply
  here by the **settle window, cancellation by the current key, and notification**
  (§5). Those protect the common theft case (a photographed printout) without ever
  putting a legitimate user one lost envelope away from permanent lockout.
- A quorum also multiplies ceremony complexity: multi-code entry, partial-share state,
  and a new failure mode (some codes present, reconstruction fails).

Why single-use: a used code must not be a standing credential. Consumption is recorded in
durable signed state at settle (§5.3), never in an in-memory cache — the existing 5-minute
nonce cache and 12-hour sessions in `directory_auth.rs` are explicitly unsuitable for a
"replace my identity key" operation, exactly as the #2060 investigation found.

**Threshold is not configurable per identity.** The parent fixes ten codes; this record
fixes any-one. A per-identity choice would make the plain-language statement conditional
("if you chose quorum mode…") and re-open the lockout failure.

## 5. Redemption

### 5.1 Claim

The claimant enters one code on a device that will hold the new key. The client derives
`recovery_sk_i`, then signs and publishes:

```
identity.recovery.claim {
  identity, set_generation, slot,
  new_key,                 // raw 32-byte key this recovery would install
  signature,               // recovery-key signature over claim_signing_bytes
  claim_id,                // derived metadata; excluded from signed/hashed fields
}
```

For interoperability, `claim_fields` is the deterministic CBOR array
`[identity_id_raw, set_generation, slot, new_key_raw]` (RFC 8949 deterministic
encoding; byte strings for both keys, unsigned integers for generation and slot).
`claim_signing_bytes = ASCII("id-recovery-claim/v1") || 0x00 ||
canonical-CBOR(claim_fields)`. The derived recovery key signs those exact bytes with
Ed25519. Then `claim_id = SHA-256(ASCII("id-recovery-claim-id/v1") || 0x00 ||
claim_signing_bytes || signature)`. The journal stores the fields, signature, and
derived `claim_id`; neither domain-separated byte string includes `claim_id`, so
cancellation and settlement can refer to it without a circular hash.

The claim **must be anchored** — recorded at a monotonic position in the durable store
that holds the identity's chain — and accompanied by the trusted time certificate in
§13. The journal position proves order; the certificate, not a client or server wall
clock alone, proves elapsed time. The claim is public: peers can verify the claimant
holds the precommitted key for that slot, which prevents an authentication service from
fabricating a recovery claim.

A claim does not consume the code. An abandoned or cancelled claim leaves the set intact.

### 5.2 Window, cancellation, notification

- **Settle window: 72 hours** (`RECOVERY_SETTLE_SECS = 259_200`). This is a settled
  parameter, like `REBIND_WAITING_PERIOD_SECS` in `hosted_binding.rs`: an implementation
  must not shorten it locally, and changing it is a design change.
  - Not immediate: a stolen or photographed printout is the realistic compromise, and
    the window plus notice plus cancel is the only defence that costs the legitimate user
    nothing.
  - Not the 30-day delayed-rebind figure: that path presents **no credential at all**;
    this one proves possession of a 100-bit precommitted secret. 72 hours is the shortest
    window that survives a weekend with the user offline.
- **Cancellation:** the current-generation identity key may publish
  `id-recovery-cancel/v1 || claim_id` at any point before settle. **Cancel wins**, the
  claim is dead, and the slot is unchanged. Only the current key can cancel; nobody else
  can, and no one can cancel a settle after it lands.
- **Notification:** every accepted claim notifies every registered contact channel of the
  identity (directory account emails, binding notification channels) with the claim id,
  slot, and how to cancel. Honest limit, stated in the record and in the notice: this
  helps a user who still holds a working key. It cannot help a user who has lost the key
  and the codes — for them the notice is a record, not a rescue.
- **Rate limits and expiry:** claims are rate-limited per identity (default: 5 accepted
  claims per rolling 24 hours, 3 live claims concurrently), and failed attempts at an
  ingress are throttled like any other unauthenticated input. A claim may settle during
  the seven days after its 72-hour wait; after that it expires and can never settle.
  Expired, cancelled, or superseded claims stop counting toward the concurrent limit.
  Expiry does not consume the code or slot, so the user may open a new claim with the
  same or another unused code. Cancelled and expired claims still count toward the daily
  limit. Rate limits are a mitigation, not a security property: the cryptographic check
  is the commitment.

### 5.3 Settle — rotation, not rebind

After the window, and before the claim expires, the claimant publishes
`identity.recovery.settle`, naming the claim, the new key and the new key generation,
**countersigned by the new key** (the successor-proof pattern from `issuer_rotation.rs`).
Before any state changes, validators atomically require all of the following: the claim
is anchored and not expired or cancelled; its time certificate proves the full 72-hour
window; its `set_generation` equals the current `RecoverySet.set_generation`; and its
slot is still unspent and matches the derived recovery-key commitment. A regeneration or
another recovery that has advanced the set makes every old-generation claim stale.
Settle then:

1. installs the new key as the current generation;
2. marks the used slot **consumed** in durable signed state (monotonic, replicated);
3. increments `set_generation` and publishes a **fresh set of ten codes**, generated in
   the same ceremony on the recovering device;
4. invalidates the previous set in its entirety.

If any precondition fails, settlement is rejected with **no state change**: no key is
installed, no slot is consumed, no generation advances, and no replacement set is
published. A stale claim cannot win by racing a regeneration.

This is the #2060 rule applied literally: **recovery rotates the identity key; it never
rebinds an existing key**, and no authentication event (SSO, password, email link) can
stand in for it. Actions signed by the old generation remain valid history — they were
valid when signed — and remain attributable to the old generation in the raw view.

### 5.4 What recovery restores, and what it does not

Recovery restores **signing authority over the identity**: the ability to act as the
identity, publish, and issue new delegations. It does **not** restore:

- secrets that died with the lost device — group encryption keys, local drafts, anything
  encrypted to a key that no longer exists (the group-encryption roster in
  `encryption/roster.rs` rotates on membership change; codes are not a group-key backup);
- pseudo-login sessions or delegations issued under the old generation — those are
  revoked by the key change (§6.3);
- access to a hosted account credential. The code recovers the _identity_, not the
  account. A hosted binding may still need its own rebind (that is #2046's path, and it
  is not replaced by this one).

Saying this plainly at recovery time is part of the design, not an implementation note:
"you are back as yourself; the files that were only on the old device are gone."

## 6. Regeneration

### 6.1 Triggers

- **Always, as part of settle.** A settled recovery ends with a fresh set of ten on the
  recovering device, and the old set is dead. Reissuing the whole set — rather than
  marking one of the remaining nine spent and keeping the rest — is deliberate: the
  printout that produced the used code may itself have been exposed, and the user is at
  that moment in the best position to print a new one. It costs one more print, and it
  removes a class of "the other nine are still sitting in a drawer I no longer trust".
- **On request**, authorised by the current identity key: lost or exposed printout,
  moving house, custody migration (a↔b), or a routine yearly refresh.
- **On suspected exposure**, same authorisation.

### 6.2 Authorisation

Only a key authorised for the **current key generation** may sign
`identity.recovery.regenerate/v1 || {identity, expected_set_generation, new_set}`. A
recovery code never authorises a standalone regeneration: a code authorises a _recovery_,
and recovery ends with a fresh set. This keeps one rule — a code's only power is to
replace the key, and replacing the key always replaces the set.

### 6.3 Atomicity, conflicts, and delegations

- A set is replaced **as a whole**. No partial or mixed-generation set ever becomes
  current; a request whose `expected_set_generation` is stale is refused.
- `set_generation` is monotonic and predecessor-linked. The identity journal serializes
  competing current-key-authorized requests with compare-and-swap on the expected
  generation: the first complete valid request durably accepted as the sole successor
  wins and advances the generation; other requests against that predecessor are stale
  and rejected. The owner can then use the current key to issue another whole-set
  regeneration against the new generation, without recovery or key rotation. If the
  journal cannot establish one canonical order, it fails closed and no candidate set
  becomes current until ordering is restored.
- A key-generation change (any settle) **revokes pseudo-login delegations issued under
  the previous generation**. Actions already taken under those delegations stay in the
  record, with provenance showing the pseudo-login acted under delegation from the old
  generation. This is the parent's variant (d) rule composed with this one: the friendly
  UI renders "you", the raw view never forgets which generation authorised the action.

## 7. Unrecoverable-state UX (verbatim)

### 7.1 At setup — a real informed choice

The setup ceremony shows, before the codes are handed over:

> **Keep these ten codes: they can get you back in if you lose access to your current
> key.**
> If you still have your current key, you can keep using this ID and authorize a new
> code set. If you no longer have access to that key, one unused code from the most
> recently generated set can start recovery on a new device; recovery needs an online,
> anchored claim and a 72-hour wait. If you have no access to the current key, no unused
> code from the current set, and no other recovery path, nobody can unlock this ID. Not
> us, not support, not the developers. There is no reset and no back door. Your ID is
> unrecoverable.
>
> If a service holds your current key, it can keep acting as you and can cancel a code
> recovery before it settles. These codes do not force that service to let you leave.
> Read the service-held-key warning before choosing that custody option.

Then the ceremony requires the user to **type back one randomly chosen code**, and refuses
to make the set current until it matches. This is the informed-choice gate: it proves the
codes are legible and transcribable, which is the actual precondition for "you can recover
this", and it is not a checkbox anyone can click past. (A set a user cannot read is a set
that does not exist.)

### 7.2 At loss — when no key and no unused code exist

> **This ID can't be recovered.**
> You have no access to its current key and no unused code from its most recently
> generated recovery set. There is no reset — not by us, not by support, not by the
> developers. This is what the codes were for.
> You can create a new ID now, but it will be a different identity: your name, worlds,
> grants and history stay attached to the old ID, and nothing can sign as it again.
> If you find an unused code from the most recently generated set, it can still start
> recovery after the online claim is anchored; the full 72-hour wait still applies.
> Codes from an older set or a code already used will not work. Keep looking before you
> give up on it.

Rules for the surface:

- **No support escape hatch may be shown or offered.** A "contact support to recover"
  button in this state is a lie the product cannot honour and is forbidden by this record.
- Support documentation, help-centre copy, and dashboards must not promise recovery.
  Anyone who writes "we can help you get back in" without a key or a code is writing a
  false statement.
- The screen must not silently create a new identity in the background. Creation is the
  user's explicit choice, made after reading that it is a different identity.

### 7.3 The inverse statement

The same copy must state the positive case, so the user knows what _does_ work: one
unused code from the current set can start recovery from any device; an anchored claim,
trusted time evidence, and the full 72-hour wait are required, and the current key can
cancel before settlement. A user who still has the current key can instead authorize a
replacement set. A design that only says "you can be locked out forever" teaches fear,
not custody. Show the unrecoverable screen only when neither a usable current key nor
another configured recovery path remains; it is not the service-held-key outage screen.

## 8. Interaction with the custody variants

- **(a) Service holds the key.** A code can propose a user-chosen key and take the
  identity out of service custody only if the service does not cancel the public claim
  before settlement. The service holds the current key and can veto each pending claim
  or regenerate the set, so codes are **not an independent or guaranteed exit** from an
  uncooperative custodian. They also do **not** stop a compromised service from acting
  as the user while it holds that key. A guaranteed exit against a hostile custodian
  would require a different custody design and is not adopted here; do not promise one
  in variant-(a) setup or UX copy. This preserves #5684's service-held-key variant while
  making its trust boundary explicit. SSO/account custody can be an account pointer,
  never a rebind authority.
- **(b) User holds the key.** Codes are the primary and only recovery path; ordinary
  recovery is the key file itself. A lost key file plus a surviving code is exactly the
  case this design exists for.
- **(c) This design.** Codes recover the root identity. The key installed at settle lives
  in whichever custody the user selects at recovery time, defaulting to the identity's
  current variant. Choosing user-held at settle is possible from (a) only if the service
  allows the claim to settle. The migration surface itself belongs to #5890.
- **(d) Pseudo-login identities.** Codes do not recover a pseudo-login. They recover the
  root identity, and the key change revokes the pseudo-logins issued under the old
  generation (§6.3). A user who lost only a delegated device should revoke that
  pseudo-login, not burn a recovery code.

## 9. Coordination with siblings

- **#5884 (setup ceremony), checked fresh 2026-11-09.** It remains OPEN with setup
  decisions in PR #7850. Its ceremony generates ten codes, transcription-proves one,
  and publishes the `RecoverySet` before presenting the identity as created. This record
  owns the 20-character Crockford format (printed as five groups of four), count, KDF,
  threshold, and regeneration rules; #5884 owns ceremony ordering and the print screen.
  The interface is the `RecoverySet` object above; the setup PR's proposed interface is
  consistent with this record.
- **#5890 (identity management UX), checked fresh 2026-11-09.** It remains OPEN with no decision record yet. It consumes the qualified §7 copy, the current-set status, and the regeneration entry point; #5890 owns surface and migration presentation. That integration is follow-up work, not absorbed here.
- **#5885 (keyless feasibility), re-checked fresh 2026-11-09.** It is OPEN and has
  a decision record in its 2026-11-08T12:23:36Z comment. It recommends a random ID
  signing key generated and held client-side; OIDC as account authentication/pointer or
  augmented PAKE (such as OPAQUE) for password login; and no server-held unwrapped key
  or key capable of decrypting a backup. It explicitly allows an independently
  authorised recovery mechanism. This result **does not obsolete this record**: these
  codes are that explicit recovery authority, work independently of everyday login, and
  let a client recover after losing the device-held key without giving the authorizer
  key custody. Redemption and key derivation therefore remain client-side; neither OIDC
  nor password authentication alone substitutes for a code or surviving key. The
  chosen 100-bit code entropy is independent of password entropy. It also sharpens the
  variant-(a) boundary: a client-held recovery secret does not make recovery independent
  when the service holds the current key and can cancel the claim.
- **#5886 / #5887 / #5889 (variants a, b, d).** Each must state what recovery and
  revocation mean in its own variant, consuming §8 rather than redesigning it.

## 10. Parent (#5684) decisions

**No parent decision is changed by this record.** The parent explicitly delegates the
threshold and use model, and says the identity is unrecoverable when all codes and every
other recovery path are unavailable. A still-usable current key can authorize a new code
set. Two elaborations that stay inside the parent's direction:

1. A code authorises a **key rotation**, not a rebind. This follows #2060 and the parent's
   own "login UX indistinguishable from ordinary logins" requirement without letting an
   ordinary login ever rebind a key.
2. **Custody may be chosen at settle**, so a cooperative variant-(a) service can permit
   recovery to double as an a→b migration. The parent already lists a→b migration as a
   flow, so this does not change its direction. This record does not promise an
   independent exit from a hostile service; that would change the parent-level custody
   guarantee and must return to #5684 for a new decision. The UI for a permitted
   migration is #5890's.

If a later answer here would change a parent decision, it must go back to #5684 as a new
comment rather than silently forking the parent — none is raised now.

## 11. Alternatives rejected, and why

| Alternative                                                    | Why rejected                                                                                                                                                                                                                          |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **k-of-n quorum** (e.g. 2-of-10)                               | Simplicity-first and lockout risk: users keep one code believing one suffices and are then locked out. The single-stolen-code threat it addresses is handled by the settle window, cancellation, and notification at lower user cost. |
| **One long recovery passphrase**                               | Human-chosen secrets lose entropy in practice, get reused, and add a second password to remember. Ten printed 100-bit codes are both stronger and easier.                                                                             |
| **Service-held recovery key**                                  | Makes the service a custodian of the identity and collapses variant (c) into variant (a); breaks LIST 7 and #2060's "SSO never rebinds a key".                                                                                        |
| **Codes as a password reset**                                  | Recovers the account row, not the identity key — exactly the failure #2060 documents. The code must install a new identity key, never a new password.                                                                                 |
| **Email magic link / SSO as the sole recovery path**           | Requires the service to be able to rebind, which makes the "nobody can" statement false. Usable as an account pointer; never as recovery authority.                                                                                   |
| **Social / guardian recovery (Shamir among guardians)**        | Parent non-goal; also depends on stable guardian identities that do not exist yet.                                                                                                                                                    |
| **Passkey / hardware key as recovery**                         | Parent non-goal; a plausible future _additional_ slot type, not v1.                                                                                                                                                                   |
| **Mark one code spent and keep the other nine**                | Leaves a printout in a drawer the user may no longer trust after a recovery event. Reissuing all ten at settle costs one print and removes the class.                                                                                 |
| **Treat codes as a guaranteed exit from service-held custody** | The service-held current key can cancel a pending claim or authorize regeneration. Promising an exit against that custodian would exceed variant (a)'s stated trust model; a new parent-level decision is required.                   |
| **Immediate recovery with no wait**                            | A photographed printout becomes an instant permanent takeover with no notice to the legitimate holder. The 72-hour window is the cheapest defence that costs the honest user nothing but time.                                        |
| **Support override / break-glass**                             | Destroys the honest statement and the whole point of the variant. If ever proposed, it is a new design decision and a new record, not an implementation detail.                                                                       |

## 12. Invariants implementation must hold

1. **Commitment-only at rest.** The only recovery material stored, synced, or served is
   the `RecoverySet` of slot hashes and its generation. Plaintext codes are never written,
   logged, telemetered, or sent.
2. **Irreversibility.** Codes are not derivable from the identity key, the account
   credential, a password, the commitment set, or any stored state.
3. **Set shape.** Exactly ten slots. The current set always has ten unspent slots — a
   settle consumes its slot and publishes a fresh ten, so a shrinking set is never
   current.
4. **One code, one slot, one key.** Derivation is domain-separated by slot; a code from
   one slot never validates against another.
5. **Single use, durably.** Consumption is recorded in durable signed state, replicated
   with the chain — never only in memory.
6. **Recovery is rotation.** Settle installs a _new_ key and increments the generation; it
   never re-authorises a retired key and never rebinds an existing key binding.
7. **Any one suffices.** Exactly one unused code is sufficient. No other code, secret,
   support action, or operator capability can substitute.
8. **Anchored ordering and trusted elapsed time.** A settle requires a claim at a
   monotonic anchored position plus valid quorum time certificates proving the complete
   72-hour interval; caller-supplied or lone-server wall-clock time never authorises it.
9. **Cancel wins.** The current-generation key's cancellation beats any pending claim;
   nothing cancels a settled rotation.
10. **No backdoor.** No operator, developer, support, or break-glass path reconstructs a
    code or key, decrypts the codes, or forces a recovery. A future path of that kind is
    a design change, because it changes the plain-language statement.
11. **Atomic regeneration.** A new set replaces the whole set or nothing; no
    mixed-generation set is current; journal compare-and-swap selects one successor, and
    stale requests are rejected.
12. **Printable and canonical.** Codes are uppercase Crockford base32 in five groups of
    four, ASCII, free of `Cc`/`Cf` characters (#3455); input normalisation folds case and
    separators only and refuses anything else.
13. **Delegations follow the generation.** A key-generation change revokes pseudo-login
    grants issued under the previous generation; completed actions remain, with
    provenance.
14. **Notice and limits.** Every accepted claim notifies registered channels, and claims
    are rate-limited per identity.
15. **Transcription is proven.** The set does not become current until the user has typed
    back a randomly chosen code correctly.
16. **Bounded claims.** A claim expires seven days after its 72-hour wait elapses;
    expired/cancelled/superseded claims cannot settle, do not consume codes, and release
    concurrent-claim capacity.
17. **No stale-set settlement.** Settlement atomically checks the claim's generation and
    unspent slot against the current set before any mutation; mismatch leaves all state
    unchanged.
18. **Canonical claim identity.** Claim signing bytes exclude `claim_id`; the id is
    derived only after signing from the specified domain-separated bytes.

## 13. Open questions for implementation (with defaults)

| Question                                                              | Default this record sets                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Which store and time evidence anchor the settle window?               | Every claim is durably appended to the identity-chain journal at a monotonic position. For elapsed time, the network configures three independent witness keys, not controlled by the current key custodian; at least two sign receipts binding `identity_id`, journal position, event hash, witness-set version, and UTC time. Each witness has a documented clock-error bound of ±60 seconds, with at most one faulty witness. Validators require a 2-of-3 certificate both at claim and settle, using the same configured set. To prove the full wait, `min(settle receipt times) - 60s` must be at least `max(claim receipt times) + 60s + 72h`. The journal position proves event order; conservative witness-time bounds prove elapsed time. Without a durable anchor or valid time certificate, a client may prepare locally but no claim is accepted or settled; recovery remains unavailable until evidence is available. There is no offline/advisory bypass. |
| Where do notification channels come from?                             | The directory account's emails and the hosted binding's channels; an identity with neither gets the claim recorded but no notice, and the UX says so.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Does a cancelled claim count against the daily claim limit?           | Yes — otherwise cancellation is free rate-limit evasion.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| How is the print screen implemented (PDF, plain text, browser print)? | #5884's call; this record only requires legible grouped codes, the slot labels, and the §7.1 copy.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| What happens to the old generation's published objects?               | They remain valid history and remain attributable; no retroactive invalidation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Should slot indices be randomised rather than 0–9?                    | No: the index is not secret, and sequential labels are easier to read and to track.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

## 14. Handoff and definition of done for this design ticket

- This record is linked from #5888.
- #5884 owns the setup artefact and must publish the `RecoverySet` before declaring the
  identity created; its current proposal is consistent with the shape settled here.
- #5890 consumes the qualified §7 copy and regeneration rules; that sibling remains
  open and is not part of this task.
- No implementation is started; timing stays a separate decision.
