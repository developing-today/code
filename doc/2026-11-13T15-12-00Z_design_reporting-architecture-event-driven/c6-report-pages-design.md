# C6 — Report pages, freshness, questions, and annotations

Status: component design for #9574 under reporting-architecture epic #9568.
Consumes [`note.md`](note.md), C4's forecast records, C7's rollup/timeline
design, C8's Atlas ingestion, the existing PM-stack questions and decision
machinery, and the 2026-11-13 amendment on #9574. This record specifies page
contracts; it does not implement them or replace their existing producers.

## Decision summary

1. **A page is a projection of versioned data, not the data store.** Producers
   publish a validated JSON page-data blob with schema version, source
   cursors, `as_of`, and content version. The renderer swaps the current blob
   pointer atomically; it never runs a provider query or mutates narrative
   content while rendering. Presentation and data can evolve independently.
2. **Freshness is measured per page class.** Every page advertises its
   expected update interval and last successful source update. A missed
   deadline is a defect, shown as stale with the last-good data still
   available and an exception routed to Reports. A failed refresh never
   replaces good data with an empty or partial blob.
3. **Phone is the primary layout.** One-column, narrow-width reading and
   controls come first; desktop may add columns and density. Actions, forms,
   tables, charts, quote resolution and comments remain usable at 320 CSS px
   without horizontal scrolling (except explicitly scrollable chart/table
   regions with an accessible text alternative).
4. **Questions extend the existing decision system.** Pages embed the
   existing questions-module contract and decision-set processor. Answers
   are durable, idempotent events; action-bearing answers enqueue work and
   report outcome back to the page. Reset actions require an explicit human
   confirmation and use the pool steward's existing reset machinery.
5. **Comments and anchors bind to immutable page versions.** A comment
   thread can target a page, stable section, or text range. Quote URLs retain
   the original text and content hash. Re-resolution never silently presents
   a fuzzy match as exact. Central page history remains the source of truth;
   no forks or CRDT are required for V1.
6. **Incident pages are selective, searchable reports.** A page is earned by
   meaningful impact, recurrence, or a durable remediation need, not by every
   quota wall or reset. The index has stable IDs and feeds C7's timeline and
   C8's ingestion through metadata links.
7. **The #15002 boundary is strict.** The candidate unattended cloud-reset
   rule (all other queues down and Claude at 100%) remains out of scope until
   an approving/amending FDR is recorded under #9180. C6 supports asking and
   human-confirmed actions only; no timer, hook, or answer path may bypass
   confirmation to trigger that reset.

## 1. Page data and refresh contract

### 1.1 Page-data envelope

Each rendered page consumes one immutable, validated envelope. `page_id` is
the stable UI identity; it is not a storage key. Persist the version through
the existing canonical report-record append path as `record_id`, and retain a
content hash for byte-for-byte content identity. Period pages additionally
carry the C7 `period_id` and link their `record_id` to the canonical report
record; C6 must not create a parallel report database. A producer
may compose it from C2/C3 events, C4 forecasts, existing PM-stack records,
and attributed editorial blocks; the page renderer does not own those facts.

```jsonc
{
  "schema": "c6.page-data/v1",
  "page_id": "quota:workhorse:weekly",
  "record_id": "<canonical report-record id>",
  "page_class": "quota | status | incident | period | index",
  "content_version": "sha256:<canonical-envelope-hash>",
  "generated_at": "<ISO-8601 UTC>",
  "as_of": "<ISO-8601 UTC>",
  "freshness": {
    "budget_seconds": 900,
    "last_source_event_at": "<ISO-8601 UTC>",
    "state": "fresh | stale | unavailable",
    "reason": null,
  },
  "sources": [{ "id": "<stable source id>", "cursor": "<opaque cursor>", "observed_at": "<ISO-8601 UTC>" }],
  "template": { "id": "<template id>", "version": 1 },
  "sections": [{ "id": "<stable section id>", "title": "...", "kind": "facts | narrative | chart | questions | comments", "data": {} }],
  "links": [{ "rel": "timeline | issue | incident | source", "href": "..." }],
}
```

