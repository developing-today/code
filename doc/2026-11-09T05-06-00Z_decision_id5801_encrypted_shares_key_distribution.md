# Decision: encrypted shares & key distribution — share shape, key holders, rotation, recipient proofs

- **Date:** 2026-11-09
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5801](https://github.com/developing-today/code/issues/5801), "id: design: encrypted shares & key distribution (from #5672)". Queue task 14235.
- **Parent design:** [#5672](https://github.com/developing-today/code/issues/5672) — ID selective sharing and foreign data-store adapters v1. This record is decomposition item 3 of the parent ("Design: encrypted shares & key distribution; composition with sensitivity classification"). It does not re-decide the parent; §1 lists what it takes as given.
- **Consumed, not re-decided:** the grants model detail is ticket [#5799](https://github.com/developing-today/code/issues/5799) (named as the grants-model design in the #5804 record, §1); public shares are [#5802](https://github.com/developing-today/code/issues/5802) (§7 states the boundary); the foreign data-store adapter contract is [#5803](https://github.com/developing-today/code/issues/5803) (§7). Sibling record: `doc/2026-11-08_design_id5804.md` (IndexedDB adapter).
- **Substrate:** group encryption already implemented in `pkgs/id/src/encryption/` — the p2panda-encryption `DataScheme` adapter (`scheme.rs`), one `Session` per device and group, with `roster`, `welcome` (history cut to `full` or `recent:N`), `history`, and `ordering` modules. Its epoch discipline is load-bearing for D3: **the epoch rotates only on create, update, add-by-join, remove, and recovery — no timer rotates it (N7); the module takes no clock.**
- **Evidence baseline:** `origin/main` at `129df4af83`, worktree `~/git/fleet/14235`, 2026-11-09. Verified at that baseline: issue #5801 is OPEN with no linked PR and no decision record in `doc/` (triage verdicts of task 10204, 2026-11-05, and task 13870, 2026-11-08, both confirm the deliverable is unwritten); the only `doc/` mention of key distribution is cross-reference text, not a design.

## Decision summary

| Question | Decision |
| --- | --- |
| Share shape | An encrypted share is the granted subset encrypted under a **per-share key**, in one of two modes recorded in the share manifest: **readable** (grantees receive the key and can decrypt) or **sealed** (storing/forwarding peers hold ciphertext only and never receive the key) (D1) |
| Who holds keys | The owner always; readable-mode grantees hold the current epoch key; sealed-mode peers hold no key. Keys are delivered wrapped to each holder's identity key, never peer-to-peer (D2) |
| Rotation | Event-driven epochs, reusing the group scheme's discipline: the share epoch advances on grantee add, grantee remove/revocation, and explicit owner rotation — never on a timer. Re-encryption is lazy; old epochs stay readable by those who legitimately held them (D3) |
| Revocation | Cryptographic take-back is **forward-only** in v1: revocation cuts off all future epochs and all sealed ciphertext completely; it cannot retroactively unread epochs whose keys the grantee already held. This answers the parent's open question — forward take-back is the v1 promise, retroactive take-back is not promised (§D3, §5) |
| Recipient proofs | Every share carries an owner-signed **share manifest** (grant lineage, granted-set commitment, epoch, classification ceiling, expiry). A recipient can prove provenance, integrity, and completeness *within the grant* — and nothing about anything outside it (D4) |
| Composition | Effective shareability = sensitivity-classification ceiling ∩ grant ∩ share mode. Classification decides what is shareable; the grant decides who; encryption decides who can decrypt. Encryption may only tighten, never widen (D5) |

## 1. What the parent already fixed (not re-decided here)

From #5672, taken as given:

1. **The grants model** — per-object and per-collection grants (per-table/per-database as shorthand expanding to collections); grant dimensions of who / what / operations / duration (standing, expiring, one-shot snapshot); delegation defaults to no and must be explicit, attenuated, and visible to the owner.
2. **Existence privacy** — partial replication must not leak the shape of what was withheld: no counts, IDs, or metadata of ungranted objects are exposed to a peer, and a peer must not be able to distinguish "no such object" from "not granted".
3. **The revocation honesty boundary for plaintext** — revoking a grant stops future sync/access via ID; it cannot un-copy bytes a peer already holds in plaintext. Encrypted shares improve on this boundary in exactly the ways D3 states, and no further.
4. **The adapter-contract framing** — credentials are referenced, never stored in ID data; adapters are read/materialize-first in v1. §7 applies this to keys and materialized data.
5. **Encrypted shares exist in two flavours** — shares that remain encrypted at the peer (peer stores/forwards but cannot read) vs. readable shares. D1 names these modes and decides their key stories; the parent left both key stories to this record.

## D1 — Share shape: per-share key, two modes

- A **share** is a granted subset (per-object and/or per-collection, per the parent grants model) packaged for replication: an initial snapshot of the subset plus scoped ongoing updates, as in the parent's partial-replication sketch.
- Every encrypted share has exactly one **share key lineage**: a random per-share key, versioned by epoch (D3). Individual objects are encrypted under per-object data keys wrapped by the current epoch key, so an epoch change re-wraps headers instead of re-encrypting object bodies.
- The share's **mode** is a property of the share, recorded in its manifest (D4), and is one of:
  - **Readable** — each grantee receives the epoch key (D2) and decrypts locally. This is the encrypted transport/storage form of an ordinary grant: the peer ends up able to read exactly what it was granted, and nothing else it may store or forward for others.
  - **Sealed** — the peer stores and forwards ciphertext but never receives any epoch key. The only key holders are the owner and, where the grant names end recipients distinct from the storing peer, those end recipients. A sealed share is how a peer can relay or host a subset it cannot read (including the foreign-adapter materialization case, §7).
- A grant still exists independently of encryption: an unencrypted (plaintext) share under the parent model remains possible where classification permits (D5). Encryption is a property layered on a grant, never a substitute for one — holding a key without a grant is not access (the manifest binds key epochs to grant lineage, D4), and holding a grant without the key yields ciphertext, not access.

## D2 — Who holds keys, and how keys move

- **The owner holds every epoch key of every share it issued.** Key loss by the owner is data loss for sealed shares; owner key backup is the identity layer's recovery story (the group scheme's recovery flow), not a new mechanism here.
- **Readable mode:** each grantee holds the current epoch key. **Sealed mode:** storing/forwarding peers hold **no** key material at all — not wrapped, not escrowed; there is nothing on a sealed peer to steal that decrypts the share.
- **Delivery is wrapped and owner-originated.** An epoch key reaches a holder wrapped to that holder's identity key, delivered as a **welcome-style message** reusing the group scheme's `welcome` machinery: the welcome carries the epoch key, the manifest (D4), and a history cut — `full` for a snapshot grant, `recent:N` or snapshot-only for a scoped grant — mirroring the parent grant duration dimension. Keys never travel peer-to-peer: a grantee cannot hand a key to anyone else, because there is no protocol message by which it could do so with any standing.
- **Delegation mints a new share, not a key hand-off.** Where the parent grants model permits explicit, attenuated, owner-visible delegation, the delegate's access is a **new share with its own key lineage**, whose plaintext scope is the attenuated subset, issued through the owner's key-delivery path (the delegator requests; the owner mints and delivers). This keeps every key lineage traceable to exactly one grant lineage — the property D4's proofs rely on. (Rejected alternative: delegator re-wraps the same epoch key — see Alternatives.)
- **Group keys are not share keys.** The existing group scheme (`pkgs/id/src/encryption/`) keys a *group roster*; a share's grantee set is a *grant set*, which changes on grant events, not roster events, and one group may back many shares with different grantee sets. Share key lineages are therefore separate from group session keys, while deliberately reusing the scheme's epoch discipline (D3) and welcome transport. Implementation may share primitives; it must not share key material.

## D3 — Rotation: event-driven epochs, forward-only take-back

- The share epoch **advances on**: grantee added, grantee removed or revoked, share mode change, and explicit owner rotation. It **never advances on a timer** — the group scheme's N7 rule ("no timer rotates it; the module takes no clock") is adopted verbatim, for the same reason: time-based rotation without a membership event buys no confidentiality and costs a full key redistribution.
- On epoch advance, a **new epoch key** is generated and delivered per D2 to the post-change holder set. New and updated objects are written under the new epoch only. Existing objects are **lazily re-encrypted**: an object is re-keyed to the current epoch when it is next written; until then it remains under its old epoch key, which all current holders keep for epochs they legitimately joined.
- **Revocation semantics (the honest boundary, stated precisely):**
  - *Future data:* a revoked grantee receives no further epoch keys, so everything written after revocation is cryptographically closed to them. This is real take-back, and it is stronger than the parent's plaintext boundary.
  - *Sealed shares:* a sealed peer never held any key, so revocation (or unpublish of the sealed placement) is **complete** take-back for everything it stored, at every epoch.
  - *Past epochs in readable mode:* a revoked grantee keeps the epoch keys they already held and can still decrypt ciphertext from those epochs that they already copied. Encryption does not repeal the parent's honesty boundary for data already delivered in decryptable form; any design or UX copy claiming otherwise is wrong.
- **v1 promise (answering the parent's open question):** cryptographic take-back is promised **forward-only** (future epochs + all sealed data). Retroactive take-back is a non-goal for v1 and is not promised anywhere.

## D4 — What a share recipient can prove: the share manifest

Every encrypted share carries a **share manifest**: a signed object in ID's native object model, signed by the owner's identity key, containing:

- `share_id` and the **grant lineage** it was issued under (grant id and parent chain, per the grants model).
- The **granted-set commitment**: the root of a hash tree over the granted objects' ids and content hashes at snapshot time; updates extend the tree epoch by epoch.
- The **mode** (readable/sealed), the current **epoch**, the share **expiry** (from the grant's duration dimension), and the **classification ceiling** the share was issued under (D5).

Against the manifest, a recipient can prove — to themselves, or to a third party by presenting manifest + data:

1. **Provenance** — this data was shared by this owner under this grant lineage (owner signature over the manifest; each epoch's manifest extension is likewise signed).
2. **Integrity** — each received object's content hashes up to the granted-set commitment; tampered or substituted ciphertext fails verification before decryption is even attempted.
3. **Completeness within the grant** — the received set matches the granted-set commitment: nothing *inside* the grant was silently dropped or altered in transit or by a forwarding sealed peer.
4. **Freshness bound** — the manifest's epoch and expiry state exactly which epoch the data belongs to and until when the grant runs.

And, by deliberate symmetry with existence privacy, a recipient **cannot** prove anything about objects outside the grant: the manifest enumerates the granted set only, and "the owner withheld X" is unprovable from share artifacts — there is no total-set commitment anywhere in the design. Completeness is a property of the grant, never of the owner's whole store.

## D5 — Composition with sensitivity classification and the grants model

Classification, grants, and encryption answer three different questions and compose by **intersection**:

- **Sensitivity classification decides what is shareable at all** (and is owned by the classification / ID-query design line, consumed here, not defined here). A classification ceiling is recorded in the manifest (D4) at issuance.
- **The grant decides who** receives a share and with which operations and duration (parent model, #5799 detail).
- **Encryption decides who can decrypt** what they hold (this record).

Rules that keep the composition from contradicting itself:

1. **Effective shareability = classification ceiling ∩ grant scope ∩ mode.** Any layer may only narrow the result. No layer may widen another: a grant cannot make unshareable-classified data shareable, and encryption cannot make an ungranted object readable.
2. **Classification may mandate a mode.** Where classification marks data as never-readable-by-peers, the only permitted encrypted form is **sealed** (or no share at all); a readable share of such data is a classification violation even to a legitimate grantee. Conversely, classification may permit plaintext shares for low classes; encryption is then optional, not mandatory.
3. **Key delivery follows the grant set exactly.** The holder set of D2 is derived from the live grant (plus sealed placement), never from roster membership, pairing state, or session presence — ambient key possession is the failure this rule excludes, mirroring the no-ambient-authority rule of the provider-grants record (#5900, D2).
4. **Refusals are indistinguishable under existence privacy.** "Not granted", "classified above your share's ceiling", and "no such object" present identically to the requester (§5, invariant 2); the owner's own audit surface may distinguish them, a peer's surface may not.

## 5. Invariants implementation must hold

1. **One key lineage per share, one grant lineage per key lineage.** Every epoch key in existence is attributable to exactly one share and one grant chain; there is no shared or ambient key material between shares.
2. **Existence privacy is preserved through the key layer.** Key-delivery messages, manifests, and ciphertext metadata expose nothing about ungranted objects — no counts, ids, or sizes beyond the granted set itself. (Accepted bounded leak, stated openly: ciphertext sizes and timing of *granted* objects are visible to their holders and to sealed forwarders, and a share's grantees can see the share's own roster, as in the group scheme. Padding/batching to blunt size analysis is an implementation option, not a v1 invariant.)
3. **No timer rotation.** Epochs advance on the D3 events only; no clock is introduced into the share key layer.
4. **Forward-only take-back is the stated promise** — in code comments, UX copy, and docs. Nothing may claim retroactive unreadability for readable-mode epochs already delivered.
5. **Sealed peers hold zero key material**, in every storage and memory path — including logs, caches, and crash dumps. (This is the invariant the #7370-salvage class of bug — credential material reaching logs — must never be able to violate for share keys: share keys are never logged, at any level.)
6. **Verification before decryption.** A recipient verifies manifest signature and object hashes before decrypting; unverifiable data is refused with a stable error, never decrypted opportunistically.
7. **Expiry is a grant property, enforced at delivery.** An expired grant delivers no new epoch keys and no new snapshot; expiry does not (and cannot) recall keys already delivered — same honesty boundary as D3.

## 6. Alternatives rejected (summary)

- **Reuse the group session key as the share key** — group roster ≠ grant set; every grant change would churn group encryption for all members, and per-share revocation becomes impossible without re-keying the group (D2).
- **Per-object keys delivered individually, no share key** — distribution and revocation become per-object bookkeeping with no lineage to audit; the manifest would need a key entry per object per grantee. The wrapped per-object data key under a share epoch key gets the same isolation at share granularity (D1).
- **Delegator re-wraps the same epoch key for a delegate** — creates key lineages the owner never issued, breaking D4 provenance and §5 invariant 1. Delegation is a new share (D2).
- **Owner-held keys with decryption-as-a-service** (peers ask the owner to decrypt) — breaks offline use, breaks sealed forwarding (the forwarder was never the problem), and makes the owner a live dependency and a plaintext bottleneck for every read.
- **Transport confidentiality (TLS/relay encryption) counted as an "encrypted share"** — the receiving peer still reads plaintext; that is a readable share with extra steps, and labelling it encrypted would silently void the sealed mode's guarantee.
- **Retroactive take-back as a v1 promise** (e.g. via claimed secure deletion on the peer) — unprovable and false once key + ciphertext have both been delivered; the honest forward-only promise of D3 is the strongest true statement available.
- **Timer-based epoch rotation** — rejected by the substrate's own N7 rule; cost without a membership event to justify it (D3).

## 7. Boundaries with sibling decisions (consumed, not duplicated)

- **#5799 (grants model detail)** owns grant object shape and lifecycle. This record assumes grants carry the parent dimensions (who/what/operations/duration) and consumes grant lineage as an opaque chain; if #5799 changes lineage shape, D4's manifest field changes spelling, not semantics.
- **#5802 (public shares)** — a public share is world-readable by definition, so encryption in this record's sense does not apply: there is no restricted holder set for a key to protect. The D4 manifest (minus key material) is reusable as the public share's integrity/provenance artifact; whether #5802 adopts it is #5802's decision.
- **#5803 (adapter contract) / #5804 (IndexedDB)** — foreign data materialized into ID is shared under this record exactly as native data, with classification assessed at materialization time. Credentials for foreign stores are never share payload and never key material (parent rule: referenced, not stored). A sealed share is the only form in which a foreign store's materialized data may be *placed* on a third-party peer.

## What this record does not do

No code, no key or manifest wire format beyond the field vocabulary above, and no changes to `pkgs/id/src/encryption/` are specified. Implementation work (share key lineage in the ID object model, welcome-based key delivery, manifest signing/verification, epoch plumbing through partial replication) is follow-on work to be filed from this record; #5801 stays open until this record is merged, per its acceptance.
