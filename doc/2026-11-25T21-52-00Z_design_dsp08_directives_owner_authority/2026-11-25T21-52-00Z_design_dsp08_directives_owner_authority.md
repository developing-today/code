# DSP-08 — Directives, owner authority and reauthorization rules

Issue: [#9448](https://github.com/developing-today/code/issues/9448)
(slice `DSP-08` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-08 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row asks for "one-shot `do-now`, parked-direct-start,
severity/action permissions, live-owner rules", a policy table that
"covers allowed/denied/confirmation/triage" with every decision
receipted, and warns: "Do not invent privilege names; reconcile
actual Access roles and fleet owner policy first." This record does
that reconciliation against main, then _decides_ the table, the
directive's life, the parked start, the live-owner rules and when
authority is checked again. It authorizes no code, schema, role
change, deployment or live dispatch; those remain separately gated
stages (see §10).

Governing decisions consumed, not re-opened:

- **Component #9203 §1 and §3.6**: `do-now` "bypasses waiting behind
  ordinary selection only; it cannot bypass actor authorization, hard
  dependencies, task eligibility, existing lease, applicable
  concurrency cap, provider gate, or owner/manual-only policy. It
  never rewrites numeric urgency or priority label." The directive
  moves `requested → reserved → consumed | expired | rejected`.
  `manual_only` is an eligibility gate; an explicit owner direct
  start may use #9178's manual-claim mode without clearing it.
- **Component #9203 §3.2**: "Recheck actor permission per operation
  and target at intake, then reauthorize at effect time (roles/repo
  access may change during a queued request). Require additional
  confirmation for merge/close/stop/priority/direct start according
  to policy." `/fleet merge` "always requires merge authority,
  current clean preview, checks/reviews/head-SHA readback and
  explicit confirmation; command syntax alone is not consent."
- **DSP-01** (`work/operations.rs`): the `Verb` set (`status`,
  `triage`, `start`, `resume`, `retry`, `park`, `unblock`,
  `priority`, `do-now`, `merge`, `agent status`), `Grant { principal,
verbs, repos }`, `AuthorizationPolicy::authorize` with refusals
  `Unauthenticated`, `NoGrant`, `VerbDenied`, `ResourceDenied`, each
  non-mutating.
- **DSP-04 record**: the decision table routes to `send_to_session`,
  `direct_start`, `resume` or `blocked_answer`; no row writes
  priority; the decision is re-read at effect time.
- **DSP-06 record §4**: gate order at reservation, `do_now` consumed
  once, a replay does not consume it again.
- **O-R046**: historical design urgency 450/High stays distinct from
  one-shot Do Now.

## 1. Intent

Who may do what is decided by the roles and grants that already
exist, intersected, never by a new privilege name. `do-now` changes
one thing, the directive, and nothing that decides eligibility.
Every permission decision is written into the operation's receipt
with the policy version and the rule that fired, and authority is
checked again before each external effect, so a revocation between
request and effect stops the operation.

## 2. The authorities that exist on main

Reconciled from source, not proposed:

| Authority                       | Where                                                                                          | Values                                               | Proves                                              |
| ------------------------------- | ---------------------------------------------------------------------------------------------- | ---------------------------------------------------- | --------------------------------------------------- |
| Fleet grant                     | `work/operations.rs` (DSP-01)                                                                  | principal × verbs × repos (`*` allowed)              | the principal may run this verb on this repository  |
| PM Stack identity registry role | `pm-stack/aggregator/src/stack-token.ts`, `server-write-routes.ts` (#9604)                     | `owner`, `orchestrator`, `agent`, `read-only`        | what a PM Stack identity may mint write tokens for  |
| Verified Access owner           | `enforceAccessAuthor` in `server-write-routes.ts`; break-glass (#9608) for registry admin only | owner or not                                         | the human owner, by Cloudflare Access               |
| GitHub repository permission    | provider fact for the comment author                                                           | `admin`, `maintain`, `write`, `triage`, `read`, none | what GitHub lets that account do on that repository |
| Live holder                     | queue lease holder and lane; `BindingView` (DSP-05)                                            | worker, lane, session                                | who is working the task now                         |

Two rules follow, and they are the whole of "no invented names":

1. **The Fleet grant is necessary for every verb, from every source.**
   No role, permission or holder status substitutes for a missing
   grant.
2. **A source floor is an extra condition, never a grant.** Each
   source adds the floor its own identity system already has: a PM
   Stack action needs the registry role named in §3; a GitHub comment
   needs the author's repository permission named in §3; a CLI or
   service call has only its grant. A floor that is met grants
   nothing by itself.

`read-only` in the registry and `read`/none on GitHub can run only
read verbs, whatever their grants say.

## 3. Policy table

Severity classes: **read** changes nothing; **advisory** asks for
triage; **work** reserves, starts, parks or releases; **value**
changes stored priority; **irreversible** cannot be undone.

| Verb                                 | Severity     | PM Stack role floor                       | GitHub permission floor | Confirmation                                            | Otherwise                                 |
| ------------------------------------ | ------------ | ----------------------------------------- | ----------------------- | ------------------------------------------------------- | ----------------------------------------- |
| `status`, `agent status`             | read         | any but none                              | `read`                  | no                                                      | —                                         |
| `triage`                             | advisory     | `agent`                                   | `triage`                | no                                                      | —                                         |
| `start`                              | work         | `orchestrator`                            | `write`                 | only when the target is parked (§5) or manual-only (§5) | `blocked_answer` per DSP-04               |
| `resume`                             | work         | `orchestrator`                            | `write`                 | yes (§5)                                                | `blocked_answer`                          |
| `retry`                              | work         | `orchestrator`                            | `write`                 | no; #9178 decides whether retry is allowed              | `blocked_answer`                          |
| `do-now`                             | work         | `orchestrator`                            | `write`                 | no                                                      | `blocked_answer`; never a priority change |
| `park`                               | work         | `orchestrator`, or the live holder itself | `write`                 | no; a reason is required (#9179)                        | `rejected`                                |
| `unblock`                            | work         | `orchestrator`                            | `write`                 | no; dependency owner rechecks (DEP)                     | `rejected`                                |
| `priority`                           | value        | `orchestrator`                            | `maintain`              | per #9200 / PRI-01                                      | routed to PRI-01                          |
| `merge`                              | irreversible | `owner`                                   | `maintain`              | always (§8)                                             | `rejected`                                |
| manual-claim override (with `start`) | work         | `owner`                                   | `admin`                 | yes                                                     | `blocked_answer`, reason `manual_only`    |
| takeover of a live holder (#9178)    | work         | `owner`                                   | `admin`                 | yes                                                     | `conflict`, holder named                  |

The floors use only the names in §2. The table is data in
`operations.rs` keyed by verb and source, with a version; changing a
floor is a policy change with its own receipt, not code.

Every decision produces one of four results, each receipted (§9):
`allowed`, `denied` (with the DSP-01 refusal or `floor_not_met`),
`confirmation_required` (with the preview to confirm), or
`triage` (the request is real but cannot be acted on automatically,
for example low-confidence plain language).

## 4. The `do-now` directive

```text
Directive {
  kind:          "do_now",
  work:          WorkRef,
  operation_id:  the accepted operation that created it,
  requested_by:  principal,
  state:         requested | reserved | consumed | expired | rejected,
  expires_at:    requested_at + the policy's directive lifetime,
  reason:        RouteReason or registry code when rejected or expired,
}
```

- One live directive per work. A second `do-now` for the same work
  while one is live returns the existing directive's receipt; a
  redelivery of the same event references the same directive.
- `requested → reserved` when DSP-06 takes the reservation;
  `reserved → consumed` when the launch reaches `task_bound`;
  `→ expired` when `expires_at` passes before reservation;
  `→ rejected` when a gate refuses.
- **What it changes:** which DSP-04 row is chosen (send now, or start
  now instead of waiting for selection). **What it never changes:**
  priority value, priority label, the ratified `expedite=15` /
  `next-up=25` bands, `manual_only`, park state, hard blockers, lease,
  capacity caps, grants or floors. The directive is stored with the
  operation, not on the task's priority fields.
- The directive lifetime is a policy value with no default invented
  here; until the owner sets one, a directive that cannot reserve at
  once is `rejected` with the gate named, never left waiting.

## 5. Parked and manual-only direct start

- **Parked.** Starting parked work is #9179's explicit resume, never
  an unpark around it. It needs the `resume` floor and a confirmation
  showing the park reason, the checkpoint and the next action the park
  recorded. The resume claim does not consume an attempt (broker
  behaviour, kept). The park reason and checkpoint travel into the
  launch receipt.
- **Manual-only.** `manual_only` stays set. Only the manual-claim
  override row of §3 can start such work, through #9178's manual-claim
  mode, with the override, actor, session and reason recorded.
  `do-now` alone never supplies that authority.
- **Hard blockers** are not overridden by any role. No row in §3,
  including `owner`, turns a hard-ineligible task eligible.

## 6. Live-owner rules

A task with a live holder (lease) or a live capable bound session:

| Request                                     | Rule                                                                                          |
| ------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `start`, `retry`, `do-now`, actionable text | routed to the session (DSP-04 row 6); never a second session, never a lease steal             |
| `status`, `triage`                          | allowed concurrently; read only                                                               |
| `park`                                      | by the holder itself, or with the `park` floor plus a reason; the holder's checkpoint is kept |
| takeover                                    | only #9178's explicit operation, with its §3 row; never a side effect of start or do-now      |
| `merge` on the PR the session is working    | allowed with §8's checks; the session is told, not stopped                                    |

## 7. Reauthorization

Authority is read, not cached. It is checked at these points, each
against the grants, floors and policy version in force at that
moment:

1. **Intake** (DSP-04 row 4), before any candidate or receipt is
   disclosed.
2. **Reservation** (DSP-06 step 3).
3. **Before each external effect** that a later stage takes: session
   create, prompt send, and the merge call.
4. **At confirmation**: a confirmation is accepted only if the
   confirming principal passes the same row at that moment.

A grant revoked, a role lowered, a repository permission removed or
a policy version advanced between two points stops the operation
before its next effect with `rejected`, reason
`authorization_revoked`, and names the point. Effects already taken
stay and are reported (`partial`, DSP-09 I8–I9); nothing is undone
to "clean up" a revocation.

## 8. Merge

`merge` is allowed only when, at confirmation **and** again
immediately before the merge call: the principal passes the `merge`
row; the PR head SHA equals the SHA shown in the preview; required
checks are green on that SHA; required reviews are in place; the
PR is mergeable. A head change voids the confirmation. The preview's
digest is what the confirmation names; a confirmation for another
digest is refused.

## 9. Receipts

Each decision adds a `policy_decision` effect to the operation's
result:

```text
policy_decision {
  result:          allowed | denied | confirmation_required | triage,
  verb, source,
  policy_version,
  rule:            the §3 row (verb, source),
  grant:           matched grant id or the DSP-01 refusal,
  floor:           floor required and floor observed, with its read source,
  checkpoint:      intake | reservation | effect:<name> | confirmation,
  decided_at,
}
```

One entry per checkpoint, so a receipt shows the whole chain of
checks, not only the first.

## 10. Persisted fields, acceptance and gates

Persisted fields: the directive (§4) and the policy table version
(§3), both inside the operation record that REL-03/REL-04 own. No
new column on the task row; `manual_only`, priority and park fields
are untouched.

Tests (first red is the slice's starting test):

| Test                                                        | Proves                                                                           |
| ----------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `pmo_dsp_08_do_now_never_bypasses_hard_eligibility` (first) | property: for every generated task, do-now leaves the hard-eligibility set equal |
| `pmo_dsp_08_do_now_changes_directive_only`                  | no priority, label, band, park or `manual_only` effect from do-now               |
| `pmo_dsp_08_no_role_makes_hard_ineligible_eligible`         | property over every §3 row, `owner` included                                     |
| `pmo_dsp_08_grant_is_necessary_floor_is_not_sufficient`     | floor met without grant is denied; grant without floor is denied                 |
| `pmo_dsp_08_read_only_runs_reads_only`                      | registry `read-only` and GitHub `read` reach only read verbs                     |
| `pmo_dsp_08_revoked_role_denied_at_next_checkpoint`         | revocation between intake and effect stops with `authorization_revoked`          |
| `pmo_dsp_08_direct_start_cannot_steal_lease_or_session`     | §6 rows                                                                          |
| `pmo_dsp_08_parked_start_needs_resume_and_confirmation`     | §5 parked                                                                        |
| `pmo_dsp_08_manual_only_needs_owner_override`               | §5 manual-only; flag left set                                                    |
| `pmo_dsp_08_merge_rechecks_head_checks_role`                | §8, including a head change voiding the confirmation                             |
| `pmo_dsp_08_directive_one_live_per_work`                    | §4 duplicate and redelivery                                                      |
| `pmo_dsp_08_every_decision_receipted`                       | §9 entry for every checkpoint and result                                         |
| `pmo_dsp_08_policy_table_is_total`                          | every verb and source has exactly one row                                        |

Command:
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_08_do_now_never_bypasses_hard_eligibility`,
after proving with `-- --list` that the `pmo_dsp_08` tests are listed
(a zero-test filter is not a pass).

Gates:

- **Hard blockers:** PRI-01 (closed on GitHub) and DSP-04 (record on
  main; it is also the shared-file predecessor on `operations.rs`).
- **Runtime gates:** #9178 (manual-claim, takeover, retry semantics)
  and #9179 (park and resume) verified before any mutation path uses
  §5 or §6; offline tables proceed now.
- **Owner decisions this record does not make:** the floor values in
  §3 are proposed defaults over existing names; the owner may change
  any of them by a versioned policy change. The `do-now` lifetime has
  no default (§4).
- **Stages each needing their own evidence and approval:** wiring
  the floors into the projector forwarder and PM Stack actions,
  deployment, and live dispatch.
- **Rollback:** disable floors and directives; grants keep deciding
  as DSP-01 does today. No grant, role or history is removed.

## 11. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                                    | Where                  |
| -------------------------------------- | -------------------------------------------------------------------------------- | ---------------------- |
| C-R005, C-R107, P-R009, H-R017, O-R023 | Cohesive Fleet CLI; names reconciled; no parallel dispatch mechanism             | §2, §3                 |
| C-R009                                 | Agents understand priority and dependencies and claim safely                     | §5, §6                 |
| C-R087                                 | Small authenticated slash grammar                                                | §3                     |
| C-R094                                 | do-now bypasses waiting, not leases, auth, hard dependencies or caps             | §4, §5                 |
| C-R108                                 | Common flags (`--dry-run`, `--explain`, `--expected-version`)                    | §3 preview, §8         |
| C-R152                                 | Scenario C: an authorized live comment routes to the bound session               | §6                     |
| H-R016                                 | Blocked or parked resume, late comment, duplicate webhook, stale lease, conflict | §4–§7                  |
| H-R021                                 | Verify ownership and lease; reuse the active compatible session                  | §6                     |
| H-R024, H-R031                         | Original commands, errors and failures stay in coverage                          | §3, §10                |
| H-R038, H-R051                         | Sanctioned creation paths                                                        | §6 (via DSP-06)        |
| H-R039, H-R052                         | Created ids and records verified; pending intents are not evidence               | §9                     |
| H-R040, H-R053                         | No invented provider ids or silent pool change                                   | §2 (no invented names) |
| O-R046                                 | Historical urgency 450/High stays distinct from one-shot Do Now                  | §4                     |

## 12. Non-goals

- No new role, privilege or scope name; no change to the identity
  registry or Access configuration.
- No priority semantics (PRI-01, #9200) and no park or claim
  semantics (#9179, #9178); this record routes to them.
- No `do-now` lifetime default and no floor value presented as
  owner-ratified.
- No schema change on the task row.
