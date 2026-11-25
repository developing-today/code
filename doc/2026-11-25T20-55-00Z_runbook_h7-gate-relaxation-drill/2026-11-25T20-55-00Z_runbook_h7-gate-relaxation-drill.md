# H7 gate-relaxation drill — runbook and drill log

Issue: #14265 (child of #9086) · Design:
`doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086/` §4.4 + §5
slice H7 · Provider: `pkgs/idp-provider`

## Purpose

Dry-run the §4.4 gate relaxation — add a test enrollment, verify
what the enrolled identity gets, remove it — **without changing
production policy**. The policy change itself is a ruled decision
(design §8); this drill only proves the mechanism and records
what it actually does today.

The drill is executable: `h7_gate_relaxation_drill_add_verify_remove`
in `pkgs/idp-provider/tests/doors.rs` runs every step below against
throwaway provider configs and in-memory stores. Run it with:

```bash
cargo test -p idp-provider --test doors h7_gate_relaxation_drill_add_verify_remove
```

## Preconditions

- A test provider config whose allowlist is the production-shaped
  baseline (owner's own door identities only) — never the on-host
  production config file. The drill harness builds its own.
- A mock upstream door whose subject can be set to the test
  identity (`h7-drill-test-subject` at the Google door in the
  automated drill).
- H6 (#14264) is the go-live proof and remains a separate,
  still-open step; this drill does not depend on live traffic and
  does not substitute for it.

## Steps

1. **Snapshot the pre-drill state.** Record the baseline allowlist
   (owner identities only). Verify the test identity is denied:
   door login returns 403 `not_allowlisted`, the denial is in the
   audit log, and no identity row exists for it.
2. **Add the test enrollment.** Append exactly one allowlist entry
   (`google:h7-drill-test-subject`). This is the whole §4.4
   relaxation mechanism: a data change to the provider's
   enrollment policy, no redeploy of the auth path.
3. **Verify the enrolled identity.** Drive a door login for the
   test identity; exchange the code; read the ID token's
   `sub` / `roles` claims and the identity store row.
4. **Remove the test enrollment.** Restore the baseline allowlist
   exactly (step 1's snapshot).
5. **Verify the restore.** Repeat step 1: the same identity is
   denied 403 `not_allowlisted` and audited. Confirm the allowlist
   in force equals the pre-drill snapshot — production gate
   settings are left in their pre-drill state.

## Drill log (2026-11-25, automated run)

| Step | Observation |
| --- | --- |
| 0. Pre-drill denial | Test identity denied 403 `not_allowlisted`, audited at `door/google`, no identity row created. Gate is deny-by-default as designed. |
| 1. Enrollment added | One allowlist entry appended to a throwaway config; nothing redeployed. |
| 2. Enrolled login | Login succeeds (302 → code → token). The identity row is created `active`, linked by the door login, with the link in the identity history and audit. |
| 3. Claims observed | `sub` is the provider's single account (`drew` in the harness) and `roles` is that account's **full role set** — see the finding below. |
| 4. Enrollment removed | Baseline allowlist restored exactly. |
| 5. Post-drill denial | Same identity denied 403 `not_allowlisted`, audited — gate behaviour identical to the pre-drill state. |

## Finding — §4.4's default does not exist yet

§4.4's safety argument for open enrollment is that "a newly
enrolled account with only `read-only` on no tenant can do
nothing until granted". The current provider holds **one**
account (`registry.rs`: "the single account row"), and a
never-seen allowlisted door identity is auto-linked to that
account (`doors.rs`) and issued tokens carrying that account's
roles (`flow.rs`). There is no fresh read-only-on-no-tenant
account for a new enrollment to land on.

Consequence: today, an allowlist entry is safe only when the
enrolled identity is the owner's own new door identity (the v1
relaxation §4.4 also names: "add allowlist entries"). Enrolling
a *different person's* identity this way would hand them the
owner account and its roles. Multi-account enrollment with a
default `read-only`-on-no-tenant grant must land before any
third-party enrollment; that gap is filed as its own issue and
linked from #14265.

## Rollback / restore guarantee

The drill never opens the production config or a production
store: every step runs on a harness-owned config and in-memory
audit/identity stores, which are dropped at the end of the run.
The restore step is therefore verified by construction in the
automated drill, and by the step-5 denial check for any manual
re-run against a staging config.
