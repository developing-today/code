# Activity river: unified audit timeline

Status: proposed design for [#1261](https://github.com/developing-today/code/issues/1261), epic [#1184](https://github.com/developing-today/code/issues/1184). Defaults below are implementation guidance, not claims that identity, durability, or retention policy has already been approved. No implementation is included in this note.

Implementation follow-ups, native children of #1184: [#3321](https://github.com/developing-today/code/issues/3321) source coverage and gaps; [#3322](https://github.com/developing-today/code/issues/3322) integrity, identity and durable recovery; [#3323](https://github.com/developing-today/code/issues/3323) item context and coverage-aware filtering. Coordinate the existing stream in #1739 rather than implementing it twice.

## Context and boundaries

Today the aggregator's dashboard actions and comments have separate local audit stores (`pm-stack/aggregator/src/actions.ts:47-59`, `pm-stack/aggregator/src/comments.ts:114-120`). Action authorship is currently self-asserted, not SSO-verified. The river is an evidence-linked projection across systems, not a replacement for their authoritative records, their access controls, or the existing mediated action path. [#1739](https://github.com/developing-today/code/issues/1739) already proposes a cross-stack event stream, emitters and `/events`; reuse and reconcile that work rather than build a parallel stream. Its best-effort writes and example bounded retention do not by themselves meet the integrity and retention contract below. [#2072](https://github.com/developing-today/code/issues/2072) is related GitHub/fleet ingestion; coordinate, do not duplicate its infrastructure.

## Event contract and sources

Every persisted event uses this versioned envelope. Unknown optional fields are ignored; unknown schema versions are quarantined and reported, not silently dropped. Required fields can be explicitly unknown where a source cannot provide them; do not synthesize certainty.

```text
schemaVersion: 1
eventId: stable opaque ID
source: { kind, instance, nativeEventId, url? }
actor: { type: human | agent | service | unknown, id?, display, attribution }
action: stable namespaced verb (e.g. github.issue.closed)
object: { type, canonicalRef?, sourceRef }   # sourceRef always present
occurredAt: RFC3339 timestamp | null          # source time, not ingest time
observedAt: RFC3339 timestamp                # collector time
sourceLink: URL | null                       # link to source evidence if available
projectRef?, epicRef?, correlationId?, causationId?, summary?, metadata?
```

`attribution` is `verified`, `source-asserted`, `legacy-unverified`, or `unknown`; verified requires the identity contract below. Actor is the originator; an applying agent/service is a separate event or explicitly recorded in metadata, never silently substituted. Preserve source-native ID and timestamp; deduplicate on `(source.kind, source.instance, nativeEventId)` with a unique constraint, and use a stable source-specific idempotency key when no native event ID exists. If no stable key is available, flag possible duplicates rather than collapse distinct actions. `canonicalRef` is resolved from the existing item mapping; unresolved events remain visible with their source ref, not attached to a guessed item. Retain raw source evidence only as permitted by source access and privacy policy. Summaries are escaped, length-limited and not authoritative.

V1 source coverage is **GitHub** issue/PR changes and comments; **Kaneo** task activity; **sessions** lifecycle and meaningful user/agent actions; **orchestrator** runs, decisions and application outcomes; **dashboard** write requests and outcomes, including comments; and **watchdog** detection, remediation and recovery. Each adapter declares instance, supported kinds, start of coverage, last successful cursor/checkpoint, freshness and known limitations. Poll/reconcile or webhook ingestion may coexist; retries replay idempotently. Dashboard request and application outcome are distinct causally linked records. A system incapable of emitting a category is shown as unsupported, not presented as complete.

## Identity and durability interfaces

Identity [#1253](https://github.com/developing-today/code/issues/1253) supplies an authenticated stable principal ID, principal type, display snapshot, verification basis and agent/on-behalf-of linkage. The river consumes that contract rather than inventing SSO or trusting a client-supplied display name. Existing source identities are `source-asserted` unless the source proves its identity; pre-SSO dashboard actions are `legacy-unverified`, even if a display name matches a later account. Unknown stays unknown. A future verified mapping is a separate attributed annotation, preserving the original attribution and evidence. Source/system principals and service-agent delegation must be distinguishable from humans. Access to events and source links must not grant access beyond the original source; apply authorization before counts, search, pagination and rendering.

Durability [#1258](https://github.com/developing-today/code/issues/1258) supplies an append transaction, indexed reads, backup/restore, archive lifecycle and observable RPO/RTO. The river requires atomic event-plus-checkpoint advancement or an equivalent transactional outbox; no successful source acknowledgement before durable acceptance. On failure retry with backoff, retain source cursor and expose lag/gap; never advance past a dropped event. A source that cannot replay must expose potential permanent loss. Restore drills must verify event IDs, checkpoint consistency, gaps and query results, not merely that a database starts. These are interface requirements, not a claim that a backend or RPO has been chosen; design work on either sibling can proceed independently with a versioned adapter.

## Views, search and ordering

The global `/events` river (coordinate with #1739) provides project, epic, actor and event-kind filters, source and time range, and text search over authorized indexed summaries/references. The item page shows the same API and rendering scoped to its canonical item, with a link to the global view preserving relevant filters. Explicitly disclose unresolved associations and incomplete source coverage; don't include unrelated children merely because they share an epic. Epic/global aggregation can include descendants only when the filter says so. Filters have shareable URLs, accessible controls and mobile layout. Stable descending pagination uses `(occurredAt or observedAt, eventId)` with the fallback visibly labelled; time is UTC with local display. Cursor pagination must not duplicate or omit records when new events arrive; include stable snapshot boundary or equivalent. Show source link, actor attribution, time and ingestion delay on each row; source time may be late or unavailable. An empty result is not proof of no activity when coverage is partial.

## Retention, storage and growth

Use the durability service, not an unbacked local file as the system of record. Keep append-only audit envelopes and checkpoints in durable indexed storage; archive older partitions with their manifest, integrity verification and discoverable coverage. Keep sensitive payloads minimal and permit separately governed redaction/tombstone of content without silently rewriting audit history. Working sizing model: `events/day × mean persisted bytes × retention days`, plus indexes, replication and backups; measure source rates, peak bursts and actual byte size before setting capacity. A proposed initial operational hot window is 90 days, with older records archived and searchable or explicitly marked offline; this is **not** a deletion authorization or settled compliance policy. Never apply #1739's illustrative 30-day/5,000-entry cap to authoritative audit history. Retention duration, archive accessibility, legal deletion and cost limits need Drew's decision and #1258's policy; until resolved do not automatically prune authoritative events. Record coverage boundaries when importing or archiving, and test restoration across them.

## Integrity, gaps and annotations

All accepted envelopes, source evidence references, checkpoint changes, gap markers, annotations and corrections are append-only. No user edits or deletes original event facts through the river. Authorized maintainers may add attributed annotations or corrections that refer to the original, with timestamp and reason; readers see both, and access changes are audited. Redaction is a privileged, policy-governed content exception, itself recorded without exposing the removed content. The UI distinguishes original, correction and derived information.

Per source/instance, store durable coverage intervals and explicit gap records with start/end, detection time, reason, affected kinds, last good checkpoint and status (`open`, `recovered`, `permanent`, or `unknown`). A heartbeat can detect a stale collector but cannot prove all source events were delivered. Render a prominent partial-coverage banner on global and affected item views, and a timeline marker for the interval; unknown affected items must not hide the global warning. On recovery replay, deduplicate, reconcile source evidence, append a recovery record and close only the proven interval. A permanent or unprovable gap remains labelled even after service health returns. Source outage, stale cursor, failed writes, inaccessible source, unsupported category and backfill boundary are distinct reasons. Test outage, replay, duplicate delivery, ordering, restore and access filtering.

## Decisions needed from Drew

1. **Retention and deletion:** How long must audit envelopes and source payloads remain available, what archive retrieval latency is acceptable, and which legal/privacy deletion rules apply? Is the proposed 90-day hot window acceptable pending the durability design?
2. **Access and annotation:** Which roles may read cross-project history, annotate/correct, and authorize redaction? Must access mirror source permissions exactly when the source cannot expose them?
3. **Coverage and priority:** Are all six v1 source families mandatory before launch, and what backfill horizon and acceptable unverified historical attribution should be targeted?
4. **Durability/cost:** What loss/recovery targets and storage budget should #1258 apply to this audit stream, and should non-replayable sources block launch until protected?

Until answered, expose limitations and avoid deletion or claims of complete coverage; these questions are not permission to assert guessed policy as final.
