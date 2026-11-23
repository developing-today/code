# Identity and delegation — threat model, guarantees, and the F1/F2 decisions (#6379)

- Design record · Issue: developing-today/code#6379, stage 2 packet 1 (the
  design core routed by the 2026-11-17 triage, task 18481) · Sources: the
  stage-1 audit (task 14441, findings F1–F7 on #6379) and the design of
  record, `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/` §7 ·
  Status: decisions recorded in §4; implementation filed as #13534;
  remaining stage-2 packets filed as #13531 (F3+F4 verification), #13532
  (F5 CLI tests), #13533 (F6 legacy-forms migration).
- This record satisfies #6379's "documentation of security guarantees"
  acceptance item. It changes no code, weakens no check, and creates no
  schedule.

## 1. Current state (verified 2026-11-23 against origin/main `805258752`)

The stage-1 audit's paths predate the pkgs/id crate split (#4405–#4408).
The surface now lives at:

| Piece | Location |
|---|---|
| CLI group commands (`cmd_delegate`, `cmd_revoke`) | `pkgs/id/crates/id-cli/src/commands/group.rs` |
| CLI arg shapes (`DelegateArgs`, `RevokeArgs`) | `pkgs/id/crates/id-cli/src/cli/groups.rs` L108 / L129 |
| Delegation judge: grants, chains, vetoes, attenuation | `pkgs/id/crates/id-events/src/groups/delegation.rs` |
| Fold / settlement engine, `Refusal` vocabulary | `pkgs/id/crates/id-events/src/groups/fold.rs` |
| Group events / wire forms | `pkgs/id/crates/id-events/src/groups/event.rs` |
| Policy incl. delegation + removal modes | `pkgs/id/crates/id-events/src/groups/policy.rs` |
| Actions (`delegate`, `revoke` = veto, device ops) | `pkgs/id/crates/id-groups/src/groups/actions.rs` |
| Group log, legacy wire decoding | `pkgs/id/crates/id-groups/src/groups/log.rs` |
| Event-store ingest (signature verification) | `pkgs/id/crates/id-events/src/event_store/ingest.rs` |
| Keypair load/create | `pkgs/id/crates/id-store/src/store.rs` |

Findings state, re-verified in the current tree:

- **F1 open.** Neither `DelegateArgs` nor the `Grant` struct
  (`delegation.rs`) carries an expiry or bounds field.
- **F2 open.** The veto judge (`delegation.rs` L155) requires the author
  to hold Manage and a chain; a non-manager grantor has no revocation
  path for its own grant.
- **F3/F4 unverified end-to-end** → #13531. (One F4 fact has improved
  since stage 1: keypair creation goes through `write_private_file` +
  `restrict_key_permissions` in `id-store/src/store.rs`.)
- **F5 open.** `pkgs/id/tests/cli_delegation.rs` still holds exactly 2
  tests → #13532.
- **F6 open.** Legacy forms still decoded, no migration doc → #13533.

## 2. Threat model

**Assets.** Group authority — who may act, with which scope, in a group
or through a crossing into a world; the integrity of the group log that
authority is judged from; account keys.

**Actors.** (a) A malicious or compromised group member acting within
its membership; (b) a removed member attempting to keep or revive
authority; (c) an external party injecting forged or replayed events
over sync, relay, or hosted paths; (d) an attacker with read access to
the operator's local filesystem; (e) a grantee whose key is compromised,
where the grantor needs to react before a manager is reachable.

**Trust boundaries.**

- **TB1 — ingest.** The fold does no cryptography. Every event the fold
  judges entered the store through ingest, which verifies signatures
  (`verify_strict`) and structural rules and refuses what fails. Every
  guarantee in §3 is therefore conditional on the ingest funnel being
  total — no write path bypassing it. That totality is F3, verified by
  #13531; until it reports, the conditionality stands.
