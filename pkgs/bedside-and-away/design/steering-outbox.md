# Inactive durable steering transport

This component is executable but deliberately not connected to any live agent or deployment. It builds on the server steering lifecycle, rather than granting a worker direct write authority.

## Files and activation gates

- `external/0003_steering_outbox.sql`: durable outbox and one-result-per-job callback tables. Apply as an additive migration before activation.
- `worker/steering-outbox.mjs`: `createSteeringOutbox({db, config, authorize, clock?})`, exposing enqueue, dispatch, receive and internal get.
- `tests/steering-outbox.mjs`: real SQLite transactions with synthetic injected fetch. No actual external calls.

Trusted server config must supply enabled=true, operatorApproved=true, exact HTTPS endpoint in allowedDestinations, recipient, and a minimum 32-character HMAC secret. These values must never come from a browser request. No default network implementation is supplied: dispatch requires an explicit fetch adapter. Redirects are blocked, delivery deadline is 15 seconds, and job ID is the remote idempotency key. The operator must verify that the recipient honors that key before activation.

`authorize({storeId,requestId,actorId})` returns `{allowed,targetsFresh,storeVersion,request}` from a current store snapshot. `allowed` requires current active store editor membership and current object-level access to every target. `targetsFresh` must explicitly be true only when every current target exactly equals the saved baseline, and false on missing/stale targets. For a general request, it can be true after store authorization. Request status must still be pending/proposed, and revision and complete request body must match enqueue. Every DB write predicates on this store version and editor/admin membership, closing access changes between read and write. All ACL modifications must increment the store version. Do not expose internal get results without the same authorization/projection.

## Enqueue and delivery

The unique tuple (store,request,revision) is the durable job ID. Repeated enqueue has no effect; a changed principal/body/destination under that ID conflicts. The insert itself is atomic and guarded by current version/membership. Queueing can be a separate action after a request commits: if persistence fails, the request remains safely saved and explicit enqueue can be retried. If enqueue must be part of request creation, the host should compose the same guarded INSERT in its event transaction rather than calling this wrapper inside a transaction.

Dispatch atomically claims pending -> dispatching with a random lease. It rechecks current authority and request/target freshness immediately before claim, sends only the scoped signed request, and records sent on HTTP success. Any network or HTTP error becomes uncertain. There is no automatic retry from uncertain or dispatching. A crashed worker therefore cannot blindly send twice; reconcile with the recipient using job ID. A future operator-only reconciliation mechanism may mark confirmed nondelivery retryable. This is a deliberate availability tradeoff to avoid duplicate external jobs.

## Callback and review

The recipient posts `{payload,signature}`, HMAC-SHA256 over canonical sorted-key JSON. `signResult(payload, secret)` is the shared signing helper. Payload: schema=bedaway.archer-result, version=1, callbackId, jobId, recipient, requestRevision, issuedAt, expiresAt=issuedAt+300, patches, summary. A patch is `{kind:'item'|'category',id,changes}` and only exact queued targets and permitted steering fields are accepted. No credentials, access changes, execution commands or arbitrary payload fields become instructions. Empty patches are allowed for general requests/research-only advice.

Receive verifies HMAC, five-minute freshness, identity, target scope, current membership/object permissions, unchanged request and target baselines. In one transaction it stores the callback once and marks the job completed. Exact retry is idempotent; another result for the same job, changed payload under the same callback ID, stale revision, revoked permission or changed targets are rejected. Result reception never edits user items or applies patches. The host displays the saved result for explicit review; an authorized editor submits a normal `propose` command and later a separate `apply`. A valid signature authenticates the sender, not authority to bypass human/role checks.

## Remaining integration

No worker scheduler, endpoint, secrets, real Archer/Muse receiver, live callback route or delivery UI is wired by this module. Add rate/body limits before callback parsing; project stored proposal data with the same ACL as request history. Report-only targets use allowlisted metadata IDs (spend, inventory, activity, defaults, buying-guide). They contain no report data. Results for these scopes may include a summary but no patches; the host records manual completion or cancellation. Report visibility must be included in authorize, and scoped report writes remain disabled. Authorization races during an external network request cannot recall already sent data; callbacks recheck permissions and never apply automatically.
