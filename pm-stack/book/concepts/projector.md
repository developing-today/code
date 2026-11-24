# The projector and projections

The projector is a Bun service on port 7801, in `pm-stack/projector/` and packaged as `pkgs/pm-projector`. Its README states its whole contract in one sentence: it links and projects; it never becomes a fifth source of truth and never writes back to GitHub, Kaneo, Pad, or Grist. Everything in this chapter is that contract worked out — what a link is, how links arrive, what a projection is, and who consumes it.

Read [The aggregator: one state, many sources](aggregator.md) first if you have not: the aggregator is the projector's named consumer in the surveyed code.

## Why a separate service links

The stack's sources each know their own items. GitHub knows issues and pull requests; Kaneo knows cards; Grist knows roadmap rows; the aggregator knows sessions and orchestration items. The same work exists in several of them at once — an issue, its board card, its roadmap row — and no source holds the cross-source identity. The projector holds exactly that: a registry of links between subjects, and projections built from the registry.

Keeping the registry in its own service enforces the limit in the README. The aggregator, which renders and raises questions, cannot quietly become the place cross-source identity is decided; the projector, which decides identity, cannot render the dashboard or write to a source. Each service's failure and change surface stays separate.

## Subjects and links

Items are named by subjects: `gh:` for GitHub items, `kaneo:&lt;slug&gt;/&lt;taskId&gt;` for Kaneo cards, `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;` for roadmap rows, plus `session:` and `orch:` subjects. A link records that two subjects are counterparts. The link registry is served over HTTP:

| Route | What it does |
|---|---|
| `GET /links` | Read the registry |
| `POST /links` | Add a link |
| `DELETE /links` | Remove a link |
| `GET /items` | Read projected items |
| `GET /comments` | Read comments |
| `GET /subjects/&lt;subject&gt;` | Read one subject: its links and activity |

The store's schema is `pm-stack/projector/schema.sql`. Its inventory (survey top-up 1, T2) is 10 tables and 2 views:

