# TigerBeetle cluster topology decision — 3 vs 6 replicas, site placement, commit latency

**Date:** 2026-10-29 · **Issue:** #2064 (ID program wave-1, parent epic #2056)
**Source investigation:** #2040 — `doc/2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend/`
**Question:** Drew's sketch was one TigerBeetle cluster on 3 machines, "maybe one
per cloud". Cluster size **cannot be changed after the data files are formatted**,
so this is the one effectively irreversible decision in the currency track, and it
gates any deployment (designs #2062, #2063, #2065 all sit behind a running cluster).

## TL;DR — the recommendation

**Format the production cluster at 6 replicas: 2 per site across 3 sites, one site
per cloud provider, all three sites in the same metro-scale geography (inter-site
RTT within a few milliseconds).**

That is the vendor's production recommendation verbatim, and the analysis below
shows 3 replicas has **less** failure tolerance than 6 at the same placement,
and that commit latency is decided by *placement*, not count: a cluster spread
across distant clouds — at either count — waits on an inter-cloud round trip
for every commit, while flexible quorums let a near-site primary commit on
its fastest responders (including its same-site mate). The recommended
option wins the fault-tolerance axis *and* uses the placement the protocol
was designed around. The cost of
the extra safety is three more tiny VMs (TigerBeetle needs ~1 core per replica).
A separate throwaway 3-replica (or single-replica) dev cluster for the PoC is fine —
it holds no real value and is deleted, not migrated. What must not happen is
formatting the *production* cluster at 3 to "start small": there is no resize path;
changing your mind later means a new cluster and a control-plane state migration of
an immutable ledger, which is exactly the kind of migration a money system should
never have to do casually.

## 1. The mechanics that decide everything

From the #2040 investigation (TigerBeetle docs, single-page edition; Jepsen
0.16.11 analysis) and the vendor's Cluster Recommendations page:

- All replicas run continuously and participate in Viewstamped Replication (VSR).
  There is no passive backup; the primary is a role, promoted by view change.
- **Flexible quorums** (Heidi Howard): in a 6-replica cluster, a commit needs
  **3 of 6** replicas, but electing a new primary needs **4 of 6**. The commit
  quorum and the election quorum are deliberately different sizes.
- In a 3-replica cluster, both quorums are **2 of 3**.
- Vendor: *"For the highest availability, TigerBeetle should be deployed as a
  cluster of six replicas across three different cloud providers (two replicas
  per provider)"*, tolerating *"a complete outage of any cloud provider"* and
  likely surviving *"even if one extra replica fails."* And: *"Sites should
  preferably be within a few milliseconds of each other, since each transaction
  must be replicated across sites before being committed."*
- Cluster size is fixed at format time: *"In the current version of TigerBeetle,
  cluster size can not be changed after creation."*
- Per-replica hardware is small: Linux ≥ 5.6, one dedicated machine and disk,
  local NVMe, ECC memory, ≥ 6 GiB RAM (16–32 GiB recommended), ~1 core (single-
  threaded by design) plus a spare for the OS, a supervisor to restart crashes.

## 2. The options

| # | Topology | Shape |
|---|----------|-------|
| A | **3 replicas, 1 per cloud** (Drew's sketch) | 3 providers, 1 replica each, inevitably spread across each provider's regions |
| B | **6 replicas, 2 per site × 3 providers, near sites** | 3 providers in one metro-scale geography, inter-site RTT a few ms |
| C | 6 replicas, 2 per AZ × 3 AZs, single provider | Lowest latency, but the provider itself is one fault domain |
| D | 3 replicas, single site / home machines | Cheapest; site, power, and ISP are all single points of failure |

Option D is rejected without further analysis for any cluster holding real value:
the #2040 hardware requirements (ECC, NVMe, Direct I/O, supervised always-on
processes) and a residential site's correlated power/ISP failures make it a dev
shape, not a production one. Options A–C are compared below.

## 3. Failure cases, enumerated

"Tolerates" below means: keeps committing transfers without data loss. In every
unavailable case TigerBeetle's designed behaviour is to **halt safely** rather than
risk inconsistency — balances are never wrong, but currency operations stop until
enough replicas return.

### Option A — 3 replicas, 1 per site (quorums: commit 2/3, election 2/3)

| Failure | Outcome |
|---|---|
| 1 replica crashes | **Available.** 2/3 remain; commits and election both work. Zero fault tolerance left. |
| 1 whole site/provider lost | Same as above — that *is* losing 1 replica. Available, zero margin. |
| Any 2nd failure while 1 is down (second provider hiccup, botched upgrade, disk failure) | **Unavailable.** 1/3 is below commit quorum. Safe halt until a replica recovers. |
| Primary fails | View change among the remaining 2; brief pause, then available — *if* both survivors are healthy. |
| Network partition 1 \| 2 | The 2-side continues; the isolated replica halts. Correct, but the cluster is one more failure from stopping entirely. |
| Disk corruption on 1 replica | Repairable from quorum while available; combined with any other failure, see row 3. |

