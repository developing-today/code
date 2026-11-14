# Bedside & Away: design-mill starter architecture

Status: **proposed starter design, not an implementation-complete claim or approved deployment plan**. Prepared 2026-11-13. This document decomposes the owner's requirements into reviewable designs and build work. The companion requirements ledger is authoritative for source-by-source coverage; a missing ledger mapping is a gap, not implicit scope removal.

## 1. Objective and boundaries

Build a private-first, multi-person research, shopping, inventory, and decision workspace. Preserve both concrete products and the abstract needs they compete to satisfy. Preserve user decisions, annotations, provenance, and history as the system evolves. Support a dedicated backend independent of Atlas. Enable steering at every item, selection, category, report section, and whole-store level, including general requests with no item selected.

Use the **design mill**: starter specification → domain designs → review and explicit decisions → implementation issues → tested changes → staged deployment. Large designs may produce additional designs before implementation. Every leaf implementation issue must have an acceptance test, dependencies, data/security impact, and a requirement reference. No iteration may silently discard an unimplemented requirement.

This is not a new public service launch, an authorization to purchase, or an authorization to expose credentials. CA, fleet, and PM designs are separate workstreams; shared concepts do not justify silently importing this application's data into those systems.

## 2. Preserve the prototype and establish evidence

The current application is a prototype, with a private Tailscale entry point, a laptop gateway, Cloudflare Worker, and dedicated Turso database. Preserve its URLs, aesthetics, data, identity mappings, and existing routes during design work. The earlier Sites deployment is a separate origin and datastore, not automatically a synchronized copy.

The root coordinator must freeze an evidence inventory before asserting implementation status: repository/branch/commit, deployed build identifier, schema version, migration ledger, route ownership, tests, and configuration names (never secret values). Distinguish **deployed and verified**, **implemented locally**, **partial**, **manual workflow**, **designed**, and **not implemented**. Unmerged variants/steering drafts must not be described as live.

Treat existing snapshot-based history, source attachments, local summaries, manual research briefs, and simple submissions as migration inputs. Do not replace the live prototype with a rewrite merely to satisfy architectural cleanliness. Prototype limitations are explicit backlog items, not reasons to lose data.

## 3. Proposed system shape

- **Frontend:** retain the existing responsive aesthetic. Shared entity/command schema, explicit pending/error/conflict states, accessible keyboard and touch interactions, decision and research modes. Offline drafts are local conveniences, never authority over server access rules.
- **Private entry/gateway:** retain the approved Tailscale path initially. Trusted identity originates at the validated gateway, not a browser-supplied actor field. Browser UUID is a local installation identifier, never authentication or permission.
- **Cloudflare Worker API:** authentication, centralized policy checks, command validation, bounded queries, research/job orchestration, signed callbacks, and response filtering. Deploy with the owner's `cf` tooling, not Wrangler.
- **Turso durable system of record:** dedicated application database, migration ledger, normalized entities, policies, audit events, transactional outbox, and derived-view metadata. No Atlas ingestion in the initial release.
- **Optional Durable Object:** only if a reviewed design demonstrates need for coordination, sessions, or contention control. It is not a second authoritative copy of shopping state.
- **D1/storage portability:** define a persistence interface and supported capabilities. Supporting D1 and Turso does not mean concurrent dual writes or automatic multi-master replication. Each store has one authoritative placement. Cross-placement transfer requires an explicit migration design.
- **Media:** bounded attachments initially; later object storage with access-controlled references, checksums, quotas, and retention. Do not expand snapshot-embedded images without a reviewed migration.
- **Background workers:** durable, bounded jobs for authorized research, summaries, imports, exports, and automation dispatch. No job may obtain broader visibility than its initiating authority.

Choose one canonical schema and policy evaluator before considering a Rust shared crate. Version JSON contracts first; share executable policy only where runtime parity and tests justify it.

## 4. Domain and identity model

Use opaque, immutable identifiers independently of editable names. Names may trigger collision warnings and explicit duplicate confirmation, but must not be primary keys. Every entity has schema version, revision, timestamps, provenance, and lifecycle state.

Core entities:

