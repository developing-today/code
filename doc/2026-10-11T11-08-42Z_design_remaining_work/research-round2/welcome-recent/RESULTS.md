# Welcome size and joiner history: `recent:N` against `full`

- Date: 2026-10-11. Folder: `/home/user/work/research-round2/welcome-recent/`.
- Harness: copied from S12 (`src-s12/`, `s12-logs/`, `s12-RESULTS-copied.md`). The new harness is `src/harness.rs` and `src/main.rs`.
- Pins (BRIEF2): p2panda rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, p2panda-encryption 0.7.1, cbor-core `=0.10.1`. rustc 1.97.0, release, `--offline`, `CARGO_TARGET_DIR=/home/user/work/spikes/target`.
- Builds: stock, and serde_bytes (`variants/serde-bytes/`, the S12 annotated copy).
- Status: measured at K = 100 and K = 1,000, five seeds each. K = 10,000 is extrapolated and labelled.

Labels: **verified** = read in source, or seen in command output in this folder. **inferred** = my reasoning from verified facts. **extrapolated** = a fit through the measured K values, applied beyond them. **cited** = from an earlier spike's RESULTS, not re-run here.

## Questions (verbatim)

> n16 yes i'm fine with recent n, what does 32 mean for experience? why not 128? why not all/full?

The owner's decision, from the brief: the default joiner history is `full`, and `recent:N` is an option per world or group.

Work items from BRIEF2: reuse S12's scenario at K = 100 and K = 1,000; stock and serde_bytes; `full` and `recent:N` for N = 1, 8, 32, 128, 512; per case, the welcome bytes, the build time and the joiner's check (pass and fail counts, the error for an older message, and what the adapter needs); extrapolate to 10,000 rotations, labelled; explain a rotation in the crate, count the epochs a join, a removal and an explicit update add, and say what 32 means for frequent-join and rare-join groups.

## Short answers

**What 32 means for experience.** A joiner given `recent:32` can read every message encrypted under the 32 newest group secrets. That is every message sent since the 32nd most recent rotation began. Older messages cannot be decrypted: `receive` returns an error for each one (§E5, verified). The window is counted in epochs, not in joins, messages or days. A rotation in the crate is a call that mints a new group secret: `create`, `update` and `remove` do, and `add` does not (verified, group.rs:96, 113–131, 144, 163). So a join never moves the window. Only removals and explicit updates do. How long the window lasts in time is therefore set by the rotation rate, not the join rate (§Experience).

**Why not 128.** The measurements do not make 128 unaffordable. At K = 1,000 a 128-epoch welcome is 19.3 KB on stock encoding and 5.6 KB with serde_bytes (verified, Table A). Against 32 it costs about 3.2× (serde_bytes) to 3.6× (stock) per join and per re-add, and it gives a window 4× longer. Choosing 32 is a product judgement about how much history a new member should see. The data does not single out 32 as an optimum. My recommendation: 32 as the opt-in value, and 128 for worlds with frequent removals that want a longer window.

**Why not all, or `full`.** `full` grows linearly per join and quadratically in churn, with no bound. At K = 1,000 a join welcome is 147.4 KB on stock encoding and 40.5 KB with serde_bytes (verified). At K = 10,000 it is about 1.47 MB and 0.40 MB (extrapolated). A group that re-adds members pays 19.5 MB (stock) and 10.2 MB (serde_bytes) in re-add welcomes by K = 1,000 (verified), and about 1.93 GB and 1.00 GB by K = 10,000 (extrapolated). If a welcome must fit in one 1 MiB transport frame (design main doc line 519), stock `full` stops fitting at about 7,150 rotations and serde_bytes at about 26,200 (extrapolated, inferred). `full` is fine while epochs stay in the hundreds. `recent:N` is what bounds the cost, and serde_bytes moves the size cliff about 3.7×.

**Epochs per operation.** In the S12 scenario, one join by `EncryptionGroup::add` adds 0 epochs. One removal adds 1. One explicit update adds 1. A re-add of a removed member adds 0, and the removal before it counted 1. The `from_join` path (empty welcome, then an immediate update) adds 1 in total, from the update (verified, §E1 op counts). After K rotations the admin holds K + 1 epochs: 101 at K = 100 and 1,001 at K = 1,000 (verified).

