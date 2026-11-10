# S9 results: Seitan V2 invites, test vectors, and the ticket question

## Summary

- The Seitan V2 derivations reproduce Keybase's V2 test vectors: the stretched key, the invite ID, and the Ed25519 public key (the last one through the vector's boxed pkey). The shared steps also reproduce Keybase's V1 vector.
- Not verified against Keybase: the accept-message bytes (no vector covers a signature), and the admin-side handler and V3 (I could not fetch them). The admission checks are this crate's own and are tested only against our own rules.
- 38 of 38 tests pass, with zero warnings. Clippy with `-D warnings` is clean. Ten deliberate mutations each fail the tests I expected.
- A Seitan token can be appended to an `EndpointTicket` string, and iroh-tickets 1.0.0 then accepts it and silently drops the bytes. Do not do this. The ticket has no expiry, no signature, and no single-use state. Expiry and single use belong to the admin's invite event.

## Question

1. Implement Seitan V2 as the design specifies: token generation, the derived keys, the joiner's proof, the admin's verification, the freshness and expiry check, single use, and the admin-only acceptance path. Verify against Keybase.
2. Can a Seitan token ride inside an iroh `EndpointTicket`? What does the ticket leak? How do expiry and single use work?

## Method

### Sources fetched (raw GitHub, `webfetch`, `master` branch, 2026-10-11)

| URL under `raw.githubusercontent.com/keybase/client/master/` | Status | Used for |
|---|---|---|
| `go/teams/seitan.go` | 200 | V1 code, plus the steps V1 and V2 share: alphabet, token length, `generateIKey`, `generateSIKey`, `generateTeamInviteIDRaw` |
| `go/teams/seitan_v2.go` | 200 | V2 code: `GenerateIKeyV2`, the V2 invite-ID payload, `generateKeyPair`, `GenerateSeitanSignatureMessage`, `GenerateSignature` |
| `go/teams/seitan_v2_test.go` | 200 | V2 known sample (`TestSeitanV2KnownSamples`), and the bad-signature tests |
| `go/teams/seitan_test.go` | 200 | V1 known sample (`TestSeitanKnownSamples`) |
| `go/msgpack/msgpack.go` | 200 | `Encode` is the non-canonical encoder (struct fields in declaration order) |
| `go/teams/seitan_v3.go`, `handle_seitan.go`, `seitan_handler.go`, `team_seitan.go`, `invite_link.go` | 404 (all five) | Looked for the admin handler (`HandleTeamSeitan`, `postSeitanV2`) and V3. Not found. |

Not fetched: the go-codec encoder (integer widths), `kbcrypto` `Sign`, the `keybase1` type definitions (`Time`, `Seqno`), and `libkb.InviteIDTag`. `webfetch` returns text without line numbers, so I cite Go identifiers instead of line numbers.

### Commands

```
export CARGO="$(rustup which --toolchain 1.97.0 cargo)" CARGO_TARGET_DIR=/home/user/work/spikes/target
cd seitan-v2 && "$CARGO" test --no-fail-fast          # logs/test-output.txt
cd seitan-v2 && "$CARGO" run -q --release --example dump_vectors   # logs/dump-vectors.txt
cd seitan-v2 && cargo clippy --all-targets -- -D warnings          # logs/clippy.txt (cargo from the 1.97.0 bin on PATH)
cd ticket-probe && "$CARGO" run -q                                  # logs/ticket-probe.txt
python3 -I scripts/mutation.py                                      # logs/mutation-check.txt
```

The crate's `Cargo.lock` is copied from `/home/user/work/spikes/Cargo.lock` and then updated for the new dependencies. Cargo fetched the new crates from crates.io (the spike lock had none of scrypt, rmp-serde, proptest, or crypto_secretbox locally).

## Results

### 1. Derivation steps against Keybase

Status: **V** = matched a Keybase test vector. **R** = matched the Go code by reading it, with no vector. **N** = not verified.

