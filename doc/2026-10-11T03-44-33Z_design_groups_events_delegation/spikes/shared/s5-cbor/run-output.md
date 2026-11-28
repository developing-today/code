## (b) rejection matrix

| input                                                          | bytes                                                                                                                                                                                                                                                                                      | cbor-core STRICT          | dcbor                               | re-encode (LENIENT)          | wrapper                           |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------- | ----------------------------------- | ---------------------------- | --------------------------------- |
| map keys length-first, mixed types: {"a": 1, 1000: 2}          | `a26161011903e802`                                                                                                                                                                                                                                                                         | reject (NonDeterministic) | reject (MisorderedMapKey)           | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `a21903e802616101`                                                                                                                                                                                                                                                                         |                           |                                     |                              |                                   |
| control: text keys, length-first = bytewise: {"b": 1, "aa": 2} | `a261620162616102`                                                                                                                                                                                                                                                                         | accept                    | accept                              | accept                       | accept                            |
| map keys out of order: {"aa": 1, "b": 2}                       | `a262616101616202`                                                                                                                                                                                                                                                                         | reject (NonDeterministic) | reject (MisorderedMapKey)           | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `a261620262616101`                                                                                                                                                                                                                                                                         |                           |                                     |                              |                                   |
| integer 5 in a 1-byte argument                                 | `1805`                                                                                                                                                                                                                                                                                     | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `05`                                                                                                                                                                                                                                                                                       |                           |                                     |                              |                                   |
| integer 255 in a 2-byte argument                               | `1900ff`                                                                                                                                                                                                                                                                                   | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `18ff`                                                                                                                                                                                                                                                                                     |                           |                                     |                              |                                   |
| integer 5 in an 8-byte argument                                | `1b0000000000000005`                                                                                                                                                                                                                                                                       | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `05`                                                                                                                                                                                                                                                                                       |                           |                                     |                              |                                   |
| integer -1 in a 1-byte argument                                | `3800`                                                                                                                                                                                                                                                                                     | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `20`                                                                                                                                                                                                                                                                                       |                           |                                     |                              |                                   |
| text of length 1 with a 1-byte length                          | `780161`                                                                                                                                                                                                                                                                                   | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `6161`                                                                                                                                                                                                                                                                                     |                           |                                     |                              |                                   |
| float 1.0 as f64                                               | `fb3ff0000000000000`                                                                                                                                                                                                                                                                       | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `f93c00`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| float 1.0 as f32                                               | `fa3f800000`                                                                                                                                                                                                                                                                               | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `f93c00`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| float 1.0 as f16                                               | `f93c00`                                                                                                                                                                                                                                                                                   | accept                    | reject (NonCanonicalNumeric)        | accept                       | reject (Float)                    |
| float 1.5 as f32 (fits f16)                                    | `fa3fc00000`                                                                                                                                                                                                                                                                               | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `f93e00`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| float 1.5 as f16                                               | `f93e00`                                                                                                                                                                                                                                                                                   | accept                    | accept                              | accept                       | reject (Float)                    |
| float 0.1 as f64                                               | `fb3fb999999999999a`                                                                                                                                                                                                                                                                       | accept                    | accept                              | accept                       | reject (Float)                    |
| NaN as f16 0x7e00                                              | `f97e00`                                                                                                                                                                                                                                                                                   | accept                    | accept                              | accept                       | reject (Float)                    |
| NaN as f16, payload 1                                          | `f97e01`                                                                                                                                                                                                                                                                                   | accept                    | reject (NonCanonicalNumeric)        | accept                       | reject (Float)                    |
| -0.0 as f16                                                    | `f98000`                                                                                                                                                                                                                                                                                   | accept                    | reject (NonCanonicalNumeric)        | accept                       | reject (Float)                    |
| text e + U+0301 (not NFC)                                      | `6365cc81`                                                                                                                                                                                                                                                                                 | accept                    | reject (NonCanonicalString)         | accept                       | reject (NotNfc)                   |
| text U+00E9 (NFC, control)                                     | `62c3a9`                                                                                                                                                                                                                                                                                   | accept                    | accept                              | accept                       | accept                            |
| duplicate key {"a": 1, "a": 2}                                 | `a2616101616102`                                                                                                                                                                                                                                                                           | reject (NonDeterministic) | reject (DuplicateMapKey)            | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `a1616102`                                                                                                                                                                                                                                                                                 |                           |                                     |                              |                                   |
| indefinite byte string (chunks 01, 02)                         | `5f41014102ff`                                                                                                                                                                                                                                                                             | reject (NonDeterministic) | reject (UnsupportedHeaderValue(31)) | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `420102`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| indefinite text string (chunks a, b)                           | `7f61616162ff`                                                                                                                                                                                                                                                                             | reject (NonDeterministic) | reject (UnsupportedHeaderValue(31)) | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `626162`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| indefinite array [1, 2]                                        | `9f0102ff`                                                                                                                                                                                                                                                                                 | reject (NonDeterministic) | reject (UnsupportedHeaderValue(31)) | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `820102`                                                                                                                                                                                                                                                                                   |                           |                                     |                              |                                   |
| indefinite map {"a": 1}                                        | `bf616101ff`                                                                                                                                                                                                                                                                               | reject (NonDeterministic) | reject (UnsupportedHeaderValue(31)) | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `a1616101`                                                                                                                                                                                                                                                                                 |                           |                                     |                              |                                   |
| simple value 23 (undefined)                                    | `f7`                                                                                                                                                                                                                                                                                       | accept                    | reject (InvalidSimpleValue)         | accept                       | reject (SimpleValue(23))          |
| simple value 16 (unassigned)                                   | `f0`                                                                                                                                                                                                                                                                                       | accept                    | reject (InvalidSimpleValue)         | accept                       | reject (SimpleValue(16))          |
| simple value false (control)                                   | `f4`                                                                                                                                                                                                                                                                                       | accept                    | accept                              | accept                       | accept                            |
| tag 1000 (unknown meaning) around 1                            | `d903e801`                                                                                                                                                                                                                                                                                 | accept                    | accept                              | accept                       | reject (Tag(1000))                |
| bignum tag 2, 4-byte payload with a leading zero               | `c24400000001`                                                                                                                                                                                                                                                                             | reject (NonDeterministic) | accept                              | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `01`                                                                                                                                                                                                                                                                                       |                           |                                     |                              |                                   |
| p2panda header, seq 0 as 18 00, validly signed                 | `8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8584057a7a8e98a3a523ee69a55b416e26a2938860d32afe1a00195d467dd4bdd09c1bab7046c936c791e7a59fbcabb4dbc9c000277a85870677405c6d09f00f9b00d182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d1800` | reject (NonDeterministic) | reject (NonCanonicalNumeric)        | reject (re-encoding differs) | reject (Decode(NonDeterministic)) |
| normalized to                                                  | `8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8584057a7a8e98a3a523ee69a55b416e26a2938860d32afe1a00195d467dd4bdd09c1bab7046c936c791e7a59fbcabb4dbc9c000277a85870677405c6d09f00f9b00d182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00`   |                           |                                     |                              |                                   |