**Frequent joins against rare joins.** A group with frequent joins and few removals does not move its window. Its joiners get effectively the full history until 31 rotations have happened. For a quiet group that can be years, so `recent:32` is effectively `full` there, and each join stays at 5.3 KB (stock) or 1.7 KB (serde_bytes). A group with rare joins and frequent removals has a short window: at one removal a day, 32 epochs is 32 days. One caveat: if a world uses the `from_join` pattern, where every join is followed by an update (task-05 item 3), each join is a rotation. Then `recent:32` means the last 32 joins (§Experience).

**Joiner check, and the error for an older message.** The check covered 66,120 message checks across both builds, K ∈ {100, 1,000}, all six modes and five seeds. Of these, 20,260 decrypted as expected (the newest N epochs, each to its own text) and 45,860 were refused as expected with `UnknownGroupSecret`. There were 0 failures, and all 120 welcomes were processed (verified, §E2). When the joiner needs an older message, `EncryptionGroup::receive` returns `GroupError::UnknownGroupSecret(hex)`, displayed as "tried to decrypt message with an unknown group secret: <hex>" (verified, §E5). `receive` takes the state by value and returns no state on error (verified, group.rs:179–182, 464–465). So a call on the real state would lose the whole group state, including the orderer and the bundle (by signature; not run, see Not verified 4). The adapter must do three things. It must read `group_secret_id` from each application message and check it against the bundle before calling `receive`. It must keep refused messages in its own store, not in the crate's queue. And it must hold every message received before the welcome outside the crate until the welcome is processed. Without the last rule, `recent:N` with N ≤ K makes the welcome itself fail: 80 of 120 pre-welcome runs failed, exactly the cases with N ≤ K (verified, §E6).

**Extrapolation to 10,000 rotations (extrapolated).** A full join is 1.47 MB on stock encoding and 0.40 MB with serde_bytes, and it builds in about 12.8 ms and 2.2 ms. Every `recent:N` welcome is flat in K at its K = 1,000 value: 0.75 KB to 75.6 KB on stock, 0.49 KB to 20.9 KB on serde_bytes. Its build time stays at 0.3 to 1.0 ms. Re-add churn at 10,000 rotations is in the extrapolation table (§E9).

## Experience analysis

### What a rotation is in the crate

The crate has no "epoch" type. A group secret is a `GroupSecret`, and a `GroupSecretId` is its SHA-256 (verified, group_secret.rs:26, 66–68). An application message names the id of the secret it is encrypted under (verified, traits/message.rs:40–45). I call one group secret one epoch. A rotation is any call that mints a secret with `SecretBundle::generate`. `create` (group.rs:96), `remove` (group.rs:144) and `update` (group.rs:163) do. `add` does not: it passes the existing bundle to `Dcgka::add` (group.rs:113–131). So a join adds nothing to the bundle, and the welcome carries the bundle as it stands (verified by reading, and by the op counts in §E1).

### Epochs per operation

| operation (S12 scenario, seed 1) | secrets before | secrets after | epochs added |
|---|---|---|---|
| create (group of three) | 0 | 1 | 1 |
| add a new member (`EncryptionGroup::add`) | 1 | 1 | **0** |
| remove a member | 1 | 2 | **1** |
| explicit update | 2 | 3 | **1** |
| re-add the removed member | 3 | 3 | 0 |
| from_join welcome (empty bundle) | 3 | 3 | 0 |
| update right after from_join | 3 | 4 | 1 |

These are the `OP|` lines in `logs/run-seed1.txt`, lines 1–16. The serde_bytes build gives the same counts.

In the S12 loop, each odd rotation is one update (1 epoch), and each even rotation is one removal plus one re-add (1 epoch). So K rotations give K + 1 epochs, and K/2 re-adds (verified: `RUN` lines, "epochs 1001, re-adds 500").

### What 32 means, by kind of group

Let r be the rotation rate (removals and explicit updates per unit time). A window of N epochs covers roughly the last N intervals between rotations, so its length in time is about N / r. Joins do not appear in r for the default `full` and `recent:N` paths. The rates below are illustrative assumptions, not measurements.

