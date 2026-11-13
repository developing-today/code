# Versioned requests and Archer handoff

Implements BA-097 and BA-099 core request lifecycle, plus category scope for BA-068. External execution remains an integration boundary, not a claim of delivery.

## Decisions

Use a dedicated server command endpoint instead of client-controlled snapshot edits. Reuse existing store-version compare-and-swap and immutable event persistence. Commands carry an idempotency ID and expected request revision; the store revision is a separate envelope field. The server derives actor identity, membership and target snapshots. Client identity and target baselines are never authoritative. A request targets exact item/category IDs; no targets means a general store request, never permission to mutate the whole store.

States are pending, proposed, applied, completed, cancelled. Propose validates an explicit allowlist of fields; apply is a separate command against the reviewed proposal and unchanged target snapshots. Refresh discards the proposal and takes new authorized baselines. Terminal records cannot reopen. A manual completion requires an explanation and is distinct from automatic application. Prior versions are preserved in steeringAudit and ordinary store events. Prior command receipts prevent duplicate application; replay with changed payload or actor is a conflict.

Names, descriptions, state and priority are the only item patch fields. Categories support name, description and priority. Priority is an integer 0–100 (100 highest), with stable tie ordering. IDs, access, URLs, credentials, ownership and deployment are outside patch authority. Legacy schema v1 requests can be viewed/exported but must be recreated for server review.

## Integration contract

POST /api/stores/:sid/steering receives `{expectedVersion,command}`. Commands are `{commandId,action,requestId,expectedRevision?,targets?,text?,rationale?,patches?,resolution?}`. `targets` are `{kind:'item'|'category',id}`; patches add `changes`.

1. Authenticate, require active store editor/admin and current object edit permission.
2. Run transitionSteering with server store/version/actor and canAccessTarget callback. Unauthorized/missing targets produce the same error.
3. If duplicate, return prior receipt without another write, even when caller expectedVersion is now stale.
4. Otherwise require expectedVersion equals current version; persist returned snapshot and event atomically under the existing membership/version guards.
5. Generic /events must call assertSteeringUnchanged. Generic history undo/redo cannot overwrite the server-owned steering fields.
6. Reads, audit exports and request lists must apply the same target ACL projection as ordinary item data. A request snapshot can contain sensitive descriptions; hiding the current item must not expose its old request baseline.

The UI calls global `steeringCommand(command)`. The host supplies expectedVersion, blocks repeated submits, reports HTTP conflict without retrying mutations automatically, reloads store on success and resets pending in finally. Item/general buttons remain compatible; categories can use action `steering-category`. The host may use `orderedByPriority` or equivalent stable sort in its list selector.

## External integration

makeSteeringEnvelope produces a signed, five-minute, exact-request envelope only when explicitly configured with enabled, exact HTTPS endpoint, recipient, and server-side key. It does not send network requests. verifySteeringEnvelope checks recipient, time and HMAC integrity. Browser manual export remains unsigned and grants no execution authority. Implement a durable outbox and one-time receipt table before wiring real delivery; retry policy must use the same jobId and never resend a confirmed delivery. Responses are proposal-only and must pass the normal authorization/revision/baseline checks before a human/editor applies them. Do not use an agent-supplied actor or apply bypass. Key rotation and replay receipt storage remain host integration work.

## Tests and limits

Tests cover editor enforcement, scoped targets, duplicate command collisions, expected revisions, stale targets, terminal lifecycle, general-request isolation, direct snapshot edit rejection, stable sorting, disabled configuration and signed envelope tampering/expiry. Browser UI is not E2E verified by these tests. Persistence race and object ACL integration tests belong to the API integration owner. Audit/receipt arrays currently inherit the 1.5 MB snapshot ceiling; before large-scale use move immutable records into dedicated tables with retention/export. No real Archer webhook, queue worker, delivery receipt, or autonomous application is activated.
