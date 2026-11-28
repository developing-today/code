# S5: canonical CBOR for signed bytes

**Question.** Which CBOR crate and canonical rule sign `id` events (design §4.5, Phase 1 S5)? Exit: crate chosen, golden vectors, rejection matrix, interop recorded.

**Verdict.** cbor-core 0.10.1. Canonicality is decode-then-re-encode, plus four value rules. All four exit criteria pass. Raw output: `run-output.md`.

## Setup

- p2panda-core at rev 3e84cac depends on `cbor-core = { version = "0.10.1", features = ["serde"] }`. This spike resolves one cbor-core, 0.10.1, with the same features.
- Also: dcbor 0.25.2 (cross-check only), ed25519-dalek 2.2.0 (default features), blake3 1.8.7, unicode-normalization 0.1.25, p2panda-core 0.7.1 at 3e84cac. The pinned rev compiles.
- rustup 1.97.0. Commands: `cargo build -p s5-cbor`, `cargo test -p s5-cbor` (9 pass), `cargo run -p s5-cbor > run-output.md`. Set `RUSTDOC` to rustup's 1.97.0 rustdoc, since PATH has 1.97.1.

## (a) Where p2panda-core encodes headers

- Encode: `operation/header.rs:228-270` builds `cbor_core::Value::array` and calls `.encode()`.
- Sign: `operation/builder.rs:104-115` signs the encoding without the signature. `:117-128` encodes the full header and takes BLAKE3 (the operation ID).
- Decode: `operation/any.rs:191-200` uses `Strictness::STRICT`. `:341-345` re-encodes the array without the signature, then verifies.
- Verify: `identity.rs:196-197` calls `verify_strict`.
- Hazard: `cbor.rs:33-35` (`decode_cbor`, the serde path) is LENIENT. Only `decode_cbor_strict` is safe for signed bytes.

## (b) Rejection matrix

Columns: S = cbor-core STRICT. D = dcbor. R = decode leniently, re-encode, compare (`re` means the bytes changed). W = wrapper. `ok` accepts. Codes: ND NonDeterministic, NCN NonCanonicalNumeric, NCS NonCanonicalString, MMK MisorderedMapKey, DMK DuplicateMapKey, UHV UnsupportedHeaderValue(31), ISV InvalidSimpleValue. W adds value-rule codes: Float, NFC, SV (simple value other than false, true, null), Tag.

```
#   bytes               S   D    R   W     input
1   a26161011903e802    ND  MMK  re  ND    keys length-first, mixed types {"a":1, 1000:2}
2   a261620162616102    ok  ok   ok  ok    control: text keys, length-first = bytewise {"b":1,"aa":2}
3   a262616101616202    ND  MMK  re  ND    keys out of order {"aa":1,"b":2}
4   1805                ND  NCN  re  ND    5 in a 1-byte argument
5   1900ff              ND  NCN  re  ND    255 in a 2-byte argument
6   1b0000000000000005  ND  NCN  re  ND    5 in an 8-byte argument
7   3800                ND  NCN  re  ND    -1 in a 1-byte argument
8   780161              ND  NCN  re  ND    text length 1 in a 1-byte length
9   fb3ff0000000000000  ND  NCN  re  ND    float 1.0, f64
10  fa3f800000          ND  NCN  re  ND    float 1.0, f32
11  f93c00              ok  NCN  ok  Float float 1.0, f16
12  fa3fc00000          ND  NCN  re  ND    float 1.5, f32
13  f93e00              ok  ok   ok  Float float 1.5, f16
14  fb3fb999999999999a  ok  ok   ok  Float float 0.1, f64
15  f97e00              ok  ok   ok  Float NaN, f16 0x7e00
16  f97e01              ok  NCN  ok  Float NaN, f16 payload 1
17  f98000              ok  NCN  ok  Float -0.0, f16
18  6365cc81            ok  NCS  ok  NFC   text e + U+0301, not NFC
19  62c3a9              ok  ok   ok  ok    text U+00E9, NFC
20  a2616101616102      ND  DMK  re  ND    duplicate key {"a":1,"a":2}
21  5f41014102ff        ND  UHV  re  ND    indefinite byte string
22  7f61616162ff        ND  UHV  re  ND    indefinite text string
23  9f0102ff            ND  UHV  re  ND    indefinite array
24  bf616101ff          ND  UHV  re  ND    indefinite map
25  f7                  ok  ISV  ok  SV    undefined (simple 23)
26  f0                  ok  ISV  ok  SV    simple value 16
27  f4                  ok  ok   ok  ok    false (control)
28  d903e801            ok  ok   ok  Tag   tag 1000 around 1
29  c24400000001        ND  ok   re  ND    bignum tag 2, 4-byte payload, leading zero
30  (see below)         ND  NCN  re  ND    p2panda header, seq as 18 00, validly signed
```

