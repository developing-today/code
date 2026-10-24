# S12: welcome size as epochs accumulate

Status: measured. Raw output is in `logs/`. Five runs per build (seeds 1–5), release profile, pinned p2panda rev `3e84cac`.

## Question

Task 05 and the design review ask for the welcome size at 1,000 rotations, with and without the `serde_bytes` patch (design review §3.5 and §6 item 5; task-05 Acceptance). The brief adds:

- bytes per epoch and the total at K = 1, 10, 100, 1,000 rotations, for the stock crate and the serde_bytes variant;
- whether any part grows faster than linearly;
- the welcome with only the N newest epochs (N = 1, 8, 32), if the API allows it;
- the time to create the welcome at K = 1,000.

## Summary

- **Stock:** 146.6 B per epoch (fit over five runs, sd 0.1). At K = 1,000 (1,001 secrets) the welcome is 147.4 KB (mean of five runs, sd 124 B).
- **serde_bytes:** 40.0 B per epoch, exactly. At K = 1,000 the welcome is 40,493 B.
- **Growth:** each welcome grows linearly with rotations (members were fixed at three). The cumulative bytes of welcomes sent for churn (remove, then re-add) grow with the square of K: 19.5 MB (stock) and 10.2 MB (serde_bytes) at K = 1,000, from 500 re-adds.
- **recent N:** the public API allows it. At K = 1,000, stock welcomes are 747 B, 1,775 B and 5,323 B for N = 1, 8, 32 (five-run means). serde_bytes gives 489 B, 771 B and 1,732 B. The joiner learns exactly those N secrets.
- **Time at K = 1,000 (stock):** `Dcgka::add` 1.76 ms, CBOR encoding 1.96 ms. serde_bytes: 0.51 ms and 0.003 ms.
- **Wire compatibility:** a PreKey welcome from the serde_bytes build does not decode on the stock build, and the reverse also fails.
- **The S3 "patched" copy does not contain the serde_bytes change.** The serde_bytes numbers come from a copy annotated in this folder (see Method).

## Method

### Builds

| Build | Source | Change from the pin |
|---|---|---|
| stock | git, p2panda rev `3e84cac`, p2panda-encryption 0.7.1, features `data_scheme`, `test_utils` | none |
| serde-bytes | `variants/serde-bytes/p2panda-encryption`, a copy of `/home/user/work/spikes/s3-history/patched/p2panda-encryption` | `#[serde(with = "serde_bytes")]` on `TwoPartyPlaintext.plaintext` (`src/two_party/two_party.rs:244`) and on `X3dhCiphertext.ciphertext` (`src/two_party/x3dh.rs:41`). `logs/variant-diff-serde-bytes-vs-s3-path.diff` shows only these two lines. |
| found-as-is (control) | copy of the same S3 path copy, in `variants/found/` | Package renamed to `p2panda-encryption-found`. Two path packages with the same name and version collide in the lockfile. Otherwise unchanged. |

**The S3 path copy does not have the serde_bytes change.** Its `two_party/` and `x3dh.rs` have no `serde(with` attributes. `diff -rq` against the pinned checkout's `src` shows only `data_scheme/dcgka.rs` and `data_scheme/group.rs` differ (the membership fix and `add_from_join`). S3's RESULTS list the serde_bytes fix as "described, not built". The `patch.diff` in S3's logs has no field-level `serde_bytes` change. Its `Cargo.toml` hunk only turns workspace dependencies into explicit versions and drops `[lints]`. The brief's "serde_bytes variant from the patched path" therefore does not exist as described, so I built the variant myself.

### Serializer

Sizes are the length of `p2panda_core::cbor::encode_cbor(&dm)`, where `dm` is the crate's `DirectMessage`. That function calls `cbor_core::Value::serialized` and then `.encode()`, so the serializer is **cbor-core 0.10.1** with its serde feature (the version in `Cargo.lock`). It is the CBOR the p2panda stack uses for headers (S5). The encryption crate has no serializer of its own.

serde writes a `Vec<u8>` without `serde_bytes` as a CBOR array of integers. Each byte costs 1 byte below 24 and 2 bytes from 24 to 255, about 1.91 bytes on random data.

### What is measured

