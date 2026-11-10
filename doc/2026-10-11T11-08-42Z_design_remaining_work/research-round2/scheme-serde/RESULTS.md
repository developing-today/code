# scheme-serde: how DataScheme serializes its messages, and how to version and encode them

Date: 2026-10-11. Crate: p2panda-encryption 0.7.1 at p2panda rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, checkout `/home/user/.cargo/git/checkouts/p2panda-de6556ee3f16c1a4/3e84cac/`. Serializer: `p2panda_core::cbor::encode_cbor`, which calls cbor-core 0.10.1 (`p2panda-core/Cargo.toml:38`). Toolchain: cargo 1.97.0 (`rustup which --toolchain 1.97.0 cargo`), `CARGO_TARGET_DIR=/home/user/work/spikes/target`. Folder: `/home/user/work/research-round2/scheme-serde/`.

Labels: **verified** = I read the line or saw the output; **inferred** = my reasoning from verified facts; **not checked** = not done.

---

## Questions (verbatim)

Owner: "n10 what are these scheme versions? what options are there why is this custom one the best?"

Brief (from the parent session):

1. List the structs that go on the wire for the data scheme (welcome, PreKey welcome, two-party plaintext, X3DH ciphertext, update or add messages). For each, list which fields are `Vec<u8>`, and which serializer is used.
2. Explain why the stock output is 146.6 bytes per epoch. Explain what `serde_bytes` changes on the wire. Check the S12 variant patch.
3. Evaluate options (a) keep stock, (b) fork with `serde_bytes`, (c) own adapter, (d) own compact encoding, (e) compression, (f) where a scheme version tag goes.
4. For each option: bytes per epoch, lines we own, upstream-change risk, wire compatibility, failure modes.
5. Recommend one option and a versioning scheme. Address "why is this custom one the best?"

---

## Short answers

**Q1, the wire structs.** The data scheme sends four kinds of struct: the welcome (`DirectMessage` with `DirectMessageContent::Welcome`), the 2SM message in its X3DH form (`TwoPartyMessage` holding `X3dhCiphertext`), the 2SM message in its HPKE form (`TwoPartyMessage` holding `HpkeCiphertext`), and the control messages (`ControlMessage`) with one direct message per recipient for update, add and remove. `TwoPartyPlaintext` is not sent as such: it is CBOR-encoded and then encrypted inside the 2SM message. Serialization is `encode_cbor`, which is `cbor_core::Value::serialized(..).encode()` (verified, `p2panda-core/src/cbor.rs:17-20`). The crate has no serializer of its own. The `Vec<u8>` fields are `TwoPartyPlaintext.plaintext` (`two_party.rs:244`, no `serde_bytes`), `X3dhCiphertext.ciphertext` (`x3dh.rs:41`, no `serde_bytes`), and `HpkeCiphertext.kem_output` and `.ciphertext` (`hpke.rs:18,22`, both already `serde_bytes`). The welcome carries the DGM state (`history`), which is our type. The crate has no version field anywhere (verified: `grep -ri version` finds only a doc comment).

**Q2, the 146.6 bytes per epoch.** Each epoch adds one `GroupSecret` to the bundle. Its CBOR is `[bstr(32), uint]`, about 40 bytes (inferred from the encoding rules; the serde_bytes build measures exactly 40.00 per epoch, which matches). In the stock layout the bundle is encoded twice, and both times as a CBOR integer array. The inner layer is `TwoPartyPlaintext.plaintext`: its 40 bytes become about 76.9 bytes (the outer-only build measures 76.9 per epoch, and that number is exactly this inner integer array carried inside a byte string). The outer layer is `X3dhCiphertext.ciphertext`, which holds the AEAD output of those 76.9 bytes, again as an integer array: 76.9 × 1.906 ≈ 146.6 (stock measured 146.46 to 146.53 across three seeds). An integer array costs 1 byte for values below 24 and 2 bytes for the rest, so 1.906 bytes per random byte. Check: the inner-only build carries the outer layer alone over 40 random bytes, 40 × 1.906 = 76.2, and measures 76.24. `serde_bytes` replaces the array with a byte string, so each byte costs exactly 1 byte. The S12 patch (`variants/serde-bytes.patch`) contains the two annotations (patch lines 119 and 130), but it also changes `dcgka.rs` and `group.rs` (the membership fix and `add_from_join`, about 40 added lines) and `Cargo.toml`. So S12's "serde_bytes build" is more than the serialization change. My build with only the two annotations on the pinned source gives 40.00 per epoch and exactly S12's 40,493 bytes at K = 1000.

