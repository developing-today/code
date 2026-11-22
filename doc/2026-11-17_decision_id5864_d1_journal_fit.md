# Design: D1 journal fit — ordering, sessions, and replication against the signed journal

- Date: 2026-11-17
- Status: **decision record** for
  [#5864](https://github.com/developing-today/code/issues/5864)
  (D1 journal fit, design + spike). The fit is decided (§2–§5);
  the spike half is executed and merged (§6, PR #6350); the
  contract clauses the spike could not prove are named as
  **[spike-owed]** with their owner (§7), not papered over.
- Parent: [#5681](https://github.com/developing-today/code/issues/5681)
  (serverless edition storage). Parent direction, recorded by Drew
  on #5681 (2026-11-04) and synthesised in
  `doc/2026-11-10_decision_id5681_serverless_storage.md`: **BOTH,
  with roles — Turso preferred, D1 first-class.** The synthesis's
  core idea is consumed, not re-decided: the signed journal (#806)
  is the system of record; every backend store is a materialisation
  of it; store replication is never the journal's replication.
- Signed journal replication:
  [#806](https://github.com/developing-today/code/issues/806),
  design `doc/2026-11-01_design_id806.md`. Serverless edition:
  #2046. Programme epic: #2056.
- Siblings consumed, not re-decided: #5863 (storage contract —
  **landed 2026-11-17**, `doc/2026-11-17_decision_id5863_storage_contract.md`,
  PR #11179; this record is written against contract v1 as landed),
  #5865 (Turso/libSQL journal fit — landed,
  `doc/2026-11-10_design_id5865.md`; §3 mirrors its layering rule
  for D1's platform replication), #5869 (backup/recovery/exit —
  landed), #5866 (local-dev parity — landed), #5868 (cost &
  limits guardrails — landed; it prices the limits §5 declares).
- Evidence grading (parent convention): **[landed]** = merged on
  main; **[documented]** = vendor documentation claim, not
  measured by us; **[spike]** = executed in §6; **[spike-owed]** =
  not knowable without running it, owned by §7.
- Gate history: this record was gated on #5863 settling, because
  the spike's schema and ordinal cursor were provisional while
  the contract was open (PR #6350). #5863 closed 2026-11-17 with
  the contract landed; nothing in the landed contract contradicts
  the spike's shape, so the provisional choices are confirmed
  below, with one correction (§2.3: the bookmark is not the
  stream cursor — the spike itself flagged this, and the contract
  now makes the distinction load-bearing).

## 1. The question

When the serverless edition's store for a journal is D1,
three of D1's behaviours meet #806's requirements head-on:

- **Ordering.** D1 is single-primary SQLite: writes serialise
  through one primary. Does that serialisation give the journal
  the single-head ordering #806 requires, and who assigns the
  sequence?
- **Sessions.** D1 reads are served from replicas that lag the
  primary; D1's answer is sessions with bookmarks (a bookmark
  carried into a later request constrains reads to see at least
  the state the bookmark was taken at). Does that give the
  contract's read-at-watermark and the writer's read-your-writes?
- **Replication.** D1 replicates platform-side for availability.
  What may that replication be trusted with, given #806's rule
  (inherited from the synthesis and #5865) that the signed stream
  is the only journal replication?

Not this record's question: D1 vs Turso vs both (#5681, decided
with roles); the contract surface itself (#5863, landed); how
libSQL composes with the journal (#5865); placement choice for
any specific shard (#5867, consuming the flags §5 declares).

## 2. Ordering: journal-layer sequence over a single primary

1. **Sequence is assigned by the journal layer, never by D1.**
   The append path assigns the per-journal sequence
   monotonically before the row is written, exactly as contract
   §2.1 requires on both backends ("the store never assigns
   sequence from a local counter that a second writer could
   fork"). A D1 `AUTOINCREMENT`/rowid is a physical accident of
   one database file; using it as journal order would couple the
   journal's meaning to one store's internals and break the
   #5869 exit story (export = journal + snapshot, replayable
   into any backend) — the same argument #5865 §3 makes against
   libSQL frame order.
2. **Single-primary serialisation is the enforcement, not the
   source, of order.** #806 assumes a single writer per world.
   On D1 that writer's appends serialise through the primary
   **[documented]**, so two appends cannot interleave at the
   head; the journal layer's sequence assignment is therefore
   race-free without a distributed lock. The spike stored the
   per-world ordinal explicitly alongside the entry BLOB and
   observed ordinals 1, 2, 3 in append order **[spike]**.
3. **Fork refusal is a uniqueness fact plus a typed refusal.**
   The journal table's primary key is `(journal, seq)`. An
   append whose `seq` is already occupied by a _different_
   signed entry fails the insert; the adapter surfaces contract
   §2.1's typed `fork` refusal carrying both entries — it never
   overwrites, and it never treats a same-`seq` _identical_
   entry (a retried append of the writer's own entry) as a
   fork: that is an idempotent no-op returning the existing
   sequence. Exercising the fork path against a real adapter is
   **[spike-owed]** (§7) — the spike had a single writer by
   construction and could not race itself.
4. **Entries are stored as exact bytes.** The journal row holds
   the `JournalEntry` JSONL bytes as a BLOB, byte-identical to
   what the writer signed **[spike]** — the spike's snapshot +
   ordered tail compared byte-for-byte with no gap, duplicate,
   or reorder. D1 never re-serialises an entry; integrity is
   store-independent because signatures and the hash chain
   travel with the bytes (contract §2.2, §3.3).

## 3. Sessions and bookmarks: the watermark mechanism on D1

1. **The bookmark is a session cursor, not the stream cursor.**
   #806's cursor is `(entry ordinal, rolling digest)`. A D1
   bookmark is an opaque platform token asserting "reads in
   this session see at least the state I was taken at". The two
   must never be conflated (the spike's own caveat, now a rule):
   the bookmark proves _freshness against D1's own replication_;
   the stream cursor proves _position in the signed journal_.
   A reader's position is always named by the stream cursor /
   contract watermark; the bookmark is only the mechanism that
   makes D1 honour it.
2. **Read-your-writes is preserved with compensation — via the
   bookmark path, and the compensation is now spike-proven.**
   The synthesis marked this cell **[spike-owed #5864]**; the
   spike discharges it: the writer's append returns a bookmark;
   carrying that bookmark into a fresh request/session yields
   read-your-write — the appended entries are visible, in
   order, byte-identical **[spike]**. Without a bookmark, a
   writer must not read its own appends from a possibly-lagging
   replica and call the result current.
3. **Read-at-watermark (contract §2.3) maps onto sessions.**
   A read at watermark _N_ is served by a session whose
   bookmark is at-or-past the append that assigned _N_; if the
   session cannot reach _N_, the adapter returns the contract's
   typed `unavailable` refusal carrying the store's current
   watermark — never silently stale data (§2.3: "replica lag is
   expressed only as a lower watermark"). Snapshot reads are
   the strong form: the spike took an explicit snapshot and
   read snapshot + ordered tail with exact byte equality
   **[spike]**, which is contract §3.2's snapshot + delta pair
   in D1 dress. Measuring watermark behaviour under real
   production-D1 replica lag (the spike ran on the local
   runtime) is **[spike-owed]** (§7); the _semantics_ are
   decided here regardless of the numbers.
4. **Followers read with watermarks, never with trust.** D1
   replicas are eventually consistent **[documented]**; the
   synthesis weakens replica reads by design on both backends.
   A #806 follower on D1 verifies signatures against the stream
   and reports its watermark (contract §2.4 `head()`); what a
   replica holds past its last verified watermark is
   unverified materialisation — present on disk, invisible to
   readers — the same rule #5865 §2.3 states for embedded
   replicas.

## 4. Replication: platform availability, never journal replication

1. **The signed stream is the only journal replication** —
   the synthesis/#5865 rule, applied to D1 unchanged. Every
   movement of journal data across a trust boundary is #806
   replication of exact signed bytes. D1's platform
   replication moves _rows inside one deployment's trust
   domain_ to make reads available and geographically close;
   it confers no journal authority, carries no signatures, and
   cannot detect a fork (§2.3's fork is detectable from the
   stream alone, on either store).
2. **Durability of an acknowledged append requires the same
   compensation as Turso.** Until an entry is exported /
   replicated over the signed stream, platform replication is
   its only durability; #5869's export cadence bounds the loss
   window identically on both backends (synthesis §6.1). D1
   Time Travel / point-in-time restore is a _platform_ backup
   of a materialisation: it is never a journal recovery path —
   recovery is snapshot + delta replay-in (contract §3.2–§3.3),
   because a platform restore can move a store _backwards_,
   and backwards convergence is precisely the fork shape the
   journal layer exists to detect (§2.3, #5865 §3).
3. **No embedded replica, and that is declared, not
   apologised for.** D1 has no endpoint-co-located replica
   story; endpoints are online-only. Contract §4.2 already
   declares the flags this record confirms: `embedded_replica`
   **false** for D1 (session/bookmark reads are the
   compensation, exposed through read-at-watermark),
   `offline_append` **false**, `workers_native` **true**.
   Placement consumes these (#5681 §5.1 rule 3–4: Workers-only,
   read-heavy, diffuse access → D1 eligible and preferred;
   endpoint-local-first reads → Turso). §5.1 rule 4's condition
   — "D1's session semantics spike-proven (#5864)" — is met by
   §6 for Workers-originated write shapes.

## 5. Declared limits (contract §4.1, D1 values)

- **Maximum stored bytes:** the D1 per-database ceiling —
  10 GB **[documented]** at the synthesis's writing, declared
  per store at placement time (the value is a plan fact, read
  before choosing, per contract §4.1; #5868 prices it, and
  synthesis §5.1 rule 2 keeps shards projected to cross 80% of
  the ceiling off D1).
- **Retention floor:** the contract requires the journal online
  at least 30 days (#5869 §2.2); D1 placement must honour it
  via the export cadence, not via platform retention promises.
- **Metering:** contract §5.2's `meters()` maps to D1's
  rows-read / rows-written / stored-bytes accounting
  **[documented]**; #5868's row-scan discipline is what keeps
  the meter honest on this backend.
- Exceeding a declared limit is the typed `limit` refusal
  (contract §5.1). Behaviour _at_ the ceiling in a real
  deployment is **[spike-owed]** (§7) — declared here so the
  refusal, not a silent truncation, is the specified outcome.

## 6. The spike (executed, merged)

PR #6350, `spike(id): probe D1 journal sessions (#5864)`,
merged 2026-11-12 (`pkgs/id/worker-poc`, local runtime —
Wrangler/Miniflare D1, Node 24.21.0 / Wrangler 4.147.0):

- `JournalEntry` JSONL bytes → D1 BLOB with a per-world
  ordinal; appends observed at ordinals 1, 2, 3, in order.
- A D1 bookmark carried to a fresh request gave
  read-your-write across requests.
- Explicit snapshot + ordered tail reproduced the journal
  byte-identically — no gap, no duplicate, no reorder.
- The companion Rust-side feed tests passed
  (`replication_feed_snapshots_exact_bytes_and_follows_durable_appends`,
  tamper-refusal tests in the same run).

Scope honesty: the spike is a local-runtime probe, not a
production-D1 measurement and not a D1 adapter. It proves the
_shape_ (§2–§3 mechanisms compose as specified); production
replica-lag numbers and adapter-level refusal behaviour remain
owed (§7). The spike was Drew-owned and is consumed here as
evidence, untouched.

## 7. What remains spike-owed (owned, not silent)

1. **Fork-refusal under contention** (§2.3): a two-writer race
   against a real D1 adapter must surface the typed `fork`
   refusal with both entries. Owner: the first D1 adapter
   implementation; the contract suite (#5863 §7) is where it
   must pass.
2. **Watermark behaviour under production replica lag**
   (§3.3): bookmark-constrained reads against production D1,
   measuring the lag window the `unavailable` refusal reports.
   Owner: the same adapter's conformance run.
3. **Ceiling behaviour** (§5): the typed `limit` refusal at
   the declared stored-bytes ceiling, in deployment.
   Owner: #5868's guardrail checks against the adapter.

A spike-owed item that comes back negative is not compensated
locally: per the synthesis, a backend that cannot expose a
contract operation without weakening a #806 guarantee is
reversal condition **R1** — a #5681-level event, recorded
there.

## 8. Alternatives rejected

- **D1-assigned order** (rowid/autoincrement as journal
  sequence): couples journal meaning to one store's internals
  and breaks backend-independent replay (§2.1).
- **Bookmark as the journal cursor**: an opaque platform token
  cannot carry the digest chain; followers would be trusting
  the store instead of verifying the stream (§3.1).
- **Trusting replica reads without the watermark rule**: silent
  staleness is the one failure contract §2.3 exists to forbid.
- **Time Travel as journal recovery**: platform restore can
  move a store backwards; recovery is replay-in, on both
  backends identically (§4.2).
- **Waiting for a production-D1 spike before recording the
  decision**: the local-runtime spike plus vendor documentation
  settle the semantics; the owed items (§7) are measurements
  and adapter behaviours with named owners in the contract
  suite, and leaving the record unwritten was the actual gap
  (this issue's wrongful-closure history, 2026-11-14 audit).

## 9. Parent-decision impact

**None.** This record changes no #5681 decision: both-with-roles
stands, Turso stays the default, D1 stays first-class, and the
synthesis's two **[spike-owed #5864]** cells (read-your-writes
via the session/bookmark path; §5.1 rule 4's spike-proven
condition) are discharged by §6 rather than re-decided. Where
this record would have had to change a parent decision, it
would have said so explicitly (§5864's filing instruction);
it does not.

<!-- agent-note:v1 agent=repo-worker-code lane=code-d state=design-record task=19932 origin=muse-scheduled-lane -->
