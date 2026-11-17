# Decision: the serverless storage contract (journal, world state, indexes)

- Date: 2026-11-17
- Status: **decision record** — the minimal storage contract both
  serverless backends (D1 and Turso/libSQL) must satisfy is decided
  (§2–§5). Backend-specific parameters are stated as declared limits
  and capability flags inside the contract (§4), never as behaviour
  differences above it.
- Issue: [#5863](https://github.com/developing-today/code/issues/5863),
  follow-on of [#5681](https://github.com/developing-today/code/issues/5681)
  (serverless edition storage — D1 vs Turso vs both), programme epic
  #2056. Serverless edition itself: #2046. Signed journal: #806.
- Parents and siblings consumed, not re-decided: the #5681 synthesis
  (`doc/2026-11-10_decision_id5681_serverless_storage.md`, §6.2) fixes
  the seven parent requirements this contract must satisfy — §6 below
  maps each one to the contract clause that discharges it.
  [#5869](https://github.com/developing-today/code/issues/5869)
  (backup, recovery & exit, landed) contributes hooks 1–4 of that
  list. [#5864](https://github.com/developing-today/code/issues/5864) /
  [#5865](https://github.com/developing-today/code/issues/5865)
  (per-backend journal-fit spikes) establish what each backend can
  expose; where a spike is still owed, this record names the contract
  clause it must prove rather than assuming the result.
  [#5866](https://github.com/developing-today/code/issues/5866)
  (local-dev parity, landed) contributes the rule that no
  backend-specific feature may leak above the contract.
  [#5867](https://github.com/developing-today/code/issues/5867) owns
  backend choice and migration mechanics and consumes this contract;
  [#5868](https://github.com/developing-today/code/issues/5868) prices
  the limits declared here.
- **Parent-decision impact: none.** Nothing here changes a decision
  #5681 carries (both-with-roles, per-shard / per-deployment choice
  granularity, Turso default, D1 first-class). If a landed spike
  shows a backend cannot expose a §2–§3 operation without weakening
  a #806 guarantee, that is reversal condition R1 of the synthesis —
  a #5681-level event, recorded there, not compensated here.

## 1. The one idea everything follows from

The signed journal (#806) is the system of record; every backend
store is a materialisation of it (synthesis §2, #5869 §1). The
contract is therefore the *materialisation interface*: it is the
smallest set of operations that lets any conforming store be
built, read, verified, and rebuilt from the journal alone. A
backend is a choice exactly when everything above this interface
is indifferent to which store sits below it.

## 2. Journal operations (the contract core)

1. **Append** — `append(entries) -> seq`. Appends one or more
   signed entries at the single head of a named journal and returns
   the sequence assigned to the last entry. Sequence is assigned by
   the journal layer, monotonically, with no gaps visible to
   readers (synthesis §4.1); the store never assigns sequence from
   a local counter that a second writer could fork. Appending an
   entry whose sequence is already occupied by a *different* signed
   entry is a fork event: the store must surface it as a typed
   `fork` refusal carrying both entries, never overwrite.
2. **Read range** — `read(from_seq, to_seq | open) -> entries`.
   Returns entries in sequence order, each with its timestamp and
   signature intact. A read that cannot return the full requested
   range returns a typed refusal (§5); partial results are never
   presented as complete.
3. **Read at watermark** — `read_at_watermark(seq) -> snapshot view`.
   The only freshness primitive above the contract (synthesis
   §6.2.5): a reader that needs state as of sequence *N* asks for
   *N*; the store either serves a view that includes every entry
   up to *N* or refuses with its current watermark. Replica lag is
   expressed only as a lower watermark, never as silently stale
   data. Read-your-writes is the special case where the writer
   reads at the sequence its own append returned.
4. **Head** — `head() -> {seq, timestamp, watermark}`: the highest
   appended sequence, its timestamp, and the highest sequence this
   store can currently serve at-watermark. Followers report their
   watermark from this call (#806 followers verify signatures
   against the stream; the watermark is how lag is named).

## 3. World state, snapshots, indexes

1. **World state** — `get(key)`, `put(key, value, at_seq)`,
   `delete(key, at_seq)`, `scan(prefix, at_seq)`. World state is
   derived state: every mutation is stamped with the journal
   sequence that caused it, and every read names the sequence it
   is read at (defaulting to the store watermark). There is no
   unsequenced write path — a mutation that did not come from a
   journal entry does not exist above the contract.
2. **Snapshot + delta** — `snapshot(at_seq) -> snapshot_ref` and
   `delta(from_seq, to_seq) -> entries` (synthesis §6.2.2). A
   snapshot is a complete world-state image as of a named sequence,
   stamped with the schema/contract version (§5.3). Snapshot plus
   the delta after its sequence must reproduce the store exactly;
   this pair is the only rebuild, migration, and recovery mechanism
   (synthesis §6.5, #5869 §2).
3. **Replay-in** — `replay_in(bundle) -> watermark` (synthesis
   §6.2.3): load an export bundle (journal + snapshot, open format,
   #5869 §4) into an empty store, preserving sequence, timestamps,
   and signatures verbatim. Replay-in verifies the hash chain and
   signatures as it loads and refuses a bundle that fails
   verification; it never repairs silently.
4. **Indexes** — `index_query(index, key_range, at_seq) -> keys`.
   Indexes are declared in the schema (name, source key pattern,
   indexed fields) and are *derived like world state*: an index is
   correct iff it equals the index computed from world state at the
   same sequence. Index reads carry the same at-sequence semantics
   as world-state reads. An index that cannot be served at the
   requested sequence is a typed refusal, not a scan of a different
   sequence. Full scans are an implementation detail of an index
   build, never a query path above the contract.

## 4. Declared limits and capability flags (where backends differ)

Backends differ only inside this section, and only by *declared*
values a caller can read before choosing a placement — never by
silent behaviour differences.

1. **Limits, declared per store**: maximum stored bytes, maximum
   entry/value bytes, maximum appends per second sustained, and
   the retention floor the store can honour (#5869 §2.2: journal
   online at least 30 days). A store asked to exceed a declared
   limit refuses with the typed limit refusal (§5); #5868 prices
   and guards these numbers.
2. **Capability flags, declared per store**:
   - `offline_append` — the store can accept journal appends from
     a disconnected endpoint. D1: **false** (endpoints are
     online-only). Turso: **false as journal commits** — embedded
     replicas are read replicas; offline writes are client-side
     intents merged by append order at reconnect (synthesis §6.1),
     so the flag is false on both and the intent mechanism lives
     above the contract, identically for both.
   - `embedded_replica` — read replicas co-located with the
     reader. Turso: **true**. D1: **false** (replication is
     platform-side; session/bookmark reads are the compensation,
     exposed through read-at-watermark, §2.3).
   - `workers_native` — reachable from a Worker with no extra
     vendor credentials. D1: **true**. Turso: **false**.
   Flags inform placement (#5681 §5, #5867); no code above the
   contract may branch on a flag to change semantics — only to
   choose where a store is placed or to refuse a placement that
   requires a flag the store lacks.

## 5. Refusals, versioning, metering

1. **Typed refusals** (synthesis §6.2.6): every operation can fail
   only as one of `limit` (a §4.1 declared limit, naming it),
   `unavailable` (store unreachable or below the requested
   watermark, carrying the current watermark), `fork` (§2.1),
   `version` (§5.3), or `verification` (§3.3). No silent
   truncation, no silent redirect to another store, no partial
   success presented as success.
2. **Metering** (synthesis §6.2.7): `meters() -> {rows_read,
   rows_written, stored_bytes}` per store, readable without a full
   scan. Both backends expose these natively enough for the
   contract suite to check; #5868 consumes them.
3. **Version stamp**: every snapshot and export bundle carries the
   schema/contract version that produced it. A store refuses
   (`version`) a bundle newer than it understands; an older bundle
   is loaded only through the recorded migration for that version
   pair. Contract version 1 is the shape in §2–§5 of this record.

## 6. Parent requirements, discharged

| Synthesis §6.2 requirement | Contract clause |
| --- | --- |
| 1. Monotonic sequence visible to readers, timestamp per entry | §2.1, §2.4 |
| 2. Snapshot + delta read at a named sequence | §3.2 |
| 3. Replay-in preserving sequence, timestamps, signatures | §3.3 |
| 4. Schema/contract version on every snapshot and export | §5.3 |
| 5. Read-at-watermark as the only freshness primitive | §2.3 |
| 6. Typed refusal semantics, no silent truncation/redirect | §5.1 |
| 7. Per-shard metering without a full scan | §5.2 |

The #806 guarantee table (synthesis §6.1) is preserved by
construction: ordering and fork detection live in §2, integrity is
store-independent because signatures and the hash chain travel with
the entries (§2.2, §3.3), and the two weakened-by-design rows
(replica reads, durability of an acknowledged append before export)
are expressed as watermark and export-cadence rules above the
contract, identically on both backends.

## 7. Conformance: the contract suite owns "both"

Per synthesis §6.4, a shared contract-test suite runs in CI against
both backends on the same fixture journals (#5866's replay fixtures
are the corpus): append/read ordering, fork refusal, watermark
reads, snapshot+delta equivalence, replay-in round-trip with
signature verification, index-at-sequence reads, typed refusals at
declared limits, and meter sanity. A backend that fails the suite
is not shipped that release; two consecutive releases is synthesis
reversal condition R2. The per-backend spikes (#5864, #5865) prove
the clauses this record marks spike-owed — D1 session/bookmark
semantics under Workers for §2.3, Turso watermark reads from
embedded replicas for §2.3/§4.2 — and a spike that cannot prove a
clause triggers R1 at the #5681 level.

## 8. Alternatives rejected

- **Backend-specific APIs above the contract** (D1 SQL in app code,
  libSQL-only features in the world layer): rejected — that is the
  fork this contract exists to prevent (#5866 parity rule).
- **Store-as-truth with journal as audit log**: rejected — it makes
  exit and rebuild provider-dependent (#5869 §1) and makes "both"
  a second source of truth.
- **Per-world backend choice for unsharded worlds**: rejected in
  the synthesis (§6.3) and not reopened here; choice granularity
  stays per-deployment (world primaries) and per-shard
  (atlas/corpus).
- **Live dual-write migration**: rejected — one path (export,
  replay-in, verify, serve) serves migration, placement change,
  recovery, and exit (synthesis §6.5).
- **Unsequenced convenience writes for world state**: rejected —
  they would be unreplayable by construction (§3.1).

## 9. What this record does not decide

Implementation timing (the parent notes it may be months out),
backend choice for any specific deployment (#5867), prices and
guardrail thresholds (#5868), and the D1/Turso spike results
(#5864/#5865) — each stays with its owner, consuming this contract
as written.
