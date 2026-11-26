# Atlas embed-storage composition audit (slice 1 of #9098)

**Measured:** 2026-11-25 11:18–11:31 UTC. **Scope:** live Turso `atlas` monolith, all 29 live topic stores, and the internal and secret content stores. **Tracking:** [#9098](https://github.com/developing-today/code/issues/9098); this slice feeds [#9084](https://github.com/developing-today/code/issues/9084).

This is the composition audit only. It does not evaluate content-in-Turso against hash-and-reference-to-R2, measure hydration latency or cost, choose a storage design, or settle #9084’s general reference-versus-copy question.

## Monolith result

The live monolith allocated **7,278,391,296 bytes** (1,776,951 pages × 4,096 bytes; freelist 0). Against that denominator, the measured payloads are:

| Component                                |    Measured bytes | Measured payload share of allocated monolith pages | Measurement                                                                                     |
| ---------------------------------------- | ----------------: | -------------------------------------------------: | ----------------------------------------------------------------------------------------------- |
| Vector values + vector-index shadow data |     6,329,625,792 |                                             86.96% | `vec_snowflake_m_v2_768_f32.vec` and `vec_snowflake_m_v2_768_f32_idx_shadow.data` byte sums     |
| Chunk text + message text                |       639,187,867 |                                              8.78% | UTF-8 byte length sums from `chunks.text` and `messages.text`                                   |
| Measured FTS payload (lower bound)       |        64,704,278 |                                              0.89% | Byte sums from FTS5 `chunks_fts_data.block`, `chunks_fts_docsize.sz`, and `chunks_fts_idx.term` |
| Other allocated pages                    |       244,873,359 |                                              3.36% | Total page allocation less the three payload categories above                                   |
| **Total**                                | **7,278,391,296** |                                        **100.00%** | `PRAGMA page_count × PRAGMA page_size`                                                          |

The vector table has 107,639 rows and 330,667,008 bytes of vector values. Its index shadow has 37,359 rows and 5,998,958,784 bytes of data. This is why counting only the F32 vector blobs would understate the vector footprint: the vector index shadow is about 5.999 GB.

The raw-text subtotal counts chunk and message text only: 173,633,287 bytes of chunk text (107,639 chunks) and 465,554,580 bytes of message text (71,551 messages). The measured FTS payload consists of 64,470,246 bytes in data blocks, 164,205 bytes in docsize blobs, and 69,827 bytes of term-key text. The `chunks_fts_idx` table also stores `segid` and `pgno`; their storage and B-tree page overhead are not separately attributed.

## Topic stores

The live database listing contains **29 topic stores**: 28 internal stores and `atlas-core-secret-main-01`. Every row below is measured independently. All have 4,096-byte pages and zero freelist pages. Each has one vector table and one `chunks` table; none contains an FTS table or a separate vector-index shadow. `Other pages` is allocated page bytes less the vector-value and chunk-text payloads; it includes indexes, row encoding, metadata, and page overhead.

| Topic store                               | Chunks / vectors | Allocated bytes |    Vector bytes | Chunk-text bytes | Other pages bytes |
| ----------------------------------------- | ---------------: | --------------: | --------------: | ---------------: | ----------------: |
| `atlas-core-internal-agent-tooling-01`    |            3,738 |      23,248,896 |      11,483,136 |        4,976,640 |         6,789,120 |
| `atlas-core-internal-agent-tooling-02`    |            3,801 |      22,765,568 |      11,676,672 |        4,764,986 |         6,323,910 |
| `atlas-core-internal-agent-tooling-03`    |            3,866 |      22,228,992 |      11,876,352 |        4,528,465 |         5,824,175 |
| `atlas-core-internal-agent-tooling-04`    |            3,866 |      22,671,360 |      11,876,352 |        4,857,308 |         5,937,700 |
| `atlas-core-internal-cloudflare-01`       |            1,250 |       7,872,512 |       3,840,000 |        2,849,749 |         1,182,763 |
| `atlas-core-internal-fleet-repos-01`      |            3,458 |      19,804,160 |      10,622,976 |        4,211,004 |         4,970,180 |
| `atlas-core-internal-fleet-repos-02`      |            3,457 |      22,085,632 |      10,619,904 |        5,933,704 |         5,532,024 |
| `atlas-core-internal-fleet-repos-03`      |            3,457 |      22,749,184 |      10,619,904 |        6,411,233 |         5,718,047 |
| `atlas-core-internal-home-lab-01`         |            4,070 |      22,515,712 |      12,503,040 |        3,794,981 |         6,217,691 |
| `atlas-core-internal-home-lab-02`         |            4,069 |      21,184,512 |      12,499,968 |        2,984,340 |         5,700,204 |
| `atlas-core-internal-infra-nix-01`        |            4,094 |      26,095,616 |      12,576,768 |        6,368,177 |         7,150,671 |
| `atlas-core-internal-infra-nix-02`        |            4,094 |      24,936,448 |      12,576,768 |        5,524,473 |         6,835,207 |
| `atlas-core-internal-media-01`            |            1,607 |      12,414,976 |       4,936,704 |        4,425,462 |         3,052,810 |
| `atlas-core-internal-misc-01`             |              841 |       5,255,168 |       2,583,552 |        1,465,862 |         1,205,754 |
| `atlas-core-internal-models-and-plans-01` |            4,551 |      26,480,640 |      13,980,672 |        5,281,524 |         7,218,444 |
| `atlas-core-internal-models-and-plans-02` |            4,551 |      33,587,200 |      13,980,672 |       11,264,469 |         8,342,059 |
| `atlas-core-internal-people-pets-01`      |            4,502 |      37,236,736 |      13,830,144 |       14,606,110 |         8,800,482 |
| `atlas-core-internal-personal-admin-01`   |            4,357 |      29,929,472 |      13,384,704 |        9,123,482 |         7,421,286 |
| `atlas-core-internal-personal-admin-02`   |            4,360 |      35,045,376 |      13,393,920 |       14,210,057 |         7,441,399 |
| `atlas-core-internal-personal-admin-03`   |            4,360 |      35,201,024 |      13,393,920 |       14,311,058 |         7,496,046 |
| `atlas-core-internal-personal-admin-04`   |            4,359 |      35,061,760 |      13,390,848 |       14,224,781 |         7,446,131 |
| `atlas-core-internal-security-01`         |            3,486 |      22,155,264 |      10,708,992 |        5,310,830 |         6,135,442 |
| `atlas-core-internal-work-engineering-01` |            4,435 |      22,224,896 |      13,624,320 |        2,859,440 |         5,741,136 |
| `atlas-core-internal-work-engineering-02` |            4,434 |      23,580,672 |      13,621,248 |        3,314,787 |         6,644,637 |
| `atlas-core-internal-work-engineering-03` |            4,435 |      23,351,296 |      13,624,320 |        3,650,393 |         6,076,583 |
| `atlas-core-internal-work-engineering-04` |            4,435 |      23,126,016 |      13,624,320 |        3,532,467 |         5,969,229 |
| `atlas-core-internal-work-engineering-05` |            4,434 |      23,285,760 |      13,621,248 |        3,618,273 |         6,046,239 |
| `atlas-core-internal-work-engineering-06` |            4,434 |      23,470,080 |      13,621,248 |        3,761,783 |         6,087,049 |
| `atlas-core-secret-main-01`               |              838 |       4,616,192 |       2,574,336 |        1,379,409 |           662,447 |
| **All 29 topic stores**                   |      **107,639** | **674,181,120** | **330,667,008** |  **173,545,247** |   **169,968,865** |

The topic stores contain 107,639 vectors and allocate 674,181,120 bytes total: 330,667,008 vector-value bytes, 173,545,247 chunk-text bytes, and 169,968,865 remaining page bytes.

## Content stores

These stores carry the hydrated message text shared by the topic shards. Each has 4,096-byte pages and zero freelist pages.

| Content store                 | Messages | Allocated bytes | Message-text bytes | Other pages bytes |
| ----------------------------- | -------: | --------------: | -----------------: | ----------------: |
| `atlas-core-internal-content` |   70,978 |     508,764,160 |        463,559,915 |        45,204,245 |
| `atlas-core-secret-content`   |      573 |       1,626,112 |          1,399,999 |           226,113 |

## Measurement method and limits

- The source of store names was the live `turso db list` output, authenticated with the named Fleet secret `turso.platform-api-token-atlas-search`. The audit included every `atlas-core-internal-*` and `atlas-core-secret-*` topic/content database in that listing, and the `atlas` monolith.
- Each connection used a named database token resolved with `fleet secrets get`; the audit issued only `SELECT` statements and `PRAGMA` reads. The report and its [measurement data](measurements.json) contain aggregate row counts and byte sums only; no message, chunk, vector, hash, or token values were returned or retained.
- Allocated database bytes are `PRAGMA page_count × PRAGMA page_size`. Text byte counts use `length(CAST(text AS BLOB))`; vector bytes use `length(vec)`; the vector-index payload uses `length(data)`; FTS bytes sum its stored block, docsize, and term-key payloads. The SQL and returned row counts, byte counts, query read counts, and durations are recorded in `measurements.json`.
- The hosted databases do not provide SQLite `dbstat` on the shard API (the probe returned `no such table: dbstat`); a whole-monolith page walk also exceeded 40 seconds. Therefore, physical page allocation cannot be attributed independently to each ordinary B-tree index through this interface. `Other allocated pages` is a measured residual and must not be read as index-only bytes. The vector, text, and FTS figures are measured payload bytes; B-tree page overhead and unmeasured index columns remain in the residual. The 86.96%, 8.78%, and 0.89% values are lower-bound payload shares of total allocated pages, not complete physical allocations for each object family.
- The monolith shares compare measured payload bytes with the full allocated page count. They answer the first-order composition question while keeping the unassigned 3.36% visible instead of assigning it to indexes by guesswork.

## Reproduction queries

Run the following read-only SQL against the named live stores with the corresponding named Fleet token:

```sql
PRAGMA page_count;
PRAGMA page_size;
PRAGMA freelist_count;
SELECT count(*), sum(length(vec)) FROM vec_snowflake_m_v2_768_f32;
SELECT count(*), sum(length(data)) FROM vec_snowflake_m_v2_768_f32_idx_shadow;
SELECT count(*), sum(length(CAST(text AS BLOB))) FROM chunks;
SELECT count(*), sum(length(CAST(text AS BLOB))) FROM messages;
SELECT count(*), sum(length(block)) FROM chunks_fts_data;
SELECT count(*), sum(length(sz)) FROM chunks_fts_docsize;
SELECT count(*), sum(length(CAST(term AS BLOB))) FROM chunks_fts_idx;
```

For each store, omit statements whose table is absent; topic stores have no messages or FTS tables, while the content stores have messages but no vector tables. The monolith’s named token is `turso.atlas-rw-token`; the topic and content token names follow `turso.<database-name>-token`. The named tokens themselves are never part of the record.
