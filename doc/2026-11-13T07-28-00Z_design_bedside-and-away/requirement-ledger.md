# Bedside & Away: source requirement ledger

Intake snapshot: 2026-11-13. Scope: Drew's messages from 06:34 through 07:27 UTC. This is a preservation and traceability document, not an approved design or a declaration that the application is complete. Do not erase optional ideas simply because they are not selected for the first release.

## Reading this ledger

- IDs `BA-001` onward are stable; append new requirements rather than renumbering. A requirement may generate multiple designs and implementation issues.
- **R** = requested behavior; **O** = option/possibility, not a mandatory chosen solution; **Q** = unresolved meaning; **Research** = evidence-gathering task.
- **Prototype** = reported existing prototype capability, not full acceptance-tested fulfillment. **Partial** = only a narrower prototype capability exists. **Absent** = no working capability established. **Draft** = local, unpublished implementation work, not delivered. **Verified infrastructure** = limited deployed infrastructure evidence only. **Unknown** = must inspect or test before claiming.
- Status is conservative and based on recovered working context and the local file inventory. This ledger does not independently certify every implementation claim. Code, a passing unit test, and a live acceptance test are distinct evidence.
- Normative source is the user's message, not an assistant's choice of architecture. Each row retains its source and has an acceptance/design question. Conflicting or evolving directions remain traceable.

## Source index

All sources are messages in the current Bedside conversation, on **2026-11-13 UTC**. Exact message IDs are retained separately in the private source map; timestamps and stable source IDs identify statements in this review copy.

|Source|Time|Message reference|Subject|
|---|---|---|---|
|SRC-01|06:34:23|Owner conversation|Shopping inputs and comprehensive interactive tracker brief|
|SRC-02|06:36:58|Owner conversation|Defaults previews and historical edit review continuation|
|SRC-03|06:43:03|Owner conversation|Dedicated backend, database choices, store roles/invites|
|SRC-04|06:49:53|Owner conversation|Personal/shared/open stores, account linking, visibility|
|SRC-05|06:51:55|Owner conversation|Scoped research and portable policy structure|
|SRC-06|06:54:51|Owner conversation|Market evidence, credentials, extraction, summaries|
|SRC-07|07:01:26|Owner conversation|Tailnet deployment, same aesthetic, decision mode|
|SRC-08|07:07:51|Owner conversation|Use cf, not Wrangler; approved secret tooling|
|SRC-09|07:21:12|Owner conversation|Feature and main-branch status|
|SRC-10|07:25:19|Owner conversation|Repository commit, steering, stable IDs, split variants|
|SRC-11|07:26:09|Owner conversation|Steering on all items and general request/chat|
|SRC-12|07:27:34|Owner conversation|Spec issue, design PR, recursive design/implementation planning|

## A. Shopping, people, and physical constraints

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-001|01|R/Research|Resolve the private Amazon cart link in SRC-01, ingest, sort, deduplicate its list.|Absent: shared Amazon Business cart requires access; original contents not recovered. Do not call the seeded list a cart import.|
|BA-002|01|R/Research|Add related Amazon buys in the same vein; organize clear comparisons.|Partial: researched seeded candidates; prove relevance and provenance.|
|BA-003|01|R|Support two or more people, person-specific options and shared purchases in a comprehensive page.|Partial: person labels exist; not equivalent to multi-user identity, permissions, or onboarding.|
|BA-004|01|Research|Identify high-review, premium, thin/flat/flexible travel cushion that actually cushions butt/spine.|Partial: Purple Simply inferred candidate, not confirmed remembered product; verify dimensions, weight, evidence and tradeoffs.|
|BA-005|01|Research|Eye mask for nasal ResMed CPAP plus travel wrap pillow compatibility.|Partial: candidates only; exact mask and individual fit unknown.|
|BA-006|01|Research/Q|Replacement silicone component and heated CPAP tube.|Blocked on exact machine/mask/component/size; no substitution assumed.|
|BA-007|01|Research/Q|Approximately 2–5 water containers.|Ambiguous: carrying water vs humidifier tubs; retain quantity range.|
|BA-008|01|Research|Secure small CPAP stand, potentially extending toward bed edge; existing headboard.|Partial candidates; mounting dimensions, load/security and routing need verification.|
|BA-009|01|Research/O|One central or two individual headboard lights for both people.|Partial candidates; compare independent control and mounting.|
|BA-010|01|Research|Lights cover very bright through very dim and yellow-to-blue white temperatures.|Unknown against candidates; measure/specify actual dimming and CCT range.|
|BA-011|01|Research/O|Touch, Wi-Fi/smart, or both; optional color if reading quality remains good.|Unselected options; compare smart/non-smart cost and quality.|
|BA-012|01|Research|Shaded, thin/long, pet-durable lighting; avoid fragile paper; low heat/high efficiency/low price.|Partial candidate research; distinguish essential constraints from preferences.|
|BA-013|01|Research|Low flicker/no visible shuttering and stable output despite power dips/surges.|Unverified; require driver/dimming evidence and power-safety review, never promise immunity.|