| rotation rate | N = 8 | N = 32 | N = 128 | N = 512 |
|---|---|---|---|---|
| 1 per hour (24 per day) | 8 h | 32 h (1.3 d) | 5.3 d | 21 d |
| 10 per day | 19 h | 3.2 d | 13 d | 51 d |
| 1 per day | 8 d | 32 d | 128 d (4.2 mo) | 512 d (1.4 y) |
| 1 per week | 8 wk | 32 wk (7.4 mo) | 128 wk (2.5 y) | 512 wk (9.8 y) |
| 1 per month | 8 mo | 32 mo (2.7 y) | 128 mo (10.7 y) | 512 mo (42.7 y) |

- **Frequent joins, rare removals (a quiet group).** Joins add no epochs, so 300 joins a year with 4 removals a year gives 4 epochs a year. `recent:32` then reaches back about 8 years. The window is effectively the full history, and each join costs at most 5.3 KB (stock) or 1.7 KB (serde_bytes). Frequent joins cost nothing extra in window terms.
- **Rare joins, frequent removals (a churn-heavy group).** With one removal a day, a 32-epoch window is 32 days. A joiner sees about a month of messages and nothing older. The join itself is still cheap: 5.3 KB (stock) or 1.7 KB (serde_bytes).
- **The re-add cost is the one that scales with churn.** Each removal and re-add pair sends a welcome of about N secrets to the re-added member. Under `recent:N` that is about 77 B per secret on stock, and about 40 B on serde_bytes (derived from §E4 and §E9). At one such pair a day, N = 128 costs about 10 KB a day on stock and 5.5 KB on serde_bytes. Under `full` the same pair costs more every day, because the bundle keeps growing.
- **from_join caveat.** Under the `from_join` pattern from task-05 item 3, each join is followed by an update, so each join is a rotation. The window is then the last N joins, and 32 is a short window for a join-heavy world. This pattern is separate from the `full` and `recent:N` options, so it needs its own decision.
- **What the joiner experiences.** A message older than the window is not readable. The client should show it as unavailable, not drop it. Without the adapter rules in §Short answers, the same message can also stop the welcome from processing.

## Evidence

### E1. Build, runs and op counts

Commands (all exit 0; stderr empty for every run):

```
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
cd /home/user/work/research-round2/welcome-recent
"$CARGO" build --release --offline                       # logs/build-release.txt
for s in 1 2 3 4 5; do "$CARGO_TARGET_DIR/release/welcome-recent" --seed $s --k 100,1000 > logs/run-seed$s.txt; done
./summarize.sh                                           # logs/summary.txt
python3 -I analysis/tables.py; python3 -I analysis/churn_model.py; python3 -I analysis/extrap_table.py
```

Load average was 2.6 at the start of the sweep and 5.5 at the end (`uptime`), on a shared 16-core machine. Timings are order-of-magnitude figures, not benchmarks.

The op counts are in §Epochs per operation above.

### E2. Joiner check: counts

Each welcome was processed by the joiner with its own keys. Then every epoch's message (K + 1 per run) went to a clone of the joiner's state through `EncryptionGroup::receive`. A message the joiner should decrypt must return one application output with that epoch's text. A message it should not decrypt must return `UnknownGroupSecret` with that message's own secret id.

Totals from the `ROW` lines (`analysis/extrapolate.py`, output in this folder):

```
ROW lines: 120; joiner checks: total 66120, pass 66120, fail 0; welcomes processed 120/120
decrypted as expected: 20260; refused as expected: 45860
```

The 20,260 decryptions equal the sum of min(N, K + 1) over all 120 `ROW` lines (the per-mode breakdown is in Table A).

### E3. Table A: welcome, build time and joiner check

Five seeds. Mean (sd). "build µs" is `Dcgka::add` on a clone, the median of 25 runs at K = 100 and 50 at K = 1,000, averaged over seeds, as in S12. "encode µs" is `encode_cbor` of the welcome. "pre-welcome OK" is §E6.

