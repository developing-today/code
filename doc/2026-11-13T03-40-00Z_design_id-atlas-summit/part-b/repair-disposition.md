# Part B bounded repair disposition

This note records exact repairs from `../review/part-b-judge.md` and the subsequent bounded integration-coverage messages. The judge file is unchanged. No new research/reviewer loop was started.

## Fixed

1. **Statement attribution:** manifest B08 now explicitly covers S1.26–S1.27, including sync toward 1:1 sameness within reason. Added B31 for S1.1–S1.11 peer-class behavior and B32 for S1.12–S1.20 mandatory signed validation. Original B01–B30 reconstruction IDs/provenance remain intact; total is 32. S1.114–S1.120 stays crosscut under #9106/#9110.
2. **Budget isolation:** design now specifies multidimensional caps at per-query, authenticated requester/origin, supplying-peer, locally authorized tenant, and node-aggregate scopes. Atomic reservation charges all scopes; disjoint allocations, measured-use accounting, unused-only refunds, finite expiry/cancel, and idempotent retry reservations are stated. Vector dimensions are hops/peer slots/work/bytes; elapsed time is monotonic, not refundable/resettable. Requester/node local caps are enforceable; no network-wide Byzantine spending guarantee is claimed.
3. **Slice execution:** manifest defines one ordered PR unit per component, exact shared module/test path and matching integration target. Crosscuts #9110/#9111 are not counted as Part B slices. B26 is research-only in the docs PR, not an implementation blocker. Reconstructed 30-item set is still not represented as the missing original list.
4. **Expiry and identity:** result-cache eviction while origin query is valid cannot mint fresh reservations; expired signed origin queries never rerun. Origin request ID binds origin key, nonce, query digest, scope, issue/expiry and limits; changed fields with reused ID fail `BindingConflict`, while independent same-query requests differ by nonce.
5. **Reputation attribution:** invalid signatures do not authenticate their claimed signer. Rating penalties require evidence linking exact bytes to an authenticated supplying peer; otherwise record unattributed rejection only. Unilateral allegations have no direct score effect.
6. **Model:** `federation_model.py` checks scoped vector reservations, same-reservation retry idempotency (including changed retry request), mismatched query binding, nonce-distinct request IDs, requester/node cap cases, cached-result revocation/eviction/expiry, measured-use accounting, unused-only refund/expiry, and 2,000 seeded allocation-tree traces. It assumes authenticated scope IDs and does not prove crypto, fairness or Byzantine/global enforcement.

7. **V2 receipt seam:** B19 is the shared V2-crucial `accepted | retained_until | verified_present` schema, with V2 signer/gate dependencies only; #9115 maps existence to `verified_present`. V3 fanout consumes the schema later. `stored_unverified` is inventory-only, never accepted content.

## Remaining conservative defaults / seams

- Exact original 30 candidate titles/boundaries remain unrecovered; `../inventory-live/candidate-provenance.md` records the bounded search and distinct reconstruction. B31/B32 are additional foundational coverage, not asserted original candidates.
- Reputation numeric thresholds and claim limits are starting local defaults, not empirical or global weights; a deployment may tune them under #9111 without overriding signature validation.
- Tenant identity comes from local authorization context; final tenant product policy remains #9083. Content transfer/retention ownership remains unresolved with #9084.
- Historical signing time after key compromise may remain indeterminate without independent epoch/status evidence. A signed wall-clock claim cannot solve attacker backdating.
- The model is an accounting abstraction, not an implementation or proof of cryptography, identity binding, fairness, or network-wide budget enforcement.

## Narrow verification

Run:

```sh
python3 -I -m json.tool doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/implementation-slices.json >/dev/null
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/federation_model.py
```
