# Cluster egress design v1 — proposed

- Issue: [#5030](https://github.com/developing-today/code/issues/5030); brief: [intake](2026-11-10_design_cluster_egress_5030.md); program: [#1091](https://github.com/developing-today/code/issues/1091), [#1817](https://github.com/developing-today/code/issues/1817).
- This builds on the detailed 2026-11-03 proposal and 2026-11-04 review on #5030, not a new measurement or approval. Account usage, workload forecasts, and current provider terms are **unverified**. Do not deploy paid upgrades on this basis.
- Constraints: 2 Gbit/s symmetric Brightspeed, no stated data cap; free tiers preferred, $5 Workers and $5 Turso in play, $25–30 Cloudflare possible, $75 excluded. No HashiCorp; configuration recoverable from source.

## Placement and topology

`client → Cloudflare cache → R2` for immutable public objects; `client → Worker → cloudflared outbound tunnel → narrowly scoped home origin` for dynamic work that cannot run at edge. Cache only explicitly public, safe responses with versioned keys and TTLs; avoid `r2.dev` for production. Place static builds, documents and package artifacts in R2 Standard; retain durable originals and use derivatives for media. Tunnel is not an egress bypass: every dynamic response and cache miss crosses the home WAN. Publish origin authentication and cache invalidation as part of workload deployment, not an implicit blanket cache rule.

Private administration uses Tailscale direct peer-to-peer when possible, DERP as best-effort fallback, not a throughput SLA. Keep admin and control endpoints private. Turso replicas are optional only for small data sets with acceptable consistency and measured sync amplification; bulk data and backups do not belong there. Local retention and recovery belong to #1085; an R2 off-site copy is optional after restore and cost testing. #1084 owns WAN/fabric; #1092 owns workload integration.

| Path | Cost / limit to check before admission | Home link |
|---|---|---|
| CDN → R2 | R2 Standard has zero egress pricing, but storage and operations are metered; cache and provider terms must be checked | Initial publication, not repeated edge reads |
| Worker → tunnel → home | Workers requests and CPU metered; tunnel has no stated numeric throughput guarantee | Both directions, including cache misses |
| Tailscale direct / DERP | No published per-GB price in prior review; DERP fair use applies; verify plan and commercial eligibility | Both directions, direct or relayed |
| Turso replication | Monthly sync is metered; verify account limit, overage behavior, and actual page-sync amplification | Sync traffic, if adopted |
| Backup → R2 / restore | Storage and request operations; verify storage class and retrieval charges | Upload and restore respectively |
| OCI | Prior ledger lists free egress but signup blocked by #4904; zero usable until tenancy and terms verified | Depends on actual topology |

Sources are the prior issue review and provider documentation: [R2 pricing](https://developers.cloudflare.com/r2/pricing/), [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [Tunnel](https://developers.cloudflare.com/tunnel/), [Tailscale routing](https://tailscale.com/docs/concepts/traffic-routing-through-tailscale), [Turso pricing](https://turso.tech/pricing) and [embedded replicas](https://docs.turso.tech/features/embedded-replicas/introduction). Prices, quotas, and account entitlements can change; recheck before committing. R2 zero egress does not mean zero storage or operation costs.

## Compression

Build Brotli and gzip variants for static text once, negotiate with `Vary: Accept-Encoding`; measure build CPU and output sizes on the actual Threadrippers. Compress sufficiently large API text responses, benchmark CPU and p50/p95 latency, and skip small payloads when overhead dominates. Use appropriately sized AVIF/WebP or other supported media renditions; retain originals and avoid recompressing compressed media and archives. No measured CPU cost or compression ratio is yet available. Compare uncompressed, compressed and wire bytes under representative load before enabling broad dynamic compression.

## One-page budget and enforcement model

Decimal units: 2 Gbit/s × 30 days / 8 = **648,000 GB per direction** at continuous saturation. At a 75% planning ceiling this is **486,000 GB per direction** if held constantly. These are arithmetic capacity equivalents, not ISP quota, forecast, or a guarantee. The ceiling is **1.5 Gbit/s per direction** on a rolling observation window; use a provisional 5-minute rolling mean and observe shorter bursts and p95 separately. Measure real available throughput and adjust the window after baseline testing. Ingress and egress are separate: never add their capacities or hide one direction's saturation behind the other.

| Monthly class | Forecast GB | Account / path and measurement |
|---|---:|---|
| Static and media | Unknown | CDN hits, misses, origin bytes; R2 GB-month and operations |
| Dynamic APIs | Unknown | WAN ingress/egress, Workers requests/CPU, cache bypass/misses |
| Private/admin | Unknown | WAN bytes, direct vs DERP, peak/p95 rates |
| Backups and restore | Unknown | Upload and restore GB, object counts, retention, recovery test |
| Turso sync, if used | Unknown | Provider monthly sync and amplification, account-specific limit |
| OCI | 0 available pending tenancy | Verify entitlement and actual use before budgeting |

After a representative 30-day baseline, set a forecast and class budgets with an owner and dated evidence. A month is not necessarily a provider billing cycle: record each provider's actual reset date, units, allowance and attribution; include other workloads sharing the account. Missing or stale counters are **unknown**, not zero. The external ledger at `/home/user/hatch-personal/workspace/ai-subscriptions/master-tracker.md` reportedly received a dated model on 2026-11-04; freshness and current account usage must be verified rather than assumed. #5203 owns counters, reporting and alerting.

Two separate controls must not be conflated:

1. **Link utilization:** independently evaluate each direction's rolling measured rate against 1.5 Gbit/s. Above that envelope alert and throttle/queue optional bulk first; preserve headroom for control, security, health, interactive recovery and the only recoverable backup. A monthly byte total cannot detect congestion. Implement and test actual shaping/admission at the WAN or job scheduler; an alert alone does not enforce it.
2. **Monthly budgets and provider quotas:** compare cumulative measured consumption in each actual billing window to its approved class budget or verified provider allowance. Proposed stages: warn at 70%, hold admission of optional bulk at 85%, and pause deferrable bulk at 95%. For hard provider walls, set an **earlier** hold accounting for in-flight work, forecast through alert latency, other consumers, and measurement lag; if these are unknown, do not admit optional workloads that could breach the wall. Never auto-disable health, security, control-plane, or the sole recoverable backup. Alert on stale/missing metrics; require operator review rather than claiming protection when enforcement is absent.

Stage thresholds are proposed operational defaults, not approved account quotas. Each protected job needs a documented enforcement point, idempotent pause/resume and operator override, with tests for alert delivery, lag, reset, reboot, and fail-safe behavior. Until then these are manual runbook actions, not implemented automation.

## Failure and recovery

| Event | Availability and ordered degradation |
|---|---|
| Fiber cut | Cached objects may remain available but uncached home origin fails without a configured alternate. Stop optional replication/backups; preserve management and monitoring on an actually provisioned alternate; serve stale only when explicitly safe. |
| 100 Mbit/s cable fallback | Candidate, not provisioned; 1/20 primary rate. First pause bulk backups/replication and large downloads, then disable uncached media/reduce rendition sizes; retain small APIs and management subject to measured capacity. |
| Starlink/hotspot | Emergency management only until plan, caps, path/CGNAT, placement and cost are verified. Disable public origin and bulk; alerts and recovery first. |
| Provider outage or quota wall | Fail closed for optional publishing/sync; no silent unlimited home fallback or paid overage. Preserve private management and restore path; use explicitly safe stale content where available. |

Alternate-WAN routing, health detection, traffic shaping, and recovery are not implied by having a cable or mobile link. Test cutover and restore under the actual topology before relying on them. A second fiber is not assumed: TriCounty service and price at the address remain unknown.

## Delivery and outstanding decisions

- #5203 covers per-direction counters, provider usage, monthly report, 70/85% alerts and hold response; its current scope does not by itself establish the rolling-rate control, 95% handling, or end-to-end admission/shaping enforcement described above. Those are implementation work, not existing capability.
- #1084 owns fabric and paths, #1085 backup/restore, #1090 observability, and #1092 workload integration. Concrete service-specific CDN, tunnel, origin authentication and cache rules require a workload and should be tracked with that integration rather than presumed deployed.
- Verify live Cloudflare/Turso/Tailscale accounts, actual unused allowances, shared consumption and provider terms; confirm TriCounty availability. Collect the baseline and benchmark compression before numerical forecasts or production admission. Review thresholds and any spend or provider-plan change with the owner.
