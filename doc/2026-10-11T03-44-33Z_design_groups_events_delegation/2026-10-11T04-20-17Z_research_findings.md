# Research findings: each design item

- Date: 2026-10-11 (UTC 04:20)
- Parent: [events, groups, delegation, removal, and history](./2026-10-11T03-44-33Z_design_groups_events_delegation.md)
- Scope: every item in the parent design, researched in eleven parallel passes against primary sources (specs, crate source, official docs), then spot-checked by the lead.
- Status: research, not decisions. Owner decisions are in the parent document's appended Decision section.

Tags used throughout:

- [V] verified by the lead in this session, against crate source, a spec, or a crates.io record.
- [R] read by a research pass from a primary source. Not re-checked by the lead.
- [x] executed by a research pass in a scratch probe.
- [I] inference or estimate. Not tested.

## 0. Summary

| Item                         | Verdict                                                                              | What it changes                                                                                    |
| ---------------------------- | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| p2panda-core header          | Adopt the format. Do not adopt its verification path.                                | `Header::verify` returns `true` in non-test builds [V]. Verify from bytes ourselves.               |
| CBOR for signed bytes        | cbor-core, which p2panda signs with. Add our own strictness rules.                   | Not ciborium or minicbor. dcbor is the stricter profile to borrow rules from.                      |
| Fork handling                | Refuse and freeze, but store both events and resolve from the proof set.             | p2panda-store keeps both forks today. We need a unique (author, log, seq) index and a proof table. |
| Waiting queue                | Build our own, with per-author caps and expiry.                                      | p2panda-stream holds pending items forever, which is a DoS vector.                                 |
| p2panda-auth                 | Adopt StrongRemove. Build cap, dissolve, cascade, and policy on top.                 | An unknown dependency panics. Pass auth ops only, never our `after` frontier.                      |
| p2panda-spaces               | Not yet. Watch it.                                                                   | Adds do not rotate keys. Promote and Demote panic in 0.7.1 [V].                                    |
| DataScheme (DCGKA)           | Adopt behind a thin adapter. This is our E2EE default.                               | Adds mint no secret [V]. The orderer wants every last-known control message, which can exceed 32.  |
| DCGKA cost                   | Estimates only. Thresholds are the owner's call.                                     | No published breakpoint at 96 or 128. The first large jump is near 1,000 (extrapolated).           |
| MLS and openmls              | Not a drop-in. Fits a home server that orders commits.                               | RFC 9420 §14 requires an application-level answer to conflicting commits [V].                      |
| BeeKEM                       | Candidate for large groups behind the trait, after a spike.                          | Standalone crate `beekem` 0.4.0 exists (Apache-2.0). Research code.                                |
| Cascade delegations          | Keep as default. Refine to UCAN's "no alternate path" rule.                          | Keyhive's design states the cascade rule [V].                                                      |
| Transfer on removal          | Novel. Keep, but no surveyed system has it.                                          | Needs a successor who already holds the scope.                                                     |
| Strong remove timing         | No time window. This follows from causal concurrency, as in p2panda's StrongRemove.  | The late-push example holds.                                                                       |
| Matrix redaction             | Use the header/body split. Derive hidden content as a view.                          | Operation IDs survive a dropped body, because the ID covers `payload_hash`.                        |
| Joiner history `full`        | Matches Matrix's default (`shared`).                                                 | DataScheme's welcome carries the whole bundle natively.                                            |
| Observed-remove tags         | Build it. Do not adopt the `crdts` crate.                                            | Tags follow the content policy, not strong-remove voidness (a proposal).                           |
| Storage                      | redb is already compiled in. Recommend it, pending spike S6.                         | Disable iroh-docs' default `redb-v2-migration` to drop redb 3.x.                                   |
| Sync                         | Measure LogSync against range reconciliation (spike S1).                             | LogSync's `Have` list grows with the number of chains.                                             |
| iroh-docs                    | Derived index only. Not the event log.                                               | Read access is possession of the namespace ID.                                                     |
| iroh-blobs                   | Adopt, pinned `=0.103.1`. Gate gets with EventMask.                                  | No content routing. Providers serve any `get` by default.                                          |
| iroh-tickets                 | Adopt as is.                                                                         | Tickets carry IPs and cannot be revoked.                                                           |
| Discovery privacy            | Our client endpoints use N0, which queries n0 DNS for node IDs.                      | Decide: keep N0, or use `Minimal` plus tickets for invites.                                        |
| Seitan V2                    | Derivations verified in keybase/client source. Adopt them and add a freshness check. | Test vector `4uywza+b3cga7rd6yc` gives inviteID `6303ec43bd61d21edb95a433faf06227`.                |
| Pull auth                    | Advisory only. Default open, as decided.                                             | Documented: storing nodes always see chain metadata.                                               |
| Time witnesses               | Defer. Correct §12's direction.                                                      | A witness proves existence at or before T, not at or after.                                        |
| Tag storage in the event log | Keep the α/Ω index as a local read index.                                            | Discard existing tag data.                                                                         |

Toolchain: the p2panda crates (core, auth, encryption, spaces) declare `rust-version = "1.96"` [V]. `pkgs/id` declares `1.91.0` [V]. Adoption requires bumping it. `rust-toolchain.toml` pins 1.97.0, so the toolchain itself is fine.

---

## 1. Event format and encoding

### 1.1 p2panda-core header (0.7.1)

- Shape. A CBOR array: `[version=1, verifying_key, signature, payload_size, payload_hash (if size > 0), seq_num (u32, 0-based), backlink (if seq > 0), extensions (if not unit)]` [R].
- Signature. Ed25519 over the array without the signature. The body is covered only through `payload_hash` [R]. The operation ID is BLAKE3 over the full header, signature included [R].
- Fit for `after`. A `Vec<Hash>` extension is a CBOR array of 32-byte strings, so `after` needs no format change [X].
- Overhead. 104 bytes for seq 0 with no body or extensions. +34 for a backlink, +34 for a payload hash, and +34 per `after` hash. A 32-hash header with no body is about 1,235 bytes [X].
- Hazards we must design around:
  - `Header::verify` returns `true` in non-test builds, with the comment "Header was always created by us and has a valid signature" [V: `operation/header.rs:185–194`]. Never call it. Verify from the encoded bytes.
  - Typed headers have public fields and a cached `hash()`. After editing `payload_size`, the cached hash stayed the same while `blake3(encode())` changed [X]. Recompute IDs from bytes.
  - `length_limit(512)` caps each declared length, not the total [R]. Enforce total size at the frame and per kind.
  - Typed decoding ignores unknown extension keys [R]. Our decoder must reject them.
  - Signed bytes have no domain-separation tag [R]. Add one, such as a kind and version prefix inside the signed body, so signatures cannot be replayed across protocols.
  - `PruneFlag` makes the backlink check skip [R]. Do not enable pruning until retention is designed.
- Maintenance. 0.7.1 released 2026-08-21 [V]. The repo has 199 open issues [R]. The researcher found no license file in the crate; crates.io lists MIT OR Apache-2.0 [V].

### 1.2 CBOR for signed bytes

- cbor-core 0.10.1 (MIT) encodes with RFC 8949 §4.2.1 bytewise key order [X]. Its strict decoder rejects length-first maps and non-shortest integers, but accepts non-NFC text, float 1.0, and `undefined` [X].
- dcbor 0.25.2 (BSD-2-Clause-Patent) follows the dCBOR profile and rejects all of those [X].
- ciborium 0.2.2 does not sort keys: `{"b", "aa"}` encodes as `"aa", "b"`, which matches neither RFC order [X]. Excluded.
- minicbor 2.3.0 writes keys in call order and accepts non-shortest integers [X]. Excluded for signed bytes.
- Verdict: cbor-core for all signed bytes, since p2panda-core signs over its encoding and a second encoder would be a second canonical form. Add our own checks on top: NFC text, no floats in signed bodies, no `undefined`. dcbor's rules are the spec for those checks [I].
- Caveats [R]: cbor-core has no `no_std` feature and uses `std::io`. Its README says the API is not stable (0.8.0 in April 2026, 0.10.1 in June). One GitHub owner, 5 stars. Its doc comment says "length and then bytewise", but the code and tests use bytewise. Do not rely on the comment.
- Standards [R]: RFC 8949 §4.2.1 and §4.2.3. CBOR::Core draft-25 §2.2 requires bytewise order and rejects duplicate keys. dCBOR draft-18 is a standards-track draft, not an RFC.

### 1.3 Validation and forks in p2panda-core

- `validate_header` checks the signature, payload hash and size, and the backlink's presence (required iff seq > 0). `validate_operation` adds the body checks. `validate_backlink(past, h)` checks the same author, `seq = past + 1`, and `backlink == past.hash()` [R].
- No fork error and no fork detection. Two siblings at seq 1 both pass `validate_header` and `validate_backlink` against the same base [X]. Comparing them gives `SeqNumNonIncremental(2, 1)`, which is misleading [X].
- p2panda-store's operations table has only `PRIMARY KEY(hash)` and inserts with `INSERT OR IGNORE`, so a fork is stored silently [R].
- Effort: about 120 lines. A unique (author, log, seq) index, a proof table, and a frozen flag [I].

