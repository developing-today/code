# D4 slice 2: bundle grants and broker renewal

Part of #13889 (D4, sneakernet bundles), the last of its five slices. Builds
on slice 1 (the format, which reserved manifest tag `0x08`), slice 3 (import
order and creator binding), slice 4 (the admission function and the
`held_no_read_grant` hold) and slice 5 (carriers).

Decision this slice implements, recorded on #13889 on 2026-11-25: bundle
grants are **recipient-bound by default**; **bearer grants are allowed only
as an explicit opt-in with a short, stated lifetime**. The record names its
undo: a later design PR can change the default before implementations rely on
it.

Acceptance in #13889: "the renewal-collision cases enumerated (grant outlives
renewal, renewal revokes mid-bundle-lifetime) each with a stated outcome".
They are in §5.

Reference: `grants_reference.py` beside this file, stdlib only, loads the
slice 1, 3, 4 and 5 references. `python3 -I grants_reference.py` runs the
self-test. Slice 1's `parse_manifest` and `read_a`, and slice 3's
`import_bundle`, gained a `grants` switch that is off by default, so those
references still behave as older readers and their self-tests are unchanged.

## 1. What a bundle grant is

A bundle grant lets the receiver read the pairs **this bundle carries** in
one area. Without it, slice 4 holds those pairs as `held_no_read_grant`: the
bytes have arrived, but nothing shows them or passes them on.

It is one delegation step down from a read capability the broker issued to
the device that signs the bundle:

- **Parent:** a broker-signed read capability `{access: "read", area, from,
grantee, until, sig}`, the short-window capability composition A hands a
  device at check-in. `grantee` is the device key.
- **Grant:** `{area, from, until, parent}`, plus `recipient` when it is
  recipient-bound. Its area is inside the parent's area, and its window is
  inside the parent's window.
- **Signature:** none of its own. The grant sits in the manifest, which the
  bundle signature covers, and the parent must name the bundle's signing key
  as grantee. So the bundle signature is the delegation signature, and the
  domain prefix from slice 1 already separates it from anything else the key
  signs.

Three properties follow, and each answers part of #13889's "a bundle must not
become a way to mint permanent authority":

1. **Scoped to these objects.** A grant releases only the pairs in the same
   bundle. It does not make the area readable for later bundles, and it never
   lets sync deliver the area. Sync still needs the receiver's own read
   capability from the broker.
2. **Bounded by renewal.** A grant can never last longer than the parent's
   window at the moment it was made. The parent is a short-window broker
   capability, so a bundle grant inherits composition A's revocation latency
   and never extends it.
3. **No onward delegation.** The parent must be broker-issued and held by
   the signing device. A recipient who received a grant has no broker
   capability for that area, so it cannot write a grant of its own. A grant
   copied into another bundle fails, because that bundle is signed by a
   different key.

## 2. Format (normative)

Tag `0x08` is critical, as slice 1 reserved it. A slice-1 or slice-3 reader
rejects a bundle that has it (`grants_reserved_slice_2`). That is the right
failure for an older reader.

```
grants = count:u8 (1-16), then count x grant
grant  = mode:u8 len:u32 body
mode   = 0x01 recipient-bound | 0x02 bearer
body   = canonical JSON object, keys sorted, no spaces
         recipient-bound: {area, from, parent, recipient, until}
         bearer:          {area, from, parent, until}
parent = {access, area, from, grantee, until, sig}
```

- Grants are strictly ascending by `(mode, body)`, so a duplicate is
  impossible and two writers of the same grants write the same bytes.
- `recipient` is `keyset:<identity ID>`, slice 3's identity form. It names a
  person, not a device, so any of the recipient's enrolled devices can apply
  it.
- `area` is printable ASCII, 1 to 1024 bytes. Times are u64 unix seconds.
- The body is canonical JSON so that the same bytes are hashed and compared
  everywhere. Like slice 4's envelope model, it stands in for the real
  encoding: the Meadowcap-on-p2panda binding is still unbuilt (composition A,
  "what remains unbuilt"). The rules below do not depend on the encoding.

