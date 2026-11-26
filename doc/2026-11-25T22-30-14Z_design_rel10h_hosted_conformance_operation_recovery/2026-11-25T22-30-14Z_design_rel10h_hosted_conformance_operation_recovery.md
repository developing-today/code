# REL-10H — Hosted contract conformance for operation recovery

Design record for
[#9488](https://github.com/developing-today/code/issues/9488)
(`pmo-9196:REL-10H`), the hosted half of slice REL-10 in
`components/9209-leases-fencing-outbox-direct-completion.md`. The
migration half is REL-10M, merged as
`doc/2026-11-25T22-28-01Z_design_rel10m_migration_backfill_rollback/`
(#15314).

The split refinement says: "Compare state-machine, stale-fence,
operation and completion fixtures across existing hosted targets
#9079/#9081; do not reimplement either hosted service." The issue
owns one file, `pkgs/fleet/crates/fleet-queue/tests/hosted_conformance.rs`.

This is a design record. It authorizes no code, deployment or hosted
run.

## Governing decisions consumed

- **Fleet tool, Workers edition** (#9081, design
  `doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition/`): "one
  queue", the hosted Worker is "a second broker front door" on the
  same fleet-queue database; "Claim _semantics_ are identical; the
  trust boundary differs"; one data model across two implementations
  is kept honest by "a shared fixture corpus … that both the Rust and
  TS implementations test against", and "a schema change lands as a
  schema PR first, implementations second".
- **PM service, Workers edition** (#9079, design
  `doc/2026-11-23T03-22-03Z_design_pm-service-workers-edition/`).
- **Component 9209**: the REL state machines, fencing, outbox and
  direct completion this parity covers.
- **REL-02**: one monotone `claim_generation` per task, incremented on
  every holder change, and every fenced write conditional on it.
- **REL-03 / REL-04 / REL-06 / REL-07**: receipt identity, intent
  rows and their transition table, read-back, completion phases.
- **REL-10M**: what "off" and re-enable do, and the four negatives.

## Verified state on `origin/main` `d9786f35`

- `pkgs/fleet/crates/fleet-queue/tests/` holds `broker_bin_args.rs`,
  `chunked_list.rs`, `link_cli.rs`, `person_wait_cli.rs` and three
  Python SQL checks. `hosted_conformance.rs` does not exist.
- The #9081 hosted door is `pkgs/fleet/hosted-worker/`, a TypeScript
  Worker. `src/queue.ts` is described in its own header as "a faithful
  port of the native broker's verbs": claim, lease expiry,
  transfer, `complete_task` and others, over three stores (libSQL,
  Hrana, D1) that each run the shared schema with additive `ALTER`s
  at open.
- Its parity today is per suite: `libsql-queue-parity`,
  `libsql-hrana-parity`, `d1-queue-parity`,
  `live-generation-queue-parity` (against a schema fixture read back
  from the deployed D1 database), and `session-parity`, which "runs
  the case table transcribed from the native resolver's own tests".
  These compare the TS stores with each other or with a transcribed
  table. No fixture file is read by both the Rust and the TS tests.
- `hosted-worker/src/queue.ts` has no `claim_generation`, operation,
  receipt, intent or `completing` concept. Neither has native main:
  REL-02 to REL-07 are design records.
- #9081 sub-issues still open: #12584 (session registry), #12589
  (queue front door), #12592 (direct-Turso read path). #9079 is open;
  `pm-stack/pm-service/` holds tenant, dashboard, read-API and front
  door code and does not run queue verbs.
- The issue packet's first failing test and done command are
  REL-10M's (`pmo_rel_migration_rollback_preserves_remote_effect_receipt`,
  with `--lib`), copied from the pre-split REL-10 slice. A test in
  `tests/hosted_conformance.rs` is an integration test target, which
  `--lib` does not run. §7 gives REL-10H its own oracle and command.

## 1. Intent

The REL contracts land natively first. The hosted door is a second
writer on the same database. If it does not keep the same contracts,
the native guarantees are false for any task the hosted door
touches, whatever the native tests say. REL-10H fixes:

- what must be identical between the native broker and each hosted
  target, and what may differ;
- how that is checked without reimplementing either service;
- the rule that a REL capability is not switched on for a queue the
  hosted door can write until the hosted door conforms.

## 2. Ownership

| Concern                                                                        | Owner                                        |
| ------------------------------------------------------------------------------ | -------------------------------------------- |
| The fixture corpus format and the native runner, `tests/hosted_conformance.rs` | **REL-10H (this)**                           |
| Making the hosted door pass the corpus                                         | the #9081 line (`hosted-worker`)             |
| PM service participation                                                       | the #9079 line, only if it gains a REL write |
| The contracts themselves                                                       | REL-02 to REL-07                             |
| Migration and rollback model                                                   | REL-10M                                      |
| Schema, deploys and hosted runs                                                | their owners, each separately authorized     |

REL-10H writes no TypeScript. The TS runner over the same corpus is a
test in `hosted-worker/test/`, filed under #9081 as that design's
"shared fixture corpus" issue.

## 3. What parity means

**Must be identical, for the same fixture:**

- the state after each step: task status, holder, lease, and
  `claim_generation` once REL-02 lands;
- each refusal's code (`FENCED`, `FENCE_REQUIRED`,
  `IDEMPOTENCY_KEY_REUSED`, `LEGACY_OPERATION` and so
  on, as the REL records spell them) and the fact that a refusal wrote
  nothing;