### 1.4 p2panda-stream causal ordering

- 0.7.1: `Orderer::dependencies(&self) -> &[Hash]` returns a borrowed slice, so backlink and `after` must be stored in one list [R]. Main (2026-10-09): `OrdererArgs::Process { dependencies: Vec<Hash> }` takes a list per input, which fits our model directly [R].
- Neither adds the backlink automatically. Both depend on p2panda-store's SQLite [R].
- Hazards [R]: pending items never expire, there is no per-author cap, and there is no cycle detection. One row per dependency-child pair. The ready table is never cleaned up. Cheap signed events with missing dependencies can grow storage without limit. Main buffers released items in an unbounded `VecDeque` whose comment expects fewer than 100.
- Verdict: build our own waiting queue, about 200 lines, reusing the algorithm (release dependents recursively). Add per-author caps and expiry. This is a required design property, not an option [I].

### 1.5 Signature strictness

- p2panda-core uses ed25519-dalek 2.2.0. Its `verify` calls `verify_strict`, which rejects small-order public keys and small-order R, and checks that the recomputed R matches the encoding. Canonical S (`s < L`) is enforced in `from_bytes` [R].
- Our `pkgs/id` pins ed25519-dalek 3.0.0 [V: lock]. The two coexist, since keys cross as bytes.
- Verdict: use `verify_strict` for every direct check. Reject identity and small-order keys at creation, since `from_bytes` accepts the identity point [X].
- Probe result [X]: with public key = identity, R = identity, and S = 0, plain `verify` accepted arbitrary bytes. `verify_strict` rejected them. p2panda's header decode returned `InvalidSignature`.

---

## 2. Groups and auth

### 2.1 p2panda-auth (0.7.1)

- A causal-DAG group CRDT. Operations are `GroupsOperation { id, author, dependencies, group_id, action }`. Actions: Create, Add, Remove, Promote, Demote [R]. Access levels: Pull < Read < Write < Manage. Conditions are ordered but never enforced [R]. Nesting via `GroupMember::Group` [R]. One graph serves all groups [R].
- Built in: `StrongRemove` (`resolver.rs:52`), with transitive invalidation and the mutual-cycle rule [R].
- Not built: weak remove, member cap, cascade, dissolve, policy, delegation [X].
- A custom resolver controls only the public `ignore` set. `apply_action` and `state::*` are crate-private [R].
- Hazards:
  - An unknown dependency hash panics in the authority-graph builder, which calls `expect("all operations present in map")` [V: the `expect` line; panic path from a research run]. Pass only auth operations as dependencies, never our `after` frontier.
  - No member cap. 200 sequential adds were accepted [X].
  - No cascade. A delegate's add after the delegator's removal still applied [X].
  - `heads()` is global, so concurrency in any group forces a full rebuild [R].
  - Main (unreleased) adds a validation module and `required_groups()`, renames `Resolver::Error` to `Error` (which breaks custom resolvers), and stops erroring on duplicate operations [R].
- Effort. Adopting strong remove, access levels, and nesting costs close to nothing. Building cap, dissolve, cascade, and policy as a resolver plus an application projection is about 300–500 lines with tests [I].

### 2.2 p2panda-auth maturity

- Seven releases since January 2026 [R]. Breaking changes: 0.5.0 added a `Conditions` super-trait; 0.5.2 removed the high-level API and the orderer generic; 0.6.0 added an `Author` super-trait; 0.7.1 replaced `IdentityHandle` and `OperationId` [R].
- Open correctness issues [R]: #783 (promote and demote do not check direction), #799 (a silent depth cap), #1345 (application messages invalidated by a concurrent removal are not surfaced).
- 50 unit tests pass on 0.7.1 with rustc 1.97 [X].
- No external users found [R]. The encryption README says the crate "has not yet received a security audit". The 2025-02-24 blog says "pending audit by Radically Open Security". No report found [R].
- MSRV 1.96 [V]. Downloads 2,771 [V].

### 2.3 p2panda-spaces (0.7.1)

- A groups and spaces manager built on auth `StrongRemove` and the encryption DataScheme [R]. Device identity is one Ed25519 key per actor (`ActorId = VerifyingKey`), with no device type [R].
- Removal rotates the secret. Adds do not: `EncryptionGroup::add` passes no new secret [V].
- Promote and Demote hit `unimplemented!()` in 0.7.1 (`event.rs:249–250, 297–298`), so they panic. Main implements them [V].
- Pre-key lifetime 90 days, rotated after 60 (`config.rs:25`) [V]. The encryption crate's own default was reported as 84 days; not verified.
- Welcomes carry the whole secret bundle (`dcgka.rs`), matching our `full` default [R].
- Adoption: weeks [I]. APIs are not stable before 1.0 [R]. Needs causally ordered, signed input [R]. Created 2026-07-07, 1,555 downloads, two releases [R]. Open issues #807 and #1365; flaky tests #846 and #1411 [R].
- No dependency on p2panda-net or p2panda-stream [R].

### 2.4 Multi-device prior art

- Matrix cross-signing: the master key signs the user-signing and self-signing keys. The self-signing key signs device keys. A device is trusted through that chain [R].
- Signal linked devices: the primary device owns the account. Linked devices share an identity key pair but have independent prekeys and sessions. Keys arrive from the primary at linking [R].
- MLS: one leaf per client. A new device is a new client with no history (RFC 9750 §6.1, §6.7) [R].
- Implications for `device.add` and `device.revoke` [I]: Matrix trusts through an account-level signature. Signal needs provisioning from an existing device, and unlinking does not remove the shared identity key. MLS adds a leaf, and revocation is a Remove that starts a new epoch. Ours: auth `Add`/`Remove` of device actors, plus a DCGKA remove.
- Threshold counting. DCGKA sends one direct message per remaining device on update and remove [R]. Count recipient devices for the warnings. Show accounts alongside for display [I].

---

## 3. Encryption

### 3.1 DataScheme (p2panda-encryption 0.7.1)

- The group secret is 32 random bytes with a SHA-256 ID and a UNIX-seconds timestamp. There is no epoch counter. "Latest" means the highest timestamp [R].
- `create`, `update`, and `remove` each mint a secret and send it by 2SM to each member: n−1, n−1, and n−2 direct messages (the removed member gets none) [V: `group.rs` lines 86–163, with `generate` calls only in create, remove, and update]. `add` mints nothing [V].
- Welcome: one 2SM direct message carrying the adder's whole `SecretBundle`. Full history is native. The DGM history travels in plaintext alongside it [R].
- For `from_join`, a welcome with only the current secret needs a reduced bundle, plus an `update` to rotate, since `add` does not rotate [I]. Not built.
- Pre-keys: X25519 long-term bundles, XEdDSA-signed. First contact uses X3DH; later rounds use HPKE (hpke-rs 0.7.0) [R].
- Inputs: authenticated, causally ordered control messages. Each operation points at all last-known control messages. No acks [R].
- Verdict: adapt, through two trait impls (`Ordering` and `GroupMembership`). The orderer is the hard part. A few hundred lines plus tests [I].
- Risks [R]: issue #807. A member added concurrently can miss a secret from a concurrent remove or update, and there is no forwarding. Expired pre-keys block first-contact DMs. Envelope metadata is unencrypted (README "Meta-Data"). `process_ready` expects all n−1 DM ciphertexts in one message.
- Implication for our `after` cap [R + I]: `Ordering` wants each control message to name all last-known control messages. In busy groups that can exceed 32. Encryption will also need merge or checkpoint events.
- Dependencies [R]: hpke-rs 0.7.0, chacha20poly1305 0.10.1, x25519-dalek 2.0.1. Our lock has a different crate, `hpke` 0.13.0 [V]. They do not conflict.

### 3.2 DCGKA paper (Weidner, Kleppmann, Hugenroth, Beresford)

- ePrint 2020/1281, revised 2021-05-31. ACM CCS 2021 [R].
- Decentralised continuous group key agreement with forward secrecy and post-compromise security. A PCS update sends a fresh seed to each member over 2SM. Each recipient broadcasts an unencrypted ack. Assumes authenticated causal broadcast, a PKI, and unique additions [R].
- Measured (OpenJDK 8, in-process network, 8-core i7) [R]. At n = 128: create 43.4 kB, PCS update 39.6 kB, add 75.5 kB, remove 39.3 kB. Up to 128 members, at most 100 ms of CPU per party. A message costs 139 bytes and under 1 ms.
- The paper says MLS allows 50,000 members and this protocol does not (§2.1) [R].
- Reference implementation: `trvedata/key-agreement`, Java, MIT, last pushed 2021-05-19, not for production [R].
- Caveat [I]: p2panda's variant sends the group secret with no ack round. The paper's seed-and-ack protocol is different, so the paper's proofs may not cover this variant.

### 3.3 MessageScheme

