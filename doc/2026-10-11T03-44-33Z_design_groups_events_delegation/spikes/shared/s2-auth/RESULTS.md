# S2 results: auth mapping, strong remove, friendship, delegation

## Question

Can `id` use p2panda-auth's StrongRemove for membership when every event carries a general `after`
frontier (at most 32 hashes, all kinds)? This covers the four exit criteria: the `after` mapping,
the strong-remove rules on the scenarios, the app-level projections, and unknown-dependency panics.

## Setup

- p2panda-auth 0.7.1 at git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c` (confirmed in `Cargo.lock`).
  The p2panda crates compiled at this rev without changes, so no crates.io fallback was needed.
- Toolchain: rustup 1.97.0 (cargo 1.97.0 `c980f4866`, rustc 1.97.0 `2d8144b78`).
- p2panda-auth feature `test_utils`: char members and u32 op ids, standing in for VerifyingKey and Hash.
- `s2-auth/Cargo.toml` is trimmed to p2panda-auth only (see Deviations). No new crates.

## Commands

- `"$CARGO" build -p s2-auth`: 0 warnings, 0 errors.
- `"$CARGO" run -q -p s2-auth > s2-auth/run-output.txt`: exit 0, empty stderr, 41 PASS, 0 FAIL.
  The full output is in `run-output.txt`.
- Read-only inspection of the p2panda-auth source at this rev (resolver, crdt, state, authority graphs).

## Measurements

- (a) 9 scenarios. Every causal-delivery order was run: 24 orders in total. Each scenario gave one
  outcome across its orders, with 0 errors.
- (b) Mapping DAGs of 4 and 6 events, plus the 16-event delegation DAG. The incremental frontier
  matched the full walk on every event (0 mismatches).
- (c) 7 probes (P1 to P7).
- (d) 5 friendship cases, 13 orders.
- (e) 5 runs and 5 structural checks.
- Not measured: performance. The full-past walk costs O(past) per event.

## Exit criteria

| Criterion | Result |
|---|---|
| `after` mapping decided | PASS. Auth frontier, derived on receipt (see Mapping). |
| StrongRemove rules hold on the scenarios | PASS, with one correction: transitive void is authority-based (3b). |
| App-level projections listed | PASS (list below). |
| Unknown-dependency panics characterised and guarded | PASS. The research note's claimed panic was not reproduced. |

## Scenario results (a)

| # | Case | Result |
|---|---|---|
| 1 | Week-late add by the removed member | voided (both arrival orders) |
| 2 | Causally prior add | survives |
| 3 | Dave's authority came from voided Z; Dave adds Eve after Z | Eve voided |
| 3b | Same, but Claire (independent manager) adds Eve after Z | Eve stands |
| 4 | Mutual removal | both removed |
| 4b | Three-cycle | all removed |
| 5 | Re-add made after Claire's own removal, concurrent with Alice's removal | voided |
| 5c | p2panda's `concurrent_readds_filtered` shape | voided |
| 5b | Causal re-add (control) | stands |

Correction to design §6.2 rule 3 ("every event that depends on a voided event"). p2panda voids a
dependent op only when its author lost authority through the voided op. An independent author's
dependent op stands (3b). Closure over all dependents would change the outcome for 3b. That is an
owner decision. This spike recommends the authority rule for membership.

## Mapping (b)

- **Naive literal** (R's auth dependencies = auth ops in `after` = empty): p2panda **panics** at
  `group/crdt/mod.rs:822` ("group already present in states map", from `apply_action`, reached via
  `validate`). It does not void X. It crashes.
- **Naive with a root dependency** (R names op 0): **X is voided wrongly**. Dave is out, though
  Alice had seen X through D.
- **Raw `after` as dependencies** (names the non-auth event 2): `MissingDependencies(2, 3)`. R waits
  forever. This is a liveness failure.
- **Auth frontier** (R names X, the maximal auth op in its full causal past): **correct**. Dave
  stands, and the result is the same in every order. p2panda's own `GroupCrdt::heads` at the remover
  returns the same set ([1]).
- Minimality across non-auth hops: frontier(X2) = [X1], frontier(R) = [X2].

**Decision: auth frontier.** For each auth event E, our code computes the maximal auth operations of
E's group in E's full causal past, walking `after` through non-auth events. Compute it incrementally:
the candidates are the `after` events (an auth event contributes itself, a non-auth event contributes
its stored frontier), and dominance is tested against stored frontiers. The check in
`dag::incremental_mismatches` confirms this equals the full walk. Derive `dependencies` on receipt from
the verified `after` closure. Do not store a second, unsigned copy.

## Unknown and missing dependencies (c)

- **P1** unknown dependency 999 through `process`: `MissingDependencies(999, 1)`. **No panic.**
  The research note's panic at `expect("all operations present in map")` is not reachable here,
  because `process` checks dependencies before it touches the graph.
- **P2** `members_at` with an unknown head: returns an **empty membership silently**. No error.
- **P3** empty dependencies, **P4** dependencies that exist but lack the group, **P5** non-create on an
  empty state: all **panic** at `crdt/mod.rs:822`. The message is misleading. The group is absent.
- **P7** an invalid op (a read-only member adds): `process` refuses it with
  `StateChangeError(InsufficientAccess)`, it never enters the graph, and its child stalls on
  `MissingDependencies`.

**Guard** (`panics::admit`, verified by P6), applied before `process`:
1. Missing auth dependencies: hold the op as waiting. It is admitted once the dependency arrives.
2. Non-create op with no dependencies: reject.
3. Group absent at the dependencies (`state_at(deps)`): reject.
4. For every event our layer refuses, admit a placeholder no-op, so descendants do not stall (P7).

`catch_unwind` cannot be the guard. `process` takes the state by value, so catching needs a full clone
(`GroupCrdtState: Clone` only under `test_utils`). Use it only as a backstop at a worker boundary.

## Friendship (d)

- Built in: StrongRemove only. p2panda-auth has no member cap and no dissolve. Control F1 keeps four
  members.
- **Cap, ours.** The `ignore` set cannot be used. `StrongRemove::process` clears it and recomputes,
  and `apply_action` is `pub(crate)`. A custom resolver cannot inject filters either. At ingest, an op
  that names a non-pair member becomes a valid no-op, `Promote(creator, read)`. That changes no state
  and adds no authority edge. F2: two concurrent third-party adds give {A, B} in both orders.
- **Dissolve, ours (a projection).** A removal is effective if its target stood at the removal's
  dependencies and does not stand at the removal. Friendship dissolves on the first effective removal
  and stays dissolved (F3 to F5). A re-add restores membership, not friendship.
- Not built: general member caps (the "lowest IDs" rule needs a merge-time projection). Friendship
  activation is also not built. Accept stays application-level, because only a manager can Add.

## Delegation (e)

| Run | 10: Jim adds Zed (after R) | 11: Jim adds Yan (concurrent with R) | 12: Quinn adds Eve (via Joe's void delegation) | Zed in |
|---|---|---|---|---|
| Control: p2panda alone | applied | applied | applied | yes |
| Cascade, Joe's chain only | vetoed | applied | vetoed | no |
| Cascade, plus Dan to Jim | applied | applied | vetoed | yes |
| keep_prior, Joe's chain | applied | applied | vetoed | yes |
| transfer to Dan at removal | applied | applied | vetoed | yes |

Structural checks, all PASS: Joe to Quinn is void (concurrent with R). Joe to Jim is not void (made
before R). Kim (write) cannot grant manage. Joe cannot grant after his removal.

Ours: delegation edges, attenuation at creation, the chain search (any intact chain from the owner, which
is the UCAN rule), the concurrent-removal void, the three modes, and the veto as an ingest-time no-op.
p2panda gives: membership at any causal position (`members_at`), the StrongRemove concurrency rules,
and access levels.

Choices I made: the owner is the only root. Transfer re-issues are derived from the removal's successor
field, with no separate signed event. The successor must still hold at the action's position.

## App-level projections (ours)

1. Auth-frontier shim: derived `dependencies`, computed incrementally.
2. Admission gate (wait or reject, as above) and placeholders for refused events.
3. Friendship pair rule (ingest rewrite).
4. Friendship dissolve projection.
5. Delegation: edges, attenuation, chain search, modes, concurrent-removal void, veto.
6. Void reasons per event ("voided: concurrent with R", "voided: depends on voided Z"), from the same checks.

Not built: the content policy (§6.5), general member caps, friendship activation.

## Deviations

- `s2-auth/Cargo.toml` trimmed to p2panda-auth. The stub listed the whole workspace set.
- The no-op rewrite is a spike device. Production should keep the signed event and derive the rewrite.
- `delegation.rs` is 294 lines against the 300-line budget. It uses the shared fixtures in `dag.rs`
  (337 lines), which every part uses.
- Single-group DAGs only. Cross-group dependencies and multi-group rebuild cost are not tested.
- Transfer re-issues are derived, not signed by the remover as the design says.

## Blockers

None. The p2panda crates compiled at the pinned rev. Nothing was committed or pushed. Writes stayed in
`/home/user/work/spikes/s2-auth`, except for the shared workspace `target/` and `Cargo.lock` that a normal
build touches. There was no `cargo update`.

## Recommendation

1. Adopt StrongRemove for membership, with one `GroupCrdtState` per group, plus the auth-frontier shim
   and the admission gate, including placeholders for refused events.
2. Restate design §6.2 rule 3 as authority dependence, which is what p2panda does. Closure over all
   dependents is an owner decision, and it changes outcomes (3b).
3. Friendship: the pair rule and the dissolve projection. Defer general caps.
4. Delegation: `cascade` as the default, as decided. The veto needs a placeholder and a reason.
5. Correct the research note: an unknown dependency is an error, not a panic. The real panic is
   "group absent at dependencies".