| Entity                                 | Purpose                                                                               |
| -------------------------------------- | ------------------------------------------------------------------------------------- |
| Account / installation / principal     | Separate authenticated identity, local guest continuity, and service actors           |
| Store / membership / invitation        | Private, shared, or explicitly open boundary; roles and invite lifecycle              |
| Item / need / variant family           | Concrete candidate, abstract requirement, or related alternatives                     |
| Collection / project / category        | Ordered or unordered groups; categories are a distinguished label/group type          |
| Placement / relation                   | Many-to-many association, independent ordering, parent context, and relationship type |
| Label / attribute                      | Colored labels and typed normalized values with units and source evidence             |
| Annotation / comment / preference      | Text or field anchors, user decisions, strength, quantity, and rationale              |
| Source / attachment / evidence         | URL or captured content with attribution, timestamp, and access classification        |
| Policy / default / override            | Visibility and configuration, explicit decisions distinct from inherited defaults     |
| Request / research job / report        | Versioned intent, targets, workflow state, and reviewed output                        |
| Inventory movement / receipt / expense | Observable operations, quantities, allocation, and correction history                 |
| Event / command / outbox               | Auditable mutation, concurrency control, and delivery lifecycle                       |

An item can occur in multiple categories without duplication. Ordering belongs to a placement within a parent; moving one placement does not silently reorder all placements. Cycles are forbidden for hierarchical containment, but non-hierarchical related-to links may be cyclic.

A variant family does not erase member identity. Group, detach, split into several sets, and promote a variant to standalone item are explicit reversible relationship operations. Comments anchored to a variant stay there; comments on a family stay on the family unless the user chooses a new target. Show the consequences of split/merge before applying. Merging duplicate records preserves old-ID aliases and source/history links rather than reusing an ID for a different object.

## 5. Accounts, stores, and access control

Provision one personal store for a new installation/account and one explicitly configured default shared household store. A shared store is not automatically readable by every new account. Open stores require an explicit creation/action and their own default category space. Hide is a personal presentation choice; archive is a lifecycle operation; neither is permission enforcement.

Define viewer, editor, and administrator capabilities separately from ownership and service identities. Invitations are scoped, expiring, revocable, and explicitly accepted or declined. Linking a guest session to a signed-in account requires proof of possession, a preview of stores and data being attached, collision handling, and an audit event. Do not infer that two devices belong to one person from fingerprinting or matching names.

### Policy decision proposal, requiring dedicated review

Evaluate store membership plus entity/user/group policies centrally. Default deny outside an explicitly granted scope. Inherited denies are hard blocks by default. If numeric priority overrides are retained, an override must be a separately authorized, narrowly scoped rule with a documented precedence order; an ordinary editor must not defeat an inherited deny by inventing a larger number. Equal-priority conflict resolves to deny. Reject ambiguous or cyclic policy inheritance.

Visibility applies to content **and existence** where required: search results, counts, breadcrumbs, aggregate scores, comments, old revisions, exports, caches, attachments, background jobs, callback payloads, and report excerpts. A container's visibility cannot broaden its children's visibility. Adding an item to a public collection never automatically publishes it. Recheck permissions at read and write time, and again when an asynchronous result is delivered.

Do not implement Google/password/ChatGPT login by passing through another application's cookies or session. Review provider-supported authentication, callback validation, account recovery, logout, and account-link semantics separately. Tailscale-only access remains valid until that review is complete.

## 6. Commands, history, undo, and concurrency

Use versioned commands with command ID, actor, expected entity/store revision, target IDs, normalized payload, and provenance. Store state changes and the corresponding event/outbox records atomically. Repeating a command ID returns the same outcome; a network timeout is not permission to apply it twice.

Retain immutable audit events and materialized state. Undo is a new compensating command, not deletion of the past. Redo/reapply checks current versions, permissions, and referenced objects. Offer a conflict preview rather than clobbering concurrent changes. Old event payloads must remain interpretable through versioned readers or deterministic upcasters. Snapshot history may remain as a compatibility layer while migration is verified.

Bulk operations provide a clear policy: atomic within a bounded transaction or a resumable batch with visible per-target results. Never report full success when only some targets changed. Debounce accidental repeated submissions while allowing deliberate repeated actions through a distinct command ID. Comments associated with multiple targets retain one comment identity plus target edges.

Hard deletion is a separate admin workflow with confirmation, retention implications, and recovery assessment. Hiding and archiving remain normal actions. An append-only event log does not justify retaining secrets or unnecessary personal data forever; redact or cryptographically remove prohibited payloads while retaining an audit tombstone when required.

## 7. Defaults, recommendations, and versioned refresh

Separate configured value, inherited default, recommendation preview, explicit disabled value, and user override. Track the author, source, scope, version, and rationale of each. Auto-refresh is off by default. Include/exclude patterns are bounded and validated; do not allow pathological regular expressions to stall the service.

Refresh may target selected records, a category, a tree, a report section, or individual fields (reviews, competitors, descriptions, summaries, market position, attributes). Changes to canonical items appear consistently in all placements, while placement-specific notes and ordering remain local. Proposed machine-generated updates are a reviewable change set, not silent overwrites. Bulk accept excludes cancelled previews and respects explicitly disabled defaults. A future default version must not resurrect a rejected recommendation without showing why it changed.

