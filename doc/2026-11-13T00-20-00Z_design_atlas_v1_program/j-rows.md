| ID | Statement (proto §J row, verbatim) | Proto home |
|---|---|---|
| J01 | Build the search subcommand out substantially | #9092 (§A) |
| J02 | Workers fan-out; explore other techniques | #9092 (§A) |
| J03 | All workers instructed on search + queue/gh/connect | #9101 (§I) |
| J04 | Benchmark shard sizes for lowest latency | #9092 (§A, named oracle) |
| J05 | Workers + Durable Objects best use | #9092 (§A) |
| J06 | Cache until corpus change; short staleness; real-time / no-cache flags | #9092 (§A) |
| J07 | Query planner + other tooling investigation | #9092 (§A) |
| J08 | Second embedding approach beside Snowflake | #9092 (§A) |
| J09 | Backup all Turso DBs every ~14 days; Turso 30d protection inside 30d | #9093 (§B) |
| J10 | Turso → restic zstd deltas → R2; R2 → laptop + Drive; full mesh; D1/laptop DBs → Turso | #9093 (§B) |
| J11 | OpenCode DB: keep large; backup cost/growth investigation (Drive + laptop archive) | #9094 (§B) |
| J12 | Corpus constantly updating; ingest queue/PM/GitHub/Muse/OpenCode(+thinking?)/ID + missed sources | #9097 (§E) |
| J13 | Versioned Metadata schema on all items; many labels/categories applied | #9095 (§C) + #9097 (§E) |
| J14 | Issue/PR Metadata also versioned; schema version always included | #9095 (§C) → #9073/#9074 amendments |
| J15 | Additive-only evolution; old metadata valid indefinitely; old embeds on new systems | #9095 (§C) |
| J16 | Breaking changes parked, applied as one set, reclassify everything | #9095 (§C) |
| J17 | Extras / type-mismatch-as-extra / coercion rules | #9095 (§C) |
| J18 | Strict enums (kind/area/group/sensitivity); invalid sensitivity → most locked-down | #9095 (§C) |
| J19 | Optional attrs hard to filter — accepted; classifiers without schema permission | #9095 (§C) |
| J20 | Per-embed classification; boundary safety without all-secret default | #9096 (§D) |
| J21 | Granular sensitivity: project shareable / secrets hidden / personal-life off limits | #9096 (§D) |
| J22 | Disclosure by surface (public posting filtered even for privileged searcher) | #9096 (§D) + #9092 (§A) |
| J23 | Embeds backed up for restore | #9093 (§B) |
| J24 | Fan-out + Turso/D1 sharding for speed, cost-bounded | #9092 (§A) |
| J25 | Content offload to R2, fallback laptop → lab RustFS; hot cache in Turso/D1 | #9098 (§F) |
| J26 | Store result cache forever; compare queries over time | #9099 (§G) |
| J27 | Every query ingested as an embed attached to its results; all flags/params; tool hashes+versions | #9099 (§G) |
| J28 | Version every Workers deploy; version Turso + D1 schemas | #9099 (§G) |
| J29 | Turso stores embeddings + content hash + metadata; raw content in R2/RustFS/Drive — test the economics | #9098 (§F) |
| J30 | Content-addressed files + virtual path abstraction (`geordi.png` lookups) | #9100 (§H) |
| J31 | One upload, N location metadata records (host/date/process) | #9100 (§H) |
| J32 | Aggregated VFS index (own Turso DB / sidecar tables), no corpus scans | #9100 (§H) |
| J33 | Parquet / Cloudflare Basin / big-data fit for the metadata | #9100 (§H) |
| J34 | Internal use only; never exposed directly to public | #9091 + §D everywhere |