One recipient-bound grant costs 441 bytes in the reference. That is about one
chat-sized object on a QR page code (slice 5), so a QR bundle normally carries
no grant; media and email have room for many.

## 3. Where grants are checked

Grants add one stage-1 check and one step inside slice 3's stage 5:

| Stage | Grant check                                                     | Outcome if it fails       |
| ----- | --------------------------------------------------------------- | ------------------------- |
| 1     | grant structure (§2)                                            | the bundle is rejected    |
| 2–4   | signature, creator binding, digests: unchanged from slice 3     | as slice 3                |
| 5     | each grant's meaning (§4), then the pairs go to slice 4's admit | that grant is not applied |

A malformed grant rejects the whole bundle, like any other malformed
manifest field: the writer was broken, and the person should get a fresh
copy. A well-formed grant that fails its check is only not applied. The
objects import either way, because they are signed history (slice 3 §7).
Their pairs are then held, as they would be with no grant at all.

## 4. Per-grant check, in order

| Code                        | Meaning                                                                     |
| --------------------------- | --------------------------------------------------------------------------- |
| `bundle_expired`            | the bundle's own `expires_at` has passed (slice 3's "expired" row)          |
| `grant_parent_unverified`   | the parent is not signed by a broker this host trusts                       |
| `grant_parent_not_read`     | the parent is not a read capability                                         |
| `grant_parent_not_signer`   | the parent was issued to a device other than the one that signed the bundle |
| `grant_area_outside_parent` | the grant's area is not inside the parent's area                            |
| `grant_window_invalid`      | the grant's window is empty                                                 |
| `grant_outlives_parent`     | the grant starts before or ends after the parent's window                   |
| `bearer_lifetime_too_long`  | a bearer grant longer than 7 days                                           |
| `grant_parent_withdrawn`    | this host knows the broker withdrew the parent (§5, case 4)                 |
| `grant_not_yet_valid`       | the receiver's clock is before the grant's `from`                           |
| `grant_expired`             | the receiver's clock is at or after the grant's `until`                     |
| `grant_not_for_you`         | recipient-bound, and the recipient is not this host's identity              |
| `applied`                   | the pairs this bundle carries in the grant's area pass slice 4's step 6     |

The window is checked against the **receiver's clock at import**, because
that is when authority is used. `created_at` is the creator's claim and plays
no part. An applied grant still goes through slice 4's other steps:
subscription and verb filters apply as usual.

Structural codes at stage 1: `grants_malformed`, `unknown_grant_mode`,
`grants_not_ascending`.

## 5. Renewal collisions

The window rule is what keeps a grant inside composition A's renewal
discipline. Each case has a stated outcome, and each is in the self-test.