| # | Step | Keybase identifier | Our code | Check | Status |
|---|---|---|---|---|---|
| 1 | Alphabet `abcdefghjkmnpqrsuvwxyz23456789` (30 symbols) | `KBase30EncodeStd` | `token::ALPHABET` | Coverage test over 500 tokens. The vector's symbols are all in the alphabet. | R + V |
| 2 | Token length 18, `+` at index 6 | `SeitanEncodedIKeyLength`, `seitanEncodedIKeyV2PlusOffset` | `TOKEN_LEN`, `PLUS_INDEX` | Shape test (1000 tokens). Vector has `+` at index 6. | R + V |
| 3 | 17 random symbols, rejection sampling on the low 5 bits | `generateIKey` | `Token::generate` | Shape and coverage tests. Random, so no vector. | R |
| 4 | Lowercase the token before stretching | `SeitanIKeyV2.String()` | `Token::parse` | Unit test: `4UYWZA+B3CGA7RD6YC` parses to the vector token. Proptest: any case gives the same ID. The vector itself is already lowercase. | R |
| 5 | scrypt, N = 2^10, r = 8, p = 1, 32 bytes, empty salt | `generateSIKey`, `SeitanScryptCost/R/P/Keylen` | `derive::stretch` | V2 vector `Il9ZFgI1…` matches. V1 vector `Yqbj8N…` also matches. | V (V2 and V1) |
| 6 | Invite-ID payload `{stage: "invite_id", version: 2}` | `NewSeitanInviteIDPayload` | `derive::stage_payload` | The invite ID depends on these bytes, and the vector matches. | V |
| 7 | Invite ID = first 15 bytes of HMAC-SHA512(sikey, payload) + tag | `generateTeamInviteIDRaw` | `derive::invite_id` | Vector `6303ec43…6227` matches. Its last byte is `0x27`, which confirms the tag. The V1 vector `24189cc0…9c27` also ends in `27`. | V (tag confirmed by vectors, not by reading `libkb.InviteIDTag`) |
| 8 | Invite ID hex form, 32 characters | `generateTeamInviteID` (hex), `SCTeamInviteID` | `InviteId::to_hex` | Vector string matches. | V |
| 9 | Ed25519 seed payload `{stage: "eddsa", version: 2}` | `generateKeyPair` / `PrivateKeySeedPayload` | `derive::stage_payload("eddsa")` | Confirmed indirectly: the derived public key is in the decrypted pkey. | V |
| 10 | Seed = first 32 bytes of HMAC-SHA512(sikey, payload), Ed25519 from seed | `generateKeyPair` (`seed[0:32]`, `ed25519.GenerateKey`) | `derive::signing_key` | Vector pkey decrypts, and its plaintext contains our derived public key `8d3b1ccf…b528`. | V |
| 11 | pkey box: XSalsa20-Poly1305, 24-byte nonce, team key | `packAndEncryptKeyWithSecretKey`, `decryptKeyAndLabelWithSecretKey` | test only, `crypto_secretbox` | Vector opens with its team key, not with zeros. Version 2 and generation 1 match. | V (decrypt only; we never create pkeys) |
| 12 | Accept message: `{stage: "accept", uid, eldest_seqno, ctime, invite_id, version: 2}`, in that order | `SigPayload` in `GenerateSeitanSignatureMessage` | `accept::message` | Field order and types match the struct. Golden test pins our bytes. | R (order and types). Bytes are **N**. |
| 13 | Signature is Ed25519 over those bytes | `kbcrypto` `NaclSigningKeyPrivate.Sign` (not read) | `ed25519-dalek` `sign` | Round trip only. | **N** |
| 14 | Signature binds uid, eldest_seqno, ctime, and invite ID | `TestSeitanBadSignatures` | `accept::verify` | Changing any of the four fields fails. | R (test expectations) |
| 15 | Seitan V1 and V2 are single use | comment in `TestSeitanMultipleRequestForOneInvite` | `Invite::create(…, max_uses = 1)` | Single-use test. | R (as the test states it) |
| 16 | Admin path, V2 freshness check, V3 freshness check | handler not fetched | `admit.rs` | Our tests only. | **N** |