## B. Item graph, metadata, organization, and interaction

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-014|01|R|Paste items, links, wishlist material and general information.|Prototype: basic intake; test mixed and malformed input.|
|BA-015|01|R|Add/remove categories with merge/archive handling for their remaining contents.|Partial: merge exists; specify removal without orphaning data.|
|BA-016|01|R|Track purchased, changed mind, alternative product and changed direction outcomes.|Partial: item states exist; outcome semantics and transitions need design.|
|BA-017|01|R|Submit notes/comments without changing items.|Prototype; verify independent event and authorship.|
|BA-018|01|R|Attach one comment to selected items across multiple visible locations.|Prototype multi-target comments; verify dedupe by canonical item ID.|
|BA-019|01|R|Abstract/meta items represent a desired kind with many candidate options; nested competing groups.|Partial: virtual groups and one-parent hierarchy; full association graph not designed.|
|BA-020|01|R|Abstract dynamic-fill items may have zero or many categories.|Partial: virtual items exist, dynamic filling absent.|
|BA-021|01|R|Specific product links and abstracts appear in all relevant category views.|Prototype category membership; verify identity stays singular.|
|BA-022|01|R|Individual and bulk actions work consistently on items, groups, categories and applicable text.|Partial; capability matrix and batch atomicity needed.|
|BA-023|01|R|Move entries above/below peers, to top/bottom, across arbitrary parent levels.|Partial: reorder/group moves exist; define per-view vs global order and graph cycles.|
|BA-024|01|R|Multi-select, dragging/click-dragging and sorting for items, virtuals, questions and categories.|Partial; mobile and keyboard accessibility not verified.|
|BA-025|01|R|Hide/archive/tag at individual and group levels.|Prototype for several objects; full type matrix unverified.|
|BA-026|01|R|Normalize string, key-value, arrays/dictionaries and JSON metadata into a standard representation.|Absent as comprehensive schema; preserve lossless raw input and typed values.|
|BA-027|01|R|Tags accept plain strings; colored labels can be recolored and clicked to browse matching items.|Prototype; contrast/accessibility and uniqueness tests needed.|
|BA-028|01|R|Many-to-many deduped tags/labels/categories; each label membership has explicit index order.|Partial: membership exists, per-membership ordering not established.|
|BA-029|01|R|Copy/move category memberships and bulk add/remove labels without duplicating canonical entities.|Prototype; test multi-category source and shared references.|
|BA-030|01|R|Categories are a special tag kind controlling review scope.|Partial; formal review-policy behavior absent.|
|BA-031|01|R|Categories and virtual items can be grouped and commented on themselves.|Partial; item comments exist, category graph/annotations incomplete.|
|BA-032|01|R|Annotate any part of any item/text, not only a whole entity.|Partial field-target comments; robust range anchors/rebasing absent.|
|BA-033|01|R|Quantities and arbitrary decision-relevant attributes are editable and commentable.|Partial; typed quantity/unit/attribute model needed.|
|BA-034|01|R|Hidden visibility toggles at relevant levels; archive display separately enabled via settings then contextual checkbox.|Partial controls; audit every view and inherited visibility.|
|BA-035|01|R|Permanent deletion only in buried admin/settings flow.|Prototype archived-item deletion; document recovery, retained history and permission checks.|
|BA-036|01|R|Create new categories/virtuals from templates or past entities without accidental literal duplicate copies.|Partial snapshot/history support; template lineage and copy/reference semantics absent.|
|BA-037|01,10|O/Q|Unique names considered as IDs, duplicate exception checkbox; later explicit stable unique ID for every item.|Prototype UUIDs and duplicate-name check. Preserve UUID identity, treat renameable-name uniqueness as configurable constraint, not identity.|
|BA-038|01|R/O|Groups can be ordered, unordered sets, or comparison breakdowns.|Partial metadata; distinct behavior/invariants need definition.|
|BA-039|01|R|Domain-specific comparisons: speed/strength/cost/smart/durable/classic, capacity, ports, GHz, Gb, GB, price and quotas.|Partial manual attributes/decision criteria; preserve units and distinguish bits/bytes.|
|BA-040|01|R|Projects are category-like lists supporting group discussion and decisions.|Partial project labels; project lifecycle/membership and discussion model absent.|
|BA-041|10|R|Variants belong to a general item or stand alone with associations preserved.|Draft local variants work, not deployed/accepted.|
|BA-042|10|R|Split a set into two or more sets or individual entries with simple associations.|Draft local work; require provenance, undo, cross-category and referenced-comment semantics.|

