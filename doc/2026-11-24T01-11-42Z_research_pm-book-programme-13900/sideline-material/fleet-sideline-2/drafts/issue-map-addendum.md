
## Additions after the census (2026-11-23)

The census table above is a snapshot. The items below were added to
this book's coverage after it was taken — the hosted edition, the
queue watchdog, the dispatch-test jobs store, Actions exec, the
compile coordinator and the bridge notes surface — in working order,
with their live dependencies. Each names the chapter that carries it.

### Working order

1. [#13765](https://github.com/developing-today/code/issues/13765)
   — the central compile coordinator's slice-1 build (backend verbs,
   the `placement_owner_state` / `placement_claims` /
   `placement_budgets` tables, proofs 1–3). Queue task 21517, which
   carried the whole four-slice build, is parked; its record names
   this slice as the next step. Everything coordinator-shaped waits
   on it, and nothing else does: dispatch and Actions exec both run
   the singular fallback today. Chapter:
   [The central compile coordinator](../concepts/compile-coordinator.md).
2. [#13774](https://github.com/developing-today/code/issues/13774)
   — the Actions exec live proof battery, halted at the Action's
   agent-install step on a platform execution-risk flag and recorded
   on the issue with its exact slice state. The machinery, the door
   and the tiered extension policy are landed and deployed; the
   end-to-end proof is the remainder. Chapter:
   [Temporary builds and custom connections](../concepts/temporary-builds.md).
3. [#13907](https://github.com/developing-today/code/issues/13907)
   — the `fleet bridge` notes family (get/send/receive/watch for
   class/name notes, #13895), replacing ad-hoc bridge access. The
   mailbox subcommand exists and is covered in
   [fleet bridge](../reference/bridge.md); this family is the
   follow-up, not a missing subcommand.
4. The hosted edition's production deployment — **no issue number
   exists yet.** `fleet-hosted` is implemented over fixture stores;
   its production wiring waits on the #9074 metadata database, a
   libSQL queue store over `fleet-queue` (#6741) and component E's
   tenancy database, and the deployment issue itself is unnumbered
   in both `wrangler.toml` and the package README. Filing and
   numbering it is the next action. Chapter:
   [The hosted edition: fleet-hosted](../concepts/fleet-hosted.md).
5. [#6369](https://github.com/developing-today/code/issues/6369)
   — the queue-transport-alert founding issue, still open above at
   order 10; the Worker it asked for is live and in service, and the
   remaining work is the follow-ons (#13493, deploying the
   hand-deployed Workers from Actions; #13503, folding the queue's
   alert into the PM console). Chapter:
   [The queue watchdog: queue-transport-alert](../concepts/queue-transport-alert.md).
6. [#13905](https://github.com/developing-today/code/issues/13905)
   — the Lab Book, owning the primary coverage this book only
   cross-links: cluster automation and scale, the lab-side
   Cloudflare pipes, and the estate view of DNS. Chapter:
   [Cross-links: automation, Cloudflare pipes and DNS](../concepts/lab-cross-links.md).

### Dependency map

- The compile coordinator (item 1) depends on landed work only: the
  broker store and its conditional-UPDATE claim pattern (#13146),
  the surface registry's caps and budgets, and broker-side GitHub
  reads (#8276). Actions exec (item 2) depends on the coordinator
  for its first-choice path only; its singular fallback is the
  dependency-free path it already runs.
- The hosted deployment (item 4) depends on #9074 (metadata
  database), #6741 (the `fleet-queue` store) and component E's
  tenancy database; the direct-read mint's wiring depends on the
  same deployment.
- The bridge notes family (item 3) depends on #13895's bridge-side
  notes surface landing first; the fleet subcommand family follows
  it, never precedes it.
- The watchdog follow-ons (item 5) depend on nothing in this list;
  #13503 depends on the PM console's programme, not on this book.
- The Lab Book cross-links (item 6) depend on the Lab Book lane
  under #13905 for their primary targets; the fleet-side chapters
  they point from are landed with this addendum.
- The dispatch-test jobs store carries no open dependency: it is
  live, its owner is `fleet dispatch-test`, and the #13844 audit's
  MOVE-CLEAN recommendation is a programme decision, not a blocker.
  Chapter:
  [The dispatch-test jobs store: fleet-dispatch-jobs](../concepts/dispatch-jobs-store.md).
