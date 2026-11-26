# REL-08 — Direct `fleet work complete` CLI and human/JSON recovery output

Issue: [#9518](https://github.com/developing-today/code/issues/9518)
(slice `REL-08` of [#9209](https://github.com/developing-today/code/issues/9209),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the REL-08 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225): "add `fleet work complete` parser contract with explicit
refs, `--wait/--verify`, idempotency and shared structured outcome",
first oracle `pmo_cli_complete_requires_explicit_remote_resource_allowlist`,
negatives "ambiguous repo, implicit close-all, timeout, unauthorized
merge, changed digest". REL-07 decided the completion operation, its
effects and its recovery. This record decides the command line that
starts or resumes that operation, what each allocated flag means, and
how the result reads in human and JSON form. It authorizes no code,
registration, remote write or deployment (§9).

Governing decisions consumed, not re-opened:

- **REL-07** (#9517, `doc/2026-11-25T22-26-00Z_design_rel07_completion_state_machine/`,
  merged in #15290): effects `merge_pr`, `close_issue`,
  `finalize_task`; explicit targets that must also be active links of
  matching kind (`completion_ref_not_linked`); per-effect and derived
  operation states; recovery reads back before any resend; finalize
  reuses the `/complete` gates; "REL-08 renders it".
- **REL-03** (#9513, `doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`):
  scope `(principal_ref, authority_id, idempotency_key)`, request
  digest, stored receipt, and the §5.2 receipt-status → outcome
  mapping. Its trace leaves the `--idempotency-key` flag to
  CLI-02/CLI-13.
- **REL-04** (#9514, `doc/2026-11-25T22-30-00Z_design_rel04_lifecycle_mutation_outbox/`):
  operation statuses and the attempt log the `sync` façades read.
- **REL-06** (#9516, `doc/2026-11-23T12-20-33Z_design_rel06-github-preconditions-readback/`):
  head-SHA precondition and effect-specific read-back.
- **REL-09** (`work/completion.rs` on main): `evaluate_completion`,
  `merge_allowed`, the disclosed residual race.
- **CLI-02** (#9441, `doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`):
  strict parse before any service opens; typed locators only, a bare
  number is rejected; `--lease-token` never echoed; usage error exits
  2; the outcome lives in the envelope (`rejected`, `accepted`,
  `running`, `pending_sync`, `partial`, `conflict`, `uncertain`,
  `verified_complete`); human and JSON render the same facts.
- **CLI-12** (`doc/2026-11-25T22-18-00Z_design_cli12_work_link_semantics/`):
  `ResourceRef` refusal codes (`link_target_ambiguous`, …) and the
  link kinds `pr_for` and `implements`.
- **CLI-13** (#9442): the single owner of `main.rs` registration and
  shared CLI glue. REL-08 registers nothing.

## Verified state on main

Read at `origin/main` `25fbfab8d` on 2026-11-25:

- **No `fleet work` namespace.** `pkgs/fleet/src/main.rs` has no
  `work` subcommand, and `census/commands.jsonl` lists `work.complete`
  as `absent`.
- **`repo-queue complete`** (`fleet-queue/src/client.rs` dispatch
  `"complete" => cmd_complete`) is the queue-task transition: it calls
  `/complete`, which checks `claimed_by`, `gate_completion` and, with a
  disposition, `disposition::require_verified`, then closes the task.
  Its options are `<task-id> [--worker] [--lane] [--result]
[--result-file] [--telemetry] [--execution] [--resolution]
[--substatus] [--reason] [--ref]`. It makes no GitHub call.
- **`repo-queue stack record|replay`** (`client/fallback_stack.rs`
  `cmd_stack`) records a merge intent (`merge_intent.rs`) and replays
  it, including merge and land actions. The census keeps it distinct
  from lifecycle completion.
- **`repo-queue outbox`** has `enqueue`, `list`, `metrics`, `claim`,
  `resolve` and `drain`, serving the scoped `creation_outbox`. There
  is no `inspect`, `retry`, or `sync` namespace.
- **`work/completion.rs`** holds REL-09 only. `work/contracts.rs`
  holds `WorkRef`, `Outcome`, `Effect`, `Command` and `ReasonCode`.
- **Test targets** in `fleet-queue/tests/` are `broker_bin_args`,
  `chunked_list`, `link_cli` and `person_wait_cli`. None covers
  completion.

## 1. Intent

An agent finishing a piece of work types one command that names the
PR to merge and the issues to close. It gets back, in a form a person
can read and a program can parse, which of those happened, which are
still settling, and the exact command to run next. Running the same
command again after a crash or a lost answer resumes the same
operation and never merges or closes twice.

## 2. Ownership

| Concern                                          | Owner                                | This slice                       |
| ------------------------------------------------ | ------------------------------------ | -------------------------------- |
| `fleet work complete` argv contract and outcome  | `work/completion.rs`                 | **owns** (parse model, renderer) |
| Completion operation, effects, recovery          | REL-07, same file                    | consumer                         |
| Strict grammar, shared flags, usage exit code    | CLI-02                               | consumer                         |
| Subcommand registration, `main.rs`, help routing | CLI-13                               | not here                         |
| Idempotency scope, digest, receipt               | REL-03                               | consumer                         |
| Operation status, attempt log                    | REL-04, REL-05                       | consumer (`sync` façades read)   |
| Preconditions and read-back calls                | REL-06                               | consumer                         |
| Merge and close authorization                    | AUTH-01, DSP-08                      | consumer                         |
| `/fleet merge` slash grammar                     | CLI-02 §5 (grammar), #9203 (routing) | maps onto §3 only                |

## 3. The command line

```text
fleet work complete --work <ref>
    [--merge-pr <pr-ref> [--merge-method merge|squash|rebase] [--expected-head <sha>]]
    [--close-issues <issue-ref>[,<issue-ref>...]]
    [--close-queue | --no-close-queue]
    [--merge-linked-prs] [--close-linked-issues]          (dry-run only in v1, §3.2)
    [--require-clean-reconciliation]
    [--expected-version <n>] [--lease-token <opaque>]
    [--idempotency-key <key>]
    [--wait] [--timeout <duration>] [--verify]
    [--dry-run] [--json]
```

### 3.1 Flags and their REL-07 meaning

| Flag (census key)                      | Meaning                                                                                        | Maps to                             |
| -------------------------------------- | ---------------------------------------------------------------------------------------------- | ----------------------------------- |
| `--work`                               | the work being completed; typed locator per CLI-02                                             | operation subject                   |
| `--merge-pr` (`flag.merge-pr`)         | one PR, `owner/repo#N` or `--repo` plus number                                                 | one `merge_pr` effect               |
| `--merge-method`                       | defaults to the repo's single allowed method; refused if more than one is allowed and none set | effect precondition                 |
| `--expected-head`                      | the SHA the caller tested; defaults to the head read at prepare and is shown in the receipt    | REL-06 head precondition            |
| `--close-issues` (`flag.close-issues`) | listed issues, each its own effect, closed in the given order                                  | `close_issue` effects               |
| `--close-queue` (`flag.close-queue`)   | default on; `--no-close-queue` leaves the task `completing` after remote effects confirm       | `finalize_task` required or skipped |
| `--require-clean-reconciliation`       | refuse at prepare if the work's sync cursor or outbox has unresolved rows                      | prepare guard                       |
| `--expected-version`                   | optimistic version of the work; mismatch → `conflict` before any write                         | prepare guard                       |
| `--lease-token`                        | opaque credential for the claim; never echoed, logged or digested                              | fence check (REL-01/02)             |
| `--idempotency-key`                    | REL-03 key; generated and printed when absent so the caller can resume                         | operation identity                  |
| `--wait` / `--timeout` / `--verify`    | §4                                                                                             | deadline and read-back policy       |
| `--dry-run` (`flag.dry-run`)           | run parse, resolve, authorize and prepare checks; write nothing; print the plan                | no operation created                |
| `--json`                               | CLI-02 output family                                                                           | renderer                            |

`--work` is required. At least one of `--merge-pr`, `--close-issues`
or `--close-queue` must yield an effect; a command that would do
nothing is a usage error.

### 3.2 Explicit refs only

1. **Every remote target is typed in the command.** A bare number is
   a usage error (CLI-02). A ref that resolves to more than one item
   is refused with `link_target_ambiguous`, naming the candidates; the
   ambiguous-repo negative is this case (`--close-issues 42` where the
   work is linked to issues in two repos).
2. **Every named target must be an active link** of the work of the
   matching kind (`pr_for` for `--merge-pr`, `implements` for
   `--close-issues`), else `completion_ref_not_linked` (REL-07 §3).
3. **`--merge-linked-prs` and `--close-linked-issues`** stay in the
   inventory but, in v1, are accepted only with `--dry-run`. There
   they expand the work's active links of the matching kind into the
   explicit list and print the exact `--merge-pr` / `--close-issues`
   arguments to run. Without `--dry-run` they are refused with
   `completion_implicit_refs` and the same printed list. This is the
   implicit close-all negative: no run ever closes something the
   caller did not type. Lifting this needs an owner decision and its
   own test row.
4. **Parents are never targets.** An epic, component or design parent
   linked as `parent` or `related` is never offered by expansion.

### 3.3 Relation to existing commands

- `repo-queue complete` stays exactly as it is (task transition, no
  GitHub). `fleet work complete --no-close-queue` is never a synonym
  for it, and `queue.close` stays a distinct, still-absent spelling.
- `repo-queue stack record|replay` stays the merge-intent tool for
  stacked PRs. `fleet work complete` does not call it; a stack's
  merges are separate operations.
- `/fleet merge` (`agent.slash.merge`) maps onto
  `fleet work complete --merge-pr <the event's PR> --no-close-queue`
  with the same refusal rules. Its routing and authorization stay
  with CLI-02 §5 and #9203.
- `fleet github pr merge` and `fleet github issue close` stay absent;
  their verified single-effect forms are future façades over the same
  REL-07 effects and are not introduced here.

## 4. Waiting, deadlines and verification

| Flags                         | Behaviour                                                                                                                                                     |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| none                          | run phases until the first effect that is not immediately `confirmed`; return its state                                                                       |
| `--wait`                      | keep polling `pending`/`unknown` effects through read-back until all required effects settle or the deadline passes                                           |
| `--timeout <d>`               | the command's deadline (default 120 s with `--wait`, 30 s without); on deadline, return the current receipt with outcome `pending_sync` or `uncertain`        |
| `--verify`                    | after a send that already answered success, still read back each effect before reporting `confirmed`; default on for `merge_pr`, may be set for `close_issue` |
| `--verify` with no new effect | resume mode: read back every open effect of the existing operation and finalize if REL-07 §5.6 holds; sends nothing new                                       |

Rules:

1. **A deadline never cancels.** Reaching `--timeout` returns the
   operation as it stands; nothing is rolled back, and the next call
   with the same key resumes it.
2. **No risky merge to beat a deadline** (C-R154, P-R040). The merge
   send is not started when less than a merge's read-back budget
   (proposed: 20 s) remains; the command returns `pending_sync` with
   next action "rerun with the same key".
3. **Accepted is not merged.** A 202 or a queued merge is reported as
   `pending`, never `merged`, in both forms.
4. **No webhook wait.** Success needs only the command's own
   read-back (C-R146).

## 5. Idempotency and the changed-digest negative

- The intent digest is CLI-02 §6's: `work`, the effect targets in
  order, `--merge-method`, `--expected-head`, `--close-queue` and
  `--require-clean-reconciliation` go into `arguments`;
  `--expected-version` is `expected_work_version`. CLI-02 already
  excludes output format, `--wait`, `--timeout` and `--dry-run`.
  This record proposes adding `--verify` to that list, since it
  changes how much read-back the command does, never which effects
  are intended. `--lease-token` is a credential, not an argument; only
  its `lease_generation` is in the DTO.
- Same key, same digest: acceptance replays the stored receipt and
  creates nothing (REL-03 §4). If the operation is not `completed`,
  the command then runs REL-07 §6 recovery for that operation, which
  reads back before any send. Changing only `--wait` or `--timeout`
  is therefore a resume, not a new request.
- Same key, different digest (another PR, an added issue, a new
  head): REL-03's `conflict` with `IDEMPOTENCY_KEY_REUSED`, the stored
  `operation_id` and the next action "use a new idempotency key".
  The stored targets are not echoed; `fleet sync outbox inspect`
  shows them to a caller with read authority (§7).
- `--dry-run` reports REL-03's `would_accept`, `would_replay` or
  `would_conflict` alongside the plan.
- No key given: the command generates a random key on the caller's
  side (REL-03 §3.3: never from a server time), writes it into the
  receipt and prints it first, so an interrupted terminal still has
  it.
- A second operation for a work that already has an open completion
  operation is refused with `completion_in_progress`, naming that
  operation's `operation_id`.

## 6. Output

Both forms carry the same facts (CLI-02 §7). The JSON form is the
`fleet.result/v1` envelope with `outcome` from REL-03 §5.2 and a
`completion` block:

```json
{
  "schema": "fleet.result/v1",
  "outcome": "partial",
  "operation_id": "op_…",
  "idempotency_key": "…",
  "work": "fleet:…",
  "completion": {
    "operation_status": "partial",
    "effects": [
      {
        "kind": "merge_pr",
        "target": "owner/repo#123",
        "status": "confirmed",
        "expected_head": "abc123…",
        "merge_commit": "def456…",
        "method": "squash",
        "verified_at": "…"
      },
      {
        "kind": "close_issue",
        "target": "owner/repo#120",
        "status": "failed_terminal",
        "reason_code": "close_forbidden"
      },
      { "kind": "finalize_task", "target": "fleet:…", "status": "planned" }
    ],
    "late_activity": [],
    "residual_race": "activity after <watermark> is not seen"
  },
  "next_action": {
    "command": "fleet work complete --idempotency-key … --verify",
    "why": "…"
  }
}
```

Human form, one line per effect, the outcome first:

```text
partial: merged owner/repo#123 (squash def456), could not close owner/repo#120 (close_forbidden)
  key: …   work stays completing
  next: fleet work complete --idempotency-key … --verify
```

Rules: never print `--lease-token`; print every effect including
`skipped` ones; `next_action` is always present unless the outcome is
`verified_complete` or `rejected` at parse.

**Exit status.** 2 for usage errors (CLI-02). For a reached seam, 0
only for `verified_complete` and for a successful `--dry-run`; a
distinct non-zero for each of `partial`, `pending_sync`, `uncertain`
and `conflict`, so scripts never mistake a partial completion for
success. The numeric values belong to the exit-code matrix that
CLI-02 leaves to its implementation issue; this record fixes only
that none of these four is 0.

## 7. `sync` read and retry façades

The allocated `sync.*` rows are thin, read-mostly views over REL-04
and REL-05 state. They are listed here so the command map is
complete; they create no new store or relay.

| Command                             | Behaviour                                                                                           |
| ----------------------------------- | --------------------------------------------------------------------------------------------------- |
| `fleet sync status [--work]`        | counts per operation status, independent of queue state                                             |
| `fleet sync outbox list [--status]` | operations with status, target and age                                                              |
| `fleet sync outbox inspect <op>`    | one operation: effects, attempts in order, receipts; read-only                                      |
| `fleet sync outbox retry <op>`      | re-enter REL-07 §6 recovery for that operation; reads back first, never sends an unproven effect    |
| `fleet sync verify [--work]`        | read back every `confirmed` claim and report any that the remote no longer matches; changes nothing |
| `fleet sync run [--limit]`          | bounded drain of planned effects by the REL-05 relay; same ordering and identity rules              |

`repo-queue outbox` keeps its current subcommands for the scoped
creation outbox; these façades do not alias them.

## 8. Tests

All tests live in a new integration target owned by this slice,
`fleet-queue/tests/work_complete_cli.rs`, listed by exact name with
`-- --list` before running. Queue crate command tests cover only the
service composition, not this contract.

| Test                                                           | Asserts                                                                                    |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `pmo_cli_complete_requires_explicit_remote_resource_allowlist` | a command with no typed target, or a bare number, is refused before any service opens      |
| `pmo_cli_complete_refuses_ambiguous_repo`                      | `--close-issues 42` with links in two repos → `link_target_ambiguous` with both candidates |
| `pmo_cli_complete_refuses_unlinked_target`                     | a typed but unlinked PR → `completion_ref_not_linked`                                      |
| `pmo_cli_complete_refuses_implicit_close_all`                  | `--close-linked-issues` without `--dry-run` → `completion_implicit_refs`, nothing written  |
| `pmo_cli_complete_dry_run_expands_linked_refs_without_writes`  | dry-run prints the explicit argument list; operation count unchanged                       |
| `pmo_cli_complete_refuses_unauthorized_merge`                  | principal without merge authority → `rejected`, no effect row                              |
| `pmo_cli_complete_reports_partial_receipt`                     | merge confirmed, close failed → outcome `partial`, non-zero exit, `next_action` present    |
| `pmo_cli_complete_timeout_returns_resumable_pending`           | deadline during a queued merge → `pending_sync`, same key resumes, no second merge send    |
| `pmo_cli_complete_does_not_start_merge_near_deadline`          | less than the read-back budget left → no merge send, `pending_sync`                        |
| `pmo_cli_complete_changed_digest_refused`                      | same key, added issue → `IDEMPOTENCY_KEY_REUSED`, stored `operation_id`, no targets echoed |
| `pmo_cli_complete_same_key_resumes_with_new_wait_flags`        | same key, different `--timeout` → resume, not a new operation                              |
| `pmo_cli_complete_never_echoes_lease_token`                    | token absent from human, JSON, error and log output                                        |
| `pmo_cli_complete_human_and_json_render_same_facts`            | golden pair: every effect, status and next action appear in both                           |

Done command (remote runner):

```sh
cargo test -p fleet-queue --test work_complete_cli -- --list
cargo test -p fleet-queue --test work_complete_cli
```

## 9. Gates and persisted fields

- **No new persisted field.** The receipt fields are REL-03's and the
  operation and effect rows are REL-04/REL-07's. This record adds
  only reason codes: `completion_implicit_refs`,
  `completion_in_progress`, each proposed for the `ReasonCode` registry owner.
- **Registration** of `fleet work complete` and `fleet sync …` waits
  for CLI-13 (#9442). Implementation waits for REL-07's pure model and
  REL-03/REL-04 storage.
- **Owner decisions asked:** the deadline floor for starting a merge
  (proposed 20 s); whether `--close-queue` defaults on; whether linked
  expansion may ever run outside `--dry-run`.
- **Rollback:** disable the subcommand; operations and receipts stay;
  `repo-queue complete` is untouched throughout.

## 10. Requirement trace (design-level)

| IDs                                                                                            | Outcome                                                                             | Where          |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------- |
| C-R007, C-R143, C-R144, C-R145, O-R025, O-R034, P-R022, P-R033                                 | composed, validated direct completion of authorized merge, close and queue close    | §3, §3.2       |
| C-R146, C-R147                                                                                 | no webhook wait on success; webhook and sweep stay for external changes and crashes | §4 rule 4, §7  |
| C-R148, P-R038, H-R015                                                                         | partial completion reported precisely; work stays `completing`                      | §6             |
| C-R140, C-R141, O-R032, P-R021                                                                 | stable operation ids and keys; verify before retry                                  | §5, §7         |
| C-R154, P-R040, O-R052                                                                         | no risky merge to beat a deadline                                                   | §4 rule 2      |
| C-R005, C-R107, C-R108, C-R087, H-R017, H-R024, H-R031, O-R023, P-R009, P-R025                 | cohesive CLI; full inventory and existing commands preserved                        | §3.1, §3.3, §7 |
| O-R027, P-R007                                                                                 | lease bound to the caller; execution state distinct from GitHub state               | §3.1, §3.3     |
| H-R016, O-R035, P-R023, P-R036, O-R050, C-R150, C-R151                                         | crash after remote mutation, lost acknowledgement, resume scenarios                 | §4, §5, §8     |
| C-R009, C-R110, H-R001, O-R022, O-R043, P-R001, P-R011, P-R027, P-R029, P-R032, P-R054, P-R069 | the completion step of the end-to-end work flow and its acceptance                  | §1, §8         |

## 11. Non-goals

- No registration, `main.rs` change, help routing or alias approval
  (CLI-13, CLI-02).
- No change to `repo-queue complete`, `repo-queue stack` or
  `repo-queue outbox`.
- No new effect kind, state, transition or recovery rule (REL-07).
- No storage, relay or ordering change (REL-03, REL-04, REL-05).
- No `fleet github` namespace.
- No implicit linked-ref run in v1.