## 8. Commerce, evidence, and privacy

Store source URL, observed date, seller, currency, price/fees when known, unit-normalized attributes, uncertainty, and original evidence. Distinguish manufacturer specification, retailer claim, review anecdote, and assistant inference. Keep captured excerpts/media within applicable use limits. Do not label inaccessible Amazon cart contents as imported or deduplicated.

CPAP compatibility depends on exact machine, mask, size, connector, and accessory model. Keep unknowns unresolved and use a compatibility gate; no invented fit or health claims. Lighting comparisons separate dimming/flicker claims from measured performance and household power-quality issues. Research freshness must be visible.

Credentials never belong in item records, comments, event payloads, browser storage, exports, or model prompts. Use approved credential custody and least-privileged service access. A user session is not an indefinitely renewable scraping credential. Do not bypass merchant restrictions. Authentication and purchasing remain separate workflows; a research request is not an order authorization.

Persist a random installation ID only for disclosed continuity. Do not implement covert fingerprinting, respawning identifiers, or promises of browser storage lasting forever. Provide export/reset and explain eviction/private-mode limitations. Service workers must not retain private data after logout or account switch. International text is supported; validate structural fields against controls/bidi spoofing as appropriate without corrupting ordinary language. Emoji sequences in comments need different handling from identifier/label sanitation, including the owner's structural-input ZWJ restriction.

## 9. Decision, reporting, and submissions

Offer overview, comparison/decision, research, inventory, and activity views over the same records, not forked copies. Weighted comparisons show missing inputs, units, scoring method, and user-set weights; unknown does not equal zero or failure. Preserve competing priorities such as comfort, price, portability, durability, brightness, and compatibility.

Natural-language submissions and receipt interpretation produce a preview of inferred changes, ambiguous matches, quantities, and expense allocations. Commit only after the applicable user review or scoped automation permission. Inventory movements and financial summaries retain correction history; receipts do not authorize payments.

Reporting supports per-store/category/user breakdowns and time dimensions with an explicit timezone, calendar source, and definitions for holiday/busy/slow. Use access-filtered source records and avoid revealing hidden activity through totals. Reports show their source revision and refresh time. Multi-select summary can mean one aggregate summary or one summary per selected item; make the choice explicit.

## 10. Steering and Archer integration

Provide a request action on every item/variant, selection, category/project, report section, and store, plus an always-available general chat/request entry. Capture requested change or reprioritization, optional rationale, target IDs, current versions, allowed context, and requested response mode.

**Manual mode is a complete honest first stage:** save request, produce a versioned reviewable brief, let the user copy/export it, and permit manual status/result attachment. Label it manual. A saved request is not evidence that Archer received it.

**Connected mode requires a separately verified integration:** resolve Archer's supported endpoint/queue, authenticate with least privilege, and use the existing orchestration infrastructure rather than creating a shadow fleet queue. Store request and outbox entry in one transaction. Dispatch with idempotency key; record accepted/queued/running/needs-input/completed/failed/cancelled states, attempt history, deadlines, and delivery receipts. Signed callbacks must bind request ID, authorized actor, result version, and expected source revision. Reject replay and cross-store callbacks.

Archer may turn a request into design-mill child designs and issues, then supervise authorized build execution. The UI must distinguish a proposed change set from an applied version. Broad phrasing such as “improve this” cannot authorize widening visibility, disclosing credentials, purchases, or production security changes. Recheck authority before sending context and before applying a returned change. Cancel stops pending delivery; cancellation after external execution must report what already occurred rather than pretending rollback.

Versioned updates preserve user's edits made while work ran. Return conflicts for review or merge only within explicitly defined safe semantics. Watchdog/retry behavior must not produce duplicate issues or endless auto-restarts. Credentials and endpoint configuration are not complete until tested without leaking them into diagnostics.

## 11. Migration, deployment, and rollback

1. Inventory deployed state and local drafts; capture schema/build IDs and tested backups. Keep old deployments available only with clear read/write ownership labels.
2. Define stable-ID mapping from prototype records, snapshot history, source attachments, and installations. Export/import cannot treat browser UUID as account proof.
3. Rehearse migrations on a sanitized copy; compare counts, checksums, relationships, comments, ordering, permissions, and representative undo/history operations.
4. Use additive schema expansion, compatibility reads, and bounded backfill. Record completion checkpoints; no destructive cleanup before verification and rollback window.
5. Deploy API and frontend with compatible version ranges. Reject unsupported commands gracefully. Restrict rollout to the existing private audience.
6. Verify authenticated writes, denial cases, concurrent edits, source media, restoration, and route health. Preserve all unrelated Tailscale and laptop services.
7. Roll back app version only if it can safely interpret the current schema. Otherwise freeze writes and perform a reviewed forward repair or restore with explicit data-loss accounting. Never silently restore over newer user activity.

