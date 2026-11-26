# D4 slice 3: export and import flows, creator binding, rejection catalogue

Refs #13889 (D4, acceptance item 3). Builds on slice 1 (the format), slice 5
(`slice-5-carrier-profiles.md`) and the keyset authorization log: the ADR in
`doc/2026-11-12T19-10-07Z_design_keyset_authorization_log/` (#2380), already
implemented in `pkgs/id/crates/id-directory/src/keyset.rs`.

Reference: `import_reference.py` (stdlib only; it loads the slice 1 and
slice 5 references beside it). `python3 -I import_reference.py` runs 35
catalogue cases through one importer. `--table` prints binding sizes and what
they do to the QR ceilings.

## 1. What this slice decides

- What the creator binding (manifest tag `0x04`) carries, and how a reader
  checks it offline.
- The form of `creator_identity`.
- The order an importer checks things in, and the outcome of every failure:
  rejected, held for the person, or imported with a note.
- The export flow that produces a bundle this importer accepts.

Grants stay slice 2's (they wait on Drew's bearer or recipient-bound
decision). Merge on contact stays slice 4's. Section 7 shows that the one
grant row in this catalogue, an expired grant, has the same outcome under
either answer.

## 2. Who made the bundle: the creator binding

The bundle signature proves only that some device key signed it. The binding
proves that the key belongs to the creator's identity. It uses the keyset log,
because that is where the id app already records which devices an identity has
and which it has revoked:

- **Identity:** `creator_identity` is `keyset:` followed by the identity ID,
  which is the hex genesis admin key (keyset ADR Decision 1). It never
  changes, even after the admin key rotates.
- **Device:** the bundle's `signing_key` must be a device key that is enrolled
  and not revoked in the identity's folded keyset state.
- **Who it is to the reader:** a name or face comes from the receiver's own
  contacts. The bundle never introduces a new identity by itself (§5, held).

This changes slice 1's example form `idp:example/ben`. An IdP name cannot be
checked offline: proving it needs the hosted binding service's record and the
IdP. The keyset ID can be checked from the bundle and the receiver's own
records alone, which is the point of a sneakernet. An IdP binding still exists
inside the keyset log (`idp_binding_bound`), so nothing about the IdP story is
lost.

### 2.1 Binding format

```
creator_binding = kind:u8 body
kind 0x01 keyset      count:u16 (1-256), then count times (len:u32 record)
                      record = one KeysetRecord in its serde JSON form
                      (keyset.rs), in chain order from the genesis record
kind 0x02 fleet-ca    reserved until fleet-cert has revocation and an ID
                      leaf profile (#9157): held
kind 0x03 keyset-ref  identity (32 bytes) + head record hash (32 bytes)
other kinds           rejected (unknown_binding_kind)
```

- **`keyset`** carries the writer's whole current chain: genesis, every
  enrolment, rotation and revocation. A receiver that has never synced the
  creator's keyset learns it from the bundle. A receiver that has, learns
  whatever is newer. Each record must be in its one canonical JSON encoding,
  the same bytes keyset.rs hashes and signs.
- **`keyset-ref`** names the chain instead of carrying it, for carriers too
  small for the chain (§8). The receiver must already hold the records through
  that head. If it does not, the bundle is held until the records arrive by
  sync or by another bundle.