Step 5 has a second check: the V1 vector (`raw2ewqp249dyod4`) gives the same scrypt output and the same tag, so the scrypt parameters are confirmed twice.

Two V1 and V2 differences in the source matter for anyone implementing from `seitan.go` alone. `seitan.go` still has the V1 invite-ID payload, `{stage: "invite_id"}` with no version. It also has `GenerateIKey`, which uses the V1 `+` position (index 5). The V2 values are in `seitan_v2.go`.

### 2. Test suite

| Test binary | Tests | Result |
|---|---|---|
| unit (`derive`, V1 vector, payload bytes, hex) | 3 | pass |
| `tests/accept.rs` (golden bytes, parse rules, verify, forgery, proptests) | 9 | pass |
| `tests/admit.rs` (admin path, boundaries, proptest on the freshness window) | 15 | pass |
| `tests/token.rs` (shape, coverage, parse errors, redacted Debug, proptest) | 7 | pass |
| `tests/vectors.rs` (Keybase V2 vectors) | 4 | pass |
| **Total** | **38** | **38 pass, 0 fail, 0 warnings** |

Three separate runs of the final code gave 38/38. `cargo fmt --check` passes. `cargo clippy --all-targets -- -D warnings` exits 0 with zero warnings (`logs/clippy.txt`). A ticket-probe run has no warnings either.

### 3. Mutation check

`scripts/mutation.py` applies one change to a scratch copy and records which tests fail. Baseline on the same copy: 38 pass.

| # | Mutation | Failing tests | Caught by |
|---|---|---|---|
| M1 | Ed25519 seed payload label `eddsa` → `eddsb` | 1 | the pkey vector only |
| M2 | Invite-ID tag `0x27` → `0x28` | 3 | vectors, and the V1 vector |
| M3 | scrypt `r` 8 → 1 | 5 | vectors |
| M4 | Stale check removed | 3 | freshness proptest, two stale tests |
| M5 | Use limit removed | 2 | single-use and multi-use tests |
| M6 | Expiry check removed | 2 | expiry tests |
| M7 | Signature not checked | 8 | forgery, tamper, and replay tests |
| M8 | `ctime` and `eldest_seqno` swapped in the message | 1 | golden bytes only (no Keybase vector covers this) |
| M9 | Use not recorded | 2 | single-use and multi-use tests |
| M10 | Revocation check removed | 1 | revocation test |

The script reports a mutation as `NOT APPLIED` when its pattern does not match exactly once. In the final run all ten were applied. Earlier runs skipped M1 (its pattern matched three times) and M7 and M9 (`cargo fmt` had changed the text). I fixed those patterns and reran. The earlier runs also had the artifact collision described in Surprising 4, so only the final run counts. M1 is caught by one test, the pkey vector, which is therefore the only Keybase check on the Ed25519 derivation. M8 is caught only by our golden-bytes test.

### 4. Admin path

`Invite::admit` checks, in this order, with the signature last because it is the costly check. A rejected attempt records nothing (test `a_rejected_attempt_does_not_use_up_the_invite`).

| Check | Reject | Test |
|---|---|---|
| Invite ID matches the stored invite | `UnknownInvite` | `an_accept_for_a_different_token_names_no_known_invite` |
| Not revoked | `Revoked` | `a_revoked_invite_admits_no_one` |
| `now ≤ expires` and `ctime ≤ expires` | `Expired` | `an_expired_invite_rejects_at_processing_time`, `an_accept_signed_after_expiry…` |
| Uses left (`max_uses = 1` for Seitan) | `Used` | `a_valid_accept_admits_once…` |
| Joiner not already admitted | `AlreadyAdmitted` | `multi_use_invites_still_block_the_same_joiner_twice` |
| `ctime ≥ created − 5 min` | `Stale` | `an_accept_from_before_the_invite_existed_is_stale`, `an_accept_for_an_earlier_invite…` |
| `ctime ≤ now + 5 min` | `FromFuture` | `an_accept_from_the_future…` |
| Signature verifies under the invite's public key (strict) | `BadSignature` | `a_forged_message…`, `changing_any_signed_field…`, `a_flipped_signature_bit…` |

