# CNCF Landscape — Data Layer

*Researched 2026-11-23. Scope: databases, storage, and streaming/messaging — the data layer of the CNCF landscape, plus the non-CNCF systems you cannot honestly discuss the category without (CockroachDB, Kafka, Redpanda, FoundationDB).*

*Estate lens used for every "should we use it" verdict: Postgres-first today (Postgres in services, Turso/SQLite in Atlas), DRBD/LINSTOR storage under Cozystack, a small team, and a standing rule that performance claims must be benchmark-proven, never asserted. Vendor benchmark numbers are labelled as vendor numbers wherever they appear.*

*Status corrections found during research, because the landscape moves fast: **NATS graduated on 2026-11-02** (announced at KubeCon NA, 2026-11-10) — it is no longer incubating. **CloudNativePG was promoted from sandbox to incubating on 2026-11-02** — it is no longer a sandbox project. **Dragonfly graduated on 2025-10-28** — it is no longer incubating. Verdicts below use the current statuses.*

---

## Headline verdicts

- **TiKV for us: no.** It is a genuinely good system with an honest Jepsen record (see below), but it is a *storage layer for a distributed SQL database*, not a database you adopt on its own — and the SQL layer on top of it (TiDB) speaks MySQL, not Postgres. Adopting it means leaving the Postgres ecosystem to solve a scale problem we do not have.
- **Postgres answer: CloudNativePG + replicas.** For our scale, a good Postgres operator with streaming replication and read replicas covers everything on our roadmap. CloudNativePG is the operator to standardise on — and it is the one Cozystack already ships for managed Postgres.
- **NATS: yes — the plan is right, with a shape.** Core NATS for service communication and request/reply, JetStream for durable streams and work queues, the JetStream KV store for small watched state. The thing to avoid is treating JetStream as a Kafka-class infinite log; it is not built for that and doesn't claim to be.
- **Rook vs LINSTOR: stay on LINSTOR.** Rook means running Ceph. Ceph wins at very large scale, native object storage, and ReadWriteMany file volumes; LINSTOR/DRBD wins on latency, resource footprint, and operational simplicity at our scale — which is why Cozystack ships it. Add Rook only if a specifically Ceph-shaped need appears.

---

## 1. TiKV — the distributed transactional key-value store

**CNCF status: Graduated** (September 2020). **Maturity verdict: genuinely mature — heavily deployed at very large scale in China especially, actively maintained, Apache 2.0.**

### What it is

TiKV is a distributed, transactional key-value database written in Rust by PingCAP. Data is automatically sharded into "Regions," each replicated with Raft (on top of RocksDB) and placed by a separate scheduling component, the Placement Driver (PD), which also hands out globally monotonic timestamps. Transactions follow Google's Percolator model — two-phase commit coordinated against PD's timestamp oracle. It exists in two modes: RawKV (fast, no transactions) and TxnKV (full Percolator transactions). It was built as, and still primarily is, the storage layer under TiDB, PingCAP's distributed SQL database; the TiKV project itself describes its role as a "unifying distributed storage layer" that complements etcd, which it positions as the tool for low-volume metadata ([TiKV concepts](https://tikv.org/docs/3.0/concepts/overview/?__hstc=86493575.e813f5732dc462167963922d68a481da.1744540606403.1744540606403.1744540606403.1&__hssc=86493575.1.1744540606403&__hfp=2888266827)).

### Does it fail Jepsen?

No — but the honest answer is longer than "no," and the history is the most instructive part.

