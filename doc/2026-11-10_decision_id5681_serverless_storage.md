# Decision: serverless edition storage — parent synthesis for #5681

- Date: 2026-11-10
- Status: **parent decision record** for
  [#5681](https://github.com/developing-today/code/issues/5681)
  (D1 vs Turso vs both for `id` on Cloudflare Workers). This record
  synthesises the child records; it does not redo them, and it does
  not close #5681 — child-owned work listed in §9 is still in flight
  and is not claimed here as parent work.
- Serverless edition: #2046. Programme epic: #2056. Signed journal
  replication: [#806](https://github.com/developing-today/code/issues/806).
- Direction already recorded by Drew on #5681 (2026-11-04,
  comment 6646491086): **BOTH, with roles — Turso preferred,
  embedded replicas on endpoints, very large atlas shards spread
  across Turso and D1 per-shard, cold/archive tiers on the home
  lab.** This record preserves that direction, prices it, defines
  the placement policy it leaves open, and states the conditions
  that would reverse it (§3).
- Evidence grading used throughout: **[landed]** = in a merged
  record on main; **[in-flight]** = in an open PR/issue, cited but
  not landed; **[documented]** = vendor documentation claim, not
  measured by us; **[spike-owed]** = not knowable from documents —
  an open child spike owns it. No number in this record is
  presented as measured unless a landed record measured it.

## 1. Children consumed, not re-decided

| Child | Owns | State at synthesis (verified fresh 2026-11-10) |
|---|---|---|
| #5863 | serverless storage contract | OPEN — no landed contract record yet |
| #5864 | D1 journal fit (design + spike) | OPEN — Drew-owned draft PR #6350 is the canonical spike; not touched |
| #5865 | Turso/libSQL journal fit (design + spike) | OPEN — design+spike still owed (2026-11-08 triage: real/current) |
| #5866 | local-dev parity | OPEN for its backend-specific §6 remainder; backend-agnostic half **[landed]** via PR #7588 (`doc/2026-11-08_design_id5866.md`) |
| #5867 | backend choice & migration | OPEN — this record feeds it the parent placement policy (§5); the choice mechanics remain its own |
| #5868 | cost & limits guardrails | OPEN — decision record **[in-flight]** in Drew-owned PR #7591 (`doc/2026-11-08_decision_id5868_cost_limits_guardrails.md`); not touched; its unresolved risks are retained in §7 |
| #5869 | backup, recovery & exit | CLOSED — **[landed]** via PR #7774 (`doc/2026-11-08_design_id5869.md`) |

The decomposition sketch in #5681 maps 1:1 onto #5863–#5869
(contract, D1 fit, Turso fit, local-dev parity, choice &
migration, cost & limits, backup/exit). No additional follow-on
tickets are filed by this record: the sketch is fully filed, and
the gaps are gaps *inside* those children (§9), not missing
tickets.

## 2. The one idea everything follows from

From #5869 **[landed]**: the signed journal (#806) is the system
of record; every backend store is a materialisation of it. That
single fact decides most of this comparison:

- Store replication is never the journal's replication. Whatever
  D1 or Turso replication does, the signed stream must be able to
  rebuild any store from the journal alone (§6).
- "Both" is therefore affordable in a way it would not be for a
  store-as-truth design: a second backend is a second
  materialisation target behind one contract (#5863), not a
  second source of truth.
- Exit is always possible in principle: export = journal +
  snapshot in an open format (#5869 §4 bundle); a backend that
  cannot be rebuilt from the journal fails the contract, whichever
  vendor it is.

## 3. Recommendation

**Adopt C — both, with roles — at per-shard granularity for the
atlas/corpus stores and per-deployment granularity for a world's
primary store**, exactly as directed on #5681:

1. **Turso is the default and preferred backend** for world
   primaries and hot shards: embedded replicas on endpoints are a
   first-class requirement of the local-first story, and only
   Turso offers them.
2. **D1 is a first-class placement target, not a fallback**: very
   large atlas shards are placed per-shard across Turso *and* D1
   by size, heat, and access pattern (§5). D1's Workers-native
   binding (no extra vendor, no extra credentials) is a real
   operational advantage for shards whose access is Workers-only.
3. **Cold/archive tiers are neither**: cooled shards sink to the
   home lab (rustfs/S3-compatible storage, lab cluster). Cloud is
   for hot and warm only.
4. **One common storage contract** (§6) binds both backends; no
   backend-specific feature may leak above it (the #5866 parity
   rule, **[landed]**).

**Reversal conditions** — the recommendation flips to a single
backend if any of these is *evidenced* (a landed spike or a
production incident, not a preference):

- R1. A landed #5864/#5865 spike shows one backend cannot expose
  a #5863 contract hook (§6) — e.g. monotonic sequence +
  snapshot/delta + replay-in — without weakening a #806
  guarantee in §6's table. That backend is dropped, not
  compensated silently (#5869's rule).
- R2. Contract-test/CI cost of the second backend measurably
  exceeds the placement benefit for two consecutive releases
  (the "who keeps the second backend honest" question in #5681 —
  owner: the contract-test suite, §6.4; if it cannot be kept
  green, "both" is speculative generality and collapses to the
  surviving backend).
- R3. A vendor change removes a load-bearing documented property
  (e.g. Turso embedded replicas, D1's 10 GB ceiling rising to
  make sharding unnecessary at our scale, or a pricing change
  that inverts §7's large-world arithmetic by >2×).
- R4. Drew records a new direction on #5681 superseding the
  2026-11-04 direction.

## 4. Eight-axis comparison

### 4.1 Consistency & ordering

- **D1 only.** Single-primary SQLite; writes serialise through
  the primary. Read replication with session bookmarks gives
  read-your-writes inside a session **[documented]**; cross-region
  replica lag is real and unbounded in the worst case
  **[documented]**. For the journal this is sufficient *iff*
  appends always go to the primary and readers who need a
  sequence guarantee use the bookmark/session path — the spike
  that must prove the session semantics under Workers is #5864
  **[spike-owed, in-flight PR #6350]**.
- **Turso only.** Single primary per database with embedded
  replicas syncing from it; embedded replicas are read-only
  locally and sync writes through the primary **[documented]**.
  Same single-writer shape as D1 at the journal head; replica
  sync latency and offline-then-reconnect behaviour are
  **[spike-owed — #5865]**.
- **Both.** The contract may only assume the intersection:
  single-writer append at a named primary, a monotonic journal
  sequence assigned *by the journal layer* (never by store
  auto-increment semantics, which differ), and read-your-writes
  only through an explicit sequence watermark ("read at ≥ N")
  that each backend implements via its own session mechanism.
  Anything stronger is backend-specific and forbidden above the
  contract.

### 4.2 Replication / journal fit (#806)

Full guarantee table in §6. Summary: neither store's replication
is the journal's replication. D1 read replicas and Turso embedded
replicas are *read accelerators*; Turso primary replication and
D1's platform replication are *availability mechanisms for the
materialisation*. The hazard #5681 names — two replication
systems moving the same data — is resolved by rule: **per shard,
exactly one system replicates for availability** (the store's),
and the signed stream replicates *between stores and endpoints
for verification, export, and rebuild* — never as a second
availability path racing the store's own replication on the same
shard.

### 4.3 Cost shape

Model and guardrails are #5868's, **[in-flight PR #7591]**,
vendor-documentation-sourced 2026-11-08; §7 reproduces its
three-scale envelopes as the parent cost model with assumptions.
Headline: at single-user and small-community scale the binding
constraint is free-tier *writes*, not reads; at large-world scale
both vendors land within ~$2/month of each other in the modelled
envelope, so cost does not decide the placement — heat and access
pattern do (§5).

### 4.4 Limits & ceilings

From #5868 **[in-flight]**, **[documented]**: D1 — 10 GB max
database size (cannot be raised), 500 MB on free; per-database
single-threaded throughput (~10 qps at 100 ms average query);
free tier fails closed at daily read/write limits, paid tier has
**no hard spending cap** (uncapped overage is a recorded risk,
§9). Turso — plan storage/row envelopes (§7); on limits, appends
fail with a typed `BLOCKED` rather than truncating; overages are
opt-in per plan. Parent consequence: **the 10 GB D1 ceiling is a
placement input, not a blocker** — a shard approaching it is
exactly what §5's cooling/rebalancing moves; no single database
on either backend is ever the corpus.

### 4.5 Local-dev story

Decided by #5866 **[landed]**: parity is contract parity, not
backend identity; local workflows run against the #5863 contract
only; fixtures are contract-level journal replays; one command up,
one command reset. Backend-specific local targets (Wrangler/
Miniflare D1 vs local sqld) are #5866's §6 remainder, explicitly
blocked on the backend choice — this record unblocks it in one
respect: since the answer is *both*, §6 of #5866 must cover both
local targets behind the same contract, and any divergence goes
on its parity-gap list, not into local-only code paths.

### 4.6 Durability, backup & exit

Decided by #5869 **[landed]**: PITR is journal replay at
entry granularity; retention floors (journal online ≥ 30 days,
daily snapshots 90 days, weekly 1 year, export bundles ≥ 1 year
independent of provider); provider-native PITR (D1 Time Travel,
Turso restore) is an accelerator, never the mechanism; restores
are always to a fresh store. Identical for both backends — this
axis does not differentiate, by design.

### 4.7 Security & tenancy

- **D1.** Access is a Workers binding: no credential exists to
  leak; a compromised Worker reaches exactly the databases bound
  to it. Per-world isolation = per-database binding discipline.
  **[documented]**
- **Turso.** Access is a bearer token per database/group; a
  compromised Worker reaches whatever its token scopes. Token
  scoping per shard/world is therefore a contract-adjacent
  requirement: one token per placement scope, never one
  account-wide token in a Worker. Self-hosted sqld removes the
  vendor from the trust path at the cost of operating it (§4.8).
  **[documented]**
- **Both.** The journal's signatures are the tenancy backstop on
  either store: a store (or a neighbouring tenant's bug) that
  returns forged or reordered entries is detected by signature /
  hash-chain verification at read time, not trusted. Row-level
  isolation inside a database is *not* assumed by the contract;
  isolation granularity is the database/shard.

### 4.8 Operational burden

- **D1 only.** Least to provision: no extra vendor, account, or
  credential. Failure modes are Cloudflare's. The operator's
  burden is the §4.4 ceilings and the uncapped-paid-overage risk.
- **Turso only.** One external vendor + account even in the
  "serverless" edition; token lifecycle to own; self-hosted sqld
  is an escape hatch that quietly creates a third, accidental
  edition if adopted per-deployment without a policy — this
  record's policy: sqld is the *home-lab cold/warm tier's*
  runtime (§5), not a per-deployment improvisation.
- **Both.** Two vendors' status pages, two limit ladders (#5868's
  four-state ladder is per backend, per plan, per shard), and the
  contract-test matrix (§6.4). This is the honest price of §3,
  and reversal condition R2 exists precisely if it stops being
  worth paying.

## 5. Shard placement policy (the decision #5681 leaves open)

Placement is **per-shard** for atlas/corpus stores and
**per-deployment** for a world's primary store (a world primary
starts on Turso; it is one database, and per-world backend choice
finer than that is speculative until a world outgrows one
database — at which point it shards and §5 applies).

### 5.1 Placement inputs, in decision order

1. **Tier by temperature.** Hot (read or written within 7 days) /
   warm (within 90 days) / cold (older). Cold shards sink to the
   home-lab tier regardless of backend. Temperature is computed
   from the shard's journal tail and read meter (#5868's per-shard
   counters), never from operator intuition.
2. **Size.** A shard whose materialisation would exceed 8 GB on
   D1 (80% of the 10 GB hard ceiling) is not placed on D1; a
   shard projected to cross it within one rebalance interval is
   moved before it must be (§5.3).
3. **Access pattern.** Endpoint-local-first reads (laptop /
   endpoint embedded replica in the loop) → Turso. Workers-only,
   read-heavy, geographically diffuse access with no endpoint
   replica → D1 is eligible and preferred when it is also the
   cheaper meter for that shard's read shape (#5868: row scans
   are the meter on both, so index discipline decides, not the
   logo).
4. **Write shape.** Any shard with a signed-journal append path
   defaults to Turso unless its writes are Workers-originated
   only and D1's session semantics for it are spike-proven
   (#5864).

### 5.2 Where the placement fact lives

The placement record (shard id → backend, tier, primary endpoint,
contract version) is itself **a signed journal record in the
world's manifest**, materialised into a small placement registry
store on Turso (the default backend) and mirrored into every
export bundle (#5869 §4). Readers resolve placement from their
local materialisation; the registry is a cache of the manifest,
never the truth. A placement *change* is a journal append, so it
is ordered, signed, and replayable like everything else.

### 5.3 Cooling and rebalancing

- Cooling is a placement change, executed as: snapshot at
  sequence N → export bundle → replay into the target tier/
  backend → verify (§6 hooks: snapshot+delta, replay-in,
  signature check) → append the placement change → drain the old
  materialisation after one retention interval. The shard is
  readable throughout from the old materialisation until the
  placement append lands; writes pause only for the final delta
  replay (bounded by the delta since N, kept small by choosing N
  at a low-write moment or by iterating snapshot+delta twice).
- Rebalancing between Turso and D1 is the same operation with a
  different target. There is no live cross-backend replication;
  movement is always export/replay. This is deliberate: it is the
  mechanism #5869 already mandates for exit, so placement changes
  rehearse the exit path continuously.
- Nothing moves silently: every placement change names its §5.1
  trigger (tier, size, or access) in the manifest record.

### 5.4 When a shard's store is unavailable

- **Reads**: serve from any verified materialisation — embedded
  replica, endpoint copy, or a peer's signed-stream copy — at a
  stated sequence watermark; staleness is reported, never hidden.
- **Writes**: appends to an unavailable primary are *refused with
  a typed error and queued client-side as unsigned-intent, never
  redirected to the other backend*. Redirecting writes would fork
  the journal head; a refused append is recoverable, a fork is a
  §6 incident. (This mirrors #5868's guardrail ordering: refuse
  analytics and bulk work first, journal appends last — but an
  append to a *down* primary is not served elsewhere.)
- **Recovery**: when the primary returns, replay-in reconciles;
  if it does not return, §5.3's movement runs from the export
  bundle to a fresh store and the placement append names the
  incident.

## 6. Journal guarantees (#806) per store, and the common contract

### 6.1 Guarantee table

| #806 guarantee | D1 | Turso/libSQL |
|---|---|---|
| Append ordering at a single head | **Preserved** via single-primary serialisation; sequence assigned by the journal layer (§4.1) | **Preserved** via single-primary serialisation; same journal-layer sequence rule |
| Signature / hash-chain integrity | **Preserved** — store-independent; verification is at read/replay time on both | **Preserved** — same |
| Read-your-writes for the writer | **Preserved with compensation**: requires the session/bookmark path; default replica reads may lag **[spike-owed #5864]** | **Preserved with compensation**: writer reads from primary or at a sequence watermark; embedded-replica reads may lag **[spike-owed #5865]** |
| Replica reads (followers) | **Weakened by design**: D1 replicas are eventually consistent; followers must verify against the signed stream and report their watermark | **Weakened by design**: embedded replicas likewise; same watermark rule. This is acceptable because #806 followers verify signatures rather than trusting the store |
| Fork detection | **Preserved** — two different signed entries at one sequence are detectable from the stream alone, on either store | **Preserved** — same |
| Durability of an acknowledged append | **Requires compensation**: platform replication is the only durability until the entry is exported/replicated over the signed stream; §5869's export cadence bounds the loss window, on both stores equally | **Requires compensation** — same rule, same bound |
| Offline endpoint appends | **Not offered** — no embedded replica story; endpoints are online-only against D1 | **Weakened/requires compensation**: embedded replicas are read replicas; offline *writes* are client-side intents (§5.4), merged by append order at reconnect — never local commits to the replica presented as journal entries **[spike-owed #5865]** |

### 6.2 The common storage contract (parent requirements on #5863)

#5863 owns the contract design; this record fixes the parent
requirements it must satisfy (the first four are #5869's hooks,
**[landed]**):

1. Monotonic journal sequence visible to readers, timestamp per entry.
2. Snapshot + delta read at a named sequence.
3. Replay-in of exported entries into an empty store, preserving sequence, timestamps, signatures.
4. Schema/contract version stamped on every snapshot and export.
5. Read-at-watermark (`read at ≥ N`) as the only freshness primitive above the contract (§4.1).
6. Typed refusal semantics for limit/unavailable cases (no silent truncation, no silent redirect), aligned with #5868's ladder and §5.4.
7. Per-shard metering counters (rows read/written, stored bytes) readable without a full scan — both backends expose these **[in-flight #5868]**.

### 6.3 Choice granularity

Per-deployment for world primaries; per-shard for atlas/corpus
stores (§5). Per-world choice finer than per-deployment for
unsharded worlds is rejected for now: it would put a routing fact
in front of every world open for a benefit no landed evidence
demonstrates — the #5681 open question is answered "per-deployment
is enough until a world shards", with §5 as the escalation path.

### 6.4 Keeping the second backend honest

Contract tests, run in CI against both backends on the same
fixture journals (#5866's replay fixtures are the corpus), are
the owner of "both". A backend that fails the contract suite is
not shipped that release; two consecutive releases of that is
reversal condition R2 (§3).

### 6.5 Migration / export-replay path

One path serves migration, placement change, recovery, and exit:
#5869's export bundle (journal + snapshot, open format, version
stamped) replayed into the target store behind the contract and
verified before it serves (§5.3). There is no second, live
migration mechanism, and #5867's migration design consumes this
path rather than inventing a parallel one.

## 7. Cost model (three scales)

Source: #5868's decision record, **[in-flight PR #7591]**,
vendor documentation read 2026-11-08 — **[documented]**, not
measured. Assumptions, stated per the parent acceptance: annual-
billing equivalents for Turso plans; Workers Paid $5 minimum
where D1 is used; overages explicitly enabled where a row says
so; row counts are the vendors' scan-based meters (an unindexed
query bills every row it walks, on both).

| Scale | Workload envelope (reads/mo, writes/mo, storage) | Turso | D1 |
|---|---|---|---|
| Single user | ~30 M reads, ~200 k writes, < 100 MB (< 1 GB sync) | Free tier fits (500 M / 10 M / 5 GB) | Workers Free fits if reads stay < ~5 M/day |
| Small community | ~300 M reads, ~3 M writes, 1–3 GB (~6 GB sync) | Developer, $4.99/mo annual equivalent | Workers Paid, $5 minimum |
| Large public world | ~5 B reads, ~60 M writes, ~30 GB (~40 GB sync) | Scaler $24.92/mo + overage arithmetic ≈ **$31.92/mo** total in #5868's worked example | Workers Paid arithmetic ≈ **$33.75/mo** (25 GB billable storage $18.75 + $5 minimum + row overage) |

Reading the model: (a) free-tier *writes* bind first at small
scale (~330 k/day on Turso Free is exhaustible by a chatty sync
loop — a guardrail problem, #5868, not a vendor problem);
(b) at large scale the two totals are within ~6% of each other,
so cost is a *guardrail* input (ceilings, ladders, warnings), not
the placement decider — §5.1's heat/access rules are; (c) the
home-lab cold tier removes the long tail of cold storage from
both columns entirely, which is worth more at archive scale than
any per-GB difference in this table.

## 8. Open questions from #5681, answered or assigned

- *Does #806 need store replication at all?* Store replication is
  an availability/read accelerator for the materialisation only;
  the signed stream is the replication of record (§4.2, §6.1).
- *Per-world or per-deployment?* Per-deployment, escalating to
  per-shard when a world/corpus shards (§6.3, §5).
- *Largest intended world?* Bounded by placement, not by a
  single store: the 10 GB D1 ceiling and Turso plan envelopes are
  per-shard inputs (§5.1); no landed record sets a corpus ceiling,
  and this record sets none — the atlas cold-tier format work
  owns archive-scale numbers.
- *Documentation vs spike?* Every **[documented]** entry in §4/§7
  is a claim, and the load-bearing ones (session semantics,
  embedded-replica sync/offline behaviour) are **[spike-owed]** to
  #5864/#5865. This record's recommendation does not depend on
  any single one of them being favourable — the contract
  intersection (§4.1) is what is being adopted.
- *Who keeps the second backend honest?* The CI contract suite
  (§6.4), with R2 as the enforcement.
- *Does self-hosted sqld blur the editions?* Only if improvised;
  §4.8 assigns sqld to the home-lab tier as policy.

## 9. What remains open (not claimed by this record)

- #5863: the contract itself is undesigned-as-landed; §6.2 is its
  requirements input, not its design.
- #5864 / PR #6350 (Drew-owned, draft): D1 session/ordering spike
  results are not in; §4.1/§6.1 entries marked **[spike-owed]**
  stay that way until it lands.
- #5865: Turso journal-fit design + spike not landed.
- #5866 §6: backend-specific local targets — now unblocked in
  scope (§4.5), still to be written there.
- #5867: choice & migration mechanics consume §5/§6.5; its own
  record is still owed.
- #5868 / PR #7591 (Drew-owned, open): cost record not yet
  landed; its recorded risks — D1 paid tier's uncapped overage,
  the 10 GB hard cap, Turso `BLOCKED`-at-append behaviour, plan
  continuity — are retained here unchanged and are guardrail
  requirements, not footnotes.
- #5681 itself stays **open**: its acceptance requires every
  criterion evidenced with no child-owned work claimed as parent
  work; this record is the parent deliverable, and the issue
  closes when the children above land or are explicitly
  dispositioned against it.

---

*Parent synthesis for #5681, produced by task 13920 (lane
code-v). Children: #5863–#5869. Direction: Drew, on #5681,
2026-11-04. Relates to #2046, #2056, #806.*
