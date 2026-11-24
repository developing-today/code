# Supplement: benchmark extracts

Gathered 2026-11-23/24. Each run is recorded with its own conditions; runs
are not comparable across rows unless the source itself ran them head-to-head.

## A. Independent single-node head-to-head (iamkunal9, published 2026-10-19)

Source: https://iamkunal9.in/blog/minio-alternatives-rustfs-vs-seaweedfs-benchmark
Conditions: RustFS 1.0.3 vs SeaweedFS 4.48 (`weed mini`) vs MinIO last
community release; identical single-node conditions; defaults only; median
of 5 runs.

Throughput (MiB/s):

| Workload | RustFS | SeaweedFS | MinIO |
| --- | --- | --- | --- |
| 64 KiB PUT | 56.8 | 99.4 | 49.7 |
| 64 KiB GET | 149.5 | 114.0 | 121.7 |
| 8 MiB PUT | 325.5 | 106.9 | 318.8 |
| 8 MiB GET | 455.8 | 410.3 | 426.6 |
| 256 MiB multipart PUT | 276.4 | 125.7 | 323.8 |
| 256 MiB GET | 1,036.3 | 580.8 | 1,279.6 |

p99 latency: 64 KiB PUT — SeaweedFS 39.3 ms, RustFS 61.6 ms. 8 MiB PUT —
RustFS 158.3 ms, SeaweedFS 580.5 ms.

Resource use: SeaweedFS peak ~1.4 GiB RAM (~4.3× RustFS's 348 MiB; MinIO
394 MiB); SeaweedFS used ~1.6× RustFS's CPU time in every run.

Durability: zero acknowledged objects lost or corrupted across 6 `kill -9`
cycles per store (26,111 writes total).

S3 compatibility spot-checks: each store passed 14/15. RustFS and SeaweedFS
both drop metadata signed into presigned PUT URLs; MinIO lacks per-bucket
CORS.

## B. Community 4-node warp comparison (rustfs/rustfs Discussion #1500, crawled 2026)

Conditions: 4 nodes × 32 cores / 128 GB / 1 SATA SSD; MinIO
RELEASE.2024-10-29 vs RustFS 1.0.0-alpha.79 vs SeaweedFS 4.02; dual-replica
writes, cold-data EC; `warp mixed`.

- GET, small sizes: SeaweedFS far ahead — 4 KiB ≈ 75.9–112.4 MiB/s vs RustFS
  ≈ 12.6; 32 KiB ≈ 497–920 vs ≈ 100.
- GET, large sizes: converge ≈ 1.8–1.9 GiB/s for both.
- PUT, small sizes: RustFS very low (≈ 4.2 MiB/s at 4 KiB).
- RustFS crashed at 512 concurrency on 1 MiB objects (could not upload) —
  alpha build; dated evidence only.

## C. RustFS vendor numbers

- Original "2.3× MinIO" claim: project benchmark, 2-core Xeon Platinum
  8475B, 4 GB RAM, 15 Gbps network, 4× 40 GB NVMe, 4 KB small objects.
  Vendor-side.
- Beta.10 blog (build dated 2026-07-17): Azure, 4 nodes × 4 disks (300 GB,
  32k IOPS), 8 CPU / 16 GB, warp 64 concurrent × 5 min — PUT beats MinIO at
  all 11 sizes (up to 2.85×); GET gap "narrowing". Independent summaries
  (LLMS3) note a mid-2025 independent test with MinIO at 53 Gbps vs RustFS
  23 Gbps on 20 MiB reads.
- Vendor cluster claim: 323 GB/s read / 183 GB/s write — evaluation testing,
  not third-party.

## D. SeaweedFS project numbers (labelled unscientific by the project)

- MacBook SSD, `weed benchmark`, 1M × 1 KB files, concurrency 16: write
  15,708/s (p50 0.8 ms, p99 2.6 ms); random read 47,019/s (p50 0.3 ms,
  p99 0.7 ms).
- Local warp mixed: total 3,302.88 MiB/s (GET 2,477 MiB/s at 10 KB-class
  objects).

## E. First-hand single-node warp run (dev.to, RustFS 1.0.1)

Single node, Ryzen 7 5800H laptop, loopback, warp 1.3.1: 1 MiB GET
663 MiB/s avg, PUT 102 MiB/s; 10 KiB GET 13,965 obj/s, PUT 360 obj/s
(median latency 21 ms). Loopback — relative reference only.