**Q3, the options.** (a) Stock: no code, 146.5 bytes per epoch. (b) Fork with `serde_bytes` on the two fields: 40.00 per epoch, two lines in the encryption crate. (c) Our own serde structs: not possible as specified. `TwoPartyMessage` and `TwoPartyPlaintext` have private fields and no constructor (`two_party.rs:225-228, 242-255`), so any conversion has to pass through serde on the encoded form. A Value-level transcoder does work without a crate change, but only on the outer field: 76.9 per epoch for joiners, and it cannot reach the inner layer, which sits inside the AEAD. (d) Our own compact welcome: the bundle's own CBOR is 40 bytes per epoch, so the floor is near 40 and the gain is at most about 15% per epoch. Reaching the bundle also means replacing the 2SM welcome construction, which is custom cryptography. Not recommended. (e) Compression: the brief expected little gain. On stock bytes the gain is large (bzip2 at −9 gives 78,344 bytes at K = 1000, 47% less), because the integer-array framing is predictable. On v1 bytes compression gives nothing (+4 to +534 bytes). (f) Version tag: 5 to 12 bytes per welcome. Put it in the signed group policy as the authority, with a checked copy in the event header and the welcome envelope.

**Q4, numbers.** The table in §Options has all of them. The headline: joiner welcome at K = 1000 is 147.3 KB stock, 40.5 KB with `serde_bytes` on both fields, 77.4 KB with the outer field alone (transcoder), and 76.9 KB with the inner field alone. The per-recipient update message is 375 to 383 bytes stock and 342 to 344 bytes with `serde_bytes`. So `serde_bytes` saves only about 10% on updates, because the HPKE outer layer is already a byte string. The inner field is what matters for updates and re-adds; the outer field only matters for PreKey welcomes.

**Q5, recommendation.** Choose (b): a fork with `serde_bytes` on `TwoPartyPlaintext.plaintext` and `X3dhCiphertext.ciphertext`, named DataScheme scheme v1 from the first release, with the version bound to the signed group policy and checked in the event header and the welcome envelope. "This custom one" is the best option that reaches the floor. It is 3.66 times smaller on joiner welcomes, it adds two lines to a fork we already carry for `from_join` (N9), it uses the attribute the crate already uses for every other byte field, and it has no migration cost, because nothing has shipped: nothing in `/home/user/code` depends on p2panda-encryption (verified by grep). It is not free. It is incompatible with stock in both directions (verified), the update path gains only about 10%, and the version tag and determinism fixes in §Recommendation are required. If the owner will not carry any crate patch, the fallback is (c), the transcoder, which gets half the joiner saving and nothing elsewhere.

---

## Evidence

### E1. Wire structs and their byte fields (pinned rev, verified)

| Struct (file:line) | Fields | `Vec<u8>` | `serde_bytes` today |
|---|---|---|---|
| `DirectMessage` (`data_scheme/dcgka.rs:564-570`) | `recipient: ID`, `content` | none | n/a |
| `DirectMessageContent::Welcome` (`dcgka.rs:606-613`) | `ciphertext: TwoPartyMessage`, `history: DGM::State` | none directly; `history` is our DGM state (test DGM uses `HashSet`, `data_scheme/test_utils/dgm.rs:35`) | n/a |
| `DirectMessageContent::TwoParty` (`dcgka.rs:614-616`) | `ciphertext: TwoPartyMessage` | none | n/a |
| `TwoPartyMessage` (`two_party/two_party.rs:224-228`) | `ciphertext: TwoPartyCiphertext`, `key_used: KeyUsed`. **Private fields.** | none | n/a |
| `TwoPartyCiphertext` (`two_party.rs:230-237`) | `PreKey(X3dhCiphertext)` / `Hpke(HpkeCiphertext)` | none | n/a |
| `X3dhCiphertext` (`two_party/x3dh.rs:28-45`) | `identity_key: PublicKey`, `onetime_prekey_id: Option<_>`, `prekey_id`, `ciphertext: Vec<u8>` (line 41), `ephemeral_key: PublicKey` | `ciphertext` | `PublicKey` is `serde_bytes` (`x25519.rs:65`) |
| `HpkeCiphertext` (`crypto/hpke.rs:15-24`) | `kem_output`, `ciphertext` | both | both (`hpke.rs:18,22`) |
| `TwoPartyPlaintext` (inside the AEAD, `two_party.rs:241-255`) | `plaintext: Vec<u8>` (line 244), `receiver_new_secret: SecretKey`, `sender_new_verifying_key: PublicKey`, `sender_next_index: u64` | `plaintext` | no (`Secret` is `serde_bytes`, `secret.rs:22`) |
| `ControlMessage` (`dcgka.rs:484-490`) | `Create { initial_members: Vec<ID> }`, `Update`, `Remove { removed: ID }`, `Add { added: ID }` | none | n/a |
| `GroupSecret` (`data_scheme/group_secret.rs:39-40`) | `GroupSecret(Secret<32>, Timestamp)` | none | the `Secret` is |
| `SecretBundleState` (`group_secret.rs:109-112`, Serialize at 174-186) | a CBOR sequence of `GroupSecret`, written in `HashMap` order (line 180) | none | n/a |
| application data (`data_scheme/data.rs:5-13`, `group.rs:443`) | XChaCha20-Poly1305 output, `Vec<u8>`; the crate defines no serialized struct for it | the output | n/a |