The one full, independent Jepsen analysis is of **TiDB 2.1.7** (2019, funded by PingCAP). It found that TiDB, in its **default configuration**, violated the snapshot isolation it promised: two auto-retry mechanisms — one in the client library, one in the server — blindly re-applied transactions after conflicts, producing read skew and lost updates (in one test, 64 of 378 insertions were lost in 30 seconds). The same report found that **with both retry mechanisms disabled, TiDB passed** its tests for snapshot isolation and single-key linearizability, and that release 3.0.0-rc.2, which turned auto-retry off by default, also passed ([Jepsen: TiDB 2.1.7](https://jepsen.io/analyses/tidb-2.1.7); the failure-and-fix arc is also summarised in [this TiDB history](https://www.stacksync.com/blog/the-titanium-database-the-origin-story-of-tidb)). PingCAP published a companion response, fixed the issues, and — unusually — kept Jepsen in its own test infrastructure afterwards; a PingCAP engineer's [survey of safety pitfalls found by Jepsen](https://hoverbear.org/blog/common-safety-pitfalls-found-by-jepsen/) describes Jepsen as "indispensable for battle-testing TiDB and TiKV."

Later findings exist and should be stated plainly, because they are the normal texture of a system that keeps being tested:

- In 2021, a Jepsen append-workload run against the TiKV/TiDB 5.0 line found a strict-serializability anomaly (a cycle in the transaction history) and it was filed and tracked in the open ([tikv/tikv#10468](https://github.com/tikv/tikv/issues/10468)).
- TiKV's **async commit** optimisation is documented by its own developers as not ensuring linearizability or external consistency in corner cases; the recorded position is that users who depend on external consistency can disable async commit ([tikv/tikv#8589](https://github.com/tikv/tikv/issues/8589)).

So the accurate one-liner: **TiDB failed Jepsen in its default configuration in 2019, the root causes were fixed and the defaults changed, and the project has kept finding and disclosing edge-case anomalies in new features since — which is what a healthy correctness culture looks like, not a failing one.** There is no more recent full independent Jepsen analysis of a current TiDB/TiKV release, so claims about today's versions rest on PingCAP's internal testing rather than independent verification.

### Is it slow?

It depends what you compare it against, and most published numbers are vendor numbers. TiKV's own documentation claims RawKV average latency under 1 ms (P99 = 10 ms) and publishes YCSB results of roughly 212,000 point reads/sec and 43,200 updates/sec on a 3-node cluster of large (40 vCPU) machines ([TiKV performance overview](https://tikv.org/docs/dev/deploy/performance/overview/)). A more independent data point comes from JuiceFS, which supports TiKV as a metadata engine and benchmarked the field: in their operation-latency tests the ordering was Redis fastest, then TiKV (3 replicas), then local MySQL roughly tied with etcd (3 replicas) — and their qualitative table rates TiKV "medium" performance / "high" scalability against etcd's "low / low" ([JuiceFS metadata engine guide](https://juicefs.com/en/blog/usage-tips/juicefs-metadata-engine-selection-guide)).

The design-level truth underneath the numbers: TiKV pays for what it is. Every transactional write goes through two-phase commit with timestamps from PD, data is sharded across many Raft groups, and reads may need to resolve locks — so per-operation latency, especially for TxnKV, is structurally higher than etcd, Redis, or a single Postgres node. What you get in exchange is horizontal scale to very large datasets with strong consistency. **"Slow" is the wrong word; "you pay latency for scale you may not need" is the right one.** Nobody should adopt or reject it on these published figures — if it ever became a real candidate here, the estate rule applies: benchmark it against our actual workload on our actual hardware first.

### Should we use it?

**No.** Three independent reasons, any one sufficient: (1) it is a layer, not a database — adopting it means adopting TiDB on top (next section); (2) we have no workload within orders of magnitude of needing sharded transactional KV; (3) the places we do need small, strongly consistent KV inside Kubernetes are already served by etcd, and the places we need real data storage are served by Postgres and SQLite/Turso.

---

## 2. TiDB — and the "could TiKV replace CockroachDB?" question

**CNCF status: not a CNCF project** (TiDB itself is a PingCAP project under Apache 2.0; only TiKV is in CNCF). **Maturity verdict: mature, widely deployed, the standard-bearer for open-source distributed SQL on the MySQL side.**

### What it is

TiDB is the stateless SQL layer that sits on TiKV: it parses MySQL-protocol queries, plans and executes them, and pushes as much computation as possible down into TiKV's coprocessor. It offers snapshot isolation (which, for MySQL compatibility, it calls "repeatable read" — a naming confusion the Jepsen report itself flags), horizontal scaling of both reads and writes, and an HTAP story via TiFlash, a columnar replica engine for analytical queries ([Jepsen: TiDB 2.1.7](https://jepsen.io/analyses/tidb-2.1.7)).

### TiKV vs CockroachDB is a category error — TiDB vs CockroachDB is the real comparison

TiKV alone cannot replace CockroachDB: CockroachDB is a complete SQL database, TiKV is the storage engine half of one. The comparable stack is **TiDB + TiKV + PD** versus CockroachDB. Compared honestly:

| | TiDB (+ TiKV) | CockroachDB |
|---|---|---|
| Wire protocol / dialect | **MySQL** | **PostgreSQL** |
| Default isolation | Snapshot isolation (allows write skew) | **Serializable** (strongest standard level; clients must handle retry errors) |
| Licence | **Apache 2.0**, both layers | Proprietary since v24.3 (see §3) |
| Architecture | Separate SQL, storage, and timestamp/scheduler tiers | Symmetric nodes, storage and SQL in one binary |
| Jepsen posture | Failed in default config (2019), fixed; anomalies still found and disclosed in new features | Failed as a beta (2016), fixed pre-1.0; guarantees serializable, *not* strict serializable (§3) |
| Extras | HTAP via TiFlash columnar replicas | Multi-region row placement, built-in changefeeds |

### Should we use it?

**No, and the wire protocol is the decisive reason.** We are a Postgres shop: our services, tooling, and instincts are Postgres. TiDB would mean adopting the MySQL dialect and ecosystem in exchange for horizontal write scaling we do not need. If the day ever comes when a single Postgres primary genuinely cannot carry a workload, the distributed-SQL question gets re-opened then — and the Postgres-protocol candidates (CockroachDB, YugabyteDB) start ahead of TiDB for us precisely because they don't require changing dialect.

---

## 3. CockroachDB — the comparator that left open source

**CNCF status: not CNCF. Licence status: no longer open source.** **Maturity verdict: technically excellent, commercially re-enclosed — the licensing trajectory is now the dominant fact about it.**

### What it is

A distributed SQL database patterned on Google Spanner: data is split into ranges replicated with Raft, transactions are timestamped with hybrid logical clocks, and it speaks the PostgreSQL wire protocol with serializable isolation by default. Its Jepsen history is the mirror image of TiDB's: the [2016 Jepsen analysis of the beta](https://jepsen.io/analyses/cockroachdb-beta-20160829.pdf) found real serializability violations (a timestamp-cache bug; transactions applied twice via internal retries); Cockroach Labs fixed them, and [announced in 2017 that the beta passed Jepsen testing](https://cockroachlabs-www-prod.netlify.app/blog/cockroachdb-beta-passes-jepsen-testing/). One nuance worth keeping: CockroachDB guarantees serializability, not strict serializability — the whole database is not linearizable, by design, because it uses hybrid logical clocks rather than Spanner's TrueTime.

### The licensing story, because it decides the question

CockroachDB's core began under Apache 2.0, moved to the Business Source License in 2019, and in **November 2024 (v24.3) the free Core edition was retired entirely** in favour of the proprietary CockroachDB Software License: self-hosted production use requires a licence key — paid Enterprise, or "Enterprise Free" for businesses under $10M revenue, which mandates telemetry and throttles clusters that stop sending it ([licence history summary](https://cloudparts.dev/technologies/cockroachdb/); [consequences catalogue](https://codewithsql.com/sql-tools/best-cockroachdb-alternatives)). In 2026 Cockroach Labs went a step further and announced that **new versions will be developed in private repositories** — the public repo remains as an unmaintained snapshot ([Cockroach Labs announcement](https://www.cockroachlabs.com/blog/source-code-protection/)).

### Should we use it?

**No.** Whatever its technical merits — and serializable-by-default on the Postgres wire protocol is a genuinely attractive combination — adopting a database whose licence now requires keys, telemetry, and a revenue ceiling, and whose source is going private, runs directly against the reason this estate prefers open infrastructure. It remains useful as the benchmark for what distributed SQL should feel like, and YugabyteDB (Apache 2.0, Postgres-compatible, not CNCF) is the name to remember if that shape is ever needed without the licence.

---

## 4. Vitess — MySQL's sharding layer

**CNCF status: Graduated** (2019). **Maturity verdict: extremely mature — born at YouTube, now the default answer for MySQL at serious scale (Slack, GitHub, PlanetScale's business is built on it). Apache 2.0.**

### What it is

Vitess is a proxy and orchestration layer that makes a fleet of MySQL instances behave like one database: it shards data across MySQL servers, routes and rewrites queries, manages failover, and presents a single MySQL-protocol endpoint. The crucial thing to understand is what it is *not*: it is not a new storage engine or a new database — the data lives in ordinary MySQL, and Vitess supplies the distribution brain that MySQL never grew natively.

### When Vitess is the answer — and why it isn't ours

Vitess is the answer when you already have MySQL, your data or write volume has outgrown one primary, and you cannot change databases. That is a real and common situation, and for it Vitess is close to the only serious open-source option. **None of that describes us**: we are Postgres-first, Postgres has no equivalent "you must shard or die" cliff at our scale, and adopting Vitess would mean adopting MySQL first. Verdict: **know it exists; not ours.**

---

## 5–7. Running Postgres in Kubernetes: the operator field

This is the section with the most direct consequence for us, because it answers Drew's question — *"or just use the Postgres operator and a bunch of read replicas?"* — with: **yes, exactly that.**

### 5. CloudNativePG

**CNCF status: Incubating** (accepted as sandbox January 2025; promoted to incubating 2026-11-02 — the first Postgres project to reach that level). **Maturity verdict: the current favourite of the field, and the one to standardise on. Apache 2.0.**

**What it is.** An operator from EnterpriseDB (EDB) that manages the full lifecycle of a PostgreSQL cluster — primary plus streaming-replication standbys, automated failover, rolling updates, continuous backup and point-in-time recovery to object storage, and PgBouncer pooling. Its distinguishing design choice is that it does **not** use Patroni: each Postgres instance runs CloudNativePG's own instance manager, and the Kubernetes API itself serves as the consensus store, so there is no extra HA machinery to operate ([operator comparison](https://www.osseva.io/blog/postgresql-kubernetes-operator-comparison); [EDB on the incubation review](https://enterprisedb.com/blog/postgresr-belongs-kubernetes-cloudnativepgtm-reaches-cncf-incubation)). The CNCF's incubation due diligence interviewed production adopters — 50+ organisations are listed — which is the kind of verification most operators never get ([incubation record](https://forum.unchainedstack.org/t/cloudnativepg-1-30-2-postgresql-18-7-by-default-postgresql-14-dropped-and-a-cncf-incubating-operator/808)).

**Estate note:** Cozystack already delivers its managed PostgreSQL via CloudNativePG ([Cozystack overview](http://rawkode.academy/technology/cozystack)) — so standardising on it is not a new dependency, it is aligning with what our platform layer already chose.

### 6. Zalando postgres-operator

**CNCF status: not CNCF.** **Maturity verdict: the elder statesman — battle-hardened at Zalando's own scale, still maintained, visibly slower cadence than CloudNativePG. MIT (operator).**

**What it is.** Zalando's operator runs each cluster on **Spilo**, an image bundling PostgreSQL + Patroni + WAL-G, with Patroni using the Kubernetes API as its consensus store. It has run hundreds of production clusters inside Zalando for years and is a perfectly respectable choice — particularly if you already have Patroni skills. Recent comparisons consistently place it second to CloudNativePG for new deployments: equally capable HA, but the Patroni-in-image model is more machinery, and development momentum has shifted ([operator comparison](https://www.osseva.io/blog/postgresql-kubernetes-operator-comparison); [five-operator dissection](https://blog.seon.world/blog/hybrid-k3s-04-cloudnativepg/)).

### 7. Crunchy Data PGO

**CNCF status: not CNCF.** **Maturity verdict: capable and enterprise-oriented, with a licensing catch in the images. Operator code Apache 2.0; container images under Crunchy Data Developer Program terms.**

**What it is.** Crunchy Data's operator: Patroni-based HA like Zalando's, but paired with **pgBackRest**, the most respected backup tool in the Postgres world, and a general emphasis on data protection. The catch is commercial: while the operator code is Apache 2.0, Crunchy's container images are distributed under terms that require a support subscription for production use at organisations with 50+ employees — the kind of term that is easy to miss until procurement finds it ([operator comparison](https://www.osseva.io/blog/postgresql-kubernetes-operator-comparison)).

### The pragmatic verdict for our scale

**CloudNativePG, one primary + one or two streaming replicas per cluster, backups to object storage, read replicas added when a specific service shows read pressure.** That combination covers every database need currently visible in this estate. The failure mode to avoid is reaching for distributed SQL (TiDB, CockroachDB) as insurance against a scale that hasn't arrived: you pay its costs — latency, operational complexity, dialect or licence compromises — every day, for a benefit that only exists hypothetically. Postgres's own scaling ladder (tuning → replicas → partitioning → Citus-style sharding) is long, and we are near the bottom of it.

---

## 8–10. Storage: Rook/Ceph vs LINSTOR/DRBD vs Longhorn

### 8. Rook — Ceph, operated from inside Kubernetes

**CNCF status: Graduated** (2020). **Maturity verdict: mature and widely deployed; it is the standard way to run Ceph on Kubernetes. Apache 2.0.**

**What it is.** Rook is a storage *orchestrator*: it deploys, configures, and manages storage systems — above all **Ceph** — as Kubernetes-native resources. Ceph itself is a unified distributed storage system offering block (RBD), file (CephFS), and object (RGW, S3-compatible) storage from one cluster, placing data with its CRUSH algorithm across as many nodes as you have. Rook's job is to make running that famously intricate system survivable: it handles the monitors, OSDs, managers, and their upgrades as an operator ([storage overview](https://kb.ekarisky.com/storage/)).

### 9. LINSTOR / DRBD — Cozystack's storage (not CNCF)

**CNCF status: not CNCF** (LINBIT's projects; DRBD is in the Linux kernel, GPL). **Maturity verdict: DRBD has replicated Linux block devices for over two decades; LINSTOR is its mature cluster manager. This is the conservative option, not the exotic one.**

**What it is.** The opposite philosophy to Ceph. Instead of one cluster-wide data plane, each volume is an ordinary local block device (LVM or ZFS underneath) that **DRBD replicates synchronously to one or two specific other nodes**, and LINSTOR is the control plane that decides placement and manages the volumes, exposing them to Kubernetes through a CSI driver. A volume's data lives on a small, knowable set of nodes; reads are usually local.

**The two philosophies, and when each wins.** Independent benchmarking by Palark (a Kubernetes consultancy; treat as informed third-party, not gospel) found LINSTOR consistently among the fastest options tested, with Ceph trailing and consuming significantly more CPU and RAM per node ([Palark storage benchmarks](https://palark.com/blog/kubernetes-storage-performance-linstor-ceph-mayastor-vitastor/)). That matches the architectures: DRBD replication of a local device is a short path; Ceph's CRUSH placement, OSD daemons, and cluster-wide coordination are a longer one. Ceph's advantages are equally real: it scales to hundreds of nodes and petabytes, offers native S3 object storage and ReadWriteMany file volumes, and rebalances data across the whole cluster automatically. LINSTOR's limits are the mirror image: replication is per-volume to fixed peers, there is no built-in object store, and very large clusters mean managing many individual replication relationships. Cozystack's design reflects exactly this trade: **DRBD + LINSTOR for block storage, with disaster recovery via stretched LINSTOR clusters across sites** ([Cozystack comparison](https://github.com/livenson/talu/blob/HEAD/docs/architecture/comparison.md)), and S3 needs met by a separate object store rather than Ceph.

**Would we need Rook if we're on Cozystack with DRBD/LINSTOR? No.** Rook is not a complement to LINSTOR; it is the on-ramp to a *different storage system* (Ceph) that would replace LINSTOR's role. Running both means operating two distributed storage stacks — the single most operationally expensive category of software there is — for a small team. The legitimate triggers to revisit: a genuine need for large-scale ReadWriteMany file storage, native S3 at a scale beyond what Cozystack's object options serve, or growth to a node count where per-volume replication management becomes the bottleneck. None are currently on the roadmap.

### 10. Longhorn — the middle path

**CNCF status: Incubating** (since 2021-11-04; [CNCF project page](https://www.cncf.io/projects/longhorn/)). **Maturity verdict: mature, popular with smaller clusters, backed by SUSE/Rancher. Apache 2.0.**

**What it is, in one paragraph.** Longhorn (from Rancher) takes a third approach: for each volume it runs a dedicated engine process that synchronously replicates to replica processes on other nodes, exposed via iSCSI (the V1 engine) or SPDK/NVMe-TCP (the V2 engine, GA in v1.12), with incremental backups to S3/NFS built in ([storage overview](https://kb.ekarisky.com/storage/)). It is simpler to adopt than Ceph and more Kubernetes-native in feel than LINSTOR, and it is the common recommendation for two-to-four-node clusters. For us it is a solution in search of a problem: Cozystack already made this choice (LINSTOR), and Longhorn's per-volume engine overhead historically costs more performance than DRBD's in-kernel path. Mention it when advising others with small clusters; don't adopt it alongside Cozystack.

---

## 11–17. Streaming and messaging: NATS and the field

### 11. NATS — the plan, and why it's the right one

**CNCF status: Graduated** (2026-11-02, announced 2026-11-10 — [CNCF announcement](https://www.cncf.io/announcements/2026/11/10/cncf-announces-nats-graduation-solidifying-a-production-ready-foundation-for-messaging-across-cloud-and-edge/)). **Maturity verdict: mature, broadly deployed (2,000+ companies per the graduation announcement), freshly graduated after a third-party security audit. Apache 2.0.**

**What it is.** One Go binary that is simultaneously four things, addressed by subject names rather than hosts:

- **Core NATS** — publish/subscribe, request/reply, and queue groups, with no persistence: fire-and-forget at very low latency and a tiny footprint (idle memory measured in tens of MB).
- **JetStream** — the persistence layer built in: named streams capture messages (by subject, with retention policies), and consumers track delivery with acknowledgements, replay, and deduplication. This is the streaming/work-queue engine.
- **KV Store** — a key-value store built on JetStream, with watch semantics: clients can observe every change to a key or key prefix. Good for configuration, feature state, leader-ish coordination data, and small shared state.
- **Object Store** — large blobs chunked over JetStream, for artifacts and files rather than messages.

Clusters replicate JetStream data with Raft; leaf nodes and gateways extend one system across sites and edges. Cozystack, incidentally, already offers managed NATS as a tenant service ([Cozystack overview](http://rawkode.academy/technology/cozystack)).

**How we would best use it.** Core NATS as the default service-to-service fabric — request/reply for synchronous calls that shouldn't be HTTP, pub/sub for events that are allowed to be lost. JetStream for anything that must not be lost: durable event streams between our own services, work queues with ack-based delivery, and replayable integration feeds. The KV store for small, watched, shared state — service configuration, routing tables, presence — rather than standing up etcd or abusing a database table. Object Store sparingly, for message-adjacent blobs, not as a general file store.

**Downsides, honestly.** (1) JetStream is not a Kafka-class log: its per-stream throughput ceiling is well below what Kafka/Redpanda sustain, its consumers model is heavier per consumer, and it has nothing like Kafka Connect's connector ecosystem or the lakehouse (Iceberg) integrations — if the future holds high-volume CDC or analytics pipelines, that is Kafka's home turf, not NATS's. (2) Ordering guarantees are per-stream/per-subject, not global. (3) The deduplication window is time-based (2 minutes by default), which is a real footgun for exactly-once illusions at the edge. (4) Governance had a genuine scare: in 2025 Synadia, NATS's corporate sponsor, moved toward pulling the project back under a commercial licence; the settlement left the **trademark held by the Linux Foundation** ([NATS knowledge base](https://kb.ekarisky.com/messaging/nats/)) and graduation followed — resolved, but worth knowing it happened. None of these downsides touch our planned use; all of them argue for keeping Kafka-shaped doors open in service designs (publish events in a way that could be re-pointed) rather than welding everything to JetStream semantics.

**Verdict: yes, NATS is the right choice for streams and communications in this estate** — because most of what we call "streaming" is actually messaging plus modest durable queues, and NATS does all of it in one small system a small team can operate. Kafka would be a second distributed system bought to serve the 10% case.

### 12. Apache Kafka — the incumbent log

**CNCF status: not CNCF (Apache Software Foundation).** **Maturity verdict: the industry default for high-volume event streaming; nothing else has its ecosystem. Apache 2.0.**

Kafka is a partitioned, replicated, replayable commit log: producers append to topics, consumer groups read at their own pace, and retention is time/size-based rather than consumption-based. Version 4.x removed ZooKeeper (KRaft mode), which closed the oldest operational complaint. Its strengths are throughput at scale, the connector ecosystem (Kafka Connect, Debezium CDC), stream processing (Kafka Streams, Flink), and the new lakehouse integrations. Its downsides for us are equally clear: it is a JVM distributed system that wants dedicated expertise (partition management, rebalancing behaviour, retention/disk planning), its economics assume volume, and its request/reply and lightweight messaging story is poor — it does one job (the durable log) and everything else is scaffolding ([alternatives analysis](https://redisson.pro/blog/apache-kafka-alternatives.html)). On Kubernetes, **Strimzi** (CNCF incubating) is the standard operator if Kafka is ever adopted. Note also the ecosystem weather: IBM completed its ~$11B acquisition of Confluent in March 2026, which is pushing some Kafka users to re-evaluate ([Factor House analysis](https://www.factorhouse.io/resources/kafka/comparisons/confluent-alternatives-after-ibm/)) — Apache Kafka itself is unaffected, but the commercial centre of gravity moved.

### 13. Redpanda — Kafka's protocol, rewritten in C++

**CNCF status: not CNCF.** **Maturity verdict: technically impressive and genuinely easier to run than Kafka; commercial licensing on the enterprise tier. Community edition free; source-available rather than fully open for parts.**

Redpanda reimplements the Kafka wire protocol in a single C++ binary (thread-per-core, no JVM, no ZooKeeper/KRaft layer to operate), so existing Kafka clients and most tooling connect unchanged. Independent testing confirms the compatibility is real but not total — documented gaps exist around transactions, exactly-once semantics, and Connect/Streams edge cases ([Factor House evaluation](https://factorhouse.io/resources/kafka/cloud-brokers/)). Vendor-adjacent benchmarks consistently show lower tail latency than Kafka; per estate rules, treat the magnitude as unproven until measured locally. **For us:** Redpanda only becomes interesting if we first decide we need the Kafka API — at which point it is the most operable way to get it. It does not compete with NATS for the messaging job.

### 14. Apache Pulsar — the other Apache streaming platform

**CNCF status: not CNCF (Apache).** **Maturity verdict: capable, feature-rich, and the most operationally heavy option in this list.**

Pulsar separates serving (brokers) from storage (Apache BookKeeper), giving it genuinely nice properties — tiered storage to S3, multi-tenancy, geo-replication, and both streaming and queue semantics in one system. The price is running two distributed systems (plus, historically, ZooKeeper) where Kafka runs one and NATS runs a single binary. It has a loyal user base at large scale, but for a small team it is the worst operability-per-benefit ratio here. **Verdict: know it exists; not a candidate.**

### 15. Redis Streams — the one already in the building

**CNCF status: not CNCF.** **Maturity verdict: mature as a feature of Redis; deliberately small in scope.**

Redis Streams gives you an append-only log with consumer groups and acknowledgements inside a system many estates already run. For low-volume, "we need a queue with replay and we're already on Redis" situations it is entirely respectable, and it is the honest baseline NATS should be compared against at the small end. Its limits: durability and replication semantics are Redis's (asynchronous replication can lose acknowledged writes on failover), memory-first storage makes long retention expensive, and it is not a messaging fabric — no request/reply culture, no leaf/edge topology. **Verdict: fine where it already exists; not a reason to skip NATS, and NATS's KV/Object/JetStream breadth is exactly what Redis Streams doesn't try to be.**

### 16. WarpStream / Bufstream — the object-storage-native Kafka clones

**CNCF status: not CNCF (both commercial).** **Maturity verdict: an interesting architectural bet — brokers store everything in S3, so disks and rebalancing mostly disappear — with vendor-turbulence caveats.**

Both implement the Kafka protocol with object storage as the only storage tier, trading latency (noticeably higher than local-disk systems) for elasticity and cost at cloud scale. The turbulence: WarpStream was acquired by Confluent in 2024 (and so is now inside IBM), and Buf announced in May 2026 that CoreWeave had acquired Bufstream, with its future availability to be confirmed ([Factor House analysis](https://www.factorhouse.io/resources/kafka/comparisons/confluent-alternatives-after-ibm/)). **Verdict: irrelevant to a bare-metal Cozystack estate** — their entire premise is cheap cloud object storage and elastic cloud compute — and a useful reminder that "Kafka-compatible" startups are acquisition targets.

### 17. "Trimmer" — an identification, honestly labelled

**No streaming platform named "Trimmer" exists in the CNCF landscape or elsewhere that we can find.** The overwhelmingly likely referent is **Tremor** — similar name, present on the CNCF landscape in the streaming/messaging area:

**Tremor** — **CNCF status: was Sandbox (accepted 2020-09-08; [CNCF project page](https://www.cncf.io/projects/tremor/)); now archived.** A Rust event-*processing* engine from Wayfair: it sits *on top of* systems like Kafka and NATS and does filtering, routing, traffic shaping, and transformation of high-volume event streams (born in observability pipelines, replacing Logstash-style tooling), with its own scripting and query languages ([CNCF annual review](https://gitea.cncfstack.com/cncf/toc/src/commit/8542f7fdc56e06bdee76acdea679b054b1052e74/reviews/2021-tremor-annual.md)). Two things matter for Drew's question: **it is not a Kafka replacement** — it is a processor that consumes from brokers, so it never competed with NATS in the first place; and **the project is effectively dormant/archived now** ([Rawkode guide](http://rawkode.academy/technology/tremor)), so even its real niche (structured event pipelines) is better served today by tools like Vector or Benthos. If "Trimmer" was instead a misreading of **Strimzi**, that's the Kafka-on-Kubernetes operator mentioned in §12 — an operator, not a broker.

*(One more name in this neighbourhood: **Pravega**, a CNCF sandbox stream-storage system from Dell, still exists on the landscape but is quiet; it stores streams as first-class objects rather than topics. Not a candidate for us over NATS, for the same operability reasons as Pulsar.)*

---

## 18–20. The supporting cast

### 18. etcd — the consensus store underneath everything

**CNCF status: Graduated** (2020). **Maturity verdict: one of the most dependable pieces of software in the entire landscape. Apache 2.0.**

etcd is a Raft-based, strongly consistent key-value store with watches and leases — and it is already running in our estate, because it is Kubernetes' own datastore: every cluster we have trusts it with everything. Using it *directly* makes sense for small, metadata-shaped, must-be-consistent data: coordination, configuration, service discovery, leader election — by design it targets low-volume metadata and pushes back (practically, around 8 GB) against being used as a general database ([TiKV concepts](https://tikv.org/docs/3.0/concepts/overview/?__hstc=86493575.e813f5732dc462167963922d68a481da.1744540606403.1744540606403.1744540606403.1&__hssc=86493575.1.1744540606403&__hfp=2888266827)). **Verdict: already ours via Kubernetes; reach for it directly only for small coordination data, and prefer NATS KV for application-level watched state since it's the system we're already choosing to operate.**

### 19. Dragonfly — ⚠️ two projects, one name

**CNCF status: Graduated** (2025-10-28, announced 2026-01-14; [CNCF presentations record](https://presentations.cncf.io/projects/dragonfly/)). **Maturity verdict: proven at Alibaba/Ant Group scale. Apache 2.0.**

The CNCF's Dragonfly is a **peer-to-peer image and file distribution system**: nodes fetch container images and large files from each other under a scheduler's direction instead of all hammering one registry, which the graduation announcement credits with up to 90% storage-bandwidth savings at adopters like Ant Group, Alibaba, and Datadog ([graduation coverage](https://daily.dev/tags/cloud-native/best-of/2026/01)). It belongs properly to the registry conversation and is covered there; it appears here only to plant the warning flag: **DragonflyDB — the fast Redis-compatible in-memory store you will also encounter — is a completely different, non-CNCF, source-available project that shares the name.** Confusing the two in a design document would be embarrassing and is now on record as avoidable. For the data layer itself: Dragonfly-the-distribution-system stores no application data; DragonflyDB-the-store is a Redis alternative we have no reason to prefer over the Redis/Valkey we already know.

### 20. FoundationDB — the layer underneath other people's databases

**CNCF status: not CNCF** (Apple's project, Apache 2.0). **Maturity verdict: exceptionally solid engineering with a famously strict testing culture; an ingredient, not a product you deploy as "the database."**

FoundationDB is a distributed transactional key-value store (the original "unbundled database" design) whose claim to fame is simulation testing: its entire codebase is built to run deterministically in simulation so failure scenarios can be tested exhaustively. Apple runs it at enormous scale for iCloud, and its real role in the ecosystem is as a *substrate* — other databases and record layers are built on it (it is the reference point TiKV is most often compared against, and JuiceFS lists FDB support as in progress alongside TiKV and etcd — [JuiceFS guide](https://juicefs.com/en/blog/usage-tips/juicefs-metadata-engine-selection-guide)). **Verdict: admire it; there is no path by which this estate operates it directly — anything we'd build on it, we'd build on Postgres or TiKV-shaped layers instead, and we've already declined those.**

---

## Summary table

| Project | CNCF status | What it is, in five words | For us |
|---|---|---|---|
| TiKV | Graduated | Sharded transactional KV in Rust | No — a layer, not a database |
| TiDB | Not CNCF | Distributed SQL on TiKV | No — MySQL dialect, unneeded scale |
| CockroachDB | Not CNCF | Distributed SQL, Postgres protocol | No — proprietary licence since 2024 |
| Vitess | Graduated | Sharding brain for MySQL fleets | No — we're Postgres-first |
| CloudNativePG | Incubating | Postgres operator, no Patroni | **Yes — standardise on it** |
| Zalando operator | Not CNCF | Postgres operator, Patroni/Spilo | Credible second choice |
| Crunchy PGO | Not CNCF | Postgres operator, pgBackRest | Watch the image licence terms |
| Rook | Graduated | Operator that runs Ceph | No — LINSTOR already fills the role |
| LINSTOR/DRBD | Not CNCF | Per-volume replicated block storage | **Yes — already ours via Cozystack** |
| Longhorn | Incubating | Per-volume replicated block, K8s-native | No — same job, already filled |
| NATS | Graduated | Messaging + streaming + KV, one binary | **Yes — as planned, JetStream included** |
| Kafka | Not CNCF (Apache) | The high-volume replayable log | Only if a true log workload appears |
| Redpanda | Not CNCF | Kafka protocol, C++ binary | Only as the operable way to get Kafka API |
| Pulsar | Not CNCF (Apache) | Streaming + queuing, split storage | No — heaviest to operate |
| Redis Streams | Not CNCF | Log inside Redis | Fine where Redis already is |
| WarpStream/Bufstream | Not CNCF | Kafka API on object storage | No — cloud-shaped, vendor turbulence |
| Tremor ("Trimmer"?) | Archived (was Sandbox) | Rust event *processor*, not a broker | No — dormant; never a broker rival |
| etcd | Graduated | Raft KV for metadata/coordination | Already ours via Kubernetes |
| Dragonfly | Graduated | P2P image/file distribution | Registry lane's topic; not data storage |
| FoundationDB | Not CNCF | Transactional KV substrate | No — an ingredient, not a product |