- The welcome is the whole `DirectMessage` with `DirectMessageContent::Welcome { ciphertext, history }`. `history` (the DGM state) and `ciphertext` (the `TwoPartyMessage`) are also encoded separately.
- The probe joiner is a fresh identity, so its welcome takes the X3DH (PreKey) path (`logs/welcome-k1-stock.diag`: `"key_used": "PreKey"`).
- The ordering message that carries the welcome is not included. S3 measured the same DM alone.

### Scenario

For each build and run. The RNG seed is the same for all builds.

1. Admin (the adder), Bob and Carol: `EncryptionGroup::create(admin, [admin, bob, carol])`. One secret.
2. Rotations k = 1 … 1000. **Odd k:** `EncryptionGroup::update` (one secret). **Even k:** `EncryptionGroup::remove(carol)` (one secret), then `EncryptionGroup::add(carol)`, which mints no secret and keeps three members. After K rotations the admin's bundle holds K + 1 secrets.
3. Samples at K ∈ {0, 1, 2, 5, 10, 50, 100, 250, 500, 750, 1000}. At each sample a probe joiner gets a welcome from `Dcgka::add` on a clone of the admin's state, with three bundles:
   - all secrets (full history);
   - the N newest secrets for N = 1, 8, 32, built with `SecretBundle::from_secrets`;
   - an empty bundle (the from_join welcome).
4. At K = 1000, the public `EncryptionGroup::add(join)` path gives the joiner welcome with the full bundle.
5. Joiner check, at K = 1000: the probe joiner processes each welcome with its own keys, through `Dcgka::process` with the Add control message. The secret IDs it learns are compared with the expected set.
6. Times: `Instant` around `Dcgka::add` (the welcome), around `encode_cbor` (the bytes), and around `EncryptionGroup::add`. State clones are made outside the timer. Medians of 15 runs per sample, and 50 at K = 1000.

Two consequences of the scenario:

- Bob and Carol never process or send messages. The admin's state changes only through its own calls, and nothing comes back to the admin, so receiver processing should not change the admin's welcome. I did not check this by running a full group.
- Re-adding Carol uses her existing 2SM session, because `remove` does not delete the `two_party` entry (`dcgka.rs`, `remove` and `encrypt_to`). So the re-add welcome takes the HPKE branch (`logs/readd-first-stock.diag`: `"key_used": "ReceivedKey"`, `"Hpke"`).

**Stock sizes are not reproducible across processes, even with the same seed.** Seed 1 at K = 1,000 gave 147,438 B in the logged run and 147,401 B on a re-run of the same harness. The serde-bytes sizes were identical in every run. Two sources of run-to-run variation are visible in the code. `GroupSecret::from_rng` stamps each secret with the wall clock (`SystemTime::now()`), and `SecretBundleState` serializes its secrets in `HashMap` order. Both change the bytes that stock's integer-array encoding then sizes. I did not isolate which one matters, so the stock spread below is run-to-run, not seed-driven.

### Commands

```
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"     # rustc 1.97.0
cd /home/user/work/spikes-next/s12-welcome
"$CARGO" build --release --offline
for s in 1 2 3 4 5; do "$CARGO" run --release --offline -q -- --seed $s > logs/run-seed$s.txt; done
sh summarize.sh                                        # writes logs/summary.txt
diff -ru /home/user/work/spikes/s3-history/patched/p2panda-encryption variants/serde-bytes/p2panda-encryption > logs/variant-diff-serde-bytes-vs-s3-path.diff
```

The first release build took 33 s, and one run takes about 4 s. There was no stderr output. The logs are from the last run of the final harness.

## Results

### Stock, run seed 1