## C. Identity, comments, events, history, inventory, and analysis

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-043|01|R/Q|Generate UUID and retain browser identity indefinitely, using IndexedDB/service-worker/other storage.|Partial local UUID only. Browser deletion/eviction prevents a truthful forever guarantee; recovery and retention design required.|
|BA-044|01|R/Q|Collect fingerprint information and correlate a user across stored identity locations.|Absent deliberately; privacy, consent, purpose/minimization and unlink/reset must be designed; fingerprint must never grant authorization.|
|BA-045|01,04|R|Offer editable display name/nickname while retaining stable identity.|Partial; distinguish person label, browser ID and authenticated account.|
|BA-046|01|R/Q|Allow valid international Unicode/paste, reject unnecessary controls and client-side zero-width joiner.|Partial sanitation; resolve blanket ZWJ prohibition vs valid emoji sequences, define exact allowed/rejected classes server-side too.|
|BA-047|01|R|Emoji and multiple comments, visible author attribution.|Prototype; ensure server authoritative identity and correct guest attribution.|
|BA-048|01|R|Light/strong preferences and optional comments; preserve like/dislike event history per person.|Prototype preference state/events; test aggregation without double-counting repeated votes.|
|BA-049|01|R/Q|Duplicate submission delay with cancellation/override; unclear “unhard ... a number of times.”|Partial 15-second duplicate guard/override, not necessarily cancellable pending submission. Preserve ambiguous phrase for clarification, likely undo/unsend not assumed.|
|BA-050|01,02|R|Full inspectable change log, authorship, replay, undo/redo, reapply and old/new merge.|Partial snapshot log; bounded payloads and selective reapply do not establish complete durable event sourcing.|
|BA-051|01,02|R|Review edits across all items with authors; apply selected historical edits, merge or recreate from them.|Partial history interface; cross-store review and conflict semantics not established.|
|BA-052|01|R|Inventory and consumption with documented reasons.|Prototype purchase/consume/adjust; test units, stock bounds and corrections.|
|BA-053|01|R|Simple “what was done” submit page previews additions/removals inferred from text.|Partial local parser, not general contextual natural-language understanding.|
|BA-054|01|R|Infer references without explicit item names using context.|Absent; confidence, provenance and confirmation-before-mutation design required.|
|BA-055|01|R/O|Plan receipt interpretation and expenses per store/category.|Partial manual expenses only; OCR/receipt reconciliation absent.|
|BA-056|01|R|Budget and optional bar graph/summary, not necessarily prominent.|Partial budget and aggregates; chart/accessibility acceptance pending.|
|BA-057|01|R|Category allocation/splits and user-category/category-user action aggregation.|Partial equal split heuristic; explicit allocation and non-double-counting design needed.|
|BA-058|01|R|Analyze hour/time of day, day-of-month/week/date, holiday/busy/slow.|Partial date groups/manual labels; timezone, calendar and classification policies required.|

