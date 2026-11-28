# Round 2: ed25519-dalek 2.x vs 3.x

Folder: `/home/user/work/research-round2/ed25519/`. Date: 2026-10-11 (UTC, reads 16:05–17:23). `pkgs/id` was read, never built or modified. Every statement is tagged verified, inferred, or not checked.

## Questions (verbatim)

Owner: "for ed25519 version, what is the difference? is 3.0 the newest? is there a newer one? can 2.x compat with 3.x? i'm not against moving to 2.x across board but i'd rather know more."

Brief items (condensed, not verbatim):

1. The newest ed25519-dalek on crates.io, stable and pre-release, with dates.
2. What changed between 2.x and 3.x: dependency changes (curve25519-dalek, rand_core, signature, zeroize, sha2) and API changes for key generation, signing, verify versus verify_strict, from_bytes, and error types.
3. Wire compatibility: do keys, signatures and verification results agree? Test with bytes only. Include non-canonical S, small-order public keys, and seed versus expanded-key format.
4. Can both versions be in one build, and what conversion is needed?
5. What our tree uses today: `pkgs/id/Cargo.toml`, `Cargo.lock`, the p2panda rev, iroh 1.3.0, iroh-tickets, and every ed25519 use in `pkgs/id/src`.
6. Security advisories against 2.x and 3.x.
7. Migration cost, for moving the whole tree to 2.x and for staying on 3.x while p2panda needs 2.x.

Recommend one choice with reasons. Do not modify `pkgs/id`.

## Short answers

**1. Newest version.** 3.0.0 is the newest published ed25519-dalek, and it is stable. crates.io reports `max_version` = `max_stable_version` = 3.0.0, created 2026-07-06T16:23:09Z. The newest pre-release is 3.0.0-rc.1 (2026-06-18), which is older than 3.0.0; no pre-release is newer than the stable release. The newest 2.x is 2.2.0 (2025-07-09). No yanked version is in the 2.x or 3.x line. The GitHub `main` Cargo.toml also says 3.0.0. Status: verified (crates.io API, GitHub raw read). Not checked: versions published after 2026-10-11 16:06Z.

**2. What changed.** The core code is the same. `from_bytes`, `to_bytes`, `to_keypair_bytes`, `verify`, `verify_strict`, the canonical-S check (`check_scalar`), and the decompression check have the same logic in both. The diffs of `verifying.rs`, `signing.rs`, `signature.rs`, `constants.rs` and `hazmat.rs` change only plumbing (message slices, trait bounds, cfg gates). Dependencies moved: curve25519-dalek 4.1.3 → 5.0.0, rand_core 0.6.4 → 0.10.1, signature 2.2.0 → 3.0.0, ed25519 2.2.3 → 3.0.0, sha2 0.10 → 0.11 (digest 0.10 → 0.11), pkcs8 0.10 → 0.11, edition 2021 → 2024, and MSRV 1.81 → 1.85. zeroize is `1.x` in both, so nothing forces a change there. The breaking changes that could reach our code: `SigningKey::generate` now takes a rand_core 0.10 `CryptoRng`, not 0.6's `CryptoRngCore`; the `std` feature is removed; the `signature` 3.0 error `Display` no longer prints the source; and `DigestSigner`/`DigestVerifier` take a closure. The `pkcs8` trait names changed. Status: verified (source diffs, CHANGELOGs of ed25519-dalek 2.2.0 and 3.0.0, curve25519-dalek 5.0.0, signature 3.0.0, ed25519 3.0.0, rand_core 0.10.0). Inferred: which breaking changes reach `pkgs/id`, from the grep in §E9.

**3. Wire compatibility.** Yes, in every case tested. The test binary links both versions and passes 25 of 25 checks. Across 256 seeds, public keys and 64-byte keypairs are identical. Across 64 seeds × 15 message lengths (960 cases), signatures are byte-identical, and each version verifies the other's signatures under both `verify` and `verify_strict`. The five RFC 8032 §7.1 vectors pass in both. A malleated S+L (non-canonical) is refused by all four paths (`verify` and `verify_strict`, both versions), and a canonical control passes. For small-order keys and R values, both versions produce identical counts. `verify_strict` accepts none of the forgeries tried. Plain `verify` does accept some, including a forgery under the identity key for any message. Of 5806 candidate 32-byte encodings, both versions decode the same 2900, and `is_weak` agrees on all of them. PKCS#8 DER of the same seed is byte-identical. One visible difference: the `Display` of a decode error is `signature error: Cannot decompress Edwards point` in 2.2.0 and `signature error` in 3.0.0 (Debug is identical). Status: verified (test run, §E10). Not checked: batch verification, Ed25519ph and context signing (no test; not used by `pkgs/id` in the grep).

**4. One build.** Yes. The test crate pins `=2.2.0` and `=3.0.0`, and cargo 1.97.0 resolves both side by side, with their dependencies duplicated: curve25519-dalek 4.1.3 and 5.0.0, sha2 0.10.9 and 0.11.0, digest 0.10.7 and 0.11.3, signature 2.2.0 and 3.0.0, ed25519 2.2.3 and 3.0.0, rand_core 0.6.4 and 0.10.1, pkcs8 0.10.2 and 0.11.0 (`cargo-tree-d.txt`). The spikes lock already has both (p2panda-core → 2.2.0, iroh → 3.0.0). The conversion has to go through bytes. The types are distinct, neither crate depends on the other, and there are no `From` impls between the versions. Verifying-key conversion is fallible (point decompression), and the error types differ (`signature` 2.x vs 3.x), so each crossing needs an explicit error mapping. Status: verified (build, `cargo tree -d`, spikes lock). Inferred: the absence of cross-version `From` impls, from the dependency graph and the test code, which had to convert through bytes.