| build | K (epochs) | history | bundle (secrets) | welcome B | build µs (median of reps) | encode µs | decrypt, expected | refused, expected | fail | welcome processed | pre-welcome delivery OK |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stock | 100 (101) | full | 101 | 15,423.4 (55.2) | 434 (52) | 198.8 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| stock | 100 (101) | recent:1 | 1 | 750.4 (6.7) | 303 (38) | 7.4 | 1 of 101 | 100 | 0 | 5/5 | 0/5 |
| stock | 100 (101) | recent:8 | 8 | 1,777.6 (8.0) | 301 (30) | 21.3 | 8 of 101 | 93 | 0 | 5/5 | 0/5 |
| stock | 100 (101) | recent:32 | 32 | 5,295.4 (33.7) | 344 (39) | 64.5 | 32 of 101 | 69 | 0 | 5/5 | 0/5 |
| stock | 100 (101) | recent:128 | 101 | 15,404.2 (47.3) | 404 (12) | 203.2 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| stock | 100 (101) | recent:512 | 101 | 15,427.0 (34.2) | 446 (76) | 213.2 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| stock | 1000 (1001) | full | 1001 | 147,403.2 (68.2) | 1,561 (173) | 1,970.4 | 1001 of 1001 | 0 | 0 | 5/5 | 5/5 |
| stock | 1000 (1001) | recent:1 | 1 | 747.0 (4.2) | 307 (51) | 8.3 | 1 of 1001 | 1000 | 0 | 5/5 | 0/5 |
| stock | 1000 (1001) | recent:8 | 8 | 1,766.0 (14.6) | 339 (59) | 20.3 | 8 of 1001 | 993 | 0 | 5/5 | 0/5 |
| stock | 1000 (1001) | recent:32 | 32 | 5,299.2 (31.2) | 337 (26) | 69.3 | 32 of 1001 | 969 | 0 | 5/5 | 0/5 |
| stock | 1000 (1001) | recent:128 | 128 | 19,325.2 (32.8) | 461 (65) | 262.8 | 128 of 1001 | 873 | 0 | 5/5 | 0/5 |
| stock | 1000 (1001) | recent:512 | 512 | 75,647.8 (88.8) | 977 (138) | 1,038.0 | 512 of 1001 | 489 | 0 | 5/5 | 0/5 |
| serde-bytes | 100 (101) | full | 101 | 4,492 (0) | 320 (44) | 2.0 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| serde-bytes | 100 (101) | recent:1 | 1 | 489 (0) | 302 (44) | 1.8 | 1 of 101 | 100 | 0 | 5/5 | 0/5 |
| serde-bytes | 100 (101) | recent:8 | 8 | 771 (0) | 277 (11) | 1.7 | 8 of 101 | 93 | 0 | 5/5 | 0/5 |
| serde-bytes | 100 (101) | recent:32 | 32 | 1,732 (0) | 278 (8) | 2.1 | 32 of 101 | 69 | 0 | 5/5 | 0/5 |
| serde-bytes | 100 (101) | recent:128 | 101 | 4,492 (0) | 297 (21) | 1.9 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| serde-bytes | 100 (101) | recent:512 | 101 | 4,492 (0) | 302 (23) | 2.1 | 101 of 101 | 0 | 0 | 5/5 | 5/5 |
| serde-bytes | 1000 (1001) | full | 1001 | 40,493 (0) | 494 (48) | 3.7 | 1001 of 1001 | 0 | 0 | 5/5 | 5/5 |
| serde-bytes | 1000 (1001) | recent:1 | 1 | 489 (0) | 309 (42) | 2.0 | 1 of 1001 | 1000 | 0 | 5/5 | 0/5 |
| serde-bytes | 1000 (1001) | recent:8 | 8 | 771 (0) | 319 (41) | 1.9 | 8 of 1001 | 993 | 0 | 5/5 | 0/5 |
| serde-bytes | 1000 (1001) | recent:32 | 32 | 1,732 (0) | 324 (35) | 2.0 | 32 of 1001 | 969 | 0 | 5/5 | 0/5 |
| serde-bytes | 1000 (1001) | recent:128 | 128 | 5,572 (0) | 350 (52) | 2.1 | 128 of 1001 | 873 | 0 | 5/5 | 0/5 |
| serde-bytes | 1000 (1001) | recent:512 | 512 | 20,933 (0) | 426 (62) | 2.7 | 512 of 1001 | 489 | 0 | 5/5 | 0/5 |