## D. Defaults, scoped regeneration, and research

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-059|01|R|Generated descriptions/sections must not auto-sync by default; explicit enable at any applicable level.|Partial policy settings only, generation pipeline absent.|
|BA-060|01|R|Include/exclude pattern arrays cover files and content.|Partial pattern settings; matching language, precedence and actual enforcement need tests.|
|BA-061|01|R|Refresh sections/reports/categories and propagate canonical changes wherever reused.|Absent end-to-end refresh; identity/transaction propagation design required.|
|BA-062|01|R|Select refresh dimensions: all items, reviews, competitors, descriptions, summaries, market position and attributes.|Partial selection configuration, not actual data refresh.|
|BA-063|01|R|Defaults additive unless explicitly unchecked, disabled or archived.|Partial config; overrides must survive refresh and migrations.|
|BA-064|01|R|Defaults configurable at item/category/project/text/report/graph and recommended-minimum levels.|Partial store/category/item scopes; full scope lattice absent.|
|BA-065|01,02|R|Disable incoming defaults; add/remove defaults; preview without acceptance.|Prototype policy UI; test distinction between automatic, disabled, preview and explicit custom.|
|BA-066|02|R|Show enabled, recommended-but-disabled and custom answers distinctly.|Partial; need complete provenance/inheritance display.|
|BA-067|02|R|Select all defaults or accept all uncancelled previews.|Prototype; batch transaction and stale-preview handling need tests.|
|BA-068|05|R|Market research on any entity or report section with optional steering.|Partial saved briefs; no executing research runner.|
|BA-069|05|R|Without steering, derive research intent from related history, comments and policies.|Absent automatic derivation; use only authorized relevant context, retain provenance.|
|BA-070|05|R/O|Portable common policy structure; possibly Rust shared crate.|Absent; portability requirement retained, Rust is an option not a selected dependency.|
|BA-071|06|R|Attach market reviews, comments, pictures and links to one/many entities.|Prototype pasted evidence/local resized images; not automated market ingestion.|
|BA-072|06|R|Extract core details from market evidence.|Partial regex/pasted fields; no general parser/OCR or verified source extraction.|
|BA-073|06|R|Summarize selections, subtree, whole category or each item separately.|Prototype deterministic saved-data summaries; not live research/LLM summaries.|
|BA-074|06|O|Amazon login can be supplied to unlock source access.|Not connected; use approved authentication flow, never request secrets in chat or assume account access.|
|BA-075|06|R/Q|Backend stores needed credentials and keeps them fresh.|Absent general provider integration; scopes, vault storage, rotation/revocation and provider restrictions need design. Existing worker token is not this feature.|

## E. Storage, authorization, accounts, and search

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-076|03|R|App has dedicated backend/database, isolated from Atlas for now.|Verified infrastructure: dedicated Turso database and private gateway/Worker path; retain isolation.|
|BA-077|03|O/Q|Cloudflare Workers, Durable Objects, D1 and Turso; private/shared databases from either location.|Partial: old Site D1 and newer Turso are separate, not synced. Provider options and location semantics need explicit design; DO not established.|
|BA-078|03|R|Store read-only/edit/admin roles, type and per-user settings.|Prototype role enforcement/settings; comprehensive negative authorization tests required.|
|BA-079|03,04|R|Create/add/invite/decline stores; view/hide/archive at store level.|Prototype invitation accept/decline/revoke and store actions; no invite email or outer Tailnet access grant.|
|BA-080|04|R|Each new user/device UUID gets its own default private store named by editable display name.|Partial authenticated provisioning only; automatic anonymous/device-first creation absent.|
|BA-081|04|R|One default shared store, plus private unshared stores and account-wide signed-in access.|Partial owner personal/shared provisioned; second-person access not established.|
|BA-082|04|O|Username/password or Google authentication.|Absent; current owner Tailnet identity is different authentication model.|
|BA-083|04|R|Guest starts on first activity, then attach guest database to account on sign-in without losing open work.|Absent; require secure claim, collision, rollback and concurrent-device semantics.|
|BA-084|04|R|Link multiple sign-ins into one identity.|Absent; prevent account takeover, require proof of both identities.|
|BA-085|04|R|Open store with separate categories by default.|Partial signed-in visitor read-only prototype; define “open” audience explicitly, not public-by-assumption.|
|BA-086|04|R|Search across all stores the requester can access.|Partial current entries only, hidden/archive excluded; define query scopes and denial behavior.|
|BA-087|04|O/Future|Later Snowflake embeddings of all text and Atlas search partitioned by visibility.|Absent and explicitly not current ingestion; revocation/deletion and embedding leakage design prerequisite.|
|BA-088|04|R|Scalable per-item visibility with users and groups.|Absent granular item ACL; store membership alone does not fulfill.|
|BA-089|04|R/O/Q|Inherited deny normally hard-blocks except explicit priority allow; consider Nix-style numeric priority.|Absent; formal conflict algorithm and safe defaults unresolved. Test every query/export/history/research path before enabling.|

## F. Deployment, steering, and design-mill delivery