The canonical hash covers the exact rendered-source content (including
section IDs and values) but excludes transport-only timestamps and storage
identifiers. It is immutable once published; it is distinct from both
`page_id` and `record_id`. The server validates required fields, known schema
version and source completeness before atomically promoting a blob. Unknown
fields are preserved for forward compatibility; unknown major schema
versions are rejected and leave the prior blob live. Readers receive the
last-good version plus an explicit stale/unavailable state, never a silent
blank.

### 1.2 Freshness budgets and exception path

Budgets are maximum time since a successful producer observation, not a
promise that a source changes on that cadence. Freshness is computed by the
reader from per-source successful-observation timestamps and the class budget,
not trusted from a cached envelope state; the response may expose the computed
state while retaining the original envelope and `as_of`. A separate breach
detector emits deduplicated exceptions even if no reader happens to load the
page. Event-driven producers refresh
on relevant source events; a lightweight scheduled reconciliation may detect
missed events, but does not become a second data producer.

| Page class               |                                                                 Freshness budget | On breach                                                                                                                                              |
| ------------------------ | -------------------------------------------------------------------------------: | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Quota / pool status      |                                                                       15 minutes | Mark stale, retain last-good values, alert Reports and enqueue one deduplicated refresh/diagnosis task.                                                |
| Service / health status  | 5 minutes while a check is expected; otherwise show the check's declared cadence | Mark the specific check stale; use the existing page-health-push exception path for actionable failure.                                                |
| Active incident          |                                                15 minutes while incident is open | Show last update and stale badge; enqueue a follow-up and notify the incident/report sink.                                                             |
| Closed incident          |                         24 hours after a new linked finding; otherwise immutable | Append a versioned update when new evidence arrives; do not poll unchanged history.                                                                    |
| Period / forecast report |      1 hour while its source window is open; at least one final refresh at close | Show `as_of`; stale breach creates a Reports exception, not a fabricated estimate. C4's prediction freshness remains authoritative for forecast facts. |
| Reports index            |                                    15 minutes after a report create/update event | Retain searchable last-good index and enqueue a deduplicated rebuild.                                                                                  |

The producer records the failed source, attempted time, error class and
recovery reference in the health/exception record. Exceptions are deduped by
`page_id + source + breach-window`; recovery closes that exception with a
freshness-restored event. Existing last-good content remains readable with
its original `as_of`. Budgets are initial defaults: implementation may tune
them only with recorded evidence and without weakening the hard visibility
of staleness.

### 1.3 Refresh and storage

1. An event or reconciliation identifies affected page IDs and source
   cursors; it queues a refresh, it does not render inline in a webhook.
2. One worker builds the complete envelope from an explicit source snapshot.
   It uses idempotency key `(page_id, schema, sorted source cursors, template
version)`; retries of the same inputs cannot duplicate blocks or edits.
3. Validate the envelope and render a preview. Persist the immutable canonical
   report record and version metadata, then compare-and-swap the current
   pointer against the source version observed when this worker started. If a
   newer worker has already promoted a version, the stale worker must not
   overwrite it; it records a superseded outcome and may enqueue a refresh
   against the newer cursor.
4. Preserve the prior pointer on failure. Emit refresh outcome and measured
   latency; on breach follow §1.2.

The canonical PM-stack report-record store is the source of truth and storage
adapter for these versions (`report-records.ts` and `report-records-c7.md`);
do not introduce a parallel page/report database. Period pages use the
existing C7 record identity and append/snapshot/correction operations. Quota,
status, incident, and index records are not falsely encoded as C7 periods:
their implementation extends the same canonical append-only store/backend
with a typed page-record operation and a stable record ID derived from
`(page_id, template_version, source_cursor)`. The extension must retain the
existing idempotency/conflict guarantees and use its atomic insert-if-absent
boundary; it must not assume the local JSONL backend provides cross-process
locking. Promotion of a current-page pointer is conditional on its expected
prior version using the durable backend's concurrency primitive. A refresh
never overwrites human-authored prose: generated facts
occupy typed sections, and editorial changes are separate attributed
versions/events. This composes with C7's append/version/conflict rules.

