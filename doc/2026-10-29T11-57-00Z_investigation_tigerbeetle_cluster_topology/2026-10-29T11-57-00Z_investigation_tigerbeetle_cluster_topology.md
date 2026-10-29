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
shows the alternatives are worse on *both* axes Drew asked about: 3 replicas across
3 clouds has **less** failure tolerance *and* **worse** commit latency than 6 across
3 near sites, because flexible quorums let a 6-replica primary commit on its fastest
responders (including its same-site mate) while a 3-replica cluster spread across
clouds must wait on an inter-cloud round trip for every single commit. The cost of
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
| Partition 3 \| 3 | **Unavailable by design.** Neither side can reach the election quorum of 4, so neither can guarantee a unique primary; the system chooses safety over availability. Worth naming because it is the one partition where B is *less* available than a naive majority system — and it is the correct trade for a ledger. |
| Rolling upgrade / maintenance, 1 replica at a time | Available throughout with margin to spare — routine operations stop being fault-budget events. |

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
  guidance.
- **Option B (6, 2/site, near sites):** commit needs 2 backup acks. The first is
  the primary's **same-site mate** (sub-millisecond to ~1 ms within a site); the
  second is the nearest replica at another site — a few ms away by construction
  (the vendor's site guidance). Expected added commit latency: **~1–5 ms**
  (estimate from the vendor's "few milliseconds" site definition). Note the
  counter-intuitive result: **B commits roughly an order of magnitude faster
  than A**, because A's quorum is forced across clouds and B's is not.
- **Option C (single provider, 3 AZs):** intra-region AZ RTTs are typically
  **~1–2 ms** (planning range); commit latency comparable to B, marginally
  better — bought with the provider-correlated risk in §3.

**What latency does and does not cost here.** This is a token ledger for game
services, redemptions, and issuer conversions — human-scale operations, not
high-frequency trading. Even Option A's ~20–40 ms commit latency would be
*usable*; throughput is barely affected in any option, because TigerBeetle
pipelines and batches (up to 8,189 events per request) — latency per commit is
not throughput. The latency argument is therefore not "A is too slow to use";
it is that A pays a permanent per-commit latency tax *and* gets the worst
failure tolerance — the worst of both axes — while B's near-site placement is
what the protocol was designed around. The one workload that would feel A's
tax directly is a synchronous conversion flow (the #2063 linked-pair engine
submitting and awaiting commit before responding to a user), where +20–40 ms
lands on every user-visible conversion.

**Placement rule that falls out:** choose the three providers' regions as a
*m
...[truncated 1453 chars]