|ID|Source|Kind|Requirement or preserved idea|Status / next evidence|
|---|---|---|---|---|
|BA-090|07|R|Move to private Tailscale URL with real backend, preserve same aesthetic.|Verified infrastructure for owner route; full visual/accessibility parity not verified.|
|BA-091|07|R|Working version before deployment; laptop deployment to Cloudflare/Turso with authorized secrets.|Partial deployment occurred as prototype, not all-feature fulfillment; enforce design/review/testing gates going forward.|
|BA-092|07|R|Decision-assistance mode.|Prototype weighted manual criteria, shortlist, reasons; multi-person constraints and uncertain ratings need design.|
|BA-093|07|O/Q|Login from ChatGPT website and pass through full site/data.|Absent; clarify connector vs embedding vs authentication, no arbitrary ChatGPT credential/session forwarding.|
|BA-094|08|R|Use native `cf`, not Wrangler; use approved 1Password/BWS daily/admin secret sources.|Deployment followed native cf; audit scripts/docs to prevent fallback. Least privilege, never publish secrets.|
|BA-095|09|R|Report what is built/not built and whether merged to main accurately.|This ledger is partial evidence; no application commit/design PR/main merge established. Distinguish local draft, deployed prototype and reviewed release.|
|BA-096|10|R|Commit application into code repository.|Pending: isolated branch/worktree prepared, application landing paused for design process; do not claim merged.|
|BA-097|10|R|Request changes/reprioritization from prominent items; trigger Muse or equivalent with versioned update.|Draft local steering UI, no verified live Muse webhook or completed execution loop.|
|BA-098|11|R|Steering applies to all other items, not only big/front-page ones.|Draft; multi-target, category/group, arbitrary entity and permission tests needed.|
|BA-099|11|R|General chat/request button independent of a selected item.|Draft; request record is not proof of external delivery.|
|BA-100|10,11|R|Steering triggers versioned updates rather than uncontrolled overwrites.|Absent verified end-to-end dispatch/review/apply loop; require idempotency, authentication, audit, stale-base conflict handling and rollback.|
|BA-101|12|R|Initial buildout has spec issue and design PR reviewing every statement.|Absent before intake; this ledger is an input, not the actual issue or PR.|
|BA-102|12|R|Design mill recursively breaks broad requirements into component designs, plans and implementation issues, then builds them.|Pending workflow; produce traceable design graph and acceptance gates instead of calling a prototype complete.|

## Required follow-up design packages

1. **Source preservation and specification:** retain every row, classify mandatory/optional/research, record explicit decisions and unresolved wording. Link spec issue and design PR when they actually exist.
2. **Canonical graph/data model:** UUIDs, variants/splitting, associations, category membership order, metadata/units, templates, annotations and migrations. Prove that splitting or regrouping cannot lose comments, evidence or history.
3. **Identity and authorization:** anonymous-to-account claim, linked sign-ins, personal/shared/open stores, groups and item policies. Denial must cover searches, summaries, events, exports, research, shared views and future embeddings. Decide fingerprint handling explicitly, not silently.
4. **Event and revision system:** durable replayable changes, batch transactions, undo/redo, merge/reapply, offline/concurrent edits, deletion retention and bounded storage. Snapshot logging alone is not the complete requirement.
5. **Defaults and research orchestration:** scope inheritance, opt-in refresh, provenance, preview review, provider evidence and credential lifecycle. Preserve custom and disabled values through regeneration.
6. **Steering and design-mill integration:** authenticated request submission, general and contextual requests, queue/Archer handoff, explicit scope, versioned proposals, approval/application boundaries, retry and run status. External messages are data, not blanket permission to execute arbitrary instructions.
7. **Shopping/inventory/decision UX:** recovered cart, product compatibility, multi-person comparisons, preferences, receipts, allocations, budgets and accessible mobile bulk interactions.
8. **Infrastructure and acceptance:** provider/storage decision, private deployment and identity checks, observability, backup/restore, token renewal, native cf deployment, test plan and release evidence.

Each package must link its BA IDs to design documents, child design/implementation issues, tests and release evidence. Mark gaps explicitly rather than dropping them from the release checklist.

## Critical unresolved facts and non-claims

- Original Amazon cart contents remain unavailable. The seed candidates are not a deduplicated copy of the cart.
- Exact CPAP machine/mask/component sizes, water-container meaning, and headboard dimensions remain unknown.
- Browser storage cannot guarantee permanent identity. Fingerprint correlation is neither authenticated identity nor an acceptable substitute for access control.
- Name-based uniqueness was tentative; subsequent explicit unique-ID direction should not be implemented as mutable names becoming database primary keys.
- Old Site D1 and current Turso represent separate installations, not a multi-provider sync system. A verified owner route does not establish second-user access.
- Local `steering.js`/`variants.js` and corresponding test files are drafts; file existence does not mean live functionality, webhook delivery, or passing acceptance.
- The [spec issue](https://github.com/developing-today/code/issues/9359) and [draft design PR](https://github.com/developing-today/code/pull/9361) exist. Neither all-feature completion, completed review, nor application merge to code/main is claimed.
- The source conversation contains a separately approved private Tailscale listener action. This ledger does not extend that approval to public exposure, new security settings, credential sharing, or autonomous high-impact actions.