- Not a Double Ratchet per member pair. Each member has one sending chain. Each receiver keeps one decryption ratchet per other member. Chains are seeded from DCGKA update secrets. Keys are HKDF-derived and erased after use [R].
- No per-message DH step. PCS happens only at updates and acks [R]. Every create, add, update, and remove needs an ack from each other member [R].
- Defaults: forward distance 1,000, out-of-order tolerance 100 [R].
- Build, large: ack tracking and strict per-sender ordering [I].
- Open issue #1426 [R]: no AAD, and the ratchet advances before authentication, so a forged generation can DoS a channel. The maintainer says authentication is out of scope for this layer.
- The p2panda blog says DCGKA is "performant for small- to mid-size groups of ca. 128 members" and prefers TreeKEM or MLS for thousands [R].
- Correction to the parent document §8.5. It is not a per-pair Double Ratchet.

### 3.4 Key-agreement trait seam

- Public traits [R]: `GroupMembership` and `Ordering` (DataScheme); `AckedGroupMembership` and `ForwardSecureOrdering` (MessageScheme); `IdentityManager`, `PreKeyManager`, `IdentityRegistry`, `PreKeyRegistry`, and `KeyBundle`.
- Not traits [R]: `two_party::TwoParty`, `data_scheme::dcgka::Dcgka`, and the `ControlMessage` and `DirectMessageContent` enums, which hard-code 2SM and the Welcome. DataScheme cannot be swapped without forking `dcgka.rs` and `group.rs`. The real seams are DGM and `Ordering`.
- Boundary we need [R]: input is causally delivered events (sender, id, seq, frontier). Output is control bytes, per-recipient DM bytes, Welcome bytes tagged with a `scheme_id` (the crate has none), and secret events. `receive` does not report which secrets it learned, so derive them by diffing `GroupState::secrets`.
- Risks [R + I]: main adds a `PartialEq + Eq` bound to `GroupMembership::State`, which breaks custom DGMs. 2SM is stateful per pair, so a refused fork carrying a DM could advance a 2SM state.
- Effort: a thin adapter over `EncryptionGroup`, small to medium [I].

### 3.5 p2panda-encryption maturity

- Eight pre-1.0 releases in 13 months, from 0.4.0 (2025-07-07) to 0.7.1 (2026-08-21) [R].
- Main is 305 commits ahead, but only four encryption files differ (derives, the `State` bound, two tests). No protocol change [R].
- Open issues [R]: #807 (open since 2025-09-23), #1426, and #1244 (encoding compatibility of key state).
- Audit: none. The 0.7.1 README and main both say so. The blog reports a pending audit by Radically Open Security [R].
- No test vectors in the crate or in the Java reference [R].

### 3.6 MLS (RFC 9420) and openmls

- RFC 9420 is TreeKEM group key agreement (Standards Track, July 2023). Each change is a Commit bound to one epoch. The Delivery Service only routes messages (§3) [R].
- Ordering [V: RFC text, line 5207]: §14 says applications MUST have an established way to resolve conflicting Commit messages for the same epoch. MLS therefore leaves the winner to the application. A losing committer may never be able to send a Commit [R].
- Welcome: one HPKE-wrapped secret per joiner, plus GroupInfo. Joiners receive no earlier messages (§3.2) [R]. That conflicts with our `full` default. It matches Matrix's `joined` option.
- Removal: logarithmic ciphertexts (§16.2). PCS needs a Commit that includes an Update (§16.6) [R].
- Real-world: Discord's DAVE uses MLS with a central voice gateway as the Delivery Service. The gateway broadcasts the first commit it receives for each epoch [R].
- openmls 0.9.1 (MIT, MSRV 1.91, released 2026-10-07) [R]. API: `MlsGroup::{add_members, remove_members, self_update, process_message, merge_pending_commit}`, and `StagedWelcome::new_from_welcome`. An SRLabs audit (2026-05-27) reported 8 findings, one High, seven fixed. Used by Wire's core-crypto [R].
- No ordering in openmls. A stale commit fails with `EpochMismatch`. Past-epoch application messages are undecryptable by default (`MaxEpochs(0)`) [R].
- Benchmark sheet (undated, 2.8 GHz i7, suite 0x0001) [R]: remove takes 4.3 ms at n = 100 and 11.7 ms at n = 1,000. Setup takes 398 ms at 1,000. Nothing is published above 1,000.
- Verdict [I]: MLS is not a drop-in for decentralised commits. It fits a channel hosted by a home server that orders commits, as in DAVE. That is an architectural option, not the default.

### 3.7 Published group-size limits

- Signal: 1,000 per group (support page) [R].
- WhatsApp: 1,024 per group, 2,000 per community (snippet) [R-snippet].
- Telegram: 200,000 per supergroup, 200 per group call (FAQ, undated). Ordinary chats are cloud-based, not end-to-end [R].
- Discord: DAVE covers voice and video. Its spec sets no cap and batches for "large or busy" sessions [R].
- Matrix: no cap found [I].
- Wire: channels 2,000, calls 150 (blog dated 2025-12-02). The blog says encryption becomes "inefficient and slow" at thousands [R].
- Implication for the warnings [I]: the owner's 96 and 128 sit far below mainstream end-to-end limits (1,000 and 1,024). They would fire for ordinary groups. That is not a reason to change them. It is a reason the owner should see the comparison.

### 3.8 Cost estimates for the warning thresholds

- DCGKA: removal sends n−2 2SM messages. An update sends n−1. An add sends one Welcome carrying every past secret [R]. A message is about 250–380 bytes [I]. The paper's 40 kB ÷ 127 ≈ 320 bytes [R].
- TreeKEM: one Commit ≈ 250 + 115·⌈log₂ n⌉ + 115 bytes [I]. A wrapped 32-byte secret is about 81 bytes (RFC 9180 overhead: Nenc 32, Nt 16) [I].

Removal, estimated:

| n     | DCGKA messages / bytes | TreeKEM Commit |
| ----- | ---------------------- | -------------- |
| 96    | 94 / ~28 KB            | ~1.2 KB        |
| 128   | 126 / ~37 KB           | ~1.2 KB        |
| 1,000 | 998 / ~290 KB          | ~1.5 KB        |
| 5,000 | 4,998 / ~1.4 MB        | ~1.9 KB        |

- Reading the table [R + I]: with unicast delivery, DCGKA is smaller per member (about 300 bytes against 1.2–1.9 KB). TreeKEM wins on sender egress only when a fan-out Delivery Service or relay sends each commit once.
- Verdict on 96 and 128 [I]: there is no published breakpoint at either number. 128 matches the paper's one data point, about 40 kB per update. It is too small a cost to justify a cryptographic warning on its own. The first real jump is probably near 1,000 (about 0.3 MB and 0.5 s CPU, extrapolated). Uncertainty: the 2SM serialisation size was assumed, so sizes may be off by about 30%.

### 3.9 BeeKEM and other decentralised tree variants

- BeeKEM is in keyhive's `beekem` crate. Apache-2.0. `beekem` 0.4.0 is on crates.io, released 2026-09-25 [V: crates.io]. It depends only on `keyhive_crypto`, so it works standalone [R].
- API [R]: `cgka::Cgka::{new, add, remove, update, merge_concurrent_operation, new_app_secret_for}`, with operations `Add`, `Remove`, and `Update`.
- Concurrency [R]: concurrent path updates keep both keys as conflict keys. The node is treated as blank, so no leaf wins. Concurrent adds sort by identity, and removes apply last. A merged tree has no root until the next update.
- Welcome [R]: `CgkaOperation::Add` carries no path secret. A joiner needs a later Update to get a root. MLS's Welcome differs: it HPKE-wraps `GroupSecrets` per joiner.
- Costs [R]: O(log n) per operation in the common case. Worst case O(n) time and O(n log n) conflict storage.
- Assumptions and gaps [R]: causal delivery. No forward secrecy for the root. Unaudited. The nonce and key-commitment construction is unreviewed. The README says "Expect bugs."
- Other variants [R]: Causal TreeKEM (Keyhive notebook; DCGKA paper Table 1) needs causal order and BLS-style crypto, and the paper rates its forward secrecy "severe". Concurrent TreeKEM (paper Table 1) gives PCS only. The researcher checked no code for either.
- Verdict [I]: BeeKEM is the candidate for a large-group scheme behind our trait, after a spike. Its causal-delivery assumption matches our DAG better than MLS's ordering requirement. The 96 and 128 thresholds would need re-deriving under it.

### 3.10 Causal encryption and keyhive's grant graph