Row 30 (140 B): `8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8584057a7a8e98a3a523ee69a55b416e26a2938860d32afe1a00195d467dd4bdd09c1bab7046c936c791e7a59fbcabb4dbc9c000277a85870677405c6d09f00f9b00d182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d1800`

Findings:

- R accepts four classes: non-NFC text, shortest-form floats, undefined and unassigned simple values, and tags. The value rules cover those.
- cbor-core sorts keys bytewise, not length-first as its doc comment says. Length-first and bytewise agree within one major type (tested). They differ across types (row 1).
- cbor-core accepts only the shortest float form: f16 1.0 passes, f32 and f64 1.0 fail. dCBOR also refuses integral floats and non-default NaN payloads.
- dcbor accepts the non-canonical bignum tags that cbor-core refuses (row 29).

## (c) Decode-then-re-encode

**Argument.** Each STRICT check refuses something the encoder never emits: non-shortest heads, indefinite lengths, unsorted or duplicate keys, non-shortest floats, non-canonical bignums. Everything STRICT accepts is re-emitted unchanged. Heads are shortest. Strings are copied verbatim (UTF-8 is validated). Maps come out in `Value` order, which compares head, then payload, so it is bytewise over the encodings. Floats keep their stored bits. Trailing bytes are refused. So for any accepted input, `encode(decode(b)) == b`. STRICT's accept set therefore equals "decodes leniently and re-encodes identically", and the test checks that equality.

**Test.** Seed 20261011. 10,000 random canonical values (depth ≤ 3; floats, tags, undefined, and non-NFC text included). Each gets 16 mutants with one to three random edits (bit flip, byte replace, insert, delete, truncate). That is 160,000 mutants.

- Baseline: 9,991 of 10,000 canonical encodings pass STRICT with identical re-encoding. The other 9 are NonDeterministic refusals of values holding bignum tags 2 or 3. The printed examples are tags over payloads of eight bytes or fewer, which cbor-core's encoder emits and its decoder refuses.
- Accepted mutants: 30,365. Re-encode mismatches: **0**.
- STRICT versus decode-then-re-encode disagreements: **0** of 160,000.
- Value rules on the 30,365 accepted: refused 9,362 (float 2,219; other simple value 2,374; tag 3,137; non-NFC 1,632). The wrapper accepts 21,003.
- dcbor: both accept 24,577; both refuse 129,604; cbor-core only 5,788 (ISV 2,625, NCN 964, NCS 2,199); dcbor only 31, all bignum tags. dcbor re-encode differences: 0. Wrapper-accepted but dcbor-refused: 0. Panics: 0.

## (d) Golden vectors

Each is one logical value, encoded by cbor-core and by dcbor. Bytes match on all five. dcbor also decodes the cbor-core bytes and re-encodes them identically.

1. Plain body map, 35 B: `{"kind": "group.policy", "group": "g1", "level": 3}`
   `a3646b696e646c67726f75702e706f6c6963796567726f7570626731656c6576656c03`
2. Byte strings, 56 B: `{"blob": h'00ff10', "empty": h'', "hash": blake3("x")}`
   `a364626c6f624300ff10646861736858203ae7d805f6789a6402acb70ad4096a85a56bf6804eaf25c0493ac697548d30b565656d70747940`
3. Hashes, 103 B: `[blake3("a"), blake3("b"), blake3("c")]`
   `83582017762fddd969a453925d65717ac3eea21320b66b54342fde15128d6caf21215f582010e5cf3d3c8a4f9f3468c8cc58eea84892a22fdadbc1acb22410190044c1d5535820ea7aa1fc9efdbe106dbb70369a75e9671fa29d52bd55536711bf197477b8f021`
