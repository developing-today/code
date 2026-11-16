# Decision: cost & limits guardrails for serverless storage

- **Date:** 2026-11-08
- **Status:** Decision record. No implementation is included here.
- **Issue:** [#5868](https://github.com/developing-today/code/issues/5868)
- **Parent design:** [#5681](https://github.com/developing-today/code/issues/5681), serverless
  edition storage — D1 vs Turso vs both. Programme epic: [#2056](https://github.com/developing-today/code/issues/2056).
- **Scope:** quotas, backpressure, and operator-visible warnings when a world approaches a
  D1 or Turso ceiling, at single-user, small-community, and large-public-world scale.

## What the parent already decided (not re-decided here)

Per Drew's architecture direction recorded on #5681 (2026-11-04/05), the storage answer is
**both, with roles**: Turso is the definite preference and endpoints read from embedded
replicas; very large corpora place shards per-shard across Turso *and* D1 by size, heat and
access pattern; cold tiers sink to the home lab. This record therefore assumes a
**multi-backend, multi-shard** substrate and does not pick a winner.

Two consequences follow directly, and are the reason this ticket exists separately:

1. A "ceiling" is no longer a single account-level number. It is **per backend, per plan,
   per shard**, and a world may hold shards on both platforms at once.
2. A single exhausted backend must not take the whole world down. Guardrails have to be
   **placement-aware**: they report and enforce against the shard that is running out, while
   the rest of the world keeps serving.

## Vendor ceiling facts used as evidence

Recorded from vendor documentation read on 2026-11-08. Treat every number as *configuration*,
not as a constant (see the "why the numbers are configuration" section).

**Cloudflare D1** — [pricing](https://developers.cloudflare.com/d1/platform/pricing/),
[limits](https://developers.cloudflare.com/d1/platform/limits/),
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/),
[free-tier enforcement changelog](https://developers.cloudflare.com/changelog/post/2026-09-01-d1-free-tier-limit-enforcement).

| Dimension | Workers Free | Workers Paid |
| --- | --- | --- |
| Rows read | 5 million / day | first 25 billion / month, + $0.001 / million |
| Rows written | 100,000 / day | first 50 million / month, + $1.00 / million |
| Storage | 5 GB total | first 5 GB included, + $0.75 / GB-month |
| Databases | 10 | 100,000 included, + $0.10 / database / month |
| Max database size | 500 MB | 10 GB (cannot be raised) |
| Max account storage | 5 GB | 1 TB (raisable) |
| Max row / BLOB size | 2 MB | 2 MB |
| Max query result | 50 MB (effective 2026-11-04) | 50 MB |
| Queries per Worker invocation | 50 | 1,000 |
| PITR (Time Travel) | 7 days | 30 days |

Three properties matter for guardrails:

- **D1 meters every query for you.** Each response carries `meta.rows_read` and
  `meta.rows_written`. Metering needs no extra query and no full scan.
- **Free tier fails closed, paid tier does not.** Since 2026-09-01, exceeding the free daily
  read/write limits makes queries fail until 00:00 UTC. On Workers Paid there is **no hard
  spending cap**; overage is simply billed. A runaway backfill on the paid tier is an
  invoice, not an outage.
- **Throughput is per database, not per account.** Each D1 database is single-threaded;
  a 100 ms average query caps it at ~10 queries/second, and a full queue returns an
  "overloaded" error. Saturation is therefore a *per-shard* limit event that looks nothing
  like a quota error and must not be reported as one.

**Turso** — [pricing](https://turso.tech/pricing),
[usage & billing](https://docs.turso.tech/help/usage-and-billing),
[limits](https://docs.turso.tech/cloud/limitations).

| Dimension | Free | Developer ($4.99/mo) | Scaler ($24.92/mo) |
| --- | --- | --- | --- |
| Databases | 100 | unlimited | unlimited |
| Storage | 5 GB | 9 GB, + $0.75/GB | 24 GB, + $0.50/GB |
| Rows read / month | 500 M | 2.5 B, + $1/B | 100 B, + $0.80/B |
| Rows written / month | 10 M | 25 M, + $1/M | 100 M, + $0.80/M |
| Embedded syncs | 3 GB | 10 GB | 24 GB |
| PITR | 1 day | 10 days | 30 days |

The Turso pricing page currently displays Developer at $4.99/month and Scaler at
$24.92/month with "save" amounts, i.e. annual-billing equivalents; monthly-billing
prices may be higher. Cost examples below use the displayed annual equivalents and
exclude Workers request/CPU overages and other platform services. Reconfirm billing
cadence and terms when provisioning; the page currently notes Turso is joining
Supabase, so these are not long-term price guarantees.

Three properties matter here:

- **Turso's hard failure mode is a typed error.** A query that exceeds a plan quota returns
  the `BLOCKED` error code. That is better than a silent truncation, but it arrives *at the
  call site*, which for us can be the middle of a journal append.
- **Billable overages are opt-in.** Overages can be enabled or disabled per plan. The
  guardrail must know which posture a deployment is in, because it changes whether "near the
  ceiling" means "slow down and keep working" or "you are about to stop".
- **The per-query meter exists too.** Turso's SQL-over-HTTP result carries `rows_read`,
  `rows_written` and `query_duration_ms` per statement, and a database-level usage endpoint
  reports rows read/written plus total storage size. The meter can therefore be advanced from
  the call site and reconciled against the account record, exactly as on D1.

Also relevant: Turso's own accounting is **row scans**, not rows returned. An unindexed
`SELECT … WHERE` or a `count(*)` bills every row it walked. Any guardrail that measures the
budget with aggregate queries over the journal would pay for the measurement.

## Decision

Adopt a **derived per-world budget ledger with a fixed degrade ladder and typed refusals**.

Concretely:

1. **Meters at both account and world/shard scope.** Each shard keeps a small side table of
   accumulated metered units per billing period — rows read, rows written, stored bytes and
   sync bytes — plus its plan/ceiling. The account meter aggregates every world/shard on that
   backend because D1 and Turso quotas are shared by account. Each world also receives an
   explicit allocation of that account budget; it is gated by whichever is reached first,
   its allocation or the account ceiling. The meter is *derived*: advanced from per-query
   counters the backend already returns (`meta.rows_read` / `meta.rows_written` on D1;
   `rows_read` / `rows_written` on Turso's SQL-over-HTTP result) and a cheap periodic storage
   sample, never from aggregate scans over the journal.
2. **Budgets are configuration, expressed as fractions of the plan ceiling.** A deployment
   declares its account plan, per-world allocations and each shard's hard size ceiling. The
   guardrail derives warn/degrade/stop thresholds from those values. Nothing in code may
   hardcode a vendor number from the table above.
3. **A four-state ladder per account, world and shard:**
   `normal → warning → degraded → refused`, with hysteresis so a budget near a threshold
   does not flap on every query. Defaults: warn at 50%; enter degraded at 75%; at 90%, refuse
   non-journal work and reserve the remaining write budget for signed journal appends; at
   100% (or earlier if the vendor hard-stops) return a typed refusal for all writes. Clear
   each state only after usage is below its entry threshold by at least 10 percentage points.
   Also warn early when current-period burn projects a threshold crossing before the next
   reset (daily for D1 Free, monthly for paid D1 and Turso).
4. **A fixed, documented degrade order.** When a shard enters `degraded`, the guardrail sheds
   work in this order and no other: (a) advisory/derived surfaces — summaries, analytics,
   dashboard aggregates; (b) non-journal bulk reads; (c) non-journal writes that can be
   deferred; (d) **never** journal appends. The signed journal is the last thing to be
   refused, because refusing an append can fork a world, while refusing an analytics read
   costs a dashboard refresh.
5. **Refusal is typed, not incidental.** Every refusal is a structured error naming the
   world, the shard, the backend, the dimension that ran out, the threshold crossed, and the
   operator action that clears it. A vendor `BLOCKED` or D1 limit error is *mapped into* the
   same type rather than escaping as a raw storage error.
6. **Operator-visible state is a first-class surface.** The world's status carries its
   guardrail state, the shard responsible, and time-to-reset; approaching a ceiling is
   visible before it is reached, not only when a request fails. Warnings are pull-based
   (status route) and push-based (the existing alert path), but the status route is the
   authority.
7. **Per-world totals are summed across shards.** A world's reported budget usage and state
   is the worst state among its shards, with per-shard detail retained. Placement changes
   move the meter with the shard rather than resetting the budget.
8. **Reserve before expensive work.** Every background replay, index build, migration and
   backfill must declare an upper-bound or conservative estimate and atomically reserve its
   budget before starting. Actual response counters settle the reservation afterward. A
   query whose scan cost cannot be bounded is split into bounded chunks or denied while the
   shard is degraded. Vendor counters arrive after execution, so this is a best-effort
   application budget, not a provider-enforced hard spending cap; the plan needs headroom for
   an in-flight query that exceeds its estimate.

## Alternatives rejected

**A. Rely on the vendor.** Do nothing and let D1/Turso push back. Rejected: on the D1 free
tier the first signal is a hard outage until midnight UTC; on Workers Paid there is no
signal at all, only a bill; on Turso the first signal is a `BLOCKED` error at whichever call
site happens to run next, which for a journal append is the worst possible place. Vendor
email alerts are a late, out-of-band, per-account channel — useful as a backstop, never as
the control plane.

**B. A metering proxy in front of every storage call.** Accurate and simple to reason about,
but it puts another hop on the journal write path, becomes a single point of failure for a
world, and would need to understand shard placement anyway. Rejected as the primary
mechanism; the derived meter gets the same accuracy from counters the backend already
returns at no extra round trip.

**C. Static capacity planning only.** Model each world's expected usage, size shards to fit,
and enforce nothing at runtime. Cheap, and correct for steady state — but it cannot survive
backfills, replays, migrations, or a repair sweep, which are precisely the events that
produce the surprising invoices (turso's own docs warn that `ALTER TABLE` and index creation
bill a read per existing row). Rejected as the only mechanism; retained as the sizing input.

**D. One global account budget.** Track only the account total. Rejected: it cannot attribute
cost to a world, so an operator cannot answer "which world is burning my quota", cannot price
per world, and cannot degrade one world without degrading all of them.

The chosen design is A's cheapness in steady state, B's accuracy where it is free, C's
sizing discipline, and D's simplicity at the accounting layer — with per-world attribution.

## Cost model at the three scales

Assumptions are stated per row so the numbers can be re-derived if plans change. "Rows" means
scanned rows for reads and affected rows for writes, per both vendors' definitions.

| Scale | Shape assumed | Read volume / month | Write volume / month | Storage | Cheapest honest plan |
| --- | --- | --- | --- | --- | --- |
| Single user | one person, a few worlds, occasional replay; endpoint replica does most reads | ~30 M | ~200 k | < 100 MB; < 1 GB sync | Turso Free (500 M / 10 M / 5 GB / 3 GB sync) or D1 Workers Free if reads stay under ~5 M/day |
| Small community | tens of active members, daily traffic, periodic backfill | ~300 M | ~3 M | 1–3 GB; ~6 GB sync | Turso Developer ($4.99 annual equivalent) or D1 Workers Paid ($5 minimum) |
| Large public world | ~10,000 members, continuous ingestion, sharded | ~5 B | ~60 M | ~30 GB; ~40 GB sync | Turso Scaler ($24.92 annual equivalent) with approved overages, or D1 Workers Paid with overage |

Illustrative cost arithmetic for the large-world row: Turso Scaler is within its read/write
allowances; 30 GB storage is 6 GB over the included 24 GB and 40 GB sync is 16 GB over 24 GB,
so at the listed rates its annual-equivalent base is `$24.92 + (6 × $0.50) + (16 × $0.25) =
$31.92/month`. This assumes overages are explicitly enabled. On D1 Workers Paid, 5 B reads
are within the 25 B inclusion; 60 M writes leave 10 M billable (`$10`); 30 GB account
storage leaves 25 GB billable (`$18.75`), plus the `$5` Workers minimum: `$33.75/month`
before Worker request/CPU overages. D1's 10 GB hard cap means these 30 GB must be split over
at least three databases; this arithmetic excludes the operational cost of that split.

These examples are workload envelopes, not a claim that all worlds of a given population
consume these rows. Before setting a real allocation, measure row scans/writes, sync volume
and stored bytes on a representative world. The rule is the measured usage vector, not the
member count.

Reading the table: at single-user and small-community scale the binding constraint is usually
**writes on the free tier**, not reads — 10 M/month on Turso Free is roughly 330 k/day, which a
lazy client with a chatty sync loop can exhaust. At large scale the binding constraint moves
to **storage and per-shard throughput** (D1's 10 GB hard database cap; Turso's storage
overage rate), not rows. That shift is why the ladder is per-dimension rather than a single
"is this world too big" bit.

Cost at risk from a single unguarded event, worth stating because it motivates the design: an
index build or `ALTER TABLE` over a 10 M-row table bills ~10 M reads on either platform. On
Turso Free that is 2% of the monthly read allowance; on Turso Developer, 0.4% — and roughly
$0.01 of overage. A repair sweep that does that across 200 shards is 2 B reads: 80% of the
Developer allowance, but 2% of Scaler's. Guardrails exist to make that multiplication visible
*before* it completes.

## Invariants implementation must hold

1. **No silent loss.** A refused journal append is reported as a refusal; it is never
   swallowed, coerced to success, or buffered into an unbounded retry queue.
2. **The meter is derived and rebuildable.** Deleting the meter table must lose accuracy, not
   history: it can be re-seeded from backend counters. The journal is never the meter and the
   meter is never the journal.
3. **Metering never scans the journal.** Counters come from per-query metadata and a bounded
   storage sample; `count(*)` over event rows is forbidden on the budget path.
4. **Degrade order is fixed and tested.** Advisory reads shed before journal writes, always;
   the order is a property test, not a comment.
5. **Hysteresis.** State transitions require crossing the threshold *and* staying across it;
   a test drives a shard across a threshold repeatedly and asserts no flapping.
6. **Placement-agnostic accounting.** A world spread across N shards on M backends reports
   one budget with per-shard detail; moving a shard moves its accumulated usage rather than
   resetting it.
7. **Typed mapping of vendor failures.** `BLOCKED`, D1 daily-limit errors, and D1 overloaded
   errors map to distinct guardrail states (quota exhausted vs throughput saturated) — the
   operator action differs.
8. **Backstop, not control plane.** Vendor alerts and account-level dashboards remain
   configured, but no behaviour depends on them.
9. **Reservations are atomic across workers.** Two concurrent replays cannot both reserve
   the same remaining account/world budget; an uncertain reservation fails closed for
   background work.

## Evidence that changes or constrains a parent decision (flagged, not forked)

1. **D1's uncapped paid overage is a live cost risk in the parent's "both" answer.** The
   parent treats D1's Workers-gating as "not a real cost". That is right about *architecture*
   and wrong about *billing*: placement-by-heat puts bulk cold-ish scan traffic on D1, which
   is exactly the traffic shape that multiplies rows read. If the atlas does place large
   shards on D1, the parent should record billing ceilings as part of the placement policy,
   not only latency and size. **This does not overturn "both"; it narrows how D1 is used.**
2. **Turso's `BLOCKED` semantics conflict with a mid-append refusal.** The parent should
   confirm that a world whose Turso shard hits a quota is allowed to *degrade* rather than
   fail, because the alternative — a hard fail at the vendor — can interrupt a journal append
   with no guardrail able to intervene in time. Interim answer recorded here: decision 3's
   reserve keeps the journal writable through 90–100% of the write budget, and any residual
   vendor refusal maps to the typed refusal instead of a raw error. If the parent rules that a
   blocked shard must hard-fail the world, the reserve rule changes.
3. **Turso plan continuity is unresolved.** Supabase's announced acquisition of Turso
   (announced 2026-10-02) leaves current plan terms uncommitted. Any deployment whose budget
   is expressed in Turso plan units must treat them as configuration with a review date, and
   the parent should note that a plan change is an operator-visible event, not a code change.
4. **D1's per-database 10 GB hard cap** interacts with the parent's per-shard placement: a
   shard cannot simply grow. If a hot shard approaches 10 GB the answer is a split, which
   means the guardrail must be able to warn on *bytes-to-cap*, not only on rows — recorded
   here as a requirement on the split path (#5867 owns the split mechanics).
5. **Read replication adds no D1 charge but also no write capacity.** Cloudflare documents
   that read replicas are not billed separately; it does not document any increase in write
   throughput, and each database (primary or replica) remains single-threaded. A placement
   policy that puts a hot *writer* on D1 expecting replicates to absorb it will be wrong.
   This constrains #5867's placement rule, not the "both" answer.

## What this record does not decide

- The storage contract itself (#5863), the per-backend journal-fit spikes (#5864, #5865), local
  dev parity (#5866), placement/migration policy (#5867), and backup/exit (#5869).
- Prices and plan limits beyond the date on this record; those are inputs, and the vendor pages
  are the source.
- Any implementation ticket. Splitting this into implementation work is a separate filing, and
  per #5868's own instruction any split is filed and linked rather than grown here.

## Sources

- Cloudflare D1 pricing — <https://developers.cloudflare.com/d1/platform/pricing/> (read 2026-11-08)
- Cloudflare D1 limits — <https://developers.cloudflare.com/d1/platform/limits/> (read 2026-11-08)
- Cloudflare Workers pricing — <https://developers.cloudflare.com/workers/platform/pricing/> (read 2026-11-08)
- Cloudflare changelog, D1 free-tier limit enforcement 2026-09-01 —
  <https://developers.cloudflare.com/changelog/post/2026-09-01-d1-free-tier-limit-enforcement>
- Turso pricing — <https://turso.tech/pricing> (read 2026-11-08)
- Turso usage & billing — <https://docs.turso.tech/help/usage-and-billing> (read 2026-11-08)
- Turso SQL over HTTP result counters — <https://docs.turso.tech/sdk/http/reference> (read 2026-11-08)
- Turso database usage API — <https://docs.turso.tech/api-reference/databases/usage> (read 2026-11-08)
- Turso Cloud limitations — <https://docs.turso.tech/cloud/limitations> (read 2026-11-08)
- Parent design and Drew's architecture direction — <https://github.com/developing-today/code/issues/5681>