### 1.4 Reuse existing freshness and decision machinery

C6 adds the page-class freshness view and budgets, not a second event journal,
quota gatherer, or decision system:

- `FreshnessStore` (`freshness.ts`) remains the append-only source-event
  journal, with its cursor, deduplication, and bounded retention. Its notices
  invalidate affected page projections and provide source cursors; they are
  not themselves proof that every report source had a successful observation.
  The freshness delivery surface (`freshness-delivery.ts`) remains the live /
  stale / silent source-health feed and its five-minute / 24-hour windows.
  C6 derives page freshness from each page's successful source-observation
  timestamps and its class budget, and uses the existing exception path for
  breach/recovery visibility.
- Quota pages consume the existing quota snapshot measurements and
  `quota-refresh.ts` lazy refresh trigger (five-minute measurement age by
  default); they do not fetch providers from the page handler or replace the
  gatherer. The page-class budget can be configured separately, but its
  freshness display must also surface the underlying measurement state and
  refresh-in-flight outcome.
- Questions embed the existing decision-set/question/resolution contract
  (`decisions.ts`) and route effectful answers through its audited task/action
  path. C6 supplies page/version context and confirmation; it does not mint a
  competing question ledger.
- Incident detail reads the existing alerts ledger's incident and event
  records. Its event detail includes the C3 `healthHook` extension where
  present; the audit projection alone is insufficient evidence.

## 2. Mobile-first rendering

- At 320 CSS px, the primary reading order is title and freshness, key facts,
  next action/questions, narrative, then expanded evidence and comments.
- Body text is at least 16 CSS px; primary actions have a 44×44 CSS px
  target, labels remain visible, and forms do not depend on hover or precision
  gestures. Safe-area insets and keyboard focus are handled on mobile.
- Graphs have concise text summaries, labeled axes, and a tabular/data
  alternative. Dense data tables may scroll within their own region; page
  navigation and body do not scroll horizontally.
- Desktop may place facts and questions side-by-side and expose more columns;
  it must preserve the same content, action ordering, and safety prompts.
- Automated acceptance checks render quota, incident, index and quote
  resolution views at 320px, 390px and desktop widths, including keyboard
  navigation and zoom/reflow.

## 3. Questions module and effectful actions

### 3.1 Embed and answer

Embed the existing question set beside the fact or status it concerns. Each
item carries stable set/question IDs, page ID and content version, context
link, answer mode, options/recommendation where present, open/closed state,
and expiry or supersession relation. Preserve the existing single/multi and
free-text semantics, version history and first-resolution-wins behavior.
Submission records the answer before dispatch. Duplicate submissions with
the same idempotency key return the recorded result; conflicting later
answers do not replay an effect and are shown as already resolved.

Action lifecycle: `proposed → awaiting_confirmation → queued → running →
applied | rejected | failed | cancelled`. An answer may create a proposed
action; an effect requires an explicit confirmation event tied to the action
ID, exact action summary, target, and page version. Record actor, time,
payload digest, authorization, queue/task ID, and outcome. The confirmation
screen states the provider/pool and consequence. The worker uses the
pool-steward reset actuator and returns the outcome into the existing
decision/action result path. No page handler executes infrastructure work.

For OpenAI, Anthropic and Fable quota thresholds, C6 may offer a reset action
when the matching quota reaches 100%; it remains a human-confirmed queueable
action. The exact provider's reset eligibility and constraints come from the
pool steward; a page cannot infer a reset is safe merely from a displayed
percentage. If confirmation is stale because the page/action version
changed, require fresh review. Audit failure means do not dispatch.

### 3.2 Error question sets and notification