| #   | Case                                                                                                                      | Outcome                                                                                                                                                                                                         |
| --- | ------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Grant outlives renewal:** the writer asks for longer than its parent                                                    | Not applied, `grant_outlives_parent`. Monotone attenuation: a delegation can never widen its parent, in area or in time.                                                                                        |
| 2   | **Renewal revokes mid-bundle-lifetime:** the broker stops renewing the creator's read capability after the bundle is made | The grant stays good until its own `until`, which can be no later than the parent's end at the time it was made. This is composition A's revocation latency, unchanged. It is why bearer grants are short.      |
| 3   | The parent is renewed after the bundle is made                                                                            | No effect. Renewal never extends a bundle grant; the old grant still ends at its own `until`. A fresh bundle, or a grant made from the renewed parent, is the renewal path.                                     |
| 4   | The broker explicitly withdrew the parent, and the receiver knows it before import                                        | Not applied, `grant_parent_withdrawn`. "Knows" means the receiver holds the broker's withdrawal record, by sync or by another bundle. Without it, case 2 applies.                                               |
| 5   | The receiver imports after the grant's `until`                                                                            | Not applied, `grant_expired`. The objects import and their pairs are held.                                                                                                                                      |
| 6   | The receiver later gets its own read capability for the area from the broker (overtaken by renewal)                       | The broker capability governs the area from then on, by sync. The bundle grant only ever covered its own pairs, so the two cannot conflict.                                                                     |
| 7   | The creator's signing device is revoked in its keyset before import                                                       | The whole bundle is rejected at slice 3's stage 3 (`signing_key_revoked`). Grants are never looked at.                                                                                                          |
| 8   | Any of 2–7 happens **after** the import                                                                                   | No retroactive effect. The released pairs stay readable, exactly as data already replicated by sync stays readable when a read capability lapses (D3's "the dead stay readable only where already replicated"). |

## 6. Recipient-bound and bearer, as built

- **Recipient-bound (default).** Only a host whose identity is the
  recipient applies it. In the self-test, Ben gives Ana a bundle with a
  recipient-bound grant for his `ben/notes`. Ana's host releases the note,
  including from the hold an earlier grant-less bundle left it in. The same
  file imported on Cat's host (a copied stick) reports `grant_not_for_you`
  and the note stays held.
- **Bearer (explicit opt-in).** Any host that imports it in time applies it.
  Its lifetime is at most 7 days (`BEARER_MAX_SECONDS`, a designer default
  for "short", inside the decision's bound), and still inside the parent's
  window. The export flow has to ask for it by name; there is no path that
  writes a bearer grant by default. In the self-test, a 3-day bearer grant
  releases the note on Cat's host, and an 8-day one is refused.
- **What the import shows.** For a bearer grant, the person sees who made it
  and until when anyone holding the file can read it. For a recipient-bound
  grant meant for someone else, they see that it was not for them.

## 7. Honest limit: a grant is authority, not secrecy

The object bytes in a bundle are not encrypted. A recipient-bound grant
stops every host that follows these rules from showing, admitting or passing
on the pairs. A copied stick therefore gives a stranger no standing in the
graph. But anyone can read the plaintext bytes off the stick with a tool
that ignores grants. This is the same as composition A's read enforcement
being cooperative, which the p2panda investigation states: "peers must
honour read capabilities; data already replicated is already read."

If the sneakernet case needs confidentiality on a lost stick, the answer is
to seal the area's pairs to the recipient's enrolled device keys (for
example HPKE to each key in the folded keyset). That is a format addition:
new entry kinds for sealed objects, with the digest over the ciphertext. It
does not change this slice's grant rules. It is not designed here, and it is
the one open item D4 leaves.

## 8. Alternatives weighed

- **A grant that makes the area readable from then on.** This would let
  a bundle turn into a standing read capability that sync honours. It is
  exactly the "permanent authority outside renewal" the issue rules out.
  Rejected; scoping to the bundle's own pairs is what keeps sync and renewal
  the only standing path.
- **A separate signature on each grant.** It would let a grant be checked
  without its manifest. It is unnecessary here, because the receiver records
  the outcome at import and never re-checks the grant later. It also adds 64
  bytes per grant. Rejected for v1; a real Meadowcap delegation encoding may
  carry its own signature, and that changes nothing in §4.
- **Grants from any capability in a chain (transitive delegation, as
  Meadowcap allows).** Rejected: it would let a recipient re-grant what it
  was handed, which is onward delegation with no broker in the loop.
- **Rejecting the whole bundle for an invalid grant.** Rejected for
  well-formed grants: the objects are signed history and import either way,
  as slice 3 §7 already settled for an expired grant.
- **No grants (data only).** Weighed in #13889 and rejected there: handing
  someone a readable slice would always need a second, online step.

## 9. What is left for implementation

- The Meadowcap-on-p2panda binding for the parent capability and the
  broker's withdrawal record (composition A's unbuilt parts).
- A Rust importer that agrees with this reference on the self-test's
  bundles, as slice 1 §9 already suggests for the format.
- The export flow's bearer opt-in wording in the app.
- Sealed objects for confidentiality (§7), if wanted.
