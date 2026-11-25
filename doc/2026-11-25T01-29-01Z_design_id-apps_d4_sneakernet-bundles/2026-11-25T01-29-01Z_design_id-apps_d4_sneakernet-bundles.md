# D4 slice 1: sneakernet bundle manifest and contents format

Refs #13889 (D4, sneakernet bundles for ID). Parent epic #13880.
Inputs: D3 graph layer (#13885, landed in PR #13997), D1 id:// v2 shapes
(slice 1 landed in PR #14725), carrier context #13752.

Reference: `bundle_reference.py` (stdlib only) is a writer and two readers.
`example.idbundle` is the hand-built example (2835 bytes, sha256
`95f95205826df1419619663572e0698eb2a3133485fa81cc275263bf031b7e93`).
`python3 -I bundle_reference.py` checks the example and 26 rejection cases
against both readers; `--cross-check` also checks the BLAKE3 and Ed25519 code
against the `blake3` and `cryptography` packages when they are installed.

## 1. What this slice decides, and what it leaves

Decides: the bundle's byte layout, the manifest fields, the signing rule, the
digest rule, size ceilings, the bundle ID, and how a reader rejects.

Leaves to later slices, with the manifest slot already reserved:

| Slice | Topic                                                                     | Slot or hook in slice 1                                                                                  |
| ----- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| 2     | Bundle grants and broker renewal                                          | tag `0x08 grants`; slice-1 readers reject a bundle that has it                                           |
| 3     | Import order, creator binding against fleet CA / IdP, rejection catalogue | tag `0x04 creator_binding` is carried and signed; slice 1 reports it as `unbound` or `present-unchecked` |
| 4     | Merge on contact                                                          | none needed: contents are plain content-addressed objects                                                |
| 5     | Carrier profiles (media, email, QR)                                       | §5 format ceilings; carriers only tighten them. Done: `slice-5-carrier-profiles.md`                      |

Bearer versus recipient-bound grants is Drew's decision (#13889 open question
1). This record does not settle it. §7 puts the options in plain words.

## 2. Shape in one paragraph

A bundle is one file: a fixed header, a signed manifest, and then the object
bytes. The manifest lists every object by algorithm-tagged digest, size and
kind, in digest order. The objects follow in that same order, back to back.
A reader can check everything offline: the header, the manifest structure, the
signature over the manifest, the total length, and each object's digest. It
checks before it trusts anything in the bundle.

## 3. Byte layout (normative)

All integers are big-endian.

```
bundle        = magic version manifest_len manifest_body sig_alg signature objects
magic         = "IDBUNDLE"                       ; 8 bytes
version       = u16                              ; 1
manifest_len  = u32                              ; <= 1 MiB
manifest_body = *record                          ; tags strictly ascending
record        = tag:u8 len:u32 value
sig_alg       = u8                               ; 0x01 = Ed25519
signature     = 64 bytes                         ; over DOMAIN || manifest_body
objects       = the bytes of each contents entry, in index order, concatenated
DOMAIN        = "id-bundle/1" 0x00 "manifest" 0x00
```

Nothing may follow the last object.

### 3.1 Manifest records

Tags `0x01`–`0x7f` are critical: a reader that does not know one rejects the
bundle. Tags `0x80`–`0xff` are non-critical: a reader ignores an unknown one,
and it is still covered by the signature, so it cannot be stripped or added
in transit.

| Tag    | Name             | Value                                                  | Presence                                |
| ------ | ---------------- | ------------------------------------------------------ | --------------------------------------- |
| `0x01` | format           | `id-bundle/1`                                          | required                                |
| `0x02` | creator_identity | UTF-8, the IdP-stable identity, no control characters  | required                                |
| `0x03` | signing_key      | key alg `0x01` (Ed25519) + 32 bytes                    | required                                |
| `0x04` | creator_binding  | opaque proof that the key belongs to the identity      | optional in slice 1; checked in slice 3 |
| `0x05` | created_at       | u64 unix seconds; the creator's claim                  | required                                |
| `0x06` | expires_at       | u64 unix seconds, later than created_at                | optional                                |
| `0x07` | contents         | count:u32, then entries (§3.2)                         | required, count 1–4096                  |
| `0x08` | grants           | reserved for slice 2                                   | must be absent                          |
| `0x09` | scope            | ASCII D1 URI naming the exported slice, e.g. a `head/` | optional                                |
| `0x81` | renewal_note     | UTF-8 human note on renewal                            | optional                                |

### 3.2 Contents entries

```
entry = alg_len:u8 alg digest_len:u8 digest size:u64 kind:u8
```

- `alg` is `raw-blake3-256` in v1, the same profile D1 uses, and `digest` is
  32 bytes. An unknown algorithm is a rejection, not a skip. Adding WILLIAM3
  (still open estate-wide) is a new `alg` string, with no layout change.
- `kind` is 1 content object, 2 link object, or 3 blob. A blob is the media
  bytes that a D3 media body `{digest, size, mime}` points at; on a carrier
  with no network, inline is the out-of-band path.
- Entries are strictly ascending by digest bytes, so a duplicate object is
  impossible and two writers of the same set produce the same manifest bytes.
- `size` is the exact object length; the reader knows the total length before
  it hashes anything.

### 3.3 What an object's bytes are

The carried bytes are exactly the bytes whose hash is the object's D3
`object_id`: the canonical encoding without the `object_id` field (D3 §2.3).
So the entry digest _is_ the object ID, and a bundle needs no second
identifier. D3 has not fixed that byte encoding yet. The format does not
depend on it, and the example uses compact JSON as stand-in bytes, labelled
as such in the script.

## 4. Signing and the bundle ID

- The signature is Ed25519 over `DOMAIN || manifest_body` by `signing_key`.
  The domain prefix keeps a bundle signature from being replayed as any other
  signed thing the same key makes.
- The signature covers the manifest, and the manifest covers each object by
  digest and size, so one signature check plus per-object hashing verifies the
  whole bundle. Objects keep their own D3/p2panda signatures; the bundle does
  not re-sign them.
- The bundle ID is BLAKE3-256 of `manifest_body`, written as the D1 portable
  object URI `id://any/graph/v2/obj/raw-blake3-256.<hex>`. It is derived, not
  a field, so a manifest cannot claim an ID it does not have. It names exactly
  one set of contents, creator and lifetime; re-exporting with a new expiry is
  a new bundle ID.
- `signing_key` is a device key. Whether it speaks for `creator_identity` is
  the `creator_binding` question, checked in slice 3 against the fleet CA /
  IdP. This follows the SSB lesson in #13889 and D3 §7: identity is the IdP
  identity, not one device key. A slice-1 reader reports an absent binding as
  `unbound` rather than rejecting, so the format can be exercised before the
  binding check exists. Slice 3 makes it a rejection.

## 5. Size discipline

Format ceilings, which every carrier profile may tighten (slice 5,
`slice-5-carrier-profiles.md`):

- manifest at most 1 MiB;
- at most 4096 entries;
- each object at most 16 MiB (bigger media stays out of band through its D3
  media body).

A bundle is a scoped slice, named by its optional `scope` URI, not a history.
No entry depends on any earlier entry, so a bundle of objects that are pruned
later still verifies: there is no chain to break. This answers the SSB
"append-only, no pruning" lesson in #13889.

## 6. Dangling links (designer decision, recorded here)

A bundle MAY carry a link object whose target it does not carry. The example
has one: a `mention` whose target is absent. Reasons:

- D3 already treats backlinks as computed and links as valid on absent
  targets; refusing them in bundles would make bundles stricter than sync.
- A scoped slice that had to close over every target would grow without
  bound, which is the SSB mistake this design avoids.
- Verification is unaffected: the link object's own bytes are digested like
  any other object.

Slice 3's import report will list such targets as "dangling, resolve later",
not as rejections. Slice 4 says how they resolve on contact.

## 7. Bearer versus recipient-bound grants (Drew decides; slice 2)

In plain words:

- **Bearer:** whoever holds the file can read what it grants. Handing over a
  USB stick is enough. If the stick is copied or lost, every copy works until
  the grant runs out.
- **Recipient-bound:** the grant names one person's identity. Only that person
  can use it; a copied stick is useless to anyone else. The creator has to
  know who it is for before making it.
- **Both, with bearer opt-in:** recipient-bound by default, bearer only when
  the creator explicitly asks and with a short lifetime. #13889 already says
  bearer must never be the default by accident.

Either way, a bundle grant is bounded by composition A's renewal discipline;
slice 2 enumerates what happens when a grant outlives, or is overtaken by,
broker renewal.

## 8. Rejection codes (slice 1)

Both readers produce the same code for every case in the self-test.

| Code                                                                                                     | Cause                                      |
| -------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| `bad_magic`, `unknown_version`                                                                           | not a v1 bundle                            |
| `truncated_header`, `truncated_manifest`, `truncated_signature`, `truncated_objects`, `truncated_record` | file ends early                            |
| `trailing_bytes`                                                                                         | bytes after the last object                |
| `manifest_too_large`, `too_many_entries`, `object_too_large`                                             | §5 ceilings                                |
| `records_not_ascending`, `entries_not_ascending`                                                         | duplicate or out-of-order tag or entry     |
| `unknown_critical_tag`, `missing_required_field`, `unknown_format`                                       | manifest structure                         |
| `grants_reserved_slice_2`                                                                                | grants present before slice 2 defines them |
| `bad_signing_key`, `unknown_signature_alg`, `bad_signature`                                              | signature                                  |
| `unknown_digest_alg`, `bad_digest_length`, `unknown_kind`, `bad_contents`, `empty_contents`              | contents index                             |
| `digest_mismatch`                                                                                        | an object's bytes do not hash to its entry |
| `bad_creator_identity`, `bad_time`, `expiry_before_creation`, `bad_scope`, `bad_renewal_note`            | field values                               |

Reader A checks the manifest structure before the signature, and reader B
checks the signature after the structure too; neither trusts a field it has
not parsed. An expired manifest is a status (`expired`), not a rejection, in
slice 1: what expiry means for grants is slice 2's question.

## 9. The "two independent readers" acceptance

Reader A parses into a `Manifest` and verifies; reader B is one forward pass
written from §3 with no shared parsing code. They share only the BLAKE3 and
Ed25519 primitives, which are checked against official vectors (BLAKE3 empty
and `abc`, RFC 8032 test 1) and against the `blake3` and `cryptography`
packages by `--cross-check`. Both agree on the example's bundle ID and object
list and on all 26 rejection cases.

Honest limit: both readers were written by the same author in one sitting.
The stronger form of the acceptance is a reader in another language, such as
the Rust id crate, agreeing on `example.idbundle`. That is the natural first
step of the implementation issue.

## 10. Alternatives weighed for the format

- **Tar or zip container:** familiar, but several encodings for one set of
  files, metadata like mtimes leaking in, and no single canonical byte
  sequence to sign. Rejected for the signed part; a carrier may still wrap a
  bundle in one.
- **CBOR/DAG-CBOR manifest:** compact and typed, but brings a dependency and
  its own canonical-form rules. The TLV here is about 60 lines to read in any
  language. A later version can switch with a new `format` value.
- **Sync transcript (replay the op log):** rejected in #13889 itself: couples
  the file to the sync protocol and imports ops the receiver may not be
  allowed to replay.