- **TB2 — key custody.** The account is its keypair file. Anything that
  can read the file can act as the account; no guarantee in §3 survives
  loss of the file's confidentiality. Custody facts per surface are
  F4, recorded by #13531.
- **TB3 — causal order.** All judgments are made at causal positions in
  the signed log, never at wall-clock "now". Two replicas folding the
  same log reach the same verdicts; a verdict can change only by the
  log growing.

**Out of scope.** Full host compromise beyond key-file read; denial of
service beyond the bounded-waiting properties of the design of record;
the world bearer-capability join path (design §7.5's migration target),
named here only where its bounds contrast with group grants.

## 3. Security guarantees

Each guarantee is stated so it can be falsified: the judging code is
named, and a folded log exhibiting the counter-behaviour falsifies it.

- **G1 — Attenuation.** No grant conveys a scope its grantor did not
  hold at the grant's causal position, and no chain authorizes an
  action unless every link was valid where it was made. Judged in
  `id-events/src/groups/delegation.rs`; refusals `Attenuated`,
  `NoChain` (`fold.rs` L99, L102).
- **G2 — Admin is never granted; no self-grants.** Admin is a policy
  right, not a scope (`BadScope`, `fold.rs` L104); a grant to oneself
  is refused (`BadGrantee`, `fold.rs` L106).
- **G3 — Cross-world cap.** A crossing grant is capped at Write and
  must name a non-empty world (`NotCrossable`, `delegation.rs` L131).
- **G4 — Veto discipline (current form).** A veto takes effect only
  from an author holding Manage at the position with an authorized
  chain, against a grant in the veto's causal past in the same group
  (`delegation.rs` L155). §4 D2 extends the author set; when #13534
  lands, G4 is restated to match, and the manager path is unchanged.
- **G5 — Removal disposition is explicit.** Every removal's mode
  decides its grants: `Revoke` ends them, `Transfer` reissues them
  from a permitted successor, `KeepPrior` retains the original
  grantor, `ReassignToGroup` re-roots them at the group. Grants never
  revive on re-add after a `KeepPrior` removal (issue #169 ruling).
- **G6 — Device scope cap.** A device key acting for an account is
  refused above its device scope (`delegation.rs`,
  `authorized_device_account`, L273).
- **G7 — Ingest authenticity.** Bytes that do not verify are refused
  and change nothing (`event_store/ingest.rs`; test
  `test_bytes_that_do_not_verify_are_refused_and_change_nothing`,
  L2381). Conditional on TB1 totality (F3 / #13531).
- **G8 — Determinism.** The fold is a pure function of the signed log:
  same log, same settled grants, vetoes, and removals, on every
  replica. Any future judging input that is not a signed log field
  (a local clock, local state) breaks G8 and is out of bounds for
  changes under this record.
- **G9 — Interim grant lifetime (until #13534 lands).** A grant has no
  expiry: it lives until a veto or a removal-mode disposition ends
  it. This is stated as the operative fact operators must plan
  around — issuing a grant is issuing standing authority — not as a
  design goal. D1 supersedes it.

## 4. Decisions

### D1 (F1) — Adopt the designed expiry bound

**Decision.** Grants gain an optional signed `expires_at` bound,
exactly as the design of record specifies: `delegate{ …, bounds, … }`
(§7.1), `expires_at` signed so it converges, checked at each action's
position (§7.4), under validity rule §7.2(5) — "D has not expired at
E". Implementation is #13534. Until it lands, G9 is the operative
guarantee.

**Rationale.** (1) The semantics are already settled by the design of
record; this record adopts them rather than inventing new ones. (2)
Judged only from signed fields, expiry preserves G8 — the fold gains
no clock or local-state input. (3) The neighbouring world-capability
system already carries the same bound (`CapabilityBounds { expires_at,
uses }`); group grants are the outlier. (4) The audit's core concern —
dormant grants as standing authority — is not answered by documenting
the status quo as intended.

**Explicitly rejected.** `uses` as a group state rule (§7.4: counts do
not converge across a partition; they remain local enforcement by the
hosting node, as today). Also rejected: declaring
indefinite-until-veto the intended end state — it would contradict the
design of record on no new evidence.

**Stated limitation.** The bound is judged at the action's position
against signed timestamps, which the acting author asserts. Expiry is
therefore a convergent, auditable bound — a backdated action is a
signed, attributable log fact forever — but it is not a hard real-time
cutoff against an actor willing to sign a false timestamp. The hard
cutoff remains veto and removal (G4, G5). #13534 must state its exact
comparison rule in its PR and reconcile it with this limitation.

### D2 (F2) — Grantor veto adopted; manager veto confirmed

**Decision.** A grant has two legitimate revokers:

- **(a) A manager** — holds Manage with an authorized chain at the
  veto's position. The governance path; unchanged from today.
- **(b) The grant's own grantor** — for that grant only. Judged:
  author is the grant's grantor of record; the grant is in the veto's
  causal past; same group. Designed here, implemented in #13534.

A grant re-anchored at the group by `ReassignToGroup` has grantor
`None` and remains manager-veto only. A grant reissued by `Transfer`
has the reissuer (successor) as its grantor of record, matching G5's
reissue semantics. A member who is neither manager nor the grant's
grantor still cannot veto it.

**Rationale.** (1) Revocation only ever removes authority, so widening
who may revoke cannot create authority — the change fails closed by
construction, and no existing refusal is weakened. (2) Incident
response: today a grantor who learns its grantee's key is compromised
must find a manager before the grant stops working (actor (e) in §2);
the grantor is the party with the context and the least escalation
need. (3) The design of record's `revoke{ delegation }` (§7.1) is
per-delegation and author-agnostic, and validity rule §7.2(4) judges
vetoes without an author restriction — the manager-only rule is an
implementation narrowing, not a design requirement.

**Stated limitation.** A grantor veto cuts every chain downstream of
its grant, including links others redelegated in good faith. That is
inherent to delegation — the same cut follows a cascade removal of
the grantor — and the veto is a signed, attributable log event, so
the cut is auditable after the fact.

## 5. Assumptions and open verifications

- **A1 (TB1 / F3).** §3 is conditional on ingest totality. The
  end-to-end trace of local, sync, hosted, and relay write paths is
  #13531; a confirmed bypass is a separate security issue, fail-closed
  until fixed.
- **A2 (TB2 / F4).** The account boundary is the keypair file's
  custody. Creation now writes through `write_private_file` and
  restricts permissions on create and on load (`id-store/src/store.rs`);
  the observed mode per surface (CLI create, CLI load of a pre-existing
  file, hosted/serve) is #13531's to record.
- **A3 (F6).** Legacy forms — the legacy remove `successor` wire forms
  and the silent `cascade` default for a log naming no delegation
  preference (`policy.rs` L312) — are read-compat only. The silent
  default is a known sharp edge: an old log that names no preference
  folds as `cascade`. Their migration/removal record is #13533.
- **A4 (F5).** CLI-level scenario coverage of G1–G6 is thin (2 tests)
  until #13532 lands; the fold-layer suite (71 tests in
  `id-events/src/groups/tests.rs`, plus the scenario and property
  files) is what currently backs the guarantees.

## 6. What remains of #6379 after this record

| Acceptance item | State |
|---|---|
| Security audit completed | Done — stage 1 (task 14441) |
| All security-critical code reviewed | Done for the delegation core (stage 1); F3/F4 end-to-end verification is #13531 |
| Documentation of security guarantees | Done — this record |
| Test coverage for security scenarios | Fold layer done; CLI layer is #13532 |
| Migration path for deprecated features | #13533 |
| F1/F2 implementation | #13534, gated on this record |

#6379 closes when #13531–#13534 have landed or reported, or by a later
triage that re-homes them.