**5. What our tree uses.** `pkgs/id/Cargo.toml:44` declares `ed25519-dalek = { version = "3", features = ["rand_core"] }`. `pkgs/id/Cargo.lock` has one copy, 3.0.0 (lock line 1681), with ed25519 3.0.0 (1670), signature 3.0.0 (6243), curve25519-dalek 5.0.0 (1406) and 4.1.3 (1391, via x25519-dalek 2.0.1), sha2 0.11.0 (6180), and rand_core 0.10.1 (5303). `pkgs/id` does not depend on p2panda: the lock has no `p2panda` entry. Eight lock packages depend on ed25519-dalek directly, and all are on 3.x: `id` (2866), distributed-topic-tracker 0.3.5 (1597), ed25519-dalek-hpke 0.0.5 (1697), iroh 1.3.0 (3033), iroh-base 1.3.0 (3083), mainline 7.0.0 (3807), russh 0.64.1 (5556), ssh-key 0.7.0-rc.11 (6412). p2panda-core 0.7.1 at rev `3e84cac` (spikes lock line 3351) requires ed25519-dalek 2.2.0. `pkgs/id/src` has dalek calls in 10 files (§E9; `world_ssh.rs` touches only ssh-key's key bytes). `pkgs/id` calls the non-strict `verify` at five sites (`directory_log.rs:100`, `artifact.rs:300`, `directory.rs:396`, `directory_auth.rs:349`, `web/identity.rs:432`) and never calls `verify_strict`. iroh-base (`key.rs:136`) and p2panda-core (`identity.rs:197`) both use `verify_strict`. The design text (`task-11-dependencies-and-upstream.md:30`) says "both sides use `verify_strict`", which is not true of `pkgs/id`. Status: verified (lock, source, grep). Inferred: the impact of the non-strict calls, which depends on how each call site uses the key (not traced).

**6. Advisories.** Two advisories apply to this family, and neither hits the versions in use. RUSTSEC-2022-0093 (CVE-2022-50237, GHSA-w5vr-6qhr-36cc) affects ed25519-dalek `< 2.0.0`, so 2.2.0 and 3.0.0 are patched. RUSTSEC-2024-0344 (CVE-2024-58262, GHSA-x4gp-pqpj-f43q) affects curve25519-dalek `< 4.1.3`, so 4.1.3 and 5.0.0 are patched. The GitHub advisory API lists no other ed25519-dalek or curve25519-dalek advisory (each has one duplicate entry). RustSec has no advisory directory for `ed25519`, `signature` or `zeroize`, and the GitHub query for `ed25519` and `signature` returned nothing. `sha2` RUSTSEC-2021-0100 is patched at `>= 0.9.8`, so 0.10.9 and 0.11.0 are clear. Status: verified (advisory-db raw files, GitHub advisory API, OSV GET). Not checked: `cargo audit` was not run; `rand_core` RUSTSEC-2019-0035 and RUSTSEC-2021-0023 were read only for their dates.

**7. Migration cost.** Moving the whole tree to 2.x is not a version bump, because iroh 1.3.0 and iroh-base 1.3.0 require `>=3.0.0-rc.0, <4.0.0` (`iroh-1.3.0/Cargo.toml:239-240`, `iroh-base-1.3.0/Cargo.toml:92-93`). Five more pins also require 3.x: distributed-topic-tracker 0.3.5 (`^3.0.0-rc.0`, public record API takes dalek types), mainline 7.0.0 (`^3.0.0-pre.1`, re-exports `ed25519_dalek::SigningKey` at `lib.rs:31`), russh 0.64.1 (`^3`), ssh-key 0.7.0-rc.11 (`^3.0.0-rc.1`), and ed25519-dalek-hpke 0.0.5 (`3.0.0-pre.1`). A 2.x tree would mean replacing iroh 1.3 and the DHT and ssh dependencies. Limited to `pkgs/id`'s own code, a 2.x move would change `Cargo.toml:44`, `web/identity.rs:208` (`SigningKey::generate(&mut rand::rng())` passes a rand_core 0.10 RNG), and the dalek key passed to `RecordPublisher::new` at `commands/serve.rs:889` and `commands/peers.rs:398`. Status: verified (pins, lock). Inferred (not compiled): the list of code changes. Staying on 3.x while p2panda is on 2.2 costs: byte conversions at each p2panda crossing (`SigningKey` via `as_bytes`/`from_bytes`, `VerifyingKey` via fallible `from_bytes`, `Signature` via `to_bytes`/`from_bytes`); an error mapping, because p2panda's `IdentityError` wraps the 2.x `SignatureError` (`identity.rs:401`); two copies of curve25519-dalek, sha2, signature, ed25519 and rand_core in the binary; and an explicit `verify_strict` rule on both sides, so the two layers accept the same signatures. Status: verified (the wire test shows the conversions work and that strict verification agrees); inferred (the size of the boundary).

**Recommendation.** Keep ed25519-dalek 3.0.0 for everything we own. Treat p2panda-core's 2.2.0 as a boundary: convert keys, signatures and errors through bytes in one module, and change the five `pkgs/id` call sites to `verify_strict`. This choice is version-independent and matches iroh-base and p2panda. Reasons are in §Recommendation below.

## Evidence

Commands and sources, in the order run. Output files are in this folder.

**E1. crates.io versions.** `curl -A <UA> https://crates.io/api/v1/crates/{ed25519-dalek,curve25519-dalek,ed25519,signature}` → `meta/*.json`. Parsed by `scripts/versions.py` (python3 -I). Results (2026-10-11 ~16:06Z):
- ed25519-dalek: `max_version` 3.0.0, `max_stable_version` 3.0.0, 46 versions. 3.0.0 created 2026-07-06T16:23:09; 3.0.0-rc.1 2026-06-18; 3.0.0-rc.0 2026-05-28; 3.0.0-pre.7 2026-05-06; 2.2.0 2025-07-09 (rust 1.81); 3.0.0 rust 1.85. Yanked: 0.9.0, 0.4.3.
- curve25519-dalek: `max_version` 5.0.0 (2026-07-06T16:22:48); 4.1.3 (2024-06-18); 4.2.0 yanked ("`hash_to_curve` improperly implemented", per the 5.0.0 CHANGELOG).
- ed25519: 3.0.0 (2026-05-03); 2.2.3 (2023-10-15).
- signature: 3.0.0 (2026-05-02); 2.2.0 (2023-11-12).

**E2. Downloads.** `.crate` files via `static.crates.io`, each sha256 checked against the crates.io `checksum` field: ed25519-dalek 2.2.0 and 3.0.0, curve25519-dalek 5.0.0 and 4.1.3, ed25519 3.0.0 and 2.2.3, signature 3.0.0 and 2.2.0, rand_core 0.10.1, zeroize 1.9.1. All sha256 OK. Each was extracted into its own fresh directory under `src/`. Nothing was executed from them.

**E3. GitHub main.** `raw.githubusercontent.com/dalek-cryptography/curve25519-dalek/main/ed25519-dalek/Cargo.toml` → version `3.0.0` (`gh/main_ed25519-dalek_Cargo.toml`). `p2panda/p2panda` main `p2panda-core/Cargo.toml:39` → `ed25519-dalek = { version = "2.2.0", features = ["rand_core"] }`, and main's `identity.rs` has the same structure as rev `3e84cac` (wrapper types at lines 49/165/291; `verify_strict` at 197). Latest commit touching that Cargo.toml: `083b48215` (2026-08-21). Rev `3e84cac` is dated 2026-10-09 (GitHub commits API).

**E4. Dependency changes.** From the crate manifests in `src/`:
- ed25519-dalek 3.0.0 `[dependencies]`: curve25519-dalek `5.0.0` (features digest), ed25519 `3`, rand_core `0.10` (optional), signature `3` (optional), sha2 `0.11`, subtle `2.3.0`, zeroize `1.5` (optional). Default features: `fast`, `zeroize`. `rust-version` 1.85.
- ed25519-dalek 2.2.0: curve25519-dalek `4` (digest), ed25519 `>=2.2, <2.3`, rand_core `0.6.4` (optional), signature `>=2.0, <2.3` (optional), sha2 `0.10`, subtle `2.3.0`, zeroize `1.5` (optional), merlin `3` (batch only). Default features: `fast`, `std`, `zeroize`. `rust-version` 1.81.
- The `std` feature is removed in 3.0.0, and `batch` uses strobe-rs instead of merlin (CHANGELOG 3.0.0; `src/batch/transcript.rs`).

**E5. API surface diff.** `grep -rnE '^\s*pub(\(crate\))? (fn|struct|enum|trait|const|type|use|mod|static)'` on both `src/` trees → `api/full-2.2.0.txt` (82 lines) and `api/full-3.0.0.txt` (95 lines). The public names are the same, with these differences:
- `SigningKey::generate`: `R: CryptoRngCore + ?Sized` (2.2.0, `signing.rs:202`) → `R: CryptoRng + ?Sized` (3.0.0, `signing.rs:213`). The cfg gate is now `feature = "rand_core"` only.
- `lib.rs`: 3.0.0 adds `pub use rand_core;` (feature rand_core). `DigestSigner`/`DigestVerifier` are re-exported from `::signature` (3.x).
- Added in 3.0.0: `MultipartSigner`/`MultipartVerifier` impls on `SigningKey`/`VerifyingKey`; `KeySizeUser`, `TryKeyInit`, `Generate` impls (digest feature); `verify_digest`/`try_sign_digest` take `F: Fn(&mut D)` instead of a digest value.
- pkcs8: `DynSignatureAlgorithmIdentifier` → `SignatureAlgorithmIdentifier`; `TryFrom<PrivateKeyInfo>` → `TryFrom<PrivateKeyInfoRef>`; `KeyMalformed` now carries `KeyError`.
- Unchanged in name and signature: `from_bytes`, `to_bytes`, `as_bytes`, `from_keypair_bytes`, `to_keypair_bytes`, `verifying_key`, `verify`, `verify_strict` (2.2.0 `verifying.rs:357`, 3.0.0 `verifying.rs:367`), `verify_prehashed`, `verify_prehashed_strict`, `verify_stream`, `is_weak`, `to_scalar`, `to_scalar_bytes`, `with_context`, the hazmat items, `Signature::from_bytes` (infallible), `SignatureError = ed25519::signature::Error`.

**E6. Verification core.** `diff -u` of each file pair (2.2.0 vs 3.0.0, `src/`), counting changed lines: lib.rs 66, signature.rs 2, errors.rs 8, verifying.rs 74, signing.rs 130, hazmat.rs 27, constants.rs 0, batch.rs 40, context.rs 36. In `signature.rs` the only code change is removing `#[allow(deprecated)]` on `Scalar::from_bits`, which curve25519-dalek 5.0.0 undeprecated (its CHANGELOG #780). `check_scalar` has the same default branch (`Scalar::from_canonical_bytes`, rejects S ≥ L) and the same `legacy_compatibility` branch in both. In `verifying.rs`, `from_bytes` (decompress only), `raw_verify` (cofactorless, compares compressed bytes), and `verify_strict` (rejects small-order A or R) are unchanged apart from the `&[&[u8]]` message plumbing. `errors.rs`: 3.0.0 uses `core::error::Error` and gates `from_source` on `alloc` (2.2.0 gates it on `std`). The `Display` difference in E10 comes from `signature` 3.0.0, which removed the source from `Display` (its CHANGELOG #1689). Debug prints the source in both builds, because both builds have alloc.

**E7. curve25519-dalek 5.0.0 CHANGELOG** (`src/curve25519-dalek-5.0.0/.../CHANGELOG.md`): breaking changes are edition 2024, MSRV 1.85, removal of `group-bits`, rand_core re-export, rename `batch_invert` → `invert_batch`, removal of deprecated `FieldElement::as_bytes` and `nonspec_map_to_curve`, rand_core 0.10, digest/sha2 upgrade. Also "constant-time equality testing for compressed Ristretto and Edwards points" (this affects the `expected_R == signature.R` comparison in timing only). 4.1.3 fixed the timing leak in `Scalar` subtraction (RUSTSEC-2024-0344).

**E8. signature / ed25519 / rand_core CHANGELOGs.** signature 3.0.0 (2026-05-02): closure-based `*DigestSigner`/`*DigestVerifier`; rand_core 0.10; digest 0.11; `MultipartSigner`/`MultipartVerifier`; `std`, `derive` and `PrehashSignature` removed; error source removed from `Display` (#1689). ed25519 3.0.0 (2026-05-03): pkcs8 0.11, signature 3, serdect, `Zeroize`/`Hash` for `Signature`, `std` and `serde_bytes` removed. rand_core 0.10.0 (2026-02-01): `RngCore`→`Rng`, `TryRngCore`→`TryRng`, `OsRng` and the `getrandom` dependency removed.

**E9. Our tree.**
- `pkgs/id/Cargo.toml:44` as above. `rust-version = "1.91.0"` (line 5).
- `pkgs/id/Cargo.lock`: parsed with a python script over `[[package]]` blocks (inline, not saved). Results in §Short answers 5. Other lock entries: rand_core 0.6.4 (5285) is pulled by elliptic-curve 0.13 and others; sha2 0.10.9 (6169) by hpke 0.13 and wasmtime-environ; curve25519-dalek 4.1.3 (1391) by x25519-dalek 2.0.1 (8065).
- Dalek call sites in `pkgs/id/src` (file:line, from `grep -rnE 'SigningKey|VerifyingKey|Signature|Signer|Verifier|verify_strict|\.verify\(|\.sign\(|ed25519|generate\(|OsRng|rand_core|CryptoRng'`):
  - `directory_auth.rs`: 14 (import); 292 (`&SigningKey` param); 304 (`key.sign(...).to_bytes()`); 345 (`VerifyingKey::from_bytes`); 349 (`.verify(...)`, non-strict, with `Signature::from_bytes` at the same site). Tests: 449, 484, 513, 530, 541 (`SigningKey::from_bytes(&[7; 32])`).
  - `artifact.rs`: 28 (import); 221, 236, 505, 516 (`&SigningKey` params); 248 (sign); 293 (`VerifyingKey::from_bytes`); 297 (`Signature::from_bytes`); 300 (`key.verify`, non-strict); tests 860–861, 1113–1115.
  - `directory_log.rs`: 15 (import); 61, 132 (`&SigningKey` params); 65 (sign); 93 (`from_bytes`); 97 (`Signature::from_bytes`); 100 (`key.verify`, non-strict); tests 313–314.
  - `directory.rs`: 14 (import); 356 (param); 376 (sign); 388 (`from_bytes`); 392 (`Signature::from_bytes`); 396 (`key.verify`, non-strict); 1410 (`authenticate` returns `SigningKey`); 1747–1751 (`signing_key()` from a SHA-512 digest via `SigningKey::from_bytes`); 1769 (`VerifyingKey::from_bytes`); tests 2408, 2694.
  - `web/identity.rs`: 41 (import); 89, 91 (`Arc<SigningKey>`, `VerifyingKey` fields); 123 (`from_bytes`); 208 (`SigningKey::generate(&mut rand::rng())`); 395 (sign); 431 (`Signature::from_bytes`); 432 (`verify`, non-strict).
  - `web/world_ws.rs`: 325 (test import); 648 (test param); 673 (test `from_bytes`).
  - `world_session.rs`: 2123 (test import); 2223 (test `from_bytes`).
  - `directory_view.rs`: 1560 (test import); 1867, 1893, 2183, 2339 (test `from_bytes`/`verifying_key`).
  - `commands/serve.rs`: 888–889 (iroh `SecretKey` → dalek `SigningKey` via `from_bytes(&key.to_bytes())`), 891–896 (`RecordPublisher::new` takes that dalek key).
  - `commands/peers.rs`: 398 (same pattern), 400–405 (`RecordPublisher::new`).
  - `world_ssh.rs`: 157, 167, 849 use ssh-key's `ed25519()` public key bytes. No dalek types.
  - `SecretKey::generate` in `local.rs:66`, `store.rs`, `commands/serve.rs` tests: these are `iroh_base::SecretKey` (imports at `local.rs:28`, `store.rs:48`), not dalek.
  - `verify_strict`: 0 occurrences in `pkgs/id/src`. `verify_batch`, `DigestVerifier`, `hazmat`, `legacy_compatibility`: 0 dalek uses. `with_context` matches are anyhow's; "legacy" appears only in unrelated metadata-migration code (`local.rs:3, 113`).
  - `pkgs/id/repl-test` is a standalone manifest (package `id`, iroh 0.96, `ed25519-dalek = "3.0.0-pre.1"`). It is not in the lock.
- Transitive pins (registry copies under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`): iroh-1.3.0 `Cargo.toml:239–240` (`>=3.0.0-rc.0,<4.0.0`, features serde, rand_core, zeroize, pkcs8, pem); iroh-base-1.3.0 `Cargo.toml:67–68` (curve25519-dalek `>=5.0.0-rc.0,<6.0.0`) and `92–93` (ed25519-dalek `>=3.0.0-rc.0,<4.0.0`); ed25519-dalek-hpke-0.0.5 `Cargo.toml:44–46` (`3.0.0-pre.1`, rand_core). iroh-tickets-1.0.0 has no direct ed25519 dependency (deps: data-encoding, derive_more, iroh-base, n0-error, postcard, serde), so it inherits 3.x via iroh-base. The crates.io dependency metadata (`meta/deps-*.json`) gives: ssh-key 0.7.0-rc.11 → ed25519-dalek `^3.0.0-rc.1` (optional); russh 0.64.1 → ed25519-dalek `^3` (features alloc, rand_core, pkcs8), curve25519-dalek `^5`, signature `^3`; distributed-topic-tracker 0.3.5 → ed25519-dalek `^3.0.0-rc.0`; mainline 7.0.0 → ed25519-dalek `^3.0.0-pre.1`; x25519-dalek 2.0.1 → curve25519-dalek `^4`. iroh 1.3.0 → ed25519-dalek `>=3.0.0-rc.0, <4.0.0`.
- Public-API coupling: distributed-topic-tracker-0.3.5 `src/crypto/record.rs:249, 259, 431, 566` (`pub_key()`, `signing_key()`, `decrypt`, `encrypt` take or return `ed25519_dalek` types); `src/crypto/keys.rs:103` (`pub fn signing_keypair` → `ed25519_dalek::SigningKey`; `mod crypto` is private, so this is reachable only through the re-exports). mainline-7.0.0 `lib.rs:31`: `pub use ed25519_dalek::SigningKey;`.
- Spikes: `/home/user/work/spikes/Cargo.lock`: `ed25519-dalek` 2.2.0 (lines 1090–1102), 3.0.0 (1105–1117); p2panda-core 0.7.1 (3351) → `ed25519-dalek 2.2.0` (3357); iroh 1.3.0 (2122) → `ed25519-dalek 3.0.0` (2134). `/home/user/work/spikes/Cargo.toml` pins `ed25519-dalek = "2"` in workspace dependencies. `s1-transport/RESULTS.md:121` says the 2.2.0 check "is not cross-tested against 3.0.0". This round's test does that cross-check.
- p2panda rev `3e84cac` checkout (`~/.cargo/git/checkouts/p2panda-de6556ee3f16c1a4/3e84cac/`):
  - `p2panda-core/Cargo.toml:39`: `ed25519-dalek = { version = "2.2.0", features = ["rand_core"] }`.
  - `p2panda-core/src/identity.rs`: wrapper types `SigningKey(ed25519_dalek::SigningKey)` (49), `VerifyingKey(ed25519_dalek::VerifyingKey)` (165), `Signature(ed25519_dalek::Signature)` (291); `VerifyingKey::from_bytes` (181–182, fallible); `VerifyingKey::verify` (196–197) calls `verify_strict`; `From` impls that expose 2.x types (215–222, 334–341); `IdentityError::InvalidSignature(#[from] ed25519_dalek::SignatureError)` (400–401); `SigningKey::generate` uses `rand::rngs::OsRng` (rand 0.8, `rand_core` 0.6).
  - Callers in p2panda: `operation/header.rs:370–378` (sign and verify), `operation/any.rs:344` (verify).
- iroh-base-1.3.0 `src/key.rs`: `PublicKey(CompressedEdwardsY)` (30); `PublicKey::from_bytes` (122–124) → `VerifyingKey::from_bytes`; `PublicKey::verify` (134–137) calls `verify_strict`; `SecretKey(SigningKey)` (261) with `as_signing_key` (`#[doc(hidden)]`, 344); `Signature(ed25519_dalek::Signature)` (376).

**E10. Wire test.** Crate: `wire/` (its own `[workspace]`, not part of pkgs/id or the spikes). Pins: ed25519-dalek `=2.2.0` (features rand_core, hazmat, pkcs8, digest) and `=3.0.0` (features rand_core, hazmat, serde, zeroize, pkcs8, pem, digest); curve25519-dalek `=4.1.3` (constants only); rand_core `=0.6.4` (getrandom); getrandom `0.4` (sys_rng); sha2 `0.10`; hex. Vectors: `scripts/gen_rfc_vectors.py` reads `rfc/dl/rfc8032.txt` (rfc-editor, 3363 lines, §7.1 at lines 1301–1497) and writes `wire/src/rfc_vectors.rs` (5 vectors, asserted lengths). Build: `CARGO="$(rustup which --toolchain 1.97.0 cargo)"; "$CARGO" build --release --manifest-path ./Cargo.toml` (final build finished with no warnings shown; two first-build compile errors were fixed by enabling the `digest` feature). Run: `./target/release/ed25519-wire-compat` → `wire-run1.txt`, exit 0.

Output (verbatim checks, trimmed):

```
PASS keys: public key from the same 32-byte seed, 256 seeds: mismatches=0
PASS keys: 64-byte keypair (seed||pk), 256 seeds: mismatches=0
PASS RFC 8032 TEST 1 (0 byte msg) ... v2 verify/strict=true/true v3 verify/strict=true/true
PASS RFC 8032 TEST 2 (1 byte msg) ... (same)
PASS RFC 8032 TEST 3 (2 byte msg) ... (same)
PASS RFC 8032 TEST 1024 (1023 byte msg) ... (same)
PASS RFC 8032 TEST SHA(abc) (64 byte msg) ... (same)
PASS sign: same seed+message gives identical 64 bytes in 2.2.0 and 3.0.0: cases=960 differing=0
PASS cross-verify: 2.2.0 signatures accepted by 3.0.0 (verify and verify_strict): cases=960 rejected by verify=0 by verify_strict=0
PASS cross-verify: 3.0.0 signatures accepted by 2.2.0 (verify and verify_strict): cases=960 rejected by verify=0 by verify_strict=0
PASS tampered message, R, S or key: all rejected: cases=96 accepted [v2 verify, v2 strict, v3 verify, v3 strict]=[0, 0, 0, 0]
PASS non-canonical S: control (canonical S) accepted by all four paths: v2=(true, true) v3=(true, true)
PASS non-canonical S+L: rejected by v2 verify and verify_strict and v3 verify and verify_strict: ... top byte of S+L=0x12 (old top-3-bits check would pass)
PASS S with bit 255 set: rejected by all four paths
PASS small-order keys: verify_strict accepts no torsion-R forgery (2.2.0 and 3.0.0)
PASS small-order keys: 2.2.0 and 3.0.0 give identical decode, is_weak and verify counts
PASS identity key: plain verify accepts a torsion-R forgery (2.2.0), as predicted: accepted=1
PASS identity key: forgery R=[s]B, S=s accepted by verify, rejected by verify_strict (both versions): trials=8 accepted [v2 verify, v2 strict, v3 verify, v3 strict]=[8, 0, 8, 0]
PASS control: non-small-order key with R=identity, S=0 rejected by all four paths
PASS expanded key: SHA-512(seed) through hazmat gives the same signatures and keys as seed signing: seeds=32 mismatches=0
PASS keypair (seed||pk): 2.2.0 bytes load in 3.0.0, and a tampered public half is refused by both: seeds=32 failures=0
PASS PKCS#8 DER of the seed: byte-identical in 2.2.0 and 3.0.0, and 3.0.0 reads 2.2.0's: seeds=32 failures=0
PASS decode: VerifyingKey::from_bytes accepts the same 32-byte strings in 2.2.0 and 3.0.0: encodings=5806 decodes v2=2900 v3=2900 disagreements=0
PASS decode: is_weak agrees for every decodable key: disagreements=0 small-order among decodable=14
PASS generate(): keys from OsRng (2.2.0, rand_core 0.6) and SysRng via UnwrapErr (3.0.0, rand_core 0.10) cross-verify
SUMMARY pass=25 fail=0
```

Small-order table (INFO lines; counts are "of 8 torsion R values with S = 0 accepted"; v2 and v3 counts are identical in every row):

| key | verify (2.2.0 = 3.0.0) | verify_strict |
|---|---|---|
| EIGHT_TORSION[0], identity `0100…00` | 1 | 0 |
| EIGHT_TORSION[1] `c7176a70…037a` | 3 | 0 |
| EIGHT_TORSION[2] `00…0080` | 1 | 0 |
| EIGHT_TORSION[3] `26e8958f…fc05` | 0 | 0 |
| EIGHT_TORSION[4] `ecff…ff7f` | 2 | 0 |
| EIGHT_TORSION[5] `26e8958f…fc85` | 0 | 0 |
| EIGHT_TORSION[6] `00…00` (order 4) | 2 | 0 |
| EIGHT_TORSION[7] `c7176a70…03fa` | 1 | 0 |
| identity, non-canonical `eeff…ff7f` (y=p+1) | 1 | 0 |
| order-4, non-canonical `edff…ff7f` (y=p) | 1 | 0 |

Identity key with R=[s]B, S=s (4 scalars × 2 messages): plain verify accepts 8/8 in both versions; verify_strict 0/8.

Error text (INFO): off-curve key `0200…00`: 2.2.0 `Display` = `signature error: Cannot decompress Edwards point`; 3.0.0 `Display` = `signature error`; Debug is `signature::Error { source: Some(Cannot decompress Edwards point) }` in both. 63-byte signature slice: `signature error` in both.

PKCS#8 for seed 0 (identical in both): `3051020101300506032b657004220420` + seed + `8121` + pk. This is the RFC 8410 v2 structure with the public key `[1]`.

What the test does not exercise: the cargo `legacy_compatibility` feature (not enabled; no dependent in the registry or crates.io metadata requests it), batch verification, Ed25519ph and context signing, and performance.

**E11. Advisories.** Read 2026-10-11 ~16:07–16:10Z:
- `raw.githubusercontent.com/RustSec/advisory-db/main/crates/ed25519-dalek/RUSTSEC-2022-0093.md`: `date = 2022-06-11`, `patched = [">= 2"]`, `aliases = [CVE-2022-50237, GHSA-w5vr-6qhr-36cc]`. Summary: "Double Public Key Signing Function Oracle Attack" (keypair-type APIs before 2.0).
- `.../crates/curve25519-dalek/RUSTSEC-2024-0344.md`: `date = 2024-06-18`, `patched = [">= 4.1.3"]`, aliases CVE-2024-58262, GHSA-x4gp-pqpj-f43q. Timing variability in `Scalar29::sub`/`Scalar52::sub`.
- `api.github.com/repos/RustSec/advisory-db/contents/crates/{ed25519-dalek,curve25519-dalek,ed25519,signature,rand_core,zeroize,sha2}`: ed25519-dalek has RUSTSEC-2022-0093 only; curve25519-dalek has RUSTSEC-2024-0344 only; ed25519, signature and zeroize return 404 (no directory); rand_core has RUSTSEC-2019-0035 and RUSTSEC-2021-0023; sha2 has RUSTSEC-2021-0100 (`patched >= 0.9.8`, `unaffected < 0.9.7`).
- `api.github.com/advisories?ecosystem=rust&affects=…` for ed25519-dalek: GHSA-w5vr-6qhr-36cc (CVE-2022-50237, `< 2.0.0`, patched 2.0.0) and GHSA-g693-v3jr-8hcr (duplicate). For curve25519-dalek: GHSA-x4gp-pqpj-f43q (CVE-2024-58262, `< 4.1.3`, patched 4.1.3) and GHSA-4hff-hh47-7788 (duplicate). For ed25519 and signature: empty.
- OSV `GET api.osv.dev/v1/vulns/RUSTSEC-2022-0093`: affected `ed25519-dalek` introduced 0.0.0-0, fixed 2.0.0. `…/RUSTSEC-2024-0344`: `curve25519-dalek` introduced 0.0.0-0, fixed 4.1.3.

**E12. Design text read (grep only).** `/home/user/work/integrated/doc/2026-10-11T11-08-42Z_design_remaining_work/`:
- Index row 19 (`2026-10-11T11-08-42Z_design_remaining_work.md:75`): "ed25519-dalek version | Open | 2.x, to match p2panda-core, one copy in the tree | 3.0 with a shim at the boundary".
- `task-12-dependencies-and-toolchain.md:50`: option C, "Align with p2panda-core's 2.2 (preferred, one copy in the tree), or keep 3.0 and shim at the boundary (alternative)".
- `task-11-dependencies-and-upstream.md:30`: "p2panda-core uses 2.2, and our code and iroh use 3.0. Keys cross as bytes, and both sides use `verify_strict`" (see the correction in Short answer 5).
- `task-07-invites-and-pull.md:70`: "the spike used ed25519-dalek 3, to match `pkgs/id`".

**E13. Other checks.** `pkgs/id/Cargo.lock` has no `p2panda` entries (grep count 0). `pkgs/id/Cargo.toml` has `edition = "2024"`, `rust-version = "1.91.0"`.

## Options

| Option | What it means | Feasible with the current pins? | Cost | Verdict |
|---|---|---|---|---|
| A. `pkgs/id` on 3.0.0; p2panda's 2.2.0 behind a byte boundary; `verify_strict` on both sides | Two copies of dalek and its dependencies in the binary | Yes. Builds in one tree (verified, E10) | A conversion module (3 helpers and error mapping); five `verify` → `verify_strict` edits; a rule to keep the two sides' acceptance identical | **Recommended** |
| B. `pkgs/id` on 2.x ("across the board") | One dalek 2.x in code we own | **No.** iroh 1.3.0 and iroh-base 1.3.0 require 3.x, and so do distributed-topic-tracker, mainline, russh, ssh-key and ed25519-dalek-hpke (E9) | Replacing iroh 1.3 (with iroh-tickets, -blobs, -docs, -gossip), the DHT and ssh stack. Not a version bump | Rejected |
| C. Wait for p2panda to move to 3.x | Removes the boundary | Not yet. Upstream main still says 2.2.0 (E3) | None until upstream moves | Watch |
| D. Fork p2panda-core onto 3.x | Removes the boundary | Technically (not tested) | Fork maintenance; semantics would need re-testing | Last resort |
| E. No boundary code; let p2panda types meet ours directly | None | No. p2panda's types wrap 2.x and expose 2.x `From` impls | Not possible without conversion code | Not an option |

## Recommendation

Choose **Option A**: keep ed25519-dalek 3.0.0 for everything we own, and treat p2panda-core's 2.2.0 as a boundary.

Reasons:
1. The "2.x across the board" move is blocked by the pins. iroh 1.3.0, iroh-base 1.3.0, distributed-topic-tracker, mainline, russh, ssh-key and ed25519-dalek-hpke all require 3.x (E9). Moving `pkgs/id` alone would only move the boundary into iroh.
2. The design goal of "one copy in the tree" (index row 19) cannot be met while iroh 1.3.0 and p2panda-core 0.7.1 are both pinned, since they need different majors. Two copies is the minimum. The question is only where the boundary sits.
3. 3.0.0 is the newest stable release, and in every case tested its keys, signatures and verification results match 2.2.0 (E10). Its only visible change in our tests is the `Display` text of errors.
4. The boundary is small and mechanical. Keys, signatures and verifying keys cross as bytes (each is a 32- or 64-byte array). The one thing to get right is making both sides verify the same way. p2panda already uses `verify_strict` (`identity.rs:197`), and iroh-base does too (`key.rs:136`). `pkgs/id` uses plain `verify` at five sites, and the design text assumes it does not. Changing those five sites is independent of the version choice, and it removes the mismatch.
5. Plain `verify` is not safe for untrusted keys. The test accepts a forgery under the identity key for any message, and a torsion-R forgery under several other small-order keys (E10). Whether that is exploitable in `pkgs/id` depends on how each call site uses the key, which I did not trace.

What would change this recommendation:
- p2panda releasing a version on ed25519-dalek 3.x. Upstream main still requires 2.2.0 (fetched 2026-10-11), so the boundary stays until then.
- iroh, distributed-topic-tracker or mainline moving back to 2.x. Not expected; they are moving forward.
- An advisory against ed25519-dalek 3.0.0 or curve25519-dalek 5.0.0. None is known as of 2026-10-11.
- A decision to fork p2panda-core (Option D). That would remove the boundary but add maintenance.
- A need for p2panda types in `pkgs/id`'s public API. Then the conversions spread, and a thin wrapper type at the edge would be worth it.

Suggested edits, not made here (the brief says not to modify `pkgs/id`, and I did not touch the design docs):
- Decision 19 (index row 19 and `task-12` option C): replace "one copy in the tree" with "two copies, one boundary" and recommend 3.0 with a shim.
- `task-11-dependencies-and-upstream.md:30`: change "both sides use `verify_strict`" to say `pkgs/id` uses `verify` today and must change to `verify_strict`.
- `pkgs/id`: change `directory_log.rs:100`, `artifact.rs:300`, `directory.rs:396`, `directory_auth.rs:349` and `web/identity.rs:432` from `verify` to `verify_strict`. Test them after the change, since each call site's key handling may have its own assumptions.

## Not verified

- `pkgs/id` was not built, and `cargo tree` was not run there. The migration line list (E9, Short answer 7) comes from reading, not compiling. The "2.x would change these lines" claim is inferred.
- I did not trace whether each non-strict `verify` call site also checks the key against an allowlist or membership list. So the risk from E10's forgery results is not assessed for any call site.
- Not tested: batch verification (`verify_batch`, which differs in transcript: merlin in 2.x, strobe-rs in 3.x), Ed25519ph and context signing, the hazmat `raw_verify` entry points directly, and performance. None of these appear in `pkgs/id` or p2panda in the grep, but p2panda was grepped only in `p2panda-core`.
- Not checked: whether iroh-blobs, iroh-docs or iroh-gossip request ed25519-dalek directly (I only read iroh and iroh-base).
- Not checked: whether any dependent in our tree requests `legacy_compatibility`. The registry copies do not, and the crates.io metadata for russh and ssh-key does not. Mainline and distributed-topic-tracker are in the registry and do not request it. The check is by file search, not by `cargo tree -e features` on `pkgs/id`.
- Not checked: RUSTSEC-2019-0035 and RUSTSEC-2021-0023 (rand_core): I read only their dates, not their affected ranges. Both concern older rand_core lines, but I have not confirmed that against 0.6.4 or 0.10.1.
- Not run: `cargo audit`, and OSV bulk queries. Advisory results are from the specific reads listed in E11, on 2026-10-11.
- Not checked: upstream dalek `CHANGELOG.md` on `main` for any unreleased section (only `Cargo.toml` version was read).
- The statement "no `From` impls between 2.x and 3.x types" is inferred from the dependency graph and the test code, not from a compile-fail test.
- The error `Display` text depends on the `signature` version (3.0.0 omits the source) and on ed25519-dalek's `alloc`/`std` gates, which decide whether a source exists. The test's 3.0.0 build enables `pem` (so `alloc`), matching iroh 1.3.0's feature request (`iroh-1.3.0/Cargo.toml:239–246`). I did not run `pkgs/id`'s build, so its exact feature set is not checked.
- Timing: nothing here measures constant-time behaviour. The curve25519-dalek 5.0.0 change to constant-time point equality was read from the CHANGELOG, not measured.
