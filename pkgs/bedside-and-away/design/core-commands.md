# Core command and history design

Implements DM-03 and interfaces DM-01/02/08. Canonical command intent and actor bind an immutable event ID. CAS binds expected store version; event insertion and snapshot update share the existing transaction. INSERT OR IGNORE plus actor/payload predicates make concurrent command retries safe without applying changed content. Old legacy receipts without canonical intent are deliberately not replayable automatically: inspect the current state rather than guess.

Dedicated POST routes:

- `stores/:id/steering`: `{expectedVersion,command}` using versioned steering transition.
- `stores/:id/variants`: `{expectedVersion,commandId,ids,options}` using server-allocated UUIDs.
- `stores/:id/policy`: `{expectedVersion,commandId,access}` administrator-only and validates owner/caller do not lose read/write/admin access. GET returns `{access,version,members}` to administrators.

Dedicated mutations return only version, optional requestId and duplicate. They never return full event snapshots. Clients refetch. Generic events retain compatible event payloads only for unrestricted snapshots and strip internal intent/requestResult. GET state projects current permission and withholds historical snapshots when scoped; search uses the same projection. `snapshotWritable` communicates fail-closed editing. Generic events cannot change any steering/receipt/audit fields or family metadata. Scoped snapshots and scoped steering mutations are read-only pending fully granular command coverage; policy repair remains possible.

Alternatives rejected: merging filtered snapshots back into authoritative state (ambiguous field removal and silent overwrite); trusting browser-generated before state or actor; replaying by after-state equality only; returning historical snapshots to someone who can no longer read current records.

All changes are additive, no schema migration, no permanent deletion endpoint, existing reversible removal/history retained. Existing API, external Worker gateway and gateway authentication suites remain green. New negative tests cover actor/intent collisions, forged steering metadata, dedicated state transitions, graph changes, scoped search/history/retry denial, policy lockout, and lack of internal command metadata in history.

Known gaps: event snapshots still grow with state; no streaming/retention migration here; legacy retry conflict requires refresh; granular scoped edits and request-specific ACL need further implementation. This is not a claim that all design requirements ship.