Use protected secrets through approved tooling. Track expiry/rotation and monitor failures without logging secret values. Laptop availability is an operational dependency of the current entry point; document it rather than implying an always-on cloud URL. Tests and design PRs do not merge or deploy themselves.

## 12. Non-negotiable invariants

- No state-changing request trusts client identity or browser UUID as authorization.
- No permission change is implied by moving, grouping, sharing a container, or changing a name.
- No mutation succeeds without an auditable version and deterministic command outcome.
- No lost variants, annotations, explicit rejected defaults, source provenance, or concurrent edits during grouping/migration.
- No secrets, unrestricted fingerprints, or private-session tokens enter ordinary application records.
- No automation is called connected until delivery, authentication, callback, and failure behavior are verified.
- No requirement is marked complete solely because a UI control exists.
- No new Atlas ingestion, public access, unrelated route changes, or destructive migration by default.

## 13. Design-mill decomposition and gates

The following are **proposed issue families**, not claims that repository issues already exist.

| Workstream                          | Child design required                                          | Acceptance gate                                                                                                                    |
| ----------------------------------- | -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| DM-00 Evidence and coverage         | Requirements ledger; prototype audit; source traceability      | Every owner statement mapped to requirement, conflict, clarification, or explicit deferral; all completion claims tied to evidence |
| DM-01 Domain and variants           | Entity/edge schema; split/merge semantics; normalization       | Same item in multiple groups; independent ordering; split and regroup preserve IDs, notes, and history                             |
| DM-02 Identity and ACL              | Guest/account claim; stores; invites; policy precedence        | Denial matrix covers API, search, history, summaries, exports, media, jobs, and priority override attempts                         |
| DM-03 Commands and history          | Event contract; idempotency; concurrency; undo                 | Duplicate and unknown-outcome requests cannot double-apply; undo after concurrent edit gives safe conflict resolution              |
| DM-04 Interaction and accessibility | Selection/tree actions; annotations; mobile decision views     | Keyboard/touch tests; operations work at all promised levels; non-color status indicators                                          |
| DM-05 Defaults and refresh          | Inheritance; preview/override; machine changes                 | Cancelled/disabled settings remain so across refresh and migration; bulk review preserves author/provenance                        |
| DM-06 Research and media            | Merchant access; evidence; extraction; attachments             | Unknown fit/price remains unknown; source freshness visible; no credentials in DB/logs; quotas and malicious uploads tested        |
| DM-07 Inventory and reporting       | Receipts; units; budget allocations; calendar dimensions       | Preview before ambiguous mutation; totals reconcile; timezone and permissions tested                                               |
| DM-08 Steering and orchestration    | Manual request contract; authenticated outbox/callback adapter | Request available globally and per-target; manual mode honest; replay/retry/cancel/conflict tests pass                             |
| DM-09 Backend and operations        | Placement interface; migrations; secrets; backup/restore       | Dedicated data remains isolated; restore rehearsal and version rollback tested; approved routes unchanged                          |
| DM-10 Release and documentation     | Phased rollout; acceptance matrix; owner-facing status         | Linked design PRs/issues/commits, clear built/partial/not-built states, no unreviewed scope deletion                               |

Review order: DM-00 first; DM-01/02/03 and DM-09 establish shared invariants; DM-04/05/06/07 and manual DM-08 may develop against those contracts; connected DM-08 follows endpoint and authorization review; DM-10 verifies the integrated result. Do not serialize unrelated designs unnecessarily, but do not let parallel teams invent incompatible identity, ordering, event, or ACL models.

Each child design must state alternatives, rejected approaches, compatibility, security/privacy risks, failure modes, tests, rollout, rollback, open decisions, and implementation issue graph. A review finding can generate another child design. The completion artifact is an evidence-backed coverage matrix plus integrated working behavior, not just a count of closed issues.

## 14. Decisions still requiring review

- Exact per-item policy precedence and narrowly privileged override semantics.
- Whether independent database placement is a near-term feature or an interface-only extension point.
- Identity provider and safe guest-account linking; cross-device ownership proof.
- Event retention versus privacy erasure, and large media retention/storage.
- The verified Archer queue/webhook contract and automated application boundaries.
- Prototype migration path and how old Sites-origin users are directed to the authoritative store.
- CPAP model/mask details and inaccessible cart contents for product-specific recommendations.

These are not reasons to omit the related requirements. They are explicit design gates. The owner's latest instruction is to establish the spec, design PR, decomposition, and implementation issues before treating the initial buildout as complete.
