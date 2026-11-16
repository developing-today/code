# Fleet quota admission: protect capacity before launching work

**Status:** implementation proposal, not deployed policy.
**Tracks:** [#2732](https://github.com/developing-today/code/issues/2732). Keep that issue open through deployed acceptance.

## Why now

On November 14, Fleet recovered from ClinePass exhaustion by routing to OpenCode Go, and then a later wave stalled on Go. In the [17:45 UTC sample and 17:47 census](https://github.com/developing-today/code/issues/10085#issuecomment-6881707592), all three Go accounts had exhausted their **weekly** windows, including an account with only 54% monthly usage. That verifies exhaustion at that time, not why earlier attempts failed to find the then-working sibling.

[#9950](https://github.com/developing-today/code/issues/9950) supplies a reactive breaker and half-open probes. This proposal complements it: use quota evidence **before** starting a session, preserve work if there is no eligible route, and check again when the router selects the actual credential. It does not broaden 429 classification, change model/plan entitlements, or add provider accounts.

## Contract

Publish a versioned, read-only admission projection from existing quota inputs and steward stops. Keep low-level attempt health in the router. Use the same policy and fixture corpus across the launcher and resolver rather than inventing policies in each lane prompt.

- **Account identity:** key by provider, stable non-secret billing account/workspace alias, quota bucket and window kind. Record the period or reset boundary with the evidence. Credentials that share an allowance map to the same bucket; independent accounts do not. A route alias is a candidate list, not a billing bucket. An unlabeled aggregate is not proof about an individual sibling; [#8260](https://github.com/developing-today/code/issues/8260) is the regression case.
- **Measurements:** carry source, observation ID/time, numeric value and limit or percent, reset boundary, and exact account mapping. Existing `pm-stack/aggregator/src/quota-current.ts` preserves provider and window freshness separately; do not let a freshly generated file renew an older window. Set the admission freshness contract explicitly: for the prototype use the lower of the provider's freshness limit and #2732's 60-minute ceiling. Future, malformed, stale or unmapped values are unknown, not zero. No synchronous provider fetch on the launch/request hot path.
- **Steward stops:** carry exact scope, effective time, reason code, evidence reference and release condition. Publish atomically with a schema version and generation ID. An absent or unreadable snapshot may not silently clear a previously known stop. Only release explicit stops according to their declared condition.
- **All windows bind:** a candidate is eligible only when none of its applicable windows blocks it. A healthy monthly value cannot override an exhausted weekly window. No new spend or implicit fallback to an unapproved plan or paid route.

| Decision    | Evidence                                                                                           | Launch behavior                                                                                 |
| ----------- | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `stopped`   | Active steward or administrative stop                                                              | Exclude within its exact scope; preserve the release condition                                  |
| `walled`    | Fresh known 100%, confirmed quota-exhausted response, or sustained ≥99.5%                          | Exclude from ordinary new requests until verified recovery                                      |
| `pace_hold` | Configured pre-wall reserve threshold reached, or same-month burn predicts exhaustion before reset | Exclude for new seating on scoped policies; review on newer valid evidence                      |
| `unknown`   | Missing, stale, ambiguous or contradictory observation                                             | Prefer an eligible configured candidate; coalesce refresh; never treat as proof of availability |
| `eligible`  | Sufficient fresh evidence and no other gate                                                        | Candidate remains in normal ranking; router still applies live health                           |

### Separate a wall from a pace hold

[#10128](https://github.com/developing-today/code/issues/10128) records Drew's ruling that sustained ≥99.5% is walled. Proposed implementation: two distinct provider observations in the same period, separated by at least one configured collection interval, with the newest fresh. Duplicate reads of one cached observation are not sustained. The observed Command Code re-enable contract also requires a fresh below-threshold post-reset read and a live completion. Keep that provider-specific rule; for other accounts use the existing controlled probe rules after refresh, not normal traffic as a probe. Never revive an administratively disabled route automatically.

The 97% monthly and >1 percentage point/hour values in #2732 are **reserve-policy proposals**, not proof of hard exhaustion. Make them configurable per bucket and shadow them before applying enforcement. Rate calculations reuse measured same-window history; never compute across resets, denominator changes, or downward corrections. Enough margin to reach reset is not the same as a wall. Insufficient pace samples are a separate unknown, not an invented zero burn.

A known wall persists for its valid period even if the next read is unavailable. Passing the reset moves the observation to unknown until verified new evidence or the existing controlled recovery path, not directly to healthy. Never reclassify a bare transient 429 as monthly exhaustion.

## Launch, resume, and deferral

1. **Claimant and dispatcher:** preflight the configured route, evaluate candidate accounts, and record a revision plus eligible set. If no eligible route remains, defer the existing task or session with a reason and next eligible check (refresh or known reset); no retry-budget increment or throwaway session. Keep the established queue ownership and lease contract. Shadow mode records what would happen but performs no queue mutation.
2. **Resolver:** immediately before every model request, intersect configured seats, current admission decision, and live breaker state. Re-check the exact candidate account when credentials are selected or rotated; a pool intending to use a healthy account may not silently fall back to a walled sibling. Admission and breaker state cannot clear each other. For newly quota-managed paths, an empty eligible set defers rather than restoring known-walled candidates. Do not change unrelated route behavior in the first slice.
3. **Resume:** preserve existing bindings and worktrees. Re-evaluate before the next request in an already-bound run. A new observation alone does not kill an in-flight request; its next provider call is subject to the guard. Manual operator entry points must use the same preflight, not a second registry.
4. **Proof:** record task/session reference, stable account alias, reason, source generation, measured window, whether fallback was selected, and recheck horizon. Store neither credentials nor raw provider responses. Count deferred and rerouted decisions separately. Preflight is not a capacity reservation; use the latest snapshot at the actual request, and make rate/lease refinements a later slice.

## Implementation packets and acceptance

| Packet                     | Deliverable                                                                                                   | Checks                                                                                                                                                                                            |
| -------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A. Projection and fixtures | Additive schema and pure builder beside the existing aggregator quota inputs; no consumer or behavior changes | Shared-bucket aliases, distinct siblings, stale/future windows, duplicate evidence, 97/99.5/100 boundaries, short window defeating monthly headroom, same-window pace, and manual stop precedence |
| B. Shadow preflight        | Launcher/claimant uses the projection and emits would-defer reasons, including launch and resume              | Compare expected and actual selected account; no task or session writes from shadow mode; one issue/task per existing claimant lifecycle                                                          |
| C. Resolver canary         | Recheck selected credentials and activate pre-seat deferral on one approved route after proof                 | Simulated known wall generates zero calls to that account, leaves the task resumable, and can use a proven eligible sibling; controlled recovery does not unbench other accounts                  |
| D. Rollout and proof       | Extend to other launchers and consume decision counts in steward reporting                                    | Same guard for scheduled/manual/resume; fresh valid alternative advances; all-blocked case parks work; real threshold refusal seen without inducing exhaustion                                    |

A should not wait for operator choices about applying the reserve policy in C. Do not close #2732 on a spec, a merged unused registry, a successful shadow run, or a provider reset alone. Close after enforcing on the approved scope and verifying the refusal/re-route in the launch and attempt logs.