The update, add and remove messages (`dcgka.rs:343-374`, `send_group_secret`) send one 2SM message per recipient. Each carries one `GroupSecret` (`to_bytes` at `group_secret.rs:86-88`, `encode_cbor(self)`). The welcome carries the whole `SecretBundleState` (`dcgka.rs:260`, `bundle.to_bytes()`), and that is the part that grows with epochs.

### E2. Serializer and serde mapping

- `encode_cbor` = `cbor_core::Value::serialized(value)?.encode()` (verified, `p2panda-core/src/cbor.rs:17-20`). `decode_cbor` uses the LENIENT strictness (`cbor.rs:33-36`); `decode_cbor_strict` exists (`cbor.rs:47`).
- `Vec<u8>` serializes as a CBOR array of unsigned integers. Observed: `[84, 01, 02, 03, 18, c8]` for `[1,2,3,200]`.
- `#[serde(with = "serde_bytes")]` serializes as a byte string. Observed: `[44, 01, 02, 03, c8]` for the same data (run-all.txt, serde read check).
- The cbor-core decoder rejects the other form: `deserialize_seq` errors with "expected array, got {type}" (verified, `cbor-core-0.10.1/src/ext/serde.rs:758-766`); `deserialize_bytes` and `deserialize_byte_buf` go through `visit_borrowed_bytes` (`serde.rs:724-725`) and fail on an array with "incompatible CBOR type Array" (observed). serde_bytes' own `ByteBuf` visitor does implement `visit_seq` (`serde_bytes-0.11.19/src/bytebuf.rs:211`), but cbor-core never calls it, so `ByteBuf` cannot read arrays.
- Struct fields become text keys, sorted length-first (verified, `welcome-k1-stock.diag`: `"history"` comes before `"ciphertext"`); enum variants are externally tagged (`{"Welcome": {...}}`); unit variants are text (`"PreKey"`); `None` is `null` (verified in the same diagnostic).
- `IDs`: `p2panda-core` `VerifyingKey` serializes via `serialize_hex` (`p2panda-core/src/serde.rs:92-99`), which writes a byte string for non-human-readable formats such as CBOR (`serde.rs:21-31`). `ed25519-dalek` 3.0.0 (`pkgs/id` uses version 3, `pkgs/id/Cargo.toml:44`) uses `serialize_bytes` (`ed25519-dalek-3.0.0/src/verifying.rs:700-704`). So product IDs built on `VerifyingKey` are byte strings. `[u8; 32]` would be an array (inferred from serde's tuple rule; not measured).

### E3. The S12 variant patch

`/home/user/work/spikes-next/s12-welcome/variants/serde-bytes.patch` (verified by grep):

- Files changed: `Cargo.toml` (explicit deps, no `[lints]`); `src/data_scheme/dcgka.rs` (the membership fix: the patch's comments say "The joiner applies its own add in `process_welcome`, after the welcome's history" and "The welcome's history predates this add, so the joiner applies the add itself", patch lines 45 and 71); `src/data_scheme/group.rs` (new public `add_from_join`, patch lines 84-107); `src/two_party/two_party.rs` (patch line 119: `#[serde(with = "serde_bytes")]`); `src/two_party/x3dh.rs` (patch line 130: `#[serde(with = "serde_bytes")]`). Patch line numbers are given throughout, not source line numbers.
- So the S12 "serde_bytes" build is the encryption crate with the membership and `from_join` changes and the two annotations. The two annotations are the only change that affects the wire sizes measured below. S12's RESULTS.md (line 34) says the S3 path copy has no `serde_bytes` change, so the S12 variant was built separately.
- My builds start from the pinned source and add only the annotations. `diff -ru` of `src` against the pinned checkout shows exactly the annotation lines (run in this session).

### E4. Builds and commands (all run in this session)

```
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
cd /home/user/work/research-round2/scheme-serde

# variants: pinned checkout copied, Cargo.toml taken from S12's serde-bytes variant (package renamed),
# annotation lines inserted with sed:
#   both:  two_party.rs before "    plaintext: Vec<u8>," and x3dh.rs before "    pub ciphertext: Vec<u8>,"
#   inner: two_party.rs only
#   outer: x3dh.rs only
"$CARGO" build --release --offline        # 1m16s first build, 6.6s incremental
/home/user/work/spikes/target/release/scheme-serde > logs/run-all.txt   # 37 s, seeds 1 to 3, four builds per seed
scripts/compress.sh > logs/compress.txt
```

The harness `src/harness.rs` is adapted from S12's `src/harness.rs`, not a byte copy. Same scenario (create with three members, then K rotations: odd K is `update`, even K is remove Carol and add her back). Removed: timers and recent-N. Added: the update direct message to Bob, the re-add welcome for Carol, and the file dumps. `src/main.rs` adds the serde read check, the cross-decode, the transcoder and the envelope overhead. Stderr was empty. `logs/run-all.txt` is the final run; an earlier run of the same binary gave the same slopes within 0.2 bytes per epoch.

The variant diffs against the pinned source (verified):

```
both:  two_party/two_party.rs +#[serde(with = "serde_bytes")] before plaintext; x3dh.rs +same before ciphertext
inner: two_party/two_party.rs only
outer: two_party/x3dh.rs only
```

### E5. Measurements (final run, `logs/run-all.txt`, three seeds; `logs/summary.txt`)

Per epoch is the least-squares slope over the 11 samples K = 0, 1, 2, 5, 10, 50, 100, 250, 500, 750, 1000, against the number of secrets (K + 1).

| build | joiner welcome, B per epoch (fit, 3 seeds) | intercept, B | joiner welcome at K = 1000, B (seeds 1, 2, 3) | update DM to Bob, B (range over samples and seeds) | re-add welcome, B per epoch | re-add at K = 1000, B (seeds 1, 2, 3) |
|---|---|---|---|---|---|---|
| stock | 146.46 to 146.53 | 597.8 to 603.3 | 147,277 / 147,261 / 147,252 | 375 to 383 | 76.87 to 76.90 | 77,407 / 77,369 / 77,381 |
| both (v1 candidate) | **40.00** (max residual 1.5) | 450.5 | **40,493 / 40,493 / 40,493** | 342 to 344 | 40.01 | 40,424 (all seeds) |
| inner only | 76.24 to 76.26 | 596.2 to 601.2 | 76,953 / 76,939 / 76,965 | 342 to 344 | 40.01 | 40,424 (all seeds) |
| outer only | 76.86 to 76.90 | 447.4 to 457.4 | 77,441 / 77,402 / 77,411 | 375 to 383 | 76.87 to 76.90 | 77,407 / 77,369 / 77,381 |

Reading the table: the inner field sets the update and re-add cost, the outer field sets only the PreKey welcome. The inner-only build's joiner welcome is 40 × 1.906 = 76.2 (matches 76.24). The outer-only build is 76.9, which is the inner integer array (`40 → 76.9`) carried in a byte string.

The welcome at K = 0 for stock is 737, 744, 735 bytes; for both it is 489 bytes (`logs/run-all.txt`, first row of each table).

### E6. Compression (seed 1 dumps, `logs/compress.txt`)

| bytes | raw | zstd -19 | xz -9e | gzip -9 | bzip2 -9 | brotli -11 |
|---|---|---|---|---|---|---|
| stock joiner welcome, K = 1000 | 147,277 | 95,594 | 86,528 | 100,591 | **78,344** | 86,562 |
| stock joiner welcome, K = 1 | 879 | 730 | 784 | 775 | 838 | 665 |
| both (v1) joiner welcome, K = 1000 | 40,493 | 40,507 | 40,564 | 40,547 | 41,027 | 40,497 |
| both (v1) joiner welcome, K = 1 | 529 | 517 | 572 | 549 | 642 | 484 |
| transcoded outer-only, K = 1000 | 77,441 | 77,457 | 77,512 | 77,512 | 78,130 | 77,446 |
| stock update DM | 383 | 385 | 448 | 428 | 519 | 367 |
| both (v1) update DM | 344 | 346 | 408 | 385 | 475 | 329 |
| stock re-add, K = 1000 | 77,407 | 77,423 | 77,480 | 77,465 | 78,106 | 77,412 |
| both (v1) re-add, K = 1000 | 40,424 | 40,438 | 40,492 | 40,476 | 40,953 | 40,428 |

Per epoch from the two points K = 1 and K = 1000 (inferred, (c(1000) − c(1)) / 999): stock bzip2 77.6, xz 85.8, brotli 86.0, zstd 95.0, gzip 99.9. For v1: 40.0 for every compressor, so no gain.

### E7. Cross-decode and the transcoder (all seeds, `logs/run-all.txt`)

Cross-decode, `decode_cbor` of the whole direct message (the inner layer sits inside the AEAD and is not read by this check):

| welcome | read by | result |
|---|---|---|
| stock K = 1 PreKey | both | `Err("incompatible CBOR type Array")` |
| both K = 1 PreKey | stock | `Err("expected array, got Bytes")` |
| stock K = 1 PreKey | inner | `Ok` |
| stock K = 1 PreKey | outer | `Err("incompatible CBOR type Array")` |
| inner K = 1 PreKey | stock | `Ok` |
| outer K = 1 PreKey | stock | `Err("expected array, got Bytes")` |
| stock re-add (HPKE) | both | `Ok` |
| both re-add (HPKE) | stock | `Ok` |

So a PreKey welcome from either side fails on the other side at the direct-message level. The HPKE re-add passes at this level only, because its inner layer is not read here. The inner-only build's `Ok` on a stock welcome is a result at the direct-message level only; the inner decode happens after decryption, and I did not run it (inferred that it would fail).

Transcoder, `to_wire` replaces the integer array at `content → Welcome → ciphertext → ciphertext → PreKey → ciphertext` with a byte string, and `from_wire` reverses it (`src/main.rs`):

| check | seed 1 | seed 2 | seed 3 |
|---|---|---|---|
| stock K = 1 → wire | 879 → 601 B | 893 → 605 B | 888 → 598 B |
| stock K = 1000 → wire | 147,277 → 77,441 B | 147,261 → 77,402 B | 147,252 → 77,411 B |
| `from_wire(to_wire(x)) == x`, K = 1 and K = 1000 | true, true | true, true | true, true |
| stock reads the round trip | Ok | Ok | Ok |
| outer build reads the wire form | Ok | Ok | Ok |
| wire size equals the outer-only build's welcome size | 77,441 = 77,441 | 77,402 = 77,402 | 77,411 = 77,411 |

The transcoder output has the same size as the outer-only build's welcome in every seed. The two welcomes come from separate processes, so I did not compare the bytes themselves; the size match is evidence of equivalence, not proof.

Re-encoding a decoded welcome with the typed API does not always give the same bytes. Stock K = 1 re-encodes exactly in seeds 2 and 3 and not in seed 1. The transcoded wire form re-encodes exactly in 1 of 6 cases (K = 1 and K = 1000, three seeds). For the stock bytes, decode followed by encode at the `Value` level is an identity (`stock K=1000 bytes are canonical: true` and `stock K=1 ... Value decode then encode is identity: true`, all seeds). I did not run that Value-level check on the transcoded form. The likely cause of the typed-API differences is hash order: the test DGM keeps members in a `HashSet` (`dgm.rs:35`), and the bundle serializes its secrets in `HashMap` order (`group_secret.rs:180`). I did not isolate it (inferred).

Envelope overhead (a version tag around the welcome): array form `[version, body]` adds 5 bytes to the 40.5 KB v1 welcome and 7 bytes to the 77 KB form; map form `{v, body}` adds 12 and 14 bytes.

### E8. Our repository (verified)

- `/home/user/code/pkgs/id` has no `p2panda-encryption` dependency (grep over `*.toml`, `*.rs`, `*.nix`, `*.md`). `pkgs/id/TODO.md:250` names it as a candidate only. There is no DataScheme data in the product, so no existing welcome or event needs to be read.
- The design says `scheme: none` is the opt-out and that DataScheme is the default (`task-05` scope items 6 and 7). Item 6: the scheme and version live in the signed group policy; key bundles advertise supported schemes; a scheme change starts a new epoch; readers keep a decoder for every scheme that ever produced an epoch; a message whose scheme differs from its epoch's is rejected. The acceptance list requires a downgrade test (task-05 line 36).
- Index items: N10, "serde_bytes on the two DataScheme fields", alternatives "Stay on stock encoding until a migration window exists" (remaining-work index, line 98). N20, "Scheme version for serde_bytes", alternative "Stay on stock encoding and accept 3.7 times larger welcomes" (line 108). The decision at task-05 line 106 says "Round 2 scheme-serde decides between the options".

---

## Options

Per-epoch numbers are for the joiner welcome (full history, PreKey path) unless the column says otherwise. "Update DM" is one direct message to one recipient for one update (HPKE path, one secret).

| Option | Joiner welcome, B per epoch | Joiner welcome, K = 1000 | Update DM, B per recipient | Re-add, B per epoch | Lines we own | Upstream-change risk | Wire compatibility with stock | Failure modes |
|---|---|---|---|---|---|---|---|---|
| **(a)** keep stock | 146.5 | 147.3 KB | 375 to 383 | 76.9 | 0 | none while pinned | identical | welcome grows linearly; churn quadratic (S12: 19.5 MB of welcomes at K = 1000 with 500 re-adds); each update costs n − 1 messages of about 380 B |
| **(b)** fork, `serde_bytes` on both fields | **40.00** | **40.5 KB** | 342 to 344 | 40.0 | 2 attribute lines; the fork already exists for `from_join` (N9) | low to moderate: same attribute the crate uses for every other byte field (`secret.rs:22`, `x25519.rs:65`, `xeddsa.rs:31`, `hpke.rs:18,22`), so a fix upstream would likely match; a different layout would conflict | none: both directions fail for PreKey welcomes (E7) | a stock build anywhere in the dependency graph breaks the welcome path; `p2panda-spaces` depends on the crate (not checked) and would pull a second copy unless patched; `decode_cbor` is lenient (`cbor.rs:33-36`); bundle order depends on `HashMap` (`group_secret.rs:180`) |
| (b-inner), inner field only | 76.2 | 76.9 KB | 342 to 344 | 40.0 | 1 line | as (b) | none | the inner decode after decryption would fail on a stock plaintext (inferred, not run); the direct-message decode succeeds (E7) |
| (b-outer), outer field only (same bytes as (c)) | 76.9 | 77.4 KB | 375 to 383 (no change) | 76.9 (no change) | 1 line | as (b) | the outer layer reads as generic CBOR; the inner stays stock | helps PreKey welcomes only; updates and re-adds are not helped |
| **(c)** own serde structs converting to and from the crate's types | not built | — | — | — | — | — | — | not constructible as specified: `TwoPartyMessage` and `TwoPartyPlaintext` have private fields and no constructor (`two_party.rs:225-228, 242-255`; construction only inside `two_party.rs:161, 171`); `X3dhCiphertext` and `HpkeCiphertext` have public fields, but their `Vec<u8>` fields cannot be read from bytes by the crate's deserializer (E2). What would have to change: a crate change (public constructors or `serde_bytes` on the fields, which is option (b)), or a conversion on the encoded form, which is (c') |
| **(c')** transcoder on the encoded welcome, no crate change | 76.9 (outer-only equivalent) | 77.4 KB (77,402 to 77,441) | 375 to 383 (no change) | 76.9 (no change) | about 40 lines for `to_wire`, `from_wire` and `navigate` (`src/main.rs`); tests are not written yet | low for the crate itself, since no fork; coupled to field and variant names (`Welcome`, `PreKey`, `ciphertext`), so a rename breaks it unless golden tests cover it | the wire form is standard CBOR; `from_wire` gives back stock bytes, which the stock decoder reads (E7) | works only on the PreKey welcome; the inner layer is inside the AEAD, so the ceiling is about 1.9 times; must be applied on send and on receive; signed bytes must be taken before transcoding |
| **(d)** own compact welcome | not built; floor about 40 (inferred: bundle CBOR per secret) | about 40.5 KB at best, a gain of at most about 15% per epoch from dropping timestamp bytes | not measured | not measured | 300 or more, plus crypto review (inferred) | highest: replaces the 2SM welcome | none | the bundle is encrypted inside `TwoParty::send` (`two_party.rs:153-171`), so reaching it needs a fork as well; a new transport on the public `x3dh_encrypt` (`x3dh.rs:48`) is custom cryptography; the fixed part, about 450 bytes per welcome, is only 1% at K = 1000 |
| **(e)** compress the welcome | about 78 (bzip2), 86 (xz, brotli), 95 (zstd), 100 (gzip) on stock bytes | 78.3 KB (bzip2) | 367 (brotli, stock) and 329 (brotli, v1): about 4% | derived from the same data, not measured separately | 1 library call plus a size cap | none | breaks stock decoders, so it needs a version tag anyway | decompression bombs (cap the output size); CPU; compressing secret-bearing data can leak if attacker-chosen text shares the context (not the case for random keys, but a policy point); compressor output is not stable across versions, so signed bytes must not be recompressed; on v1 bytes the gain is zero (+4 to +534 bytes) |
| **(f)** version tag | none (tag only) | +5 B (array) or +12 B (map) on 40.5 KB | +1 to 2 B (event header) | — | 10 to 30 lines | none | the tag sits outside the crate's payload, so the crate does not change | a tag without the policy check is a downgrade hole; placement in the table below |

Other measured points: the joiner welcome's fixed part is 450 bytes for v1 and about 600 bytes for stock (intercept). At K = 1 the v1 welcome is 529 bytes and the stock welcome is 879 bytes.

### (f) Where the scheme version goes

| Location | Cost per message | Who reads it, and when | Detection and negotiation | Downgrade protection | Verdict |
|---|---|---|---|---|---|
| Event header (in the signed body) | 1 byte plus a key, about 2 bytes | every receiver, before decoding the payload | the tag must equal the scheme of the epoch the event names, looked up in the policy; no negotiation, since senders use the epoch's scheme | yes, because the tag is signed | use, as a checked copy |
| Welcome envelope (our wrapper around the crate's direct message) | 5 bytes (array) or 12 bytes (map) | the joiner, before calling the crate; the joiner has no group state yet | read the tag first; reject unknown tags with a named error, not a CBOR error; check against the scheme in the signed add event or the policy | only with the policy check | use, as a checked copy |
| Group state (signed policy; `DcgkaState` is persisted, `dcgka.rs:31-33`) | 0 bytes per message | every member, from the signed policy | the policy names the scheme for each epoch; a manage operation changes it and starts a new epoch (task-05 item 6); key bundles advertise supported schemes, so the admin refuses to add a bundle that does not advertise the group's scheme | yes, because the policy is signed | the authority |

The three together match task-05 item 6: the policy is the authority, and the other two are checked copies that let a receiver pick a decoder without trial decoding.

---

## Recommendation

**Choose option (b):** the crate fork with `serde_bytes` on `TwoPartyPlaintext.plaintext` (`two_party.rs:244`) and `X3dhCiphertext.ciphertext` (`x3dh.rs:41`), named DataScheme scheme v1 from the first release, with the version as in the table above.

Why this custom one is the best:

1. It is the only option that reaches the floor. Joiner welcomes drop from 146.5 to 40.00 bytes per epoch (3.66 times), and a 1,000-rotation joiner gets 40.5 KB instead of 147.3 KB. The floor is the bundle's own CBOR, 40 bytes per secret, so nothing better is possible without changing what the crate puts in the bundle.
2. It costs two lines in a crate we already fork for `from_join` (N9). The membership fix and `add_from_join` are needed anyway, so the marginal cost is the two attributes and the tests.
3. It is the crate's own convention. `Secret`, `PublicKey`, `XSignature` and `HpkeCiphertext` already use `serde_bytes`. The two unannotated fields are the exception, so an upstream fix would most likely produce the same bytes.
4. Nothing has shipped, so the compatibility argument for stock does not apply. N10's "migration window" does not exist yet. The stock option's only real benefit is interoperability with stock decoders, and we have none.
5. The costs are real and should stay visible:
   - it is incompatible with stock in both directions (E7), so every node must run the fork;
   - the update path gains only about 10% (383 to 344 bytes per recipient). Updates still send one message per member, so about 380 KB per update at 1,000 members (inferred from the per-recipient size). The design already records O(n) messages per removal (design doc `2026-10-11T03-44-33Z_design_groups_events_delegation`, line 355), and `serde_bytes` does not change that;
   - the version tag and determinism fixes below are required.

Why not the others: (c') gets only half and only for joiners, so it is the fallback if the owner refuses any crate patch. (d) is custom cryptography for at most about 15%. (e) is not a substitute: on stock bytes it still leaves about 78 KB, and on v1 it does nothing.

### Versioning scheme

- **Name:** `datascheme/1` = p2panda-encryption at `3e84cac` with the two annotations. The S12 membership and `from_join` changes are a separate patch set in the same fork. Nothing is `datascheme/0` in production; the stock layout is kept only as a reference.
- **Policy:** the signed group policy carries the scheme id. A manage operation changes it and starts a new epoch (task-05 item 6).
- **Event header:** the scheme id, inside the signed body. The receiver checks it against the epoch's scheme before decoding the payload. A mismatch is rejected (the task-05 downgrade test).
- **Welcome envelope:** `{v: 1, body: bstr}` (+5 bytes in array form). The joiner checks `v` against the scheme in the signed add event or the policy. An unknown `v` gets a named error.
- **Negotiation:** key bundles advertise supported schemes. The admin refuses to add a bundle that does not advertise the group's scheme, so an undecodable welcome is never produced.
- **Determinism (recommended, not measured):** sort the secrets in `SecretBundleState::serialize` by timestamp, then id (`group_secret.rs:174-186`, about 3 lines), and use ordered collections in our DGM. The stock bytes are not a function of the logical state (E7, the K = 1 re-encode).
- **Strict decoding for signed bytes:** `decode_cbor_strict` (`cbor.rs:47`), not `decode_cbor` (lenient, `cbor.rs:33-36`).
- **Tests:** golden vectors for a v1 welcome and a v1 update message; a dependency check that the lock contains one copy of p2panda-encryption (`cargo tree -d`); a test that a stock-encoded message is rejected with the version error, not a CBOR error.
- **Tolerant reader (plan B, not built):** a serde untagged enum over `ByteBuf` and `Vec<u8>` reads both forms (verified on a standalone type: `EitherForm` read both encodings in the serde read check). It would have to be applied to the crate's fields in the fork. It is not needed while nothing has shipped.

### What would change the recommendation

- The owner will not carry any crate patch: choose (c'), which gives 76.9 bytes per epoch for joiners and no change elsewhere. The prototype is about 40 lines; it needs tests and golden vectors before use.
- Upstream changes the layout of these structs, not just the encoding: re-evaluate. The golden vectors would catch it.
- A stock-crate peer must read our welcomes: option (a) with `recent:N`. S12 measured 5.3 KB at N = 32 for stock.
- Product IDs or larger groups change the picture: the welcome's `history` grows with members (S12 did not measure this), and the update cost is O(n).
- The `full` default: under v1 a joiner welcome at 1,000 rotations is 40.5 KB. `recent:32` is 1.7 KB under v1 (S12). That is the owner's decision in task-05, not mine.

---

## Not verified

1. **Processing of the v1 welcome by the joiner, and full-group runs.** Bob and Carol never process messages; the joiner check from S12 is not repeated for v1 or for the transcoded form.
2. **Single-annotation builds past the direct-message level.** The inner-only and outer-only builds were checked only at the direct-message level; whether the inner decode would fail after decryption is inferred.
3. **The tolerant reader inside the crate.** Shown only on a standalone untagged enum, not on the crate's fields.
4. **Determinism of v1 bytes.** The bundle and history order were not checked for v1; the sort fix is recommended, not tested.
5. **Product IDs and group sizes.** The harness uses the test `MemberId` (small integers). Product IDs as `VerifyingKey` are byte strings (E2), but the sizes were not measured with them. Groups above three members were not measured; the update message size is assumed independent of n (inferred).
6. **K above 1,000, other churn patterns, and timing.** Not measured this round.
7. **The crates.io 0.7.1 tarball differs from the pinned checkout** in `dcgka.rs`, `key_bundle/lifetime.rs`, `key_bundle/prekey.rs` and `traits/dgm.rs` (`diff -rq`, this session). I did not check whether any of these changes the wire layout. The pin is `3e84cac`, so the analysis uses it.
8. **Upstream state.** I did not check upstream issues or releases this round (no web fetch). task-11 says no issue covers the two `Vec<u8>` fields.
9. **Other repositories.** Only `/home/user/code` was searched for `p2panda-encryption`. `p2panda-spaces` was not checked for how it depends on the crate.
10. **Lenient and strict decoding for the transcoder.** Not tested with non-canonical input.
11. **Compression beyond the seed-1 dumps.** The derived per-epoch figures come from two points (K = 1 and K = 1000).
12. **Byte equivalence of (c') and the outer-only build.** Shown by equal sizes in three seeds, not by equal bytes (separate processes).
13. **Re-encoding determinism of stock.** The cause (hash order) is inferred; I did not isolate it.
