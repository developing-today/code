# BA-W01 component contract and implementation plan: identity, store membership and granular authorization

Status: proposed for review. Work item: BA-W01, issue #9365 (parent #9359, intake #9361).
Scope ledger rows: BA-003, BA-043, BA-044, BA-045, BA-046, BA-078 to BA-089
(source: [requirement-ledger.md](requirement-ledger.md)).

This document is a design contract and plan. It does not claim the behaviour is built, reviewed by
Drew, or deployed. Each clause below is either a contract the implementation must satisfy or an
explicit gap.

## 1. Model

Three identifiers are never interchangeable:

| Concept      | What it is                                                       | Authoritative for                  | Never used for                                                   |
| ------------ | ---------------------------------------------------------------- | ---------------------------------- | ---------------------------------------------------------------- |
| Account      | Authenticated identity from a verified sign-in method            | Who may sign in and own stores     | Continuity of an unsigned browser                                |
| Installation | Browser/device UUID stored in local storage                      | Continuity of a guest's local work | Authentication, permission, or linking two devices to one person |
| Principal    | The actor on a command: account, guest session, or service actor | Authorization decisions            | Anything without a verified source                               |

- Trusted identity originates at the validated gateway (the existing Tailscale path), never a
  browser-supplied actor field. Every state-changing command carries a principal derived server-side.
- A browser UUID may be regenerated, lost, or evicted; the system must tolerate that and must not
  promise indefinite retention (BA-043 gap, see section 7).
- No fingerprinting, correlation of stored identity locations, or respawning of deleted identifiers
  (BA-044, issue constraint). This is a deliberate absence, not a deferred feature.

## 2. Stores, membership and roles

- A store has one authoritative placement, one boundary type (`private`, `shared`, `open`), and a
  membership list. Membership alone does not grant item-level access (BA-088 gap).
- Default provisioning (BA-080, BA-081): one personal private store per account, named by an
  editable display name; one explicitly configured default shared store. A new account is not
  automatically a member of the shared store.
- Roles: `viewer`, `editor`, `administrator`. Ownership and service identities are separate from
  roles. Policy elevation is restricted to privileged authority (section 4).
- Store actions (BA-079): create, add member, invite, accept, decline, revoke, view, hide, archive.
  Hide is a personal presentation choice; archive is a lifecycle operation; neither is permission
  enforcement. Invitation email and outer Tailnet access grants are out of scope.
- Display name (BA-045): a person label, a browser UUID and an authenticated account are three
  distinct things. Renaming changes the label only; it never changes the identifier.

### 2.1 Invitation lifecycle

States: `pending` to `accepted`, `declined`, `revoked`, or `expired`. Each invitation has an expiry
and a single-use token. Accept requires the signed-in principal to match the invitee. Replays of an
accepted, declined, revoked or expired token fail closed (issue negative test: expired/replayed
invitations).

## 3. Guest claim and sign-in linking (BA-083, BA-084)

- A guest starts on first activity with a local installation UUID and a private guest store.
- Attaching the guest store to an account requires: proof of possession of the guest session, a
  preview of stores and data to be attached, collision handling for identical names or IDs, and an
  audit event. Claims are atomic; a partial claim rolls back.
- Linking two sign-ins into one identity requires proof of both identities. A claim may never be
  completed by presenting only one proof (account takeover prevention). Cross-account guest claims
  fail closed (issue negative test).
- Concurrent devices: a claim is applied once; the second claimant receives a clear conflict result.
- Logout and account switch clear every protected cache (issue negative test) and discard the
  guest binding for the previous principal.

## 4. Policy evaluation (BA-089, BA-088)

Evaluation order for a request on entity E by principal P, action A:

1. Resolve the principal (section 1); reject if unauthenticated and the store is not `open`.
2. Require store membership or an explicit open-audience grant for the store containing E.
3. Collect policy rules that apply to E directly, to its groups, and to its store, with their
   numeric priority and effect (`allow` or `deny`).
4. Apply, in order:
   - Any inherited deny with no privileged override is a hard block.
   - An `allow` may only win over a `deny` if its priority is strictly greater and it was created by
     privileged authority (administrator or service policy owner). An editor can never create a rule
     that defeats an inherited deny by choosing a larger number.
   - Equal-priority allow vs deny resolves to deny.
   - No matching rule resolves to deny (default deny outside an explicit scope).
5. Cyclic or ambiguous group inheritance is rejected at write time, never evaluated.

Numeric priority is retained only as a tie-breaker inside the privileged-override class. The first
implementation may ship without numeric priority overrides at all (deny-only inheritance) if review
judges the override surface unnecessary; that choice is recorded as a decision for the leaf issue
L4 below, with default **no overrides** until reviewed.

## 5. Enforcement surfaces

Every surface calls the same evaluator with the same principal. No surface may filter with its own
logic.

| Surface                                   | Seam the call goes through                      | Denial behaviour                                             |
| ----------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------ |
| API reads and writes                      | Command validation layer, before the repository | 403 without revealing existence                              |
| Search (including Atlas index partitions) | Query scope builder; default-deny filter        | Results omit denied rows; no count leak                      |
| Counts and totals                         | Same query scope as search                      | Counts computed after the filter                             |
| History and audit trail                   | History reader                                  | Denied entities omitted, not redacted to a placeholder       |
| Media                                     | Media fetch route                               | Not found, indistinguishable from absent                     |
| Exports                                   | Export builder                                  | Export contains only authorized rows; source revision stated |
| Summaries and reports                     | Report pipeline input                           | Aggregates computed over authorized rows only                |
| Background jobs                           | Job envelope carries the initiating principal   | Job cannot obtain broader visibility than its initiator      |

