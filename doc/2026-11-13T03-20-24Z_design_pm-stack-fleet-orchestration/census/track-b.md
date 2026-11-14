# Track B — CLI and lifecycle capability census

**Pinned source:** `a038f084dbeb525494d7bb928ad91f597db2ac51`. All code status claims cite exact
command dispatch/parser evidence at this commit, not deployment. `commands.jsonl` enumerates source
01's command checklist, source 03 §§4.1–4.8 and its common flags, and source 04's recovered concrete
command and option spellings (including the flattened line-13 transcript); future discoveries are
additive. Source 04 remains preserved verbatim.

## Finding

The actual `fleet` multicall command registry has a queue client surface and alias routing, but the
requested `fleet work`, `fleet github`, `fleet metadata`, `fleet dependency`, `fleet activity`,
and `fleet sync` namespaces are not evidenced as existing registered commands at the pinned source.
The `repo-queue` name is an alias/forwarder into the same Fleet binary, not evidence of an
independent work-item lifecycle interface. Queue operations do exist in `fleet-queue`; tests and
smoke checks establish portions of their CLI contract. This is source-level evidence only.

Evidence anchors:

- Actual queue CLI dispatch table: `pkgs/fleet/crates/fleet-queue/src/client.rs:5301–5345`.
  `cmd_list` is :1650, `cmd_claim` :2230, `cmd_release` :2673, `cmd_park` :2914,
  `cmd_complete` :3367; all match arms are explicit. Thus queue `park` is implemented. Exact
  proposal spellings `search`, `get`, `inspect`, `rank`, `resume`, `update`, `close`, `explore`
  and `priority` are absent at that dispatch (nearby `wake`, `complete`, and `list` are not aliases).
  `repo-queue` resolves to the same client via `pkgs/fleet/crates/fleet-registry/src/lib.rs:36,220,331`.
- `pkgs/fleet/smoke.sh:474–485` checks `repo-queue --help`, version, `fleet queue --help`, and a
  broker health failure (exit 3); `:106–124` shows connect-build queue stubs (exit 69).
  `pkgs/fleet/crates/fleet-queue/tests/chunked_list.rs:65–71` invokes the actual `repo-queue list`
  executable.
- Queue-domain implementation is in `pkgs/fleet/crates/fleet-queue/src/` (notably
  `turso_queue.rs`, `queue_broker.rs`, `fallback.rs`); one-queue matching is documented at
  `turso_queue.rs:3100`. Outbox CLI dispatch at `client.rs:5281–5298` is enqueue/list/metrics/
  claim/resolve/drain, not proposed `fleet sync outbox inspect|retry`.
  The issue/charter baseline is #9178; #9180 is the FDR/design anchor. Crosswalk against #9016
  and #8887 is documented in `doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy/note.md:431–505`.
- Existing GitHub write path is `fleet gh` (not the proposed `fleet github` namespace):
  `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/note.md:38,61` identifies the
  rotation path; the PM-board genealogy note §13 says it extends #9016 and §14–16 describe
  Metadata stamping/query and healing. The source docs describe this as shipped/merged, but this
  census does not claim live deployment or verify every command parser/test.
- Creation outbox is explicitly bounded: `doc/fleet/creation-outbox.md:54–60` says exactly one
  producer (`new-issue-creator-code-a`). Design/issue filing should use sanctioned `fleet gh` plus
  queue same-pass, not expand that pilot to general mutations or completion.

## Bounded installed-runtime observation (separate from pinned source)

Receipt `issues/seed/receipt.json:5–26,28–44` records research issue #9241 as an actual child of
#9198 and its live task-15480 readback: the installed command
`repo-queue list --queue repo-health-code-pilot --task 15480 --limit 1` returned task **1111**,
while read-only broker `/coverage` verified #9241 → task **15480**, status `claimed`, coverage
`covered=1, uncovered=0` at 2026-11-13T03:51:03Z. This proves a bounded installed CLI/broker lookup
mismatch for that query, not a source-parser defect or a general production/deployment guarantee.
The pinned source parser `client.rs:1650–1689` accepts and sends `task`; no root cause is inferred.
No fix was made in this census. The proposed `work.inspect`/ID lookup oracle in #9206 `CLI-03`
must negatively test that an explicit ID can never return a different item; retain exact installed
command, observed wrong ID, coverage readback, and pinned parser claim as separate evidence types.

