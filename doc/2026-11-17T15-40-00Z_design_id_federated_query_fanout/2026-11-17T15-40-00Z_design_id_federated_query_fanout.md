# Federated query fan-out — design record (#9144)

- **Issue:** developing-today/code#9144 — "design(id): federated query fan-out — prioritized remotes, trusted relay, max-hops"
- **Parents:** epic #9108 (Part B — federation & trust); program #9106.
- **Queue task:** 20336 (repo-health-code-pilot).
- **Date:** 2026-11-17. **Status:** proposed design. Nothing in this record is implemented.
- **Extends, does not replace:** the v1 local/remote query contract
  (`doc/2026-11-08T06-45-00Z_design_id_query_local_remote_v1/`) and the Part B
  V3 query contract (`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md`,
  "Query contract" and "Response/provenance"). Where this record is more specific,
  it refines those records; where they settle a point, this record repeats it
  rather than changing it.

## 1. Problem

A requester asks an ID server a query. The server's own store may not hold
everything. Today the only remote path is a one-hop peer query
(`pkgs/id/crates/id-cli/src/commands/peers.rs:432`, `query_remote_peers`). It has
no ordering contract, no relay, no hop bound and no provenance chain. The
design must say how a server answers from its own store, then from a prioritized
list of remotes, and how a trusted remote may fan out again within a bound. It
must also say how every hop is validated and how the answer's provenance is
presented.

