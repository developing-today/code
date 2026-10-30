# Hot-world sharding — investigation: when one world outgrows one Durable Object

**Date:** 2026-10-30 · **Question owner:** Drew's ID program directive
(2026-10-28), parent epic
[#2056](https://github.com/developing-today/code/issues/2056)
**Scope:** The hosted model puts one world in one Durable Object, which
has a ~1,000 req/s soft ceiling and single-threaded execution. What
happens when a world gets hot: sharding and overflow strategies, the
signals that should trigger each, and what sharding does to the world's
journal and semantics. The base hosted fit is
[#2044](https://github.com/developing-today/code/issues/2044)'s (done);
the serverless edition v1 design that consumes this document is
[#2046](https://github.com/developing-today/code/issues/2046)'s.
**Method:** This is the wave-2 write-up of the investigation whose
findings were first recorded on
[#2059](https://github.com/developing-today/code/issues/2059) by queue
task 3361 (2026-10-29), grounded in the `WorldMetrics` surface in
`pkgs/id/src/world/metrics.rs`. That pass verified the model and left a
program-workspace draft; the in-repo document was never written. This
document formalises those findings, re-verified against current main
(2026-10-30: `metrics.rs` signal fields, the `SetCrossWorldWrite`
machinery in `src/world/actor.rs`, and `WorldLimits` in
`src/world/mod.rs` all confirmed present). Strategy judgments are
labelled conclusions, not platform facts. No proof of concept is built
here, and no permanent feature may be built on this investigation
without a review round (program rule, epic #2056).

---

## TL;DR verdict

**Do not split a hot world's journal first.** The journal *is* the
world's consistency story: the actor is serial by design, and every
sharding step trades a slice of that single-writer simplicity for
throughput. The escalation ladder is:

0. **Shed load that never needed the actor** — reads served from
   checkpoint snapshots, ephemeral traffic never journaled, WebSocket
   frame batching. Most apparent hot worlds stop here.
1. **Partition by topic/channel inside the world** — own journal
   segment per topic; the world keeps one identity and membership.
2. **Partition by region** into sibling shard-worlds under a thin
   coordinator — raises aggregate req/s and aggregate storage across
   objects; cross-region atomicity explicitly does not exist.
3. **Spill execution to native nodes** under a journaled single-writer
   lease — for indivisible load that stays critical after rungs 0–2;
   temporary per world, never the default hosting shape.
4. **Hierarchical objects** — aggregation and read fan-out *only*;
   explicitly not a write-throughput strategy.

Every rung has named trigger signals (§2, §4), all but one already
present in `WorldMetrics`, and every rung needs a de-escalation signal
or hot-world machinery becomes permanent complexity bought for a
one-off spike. Explicitly rejected: splitting one journal across
objects with a multi-writer merge protocol (§5).

---

## 1. The ceiling, precisely

From #2044 (Cloudflare Durable Objects limits/pricing, cited there):

- **~1,000 req/s soft ceiling per Durable Object**; beyond it requests
  queue, then return "overloaded" errors. WebSocket messages bill 20:1
  (100 incoming messages = 5 requests), so *message* throughput is not
  the same as request throughput — batching small packets into one
  frame raises the effective cap.
- **Single-threaded execution**, requests serialized per object. CPU
  per invocation: 30 s default, 5 min max; 128 MB per isolate.
- **Rows written are the scarce currency** ($1.00/M written vs
  $0.001/M read): a journaled event per input is the cost driver, not
  the read fan-out.
- **10 GB storage cap per object** — the second ceiling. No strategy
  raises the per-object limit itself; partitioning (rungs 1–2)
  raises *aggregate* capacity when the data actually partitions
  across objects.

The native side has the same shape of wall at a different height: the
native `WorldActor` is also serial per world, and every journal append
fsyncs before returning (`journal_append_latency` in `metrics.rs` is
documented as "including the fsync"). The native ceiling is therefore
fsync latency × command cost, not core count. **Sharding strategy must
be transport-neutral: it applies to a hot native world too.**

## 2. Signals — what to measure, and what each one means

All primary signals already exist in `WorldMetrics` / `MetricsSnapshot`
(`pkgs/id/src/world/metrics.rs`, surfaced by `id world stats`):
`commands_total`, `command_latency`, `journal_appends_total`,
`journal_append_latency`, `checkpoints_total`, `checkpoint_duration`,
`frame_bytes_total`, `actor_queue_depth`, `actor_queue_depth_max`,
`sessions`. No new instrumentation is needed to *detect* a hot world;
what is missing is (a) one counter and (b) a consumer that acts on the
snapshot — both filed as follow-on
[#2275](https://github.com/developing-today/code/issues/2275).

| Signal | Healthy | Warning | Critical | What it diagnoses |
| --- | --- | --- | --- | --- |
| Sustained command rate vs ceiling | < 40% | 40–70% for 10 min | > 70% for 10 min, or any "overloaded" errors | Approaching the DO soft cap / native actor saturation |
| Sampled `actor_queue_depth` trend | ~0–few | Growing across snapshots | Monotonic growth over 3+ snapshots | Arrival rate > service rate — the definition of hot; queueing, not CPU, is the pain |
| `command_latency` p95 | < 10 ms bucket | 10–50 ms | > 50 ms, mass in overflow bucket (> 1 s) | End-to-end handler cost too heavy — but this metric times the whole command handler, journal persistence included (`actor.rs`: `record_command` wraps `handle`), so it cannot by itself separate a slow program from slow storage. Call it a slow *program* only with corroboration (journal latency healthy, or dedicated program-execution timing); uncorroborated, treat it as storage-bound (next row) |
| `journal_append_latency` p95 | < 2 ms | 2–10 ms | > 10 ms | Storage/fsync bound (native) or row-write contention (hosted) — journal batching/checkpoint tuning before sharding |
| `checkpoint_duration` vs interval | < 5% of interval | 5–25% | > 25% | Journal too large between checkpoints (`checkpoint_every: 1000` default); raise checkpoint frequency / snapshot size before structural change |
| `sessions` + `frame_bytes_total` growth | — | Fan-out bytes growing faster than command rate | Egress/frame cost dominates | Read fan-out problem, not a write problem — rung 0/4, not journal sharding |
| Write locality (NEW: needs a counter, #2275) | One topic/region > 60% of writes | — | — | Whether partition-by-topic/region can actually split the load; a world whose writes are all one topic cannot be topic-sharded |

**One signal caveat:** `actor_queue_depth_max` is a lifetime
high-water mark — `record_queue_depth` uses `fetch_max` and snapshots
never reset it — so it can prove a burst *happened* but cannot show
growth or recovery inside a window. The queue-depth trigger above
therefore uses sampled `actor_queue_depth`; computing a windowed
maximum is part of what #2275's detector must add.

**Trigger discipline:** act only on **sustained** signals (10-minute
windows, 3+ snapshots), never on a spike — worlds are bursty by nature
(a game event, a meeting start). **De-escalation:** shards re-merge /
spill ends when the warning band clears for ~1 hour. And one
anti-trigger bears repeating: a slow *program* (latency-bound, not
rate-bound) is a profiling problem — sharding is not the fix.

## 3. The strategies, in escalation order

### Rung 0 — Shed what never needed the actor (do first, always)

- **Reads off the actor:** snapshots and retained-event catch-up
  (`retained_events: 1024`, `snapshot_bytes: 512 KiB` in `WorldLimits`)
  can be served from the last checkpoint snapshot + journal tail by a
  read replica / KV-cached snapshot, not by serial actor commands.
- **Ephemeral traffic never journals:** presence, cursors, in-flight
  game inputs that don't settle — per #2044 §5 and the ephemeral-tier
  finding (#2034, closed), these flow over WebSockets unjournaled;
  only settled events become rows. This directly cuts
  `journal_appends_total`, the scarce currency.
- **Batching:** many small packets per WebSocket frame (20:1 message
  billing; Cloudflare's own guidance in #2044 §1.3).

*Trigger:* any warning-band signal. *Consistency cost:* none —
nothing about the journal or its ordering changes. *Conclusion:* a
large fraction of apparent hot worlds are fan-out/ephemeral problems
wearing a sharding costume.

### Rung 1 — Partition by topic/channel inside the world

Split the world's *logical* spaces (chat channels, board sections,
game lobbies) so each topic has its own journal segment and its own
actor/DO, while the world keeps one identity, one membership list, one
capability root. Cross-topic operations (a moderator action, a
cross-channel post) go through the world coordinator — the existing
cross-world write machinery (`WorldCommand::SetCrossWorldWrite`,
journaled `CrossWorldWrite` entries in `src/world/actor.rs`) is the
native precedent for journaled, policy-gated cross-partition writes.

- *Trigger:* critical queue-depth/rate signals **and** write-locality
  shows ≥ 2 topics each carrying a meaningful share (no single topic
  > ~60% of writes).
- *Consistency cost:* low — topics are near-independent by definition;
  global ordering is lost *across* topics, kept within each.
- *Failure mode:* the hot topic *is* the whole world (one channel
  carries everything) — partitioning changes nothing; go to rung 2/3.

### Rung 2 — Partition by region (sibling shard-worlds)

For spatially-shaped worlds (games, maps, collaborative canvases):
shard by region into sibling worlds, each a full actor/DO with its
own journal, under a thin coordinator world that holds membership,
the region map, and cross-region handoffs. An entity crossing a
boundary is a journaled export from region A + import to region B —
a two-step handoff, not a distributed transaction; brief inconsistency
at the seam is accepted and visible.

- *Trigger:* critical signals **and** locality is spatial
  (participants cluster; cross-region interactions < ~10% of writes).
- *Consistency cost:* medium — seams are real: cross-region atomicity
  does not exist, and the design must say so rather than fake it.
- *Platform note:* regions spread load across independently capped
  objects, raising aggregate req/s and aggregate storage (no
  per-object limit changes). Topic partitioning (rung 1) does the
  same for storage *if* its hosted mapping is separate DOs per
  topic rather than segmented tables in one DO — the mapping §8
  leaves open — so a storage-bound non-spatial world is a rung-1
  candidate, not automatically a rung-2 one.

### Rung 3 — Spill execution to native nodes

Overflow execution moves to a native `id` node: the hosted DO keeps
the journal of record (or mirrors it), and a native node — no
1,000 req/s cap, no 128 MB isolate, real fsync/journal economics —
runs the hot world's program turns, with results journaled back as
signed events. This is #2044's bridge model applied per-world:
signatures are transport-independent, so the trust model survives.

- *Trigger:* a single indivisible topic/region stays critical after
  rungs 0–2 (the locality check fails: the load cannot be
  partitioned), **or** corroborated program-execution cost (§2's
  caveat: raw `command_latency` alone does not show this) proves
  the world program itself needs native CPU/Wasm headroom the
  isolate can't give.
- *Consistency cost:* medium-high — one writer at a time must be
  enforced by a journaled lease/handoff; split-brain (DO and native
  node both accepting writes) is the catastrophic failure, and the
  lease is the whole design. The lease protocol is filed as a design
  follow-up from this investigation (§7).
- *Cost note:* spill reintroduces exactly the standing-container
  spend the serverless directive caps — it is per-hot-world and
  temporary (de-escalation signal, §2), never the default hosting
  shape.

### Rung 4 — Hierarchical objects (aggregation / fan-out only)

A parent DO aggregates child worlds: presence roll-ups, leaderboards,
search indexes, rendered summaries. Children keep their own journals;
the parent consumes their settled events and serves reads.

- *Trigger:* `frame_bytes_total` / read fan-out critical while write
  signals stay healthy — the parent's job is absorbing reads, not
  writes.
- *Hard limit:* a hierarchy does **not** raise write throughput for a
  single hot child. Presenting it as a write-sharding strategy would
  be inventing a capability the mechanism doesn't have; it is listed
  here so the synthesis (#2046) scopes it correctly.

## 4. Strategy comparison — the consistency cost, explicit

| Strategy | Raises which ceiling? | Ordering guarantee after | Atomicity after | Is it still one world? | Trigger (sustained) |
| --- | --- | --- | --- | --- | --- |
| 0 — Shed non-actor load | Effective (fewer actor commands) | Unchanged: one journal, total order | Unchanged | Yes, trivially | Any warning-band signal |
| 1 — Topic partition | Req/s (per topic); aggregate storage too, under the separate-DO mapping (§8) | Total within a topic; none across topics | Cross-topic ops via journaled coordinator writes, not transactions | Yes — one identity, membership, capability root; partitioned journal | Critical rate/queue + no topic > ~60% of writes |
| 2 — Region shards | Req/s **and** aggregate storage | Total within a region; none across regions | None across regions; export/import handoff at seams | Federated — sibling worlds under one coordinator; membership/region map stay in the coordinator | Critical + spatial locality, cross-region writes < ~10% |
| 3 — Native spill | Req/s and CPU (native headroom) | Unchanged *if* the lease holds: still one writer | Unchanged under the lease | Yes — execution moved, journal and identity did not | Indivisible load critical after 0–2, or program needs native headroom |
| 4 — Hierarchy | Read fan-out only | Unchanged (children untouched) | Unchanged | Yes — parent is a derived view, not a writer | Read fan-out critical, write signals healthy |

## 5. What sharding does to the journal — and the rejected alternative

The journal is not an implementation detail; it is how a world's
history is audited and replayed. So the question "does a sharded
world stay one world?" (§4, last column) is really a journal question:

- **Rung 0/3/4 leave the journal singular.** Zero, spill (under its
  lease), and hierarchy preserve one journal, one writer, one total
  order. Their semantics cost is zero *by construction* — which is
  exactly why they bracket the ladder.
- **Rung 1 partitions the journal along lines the world already
  has.** Topics were never meaningfully ordered against each other;
  what is given up (cross-topic total order) was mostly not load-
  bearing. Membership and capabilities do *not* partition — they stay
  in the world root, which is what keeps it one world.
- **Rung 2 is the honest break.** Shard-worlds have separate journals
  and no shared order; the coordinator's journal records only
  membership, the region map, and handoffs. A sharded-by-region world
  is one world *socially* (one name, one membership) and several
  worlds *mechanically*. Designs and UIs must not paper over the
  seam: an entity mid-handoff is in transit, visibly.
- **De-escalation is a state problem, not a journal trick.**
  Re-merging shards is *not* journal concatenation + replay:
  `WorldCore::restore` (`src/world/core.rs`) validates a single
  world identity and rejects a second `Created` header, and
  independently compacted shards have independently allocated
  participant IDs, sequences, and program snapshots to reconcile.
  A re-merge needs its own state-import/reconciliation design
  (export one shard's state as an import into the survivor, with
  ID/sequence remapping), which does not exist today. Until that
  design exists — an open question for #2046 (§8) — rungs 1–2 are
  one-way doors taken only on sustained signals: another reason
  for the trigger discipline in §2.

**Explicitly rejected: splitting one journal across DOs with a
multi-writer merge protocol (CRDT journal).** It destroys the
serial-actor simplicity that makes world semantics auditable, for a
case rungs 1–3 cover. Revisit only if a workload is proven both
unpartitionable and unspillable. Also rejected: raising limits by
plan/negotiation — the ~1,000 req/s figure is a soft platform
ceiling, not an account quota the design may assume away.

## 6. Decision summary

```text
hot signal (sustained, §2)
 ├─ fan-out/ephemeral dominated ──────────────► rung 0 (+ rung 4 if reads stay hot)
 ├─ multi-topic, no dominant topic ───────────► rung 1 (topic partition)
 ├─ spatial locality, low cross-region writes ► rung 2 (region shards)
 ├─ indivisible load, still critical ─────────► rung 3 (native spill, journaled lease)
 └─ single slow program (latency, not rate) ──► profile the program; sharding is not the fix
```

## 7. Follow-up routing

Already filed before this document (not duplicated):

- [#2275](https://github.com/developing-today/code/issues/2275) —
  write-locality metrics (per-topic/per-region write split in
  `WorldMetrics`) + the sustained-threshold hotness detector that
  consumes `MetricsSnapshot` against §2's bands. This is the buildable
  gap the trigger design depends on: without the locality counter,
  rungs 1–2 cannot be triggered responsibly.

Filed from this investigation:

- **Region-seam handoff format** — [#2818](https://github.com/developing-today/code/issues/2818): design the journaled export/import
  record pair for rung 2 (what each side journals, what a client
  observes mid-handoff, how a failed import is recovered), so the
  seam semantics in §5 are implementable rather than aspirational.
- **Native-spill write lease protocol** — [#2819](https://github.com/developing-today/code/issues/2819): design the journaled lease
  grant/revoke/handoff for rung 3 (who grants, how both sides prove
  the lease moved, fencing against a partitioned ex-holder), the
  safety core of the spill strategy.

## 8. Hand-off notes for the #2046 synthesis

#2046 (serverless edition v1) consumes this document. Explicitly
handed off, not decided here:

- **Hosted-edition mapping for rung 1:** topics as separate DOs
  sharing one world-name prefix vs one DO with segmented SQLite
  tables — the 10 GB and req/s caps argue for separate DOs;
  membership/capability sharing argues the other way.
- **Shard re-merge / de-escalation mechanics** (§5): design a
  state-import/reconciliation path (journal concatenation + replay
  is not one — `WorldCore::restore` rejects a second `Created`
  header), or declare rungs 1–2 one-way and price that into the
  trigger thresholds.
- **Threshold defaults:** §2's bands are investigation-grade starting
  points from the wave-1 pass; the synthesis should confirm them
  against #2275's detector implementation experience before they
  become operator defaults.
- **Region-seam and spill-lease designs** land via the §7 follow-ups;
  the synthesis adopts their outcomes rather than re-deriving them.

## 9. Sources

- Issue #2059 and its wave-1 investigation result (queue task 3361,
  2026-10-29) — the findings this document formalises.
- Subject system, verified on main 2026-10-30: `pkgs/id/src/world/
  mod.rs` (serial actor model, `WorldLimits`: `retained_events:
  1024`, `snapshot_bytes: 512 KiB`), `src/world/metrics.rs`
  (`WorldMetrics`/`MetricsSnapshot` fields cited in §2),
  `src/world/actor.rs` (`WorldCommand::SetCrossWorldWrite`,
  journaled `CrossWorldWrite` entries).
- Platform ceilings: #2044 investigation
  (`doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/`)
  and its cited Cloudflare Durable Objects limits/pricing and
  WebSocket-hibernation docs; open question 3 of that investigation
  is this document's source question.
- Program context: #2056 (epic), #2046 (serverless edition v1,
  consumer), #2034 (ephemeral vs chained data, closed), #2275
  (signals follow-on).