- **`fleet-ca`** is the issue's "fleet CA" path. `pkgs/fleet/crates/fleet-cert`
  can already issue and validate X.509 chains. Revocation, trust
  distribution and a leaf profile that ties a certificate to an ID identity
  are not built yet (children of #9157). Without revocation, an offline
  reader could not refuse a lost device's certificate, so a reader holds such
  a bundle rather than guessing. The body format is defined when those
  exist.

### 2.2 Checking it

The receiver folds the union of the bundle's records and its own, starting at
the genesis it already trusts for that identity, with keyset.rs's rules:

- a record folds only if it verifies (canonical body, hash, signature, the
  required countersignature and no other);
- its lamport is above the head's;
- its author is an active admin key at that position;
- checkpoints never advance the head;
- a fork is settled by keyset.rs's revocation priority and checkpoint
  finality.

The receiver's own records are part of the union. So a revocation the receiver
already knows always counts, and a bundle cannot hide it by carrying an older
chain.

## 3. Export flow

1. The person picks what to share. That is a D1 set URI, which becomes the
   bundle's `scope` (slice 2 of D1).
2. The writer resolves the set to objects. It includes dangling link targets
   only as links (slice 1 §6).
3. It writes the binding: the full chain (`keyset`), or `keyset-ref` when the
   carrier profile is too small for the chain and the person is handing it to
   someone who already has their keyset.
4. It signs with the device's own key, the key the fold shows as enrolled.
5. Slice 5 checks the carrier profile and splits into independent parts.

## 4. Import flow and verification order

Each stage runs only if the one before passed. The first failure is the code
the person sees.

| Stage | Checks                                                                               | Reads                                      |
| ----- | ------------------------------------------------------------------------------------ | ------------------------------------------ |
| 0     | carrier unwrap (slice 5: QR text, email attachment, file)                            | the carrier                                |
| 1     | header, manifest structure, ceilings, critical tags                                  | header and manifest only                   |
| 2     | bundle signature over the manifest                                                   | manifest                                   |
| 3     | creator binding (§2)                                                                 | manifest and the receiver's keyset records |
| 4     | total length and every object digest                                                 | every object byte                          |
| 5     | accept: keep new keyset records, hand objects to graph ingest, list dangling targets | —                                          |

Why this order:

- **Cheapest first.** A bundle on a stick can be gigabytes. Stages 1 to 3
  touch only the manifest, so an unsigned, unbound or revoked bundle is
  refused before a single object is hashed.
- **Nothing untrusted before the signature.** The binding is read only after
  the signature shows the manifest is what the key signed.
- **Objects last, and all or nothing.** The manifest covers each object's
  digest, so a mismatch means the copy is damaged or tampered with. The
  importer refuses the whole bundle, and the person re-copies it (§9).

Stage 5 hands each object to the same graph ingest that network sync uses.
Each object's own D3 signature and author are checked there, exactly as for
sync, and that is slice 4's ground. Keyset records the receiver did not have
are kept, so a revocation carried by one bundle applies to the next.

## 5. Rejection catalogue

The outcomes are **rejected** (nothing kept), **held** (kept aside, not in
the graph, shown to the person with the reason) and **imported**.

| Code                                                       | Stage | Outcome                 | What the person sees                                                                               |
| ---------------------------------------------------------- | ----- | ----------------------- | -------------------------------------------------------------------------------------------------- |
| slice 1 structure codes (`bad_magic` … `bad_renewal_note`) | 1     | rejected                | not a readable bundle                                                                              |
| `grants_reserved_slice_2`                                  | 1     | rejected                | made by a newer version (until slice 2)                                                            |
| `unknown_signature_alg`, `bad_signature`                   | 2     | rejected                | the bundle was changed after signing                                                               |
| `unbound_creator`                                          | 3     | rejected                | no proof of who made it                                                                            |
| `binding_malformed`, `unknown_binding_kind`                | 3     | rejected                | the proof of who made it is unreadable                                                             |
| `binding_record_invalid`                                   | 3     | rejected                | a keyset record in it is forged or altered                                                         |
| `binding_chain_broken`                                     | 3     | rejected                | its keyset records do not form a chain from the genesis                                            |
| `binding_identity_mismatch`                                | 3     | rejected                | the identity it names is not the one its proof is for                                              |
| `signing_key_not_enrolled`                                 | 3     | rejected                | signed by a device that is not one of that identity's devices                                      |
| `signing_key_revoked`                                      | 3     | rejected                | signed by a device that identity has revoked                                                       |
| `unknown_identity`                                         | 3     | **held**                | "From someone you have not added. Add them to import it."                                          |
| `binding_records_missing`                                  | 3     | **held**                | "Waiting for this person's device list. It arrives when you sync or get another bundle from them." |
| `binding_kind_unavailable`                                 | 3     | **held**                | "Signed with a fleet certificate this app cannot check yet."                                       |
| `digest_mismatch`, `truncated_objects`, `trailing_bytes`   | 4     | rejected                | the copy is damaged; copy it again                                                                 |
| expired bundle (`expires_at` passed)                       | 5     | imported with a note    | "Expired: the messages were added, but its access grants were not."                                |
| dangling link target                                       | 5     | imported, target listed | "Mentions 1 item you do not have yet."                                                             |

The four cases the issue names are all here: bad signature, digest mismatch,
unbound creator, and an expired grant.

### 5.1 Decisions behind the outcomes

- **A revoked device is refused even for old bundles.** `created_at` is the
  creator's own claim, so a lost or stolen device could backdate it. The
  keyset log gives no position for a bundle, so "signed before the
  revocation" cannot be shown. The person can ask the creator for a fresh
  bundle from a device that is still enrolled.
- **Absent binding is a rejection.** Slice 1 reported it as `unbound` so the
  format could be exercised. Slice 1 said slice 3 would make it a rejection,
  and this slice does. Slice 1's `example.idbundle` is therefore refused by a
  slice-3 importer. It stays the format example; `import_reference.py` builds
  bound bundles from the same objects.
- **Holding, not trusting on first contact.** A bundle from an identity the
  receiver has never accepted could be anyone's. Importing it would let a
  file on a stick create a contact. Holding it lets the person decide. Their
  yes trusts that genesis, and the held bundle then imports through the same
  flow (the self-test does exactly this).

## 6. What a hold keeps

A held bundle is stored as it arrived, with its code, outside the graph. It
is released by the event its code names: the person adds the identity, the
keyset records arrive, or fleet certificates become checkable. Then it is re-run from
stage 1. A held bundle never imports partially.

## 7. The expired-grant row does not depend on slice 2's open question

Under a bearer grant or a recipient-bound one, an expired grant is simply not
applied. The objects in the bundle are signed history and import either way.
Drew's decision changes who may use a live grant. It does not change this row.

## 8. What the binding costs on a QR code

Real binding sizes from the reference fixture, and the slice 5 ceilings they
leave for chat-sized objects (145 bytes each):

| Binding                                             | Bytes | QR page code | QR small code |
| --------------------------------------------------- | ----- | ------------ | ------------- |
| `keyset-ref`                                        | 65    | 9            | 3             |
| `keyset`, genesis only (the first device)           | 554   | 7            | 1             |
| `keyset`, genesis + one enrolment (a second device) | 1,225 | 4            | 0             |
| `keyset`, genesis + enrolment + one revocation      | 1,768 | 1            | 0             |

This corrects slice 5's guess of a 96-byte floor: a carried chain is several
hundred bytes per record, because keyset records are hex JSON. So a QR export
uses `keyset-ref` by default, which fits people who already know each other.
Media and email carry the full chain.

## 9. Alternatives weighed

- **Import the objects whose digests match, skip the damaged ones:** useful
  for a large, partly damaged stick. Rejected for v1 because slice 1's
  readers refuse the whole bundle, and "partly imported" would need its own
  report and retry story. It can be added later without a format change.
- **Accept bundles signed before a revocation, using `created_at`:**
  rejected, because the time is a claim (§5.1).
- **Trust on first contact:** rejected (§5.1).
- **An IdP name as `creator_identity`:** rejected because it cannot be
  checked offline (§2).
- **No binding, relying on each object's own D3 author:** objects carry
  author keys, but the same device-to-identity question applies to them, and
  the bundle grant (slice 2) needs one answer for the whole bundle.
- **Carry only the records needed (genesis to the enrolment):** smaller, but
  it would drop later revocations of other devices that the receiver might
  not know about. The full chain is what makes a bundle a way to deliver
  keyset updates as well.

## 10. Honest limits

- The reference folds linear chains only. On a fork it stops with
  `fork_needs_keyset_fold`, because keyset.rs already implements revocation
  priority and checkpoint finality and the importer calls that fold.
- The reference builds keyset records in Python with the canonical body that
  keyset.rs's own field-order test pins. It has not yet been run against the
  Rust code. The first implementation step is a Rust test that verifies the
  reference's records.
- Chains longer than 256 records need a checkpoint-based proof (keyset ADR
  rule 6). That is left open.
