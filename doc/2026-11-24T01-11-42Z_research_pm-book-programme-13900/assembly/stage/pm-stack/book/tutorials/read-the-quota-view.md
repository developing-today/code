# Tutorial: read the quota and usage views

This tutorial reads the quota surfaces and nothing else. It names each surface, the data family behind it, and the order to read them in. Quota figures — pool percentages, dollar amounts, reset times for a concrete pool — are not in this chapter. Those figures carry dates and live in the LLM-plans chapters. A figure copied into a tutorial is stale on arrival.

## Step 1: Open the usage and billing page

Open `/quota`. Its manifest entry in `aggregator/src/pages.ts` reads: "Token quotas, PAYG dollars, and every SaaS service's usage vs its limits."

The machine twin is `/api/quota.json`. Page and twin read the same assembly. Read the page for the layout and the twin for the record shape.

The page carries three data families:

- Token quotas, per pool.
- PAYG dollars.
- SaaS services, each read against its limits.

The code behind the page lives in the quota module family in `aggregator/src/`, in files named `quota-*.ts`: current state, forecast, anomaly, reweight, admission, and the weekly review. Billing machinery sits behind the same page: charges catalog, collect, and match modules; spend attribution, cards, dollars, and rollups modules; and the SaaS extract module.

## Step 2: Read one service in detail

Open a service detail page:

```text
/quota/service/&lt;id&gt;
```

Its parent in the reachability map is `/quota`. Its twin follows the pattern `/api/quota/service.json`, read with a service selector in the query. The exact query contract is UNVERIFIED beyond the route existing in `aggregator/src/server.ts`. This tutorial names the route and the parent, and no parameter contract.

Use the detail page to answer questions about one service. Use `/quota` to answer questions about the whole estate. Do not reconstruct the estate view by adding detail pages together; the page assembly and the detail read are separate code paths.

## Step 3: Refresh, and know what a refresh does

The page exposes a refresh action:

```text
POST /quota/refresh
POST /api/quota/refresh
```

Collection scripts feed the store the page reads: `scripts/gather-quota.ts`, `scripts/forecast-snapshot.ts`, and the usage-snapshot scripts for append, backfill, and compact. Charge candidates come from `scripts/produce-charge-candidates.ts` and `scripts/confirm-charges-surfaced.ts`. GitHub billing has its own collector, `scripts/collect-github-billing.ts`, over the source in `aggregator/src/sources/github-billing.ts`.

A view that disagrees with a provider's own console is a stale-view problem first. Run the refresh, then re-read. The failure shape and its checks are in [Troubleshooting](../operations/troubleshooting.md).

Browser-dependent sources are a separate family. The exporters in `pm-stack/exporters/` cover sources reachable only through a logged-in website. In the surveyed tree the only adapter present is a synthetic fixture; real per-provider adapters are follow-on work, and live per-provider production use of the exporter core is UNVERIFIED. Two laptop exporters feed the same quota system from outside the aggregator: `pm-stack/cloudflare-export/cloudflare-quota-export.mjs` and `pm-stack/meta-exporter/meta-quota-export.mjs`, each with a service and timer.

## Step 4: Read the Cost Explorer

Open `/aa`. The page has two tabs:

- **AA Explorer** — Artificial Analysis measurements, frontier charts, and plan, API, and per-task efficiency.
- **Model Explorer** — every model in the fleet's catalogs, with pool seats, plan, and measured value.

The twin is `/api/aa.json`. The computation asset is `/assets/aa-explorer-computations.js`. The code is `aggregator/src/aa.ts` and `aggregator/src/aa-batches.ts`.

Read `/aa` for comparison questions: which model, which seat, which measured value. Read `/quota` for limit questions: usage against a cap. The two pages share source data and answer different questions.

## Step 5: Read the quota-adjacent surfaces

Four more surfaces carry quota state in other forms:

- The front door `/` carries a quota summary as one of its glance panels, next to needs-you, projects, and health.
- Push notifications carry quota state to the phone. The classes `quota-wall`, `quota-near-wall`, and `section-risk` land on `/quota` when tapped. The full trace is in [Tutorial: trace a push notification from event to answer](trace-a-push.md).
- `/notifications` measures the alert throat: the sent-vs-acted-on roll-up over the send ledger. Read it to judge the notifications, not the quotas.
- The console carries platform state at `#more/platform`, releases at `#more/releases`, machines at `#more/machines`, and services at `#more/services`. These are console views over platform and host state. The console has no quota page of its own in the surveyed view list.

## Step 6: Record a reading correctly

A reading from these surfaces is a dated observation, not a fact about the pool. When a reading matters — a wall, a forecast, a reweight proposal — record it with:

1. The surface read (`/quota`, the service detail page, or `/aa`).
2. The date and time of the read.
3. The refresh state: read before a refresh, or after one.

Quota-side prioritisation records proposals, not silent changes: forecast and reweight proposals live in the quota module family and in `scripts/quota-reweight-proposals.ts` and the weekly review. Detail of that proposal machinery beyond this pointer is UNVERIFIED in the surveys.

## You are done when

- Each number you report names its surface and its date, and comes from the LLM-plans chapters or from a live read you just made — never from this tutorial.
- A service question is answered from its detail page, and an estate question from `/quota`.
- A stale-looking view has been refreshed once and re-read before it is reported as a discrepancy.

Sources. survey/website-pages.md §A (entries 10 and 12), §B27, §D; survey/apps.md §15, §27–§28; survey/roadmap.md §3. Ground truth: laptop clone at 4d937b3, 2026-11-23.