4. Nested, 40 B: `{"a": null, "outer": {"inner": [1, -2, {"deep": true}], "tag": h'ab'}, "z": "last"}`
   `a36161f6617a646c617374656f75746572a26374616741ab65696e6e6572830121a16464656570f5`
5. p2panda header, 139 B: seq 0, body = vector 1, key = bytes 00..1f (test seed, not a real key)
   - header: `8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b85840be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00`
   - signed part, 73 B: `8501582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00`
   - operation ID (BLAKE3 of the full header): `90acd5f7c498ccb93dc33ce41f49d166140bef38c1f85cb95940f311b6e7f212`
   - Ed25519 signature: `be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f`
   - dcbor's header and signed part are byte-identical, so the signature verifies over either.

## (e) Signatures

```
case                          key  verify_strict     plain  p2panda
golden                        ok   ok               ok     ok
S + L                         ok   no               no     InvalidSignature
S = L                         ok   no               no     InvalidSignature
S = L + 1                     ok   no               no     InvalidSignature
S = 2^256 - 1                 ok   no               no     InvalidSignature
identity key, R = id, S = 0   no   no (small order)  ok     InvalidSignature
order-2 key (y = -1), same    no   no               no     InvalidSignature
```

- In ed25519-dalek 2.2.0's default build, `check_scalar` refuses S ≥ L while parsing. Plain `verify` refuses those too, so this protection does not come from `verify_strict`. The error text "Cannot use scalar with high-bit set" is misleading.
- The S ≥ L protection needs `legacy_compatibility` off. `cargo tree -e features` shows it off across this graph.

## Exit criteria

| Criterion | Result |
|---|---|
| Crate chosen | **Pass.** cbor-core 0.10.1 |
| Golden vectors recorded | **Pass.** Five vectors; dcbor agrees on all |
| Rejection matrix complete | **Pass.** 30 inputs covering every listed class, four checks each, pinned in `tests/spike.rs` |
| Interop recorded | **Pass.** dcbor agrees on all five golden encodings and refuses none of the 21,003 wrapper-accepted mutants. |

## Deviations

- Research §1.2 said cbor-core accepts float 1.0. Only the f16 form passes.
- Research §1.2 said cbor-core rejects length-first maps. It does so because it sorts bytewise; its doc comment says length-first, which is wrong.
- Research §1.5 said S < L is enforced in `from_bytes`. That is `InternalSignature::from_bytes`, which both `verify` and `verify_strict` reach. `Signature::from_bytes` does not check S.
- No Cargo.toml edits. The first build added 27 transitive iroh packages to the shared `Cargo.lock` (573 entries now). The pins I checked held. Flagging for the lead.
- `cargo clippy` could not run: "multiple workspace roots" from `s3-history/patched/p2panda-encryption`, outside this member.
- minicbor and ciborium were not re-tested.

## Blockers

None.

## Recommendation

- **Crate:** `cbor-core = "=0.10.1"` with `features = ["serde"]`, as p2panda-core uses. dcbor only as a dev-dependency, for cross-checks.
- **Wrapper:** `src/canonical.rs`, 93 lines.
  - `decode(&[u8]) -> Result<Value<'static>, Error>`: STRICT decode, re-encode must match, then value rules (NFC text including keys; no floats, undefined, other simple values, or tags).
  - `encode(&Value) -> Vec<u8>`.
  - `admit_key(&[u8; 32]) -> Option<VerifyingKey>`: refuses non-points and small-order keys.
  - `verify(&VerifyingKey, &[u8], &[u8; 64]) -> Result<(), SignatureError>`: `verify_strict`.
  - `Error`: `Decode`, `NotCanonical`, `NotNfc`, `Float`, `SimpleValue(u8)`, `Tag(u64)`.
- Route all signed bytes through `canonical::decode`. Never use p2panda's `cbor::decode_cbor`, which is LENIENT.
- Keep ed25519-dalek's default features. In CI, check that `cargo tree -e features` shows no `legacy_compatibility`.
- Do not construct bignum tags: cbor-core's encoder emits ones its decoder refuses.
- Design §4.5's open question is answered: p2panda signs with cbor-core, so using it gives one canonical form.