Nonmembers must not infer hidden existence through IDs, counts, search, history, summaries or
exports. Responses for missing and forbidden entities must be identical in shape and timing where
practical.

## 6. Unicode and text validation (BA-046)

- Accept any valid Unicode scalar sequence, including international text and emoji sequences.
  Reject only: control characters other than tab/newline where allowed, unpaired surrogates, and
  bidirectional override characters in identifiers.
- The client-side blanket zero-width-joiner prohibition is removed. Emoji ZWJ sequences are valid.
- Validation is structural and server-side: identifiers and names are checked against the
  structural class; free-text comments are checked for allowed classes only. Comment emoji are not
  rejected by identifier rules (issue negative test: Unicode structural validation differs from
  comment emoji).
- Moves between stores never broaden visibility (issue negative test): a moved entity is re-evaluated
  against the destination store and its policies.

## 7. Gaps and explicit non-claims

| Ledger row     | Disposition                | Reason                                                                                        |
| -------------- | -------------------------- | --------------------------------------------------------------------------------------------- |
| BA-003         | Contract + leaf L2         | Multi-person comes from membership and roles; not equivalent to full onboarding               |
| BA-043         | Gap, design only           | Browser deletion and eviction prevent a forever guarantee; recovery and retention need design |
| BA-044         | Deliberately absent        | Privacy and consent; fingerprints must never grant authorization                              |
| BA-045         | Contract + leaf L2         | Display name separated from identifiers                                                       |
| BA-046         | Contract + leaf L6         | Exact allowed/rejected classes defined server-side                                            |
| BA-078, BA-079 | Contract + leaves L2, L3   | Roles, invitations                                                                            |
| BA-080, BA-081 | Contract + leaf L2         | Provisioning rules; second-person access is an external gate                                  |
| BA-082         | Out of scope               | Username/password or Google authentication is a separate identity provider decision           |
| BA-083, BA-084 | Contract + leaf L5 (gated) | External gate: account-link proofs and provider setup                                         |
| BA-085         | Contract, gated            | "Open" audience must be defined explicitly; not public by assumption                          |
| BA-086         | Contract + leaf L4         | Query scopes and denial behaviour                                                             |
| BA-087         | Deferred                   | Atlas/Snowflake ingestion is not implemented; no interface added                              |
| BA-088         | Contract + leaf L4         | Per-item ACL, later                                                                           |
| BA-089         | Contract + leaf L4         | Override semantics; default no overrides until reviewed                                       |

External gates named by the issue: provider setup and credentials, new audience access, account-link
proofs. Leaves that depend on them are filed with the `gated` label and are not silently dropped.

## 8. Negative-test plan

The issue's acceptance tests are carried verbatim in substance. Each has an oracle, a denial matrix
row, and a surface.

1. Nonmember cannot infer existence via ID, counts, search, history, summaries or exports. Oracle:
   identical response shape for absent and forbidden entities across every surface in section 5.
2. Editor cannot elevate policy priority to defeat an inherited deny. Oracle: write rejected; the
   evaluator still returns deny.
3. Equal-priority allow vs deny resolves to deny. Oracle: evaluator table test.
4. Expired invitation fails; replayed (already accepted, declined or revoked) invitation fails.
   Oracle: state machine transition table.
5. Cross-account guest claim fails; claim without both proofs fails. Oracle: claim state table.
6. Logout and account switch clear protected caches. Oracle: cache inventory after each transition.
7. Unicode: structural validator accepts emoji ZWJ sequences in comments and rejects them in
   identifiers where the class disallows them. Oracle: golden corpus of accepted and rejected strings.
8. Moves never broaden visibility. Oracle: before/after visibility sets for a moved entity.

Where practical the policy evaluator is covered with a model-based test (the model is section 4's
ordered rules) and property tests over generated rule sets, per the testing doctrine in AGENTS.md.

## 9. Rollback

- Every schema change is additive first; the evaluator can run in shadow mode (compute and log the
  decision, do not enforce) before enforcement is enabled.
- Enforcement is switched per surface. Disabling enforcement for one surface must not disable
  others.
- A failed claim rolls back atomically; a failed invitation transition leaves the previous state.

## 10. Implementation leaves (dependency order)

Each leaf is an issue sized for one fix packet, linked from #9365.

| Leaf | Scope                                                                                    | Depends on      | Gate                                       |
| ---- | ---------------------------------------------------------------------------------------- | --------------- | ------------------------------------------ |
| L1   | Principal and installation separation: server-derived principal, UUID as continuity only | none            | none                                       |
| L2   | Stores, membership, roles, provisioning rules, editable display name                     | L1              | none                                       |
| L3   | Invitation lifecycle state machine with expiry and single-use tokens                     | L2              | none                                       |
| L4   | Policy evaluator (section 4), query scopes for search and counts, shadow mode            | L2              | priority override decision                 |
| L5   | Guest claim and sign-in linking with proofs, preview, audit                              | L1, L3          | gated: account-link proofs, provider setup |
| L6   | Unicode structural validator and golden corpus                                           | none (parallel) | none                                       |
| L7   | Enforcement on history, media, exports, summaries and background jobs                    | L4              | none                                       |
| L8   | Negative-test suite from section 8, and cache clearing on logout and account switch      | L4, L5          | none                                       |

Parallel-safe: L6 may proceed alongside L1 to L4. L5 must not start until its gate is recorded.

## 11. Decisions needed

- Whether numeric priority overrides ship at all in the first implementation (default: no).
- The exact definition of "open" audience (BA-085); default: no open store is created by this work.
- Whether BA-043 forever-retention is accepted as a best-effort requirement with a visible reset
  path (design owner decision; default: best-effort with honest UI copy).