On a page error, offer the existing question-set choices, where applicable:
“Should I try to fix it?”, “Hotpatch the binary the service runs?”, “Trigger a
NixOS reset?”, and “Wait for downtime, then reset?”, plus free feedback and
multi-selects. These choices are proposals, not hidden commands: potentially
disruptive work is summarized and confirmed before queueing. An accepted
answer creates a Muse task linked to the page, question resolution and source
error. The task posts progress/outcome back as a page event and sends the
configured notification (push, category chat, or main chat for high urgency).
Retries are idempotent, failure is visible, and unanswered questions remain
open rather than being treated as consent.

### 3.3 Explicit automation exclusion

Issue #15002 exclusively tracks the candidate unattended rule: all other
queues down plus Claude at 100% triggers an immediate cloud reset. The
proposal is not enabled by C6. Until an FDR under #9180 explicitly approves
or amends it, implementation must not add an unattended trigger, scheduled
fallback, or answer-to-reset shortcut. Even after an FDR, that behavior is a
separate implementation slice and review; this design's reset contract is
human-confirmed only. Rejection or amendment is recorded on #15002 and the
applicable FDR.

## 4. Page comments, annotations, and version-pinned quotes

### 4.1 Targets and threads

Support threaded replies on (a) a whole page, (b) a stable section ID, and
(c) a highlighted text range. Each comment stores author/agent identity,
created/edited timestamps, body, parent/thread ID, target kind, exact page
content hash, section ID where relevant, and the original selected text plus
surrounding context. Edits are attributed versions; deletion is a tombstone
so replies and audit history remain coherent. Comments are not embedded into
the page hash; annotations link to the version they discuss.

A selection creates a quote URL with opaque report ID, content hash, section
ID, normalized start/end offsets, and a signed/validated anchor token. The
URL must not expose private content or authorize writes. Pasting it renders a
quote card containing the exact original excerpt, surrounding context, and
section title. A quote card always uses archived original content, never the
current page text substituted in place.

### 4.2 Stable section IDs and addressing

Every section has a permanent ID independent of title or order; rename and
reorder preserve the ID. A removed section remains resolvable in historical
versions and redirects the current-page view to its nearest surviving
context with a “section moved/removed” notice. “Most recent page” links use
the stable section ID.

Support these addressing forms as resolvers over the versioned document:

- **Section index:** `0` is preamble before the first major heading; `1..n`
  are major sections in document order; `-1` is the trailing material after
  the last item; `-2` is the preceding trailing item, and so on.
- **Slug:** human-readable alias such as `objective`; aliases may change and
  resolve through the stable section ID.
- **Line/item index:** a zero-based line/item address within the pinned
  version, not a durable identity. Store the resolved section and offsets at
  creation so later line movement cannot retarget the annotation.

When two addressing forms disagree, the stored stable section ID wins; the
other form is diagnostic metadata. Indices are interpreted only against the
specified content version.

### 4.3 Resolution modes and fuzzy presentation

Opening a quote offers three explicit modes: (1) exact original version; (2)
newest version containing the exact selected string in the original section;
(3) newest version with the best fuzzy match in that section. Default is the
exact original version; users opt into re-resolution. Exact search prefers
the same section and nearest relative position. Fuzzy matching uses a 90%
similarity threshold (normalized token/character similarity), with candidate
score weighted by relative position and unchanged context above and below.
The 90% default is the midpoint of the proposed 80–95% range: it recovers
small typo edits but rejects most unrelated wording; do not lower it silently.
Fuzzy matching is deterministic: normalize with Unicode NFKC, case-folding,
whitespace collapse and punctuation-insensitive tokenization; candidate
similarity is the maximum of normalized token similarity and character edit
similarity. A candidate must score at least 90% and exceed the runner-up by 5
percentage points; ties fail closed. Rank passing candidates by similarity,
then unchanged surrounding-context score, then relative distance from the
original offsets, then earliest offset as a stable tie-break. Search only the
original stable section. No match is safer than a misleading one.