Notes:
- Stock welcome sizes vary between runs with the same seed. The sd column is 4 to 90 B for stock and 0 for serde_bytes. That is the wall-clock timestamp and `HashMap` order effect that S12 found (S12 RESULTS, Method).
- At K = 100, `recent:128` and `recent:512` hold the full bundle (101 secrets), so their rows equal `full`.
- Stock welcome at K = 1,000, mean of five seeds: 147,403 B. S12's five-run mean for the same quantity: 147,362 B (cited, S12 RESULTS, five-run table). The serde_bytes value is 40,493 B in both (exact).

### E4. Table B: re-add (churn) welcomes

Every re-add welcome in a run was built under each mode from the same admin state, so the bundle for each mode is the only difference. Totals are over all re-adds in the run.

| build | history | K=100 (50 re-adds) | K=1000 (500 re-adds) |
|---|---|---|---|
| stock | full | 217,311 (478) | 19,493,312 (15,782) |
| stock | recent:1 | 20,841 (7) | 208,832 (21) |
| stock | recent:8 | 47,191 (59) | 478,416 (234) |
| stock | recent:32 | 123,037 (235) | 1,386,018 (927) |
| stock | recent:128 | 217,311 (478) | 4,791,691 (3,629) |
| stock | recent:512 | 217,311 (478) | 14,871,662 (11,382) |
| serde-bytes | full | 121,076 (0) | 10,211,722 (0) |
| serde-bytes | recent:1 | 18,939 (0) | 189,862 (0) |
| serde-bytes | recent:8 | 32,677 (0) | 330,500 (0) |
| serde-bytes | recent:32 | 72,076 (0) | 802,349 (0) |
| serde-bytes | recent:128 | 121,076 (0) | 2,572,589 (0) |
| serde-bytes | recent:512 | 121,076 (0) | 7,810,722 (0) |

The stock `full` total at K = 1,000 is 19.49 MB here, against 19.47 MB in S12 (cited). Both are the same scenario.

### E5. The error for an older message (verbatim)

From `logs/run-seed1.txt`, K = 1,000:

```
ERR|stock|1|1000|recent:32|UnknownGroupSecret(aa32e0362353c54c4dcebdf73c1c2563ed101291b83f81c9314bd45a215b7ff9)
PRE|stock|1|1000|recent:32|welcome fails: tried to decrypt message with an unknown group secret: aa32e0362353c54c4dcebdf73c1c2563ed101291b83f81c9314bd45a215b7ff9
```

The first refused message is epoch 0, the oldest, whose secret the bundle lacks. The variant is `GroupError::UnknownGroupSecret(String)` (group.rs:519–520). The string is the hex id, produced in `decrypt` (group.rs:456–457, `hex::encode(group_secret_id)`). Every refusal in the sweep named the message's own secret id: `check()` counts a mismatch as a failure, and the sweep had 0 failures.

### E6. Pre-welcome delivery

The joiner receives all K + 1 application messages before its welcome, as a sync would deliver history. Then it receives the welcome.

- `full`: the welcome succeeds, and all messages decrypt (in all 20 cases: 2 builds × 2 K × 5 seeds).
- `recent:N` with N ≤ K: the welcome fails. `receive` queues the older messages before the welcome (group.rs:209), and after the welcome `process_ready` decrypts each queued message with `?` (group.rs:262–268). The first one without a secret aborts the call. This is S3's c1 hazard, now with recent:N (cited, S3 RESULTS lines 60–61).
- `recent:N` with N > K: the bundle holds everything, so the welcome succeeds.

Counts: 40 of 120 runs OK, 80 failed. Every failure is `recent:N` with N ≤ K, and no `recent:N` with N ≤ K succeeded.

### E7. Crate behaviour (source)