The proptest `a_ctime_is_admitted_exactly_inside_the_window` checks 256 random cases against the window rule.

**Decision for the owner: the freshness window is bound to the invite's life, not to the wall clock.** The design says a home server may hold an accept for later processing. A window of "now minus 5 minutes" would reject any accept held for longer than that. I used `created − 5 min ≤ ctime ≤ min(now + 5 min, expires)`. The 5 minutes is the envelope skew window from the design. A stale accept is one signed before the invite existed, which is what the re-issue case needs. Using a wall-clock window instead would require either re-signing held accepts or a window as long as the hold period.

Other checks I wrote and did not take from Keybase: the signature covers the exact bytes received (no re-encoding before verifying), `verify_strict` is used, and the decoder accepts any integer encoding of the same fields (test `signed_integer_families_decode_to_the_same_fields`).

### 5. Ticket probe (`ticket-probe/`, iroh-tickets 1.0.0, iroh-base 1.3.0)

Nothing was dialled. The probe encodes and decodes tickets locally.

| Step | Result |
|---|---|
| Ticket with one relay URL and one IPv4 address | 124-character string. Postcard payload 72 bytes. |
| Payload layout | variant (1) + EndpointId (32) + address count (1) + relay entry (30) + IP entry (8). Every byte is accounted for: no timestamp, signature, or nonce. |
| Round trip | equal |
| Token appended after the payload (18 bytes) | **parses**, and the address is unchanged. No API returns the extra bytes. |
| EndpointId swapped inside the string | **parses**, with a different ID. The ticket is unsigned. |
| Parsed twice | same address. Parsing consumes nothing. |

Source confirms the probe: `EndpointTicket` is `TicketWireFormat::Variant1 { id, addrs }`, decoded with `postcard::from_bytes`. postcard 1.1.3's `from_bytes` does not check for trailing bytes (`de/mod.rs:12-19`).

### 6. Offline guess cost

Release build, one thread, AMD Ryzen 7 7840U, 2,000 guesses: **1.40 ms per guess** (scrypt plus the invite-ID HMAC). An attacker holding the public invite ID can run this check. At that rate, 2^20 guesses take about 24 core-minutes, so a human-chosen token with about 20 bits of entropy falls to a laptop in under an hour. The 83-bit random token is far beyond that.

## Verdicts

**Seitan V2 derivations.** Verified against Keybase's V2 vectors: the stretched key, the invite ID, and the Ed25519 public key. The accept-message bytes and the admin path are not verified against Keybase. The admission rules are implemented and tested against our own spec, and the freshness window is a decision for the owner (see section 4).

**Can a Seitan token ride inside an `EndpointTicket`?** Not as a field: the format has no place for one. It can be appended after the postcard payload, and iroh-tickets 1.0.0 then accepts the string and silently discards the bytes. That is an accident of postcard, not a supported channel. Keep the token out of the ticket. The link should carry the ticket (a route) and the token (the authority) separately, with the token in the URL fragment for a bearer link.