## (c) decode-then-re-encode, 10000 values x 16 mutants, seed 20261011

Stats {
values: 10000,
baseline_strict_ok: 9991,
baseline_strict_refused: 9,
baseline_refused_bignum: 9,
baseline_refused_examples: [
"a329c341514a1e8e4ff56502512335c23b00000041f05378ed69e697a5e69cace8aa9e19053d",
"c2488ab9b7e11b2f1129",
"d9d9f7c24165",
],
baseline_signable: 6949,
baseline_dcbor_ok: 7976,
mutants: 160000,
strict_accepted: 30365,
reencode_mismatch: 0,
reencode_identical: 30365,
strict_vs_reencode_disagree: 0,
strict_rejected_by: {
"InvalidFormat": 25690,
"InvalidUtf8": 11290,
"LengthTooLarge": 514,
"Malformed": 6211,
"NonDeterministic": 5416,
"UnexpectedEof": 80514,
},
rule_refused: {
"float": 2219,
"simple value other than false/true/null": 2374,
"tag": 3137,
"text not NFC": 1632,
},
wrapper_accepted: 21003,
both_accept: 24577,
both_reject: 129604,
dcbor_refuses: {
"InvalidSimpleValue": 2625,
"NonCanonicalNumeric": 964,
"NonCanonicalString": 2199,
},
dcbor_only: 31,
dcbor_only_bignum: 31,
dcbor_only_examples: [
"a329c341514a1e8e4ff57502512335c23b00000041f05378e069e697a5e69cace8aa9e19053d",
"a329c341514a1e8e4ff56502512335c23b00000041f05378ed69e6b1a5e69cace8aa9e19253d",
"a329c341514a1e8e4ff56502785135c23b00000041f05378ed69e697a5e69cace8aa9e19053d",
"a329c341514a1e8e4ff56502512335c23b00000041f05378ed69e69fa5e69cace8aa9e19053d",
"c340",
],
dcbor_reencode_differs: 0,
wrapper_accepted_dcbor_refused: 0,
panics: 0,
}