- Causal encryption is in keyhive_core. Each envelope carries its direct predecessors' keys (an `ancestors` map), so one key reveals its causal history but not its parents or siblings [R].
- No forward secrecy [R]: "This sacrifices forward secrecy — leaking old message keys in the case of a later compromised key" (notebook).
- Cost [I]: about 64 bytes per predecessor, so at most 2 KiB at our cap of 32. Keyhive does not cap predecessor references [R].
- Over our DAG [I]: per-event keys derived from the epoch secret and the `after` set. An envelope listing (hash → key) for each `after` hash. Entry points at join time, where `full` or `from_join` decides how far a joiner can read.
- Keyhive's grant graph [R]: signed delegations and revocations form a grow-only CRDT. The same graph decides BeeKEM membership. Concurrent revocations sort by `(distance_to_root, digest)`, which is deterministic but has no single-survivor selection. Keyline (jurisdiction-scoped revocation) is design-only: zero hits in the code. No equivocation handling. The threat model covers malformed, duplicate, and unauthorised operations.
- Adoption size [R + I]: about 26K source lines across keyhive_core, beekem, and keyhive_crypto. Rust 1.90 or later. Heavy generics. A cascade-only subset is medium.
- Verdict: design input. Causal encryption is the point-in-time option the parent document deferred. The owner agreed it is not needed now.

---

## 4. Removal and delegation

### 4.1 Cascade: what the precedents say

- Keyhive's convergent-capabilities design [V: line 86]: "Revoking a delegation invalidates every delegation whose proof chain passes through it, possibly including the revoker's own."
- UCAN [R]: revoking one link kills only the dependents that have no other intact chain. The spec's example: revoking Carol→Dan kills Erin's capability X, but Y and Z survive through intact chains.
- Biscuit [I; SPEC.md was unreachable]: revoking a parent block's ID rejects every child, because children carry all parent IDs. That is cascade by containment. biscuit-auth 6.0.0 is Apache-2.0 with 12.2M downloads [R].
- Macaroons [R]: revocation by identifier is all-or-nothing. Per-holder revocation needs third-party caveats with an online discharger.
- Meadowcap [R]: no chain revocation. Removal is by expiry or by overwrite with future-timestamped entries. A removed member's delegations stay cryptographically valid, so the application must enforce removal.
- Zanzibar, SpiceDB, and OpenFGA [R + I]: deleting a tuple removes derived access from the next snapshot. These are centralised.
- p2panda StrongRemove [R]: transitive invalidation, with a mutual-removal carve-out (cycles remove everyone in the cycle).

Verdict [I]: the evidence supports `cascade`. Tighten our definition to UCAN's "no alternate valid path" rule: a delegation's authority is cut only where no other intact chain carries it. Keep the mutual-removal carve-out. Add a dry-run count before a removal applies, and a reissue path. No surveyed system has a dry run.