Statement inventory S1.47–S1.63 (atlas proto note §1E,
`doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`)
is the requirement base. The issue body adds the configuration surface that the
§7 modes issue (#9111) owns.

## 2. Proposal in one paragraph

Every query is an origin-signed, immutable `OriginQuery` with an initial hop
bound and per-query budgets. The receiving server answers from its local store
first. It then selects remotes from its configured list in priority order,
skipping any remote that is not eligible. Only fully trusted remotes with relay
permission may be asked to fan out again, and each relay forwards with a signed
`RelayGrant` that decrements the hop bound and consumes a reserved slice of the
budgets. Responses travel back with the original responder's signature intact.
Each relay adds its own signed forwarding receipt. Every hop validates the
signature, binding, scope, budget and hop chain before it forwards or uses the
answer. The requester receives a single answer with a `completeness` marker, a
list of the peers that were not asked or did not answer, and the full per-hop
provenance chain. A remote's negative answer is never treated as global absence.

## 3. Ordering and configuration

### 3.1 What is decided here

The configuration surface that this design needs, and the defaults chosen for
each item. The config machinery itself is owned by #9111.

| Item                 | Representation                                                                                                                   | Default                                                          |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Local store          | Always first; not configurable                                                                                                   | Always queried first                                             |
| Remote list          | Ordered list of peer entries `{ peer_id, priority, relay, timeout_ms?, scope? }`                                                 | Empty list: local-only behaviour, identical to v1                |
| Ordering semantics   | Ascending `priority` integer; ties broken by list position                                                                       | Position in list is the priority                                 |
| Negative answer      | Not-found from a remote is recorded as "absent at that peer"                                                                     | Continue to the next remote (see 3.3)                            |
| Eligibility          | A peer is eligible when its peer class allows query-out and the peer is not disabled or revoked (`S1.47` peer-class gate, #9141) | Ineligible peers are skipped and listed as skipped in provenance |
| Routing observations | May reorder only when the operator opts in; advisory, signed, expiring                                                           | Off; configured order is authoritative (B23 soft sequencing)     |

Prioritization inputs named in S1.53 (trust class, reputation, coverage,
latency, cost) are **candidates**, not inputs in v1. Only the configured
`priority` is used. Trust class is a gate (eligibility), not a score. Reputation
is evidence-derived local policy (Part B V4) and is not used here.

### 3.2 Priority is not authority

A priority decides only the order of asking. It does not change what a remote
may assert. A high-priority remote's answer carries the same weight as any other
signed answer. Its absence is recorded per peer. A remote that is not trusted
enough to relay is still eligible as a direct remote if its class allows
query-out.

### 3.3 Negative answers

A remote returning "no match" or an empty page means "not at this peer, at its
policy and time". The requester's answer may become complete only through an
explicit completeness claim (Part B V3: `complete=true` is an explicit claim;
omission is never inferred from absence). The default is to continue down the
list until one of these holds:

- the requested limit is satisfied (`stop_at_limit`, default on);
- the list is exhausted;
- the query deadline (section 7) is reached.

A `first_match` mode is possible but is not part of v1. See open question Q4.

## 4. Trust and relay permission

### 4.1 Representation

Relay permission is a property of the **peer class**, not of the individual
query. Part B's peer classes (#9141, S1.1–S1.25) are fully trusted, send-only and
accept-signed-from. The relay rule for this design:

- `query-out` (ask this peer): allowed for `fully_trusted` and `accept_signed_from`.
  Send-only peers are never asked.
- `relay` (this peer may fan out on our query): allowed only for `fully_trusted`
  peers that are also marked `relay: true` in configuration. Accept-only peers
  never relay (S1.55).

Default: `relay: false` for every peer. An operator must opt in per peer.

### 4.2 What a relay may do

A relay that accepts a query may fan it out only to peers that pass the same
rule for its own configuration. It may not rewrite the origin query, its
signature or its binding fields. It may not re-sign the response as the origin.
It may only add its own forwarding receipt (Part B V3, "Response/provenance").

### 4.3 Rejection

A relay refuses to forward when the peer class forbids it. The refusal is
recorded in provenance as `relay_not_permitted` for that hop. The query continues
on the other eligible remotes.

## 5. Termination, loops and hop bounds

### 5.1 Hop bound

- `initial_max_hops` is set by the origin and signed inside `OriginQuery`
  (Part B V3). Each relay decrements the count in its `RelayGrant` and passes the
  remainder on.
- **Default:** `initial_max_hops = 3` (origin → relay → relay → end). The local
  store is hop 0.
- **Ceiling:** 8. A configuration may lower the default. It may not raise the
  ceiling. A hop count above the ceiling is rejected at origin validation.
- A relay with `remaining_hops == 0` answers from its local store and does not
  fan out.

These values are chosen to match the internet-request style in the issue. They
are defaults and should be validated by the benchmark slices, not assumed. See
open question Q1.

### 5.2 Loops

A signed `RelayGrant` path proves which peers have already been visited. A relay
refuses a query whose path already contains its own ID (the `AlreadyInChain`
refusal in `relay_gate.rs`, which already exists for the #2036 relay). A repeated
`request_id` is a duplicate, not a loop: in-flight duplicates attach to the
existing work item and completed duplicates return the cached deterministic
outcome (Part B V3 "Duplicate/replay/cycle state").

### 5.3 Breadth and total cost

Fan-out is bounded by a reservable budget vector per query: hops, peer breadth,
work units, bytes in/out and a monotonic elapsed deadline. Each relay reserves a
slice of the remaining vector and passes disjoint child slices on. Breadth ×
depth is therefore bounded by the origin's vector, not by the product of the
per-hop maxima.

- **Default peer breadth per relay:** 4.
- **Default work units, bytes and deadline:** taken from the origin query
  (section 7). The defaults belong to B16 (`fanout_budget`) and are not set here.

## 6. Validation, signatures and provenance

### 6.1 Validation at every hop

Every hop validates before it uses or forwards anything:

1. origin `OriginQuery` signature, expiry and recomputed `request_id` (B20);
2. `RelayGrant` signature, chain link to the parent grant, and path (B17);
3. the relay's reservation against its own ledger (B17, B21);
4. the responder's `SignedResponse` signature, binding, scope and policy digest
   (#10249, S1.58–S1.60);
5. the relay gate sequence (`relay_gate.rs`): invalid envelope, invalid chain,
   authorization, duplicate, already-in-chain, cap.

A message that fails any check is refused. No path forwards an unchecked message
(the total `decide` function in `relay_gate.rs`).

### 6.2 End-to-end responder signature

A relayed `SignedResponse` keeps the original responder's bytes and signature.
The relay adds its own signed forwarding receipt. The requester validates the
responder signature directly and does not need to trust intermediate hops for
the content. It trusts intermediate hops only for the path claim, which is
checked through the receipt chain.

### 6.3 Provenance

The answer carries a provenance list, one entry per hop that was asked:

```
ProvenanceEntry {
  hop_index,            // 0 = local store
  peer_id,              // responder or relay that handled this hop
  path[],               // peer IDs from origin to this hop
  status,               // answered | empty | not_permitted | ineligible |
                        //  timeout | unavailable | refused | budget_exhausted |
                        //  result_evicted
  responder_signature?, // present when a SignedResponse was received
  receipt_digest?,      // relay forwarding receipt, when a relay handled it
  generated_at
}
```

`status` values beyond `answered` and `empty` are explanations for absence. They
are not verdicts. A `timeout` or `unavailable` entry does not mean the peer
lacks data.

### 6.4 Presentation to the requester

- The requester sees every provenance entry, including the path. Default: the
  full path is shown. Open question Q3 covers whether a relay may hide its
  identity behind a pseudonymous handle.
- Results are deduplicated on stable result ID and keep the provenance of every
  copy (Part B V3).
- Classification and sensitivity (#5818) still decide what content a requester
  may see. They do not change the provenance record's existence. A withheld
  result is not announced as a filtered result. This follows v1 §8 "truncation
  honesty".

## 7. Errors, timeouts and partial answers

### 7.1 Deadlines

- The origin query carries one monotonic deadline. Each relay receives a child
  deadline that is no later than its parent's.
- **Default origin deadline:** 5 s. **Default per-peer cap:** 2 s. Each hop uses
  `min(child_deadline, per_peer_cap)`.
- The deadline is never reset or refunded (Part B V3).

### 7.2 Slow or down peers

A peer that times out or is unreachable is recorded in provenance with status
`timeout` or `unavailable`, and the query continues with the next remote. The
requester's answer is `completeness = partial` whenever any eligible remote was
not asked to completion. It lists the peers that did not answer. The local
results and all successful remote results are returned and are not discarded
because one peer failed.

### 7.3 Completeness

`completeness` is one of:

- `complete` — every eligible remote in the list answered, or the limit was
  reached before the list was exhausted (with `stop_at_limit`). This is an
  explicit claim, scoped to the eligible remotes at the time of the query.
- `partial` — at least one eligible remote did not answer within the deadline,
  was refused, or ran out of budget.

A `complete` answer does not claim global completeness. Only the scope of the
query and the policy that applied to it are claimed.

### 7.4 Truncation honesty

Results cut by the limit, the budget or the deadline say so and say how to
continue (v1 §8). They do not present a cut as a complete answer.

## 8. Compatibility

- **v1 local-only queries are unchanged.** With an empty remote list, the query
  path is the v1 local path. No new fields appear in the answer.
- **Additive response fields.** `completeness`, `provenance[]` and
  `unanswered_peers[]` are new fields on the answer envelope. Older clients that
  ignore unknown fields keep working. The v2 receipt schema (S1.100–S1.104,
  #10250) is a separate record and is not changed here.
- **Peer classes.** Fan-out depends on the peer classes from #9141. A deployment
  with no peer classes configured has no eligible remotes and behaves as
  local-only.
- **Configuration.** The new keys (`remotes[]`, `priority`, `relay`, `max_hops`,
  `deadline_ms`, `per_peer_timeout_ms`, `peer_breadth`) belong to the #9111
  config schema. This record states their meaning and defaults; #9111 owns the
  file format and the merge rules.

## 9. Code and doc evidence

| Claim                                                   | Evidence                                                                                                                                                    |
| ------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A one-hop remote query exists today                     | `pkgs/id/crates/id-cli/src/commands/peers.rs:432` (`query_remote_peers`)                                                                                    |
| Validate-before-forward relay gate with hop cap         | `pkgs/id/crates/id-sync/src/relay_gate.rs` (`GateRefusal`: `InvalidEnvelope`, `InvalidChain`, `NotAuthorized`, `Duplicate`, `AlreadyInChain`, `CapReached`) |
| Federation slice registry (26 modules, not all present) | `pkgs/id/crates/id-sync/src/federation/slices/mod.rs` (`B_SLICE_REGISTRY`)                                                                                  |
| Present slices: b01–b05, b08, b20, b21, b31             | `pkgs/id/crates/id-sync/src/federation/slices/`                                                                                                             |
| Absent slices relevant here: b15, b16, b17, b18         | Not in `federation/slices/` at `5ccf878eae`                                                                                                                 |
| Settled query and relay contract                        | `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/versioned-design.md`, V3                                                                            |
| Local/remote query contract                             | `doc/2026-11-08T06-45-00Z_design_id_query_local_remote_v1/…md`, §§4, 7, 8                                                                                   |
| Statement map S1.47–S1.63                               | `doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`, §1E and §3                                                             |

## 10. Existing implementation issues and coverage

The statement IDs are covered by these filed sub-issues of #9144:

| Issue  | Scope                                                                                      | Statements         |
| ------ | ------------------------------------------------------------------------------------------ | ------------------ |
| #10229 | Prioritized remote selection (B15): inputs, ordering, negative answer                      | S1.47–S1.53        |
| #10263 | Trusted-relay permission, max-hop propagation, breadth × depth bounds (B17 and B21 inputs) | S1.54–S1.57, S1.61 |
| #10249 | Query, relay and response signature validation; responder provenance; request identity     | S1.58–S1.62        |
| #10264 | Privacy surface of query-interest disclosure and mitigation decision                       | S1.63              |
| #10250 | Shared signed receipt schema (V2)                                                          | S1.100–S1.104      |

Everything in sections 3–7 that is not in the table above is a gap. The gaps
are listed as follow-on scopes in section 11.

## 11. Proposed follow-on scopes

These are the scopes this record adds to the existing sub-issues. They are not
duplicates of them. Each names the issue that should own it.

1. **Fan-out configuration surface (owner: #9111 modes/config).** The
   `remotes[]`, `priority`, `relay`, `max_hops`, `deadline_ms`,
   `per_peer_timeout_ms` and `peer_breadth` keys, their defaults and their
   ceilings (section 8), plus validation that rejects a `max_hops` above the
   ceiling. Acceptance: a config test for every default and every ceiling.
2. **Deadline, timeout and completeness semantics (owner: a new child of #9144).**
   The `completeness`, `unanswered_peers[]` and `ProvenanceEntry.status` fields
   (sections 6.3, 7.2, 7.3), with a golden test that a slow or down peer yields
   `partial` and keeps every other result. Oracle: a fixed fixture of three peers,
   one slow, one down, one answering.
3. **Requester-facing provenance presentation (owner: a new child of #9144).**
   The full path default, the optional pseudonymous relay handle (Q3), and the
   no-announcement rule for withheld results (section 6.4). Oracle: the v1 §8
   truncation-honesty rule plus the #5818 disclosure policy.

These three are filed as follow-on issues (see the PR description) only if the
task packet allows it; otherwise they are recorded here for the owner to file.

## 12. Open questions and defaults

Each question is recorded with the default chosen for this design. A detached
session does not call `question`; the default holds until a human or the owner
changes it.

- **Q1. Is 3 the right default for `initial_max_hops`?** Default: 3, ceiling 8.
  Owner: B16 benchmark slice, with a measured latency and cost comparison.
- **Q2. Should `priority` also accept routing observations?** Default: no. Only
  when the operator opts in, and then as advisory signed observations (B23).
- **Q3. May a relay hide its identity in the provenance path?** Default: no. The
  full path is shown. A pseudonymous handle is an open option, owned by
  Part B's privacy slice (#10264).
- **Q4. Should a `first_match` mode exist?** Default: no. v1 continues down the
  list until the limit or the deadline. Owner: #9111.
- **Q5. Should the origin deadline default be 5 s?** Default: 5 s, per-peer cap
  2 s. Owner: B16.
- **Q6. Do relayed peers see the origin's query text?** Default: yes, under the
  privacy rule in #10264 (S1.63). Every queried remote learns the query interest.
  Config declares the allowed scope and retention.

## 13. What this record does NOT decide

- The peer class definitions and the signature gate — #9141 (S1.1–S1.25).
- The config file format and merge rules — #9111.
- The shared receipt schema — #10250.
- Reputation and routing observations as inputs to priority — Part B V4 and B23.
- Mutating remote operations — v1 §7 and #5822. Fan-out is read-only.

## 14. Acceptance

- Every S1.47–S1.63 statement is mapped to a filed sub-issue (section 10).
- Each requirement in the issue body is answered in a section:
  ordering and configuration (3), trust and relay permission (4), termination,
  loops and hops (5), validation, signatures and provenance (6), errors and
  timeouts (7), compatibility (8).
- Open questions are recorded with defaults (section 12).
- Gaps are listed as follow-on scopes (section 11), not silently dropped.