```
| K    | secrets | welcome B | history B | ciphertext B | recent 1 | recent 8 | recent 32 | empty B | B/secret since prev | welcome us (med) | encode us (med) | re-add B so far |
| 0    |    1    |    736    |    29     |     658      |   734    |   735    |    730    |   596   |          -          |       392        |        9        |        0        |
| 1    |    2    |    893    |    29     |     815      |   743    |   887    |    890    |   601   |        157.0        |       432        |       11        |        0        |
| 2    |    3    |   1034    |    29     |     956      |   743    |  1034    |   1035    |   599   |        141.0        |       425        |       14        |      567        |
| 5    |    6    |   1478    |    29     |    1400      |   748    |  1483    |   1476    |   598   |        148.0        |       428        |       32        |     1289        |
| 10   |   11    |   2204    |    29     |    2126      |   742    |  1766    |   2211    |   595   |        145.2        |       451        |       32        |     4377        |
| 50   |   51    |   8055    |    29     |    7977      |   747    |  1778    |   5336    |   596   |        146.3        |       372        |      111        |    60189        |
| 100  |  101    |  15402    |    29     |   15324      |   748    |  1780    |   5292    |   596   |        146.9        |       445        |      202        |   216451        |
| 250  |  251    |  37334    |    29     |   37256      |   739    |  1770    |   5297    |   594   |        146.2        |       621        |      504        |  1262259        |
| 500  |  501    |  74045    |    29     |   73967      |   740    |  1763    |   5288    |   598   |        146.8        |       916        |     1019        |  4926282        |
| 750  |  751    | 110728    |    29     |  110650      |   748    |  1775    |   5332    |   601   |        146.7        |      1212        |     1487        | 10993683        |
| 1000 | 1001    | 147438    |    29     |  147360      |   753    |  1773    |   5353    |   600   |        146.8        |      1861        |     2110        | 19468141        |
```

Fit over these 11 samples: slope 146.68 B per secret, intercept 581.8 B, largest residual 63.7 B. Final `EncryptionGroup::add` at K = 1000: 147,349 B.

### serde-bytes, run seed 1

```
| K    | secrets | welcome B | history B | ciphertext B | recent 1 | recent 8 | recent 32 | empty B | B/secret since prev | welcome us (med) | encode us (med) | re-add B so far |
| 0    |    1    |    489    |    29     |     411      |   489    |   489    |    489    |   448   |          -          |       292        |        2        |        0        |
| 1    |    2    |    529    |    29     |     451      |   489    |   529    |    529    |   448   |         40.0        |       298        |        2        |        0        |
| 2    |    3    |    570    |    29     |     492      |   489    |   570    |    570    |   448   |         41.0        |       436        |        2        |      459        |
| 5    |    6    |    690    |    29     |     612      |   489    |   690    |    690    |   448   |         40.0        |       359        |        2        |      998        |
| 10   |   11    |    891    |    29     |     813      |   489    |   771    |    891    |   448   |         40.2        |       430        |        2        |     3098        |
| 50   |   51    |   2492    |    29     |    2414      |   489    |   771    |   1732    |   448   |         40.0        |       296        |        2        |    35526        |
| 100  |  101    |   4492    |    29     |    4414      |   489    |   771    |   1732    |   448   |         40.0        |       299        |        2        |   121076        |
| 250  |  251    |  10492    |    29     |   10414      |   489    |   771    |   1732    |   448   |         40.0        |       347        |        2        |   677726        |
| 500  |  501    |  20493    |    29     |   20415      |   489    |   771    |   1732    |   448   |         40.0        |       436        |        3        |  2605722        |
| 750  |  751    |  30493    |    29     |   30415      |   489    |   771    |   1732    |   448   |         40.0        |       436        |        3        |  5783722        |
| 1000 | 1001    |  40493    |    29     |   40415      |   489    |   771    |   1732    |   448   |         40.0        |       647        |        3        |  10211722       |
```

Fit: slope 40.00 B per secret, intercept 450.5 B, largest residual 1.5 B. Final `EncryptionGroup::add` at K = 1000: 40,493 B.

### Five runs at K = 1,000

Mean over the five runs, with sample sd in brackets.

| build | probe welcome B | `EncryptionGroup::add` welcome B | B per epoch (fit) | fixed part, intercept (B) | largest fit residual (B) | cumulative re-add B |
|---|---|---|---|---|---|---|
| stock | 147,362 (124) | 147,368 (174) | 146.6 (0.1) | 583 (6) | 78 (33) | 19,467,376 (16,192) |
| serde-bytes | 40,493 (0) | 40,493 (0) | 40.00 (0) | 450.5 (0) | 1.5 (0) | 10,211,722 (0) |
| found-as-is (control) | 147,319 (152) | 147,358 (177) | 146.6 (0.1) | 587 (12) | 76 (25) | 19,467,813 (16,009) |

