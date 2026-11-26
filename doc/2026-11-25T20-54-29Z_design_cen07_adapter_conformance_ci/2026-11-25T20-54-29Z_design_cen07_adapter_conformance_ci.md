# CEN-07 — Real-adapter conformance and CI adoption

Issue: [#9437](https://github.com/developing-today/code/issues/9437)
(slice `CEN-07` of [#9198](https://github.com/developing-today/code/issues/9198),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the CEN-07 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row says: "run shared golden command/receipt/selector/cursor/authority
fixtures through existing adapters and add their owner-approved
command to CI; no new service/backend", with first test
`test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance`
and negatives "CI failure prevents claiming conformance; reverting CI
hook leaves test scripts but does not roll back any live state. Never
silently point projector writes at Fleet or treat projection ACK as
command acceptance." This record _decides_ which fixture families
exist today and which are gated, what "through existing adapters"
means for each, what the CI venue is under the repository's CI
policy, how a family that cannot run yet is kept from counting, and
what the first test checks. It also records two places where the
projector and Fleet disagree today. It authorizes no code, workflow,
deployment or live acceptance; those remain separately gated stages
(see §8).

Governing decisions consumed, not re-opened:

- **CI policy** (`.github/ci-policy.md`, owner ruling 2026-11-16):
  automatic CI runs on a nightly `schedule` plus `workflow_dispatch`,
  not on push or pull request; every workflow is listed in the
  agreement registry and checked by
  `.github/scripts/check-ci-policy-agreement.sh`.
- **Test-suite registry** (`.github/test-suite-registry.json`): each
  suite has one row naming its paths, its exact invocation and its
  venue; a suite with no venue is `uncovered`, and "self-skip must
  not count as green".
- **DSP-01** (#9445, closed): projection delivery dedupe is never
  command acceptance; the delivery GUID is provenance only. The
  projector forwarder states that "No sink is configured in the
  default serve() wiring yet, so forwarding stays inert"
  (`pm-stack/projector/src/operation-forwarder.ts:20-22`).
- **REL-03 record** (#9513, merged in #15240) for the receipt family
  and the edited-comment conflict; **HYD-06 record** (#9508,
  `doc/2026-11-23T19-40-12Z_design_hyd06_activity_revisions_tombstones/`)
  for cursors; **PRI-01** (#9478, closed, `work/priority.rs`) for the
  selector family.
- The issue's own hard blockers: PRI-01, REL-03, HYD-06, AUTH-01; and
  #9028 as a runtime gate that does not block offline contracts and
  fixtures.

## 1. Intent

A contract is only shared if both sides of it are checked against the
same examples. CEN-07 gives the PM orchestration contracts one place
where golden fixtures are fed to the code that actually serves them,
on both the Fleet (Rust) side and the PM Stack (TypeScript) side, and
one CI signal that says which contract families conform. A family
that is not built yet is listed as not built, never counted as
passing.

## 2. What exists at `origin/main` `84653ab7d`

| Family                             | Contract owner          | Code that serves it on main                                                                                       | Status |
| ---------------------------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------- | ------ |
| `command` envelope                 | DST-01 / CLI-01 (#9451) | `fleet-queue/src/work/contracts.rs`: `fleet.command/v1`, intent DTO, digest, `fleet.result/v1`                    | landed |
| `acceptance` (projection vs Fleet) | DSP-01 (#9445)          | projector `src/webhook.ts` + `src/operation-forwarder.ts`; `fleet-queue/src/work/operations.rs` acceptance ledger | landed |
| `selector` (priority profile)      | PRI-01 (#9478)          | `fleet-queue/src/work/priority.rs`; digest domain `fleet.selection-order/v1` registered in `contracts.rs`         | landed |
| `authority.managed_section`        | AUTH-04 (#9431)         | `fleet-queue/src/work/authority.rs`                                                                               | landed |
| `receipt` (operation receipt)      | REL-03 (#9513)          | none; record merged, implementation gated                                                                         | gated  |
| `cursor` (activity revisions)      | HYD-06 (#9508)          | none; record merged, implementation gated                                                                         | gated  |
| `authority.field_ownership`        | AUTH-01 (#9429)         | none; issue open                                                                                                  | gated  |

Two venues already run the code in question: `fleet-rust-tests.yml`
runs `cargo +1.99.1 test --manifest-path pkgs/fleet/Cargo.toml --workspace --all-targets --locked`
nightly, and `pm-stack-tests.yml` runs the projector's `bun test`
nightly. Neither names a PM orchestration contract, and neither can
say "the command family conforms" as a separate result.

### 2.1 Where the two sides disagree today

These are source facts, found while mapping the `acceptance` family.
They are what conformance is for, and they are recorded, not fixed,
here.

| ID  | Mismatch                                                                                                                                                                                                                                                          | Owner to resolve    |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| C1  | The projector's envelope carries the verb's `args` (`operation-forwarder.ts:136-146`); the Rust `operations::Command` has no arguments field (`operations.rs:206-223`). The argument of `/fleet priority` is dropped at the Fleet boundary.                       | REL-03 §3.2, DSP-02 |
| C2  | The forwarder handles `edited` as well as `created` comments with the same key (`operation-forwarder.ts:126-137`). Combined with C1, an edited `/fleet priority` value is accepted by today's ledger as a duplicate of the old one. REL-03 makes that a conflict. | REL-03 §7           |

## 3. Fixture families

### 3.1 Shape

Fixtures live in one directory beside the Rust conformance target,
`pkgs/fleet/crates/fleet-queue/tests/fixtures/pmo-conformance/`, one
subdirectory per family, plain JSON with a short `README.md` per
family naming its contract owner and source record. Each fixture has
an input, the expected output and, for negative cases, the expected
typed error. Fixtures hold no credentials, tokens, real comment
bodies or production identifiers; repository and user names are the
synthetic ones the existing unit tests already use.

A manifest, `manifest.json`, lists every family with its status
(`landed` or `gated`), the gating issue for a gated family, and the
exact test names that cover a landed family.

### 3.2 What "through existing adapters" means per family

| Family                      | Fleet side drives                                                                      | PM Stack side drives                                                                                                                         |
| --------------------------- | -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `command`                   | `Command::from_json`, `validate`, `compute_request_digest`, `CommandResult` round-trip | none today (no TypeScript producer of `fleet.command/v1` on main)                                                                            |
| `acceptance`                | `AcceptanceLedger::accept` with the forwarded envelope mapped to `operations::Command` | the existing projector tests that pin the forward (`webhook-delivery-id.test.ts`, `webhook-operation-forwarding.test.ts`), run by exact file |
| `selector`                  | `priority.rs` profile validation and comparison                                        | none today                                                                                                                                   |
| `authority.managed_section` | `authority.rs` capability policy                                                       | none today                                                                                                                                   |

Real adapters, fixtures in, typed results out. The only fakes are at
the network edge (GitHub, a provider, the broker HTTP sink), and a
fake never stands in for the code under test. A passing run is
contract evidence only; it says nothing about deployed behavior.

### 3.3 Cross-language check for the `acceptance` family

The projector's key template and envelope fields are the other half
of the Rust contract, and CEN-07 may not edit projector code. So the
Rust conformance target reads
`pm-stack/projector/src/operation-forwarder.ts` read-only and asserts
two facts: the key template is exactly
`ghcmd:v1:${repo}#${issue}:comment:${comment.id}:${command.verb}`,
and the envelope's field list matches the fixture's
`forwarded_envelope.fields`. If the projector changes either, the
`acceptance` family fails until the fixture and the Rust mapping are
updated together. `operation-forwarder.ts` joins the issue's
read-only consumers alongside `webhook.ts` and the delivery-ID test.

## 4. The first test

`test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance`
in `pkgs/fleet/crates/fleet-queue/tests/pmo_contract_conformance.rs`,
driven by `fixtures/pmo-conformance/acceptance/`:

1. A signed `issue_comment` delivery carrying `/fleet status` is
   mapped to the forwarded envelope. Fleet acceptance is keyed by the
   semantic key, never by the delivery ID; the stored effect carries
   the delivery ID as provenance only.
2. The same comment redelivered under a new delivery ID is a
   duplicate on the Fleet side (`duplicate: true`, one effect), even
   though projection application would treat it as a new delivery.
3. A delivery recorded as projection-applied with no forwarded
   envelope yields no Fleet receipt: a projection ACK alone is never
   acceptance.
4. A forwarded envelope refused by authorization leaves the ledger
   unchanged, and nothing in the fixture marks the projection as
   failed: the two are independent.
5. The cross-language check of §3.3 passes.

Further named tests, one per family and negative:

| Test                                                              | Family / negative                                                                                                                      |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `test_adapter_conformance_command_envelope_round_trips`           | `command`: parse, validate, digest and serialize match the golden bytes                                                                |
| `test_adapter_conformance_command_rejects_client_authorization`   | `command`: an `authorized` field is `client_supplied_authorization`                                                                    |
| `test_adapter_conformance_command_unknown_major_version_typed`    | `command`: unknown schema version is typed, never best-effort parsed                                                                   |
| `test_adapter_conformance_selector_profile_golden`                | `selector`: profile validation and comparison results match the fixtures                                                               |
| `test_adapter_conformance_authority_managed_section_golden`       | `authority.managed_section`: capability decisions match the fixtures                                                                   |
| `test_adapter_conformance_projector_key_template_matches_fixture` | `acceptance`: §3.3 cross-language check, separately named so a drift is obvious                                                        |
| `test_adapter_conformance_manifest_matches_targets`               | every `landed` family names tests that exist in the target (`-- --list`); every `gated` family names an issue and has no tests counted |

## 5. Keeping gated families honest

- A `gated` family contributes no tests and is reported as gated,
  with its issue, in the run summary. It is never shown as passing
  and never as failing.
- Moving a family to `landed` is a fixture-and-manifest change in the
  same PR that lands its implementation. The manifest test then
  requires its named tests to exist.
- Conformance is claimed per family and only from a green nightly
  run whose head contains the change (the policy's post-merge
  evidence rule). A failed or cancelled run means no family may be
  claimed for that head; a red `acceptance` family blocks any claim
  that projector forwarding is safe to attach to a sink.

## 6. CI adoption

### 6.1 Venue

One new workflow, `.github/workflows/pmo-contract-conformance.yml`,
which the issue owns:

- triggers `schedule` (the policy's 07:17 UTC nightly) and
  `workflow_dispatch` only; no push or pull-request trigger;
- `runs-on: ubuntu-latest` (GitHub-hosted);
- registered in the same change in the ci-policy agreement registry
  ("nightly schedule", manual "yes") and in
  `.github/test-suite-registry.json` as suite
  `pmo.contract-conformance`, so the agreement checker and the
  registry both know it.

Folding the steps into `fleet-rust-tests.yml` was rejected: that job
runs the whole workspace under default feature unification and
reports one result, while conformance needs its own pass/fail per
family and the exact feature set the issue names.

### 6.2 Steps

1. `rustup toolchain install 1.99.1 --profile minimal` (the pin
   `fleet-rust-tests.yml` uses).
2. List, then assert: run
   `cargo +1.99.1 test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --features client,queue-broker --test pmo_contract_conformance -- --list`
   and fail if any test the manifest names for a landed family is
   missing. A filter that matches zero tests is a failure.
3. Run the target:
   `cargo +1.99.1 test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --features client,queue-broker --locked --test pmo_contract_conformance`.
4. Projector side, by exact file:
   `cd pm-stack/projector && bun install --frozen-lockfile && bun test test/webhook-delivery-id.test.ts test/webhook-operation-forwarding.test.ts`.
5. Write a per-family table (landed and passing, landed and failing,
   gated with issue) to the job summary. The job fails if any landed
   family fails.

The job writes nothing outside its runner: no secrets, no deploy, no
GitHub write, no broker or projector endpoint.

### 6.3 Rollback

Removing the workflow file and its two registry rows stops the
signal. The test target and fixtures stay in the tree and still run
under `fleet-rust-tests.yml`'s workspace run. No live state exists to
roll back.

## 7. Negatives, restated as checks

| Negative from the issue                                       | Check                                                                                                                                                      |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CI failure prevents claiming conformance                      | §5: claims come only from a green nightly run; the summary table is the only place a family is called conforming                                           |
| Reverting the CI hook rolls back no live state                | §6.3: the job has no live effect to roll back                                                                                                              |
| Never silently point projector writes at Fleet                | the conformance target configures no sink in the projector and asserts the default wiring stays inert; attaching a sink is #9028 and DSP-01 follow-up work |
| Never treat projection ACK as command acceptance              | §4 steps 2 and 3                                                                                                                                           |
| Unexpected upstream or provider behavior is typed or degraded | every negative fixture names its typed error; an untyped panic or a string match fails the family                                                          |

## 8. Gates and sequencing

- **Hard blockers named by the issue:** PRI-01 (closed, landed), REL-03
  (record merged, implementation gated), HYD-06 (record merged,
  implementation gated), AUTH-01 (#9429, open). The landed families
  in §2 can be built now; the gated families join as their slices
  land (§5). Earlier worker notes on this issue held all of CEN-07 on
  the open trackers #9200, #9207, #9208 and #9209; this record splits
  the work by family so the landed families need not wait.
- **Runtime gate:** #9028 (the PM PostgREST producer bypass) blocks
  any live cutover claim, not offline fixtures.
- **Ownership:** the workflow file and the Rust test target are
  CEN-07's; fixtures under the target's directory are CEN-07's;
  projector files are read-only; `contracts.rs`, `operations.rs`,
  `priority.rs` and `authority.rs` are not edited by this slice.
- **Policy:** the workflow and registry rows land together, and the
  agreement checker must pass in the same PR.

## 9. Requirement trace (design-level)

| ID     | Where addressed                                                                        |
| ------ | -------------------------------------------------------------------------------------- |
| C-R160 | §3, §4: shared golden fixtures run through the code that serves each contract          |
| C-R162 | §2, §2.1: current map of landed and gated families and the two live mismatches         |
| H-R046 | §5, §6: a named CI venue, per-family claims and rollback                               |
| P-R027 | §4, §7: acceptance against realistic end-to-end contract cases, mocks only at the edge |

Full per-occurrence text is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`.

## 10. Non-goals

- No fix for C1 or C2; they belong to REL-03 and DSP-02.
- No sink attached to the projector, no live forwarding, no #6533 run.
- No push or pull-request trigger, no self-hosted runner, no new
  service, backend, queue or database.
- No receipt, cursor or field-ownership fixtures until their slices
  land.
