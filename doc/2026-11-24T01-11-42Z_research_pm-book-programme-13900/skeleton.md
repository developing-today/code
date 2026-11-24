# Project Management book — provisional skeleton (coordinator, 2026-11-23)

Status: RECONCILED 2026-11-23 ~18:45 CST against PM-SURVEY (50 pages/routes: 29 aggregator + 21 console views; question sets F1–F8; 36 components; disagreements D1–D14) + PM-CENSUS ledger as settled by TURSO-SETTLE (22 workers, 50 Turso DBs). Writer rules from the survey: (a) roadmap chapter frames Drew's four types but states the code reality — Kind is Task|Epic only, programs carry the initiative/project level (D10); (b) push chapter traces the real flow — laptop aggregator + VAPID Web Push; notify-relay is built but NOT deployed, its answer routes unrouted (D5); (c) workers chapter reconciles census 22 vs platform workers.json 20 vs README 18 (D6); (d) models chapter states the aggregator makes no LLM/embedding calls — voice-line Workers AI inference + the Atlas embedding dependency are the real model surface. Placement: `pm-stack/book/` + `pm-stack/book.toml` (mirrors pkgs/fleet). Deploy: new flake output + build workflow mirroring fleet-docs.yml; joins the mcpx-docs.yml deploy job under subpath `pm/` (siteUrl "/code/pm/") — the deploy-job edit is the high-blast-radius change: parse YAML + build the deploy variant locally before merge.

Form rules (bookwright v5): every chapter's sole H1 = its SUMMARY entry text; every reference entry same shape (syntax/shape, inputs, defaults, exit behaviour, worked example, failure shapes); escape angle-bracket placeholders; zero-warning build is the release gate; stage files before building under Nix.

## Sections and chapters

- index.md — Project Management (what the PM stack is, why it exists, who it serves, how to read this book)
- getting-started/
  - installation.md — Installation and bootstrap (aggregator + console + workers, local and deployed)
  - quickstart.md — Quickstart (read the console, answer a waiting question, follow an item)
  - shape.md — The shape of the system (aggregator, projector, dispatch, workers, website, phone)
- concepts/
  - architecture.md — Architecture
  - aggregator.md — The aggregator: one state, many sources
  - projector.md — The projector and projections
  - shared-schema.md — The shared schema
  - roadmap-hierarchy.md — The roadmap hierarchy: initiatives, projects, epics, aggregators
  - prioritisation.md — Prioritisation: how order is decided and recorded
  - questions-and-answers.md — Question sets and the answer-back flow
  - push-notifications.md — Push notifications: detection to phone to answer
  - kaneo-sync.md — The Kaneo sync and the other PM apps
  - dispatch-methods.md — Queues and dispatch methods, from the PM side (links to fleet book)
  - claimant.md — The claimant: keeping sessions honest
  - scheduled-jobs.md — Scheduled jobs and the history they keep
  - health-surfaces.md — Health, quota and usage surfaces
- reference/
  - website-pages.md — The aggregator website, page by page (29 routes, 1:1 with survey census)
  - console-views.md — The console, view by view (21 hash views, 1:1 with survey census)
  - aggregator-routes.md — Aggregator HTTP routes and API
  - question-sets.md — The question sets, enumerated (F1–F8)
  - workers.md — The workers (the 12 PM-assigned workers in full; the estate's other 10 named with their covering books; the 22/20/18 count reconciliation, D6)
  - configuration.md — Configuration and environment
  - data-stores.md — Data stores: all 50 Turso databases (PM's 15 gaps in full incl. the frozen eight as a group; the 34 covered named with their covering chapters; fleet-dispatch-jobs named as the fleet book's gap, NOT covered — settled ledger arithmetic: 34 covered + 15 PM gaps + 1 fleet gap = 50; worker estate is now 24 after D17, see ledger annotations)
  - models.md — Model dependencies (voice-line inference, the Atlas embedding model, and the no-LLM-calls fact)
- tutorials/
  - answer-a-question.md — Tutorial: answer a waiting question end to end
  - trace-a-push.md — Tutorial: trace a push notification from event to answer
  - follow-an-item.md — Tutorial: follow an item from issue to done
  - read-the-quota-view.md — Tutorial: read the quota and usage views
- library/
  - shared-schema-package.md — The shared schema as a library
  - aggregator-internals.md — Aggregator internals
  - projector-internals.md — Projector internals
- opencode-openchamber/ (the liftable section — own SUMMARY part, own framing)
  - index.md — OpenCode and OpenChamber: our forks programme
  - the-forks.md — The forks: what they are and why we carry them
  - our-changes.md — Our changes, shipped and in flight
  - goals-and-intentions.md — Goals and intentions
  - opencode-issue-map.md — OpenCode: prioritised issue map
  - openchamber-issue-map.md — OpenChamber: prioritised issue map
  - dependency-map.md — The forks programme dependency map
- llm-plans/
  - the-plans.md — The plans we use (dated reads)
  - pools-and-routes.md — Pools, routes and the switchboard
  - models-by-tier.md — Models by tier, and why (scout/workhorse/specialist/summit)
  - benchmarks-and-evidence.md — Benchmarks and real-use evidence
  - walls-and-resets.md — Walls, resets and quota doctrine
  - costs.md — Costs and monthly usage (Drew addition 2026-11-23 ~19:01 CST: Turso + Cloudflare Workers costs; monthly usage across all core quotas; PAYG separate from plan/included; light-touch — plans incl. student arrangements covered as existing, never detailed; figures dated)
  - priorities.md — LLM plans: prioritised list
- development/
  - layout.md — Repository layout
  - building-and-testing.md — Building and testing
  - add-a-page.md — Add a page or projection
  - program-roadmap.md — PM stack research and implementation roadmap
  - issue-map.md — Project Management: prioritised issue map and dependency map
  - keeping-this-book-current.md — Keeping this book current
- operations/
  - services.md — Running the stack as services
  - health-and-healing.md — Health checks and the heal stack
  - deploying-this-book.md — Deploying this book
  - troubleshooting.md — Troubleshooting
  - faq.md — FAQ
  - glossary.md — Glossary

Chapter count: ~48 + whatever the page census adds to reference/. Family range: fleet 55, atlas 57, id 67, mcpx 84+.