- `EncryptionGroup::receive(y, message) -> GroupResult<...>`, with `GroupResult<T> = Result<(GroupState, T), GroupError>` (group.rs:179–182, 464–465). On `Err` there is no state (verified).
- `receive` queues the message before it decrypts (group.rs:209). Before the welcome, application messages are queued and not processed (group.rs:212–217). On the welcome, controls are processed first, then application messages (group.rs:233–268).
- `decrypt` looks the id up in `secrets` and returns `UnknownGroupSecret` if absent (group.rs:456–457).
- The test orderer marks a message ready only when every named dependency is ready (`test_utils/ordering.rs:144–151`; `ordering.rs:102–106`). Its `previous` field is updated for control messages only (`test_utils/ordering.rs:98`, `:188`), and the crate's rules name control messages as dependencies (`traits/ordering.rs:37–39`). From that I infer that application messages are never dependencies, so a message the adapter holds outside the crate blocks nothing else (inferred, not run).
- `SecretBundleState::contains`, `get`, `ids`, `secrets` (group_secret.rs:124–154) are public. `SecretBundle::from_secrets` (group_secret.rs:229), `extend` (:277) and `EncryptionGroup::update_secrets` (group.rs:316) are public too. The adapter can add secrets to a bundle without a crate change.

### E8. Consistency with S12 and S3

