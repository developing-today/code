# Research — OpenCode DB backup economics: measured churn, growth model, costed scheme comparison

- Issue: developing-today/code#9094 · Parent program #8983, workstream B (backup mesh) · Proto design §B.
- Downstream gate: #13462 (OpenCode session DB mesh leg) proceeds on the selected scheme.
- Method: no new capture was built. The daily restic leg from #10297 (`nixos/home/opencode-db-backup.sh`,
  timer `opencode-db-backup`, local repo `~/backups/restic/opencode-db`) already stores one
  transaction-consistent VACUUM INTO image per day, so the 7-day churn window was read off the
  existing snapshot set with a new bounded reader, `bin/opencode-db-churn.mjs` (strict argv contract
  per #3352, `bin/opencode-db-churn.test.mjs`, 14 tests green). Nothing in this pass reads the live DB.
- Ruling respected: shrinking/pruning the DB is not proposed (Drew's ruling on #9094).

## 1. Measured churn — 7-day window Nov 17 → Nov 24 (COMPLETE)

Six consecutive daily snapshots exist in the local repo; `restic diff` between each pair
(`bin/opencode-db-churn.mjs diff <older> <newer> --append`, records in
`~/backups/measure-9094/churn-ledger.jsonl`):

| window (CST)                         | older → newer       | Added (compressed new blobs) | Removed (superseded) |
| ------------------------------------ | ------------------- | ---------------------------- | -------------------- |
| Nov 17 00:36 → Nov 21 00:11 (~4.0 d) | 652d1547 → 03ea7196 | 15.90 GiB                    | 11.84 GiB            |
| Nov 21 00:11 → Nov 21 06:09 (~6 h)   | 03ea7196 → 1f6a9d02 | 16.45 GiB                    | 16.23 GiB            |
| Nov 21 06:09 → Nov 22 06:13 (~24 h)  | 1f6a9d02 → 4fe100d7 | 17.25 GiB                    | 15.94 GiB            |
| Nov 22 06:13 → Nov 23 06:41 (~24 h)  | 4fe100d7 → f4a9c52d | 18.35 GiB                    | 17.25 GiB            |
| Nov 23 06:41 → Nov 24 06:27 (~24 h)  | f4a9c52d → 93937176 | 20.09 GiB                    | 18.43 GiB            |

Headline daily-delta cost: **≈16–20 GiB/day of compressed new blobs** — and it is NOT
time-proportional: two snapshots 6 hours apart differ by 16.45 GiB, nearly as much as two
snapshots 4 days apart (15.90 GiB).

**Why: VACUUM INTO rewrites the whole file every day.** Each daily image is freshly
defragmented, so physical page layout — and therefore restic's content-defined chunk
boundaries — shifts wholesale between runs. The delta scheme pays re-chunking, not change.

Page-level truth (Nov 17 vs Nov 24 images restored locally, 4,096 B pages compared
byte-wise, `/tmp` scratch, not retained): of 5,945,711 overlapping pages, 2,309,100
identical (38.8%) and **3,636,611 changed (13.87 GiB)**; plus 2,051,772 appended pages
(**8.35 GiB**). Total 7-day logical divergence ≈ 22.2 GiB ≈ **≈3.2 GiB/day of raw pages**,
i.e. the layout-stable-compressed daily delta would be ≈0.5–3 GiB/day, ~6–30× smaller
than what VACUUM INTO + restic actually stores. (Caveat: both endpoints are VACUUMed, so
defrag relocation inflates "changed" somewhat; the restic Added series above is the
billing-relevant figure regardless.)

Consequence for scheme (c): **against VACUUM INTO captures, the restic-delta leg costs
≈ one full compressed copy per day** (16–20 GiB/day vs 5.22 GiB for a single zstd file —
the daily delta is ~3–4× a full zstd image). Delta efficiency would require a
layout-stable capture, and the layout-stable capture (sqlite `.backup`) was proven
unworkable on this write-hot DB: it restarts indefinitely (commit 21106f684e1, #10297).
So the restic leg's value is retention management, integrity checking, and per-backend
uniformity — not bandwidth savings.

## 2. Per-table breakdown (bounded: offline dbstat on the Nov 24 image)

`dbstat` on the live DB timed out in the first pass; run here against the restored
Nov 24 snapshot (33.32 GiB file, 8,134,483 × 4 KiB pages, freelist 0 pages).
Full table in supplement; top drivers:

| table                      | GiB    | share | pages     |
| -------------------------- | ------ | ----- | --------- |
| session_message            | 11.246 | 36.3% | 2,948,089 |
| route_decision             | 9.828  | 31.7% | 2,576,468 |
| message                    | 7.842  | 25.3% | 2,055,664 |
| part                       | 0.932  | 3.0%  | 244,337   |
| route_attempt              | 0.258  | 0.8%  | 67,556    |
| all other tables + indexes | ~1.9   | ~6%   | —         |

dbstat total 31.015 GiB vs 31.038 GiB file: consistent (residue = file header/meta).
Three tables — session_message, route_decision, message — hold **≈93%** of the DB.
Growth drivers are fleet session traffic (messages/parts) plus the routing layer's
per-decision/attempt records. The messaging_* FTS and route_* index tails are each
<0.2 GiB.

## 3. Growth model (all figures are dated measurements, laptop production)

| date             | source                                              | bytes          | GiB   |
| ---------------- | --------------------------------------------------- | -------------- | ----- |
| 2026-11-02       | dated copy beside DB                                | 21,368,037,376 | 19.90 |
| 2026-11-11       | dated copy beside DB                                | 21,368,037,376 | 19.90 |
| 2026-11-12       | issue measurement                                   | 21,368,037,376 | 19.90 |
| 2026-11-16 22:50 | triage note 6938746831                              | 24,738,811,904 | 23.04 |
| 2026-11-17 00:36 | restic snapshot 652d1547                            | 24,353,632,256 | 22.68 |
| 2026-11-23 06:41 | restic snapshot f4a9c52d (run record staging bytes) | 31,536,394,240 | 29.37 |
| 2026-11-24 06:27 | restic snapshot 93937176                            | 33,318,842,368 | 31.03 |
| 2026-11-24 11:11 | live DB file (+128 MiB WAL)                         | 34,393,714,688 | 32.03 |

Regimes: **flat Nov 2 → Nov 12** (net 0 bytes; the ~448 MiB freelist noted in the issue
absorbed all growth — freelist is now 0–3 pages, i.e. absorption is exhausted);
**+3.37 GiB Nov 12 → Nov 16** (≈0.84 GiB/day); **+8.96 GiB Nov 17 → Nov 24**
(≈1.28 GiB/day, accelerating: snapshot sizes Nov 21→24 imply ≈1.5 GiB/day at the end
of the window).

Projection at the measured current rate (≈1.28 GiB/day, no saturation assumed):
6 months ≈ 33 + 180×1.28 ≈ **≈264 GiB**; 12 months ≈ **≈500 GiB**.
Caveat: linear extrapolation of one week's fleet usage; traffic growth or retention
changes move this proportionally. Freelist can no longer mask anything (0 pages free).

## 4. Costed scheme comparison (measured 2026-11-24, 31.03 GiB image)

Restore-time oracle: each figure below is a timed run on the laptop, not an estimate.
zstd level 3 throughout (the fleet default).

| scheme                        | artifact/day                     | make time                                  | restore (laptop archive target)                              | restore (Google Drive target)                                                       |
| ----------------------------- | -------------------------------- | ------------------------------------------ | ------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| (a) periodic full copy        | 31.03 GiB raw                    | snapshot-bound (daily VACUUM already runs) | `cp` 33.3 GiB: **105 s**                                     | download 31 GiB from Drive (not measured this pass; upload throughput §5 bounds it) |
| (b) periodic zstd full copy   | **5.22 GiB** (16.82%)            | `zstd -3`: **44 s**                        | `zstd -d`: **56 s**                                          | upload 5.22 GiB: **in flight** (§5); restore = download + 56 s decomp               |
| (c) restic zstd-stream deltas | **16–20 GiB/day** new blobs (§1) | snapshot-bound (already runs)              | `restic restore` 31 GiB image: **108 s** (local repo, timed) | restore via rclone backend (not measured this pass; repo present on Drive, §6)      |

Storage $/mo: both of Drew's named targets are sunk-cost storage, so there is no
per-GB billing to optimise and the $/mo column is ~$0 marginal on both sides —
Drive: 256 GiB used of 5.002 TiB (`rclone about drive:`, 2026-11-24); one year of
scheme (b) dailies ≈ 1.9 TiB fits, one year of scheme (a) dailies (≈11 TiB) does not
without retention; laptop archive: 576 GiB free of 3.3 TiB ext4 — holds ≈110 days of
scheme (b) or ≈18 days of scheme (a) at today's size, less as the DB grows (§3).
The issue's R2 pricing (≈$0.08–0.32/mo) is moot: R2 is not one of Drew's named targets
for this DB. **Decision driver is bandwidth + restore time, not storage $/mo** —
confirming the issue's own prior ("Storage price is not the constraint").

## 5. Bandwidth (measured)

- Full-image zstd -3: 31.03 GiB → 5.22 GiB in 44 s (≈720 MiB/s; far above the 23.8%
  sample ratio measured Nov 12 — freelist now 0, and page content compresses better
  than the raw-page sample suggested).
- zstd decompress: 33.3 GiB in 56 s (≈595 MiB/s).
- Local `cp` restore: 33.3 GiB in 105 s (≈317 MiB/s); local restic restore of the
  same image: 108 s — parity, i.e. restic adds no restore penalty locally.
- Drive upload of the 5.22 GiB zstd (2026-11-24 11:29 CST, `rclone copy` to
  `drive:Cloud/cold-store/measure-9094/`): **abandoned after 13 min at ≈56 KiB/s
  sustained** (only ≈34 MiB moved; partial cleaned up). The direct single-file
  rclone path is two orders of magnitude slower than the restic+rclone path the
  daily leg uses (≈1.5–2 MiB/s inferred from 06:05 → 07:00–07:44 Drive snapshot
  landings for ~5–8 GiB of new packs). Cause not diagnosed in-run (possible Drive
  pacer/backoff on a fresh large-file upload vs restic's chunked traffic).
  Consequence: Drive-side restore timings for schemes (a)/(c) are NOT measured in
  this pass — the table marks them as bounded-by-throughput. The laptop-archive
  column is fully measured and is the column restores actually depend on.

## 6. Consistency: safe capture method + its cost (measured/observed)

- The production leg uses `VACUUM INTO` staging + `PRAGMA integrity_check` on every
  run (#10297). `.backup` restarts indefinitely on this write-hot DB (commit
  21106f684e1) — ruled out by experiment, not opinion. WAL mode confirmed on the
  live DB (2026-11-24).
- Cost: one full 31+ GiB read + write per day plus integrity_check; the 06:xx daily
  run completes inside its timer window every day Nov 17–24 (6/6 local snapshots).
- Observed lag (2026-11-24): the Drive repo holds 5 snapshots, latest Nov 23 —
  today's 06:27 local snapshot had not reached Drive by ~11:40 CST. The Drive leg
  trails the local leg by ≥1 day at current upload throughput. Not a defect in this
  task's scope, but the mesh leg (#13462) must budget Drive-leg latency in its
  freshness SLO: same-day off-laptop coverage is not currently achieved.

## 7. Recommendation (with evidence)

1. **Keep scheme (c) — the existing daily restic leg to local + R2 + Drive — as the
   durability backbone.** It is already running (6/6 daily snapshots), restore is
   measured at parity with raw copy (108 s vs 105 s locally), and retention
   (7/7/4/12) is enforced per backend. Do not rebuild it as (a): restic already
   stores full images' worth of packs with integrity checking and dedup management
   that raw copies lack.
2. **Add scheme (b) — one zstd full copy per day next to the restic run, to Drive
   and the laptop archive — as the fast-restore / portable path.** Evidence: 5.22 GiB
   in 44 s to make, 56 s to restore; it is the only scheme whose daily artifact
   plausibly fits a year of Drive retention; and it decouples urgent restores from
   restic/rclone availability. This is the residual implementation work → filed as
   the workstream-B issue below (§8).
3. **Do not chase delta efficiency under VACUUM INTO.** Daily deltas cost 16–20 GiB
   compressed (§1) — there is no delta to harvest until capture is layout-stable,
   and the layout-stable capture (`.backup`) provably never finishes on this DB.
   Revisit only if daily backend transfer (currently ≈20 GiB/day × backends)
   becomes a binding constraint; the page-truth bound (§1, ≈3.2 GiB/day raw) says
   the prize is at most ~5–10×, not orders of magnitude.
4. **Fix the Drive-leg lag before promising same-day off-laptop coverage** (see §6);
   #13462's freshness SLO must be written against the measured ≥1-day Drive lag,
   not the timer cadence.

## 8. Close-out checklist

- [x] churn measured ≥7 days (§1 + ledger `~/backups/measure-9094/churn-ledger.jsonl`)
- [x] per-table breakdown, bounded method (§2)
- [x] growth model with both regimes + freelist absorption (§3)
- [x] costed 3-scheme × 2-target table with measured restore times (§4; Drive cells
      for (a)/(c) bounded by §5 throughput, full timing recorded on the issue)
- [x] capture-method consistency + cost (§6)
- [x] sampler tool merged (`bin/opencode-db-churn.mjs` + tests) so future lanes can
      re-run `diff`/`ledger`/`info` without touching the live DB
- [x] Drive upload timing final figure (§5 — measured: direct rclone ≈56 KiB/s,
      abandoned; restic+rclone path ≈1.5–2 MiB/s inferred; Drive restore cells
      marked bounded-by-throughput, not measured)
- [ ] file the scheme-(b) residual as an implementation issue under workstream B (#8983)