- operation status from intent statuses (REL-04 §6's precedence) and
  the rendered outcome (REL-03 §5.2);
- the receipt identity block, byte for byte;
- the intent block and `payload_hash`;
- which intents may be sent next, and in which `target_seq` order;
- completion phase per REL-07, and that finalize happens once;
- the REL-10M disable and re-enable outcomes.

**May differ:** transport, auth and tenancy (the hosted door requires
authentication; the native door keeps its local trust model, as the
#9081 design names), store engine, timing, and error message text
beside the code.

Timestamps come from the fixture's clock, not the host's, so they
compare exactly. A target that cannot take an injected clock does not
conform.

## 4. The fixture corpus

One directory, `pkgs/fleet/crates/fleet-queue/tests/fixtures/hosted-conformance/`,
added by REL-10H's implementation. One JSON file per case:

```json
{
  "schema": "fleet.rel-conformance-case/v1",
  "case": "stale_fence_after_reclaim",
  "contracts": ["REL-02"],
  "clock": ["2026-11-25T00:00:00Z", "2026-11-25T00:10:00Z"],
  "initial": { "tasks": [ … ] },
  "steps": [
    { "verb": "claim", "as": "worker-a", "expect": { "ok": true } },
    { "advance_clock": 1 },
    { "verb": "expire_leases", "expect": { "ok": true } },
    { "verb": "claim", "as": "worker-b", "expect": { "ok": true } },
    { "verb": "complete", "as": "worker-a", "fence": "1",
      "expect": { "error": "FENCED" } }
  ],
  "final": { "tasks": [ … ], "events": [ … ] }
}
```

- Each step names a verb from the broker protocol, the caller, and its
  expectation. `final` is the full state of the tables the case
  touches, compared after normalizing only what §3 lets differ.
- Cases are tagged with the contracts they exercise. A target runs
  only the cases whose contracts it implements, and reports the rest
  as not run, never as passed.
- The corpus is data. Changing an expected value is a contract change
  and lands as its own PR before either implementation changes,
  following the #9081 design's schema-first rule.

### 4.1 Required cases

| Area          | Cases                                                                                                                  |
| ------------- | ---------------------------------------------------------------------------------------------------------------------- |
| state machine | each REL-04 §6 transition, including "no transition leaves `confirmed`"; the precedence table's examples               |
| stale fence   | reclaim then stale write; transfer bumps the generation; legacy claim before `fence_enabled_at`; REL-02 re-enable bump |
| operation     | same key same digest replays; same key different digest conflicts; replay writes nothing; legacy key                   |
| completion    | REL-07's first oracle (merge applied, response lost, read-back confirms, one merge); partial close; head mismatch      |
| rollback      | REL-10M §4's timeline; partial store stays off                                                                         |
| cross-door    | a claim through one door and a fenced write through the other; a replay presented at the other door                    |

The cross-door cases are the point of the corpus. Both doors write
one database, so the native runner and the hosted runner each play
both callers against one store.

## 5. The rule for switching a capability on

A REL capability (fencing, the operation store, `completing`) is
enabled for a queue only when every door that can write that queue
passes the corpus cases for that capability. Until then:

- a door without the capability is an old binary in REL-10M §5.1's
  sense;
- for fencing this matters at once: a claim through a door that does
  not increment `claim_generation` gives two holders the same
  generation, which is exactly the hazard REL-02 §8's re-enable bump
  exists for. So fencing stays off for any queue the hosted door can
  claim from until the hosted door increments the generation.

"Every door that can write that queue" is read from the deployment,
not assumed: the queue front door issue (#12589) states which queues
the hosted door serves.

## 6. Running parity

| Stage                         | What runs                                                                         | Where                 |
| ----------------------------- | --------------------------------------------------------------------------------- | --------------------- |
| native conformance            | `tests/hosted_conformance.rs` over the corpus against an in-process store         | remote CI             |
| hosted conformance (code)     | the TS runner over the same files against the libSQL, Hrana and D1 stores         | `hosted-worker` tests |
| hosted conformance (deployed) | a read-only check of the deployed door's reported capabilities and corpus version | separately authorized |

Each stage reports the corpus version, the cases run, passed and not
run, and the commit. A hosted stage is never claimed from a native
pass, or the reverse.

## 7. Oracle

REL-10H's own first test, in `tests/hosted_conformance.rs`:

`pmo_rel_hosted_conformance_corpus_runs_natively`: loads every case
in the corpus, rejects a file whose `schema` it does not know, runs the
cases whose contracts are implemented natively, and asserts each
step's expectation and `final`. It fails on an empty corpus, so a
zero-case pass is impossible.

Then, per area in §4.1, named cases such as
`pmo_rel_hosted_stale_fence_after_reclaim` and
`pmo_rel_hosted_cross_door_replay_is_same_receipt`.

Done command, after `-- --list` shows the test:

```
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --features client,queue-broker --test hosted_conformance pmo_rel_hosted_conformance_corpus_runs_natively
```

## 8. Gates and sequencing

- REL-02, REL-03, REL-04 and REL-07 code lands natively first; a case
  is added to the corpus when its contract has native code to run it.
- The TS runner and the hosted door's REL support are #9081's work,
  sequenced after the native cases they target.
- No capability is enabled for a queue the hosted door writes before
  §5 holds.
- No hosted deploy, live call or schedule is part of this record.

## 9. Requirement trace (design-level)

| Requirement | Where                                                                                                               |
| ----------- | ------------------------------------------------------------------------------------------------------------------- |
| C-R136      | §4.1 completion and rollback cases: recovery from a stored receipt behaves the same at both doors                   |
| C-R137      | §3: claim, operation and remote state stay separate facts at both doors                                             |
| C-R142      | §4.1 completion cases: a lost response is `unknown` at both doors, never a local success                            |
| O-R011      | §4.1 operation cases only: an accepted request replays the same receipt at either door; do-now dispatch is DSP-08's |
| P-R008      | §2 and §10: no second implementation, store or service                                                              |
| P-R024      | §3: refusal codes and outcomes identical across doors                                                               |
| P-R037      | §4.1 rollback cases: interrupted work resumes without a second effect at either door                                |

Design-level only. Each row still needs the native, hosted and
deployed evidence of §6.

## 10. Non-goals

- No TypeScript, and no change to `hosted-worker` or `pm-service`.
- No second implementation of either hosted service.
- No new queue, store, service or scheduler.
- No change to any REL contract; the earlier records govern.