## (d) golden vectors

- plain body map: cbor-core `a3646b696e646c67726f75702e706f6c6963796567726f7570626731656c6576656c03` (35 bytes); dcbor agrees: true
- body with byte strings: cbor-core `a364626c6f624300ff10646861736858203ae7d805f6789a6402acb70ad4096a85a56bf6804eaf25c0493ac697548d30b565656d70747940` (56 bytes); dcbor agrees: true
- array of 32-byte hashes: cbor-core `83582017762fddd969a453925d65717ac3eea21320b66b54342fde15128d6caf21215f582010e5cf3d3c8a4f9f3468c8cc58eea84892a22fdadbc1acb22410190044c1d5535820ea7aa1fc9efdbe106dbb70369a75e9671fa29d52bd55536711bf197477b8f021` (103 bytes); dcbor agrees: true
- nested map: cbor-core `a36161f6617a646c617374656f75746572a26374616741ab65696e6e6572830121a16464656570f5` (40 bytes); dcbor agrees: true
- p2panda header (seq 0, body, fixed key), 139 bytes:
  header: `8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b85840be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00`
  signed: `8501582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00`
  public key: `03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8`
  operation id (BLAKE3 of header): `90acd5f7c498ccb93dc33ce41f49d166140bef38c1f85cb95940f311b6e7f212`
  signature: `be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f`
  dcbor header agrees: true
  dcbor signed part agrees: true
  p2panda decodes it: true
  verify_strict over signed bytes: Ok(())

## (e) signature forgeries

- golden signature
  admitted as a key: true
  verify_strict: Ok(())
  non-strict verify: Ok(())
  p2panda Header::decode: Ok(())
- S + L (non-canonical)
  admitted as a key: true
  verify_strict: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  non-strict verify: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  p2panda Header::decode: Err("InvalidSignature")
- S = L
  admitted as a key: true
  verify_strict: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  non-strict verify: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  p2panda Header::decode: Err("InvalidSignature")
- S = L + 1
  admitted as a key: true
  verify_strict: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  non-strict verify: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  p2panda Header::decode: Err("InvalidSignature")
- S = 2^256 - 1
  admitted as a key: true
  verify_strict: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  non-strict verify: Err("signature::Error { source: Some(Cannot use scalar with high-bit set) }")
  p2panda Header::decode: Err("InvalidSignature")
- key = identity (small order)
  admitted as a key: false
  verify_strict: Err("signature::Error { source: Some(Verification equation was not satisfied) }")
  non-strict verify: Ok(())
  p2panda Header::decode: Err("InvalidSignature")
- key = order-2 point (y = -1)
  admitted as a key: false
  verify_strict: Err("signature::Error { source: Some(Verification equation was not satisfied) }")
  non-strict verify: Err("signature::Error { source: Some(Verification equation was not satisfied) }")
  p2panda Header::decode: Err("InvalidSignature")