The per-epoch cost decomposes as 40 B raw × 1.91 × 1.91 ≈ 145 B. There are two `Vec<u8>` layers: the bundle inside `TwoPartyPlaintext.plaintext`, then the AEAD output inside `X3dhCiphertext.ciphertext`. The measured 146.6 B is close to that. The found-as-is build matches stock, which confirms the S3 path copy has no serde-level change on the welcome path.

### recent N and the empty welcome, K = 1,000, five runs

| bundle | stock B (mean, range) | serde-bytes B |
|---|---|---|
| empty (from_join) | 596 (593–600) | 448 |
| N = 1 | 747 (734–754) | 489 |
| N = 8 | 1,775 (1,764–1,783) | 771 |
| N = 32 | 5,323 (5,285–5,353) | 1,732 |
| full, 1,001 secrets | 147,362 | 40,493 |

The serde-bytes values did not vary across runs. Stock welcomes did.

Each recent-N welcome is flat once K + 1 ≥ N. For serde-bytes, the N = 8 value is 771 B from K = 10 on, and the N = 32 value is 1,732 B from K = 50 on.

### Joiner check

All 60 checks pass: 3 builds × 5 runs × 4 checks (full, N = 1, 8, 32). For each welcome the joiner learns 1,001 secrets for full history, or exactly N secrets, with the same IDs as the admin's N newest.

### Time at K = 1,000

| step | stock | serde-bytes |
|---|---|---|
| `Dcgka::add`: builds the welcome (bundle encoding, 2SM/X3DH encryption). Median of 50 per run, mean over five runs (range) | 1.76 ms (1.46–1.90) | 0.51 ms (0.47–0.65) |
| `encode_cbor` of the welcome | 1.96 ms (1.72–2.16) | 0.003 ms |
| `EncryptionGroup::add` (full public API, with local processing) | 1.67 ms (1.42–1.84) | 0.47 ms (0.45–0.50) |

Run seed 1, `Dcgka::add` median by K (250 / 500 / 750 / 1000): stock 621 / 916 / 1,212 / 1,861 µs; serde-bytes 347 / 436 / 436 / 647 µs. This machine is shared and noisy, so read these as order-of-magnitude figures in release builds.

### Churn: welcomes sent for re-adds, run seed 1

| K | stock cumulative B | serde-bytes cumulative B |
|---|---|---|
| 250 | 1,262,259 | 677,726 |
| 500 | 4,926,282 | 2,605,722 |
| 750 | 10,993,683 | 5,783,722 |
| 1,000 | 19,468,141 | 10,211,722 |

Ratios for each step: stock 3.90 (500/250), 2.23 (750/500, quadratic prediction 2.25), 1.77 (1000/750, prediction 1.78). serde-bytes: 3.85, 2.22, 1.77.

Derived average per epoch in a re-add welcome, from the totals: about 78 B for stock and about 40 B for serde-bytes (500 re-adds, average 501 epochs each).

### Cross-decode (run seed 1)

| welcome | encoded by | decoded by | result |
|---|---|---|---|
| K = 1 joiner, PreKey | stock | serde-bytes | `Err("incompatible CBOR type Array")` |
| K = 1 joiner, PreKey | serde-bytes | stock | `Err("expected array, got Bytes")` |
| re-add, HPKE | stock | serde-bytes | `Ok(())` |
| re-add, HPKE | serde-bytes | stock | `Ok(())` |

The PreKey failures happen at the outer DM decode, so the welcome cannot be read on the other build.

The HPKE results prove only the outer decode. `decode_cbor` reads the `DirectMessage`, and the inner `TwoPartyPlaintext` is decrypted before its own decode. This run does not exercise that step.

## What was NOT tested, and why

