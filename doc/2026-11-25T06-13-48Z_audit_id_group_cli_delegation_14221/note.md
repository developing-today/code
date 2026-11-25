# Audit: `id group` delegate, revoke and group command paths against the delegation threat model

- Record created: 2026-11-25T06:13:48Z
- Issue: developing-today/code#14221 (first slice of #6379's acceptance list)
- Threat model: `doc/2026-11-23T17-47-48Z_design_id_delegation_threat_model/note.md`
  (#13535), with its trust boundaries TB1 to TB3, guarantees G1 to G9 and the
  F1/F2 decisions D1 (grant expiry bound) and D2 (grantor veto).
- Code read on main at 09ea6ad3f:
  - `pkgs/id/crates/id-cli/src/commands/group.rs` (delegate, revoke and the
    other group commands; there is no separate `delegate.rs` or `revoke.rs`
    now, both live in this file)
  - `pkgs/id/crates/id-cli/src/cli/groups.rs` (`DelegateArgs`, `RevokeArgs`)
  - `pkgs/id/crates/id-groups/src/groups/log.rs` (`GroupLog::open`,
    `record`, `record_preformed`, `append`, `delegate`)
  - `pkgs/id/crates/id-events/src/groups/delegation.rs` (fold, expiry, veto)
  - `pkgs/id/tests/cli_delegation.rs`

## Findings

| Id  | Finding                                                                                                                                                                               | F-item / guarantee | Disposition                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ | --------------------------------------------------------------------- |
| C1  | `id group delegate` printed the grant without its `expires_at` bound, so the confirmation did not show what was recorded.                                                             | F1 / D1            | Fixed in the PR that adds this record: the bound is printed when set. |
| C2  | A bound at or before now was accepted. The fold applies such a grant, but every action under it is already past the bound, so it is dead on arrival and the command reported success. | F1 / D1            | Fixed in the same PR: the CLI refuses the bound and records nothing.  |
| C3  | `delegate --mode` records the delegation-mode change before the grant. If the grant is refused, the command fails but the mode change stays in the log.                               | G5, F5             | Per-finding issue #14877.                                             |
| C4  | The fold never expires a bounded grant for an action with no `at` (legacy lines, or a pre-epoch clock). Expiry fails open.                                                            | F1 / D1            | Per-finding issue #14876 (proposes fail-closed).                      |

## Conforming observations

- **G1 (attenuation) and G2 (no admin grants, no self-grants):** every grant
  from the CLI goes through `GroupLog::delegate` and the fold's judge; the CLI
  has no path that writes a grant around it.
- **G4 / D2 (veto):** `revoke` records a veto through the same log and is
  judged by the fold's author check; the CLI adds no author of its own.
- **G5 (removal disposition):** `group remove` resolves `server-default` to a
  concrete mode before recording, so every removal names its mode (C3 is the
  one gap, on the delegate path).
- **G8 (determinism):** the fold judges expiry from the recorded `at` and the
  signed bound only; the new C2 check reads the local clock in the CLI before
  anything is written, not in the fold, so G8 is unaffected.
- **Refused outcomes:** `print_action` fails the command on a `Refused`
  outcome, so a refused grant or revoke is never reported as success.

## Already covered elsewhere

- The local group log is unsigned JSONL; its trust is the local account
  boundary, recorded under F3 by #13531.
- Key-file permission handling was hardened by #14307.

## Not covered by this slice

Property-based tests for the security invariants and the legacy-forms
migration stay on #6379, as #14221 scoped them.