**What does the ticket leak?** The EndpointId (the admin node's long-lived public key), the home relay URL (its host, which shows where the node is homed), and any direct IP:port included. The payload has no other fields (measured). Anyone holding the ticket can also dial that endpoint. That follows from how iroh dials and was not tested here, since the probe dials nothing.

**Expiry.** The ticket has none. It works for as long as its addresses do. Expiry belongs to the invite event, which the admin checks when processing an accept: `now > expires` is `Expired`, and so is an accept whose `ctime` is after `expires`.

**Single use.** The ticket has none, since parsing and dialling consume nothing. Single use is the admin's state: the first admitted accept records a use, and later accepts are `Used`. The ticket is unsigned, so anyone who edits it can point a joiner at a different node. That node can withhold the accept or forward it. It cannot change the admitted uid, which is signed, and it cannot make a proof without the token.

## Not verified or not tested

- **Admin handler and V3.** `HandleTeamSeitan`, `postSeitanV2`, and the V3 code are not in any file I fetched (five guessed paths returned 404). So the design's claim that V2 has no `ctime` check, and that V3 has one, is unchecked.
- **Accept-message bytes.** No vector covers an accept or a signature. My integer widths follow rmp-serde's minimal encoding. go-codec may encode `ctime` or `eldest_seqno` differently. The verifier accepts either, since it verifies the received bytes, but I did not check Keybase's choice.
- **Signing primitive.** The key pair comes from Go's standard `ed25519.GenerateKey` (RFC 8032). The signing call is `kbcrypto`'s `Sign`, which I did not read. Its output is not checked against Keybase.
- **`ctime` unit.** `keybase1.Time` is not read. I use milliseconds, to match the design's `*_MS` naming.
- **Links.** The `id:join?…` format, the bearer fragment, and "typed link fails without the key" are not built. The brief's S9 list does not include them. Structurally, an accept cannot be made without the token.
- **Not built:** the chain event format, persistence, multiple admins racing on one invite (the client-side sequential completion path does not establish atomic exclusion of concurrent handlers), the approval flow, the metadata documentation, and a Keybase interop test (no Keybase server or client here).
- **Secrets.** `SiKey` and the Ed25519 seed are not zeroised on drop.
- **Ticket network behaviour.** No dial, no relay, no real endpoint. The probe is decode-only.
- **Performance.** Only the single-guess timing above. No benchmarks of admission or of `Token::generate`.

### 2026-11-01 follow-up: Keybase old-style V2 single use

The table's row 15 cites a Keybase test comment and the spike's local `max_uses = 1` model; that test covers repeated requests in one handler, not separate transactions. Review of `keybase/client` commit `952e87eed559ffc3e5c840b3354b2161e4f725f8` confirms that `InviteSeitanV2` creates an invite without `MaxUses` or expiry, and the normal sequential handler repolls chain state before consumption. A successful completion is recorded in the team chain and removes the invite from active lookups, so a later handler that sees the committed state cannot consume it again. The spike's `max_uses = 1` is a local model of that outcome, not Keybase's mechanism. This does not establish atomic exclusion of concurrent or stale handlers, or server-side replay behavior. See the source links and caveat in `pkgs/id/src/seitan/mod.rs`.

## Deviations from the brief and the design

- **Pins for crates the spike `Cargo.toml` does not list** follow `pkgs/id`'s lock: `hmac` 0.13, `sha2` 0.11, `scrypt` 0.12, `rmp-serde` 1, `getrandom` 0.3, `proptest` 1, `crypto_secretbox` 0.1, `base64` 0.22.
- **`ed25519-dalek` 3.0.0, not the spike's `"2"` pin.** `pkgs/id` uses 3, and the research (§8.1) names 3. This crate has no p2panda dependency, so the spike's pin for the p2panda side does not apply. The lock keeps only 3.0.0.
- **Token parsing is stricter and wider than Keybase's.** It rejects symbols outside the alphabet, which catches typos early. It trims surrounding whitespace. Keybase does neither. Neither changes any derivation for a valid token.
- **Non-ASCII input is rejected.** Keybase lowercases with Unicode rules. For ASCII they agree.
- **The proof is verified over the bytes received.** It is not re-encoded first.
- **`hkdf`, `blake3`, and `cbor-core` are not used.** No step in the Seitan derivations needs them, and the invite event's encoding belongs to task 02.

## New dependencies (for the brief's report requirement)

| Crate | Version | Role | Relation to `pkgs/id` |
|---|---|---|---|
| `ed25519-dalek` | 3.0.0 | runtime | direct in `pkgs/id` |
| `sha2` | 0.11.0 | runtime | direct (optional) in `pkgs/id` |
| `rmp-serde` | 1.3.1 | runtime | direct (optional) in `pkgs/id` |
| `serde` | 1.0.229 | runtime | direct in `pkgs/id` |
| `hmac` | 0.13.0 | runtime | in `pkgs/id`'s lock, not a direct dependency |
| `scrypt` | 0.12.0 | runtime | in `pkgs/id`'s lock, not direct. The research (§8.1) calls it transitive; in the spike lock it is absent. |
| `getrandom` | 0.3.4 | runtime (token generation) | in `pkgs/id`'s lock, not direct |
| `proptest` | 1.12.0 | dev only | new to the lock |
| `crypto_secretbox` | 0.1.1 | dev only (vector decryption) | new to the lock |
| `base64` | 0.22.1 | dev only | in `pkgs/id`'s lock |
| `serde_bytes` | 0.11.19 | dev only | in `pkgs/id`'s lock |
| `iroh-tickets` | 1.0.0 | probe only | direct in `pkgs/id` |
| `iroh-base` | 1.3.0 (`key` feature) | probe only | in the spike lock |
| `data-encoding` | 2.11.1 | probe only | in the spike lock |

## Surprising

1. **V1 and V2 differ in the invite-ID payload and the `+` position.** `seitan.go` still contains the V1 forms (`{stage}` only, `+` at index 5). Anyone who stops there gets the wrong ID for V2. The V2 vector catches this.
2. **V1 accepts are HMAC proofs keyed by the stretched key, and V1 stores the token itself, encrypted under the team key.** So the admin can recover the token and check the proof. V2 signs with a key derived from the token, so the admin needs only the public key. That is why the design can publish the public key in the clear.
3. **iroh-tickets 1.0.0 ignores trailing bytes** (probe step 3). The format has no integrity check, so an edited ticket still parses.
4. **Cargo artifact collision.** My mutation scratch copies and the real crate have the same package name and the same path relative to their workspace root, and they shared `CARGO_TARGET_DIR`. A mutant build was reused for the real crate, so the real crate's test run failed on the revocation test. Touching the source files fixed it. I did not confirm the exact cargo mechanism. The fix is a separate target dir for any scratch copy of a standalone crate. The other spike folders may have the same issue.
5. **The research doc's "scrypt (transitive)" does not hold for the spike lock.** scrypt is absent there. It is in `pkgs/id`'s lock.
6. **The design's 404 for Keybase is not a general block.** The raw GitHub paths for `go/teams/seitan.go` and `seitan_v2.go` return 200 on `master` today. Only the admin handler paths I tried return 404.

## Files

```
s9-seitan/
  RESULTS.md                  this file
  seitan-v2/                  the crate (Cargo.lock copied from the spike lock, then updated)
    src/token.rs              token generation and parsing
    src/derive.rs             scrypt, invite ID, Ed25519 key; V1 vector and payload bytes
    src/accept.rs             the joiner's proof: message, signing, parsing, verification
    src/admit.rs              the admin path: state, freshness, expiry, single use, revocation
    src/lib.rs
    tests/vectors.rs          Keybase V2 vectors (4 tests)
    tests/token.rs, tests/accept.rs, tests/admit.rs
    examples/dump_vectors.rs  prints the derivation steps, one proof, and the offline-guess timing
  ticket-probe/               iroh EndpointTicket probe (decode-only)
  scripts/mutation.py         the mutation check
  logs/                       test-output, clippy, dump-vectors, ticket-probe, mutation-check (all under 4 KB)
```

Code size (`wc -l`): 532 lines in `src/`, 716 in `tests/` and `examples/`.