- serde_bytes at K = 100 and K = 1,000: full 4,492 and 40,493 B; recent:32 1,732 B. All match S12 exactly (cited: the K = 100 values from S12's seed-1 table, the K = 1,000 values from its five-run table).
- Stock per-secret slope from the two K points: 146.6 B (§E9). S12's fit over 11 samples: 146.68 B (cited).
- Re-add totals match S12 for the same scenario: serde_bytes exactly (10,211,722 B), stock within 0.2% (19.49 MB here, 19.47 MB in S12; §E4).
- S3's `full` welcome test: the joiner reads a message sent before its join (cited, S3 `run-base-final`).

The new harness has two deviations from the crate's own test setup, both needed for the joiner to be welcomed. (1) `MemberDgm`: its `from_welcome` keeps the joiner. With the stock `TestDgm` the joiner is never welcomed (cited, S3 RESULTS lines 63–69). Our adapter's DGM needs the same fix (task-05 item 1). (2) `QOrderer`: a plain queue, not the crate's `MessageOrderer`. The stock orderer would hold pre-join messages pending, because a joiner never saw the control messages they name (inferred from the source in E7; not run). Decryption is not affected: `receive` decrypts with the bundle either way.

### E9. Extrapolation to 10,000 rotations

| build | history | welcome B at K=100 | welcome B at K=1,000 | welcome B at K=10,000 (extrap.) | build µs at K=1,000 | build µs at K=10,000 (extrap.) | re-add churn B at K=1,000 | re-add churn B at K=10,000 (extrap.) |
|---|---|---|---|---|---|---|---|---|
| stock | full | 15,423 | 147,403 | 1,467,201 | 1,561 | 12.8 ms | 19,493,312 | 1,927.0 MB |
| stock | recent:1 | 750 | 747 | 747 (flat) | 307 | 307 µs (flat) | 208,832 | 2.1 MB |
| stock | recent:8 | 1,778 | 1,766 | 1,766 (flat) | 339 | 339 µs (flat) | 478,416 | 4.8 MB |
| stock | recent:32 | 5,295 | 5,299 | 5,299 (flat) | 337 | 337 µs (flat) | 1,386,018 | 14.0 MB |
| stock | recent:128 | 15,404 | 19,325 | 19,325 (flat) | 461 | 461 µs (flat) | 4,791,691 | 50.7 MB |
| stock | recent:512 | 15,427 | 75,648 | 75,648 (flat) | 977 | 977 µs (flat) | 14,871,662 | 193.8 MB |
| serde-bytes | full | 4,492 | 40,493 | 400,503 | 494 | 2.2 ms | 10,211,722 | 1,002.2 MB |
| serde-bytes | recent:1 | 489 | 489 | 489 (flat) | 309 | 309 µs (flat) | 189,862 | 1.9 MB |
| serde-bytes | recent:8 | 771 | 771 | 771 (flat) | 319 | 319 µs (flat) | 330,500 | 3.3 MB |
| serde-bytes | recent:32 | 1,732 | 1,732 | 1,732 (flat) | 324 | 324 µs (flat) | 802,349 | 8.1 MB |
| serde-bytes | recent:128 | 4,492 | 5,572 | 5,572 (flat) | 350 | 350 µs (flat) | 2,572,589 | 27.2 MB |
| serde-bytes | recent:512 | 4,492 | 20,933 | 20,933 (flat) | 426 | 426 µs (flat) | 7,810,722 | 101.5 MB |

Frame check (inferred: one welcome per 1 MiB frame, `design…md:519`):
- stock: full welcome passes 1,048,576 B at about 7,146 secrets, i.e. about 7,145 rotations
- serde-bytes: full welcome passes 1,048,576 B at about 26,202 secrets, i.e. about 26,201 rotations

Method, all extrapolated. Full-history welcome bytes and build and encode times are straight lines in the number of secrets (K + 1), through the K = 100 and K = 1,000 means. Stock: 146.6 B per secret, intercept 612 B. serde_bytes: 40.0 B per secret, intercept 452 B. Recent-N values are flat in K, because the bundle is capped at N. Re-add churn uses a per-re-add model: re-add i at rotation 2i carries min(N, 2i + 1) secrets, and a re-add welcome of s secrets is c0 + c1·s bytes. Fitting c0 and c1 to the K = 100 and K = 1,000 totals gives c0 ≈ 340 B and c1 ≈ 77 B (stock) or 40 B (serde_bytes) for full, recent:32, recent:128 and recent:512 on both builds. Those four modes were fitted separately, and they agree, which is some evidence that the model holds. recent:8 deviates (c0 316 B and 293 B), because at N = 8 the transient is a larger share of the total. The fit has two parameters and two points, so it is not an independent check against the data. The K = 10,000 values are extrapolations, not measurements.

The 1 MiB frame crossing is inferred: it assumes a welcome travels in one transport frame (`MAX_FRAME_BYTES` = 1 MiB, design lines 61 and 519). I did not check that against the code.

## Options

Values from Table A and Table B (K = 1,000, five-seed means). Window at one rotation a week is approximate (§Experience).

| history | join welcome, stock (K = 1,000) | join welcome, serde_bytes (K = 1,000) | re-add churn total, stock / serde (K = 1,000) | window at 1 rotation a week | assessment |
|---|---|---|---|---|---|
| `full` (default, owner) | 147.4 KB | 40.5 KB | 19.5 MB / 10.2 MB | all history | Grows without bound: 1.47 MB per join at 10,000 (extrap.), past a 1 MiB frame at about 7,150 rotations on stock (inferred) |
| `recent:512` | 75.6 KB | 20.9 KB | 14.9 MB / 7.8 MB | 9.8 years | Too large a default on stock encoding (76 KB per join) |
| `recent:128` | 19.3 KB | 5.6 KB | 4.8 MB / 2.6 MB | 2.5 years | Affordable. The option for churn-heavy worlds that want a longer window |
| `recent:32` | 5.3 KB | 1.7 KB | 1.4 MB / 0.8 MB | 7.4 months | Recommended opt-in value |
| `recent:8` | 1.8 KB | 0.8 KB | 0.5 MB / 0.3 MB | 8 weeks | Short for most worlds |
| `recent:1` | 0.7 KB | 0.5 KB | 0.2 MB / 0.2 MB | about 1 week | Only the current epoch |

## Recommendation

1. **Keep `full` as the default**, as decided. It is cheap while epochs stay in the hundreds (about 45 KB on stock at 300 epochs, interpolated from the linear fit). It is also the behaviour the crate gives natively.
2. **For worlds that opt into `recent:N`, use N = 32.** A join costs at most 5.3 KB on stock or 1.7 KB on serde_bytes, and that does not grow with K (verified: welcome size and build time are flat from K = 100 to K = 1,000). The re-add cost stays bounded, at about N × 77 B per re-add on stock.
3. **Offer 128 as an explicit value** for churn-heavy worlds that want a longer window. It costs about 3.2× (serde_bytes) to 3.6× (stock) more per join and per re-add than 32.
4. **Do not make 512 a default.** On stock encoding a join is 76 KB, and on serde_bytes it is 21 KB.
5. **The bigger lever is serde_bytes as a new scheme version** (N10, N20). It cuts the full-history welcome by 3.6× (147 KB to 40 KB at K = 1,000), and the recent:N welcomes by 1.5× (N = 1) to 3.6× (N = 512). It moves the full-history size cliff from about 7,150 to about 26,200 rotations (extrapolated, inferred). Without it, `full` is impractical past a few thousand rotations.
6. **Adapter rules, whatever N is.** (a) Never call `receive` on an application message whose `group_secret_id` is not in the bundle. Keep it in the adapter's store. (b) Hold all pre-welcome messages outside the crate. After the welcome, deliver only the in-bundle ones. (c) Older secrets can be added later with `SecretBundle::extend` or `update_secrets`. The transport for them is a protocol we still have to design.

**What would change this:**
- A time-based window (for example, "the last 90 days") instead of an epoch count. Epochs are irregular in time, so a time window would need a rotation schedule (N7).
- Joins that rotate (the `from_join` pattern). Then the window is a number of joins, and 32 is a short window for join-heavy worlds.
- A welcome size limit below about 75 KB. If welcomes must fit an inline-payload bound like the 64 KiB one for documents (design main doc line 406), 512 fails on stock and 128 passes. Whether that bound applies to welcomes is not checked.
- Groups above three members. The DGM part grows with members, which I did not measure.
- Measured re-add rates in real worlds. Frequent re-adds make the per-re-add cost dominate, which argues for 32 over 128.

## Design-doc discrepancies found while reading

- Index N16 (`2026-10-11T11-08-42Z_design_remaining_work.md` line 104) says 32 epochs is "about 4.7 KB today". Measured: 5,299 B on stock and 1,732 B on serde_bytes (K = 1,000, five-seed mean). The 4.7 KB was an estimate made before S12 measured it.
- Index row 7 (line 63) and task-05 line 58 give `recent:N`, N = 32, as the preferred default. The decision log (`2026-10-11T03-44-33Z_design_groups_events_delegation.md` line 771, dated 2026-10-11T04-20-17Z) and task-05 line 26 say `full`. The brief also says `full`. The index and task-05 line 58 should be updated.

## Not verified

1. **Stock orderer.** I did not run a joiner with the crate's `MessageOrderer` against pre-join control messages. The claim that it would hold them pending is inferred from source (§E7).
2. **Joiner after the join.** The check does not feed the joiner later control messages. S3 showed a joiner processing a later update and reading later messages (cited, S3 `run-base-final`), but not with these harness settings.
3. **Full group.** Bob and Carol never process messages, as in S12. Receiver processing across the group was not run.
4. **Refusal on the real state.** The claim that a failed `receive` loses the group state rests on the signature (group.rs:179–182, 464–465). I did not run a call on a non-cloned state to show it.
5. **One frame.** The 1 MiB cliff assumes a welcome travels in one frame. I did not check the transport code for that.
6. **K = 10,000.** Every 10,000-rotation number is extrapolated. Nothing was run at that K.
7. **Groups above three.** The DGM history part grows with members, and I did not measure that.
8. **Timing.** Shared, lightly loaded machine. Figures are order-of-magnitude.
9. **Inline bound.** Whether welcomes are subject to a 64 KiB inline bound was not checked.
10. **Lazy history.** The cost of sending older secrets on request (task-05 `full` on request) was not designed or measured.
11. **Time windows.** The "illustrative rates" table assumes rotation rates. It is not from any real world.
12. **Re-add per-secret sizes.** c0 and c1 come from the two-point model in §E9, not from per-re-add measurements.

## Files

- `RESULTS.md` (this file); `RESULTS-body.md` is the template it was built from.
- `src/harness.rs`, `src/main.rs`: the harness. `src-s12/`: the S12 harness, as copied.
- `variants/serde-bytes/p2panda-encryption/`: the annotated copy from S12. `variants/found/` came with the S12 copy but is not built or used here.
- `logs/run-seed{1..5}.txt`: raw output. `logs/summary.txt`: `summarize.sh`. `logs/build-release.txt`: the clean release build. `logs/build-attempt1-failed.txt`: the output of my first build attempt, which failed to compile (fixed before the runs).
- `logs/probe-seed1-k100.txt`: the K = 100 probe run, before the sweep.
- `analysis/extrapolate.py`, `analysis/tables.py`, `analysis/churn_model.py`, `analysis/extrap_table.py`: the scripts behind the numbers.
- `s12-logs/`, `s12-RESULTS-copied.md`, `summarize-s12.sh`: S12's files, kept for reference.