Net: **tolerates exactly 1 failure of anything, ever, at a time.** With replicas on
three independent cloud providers, "one thing is down" is not a rare state — it
includes every single-provider maintenance event, outage, and botched deploy, and
during every one of those windows the money system is one hiccup from halting.

### Option B — 6 replicas, 2 per site × 3 near sites (quorums: commit 3/6, election 4/6)

| Failure | Outcome |
|---|---|
| 1 replica crashes | **Available**, full election and commit quorums with margin. |
| 2 replicas in *different* sites fail | **Available.** 4/6 remain; election quorum met exactly. |
| 1 whole site/provider lost (2 replicas) | **Available** — the vendor's stated guarantee. 4/6 remain, both quorums met. This is the headline difference from A: in A a provider outage consumes 100% of the fault budget; in B it consumes half. |
| Site loss **plus** 1 extra replica elsewhere | **Probably available** (vendor: "will likely survive"). 3/6 remain — commit quorum (3) is met if the primary is among the survivors; a *new* election cannot reach 4, so survival depends on the sitting primary staying up. Degraded, not dead. |
| 2 whole sites lost (4 replicas) | **Unavailable.** 2/6 is below commit quorum. Safe halt. (Losing two independent providers simultaneously is far outside the plausible failure envelope.) |
| Primary fails | View change needs 4/6 — satisfiable in all single-site-loss and single-replica cases above. |
| Partition 2 \| 4 (one site cut off) | The 4-side elects and commits; the 2-side halts. Clean. |
| Partition 3 \| 3 | **Conditionally available — depends which side holds the sitting primary.** Neither side can reach the *election* quorum of 4, so no *new* primary can be chosen; but election and commit quorums are different (per the vendor's fault-tolerance docs): if the side containing the sitting primary also holds at least 3 replicas, that side keeps committing in the existing view, and the other side halts. If the primary is stranded with fewer than 3, commits halt until the partition heals. (A conventional majority system is *worse* here, not better: a majority of 6 is 4, so neither half of a 3\|3 split could commit at all.) Safety is preserved in every sub-case — no split-brain commits — but availability is conditional, not an unconditional halt. |
| Single-replica maintenance, 1 replica at a time | Available throughout with margin to spare — routine operations stop being fault-budget events. |
| **Version upgrade** (coordinated, all replicas) | Available *except a planned pause*: per the vendor's upgrade docs, TigerBeetle coordinates the upgrade across replicas and there is **~5 s of unavailability** while replicas restart on the new binary (requests are retried internally); stage binaries by rolling deployment, but schedule the upgrade itself inside a **maintenance window**. This row must not be read as "upgrades are free" — they are cheap, brief, and planned, not zero-downtime. |

Net: **tolerates any 1 site, or any 2 scattered replicas, with certainty; likely
survives site + 1 more.** Fault budget is spent by catastrophes, not by Tuesdays.

### Option C — 6 replicas, single provider, 3 AZs

Identical quorum math to B for machine/AZ failures, and the best latency of all
(§4). The difference is the correlated failure it does *not* survive: a
provider-wide control-plane, networking, or account-level event takes all 6
replicas at once. The vendor recommends three *providers* for the highest
availability tier for exactly this reason. C is a defensible budget variant, but
the saving over B is small (the VMs are tiny) and the correlated-failure exposure
is the whole ballgame for a ledger that Steam revenue depends on.

## 4. Commit latency — numbers and their basis

**Basis, stated up front (per the intake warning on this issue):** there are no
measured RTTs between Drew's actual candidate sites yet — nobody has stood the
sites up. The numbers below are (a) the vendor's published guidance and quorum
mechanics, and (b) planning ranges for public-cloud inter-region RTT that are
standard in the industry. They are labelled as estimates wherever they appear, and
§6 files the measurement work (an RTT mesh between the actual candidate sites,
run *before* formatting) as a follow-on, because the format decision must be made
on measured numbers, not on this table.

**The mechanism:** a transfer commits when the primary has replicated it to a
commit quorum and those replicas have it durable. So commit latency ≈ RTT to the
quorum member that *completes* the quorum (plus local fsync, sub-millisecond on
NVMe). The primary does **not** wait for the slowest replica — it waits for the
fastest quorum. That single fact drives the whole comparison:

- **Option A (3 across 3 clouds):** commit needs 1 backup ack. The primary must
  wait for the *nearest other cloud* — there is no same-site mate. If the three
  providers' nearest regions are e.g. US-East / US-Central / US-West class
  distances, the fastest available inter-site RTT is typically **~20–40 ms**
  (planning range, same-continent cross-provider). **Every commit pays it.**
  Spreading further (transatlantic, ~70–150 ms RTT planning range) pays that
  instead, and directly violates the vendor's "within a few milliseconds"
  guidance. **Placement caveat (added on review):** the 20–40 ms is a property
  of the *distant placement assumed for A in this comparison*, not of the
  replica count — put A on the same near sites as B and its single completing
  ack is also only a few ms away, giving A and B comparable commit latency
  (differing by at most B's one extra nearby ack). Replica count decides the
  fault budget (§3); *placement* decides commit latency. The case for B rests
  on §3 and the vendor guidance, not on a latency advantage over an
  equivalently placed A.
- **Option B (6, 2/site, near sites):** commit needs 2 backup acks. The first is
  the primary's **same-site mate** (sub-millisecond to ~1 ms within a site); the
  second is the nearest replica at another site — a few ms away by construction
  (the vendor's site guidance). Expected added commit latency: **~1–5 ms**
  (estimate from the vendor's "few milliseconds" site definition). Note:
  B's apparent order-of-magnitude edge over the A above is a *placement*
  effect, not a replica-count effect — the A in this comparison was placed
  distantly. At equivalent (near) placement the two are within a few ms of
  each other; see the placement caveat under Option A.
- **Option C (single provider, 3 AZs):** intra-region AZ RTTs are typically
  **~1–2 ms** (planning range); commit latency comparable to B, marginally
  better — bought with the provider-correlated risk in §3.

**What latency does and does not cost here.** This is a token ledger for game
services, redemptions, and issuer conversions — human-scale operations, not
high-frequency trading. Even Option A's ~20–40 ms commit latency would be
*usable*; throughput is barely affected in any option, because TigerBeetle
pipelines and batches (up to 8,189 events per request) — latency per commit is
not throughput. The latency argument is therefore not "A is too slow to use";
it is that a *distantly placed* cluster pays a permanent per-commit latency
tax on top of whatever fault budget its count buys — placement and count are
separate axes, and the distant variant of A loses on both at once (worst
fault budget *and* the tax), while near-site placement — what the protocol
was designed around — makes latency a non-issue at either count. The one
workload that would feel a distant placement's tax directly is a synchronous
conversion flow (the #2063 linked-pair engine submitting and awaiting
commit before responding to a user), where +20–40 ms lands on every
user-visible conversion.

**Placement rule that falls out:** choose the three providers' regions as a
*matched near-site set* — three different providers, one region each, picked
so that **every pairwise inter-site RTT sits within the vendor's "few
milliseconds" guidance** (same metro-scale geography). Placement is decided
by *measured* pairwise RTT, not by region names on a pricing page — which is
why §6 gates the format on an RTT mesh between the actual candidate sites.
If no three-provider near-site set exists in the geography wanted, the
fallback order is: (1) two providers near + one slightly further, accepting
the further site's RTT on the commits that need its ack; (2) Option C
(single provider, 3 AZs) with its correlated-risk caveat stated, not
silently accepted. What the rule forbids is formatting first and measuring
later: cluster size and data files are fixed at format time.

## 5. The decision

**Format production at 6 replicas — 2 per site × 3 providers, near sites
(§3 Option B).** A throwaway 3-replica (or single-replica) dev cluster for
the conversion-engine PoC (#2063 / #2289) is fine and expected — it holds no
real value and is deleted, not migrated. This decision is Drew's to confirm
in the review round on this doc; nothing formats a production cluster
before that round and the currencies synthesis (#2043) have both landed
(see #2292's own gating).

## 6. Follow-on work

Filed as **#2292** (build: TigerBeetle cluster provisioning), containing:

1. **The pre-format RTT measurement gate** — an RTT mesh between the actual
   candidate sites (all pairwise paths, sustained measurement, not a single
   ping), reported back on #2064 before any production format. Every number
   in §4 is a planning estimate until this lands.
2. **6-replica provisioning + format** per §5, once the gate and the review
   round pass.
3. **A dead-replica replacement runbook + rehearsal** — replacing a dead
   replica is a manual, documented TigerBeetle procedure; the runbook and
   one rehearsed replacement on the dev cluster are part of the follow-on,
   so the first real replacement is not the first attempt.

Nothing else is owed by this investigation: the control plane (#2062), the
conversion engine (#2063), and the remaining currency designs consume this
decision as an input and do not re-decide it.