## Compatibility inventory rules

`commands.jsonl` has one row per command, slash/event operation, and per option. Exact per-row slice
assignment (including baseline/out-of-scope/decision boundaries) is in
[`command-slice-map.jsonl`](command-slice-map.jsonl); no family wildcard substitutes for a row.
Status is precise:
**implemented** = exact spelling is registered and has pinned parser/dispatch evidence; **partial**
= exact target exists but some described behavior remains unverified; **absent** = exact spelling
is not registered; **unverified** = source cannot establish parser or deployment state. Adjacent
operations do not make a target partial: `list` is not `search`; `wake` is not `resume`; queue
`complete` is not work-item `complete`. Suggestions are test-oracle requirements, not current
behavior claims.

## Compatibility and ownership

| Area                  | Baseline to preserve                                                                                                       | Starter                                                                                          | Minimum oracle                                                                              |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| Discovery/bootstrap   | help, capabilities, bootstrap scenarios queue-worker/assigned-item/resume/triage/review; compact/json                      | #9206 (bootstrap/CLI), #9203 (agent routing)                                                     | golden command/capability manifest; unsupported capability never advertised                 |
| Queue exploration     | list/search/get/inspect/rank, filters, explain/sampling; claim/release/park/resume/update/close                            | #9206; #9204 for selection policy; #9209 for claim/completion                                    | CLI contract tests for all accepted options; claim race/stale fencing tests owned #9209     |
| Work item             | next/open/inspect/hydrate/refresh/changes/link/checkpoint/status; notes/block/unblock/park/resume/release/handoff/complete | #9206; #9207 hydration/activity; #9208 notes/dependencies/journal; #9209 claim/outbox/completion | scenario A–D acceptance against model; outputs include canonical item and operation receipt |
| GitHub                | issue/PR metadata, comments/timeline/events, commits/diffs/reviews/checks, create/update/merge/close                       | #9203 event/slash routing; #9208 authority/managed regions; #9209 remote mutation                | adapter fixtures plus idempotent event and crash-after-mutation replay tests                |
| Metadata/dependencies | typed fields and independent queue/GitHub representations; parent vs hard/advisory dependency                              | #9208                                                                                            | ownership/conflict matrix; cycle and resolved-history model tests                           |
| Activity/sync         | source cursors, overlap/dedup, attribution, watch/reconcile/status/run/verify/outbox retry                                 | #9207 activity; #9209 outbox                                                                     | duplicate/reordered event corpus; cursor advances only after durable processing             |
| Agent control         | dispatch/status/send/resume, authenticated commands, receipts/ack, timeout recovery                                        | #9203                                                                                            | delivered vs acknowledged vs launched receipts are distinct; duplicate webhook oracle       |

### Pinned existing test evidence and gaps

These are existing source tests at the same pinned revision (they do not establish deployment).
Rows in `commands.jsonl` name exact parser/dispatch locations; this index identifies the closest
current test oracle. Where noted, command-level coverage is a gap, not an implied test.