Failure mode [R]: a mistaken manager removal voids all dependent operations. Re-adding does not revive them (p2panda's rule 3). UCAN revocation is irreversible.

### 4.2 Transfer

- No surveyed system has a transfer primitive. The nearest is UCAN Powerline (`sub: null`), which delegates future authority to a principal [R].
- Ours: a signed reissue of the removed member's outgoing delegations by the successor. The successor must hold the scope [I]. Small to medium. Novel.

### 4.3 Matrix state resolution v2 and v2.1

- v2 orders conflicted power events (power levels, join rules, other members' leave and ban) by reverse topological power: sender power, then `origin_server_ts`, then `event_id`. It then replays auth checks. Other conflicted events follow mainline order. v2.1 (MSC4297, room version 12) starts the replay from an empty state map [R].
- Auth-DAG ordering replaced v1's depth ordering, which allowed ban evasion through forks and state resets (MSC1442) [R].
- p2panda's StrongRemove uses no timestamps and keeps mutual removals [R].
- Risk [R]: a sender-set `origin_server_ts` breaks ties. Resets still happened, which is why v2.1 exists.
- Verdict [I]: StrongRemove matches the removal side. Matrix's power-ordering idea is relevant to ordering delegation changes.

### 4.4 Matrix redaction

- `m.room.redaction` strips every key not on an allow-list: IDs, sender, hashes, signatures, `prev_events`, `auth_events`, `origin_server_ts`, and listed content keys. It applies once both events are seen and the sender has redact power or shares the target's domain [R].
- Matrix keeps IDs and signatures valid after redaction. Our header and body split does the same: the operation ID covers `payload_hash`, so dropping a body keeps the ID valid [R].
- Verdict for `hide_concurrent` [I]: derive the hidden set as a view from the DAG, StrongRemove-style, instead of emitting redaction events. That avoids a hold queue and a who-may-redact policy. Dropping bodies is an optional storage measure.
- Risks [R]: a redaction waits for its target. It is irreversible. A redacted join still counts as a join.

### 4.5 Matrix history visibility

- `m.room.history_visibility` takes `world_readable`, `shared`, `invited`, or `joined`. It is checked at each event's send time. The default is `shared` [R].
- `shared` is close to our `full`. `joined` is close to our `from_join` [R + I]. Synapse sets `shared` in all three room presets [R].
- Ours: `full` is native to DataScheme (the welcome carries every earlier secret). `from_join` is not native [I].
- Risks [R + I]: in encrypted rooms, visibility depends on key delivery. MSC4268 says a server admin can lie about visibility. Delivered secrets cannot be recalled.
- Relevance: Matrix's default is `shared`, which supports `full` as our joiner default.

---

## 5. Forks and causal structure

### 5.1 Automerge `deps` and sync

- `deps` are the heads at creation, plus the author's previous change if it is not already a head [R]. Hashes are SHA-256, 32 bytes [R].
- Sync messages carry heads, a `need` list, and a `have` Bloom filter over recent hashes: 10 bits per entry, 7 probes, about 1% false positives [R].
- No cap on `deps` or heads in the source. No published size data. At our cap of 32, each event carries about 1.1 KB of hashes [I].
- Verdict [I]: our `after` is the same concept. Borrow the Bloom sizing if we add a sync summary. The cap question stays open.

### 5.2 Secure Scuttlebutt fork handling

- A feed is a hash-linked chain. A fork is a second message at an occupied sequence number [R].
- `ssb-validate` refuses, as fatal, a message with the right sequence and the wrong `previous`. `appendKVT` throws before any state changes and stores no proof. `ssb-db2` refuses writes while an old log exists, "to protect your feed from forking" [R].
- No implementation read stores or gossips a fork proof [R]. The protocol guide has no fork rule [R].

### 5.3 Hypercore fork detection

- Each core has one writer. Signed roots cover `(key, tree hash, length, fork id)`. A conflict is two valid signatures with the same length and fork id but different tree hashes. A legitimate `truncate()` bumps the fork id, and replicas accept a higher one [R].
- On conflict: the core persists `header.frozen`, closes sessions, emits `conflict`, and stops replication [R].
- Risks [R]: the writer can rewrite history by bumping the fork id, and replicas accept that by design. Only same-length conflicts are caught. The conflict test is skipped because it is flaky.

### 5.4 Synthesis: refuse and freeze, versus keep both, versus prove and stop

- Our libraries keep both forks at the storage layer. p2panda-store uses `INSERT OR IGNORE` keyed on hash, with no unique (author, log, seq). `validate_backlink` checks one predecessor only [R]. Automerge refuses a second change at the same (actor, seq) [R].
- Recommendation [I]: keep refuse-and-freeze. Store both full events as proof. Resolve from the proof set, not arrival order.
- Divergence [I]: a node that saw branch A first and one that saw branch B first hold different DAGs. Dependents of the refused side are orphaned. A late proof forces retroactive invalidation: a node that already applied branch A must show those events as voided, which needs a view state. Frozen devices need a recovery path (re-key or re-enrol).
- Our backlink rule matches SSB's, which is the closest precedent for a per-author chain.

---

## 6. Tags, trust views, and storage

### 6.1 Observed-remove semantics

- Each add gets a unique tag. A remove deletes only the tags it observed, so concurrent adds win. Removes need their observed adds delivered first [R].
- rust-crdt's `Orswot` (crdts 7.3.2) is tombstone-free and offers `reset_remove` by vector clock [R]. The crdts crate was last released 2023-08-08 and the repo last pushed 2024-06-16. Not recommended [R].
- Our mapping [I]: adds carry dots `(author, seq)`. A clear carries the remover's per-author max-seq clock. An add survives if its seq exceeds its author's entry. The clock has to cover the transitive past, not just the 32-hash frontier.
- Precedence [I]: add-wins conflicts with strong remove. A tag added concurrently by a removed member would survive add-wins. Proposal: tags follow the content policy (parent §6.5), not membership voidness.
- Explicit tag lists cannot fit in `after` [I].
- Effort: about 150–250 lines plus property tests [I].

### 6.2 Wikidata ranks, qualifiers, and references

- Ranks: `preferred`, `normal` (default), and `deprecated`. Many values may share a rank. `preferred` marks current consensus. `deprecated` marks known errors and is kept so they are not re-added. Disputes are a qualifier (`statement disputed by`, P1310), not a rank [R].
- Read model: preferred if any exists, otherwise normal. Deprecated is excluded unless asked for [R].
- Risks [R + I]: ranks are a social signal, and edit wars happen. Whoever can sign ranks decides "best". Several preferred values need a tie-break.

### 6.3 Labels: NIP-32 and ATProto labelers

- NIP-32 (draft): kind-1985 events with `L` and `l` tags on `e`, `p`, `a`, `r`, and `t` targets, signed by the labeler's key [R].
- ATProto labels carry `src` (labeler DID), `uri`, `val` (up to 128 bytes), `neg`, `cts`, `exp`, and a `sig` over DRISL-CBOR hashed with SHA-256 [R].
- Trust is client-side. Clients choose labelers through `atproto-accept-labelers`, and services may substitute a default when it is absent [R].
- Negation is `neg: true`. NIP-32 defines no trust or negation. Removal is NIP-09, which relays "SHOULD" honour. The spec says timestamps are not trustworthy, and "current" means the latest `cts`, so it is last-writer-wins [R].
- Open issues [R-snippet]: atproto#3813 (authoritative documentation of moderation labels), atproto#3134 (a default spam labeler).
- Verdict [I]: our tags already have the same shape, as separate signed claims. The piece worth copying is the trust-selection UI.

### 6.4 Storage for the event store and the index

- redb 4.3.0 (MIT OR Apache-2.0): single writer, MVCC, checksummed double-buffered roots. Single-process: a second opener gets `DatabaseAlreadyOpen` [R].
- rusqlite 0.40.2 (MIT, with bundled SQLite): atomic commit. WAL lets readers overlap the single writer. WAL works only on one host [R].
- fjall 3.1.12 (MIT OR Apache-2.0): writes stay in OS buffers until `persist(SyncAll)`, so it is not durable by default. Single-process. Avoid [R].
- Already in the tree [V + R]: iroh-blobs 0.103.1 and iroh-docs 0.101.0 use redb 4.x. iroh-docs' default features include `redb-v2-migration`, which enables a dependency on redb 3.x [V: iroh-docs `Cargo.toml`]. That is why our lock has both 3.1.3 and 4.3.0 [V].
- Since we may discard existing data, the migration feature is not needed. Disabling iroh-docs' default features (and re-listing `fs-store`, `rpc`, and `metrics` as needed) should drop redb 3.x. Not built [I].
- p2panda-store is SQLite via sqlx. Its operations table has only a primary key on `hash` [R].
- Recommendation for spike S6 [I]: redb. It is already compiled in and pure Rust, and the server already owns the data directory. Add our own unique (author, seq) and tag indexes as tables. Choose rusqlite only if we need SQL queries or several local processes. The research pass recommended rusqlite; the trade-off is a C build and a second engine.
- Benchmark caution [R]: redb 1,174 ms against SQLite 8,431 ms for random range reads. SQLite ran with its default rollback journal, not WAL, so the comparison is not fair.

### 6.5 How competing authoritative values are resolved

- Wikidata keeps every value, ranks them, and marks disputes [R].
- OpenStreetMap allows one value per key per object. A stale write gets HTTP 409, so conflicts are visible. Its Mont Blanc example shows contradictory "current" values when a dispute is mapped in two places [R].
- Wikipedia picks one canonical title by consensus against stated criteria, with redirects for other names [R].
- Copy [I]: keep all claims and show disputes. Detect conflicts, as OSM's 409 does; our `after` frontier can do that. Give items stable IDs, with aliases as redirects.
- Avoid [I]: silent overwrites; ranks used to assert one view; closed decisions that cannot be replayed from events; contradictory "current" values.

---

## 7. Transport and sync

### 7.1 iroh-blobs 0.103.1

- Hash-addressed transfer. A request names hashes or byte ranges. Replies are BLAKE3 verified streams (bao, 1 KiB chunks). A `HashSeq` is concatenated 32-byte hashes. No blob size cap [R].
- API [R]: `BlobsProtocol`, `store::{mem::MemStore, fs::FsStore}`, `api::downloader::{Downloader, DownloadRequest}`, `ticket::BlobTicket`.
- No content routing [R]: a hash gives no route, so you supply provider IDs or a `ContentDiscovery` implementation. Providers serve any `get` by default, with or without a ticket. Gate them with `EventMask` intercept.
- Yanked releases: 0.97.0 and 0.99.0–0.103.0 [R]. Pin `=0.103.1`.
- The production warning is still in the README on main [R]. No throughput benchmarks were run [R].

### 7.2 iroh-docs 0.101.0

- A multi-author replica keyed by `(namespace, author, key)`. The value is a BLAKE3 hash, a length, and a microsecond timestamp. The bytes stay in iroh-blobs. The newest timestamp wins. Any number of authors may write, given the namespace secret and an author key. Sync is range-based reconciliation [R].
- Read access is possession of the public NamespaceId. A node already syncing a namespace serves any peer that presents it. There is no capability check on sync (`engine/state.rs`) [R].
- Deletes write an empty entry. No tombstone compaction was found. A write `DocTicket` embeds the secret. No key or value size limit was found. Both signatures cover the same bytes, with no domain separation [R].
- Verdict [I]: not the event log. It is last-writer-wins per author-key, with no causal order, no fork handling, no strong remove, and no per-peer ACL. It could serve as a derived index.
- Note for the records feature [I]: a read `DocTicket` grants sync to anyone holding it. That is expected for records. Do not put anything sensitive there without end-to-end encryption.

### 7.3 Range reconciliation versus p2panda LogSync

- Range-based reconciliation (Meyer, arXiv 2212.13567, an unreviewed draft): recursively split sorted ranges, compare fingerprints, and send items once a range is small. Cost is about O(n_Δ log n) bits in O(log n) rounds [R].
- iroh-docs' `ranger` is private: XOR of per-entry BLAKE3, split factor 2, `max_set_size` 1. XOR is linear. The paper covers adversarial collisions in §5.1 [R].
- p2panda-sync 0.7.1's LogSync is not range-based. Its messages are Have, PreSync, Operation, and Done. The Have list enumerates every (author, log, height) on both sides, so bytes grow with the number of chains [R].
- Comparison [I]: range sync wins on bytes when chains outnumber new events, which is our case of many quiet per-author chains. LogSync wins on round trips, at about two whatever the diff.
- Nostr NIP-77 (draft), Negentropy, is a deployed range-style protocol: 32-byte IDs, 16-byte fingerprints from SHA-256 over `(sum mod 2^256 ‖ count)` [R].
- Effort [I]: pinning LogSync to `=0.7.1` is hours. Our own range sync is about 500–800 lines, or vendor `ranger`.
- Verdict [I]: spike S1 should measure both with 1,000 authors and two new events.

### 7.4 iroh-tickets 1.0.0

- `EndpointTicket` is an EndpointId plus transport addresses, base32 of postcard, with the prefix `endpoint`. It is unsigned and has no expiry [R].
- Holders learn the home relay and any IPs included. A ticket cannot be revoked. `AddrInfoOptions::Id` (the default) and `Relay` keep direct IPs out of DocTickets [R].
- Estimated lengths, not measured [I]: relay only about 122 characters; relay plus 2 IPv4 and 1 IPv6 about 184; relay plus 4 IPv4 and 2 IPv6 about 247.
- Verdict [I]: adopt as is. Build the address set for an invite deliberately, rather than calling `endpoint.addr()` blindly.

### 7.5 iroh discovery and privacy

- The N0 preset is Minimal, plus a pkarr publisher and resolver, plus DNS lookup on `dns.iroh.link`, plus default relays [R]. Pkarr packets are signed by the endpoint, have a 30-second TTL, and are republished every 5 minutes [R].
- By default N0 publishes only relay URLs (`AddrFilter::relay_only`), so no IPs [R]. Records are public TXT entries under `_iroh.<z32-id>.dns.iroh.link`, with optional `user-data` of up to 245 bytes [R].
- Each lookup probably reveals the EndpointId to n0 [I].
- Minimal dials only direct IPs. With `MemoryLookup::add_endpoint_info`, ticket-only dialling needs no third-party lookup [R].
- The DHT (`iroh-mainline-address-lookup`) and mDNS (`iroh-mdns-address-lookup`) are separate crates, both 0.6.0 [R]. DHT nodes see publisher IPs unless filtered [I]. BEP 44 caps DHT values at 1,000 bytes [R].
- Our state [V]: client endpoints use the N0 preset (`meta_client.rs:88`, `commands/world.rs:39`), so resolving a bare node ID queries n0. Most internal and test endpoints use `Minimal`.
- Decision needed [I]: keep N0 for convenience, or use `Minimal` and carry addresses in tickets for invites.

### 7.6 ALPN and Router

- ALPN strings in iroh: `/iroh-bytes/4`, `/iroh-sync/1`, `/iroh-gossip/1`. The examples use `n0/iroh/examples/0`. The trailing number is a convention [R].
- `Router::builder().accept(ALPN, handler)` sets the endpoint's ALPN list in registration order [R].
- An unknown ALPN fails in TLS. rustls sends a fatal `no_application_protocol` when nothing overlaps. The Router's own check only logs and drops [R + I]. For migrations, register old and new ALPNs together.
- Our ALPNs [V]: `/id-envelope/1`, `/id-artifact-pull/1`, `/id-artifact-push/1`, `/id-world/1`, and `/iroh-meta/2`. The last one uses the iroh prefix, while the others use `/id-`. Naming should be made consistent when the next protocol changes.

### 7.7 Subduction and Sedimentree

- subduction_core 0.19.0 (2026-10-07). crates.io lists MIT OR Apache-2.0; GitHub says Apache-2.0 [R]. sedimentree_core 0.15.0 [R].
- Sedimentree depth is set by the leading zero bytes of a commit's BLAKE3 digest, about 1 in 256 per byte [R]. Commits are `LooseCommit { digest, parents, blob_meta }`. Batch sync sends a per-request 128-bit SipHash seed and 8-byte fingerprints [R].
- Transports [R]: `subduction_iroh` (compatible with iroh 1.x), WebSocket (5 MB maximum frame), and HTTP long-poll. Auth is `StoragePolicy` with `authorize_fetch` and `authorize_put`. `subduction_keyhive` maps fetch to Relay and put to Edit.
- Maturity [R]: the README says "very unstable API… DO NOT use for production". 21 releases since 2025-10-27. The design documents read do not describe fork or equivocation handling. Relays see sizes, counts, and timing.
- A bug candidate [R]: `loose_commit.rs:125` writes the parent count with `as u8`, which truncates above 255 parents.
- Verdict [I]: not a drop-in. p2panda-sync fits per-author chains better. Subduction models a per-sedimentree commit DAG, without per-author freeze, fork proof, or per-event auth. Revisit if DAG sync becomes the bottleneck.

### 7.8 iroh-willow

- Only `iroh-willow` 0.0.1 is on crates.io (2025-02-07). Its `lib.rs` says "This work is not released yet". The main repo pins iroh 0.34 plus git forks [R]. Do not adopt.

---

## 8. Invitations

### 8.1 Seitan V2 (Keybase), checked against source

- Source: `keybase/client`, `go/teams`. BSD-3. Pushed 2026-10-09 [V: fetched `go/teams/seitan.go`].
- Token alphabet [V]: `abcdefghjkmnpqrsuvwxyz23456789`, 30 symbols, about 83.4 bits for 17 random characters. A `+` sits at index 6, so the full token is 18 characters [R for the `+`].
- siKey [V]: scrypt of the lowercased token, with an empty salt, N = 2^10, r = 8, p = 1, and a 32-byte output.
- Invite ID [V for HMAC-SHA512 and the 15-byte truncation]: the first 15 bytes of HMAC-SHA512 over `msgpack{stage: "invite_id", version: 2}`, followed by a tag byte. The tag is reported as 0x27 but not checked by the lead.
- Ed25519 seed [R]: the first 32 bytes of HMAC-SHA512 over `msgpack{stage: "eddsa", version: 2}`.
- The admin stores only the Ed25519 public key, encrypted under a team key with the label `Keybase-Derived-Team-NaCl-SeitanInviteToken-1`. The token never reaches the chain [R].
- Accept message [R]: `msgpack{stage: "accept", uid, eldest_seqno, ctime, invite_id, version: 2}`, signed with the derived key.
- Test vector [R]: token `4uywza+b3cga7rd6yc` gives inviteID `6303ec43bd61d21edb95a433faf06227`. The researcher reproduced it in Python.
- Risks [R]: V2 has no freshness check on `ctime` in the admin path; V3 has one. N = 1,024 is a cheap stretch. That is fine for random 83-bit tokens, weak for guessable ones. The server code is closed.
- Build [R + I]: about 300–400 lines with tests, plus msgpack encoding. Reusable here: ed25519-dalek 3, sha2 0.11, hmac, scrypt (transitive), rmp-serde (direct). We have no NaCl secretbox crate in the lock.
- Verdict [I]: adopt the V2 derivations exactly, since they are verified. Add a freshness check on `ctime`. Put the test vector in our tests.

### 8.2 localfirst/auth invitations (archived)

- Seed: 16 base58 characters. Invitation ID: the first 15 base58 characters of a keyed BLAKE2b-256 over the stretched seed. Public key: Ed25519 from the seed. Proof: a signature over `{id, invitee, keyHash}`. Fields for `maxUses`, `expiration`, and `revoked`. An `admitted` list blocks replay [R].
- Discrepancy [R]: the comment in `deriveId.ts` says scrypt, but `stretch.ts` is a fixed-key BLAKE2b-256 for inputs of 16 bytes or more.
- `id` and `publicKey` are posted in the clear, so replicas can test seed guesses offline. That is safe only for random seeds [R + I].
- Archived. Last pushed 2026-10-08. README: "no longer maintained". MIT [R].
- Verdict [I]: copy the proof design. The signature binds the invitee and a key fingerprint, so the admitter cannot choose keys. Copy the `admitted` list and the limit fields. Do not copy the stretch.

### 8.3 Real invite mechanisms

- Signal group link [R, Android strings]: a bearer link. Optional admin approval. A reset invalidates the old link.
- Matrix [R]: `invite` (typed), `knock` (a request that an admin invites or denies), `restricted` (membership in an allow-listed room), and `knock_restricted`.
- Discord [R]: bearer. Default `max_age` 86,400 seconds (0 means never; maximum 604,800). `max_uses` 0 means unlimited (maximum 100). `DELETE /invites/{code}` revokes.
- Telegram [R]: bearer. `expire_date`. `member_limit` from 1 to 99,999. `creates_join_request` gives admin approval. `revokeChatInviteLink`.
- Keybase invite link [R]: `/i/t/{id}#{ikey}`. The key sits in the URL fragment, so the server sees only the ID. That is a precedent for keeping the secret out of anything the server sees.
- Verdict [I]: our link shape `id:join?group=…&invite=…&ticket=…` with the secret in the fragment follows the Keybase pattern. Expiry, use limits, per-invite revocation, and optional approval are cheap.
- Risks [R + I]: bearer links leak when forwarded. Use counts race across replicas unless the chain serialises accepts, which Keybase does at admission.

---

## 9. Pull and replication privacy

### 9.1 Prior art

- SSB [R]: follows are public, and public feeds replicate three hops out. Private messages expose sender, time, and size. Recipients and body are hidden. The private-box size is 56 + 33 × recipients + plaintext bytes, so size leaks the recipient count [R + I].
- Hypercore [R]: `discoveryKey` is BLAKE2b-256 keyed with the label "hypercore" over the core key, so peers can find each other without revealing the key. Replication needs a per-session capability proof. Block encryption uses XSalsa20 with no per-block MAC, applied before append.
- Nostr [R]: pubkey, kind, tags, `created_at`, and content are plaintext to relays. NIP-04 is deprecated and leaks metadata. NIP-42 lets relays require AUTH to read. NIP-44 exposes `created_at` and the sender's IP. NIP-59 hides the author but keeps recipient tags.
- Takeaway [R]: every system leaks metadata to replicas. Only encryption survives copying.

### 9.2 Hooks for authorised pull

- iroh 1.3.0 `EndpointHooks::after_handshake(&Connection)` sees the remote ID and the ALPN, and can reject [R].
- iroh-blobs 0.103.1 `EventMask`: `get` and `get_many` in Intercept mode can accept or reject each `GetRequest`, replying `AbortReason::Permission`. No interception is on by default [R].
- iroh-docs 0.101.0 has no capability check on sync [R].
- p2panda-net on main (unreleased): `SyncHooks`, `SyncBlockList`, and `ConnectionBlockList`, which are iroh endpoint hooks. None of these is in 0.7.1 [R].
- Ours today [R]: `access.rs` intercepts push only, so reads are unaffected. `world_net.rs` checks `remote_id` in a custom ALPN handler.
- Effort [I]: a blobs intercept is about 40 lines. A docs sync check against an allowlist is about 100 lines. p2panda via a git pin to main, or copy the check.

### 9.3 What advisory authorisation can and cannot guarantee

- Can [R]: honest nodes refuse unauthorised IDs, per request or per topic, and emit audit events.
- Cannot [R]: stop an authorised node from re-serving. Blobs are addressed by hash, so a provider cannot bind content to one requester. Revocation cannot recall copies. The check authenticates a node ID, not a person. Hashes, sizes, and topics reach whoever is served. Only encryption, with keys held by authorised members, holds after copying.
- Decision applied in the parent document: pull is open by default, configurable to authorised, and documented as advisory.

---

## 10. Time

### 10.1 C2SP tlog-witness

- A log POSTs a checkpoint and a consistency proof (at most 63 hashes) to `add-checkpoint`. The witness checks signatures and the proof against the size it last cosigned, returning 409 on a mismatch and 422 on a bad proof. It then returns a timestamped cosignature. The spec sets no quorum [R].
- Rust [R]: `signed_note` 0.2.0 (BSD-3, from cloudflare/azul) and `tlog_tiles` 0.2.0. Neither has cosigning or the witness HTTP API. The Go implementation is `transparency-dev/witness` (Apache-2.0, pushed 2026-09-28).
- The witness network calls itself experimental. Sunlight names no witnesses [R].
- Effort [I]: about two weeks. Our per-author chains are not Merkle trees, so consistency proofs need a tree or a chain-segment scheme.
- Risks [R]: each checkpoint reveals the origin, size, and root. Witness timestamps are self-asserted.

### 10.2 OpenTimestamps

- Calendars batch digests into a Merkle tree and anchor the root in a Bitcoin transaction. A proof is an op path ending in a Bitcoin attestation, or a pending calendar URI until it is upgraded [R].
- Rust [R]: `opentimestamps` 0.2.0 (MIT OR Apache-2.0, 2023-04-12) verifies proofs. Creating proofs needs the calendar HTTP API, which the researcher did not read.
- Latency [R + I]: otsd defaults allow one Bitcoin transaction per 6 hours and wait for 6 confirmations, so expect hours per proof. Calendars can withhold upgrades.
- Compared with our idea [R]: no operator clock is trusted. Time comes from Bitcoin block headers. Calendars are free.
- License [R]: GitHub reports none for `python-opentimestamps`.

### 10.3 RFC 3161

- A TSA signs a `TSTInfo` containing `genTime`. The token shows the data existed before that time [R].
- Rust [R]: `x509-tsp` 0.1.0 (Apache-2.0 OR MIT) and `tsp-http-client` 0.1.0 (MPL-2.0). Neither validates certificate chains.
- Risks [R]: one TSA is a trust point. Identical hashes sent to several TSAs let observers link them.
- Real use [R]: Microsoft Authenticode timestamping with SHA-256.

### 10.4 Roughtime (RFC 10049, Experimental, October 2026)

- The client sends a 32-byte nonce. The server signs a Merkle root over batched requests and returns MIDP ± RADI. The nonce proves the response came after the server received the request [R].
- Rust [R]: `roughenough-protocol` 2.1.0 (Apache-2.0 OR MIT, 2026-10-07).
- Cloudflare's Roughtime README says "DO NOT USE IN PRODUCTION" and targets earlier drafts. The roughenough repo is active [R].

### 10.5 Hybrid logical clocks and p2panda-core's timestamp module

- Kulkarni et al. (2014): an HLC is `(l, c)`. On receive, `l` becomes the maximum of the local `l`, the message's `l`, and physical time. The theorem: if `e` happened before `f`, then HLC(e) < HLC(f). The clock-sync bound is |l − pt| ≤ ε [R].
- p2panda-core's `HybridTimestamp` (`now`, `increment`, `from_parts`) has no receive rule and no drift check, so it cannot give the theorem across nodes [R].
- `uhlc` 0.9.0 (EPL-2.0 OR Apache-2.0) provides `HLC::new_timestamp`, `HLC::update_with_timestamp`, and `HLCBuilder::with_max_delta`. It has 6.6M downloads and is used by zenoh [R].
- Verdict [I]: display ordering only. Use `uhlc` if needed.

### 10.6 Synthesis, and a correction

- Options [R + I]: (1) an in-band witness event whose payload is a Merkle root of a batch, with heads in `after`. (2) A sidecar C2SP-style cosignature over `(chain, seq, root)`. (3) External anchors (OpenTimestamps, RFC 3161, Roughtime) on batch roots.
- Correction [R]: a witness proves an event existed at or before the signed time, not at or after. The parent document §12 said "at or after", which is wrong. Corrected in the parent's appended section.
- Quorum [I]: Sigsum's split-view bound needs k > 2n/3 witnesses. Examples: 3-of-3, 4-of-5, 5-of-7.
- Who runs witnesses [I]: home servers suit this, since the C2SP witness serves many rarely active logs. With n = 3 the quorum is 3, so one outage stalls cosigning.
- Verdict [I]: defer, as the owner agreed. If we build it first, start with option (3), using OpenTimestamps on batch roots, since it trusts no operator clock. Add option (2) later for consistency checks.

---

## 11. Libraries (crates.io, checked 2026-10-11)

| Crate                          | Version (updated)    | License             | Downloads     | Role for us                                       |
| ------------------------------ | -------------------- | ------------------- | ------------- | ------------------------------------------------- |
| p2panda-core                   | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 21,268        | Header and operations. MSRV 1.96.                 |
| p2panda-auth                   | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 2,771         | StrongRemove and group state. MSRV 1.96.          |
| p2panda-encryption             | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 8,074         | DataScheme. MSRV 1.96.                            |
| p2panda-spaces                 | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 1,555         | Watch only. MSRV 1.96.                            |
| p2panda-sync                   | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 10,514        | LogSync, candidate in S1.                         |
| p2panda-stream                 | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 8,459         | Not recommended. Holds pending items forever.     |
| p2panda-store                  | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 11,498        | SQLite. Do not use for the index (see §6.4).      |
| p2panda-blobs                  | 0.7.1 (2026-08-21)   | MIT OR Apache-2.0   | 4,606         | Placeholder. Not adopted.                         |
| p2panda-net, p2panda-discovery | 0.7.1                | MIT OR Apache-2.0   | 9,949; 10,698 | Not now.                                          |
| iroh-blobs                     | 0.103.1 (2026-10-06) | MIT OR Apache-2.0   | 777,435       | Payloads. Pin `=0.103.1`.                         |
| iroh-docs                      | 0.101.0 (2026-06-15) | MIT/Apache-2.0      | 186,570       | Records. Derived index only.                      |
| iroh-tickets                   | 1.0.0 (2026-06-15)   | MIT OR Apache-2.0   | 913,811       | Invites and addresses.                            |
| keyhive_core                   | 0.6.0 (2026-09-25)   | Apache-2.0          | 11,124        | Design input.                                     |
| keyhive_crypto                 | 0.2.1 (2026-06-26)   | Apache-2.0          | 10,372        | Dependency of beekem.                             |
| beekem                         | 0.4.0 (2026-09-25)   | Apache-2.0          | 10,232        | Large-group candidate, after spike.               |
| subduction_core                | 0.19.0 (2026-10-07)  | MIT OR Apache-2.0   | 2,891         | Design input.                                     |
| sedimentree_core               | 0.15.0 (2026-10-07)  | MIT OR Apache-2.0   | 3,454         | Design input.                                     |
| cbor-core                      | 0.10.1 (2026-06-01)  | MIT                 | 6,081         | Signed bytes. Same as p2panda-core.               |
| dcbor                          | 0.25.2 (2026-03-16)  | BSD-2-Clause-Patent | 152,808       | Source of stricter rules.                         |
| ciborium                       | 0.2.2 (2024-01-24)   | Apache-2.0          | 259,563,158   | Excluded (key order).                             |
| minicbor                       | 2.3.0 (2026-07-23)   | BlueOak-1.0.0       | 16,672,965    | Excluded (key order).                             |
| redb                           | 4.3.0 (2026-10-02)   | MIT OR Apache-2.0   | 13,043,993    | Recommended for the index (S6).                   |
| rusqlite                       | 0.40.2 (2026-08-08)  | MIT                 | 122,455,021   | Alternative for S6.                               |
| fjall                          | 3.1.12 (2026-10-03)  | MIT OR Apache-2.0   | 1,794,354     | Avoid (not durable by default).                   |
| crdts                          | 7.3.2 (2023-08-08)   | Apache-2.0          | 2,585,156     | Not recommended (stale).                          |
| ed25519-dalek                  | 3.0.0 (2026-07-06)   | BSD-3-Clause        | 233,777,334   | Already in lock. Use `verify_strict`.             |
| hpke                           | 0.14.1 (2026-09-06)  | MIT/Apache-2.0      | 11,435,752    | In lock at 0.13.0. Not the same crate as hpke-rs. |
| openmls                        | 0.9.1 (2026-10-07)   | MIT                 | 896,683       | Not a drop-in (§3.6). MSRV 1.91.                  |
| distributed-topic-tracker      | 0.3.5 (2026-06-15)   | MIT                 | 19,011        | Already in lock.                                  |

Crate metadata came from the crates.io API. The lead checked every row. The lead did not check the MSRV of openmls or of the non-p2panda crates.

---

## 12. Verified by the lead in this session

- `Header::verify` returns `true` in non-test builds (p2panda-core 0.7.1, `operation/header.rs:185–194`).
- DataScheme `add` mints no secret. `create`, `update`, and `remove` do (p2panda-encryption 0.7.1, `data_scheme/group.rs`, lines 86–163).
- Spaces Promote and Demote are `unimplemented!()` in 0.7.1 (`event.rs:249–250, 297–298`). Pre-key lifetime is 90 days (`config.rs:25`).
- The p2panda-core, p2panda-auth, p2panda-encryption, and p2panda-spaces crates declare `rust-version = "1.96"`. `pkgs/id/Cargo.toml` declares `1.91.0`.
- The auth authority-graph builder calls `expect("all operations present in map")`.
- Keyhive's convergent-capabilities design states the cascade rule (line 86).
- RFC 9420 §14 requires an application-level answer to conflicting Commits.
- Keybase `go/teams/seitan.go`: the alphabet, scrypt parameters (N = 2^10, r = 8, p = 1, 32 bytes), and the HMAC-SHA512 invite-ID truncation.
- iroh-docs' default features include `redb-v2-migration`, which enables a dependency on redb 3.x.
- The lock file has redb 3.1.3 and 4.3.0, hpke 0.13.0, ed25519-dalek 3.0.0, distributed-topic-tracker 0.3.5, and iroh-gossip 0.101.0.
- Our client endpoints use the N0 preset (`meta_client.rs:88`, `commands/world.rs:39`). Our ALPN names are listed in §7.6.

Not verified by the lead: the cbor-core strictness results, the resolver panic path, Biscuit's SPEC, the OpenStreetMap and Wikidata examples, the openmls benchmark sheet, and every "[X]" probe result.

---

## 13. Open questions raised by the research

1. Cascade definition: adopt UCAN's "no alternate valid path" rule? (§4.1)
2. Dry run before a removal applies: required? (§4.1)
3. Transfer: who signs, and what happens when the successor lacks the scope? (§4.2)
4. Thresholds count devices. Add a third tier at about 1,000, where the cost evidence starts to bite? (§3.8)
5. Large groups: BeeKEM after a spike, or MLS with a home server that orders commits? (§3.6, §3.9)
6. Sync: LogSync or range reconciliation? Spike S1 measures it. (§7.3)
7. Storage: redb or rusqlite? Spike S6 decides. (§6.4)
8. Seitan V2: adopt the verified derivations, with a `ctime` freshness check? (§8.1)
9. Pre-key lifetime: 90 days (spaces) or the encryption crate's default? (§2.3)
10. Toolchain: bump `rust-version` to 1.96. Confirm nothing else in the workspace needs a lower one. (§0)
11. iroh-docs: keep only for records, and disable `redb-v2-migration`? (§6.4)
12. Discovery: keep N0 for client endpoints, or move to `Minimal` plus tickets? (§7.5)
13. Tag precedence against strong remove: follow the content policy? (§6.1)
14. Waiting-queue caps and expiry: what values? (§1.4)
15. Witness option (3) with OpenTimestamps first? (§10.6)

---

## 14. Sources

Specifications and primary documents:

- RFC 9420 (MLS): https://www.rfc-editor.org/rfc/rfc9420.txt
- RFC 9750 (MLS architecture): https://www.rfc-editor.org/rfc/rfc9750.txt
- RFC 8949 (CBOR): https://www.rfc-editor.org/rfc/rfc8949
- CBOR::Core draft-25: https://www.ietf.org/archive/id/draft-rundgren-cbor-core-25.txt
- dCBOR draft-18: https://www.ietf.org/archive/id/draft-mcnally-deterministic-cbor-18.txt
- RFC 8032 (EdDSA): https://www.rfc-editor.org/rfc/rfc8032
- RFC 9180 (HPKE): https://www.rfc-editor.org/rfc/rfc9180
- RFC 3161 (time-stamp protocol): https://www.rfc-editor.org/rfc/rfc3161
- RFC 10049 (Roughtime): https://www.rfc-editor.org/rfc/rfc10049
- DCGKA paper (ePrint 2020/1281): https://eprint.iacr.org/2020/1281 (archived PDF: https://web.archive.org/web/2024id_/https://eprint.iacr.org/2020/1281.pdf)
- Taming the many EdDSAs (ePrint 2020/1244): https://eprint.iacr.org/2020/1244
- Range-based set reconciliation (arXiv 2212.13567): https://arxiv.org/abs/2212.13567
- Observed-remove CRDT study (RR-7506): https://inria.hal.science/inria-00555588/document

p2panda and crate sources:

- p2panda repo: https://github.com/p2panda/p2panda (encryption design post, 2025-02-24: https://p2panda.org/2025/02/24/group-encryption.html)
- Issues cited: https://github.com/p2panda/p2panda/issues/807, /1426, /1244, /1345, /783, /799
- p2panda-encryption: https://github.com/p2panda/p2panda/tree/main/p2panda-encryption
- ed25519-dalek: https://crates.io/crates/ed25519-dalek

Keyhive, Subduction, and related:

- Keyhive: https://github.com/inkandswitch/keyhive (design: `design/convergent_capabilities.md`, `design/group_membership.md`, `design/causal_encryption.md`; notebook: https://www.inkandswitch.com/keyhive/notebook/02/)
- BeeKEM: https://github.com/inkandswitch/keyhive/tree/main/beekem
- Subduction and Sedimentree: https://github.com/inkandswitch/subduction
- Keybase Seitan (Go source): https://github.com/keybase/client (`go/teams/seitan.go`, `go/teams/seitan_v2.go`)
- Keybase book snapshot: https://web.archive.org/web/20220723162211/https://book.keybase.io/docs/teams/seitan
- localfirst/auth: https://github.com/local-first-web/auth

Delegation and authorisation:

- UCAN spec and revocation: https://github.com/ucan-wg/spec, https://github.com/ucan-wg/revocation, https://raw.githubusercontent.com/ucan-wg/delegation/main/README.md
- Biscuit: https://docs.rs/biscuit-auth, https://github.com/eclipse-biscuit/biscuit
- Macaroons (NDSS 2014): https://theory.stanford.edu/~ataly/Papers/macaroons.pdf
- Meadowcap: https://willowprotocol.org/specs/meadowcap/index.html
- Zanzibar (ATC 2019): https://www.usenix.org/system/files/atc19-pang.pdf
- SpiceDB consistency: https://authzed.com/docs/spicedb/concepts/consistency; OpenFGA consistency: https://openfga.dev/docs/interacting/consistency

Matrix and forks:

- Matrix state resolution v2 and v2.1 (MSC4297): https://spec.matrix.org/latest/rooms/v2/, https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4297-state-resolution-v2_1.md
- Matrix redaction: https://github.com/matrix-org/matrix-spec/blob/main/content/rooms/fragments/v3-handling-redactions.md
- Matrix history visibility: https://github.com/matrix-org/matrix-spec/blob/main/content/client-server-api/modules/history_visibility.md; MSC4268: https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4268-encrypted-history-sharing.md
- Synapse state resolution: https://github.com/element-hq/synapse/blob/develop/synapse/state/v2.py
- Automerge sync and changes: https://github.com/automerge/automerge (`rust/automerge/src/sync.rs`, `sync/bloom.rs`, `op_set2/change/batch.rs`)
- SSB protocol guide: https://ssbc.github.io/scuttlebutt-protocol-guide/; ssb-validate and ssb-db2 sources
- Hypercore: https://github.com/holepunchto/hypercore

iroh and transport:

- iroh-blobs: https://github.com/n0-computer/iroh-blobs; Bao spec: https://github.com/oconnor663/bao/blob/master/docs/spec.md
- iroh-docs: https://github.com/n0-computer/iroh-docs
- iroh-tickets: https://docs.rs/iroh-tickets/1.0.0
- iroh endpoints and presets: https://docs.rs/iroh/1.3.0/iroh/protocol/struct.Router.html
- pkarr: https://pkarr.org; BEP 44: https://www.bittorrent.org/beps/bep_0044.html
- Nostr NIP-77 (Negentropy): https://github.com/nostr-protocol/nips/blob/master/77.md
- Nostr NIP-32, NIP-09, NIP-42, NIP-44, NIP-59: https://github.com/nostr-protocol/nips
- ATProto labels: https://atproto.com/specs/label
- iroh-willow: https://github.com/n0-computer/iroh-willow

Invitations and labels:

- Signal group links (Android strings): https://github.com/signalapp/Signal-Android
- Discord invite API: https://github.com/discord/discord-api-docs (channel and invite resources)
- Telegram Bot API (invite links): https://core.telegram.org/bots/api

Time and witnesses:

- C2SP tlog-witness: https://github.com/C2SP/C2SP/blob/main/tlog-witness.md; Sigsum: https://www.sigsum.org/; witness implementation: https://github.com/transparency-dev/witness
- OpenTimestamps: https://petertodd.org/2016/opentimestamps-announcement; https://github.com/opentimestamps/python-opentimestamps
- Roughtime: https://github.com/cloudflare/roughtime; https://github.com/int08h/roughenough
- HLC (Kulkarni et al., 2014): https://cse.buffalo.edu/tech-reports/2014-04.pdf; uhlc: https://github.com/atolab/uhlc-rs

Governance and values:

- Wikidata ranks: https://www.wikidata.org/wiki/Help:Ranking; P1310: https://www.wikidata.org/wiki/Property:P1310
- OpenStreetMap API v0.6: https://wiki.openstreetmap.org/wiki/API_v0.6; disputed territories: https://wiki.openstreetmap.org/wiki/Disputed_territories
- Wikipedia article titles: https://en.wikipedia.org/wiki/Wikipedia:Article_titles

Group sizes and MLS deployments:

- Signal group size: Signal support page (group limits)
- Discord DAVE protocol: https://raw.githubusercontent.com/discord/dave-protocol/main/protocol.md
- openmls: https://github.com/openmls/openmls; SRLabs audit via https://blog.phnx.im/openmls-independent-security-audit/; Wire core-crypto: https://github.com/wireapp/core-crypto