| Table | What it holds |
|---|---|
| `pmp_items` | GitHub issues and PRs keyed by repo-qualified ref `owner/repo#N` — the join key everywhere |
| `pmp_item_deletions` | Durable deletion markers keyed by ref; makes item deletion absorbing against stale replays (#6782) |
| `pmp_comments` | Comments across all repos, including PR reviews and review line comments Kaneo never syncs; primary key `(id, kind)`; `deleted` is the tombstone flag |
| `pmp_links` | The link registry: subject↔GitHub ref relations (`tracks` &#124; `implements` &#124; `relates`), human/agent-written through the registry routes. The map calls it the only non-rebuildable cross-service relationship in the projector schema |
| `pmp_deliveries` | Webhook deliveries; the GitHub delivery GUID is the unique key, so a redelivered webhook is applied at most once |
| `pmp_delivery_errors` | The first failed attempt per delivery, kept separately so a successful retry does not erase the original failure evidence |
| `pmp_sync_state` | Reconciliation cursors, one per `(repo, stream)`; streams are `issues`, `prs`, `issue_comments`, `review_comments`, `reviews` |
| `pmp_push_events` | Push events recorded for audit and later exec-strip aggregation; the projector does not write back to GitHub |
| `pmp_hot_events` | The append-only normalised change ledger over the webhook stream (#5204); `seq` is the monotonic cursor consumers paginate on |
| `pmp_consumer_checkpoints` | Per-consumer sweep checkpoints over `pmp_hot_events.seq`; a checkpoint only ever moves forward |

Two SQL views are the projection surface: `pmp_v_links_items`, every link joined to the linked item's live state and title, and `pmp_v_subject_activity`, one unified activity stream per subject, newest first. A projection, in this service's vocabulary, is a derived read built from the registry — links joined to the items they connect, and activity rolled up per subject — not a copy of any source's own tables.

## How links stay current

Two intake paths keep the registry current:

1. **Webhook intake.** `POST /github/webhook` receives GitHub deliveries. The endpoint checks the HMAC signature and deduplicates by delivery GUID, so a redelivered webhook does not create duplicate work.
2. **Reconcile polls.** `pm-stack/projector/src/reconcile.ts`, driven through `src/cli.ts`, polls and reconciles the registry against the sources. The host-health snapshot the aggregator consumes records the state of the `pm-projector-reconcile` timer among the units it tracks, which places the reconcile on the laptop's timer set.

When sources conflict, there is no human dispute-resolution rule in the projector code — no disputed flag, no arbitration, no multi-writer vote (survey top-up 1, T3). What exists is a set of coded overwrite guards that decide every conflict mechanically, by source timestamp and by primary key. The projector's own schema header states its posture: GitHub stays the source of truth for code work, Kaneo and Grist stay the source of truth for their own layers, and this database only holds links and projections; it is disposable and never writes back. The coded rules, one per object:

- **Items — monotonic; a stale source never wins.** The item upsert's guard key is `source_updated_at`: a stale source (older `updated_at`) never overwrites a newer stored item.
- **Items — deletion is absorbing, with one escape.** An item whose ref carries a deletion marker is not recreated by an event at or before the deletion's source timestamp (stale webhook replays, in-flight reconcile responses). An event strictly newer than the marker wins and clears it — GitHub is the source of truth, and such an event means the item demonstrably exists at a later source time than its deletion. The marker itself is monotonic: a later delete refreshes it, an older one never moves it backwards.
- **Comments — monotonic guard plus tombstone absorption.** A deleted (tombstoned) comment accepts no update at all; a live comment accepts only an update whose payload-reported last-update time is at or after the stored one. Tombstones are retained even when their item is deleted, because they are the absorbing guard that rejects stale comment replays. One named exception exists: the #3087 one-time migration correction for pre-fix review rows.
- **Links and edges — last writer wins, silently, on the full key.** The link upsert conflicts on `(subject, ref, relation)` and replaces `note`, `created_by`, and `created_at`; no history is kept. A different `relation` value is a different row, so `tracks` and `relates` for the same subject↔ref coexist rather than dispute. Removal is a plain delete on the same key. No code path marks a link disputed, contested, or pending review.
- **Deliveries — first delivery id wins.** Intake requires the delivery id; duplicates are recorded, not reapplied, and a failed first attempt is preserved in `pmp_delivery_errors` even when a retry later succeeds.
- **The projection contract behind all of the above:** a projection writes only what the event proves — an absent field proves nothing, neither defaults a value nor erases a stored one — and the only timestamps a row carries are ones its own payload reported (#4302, audited in #4303).
- **Checkpoints move forward only,** and never past the high-water cursor the server issued for that sweep, so a failed sweep resumes from its old checkpoint and replays safely.

## The queue view

The projector also serves an HTML queue view, in `pm-stack/projector/src/queue-view.ts` and `queue-view-html.ts`. Its route path is UNVERIFIED: the survey read the file names and the projector's port, and did not establish the path the view is served on. The view exists; do not cite a URL for it from this book.

## Who consumes the projections

The projector is also the known freshness publisher: its NixOS unit carries `PM_AGG_FRESHNESS_URL=http://127.0.0.1:4871/publish`, the aggregator's loopback-only freshness IPC listener (survey top-up 1, T5; see [The aggregator: one state, many sources](aggregator.md)).

The aggregator consumes the projector through `pm-stack/aggregator/src/sources/projector.ts`, which reads links and the latest comments per subject. That read is what lets an item detail page at `/item/&lt;subject&gt;` show the item's counterparts in the other sources, and what lets discussion and comment state travel with the subject rather than with one source's copy of the work.

No other consumer is established by the survey. In particular, whether the console reads the projector directly is UNVERIFIED; the console's surveyed data paths run through its own work module and the aggregator's twins.

## The limits, stated as behaviour

- **No write-back.** The projector never writes to GitHub, Kaneo, Pad, or Grist. A wrong link is corrected in the registry — by `DELETE /links` and a corrected `POST /links` — not by editing a source to match.
- **Not a source of truth.** The registry is the authority for one fact only: which subjects are linked. The facts about the work — status, title, state — remain in the sources, and the aggregator's conflict page, not the projector, is where source disagreements are published.
- **Pad is inside the no-write-back list and outside the link sources the survey names.** The README names Pad among the services the projector never writes back to. No surveyed adapter shows the projector reading Pad content; Pad has no subject form in the surveyed set. Treat Pad as protected by the contract, not as a linked source.

[Architecture](architecture.md) places the projector among the four core services; [The shared schema](shared-schema.md) covers the separate contract layer for the shared stores — the projector's own schema is its own, and the two are not the same mechanism.

Sources. Survey file `~/workspace/pm-book/survey/apps.md` (§7); `~/workspace/pm-book/survey/website-pages.md` (§C, §G); repo paths `pm-stack/projector/README.md`, `pm-stack/projector/schema.sql`, `pm-stack/projector/src/reconcile.ts`, `pm-stack/projector/src/cli.ts`, `pm-stack/projector/src/queue-view.ts`, `pm-stack/aggregator/src/sources/projector.ts`, `pkgs/pm-projector/`.