The UI labels the result **Exact**, **Exact text in newer version**, or
**Fuzzy match (~N%)**. A fuzzy result never masquerades as exact. Within its
highlight, words unchanged from the original retain normal emphasis; words
that differ are de-emphasized, and newly inserted/replacement words are
bolded/highlighted. Show the original quote adjacent to the matched text,
with a diff and the selected version hash. A match that can flip meaning is
still visibly fuzzy and requires the reader to compare; the system does not
assert semantic equivalence.

### 4.4 Concurrency choice

V1 does not require CRDTs or operational transforms. Page versions are
centrally managed immutable snapshots; comment/reply writes append to a
separate event store with optimistic version checks. A text selection is
anchored to one immutable version, so concurrent edits cannot move its
meaning. Concurrent comment edits use compare-and-swap; conflicts retain
both proposed edits for resolution. Operational transforms may be considered
only if simultaneous collaborative editing of page prose becomes a concrete
requirement. CRDT merge semantics would not solve historical quote identity,
and add complexity without a current multi-writer editing requirement.

## 5. Templates and generated variables

Pages are Markdown-compatible records with a typed template definition.
Every variable has a stable name and invisible HTML-comment start/end
markers carrying the name and rendered value; rendered text remains readable
in-place while editing the source retains the template scaffolding. Parsing
requires exactly one balanced start/end pair per active variable; malformed,
duplicate, nested, or unknown executable directives are inert and reported as
validation errors, never partially applied. The variable value, version and
edit history are DB-backed so a query-shaped value can be answered
historically. Ordinary edits persist attributed content but do not refresh
query values. An explicit save-refresh evaluates only `replace on each save`
variables and applies append/prepend-once variables only while armed. V1
supports these per-variable refresh
modes: render once; replace on each save; append/prepend once; append/prepend
every time, with an optional separator. One-shot directives move from active
to spent/idle state in the document when fired; removing the idle marker
re-arms it. Explicit full refreshes (for example period rollover) may re-arm
only the variables named by that refresh request.

Template application creates a centrally managed page, not a fork. It can be
applied as-is or with a new title and supplied fields. Edit history records
actor/agent, timestamp and attributed diff; default imperative summary names
the primary sections and appended content. Edits may later be renamed and a
page may be marked as a template with placeholders. In-document agent hints
are inert data until a trusted parser validates them and queues an auditable
task; a marker cannot execute code or bypass action confirmation.

Use shared variable shapes across page classes. Initial catalogue includes
`timestamp`, issue counts and `projects.<id>.open_issues`, quota open/summary
and projected values, walls/renewals, under-provisioning flags, areas-to-watch
with reasons, merged PRs, accepted designs, no-rework/negative-feedback
items, and C4 forecast records. Query-shaped period values use C7's rollup
store and explicit period IDs; unsupported or insufficient-history values
render an honest unavailable state, not a guessed answer. C6 defines the
mechanics, not a parallel query engine.

## 6. Incident and quota pages

### 6.1 Incident page template

An incident record has stable ID, title, affected service/pool, severity and
impact interval, status, owner/task links, source events, page versions and
timestamps. Render sections in this order:

1. Current state, affected users/services, impact window and last updated.
2. Short story: start, detection, response, resolution/end.
3. Progress timeline and fix attempts (“trials and tribulations”), including
   failed attempts and why they failed.
4. Findings/evidence and graphs with source/time-range labels.
5. Related previous incidents and recurrence-prevention issues.
6. Final root cause (explicitly provisional until verified, if applicable).
7. Remediation by short-, medium- and long-term horizon, each linked to an
   issue/owner/status; unknown owner or due date is explicit.
8. Threaded comments and follow-up questions.

Updates append attributed facts and versioned prose; they do not erase failed
trials or rewrite the incident history. Incident evidence is assembled from
the canonical C3 `healthHook` records and the alerts ledger's opened/closed
event details, not from the thin audit stream alone. Join
`healthHook.incidentId` and `healthHook.recoveryOf` to incident identity;
preserve alerts-ledger event IDs separately because they are not incident
IDs. The Claude authentication outage is the candidate first real incident,
subject to source evidence and privacy review.

### 6.2 Page threshold