1. **Receiver processing and ordering across the group.** Bob and Carol never process messages, and the joiner processes only its own welcome. The welcome size does not depend on them in this scenario. A full-group run would also be needed to check from_join's withholding rule and the ordering of the add message.
2. **Application-message decryption by the joiner for recent N.** The joiner check compares secret IDs only.
3. **The inner `TwoPartyPlaintext` across builds.** The HPKE cross-decode above is outer only. Testing the inner layer needs Carol's full 2SM state on the other build.
4. **Single-annotation builds.** I did not build a variant with only one of the two fields annotated. By arithmetic, either single annotation should leave about 76 B per epoch, which is half the saving. This is inferred, not measured.
5. **Product DGM and orderer.** The DGM is the crate's `TestDgm`, whose history is 29 B for three members and does not grow with K. The orderer is the test orderer. Both need `test_utils`, which product code must not enable (task-05, dependencies). A product DGM would change the 29 B constant, not the per-epoch slope, because the slope comes from the bundle.
6. **Groups larger than three.** The history part grows with members, and I did not measure that.
7. **Other churn patterns, and K above 1,000.** Only alternating update and remove-then-re-add was run.
8. **The ordering message** that carries the welcome (control message, dependencies, signature). Not included in any size here.
9. **The from_join `update`** that follows an empty-bundle welcome. Its size and timing were not measured.
10. **Timing quality.** This is a shared machine, with release builds, no CPU pinning and no isolation. The medians come from 50 runs per run seed. Treat the millisecond figures as orders of magnitude.
11. **Intermediate K across runs.** The five-run table covers K = 1,000 only. The per-K tables are from run seed 1.

## Verdicts

**Q1, welcome size per epoch and at K = 1,000.** Linear growth per welcome. Stock: 146.6 B per epoch and 147.4 KB at K = 1,000. serde_bytes: 40.0 B per epoch and 40.5 KB. The per-welcome residual is at the noise level, `history` is constant at 29 B, and the increments between samples are flat. The one super-linear quantity is the cumulative size of churn welcomes, which grows about 4× for each doubling of K.

**Q2, recent N.** The API allows it without any crate change. `Dcgka::add` takes the bundle as a caller argument, and `SecretBundle::from_secrets` is public. `EncryptionGroup::add` always passes the full bundle, so an app needs to call `Dcgka::add` and do the local steps itself, as S3's `add_from_join` does. At K = 1,000 the sizes are 747 / 1,775 / 5,323 B (stock) and 489 / 771 / 1,732 B (serde_bytes) for N = 1 / 8 / 32. The joiner learns exactly those secrets.

**Q3, time at K = 1,000.** Stock takes about 3.7 ms to produce welcome bytes (1.76 ms to build, 1.96 ms to encode). serde_bytes takes about 0.5 ms. Size is the constraint, not time.

**Design implications (my reading).**

- The serde_bytes fix cuts a joiner welcome by about 3.6× (147 KB to 40 KB at K = 1,000). It also changes the wire format of every 2SM ciphertext, and a PreKey welcome does not decode across the two builds. Ship it as a new scheme version (task-05 item 6), not as a silent patch. A decoder that accepts both forms would allow a two-step rollout. I did not build or test one.
- `full` costs 147 KB per new joiner on stock encoding at K = 1,000. The fix reduces that, but it does not change the quadratic growth of churn cost. A `recent:N` option is what bounds churn, so the data supports having it before large groups with frequent removals use `full`.

## Surprising

1. The serde_bytes "patched" path copy is not patched on the welcome fields. S3 said so, but the brief and the review treat it as built. The serde_bytes numbers here are the first measured ones.
2. The fix is wire-incompatible for PreKey welcomes, in both directions.
3. A re-add is about half the cost per epoch in stock, because the HPKE ciphertext already uses `serde_bytes`. A removed-then-re-added member costs about 78 B per epoch, against about 147 B for a new joiner. This is derived from totals.
4. Stock sizes differ by about 120–170 B between runs at K = 1,000, even with the same seed. Wall-clock timestamps and `HashMap` order change the bytes that the integer-array encoding then sizes. The serde_bytes size is exact.
5. Churn is quadratic. With alternating update and remove-then-re-add, the re-add welcomes at K = 1,000 total 19.5 MB in stock.

## Files

- `Cargo.toml`, `Cargo.lock` (copied from the spikes workspace; cargo added `p2panda-encryption-found`)
- `src/main.rs`: three builds and the cross-decode. `src/harness.rs`: the scenario, included once per build.
- `variants/serde-bytes/p2panda-encryption`: the annotated copy. `variants/found/p2panda-encryption`: the renamed copy.
- `summarize.sh` → `logs/summary.txt`
- `logs/run-seed{1..5}.txt`: the per-run tables, joiner checks and cross-decode.
- `logs/welcome-k1-*.diag`, `logs/readd-first-*.diag`: CBOR diagnostics for the K = 1 joiner welcome and the first re-add welcome.
- `logs/variant-diff-serde-bytes-vs-s3-path.diff`: the two annotation lines.