| Exact current surface                                     | Existing test evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Limitation                                                                                                                                         |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `repo-queue list`                                         | `pkgs/fleet/crates/fleet-queue/tests/chunked_list.rs:65–71`; client unit `claimed_list_recovers_the_claimed_task` (:6453)                                                                                                                                                                                                                                                                                                                                                                | Supports list output/pagination and claim recovery; does not test proposal search.                                                                 |
| `repo-queue claim`                                        | client tests `claim_success_envelope_prints_the_real_row` (:6385), `claim_null_row_is_a_distinguishable_nonzero_no_claim` (:6413), `claim_payload_less_row_is_a_no_claim_not_a_task` (:6432), `claim_output_passes_any_non_null_row_through_verbatim` (:6493), `claim_bind_path_requires_explicit_bind_single_claim_and_session` (:6032)                                                                                                                                                 | Envelope/parser invariants; not a substitute for live claim/lease deployment evidence.                                                             |
| `repo-queue release`                                      | `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs:9945` test `release_verb_contract_8217`                                                                                                                                                                                                                                                                                                                                                                                                | Broker verb contract; no generalized lifecycle release workflow.                                                                                   |
| `repo-queue park`                                         | client `not_before_flag_is_validated` (:6171); `queue_broker.rs:1677` `park_and_revive_not_before_are_gated_before_the_rpc`                                                                                                                                                                                                                                                                                                                                                              | Park spelling is implemented; tests cover validation/guard, not full source proposal checkpoint composition.                                       |
| `repo-queue complete`                                     | Parser/dispatch at client.rs:3367/:5331; no exact client command-level complete test found in this pass                                                                                                                                                                                                                                                                                                                                                                                  | **Coverage gap:** add command parser/result test before claiming full tested CLI semantics. Queue task completion is not verified work completion. |
| `repo-queue stack record/list/transition/restack/replay`  | client.rs tests `stack_transition_body_carries_the_transition_shape_8582` (:5525), `stack_transition_body_rejects_unknown_state_and_bare_conflict_8582` (:5553), `stack_restack_body_carries_the_refresh_shape_8582` (:5565), `stack_record_body_carries_the_append_shape_8582` (:5582); turso_queue.rs storage wiring tests `stack_storage_wiring_8582` (:7766), `stack_transition_wiring_8582` (:7810), `stack_restack_wiring_8582` (:7842); pure tests in merge_intent.rs test module | Replay is a real substrate; does not prove cross-resource lifecycle completion.                                                                    |
| `repo-queue fallback scan/status/claim/heartbeat/release` | `fallback.rs` unit/property test module (:1189+) includes `collect_markers_*`, `marker_round_trip`, `fold_free_and_released`, `fold_live_then_stale_and_heartbeat_renews`, stale/contest tests                                                                                                                                                                                                                                                                                           | Tests state folding and parsing inputs; exact CLI argv coverage is incomplete.                                                                     |
| `repo-queue session status/result/list`, dispatch         | `placement.rs` test module starts :2155; examples include `dispatch_posts_as_app_and_finds_run` (:2571) and `dispatch_ignores_preexisting_run_and_picks_earliest_new_run` (:2689)                                                                                                                                                                                                                                                                                                        | Session status/result/list CLI parser coverage needs an exact per-subcommand test crosswalk; dispatch test is not user-prompt launch proof.        |
| `fleet gh` passthrough                                    | `pkgs/fleet/crates/fleet/src/gh.rs` tests `retry_gate_matches_the_rule` (:983), `read_call_whitelist` (:996), `rotation_order_and_owner_hint` (:1030), `failover_on_rate_limit` (:1166), `write_not_repeated_on_unknown_failure` (:1188)                                                                                                                                                                                                                                                 | Wrapper rotation/retry tested; does not prove Metadata create/query feature deployment or exact every gh subcommand.                               |

The test table is intentionally explicit about gaps. `implemented` denotes exact registered source
behavior, not that every parser edge has command-level test coverage; each gap is a candidate test
slice, not a reason to mislabel an adjacent command as implemented.

### Deliberate boundaries and questions/defaults

1. `fleet queue` and its `repo-queue` argv0 alias are current binary behavior; proposal `fleet work`
   is not an alias unless an explicit design/compatibility decision adds one. Default: additive
   syntax, no silent breaking rename.
2. Existing queue direct close/outbox primitives are not proof of the proposed composed `work
complete` transaction, verified GitHub effects, or deployed behavior. Default: treat them as
   building blocks and separately test the complete crash/retry protocol.
3. Each option spelling is separate in JSONL. Source 03 lines 545–571: `--work` 549, `--repo` 550,
   `--issue` 551, `--pr` 552, `--agent-id` 553, `--session-id` 554, `--lease-token` 555,
   `--since` 556, `--since-start` 557, `--since-checkpoint` 558, `--external-only` 559,
   `--output` 560, `--format` 561, `--json` 562, `--dry-run` 563, `--explain` 564,
   `--verify` 565, `--wait` 566, `--expected-version` 567, `--idempotency-key` 568, `--timeout` 569.
   The `--order` example at 376 is preserved as a candidate separately from supported filters.
4. Exact #9178/#9180 issue comment/FDR state and #9016/#8887 implementation status require live
   issue-source verification; local references establish only the cited design/context. Do not
   claim deployed functionality from merged code or issue titles.
5. Source 04 is flattened into one 82KB block at line 13. Its normalized commands/options are
   individually inventoried; formatting ambiguity remains a source limitation, not evidence of
   absent requirements. Any discovered item is additive.

The proposed FDR namespace `FDR-PMO-001` avoids the seed IDs `0001–0010` collision; it is a
proposal, not a ratified identifier. No production semantics, schema, dispatcher, resolver,
database, or binary changes were made.
