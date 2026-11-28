# Design brief: cluster egress strategy for 2 Gbps home fiber

- Date: 2026-11-10
- Status: **design-mill intake brief** — this document is the seeded
  source material for the design build-out, not the design itself.
  The owning summit session builds the design out from here in the
  design-mill pattern (statement inventory → state-of-the-art
  expansion → versioned designs → granular implementation issues).
- Issue: [#5030](https://github.com/developing-today/code/issues/5030)
  (design(cluster): egress strategy for 2 Gbps home fiber — budgets,
  tunnel topology, failure matrix)
- Parent: [#1091](https://github.com/developing-today/code/issues/1091)
  (cloud extension — egress half). Program:
  [#1817](https://github.com/developing-today/code/issues/1817)
  (basement cluster); program home `planning/roadmaps/basement-cluster.md`.
  Absorbs the content of superseded #1814.
- Source: idea register 2026-10-20 (goal
  `stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs`), plus
  the #5030 issue body, which remains the canonical statement set
  until the summit session's statement inventory supersedes it.
- Program rules carried from the roadmap: no HashiCorp anything;
  everything bootstrappable from config.

## The constraint

Bandwidth is the scarce resource, not compute. The basement fleet has
very fast CPUs, ≥3 TB RAM and ~200 TB of disks at near-zero marginal
cost — behind exactly **2 Gbps symmetric** home fiber (Brightspeed,
Osseo). A second fiber line would make 4 Gbps total — still small at
scale. Cable offers 100 Mbps at higher cost (possible redundancy
pickup). TriCounty Communications fiber may reach the address (open
question below).

## Strategy direction (already stated by Drew, on #5030)

Tunnel most traffic to Cloudflare or Turso, or anywhere reachable via
Tailscale: shunt bulk traffic to SaaS with cheap or **free egress**
(e.g. Cloudflare R2's zero-egress pricing, Cloudflare cache/tunnels),
compress aggressively, and only cross the home link with what must.

Budget guardrails (already stated on #5030, not open for the mill to
re-decide): Turso $5 plan and Cloudflare $5 Workers plan in play;
free tiers preferred; a $25–$30 Cloudflare plan is possible;
**$75 is out**.

## Acceptance (from #5030 — the design must deliver all of these)

- [ ] Classify the app's traffic: what can be cached at the edge
      (CDN), what can live on free-egress object storage (R2), what
      genuinely must be served from home.
- [ ] Design the tunnel topology: cloudflared from the cluster,
      Tailscale links, Turso edge replicas — with per-path egress
      costs written down.
- [ ] Compression plan per traffic class (static assets, API
      payloads, media) and the CPU cost of doing it on the
      Threadrippers (plenty available).
- [ ] Failure/redundancy matrix: fiber cut, cable fallback
      (100 Mbps — what degrades, in what order), Starlink/hotspot
      emergency tier.
- [ ] Output: a one-page egress budget model — expected GB/month per
      class vs the ~2 Gbps link and each SaaS allowance — kept
      current in the subscription/usage ledger.
- [ ] Design recorded on #5030 or a linked design doc/PR, stating
      the bandwidth budget against the 2 Gbps symmetric constraint
      and what happens when it is exceeded.
- [ ] Follow-up implementation issues filed and linked for any build
      work the design calls for.

## Open questions (record answers on #5030; do not stall on them)

- Is TriCounty fiber actually available at the address? (Needs an
  address check.)
- Which SaaS egress allowances are we already paying for but not
  using (ties into the cloud ideation ledger)?

## Mill build-out notes for the owning session

- V1 = what exists today (roadmap: egress is named the scarce
  resource; the OCI always-free fleet / R2 cold-tiering direction is
  already recorded under #1091 in `planning/roadmaps/basement-cluster.md`).
- The budget model output belongs in the subscription/usage ledger
  once produced; this brief does not invent usage numbers — the
  summit session derives them from the app's actual traffic classes.
- Questions that only Drew can settle batch up with options and a
  recommendation; everything else is decided in the mill.
