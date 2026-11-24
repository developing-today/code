# Projector internals

The projector is a Bun service in `pm-stack/projector/`, packaged as `pkgs/pm-projector/`, on port 7801. Its rule, from its own README, is the whole design: "It links and projects; it never becomes a fifth source of truth and never writes back to GitHub / Kaneo / Pad / Grist."

This chapter describes the projector's intake, its link registry, its read surface, and its consumer.

## What it holds

The projector holds links between subjects and the activity those links organise: items, comments, and per-subject reads over the linked set. Its schema is `projector/schema.sql`. Two SQL views carry the joined reads:

- `pmp_v_links_items` — links joined to items.
- `pmp_v_subject_activity` — activity per subject.

A projection is a derived record. The source systems keep their own records; the projector keeps the join. Deleting a projection loses the join and nothing upstream.

## Intake

Two intake paths feed the registry:

1. **Webhook intake** — `POST /github/webhook`. The endpoint verifies the HMAC signature on the delivery and dedupes by delivery GUID. A redelivered webhook is a no-op after its first acceptance.
2. **Reconcile polls** — `src/reconcile.ts`, driven through `src/cli.ts`. Polls close the gap between webhook deliveries: a missed delivery is a state the next poll corrects, not a permanent hole.

## The link registry

The registry is an HTTP surface:

```text
GET    /links
POST   /links
DELETE /links
```

A link joins subjects across systems — the GitHub issue, the Kaneo card, the roadmap row — under their subject forms (`gh:…`, `kaneo:&lt;slug&gt;/&lt;taskId&gt;`, `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;`, `session:…`, `orch:…`).

## Reads

```text
GET /items
GET /comments
GET /subjects/&lt;subject&gt;
```

The queue view is an HTML surface in the same service: `src/queue-view.ts` and `src/queue-view-html.ts`. Its route path is UNVERIFIED from the surveys — the files are named, the route is not. This chapter records the files and no path for the view.

## The consumer

The aggregator is the surveyed consumer. `aggregator/src/sources/projector.ts` reads links and the latest comments per subject from the projector and joins them into the aggregator's item projections. That join is what puts a Kaneo card's link and an issue's latest comment on one item page.

The projector's health and reconcile state feed the host snapshot the aggregator reads: the host-health collector records the `pm-projector-reconcile` timer state among the user units it snapshots.

## The package

`pkgs/pm-projector/` is the Nix package for the service. The surveys establish the package's existence and its place in the package set, beside `pkgs/pm-aggregator`, `pkgs/pm-dispatch`, and `pkgs/pm-git-sync`. The package's internal derivation structure is UNVERIFIED from the surveys, and this chapter does not describe it.

## Rules for working in the projector

1. Add a link type in the registry and the schema together. A link the views do not join is a write with no reader.
2. Keep intake idempotent. The GUID dedupe on the webhook path is the contract; a new intake path carries the same contract.
3. Never write back. A change that makes the projector edit GitHub, Kaneo, Pad, or Grist breaks the service's stated rule and turns a projection into a fifth source of truth.
4. Reconcile is the repair path. A divergence between the registry and a source system is fixed by the poll that re-reads the source, not by a hand edit to the registry.

Sources. survey/apps.md §7, §30, and the Nix packages summary; survey/website-pages.md §C, §G. Ground truth: laptop clone at 4d937b3, 2026-11-23.