Create a dedicated incident page when any of these holds: (a) a core service
or routing tier is unavailable for at least 30 minutes; (b) a multi-provider
or multi-queue wall materially blocks work; (c) an outage has user-visible
impact and requires coordinated remediation; or (d) a failure recurs and
needs a durable root-cause/prevention record. A single pool reaching 100%, a
routine reset, a short isolated error, or an automatically recovered blip
does not by itself earn a page: keep it in the event/alert/period record and
link it if later correlated. Escalate to an incident page if the threshold
is met later; do not backfill unsupported narrative.

### 6.3 Quota/period pages

The quota page presents `as_of`, pool and window, used/remaining quota,
renewal, C4 projected wall and wall-until with basis/freshness, and the
questions module beside the relevant facts. A reset option appears only when
the provider/pool steward reports it eligible; it is an explicit
human-confirmed action. A quota wall earns a special/incident page only under
§6.2; ordinary walls remain quota and period events. C7 owns period windows
and timeline composition; C4 owns forecast values and scoring.

## 7. Reports index and integration

The reports index is the searchable catalogue, analogous to the question-set
list, not a replacement for each report's page. Preserve `/api/reports.json`
with its existing `{records, conflicts}` response and current conflict and
late-update routes; new search/index behavior is additive and versioned (for
example `/api/v1/reports`). Each entry exposes stable report ID, `page_id`,
canonical `record_id`, content hash, C7 `period_id` where applicable, title,
kind, project/domain, affected interval, version/update cursor, created/updated
time, status, tags, freshness where live, and links to source issues,
incidents, pages and C7 periods. Search covers title, summary, tags, linked
IDs and indexed report text; filters include kind, project, date range,
status and tags. Pagination and sort are deterministic; indexing is
eventually consistent within the index freshness budget. Empty/error states
preserve filters and explain whether the index is stale or unavailable.

The index emits stable metadata links and canonical page URLs for C7's
quarter→month→week→day timeline and C8's Atlas ingestion. C6 owns report page
identity, rendering and catalogue contract; C7 owns rollup/timeline
composition; C8 owns embedding, cross-link expansion and query-planner
boosts. Neither consumes a private duplicate of C6's index database.

## 8. Implementation slices and acceptance

Three follow-on issues are filed from #9574:

1. **Quota page + questions module:** mobile-live quota page, freshness/blob
   contract, embedded existing questions machinery, one eligible
   human-confirmable reset action with recorded outcome; unattended rule
   excluded by #15002.
2. **Incident page:** render one evidence-backed real incident (Claude auth
   outage candidate) using §6.1; comments, progress, findings, root cause
   and remediation links work on mobile.
3. **Reports index:** searchable index with stable report metadata and links
   that feed C7/C8 contracts.

Design verification: schema and transition contracts are explicit; each
page class has a freshness budget and stale exception path; examples preserve
mobile-first operation; quote resolution supports all three modes and
version-pinned identity; the reset exclusion is explicit. Implementation
acceptance is demonstrated by the three slices, not claimed by this design.

## Sources and boundaries

- #9574 and its 2026-11-13 comments: source requirements, including
  comments/anchors/template mechanics.
- #15002 and #9180: unattended-reset FDR gate and recording convention.
- [`note.md`](note.md): program component/collision map, C6 ownership,
  existing Fleet Reports Space and question machinery.
- [`c4-forecasting-design.md`](c4-forecasting-design.md): C4 forecast
  records consumed by quota/period pages.
- [`c7-rollups-design.md`](c7-rollups-design.md): period and timeline
  ownership; C6 supplies page mechanics and report identity.
- `pm-stack/book/reference/question-sets.md`: existing question surfaces,
  answer/write-back semantics and action audit trail.
- `pm-stack/aggregator/src/pages.ts` and
  `pm-stack/aggregator/src/server.ts`: existing `/reports` and
  `/api/reports.json` surface; C6 extends rather than replaces it.
- `planning/DOCUMENTATION_ARCHITECTURE.md`: dated component design record
  placement under `doc/`.
