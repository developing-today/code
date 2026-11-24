# Design: implicit-trust sync — convergence toward 1:1 + conflict reports (#9142)

**Date:** 2026-11-24. **Source issue:** [#9142](https://github.com/developing-today/code/issues/9142)
(Program #9106, parent #9108; §1 of the #9106 ID directive, 2026-11-12).
**Author:** lane oc-code-08, queue task 21567 (AUDIT-7 Lane 6 DESIGN-GAPS packet).
**Kind:** sufficiency consolidation — the technical core was designed by the
#9106 ID/Atlas summit mill and merged in PR #9238
(`doc/2026-11-13T03-40-00Z_design_id-atlas-summit/`, part B = federation,
#9141–#9148). This record judges that core against the 9-point
sufficient-design standard for #9142's scope and fills the thin spots
(explicit goals/non-goals, do-nothing alternative, rollout/rollback,
named deciders) rather than duplicating the core.

**Status of the build slices at writing:** B08 scope/tolerance (#10209),
B07 conflict-report artifact (#10248), B09 divergence/anti-entropy (#10262)
are all CLOSED `status:done`. The design below is therefore the
as-built/planned record the slices implemented against.

## 1. Problem + context

ID replicas hold domain-local namespaces, registries, replicated
event/world data, authorization, peer discovery, relay and local/remote
query behavior — but there is no one uniform cross-domain registry or
generic federation/sync protocol (mill baseline `baseline/v1-audit.md`;
part-B `versioned-design.md` §V1). The #9142 ask: internal backend peers
that already trust each other implicitly must still converge toward
one-to-one sameness "within reason" (tolerance undefined in the ask),
and divergence must surface as durable, reviewable **conflict reports
first** — never silent resolution. Without this, operators cannot tell
"in sync", "diverged and reported", and "silently forked" apart, and any
later merge/CRDT work has no conflict corpus to reconcile against.

Evidence: #9142 body (§1 quote); program #9106 directive; V1 baseline
audit of existing ID sync paths; the three implementation slices each
cite the gap they closed (#10209 scope/tolerance, #10248 report
artifact, #10262 divergence/anti-entropy).

## 2. Goals and explicit non-goals

**Goals:**
- G1. Implicit-trust peers converge toward 1:1 sameness inside an explicit
  authorized scope (tenant, trust domain, capacity, retention); anything
  outside scope is excluded by rule, not by accident.
- G2. Every divergence inside scope produces a durable, addressable
  conflict report **before** any resolution; unrelated objects keep
  syncing while a conflict is open.
- G3. No silent merge,-statistic, or repair path anywhere in V2; every
  rejection/admission is typed, observable, and auditable.
- G4. Sync runs over the §0 extensible primitives/adapters (signed
  envelopes, policy gates), not a parallel trust channel.

**Non-goals (explicit):**
- N1. No automatic merge or CRDT convergence in V2 — that is conditional
  V4 work gated on the five #9143 research slices (which may conclude
  "adopt none").
- N2. No cross-tenant or cross-trust-domain disclosure: scope precedes
  disclosure; a content hash is not an access capability.
- N3. No global-completeness claims: a remote negative is local to that
  peer, never proof of global absence.
- N4. No new trust root or SSO/tenancy redesign: #9077 (SSO) and #9083
  (tenancy) remain sibling owners and boundary inputs.

## 3. Alternatives considered (with tradeoffs, including do-nothing)

- **A0. Do nothing (keep domain-local sync only).** Cost: zero new
  machinery; each domain keeps working exactly as audited. Tradeoff: the
  "within reason" 1:1 expectation in the program directive stays
  uncheckable; silent forks remain undetectable; V4 merge work would have
  no conflict corpus. Rejected: it abandons G1–G3 outright.
- **A1. Signed-gate-first sync with conflict-reports-first (chosen).**
  Reuses the universal signed acceptance gate; adds scoped convergence +
  durable reports; defers merge. Cost: report storage/retention and an
  anti-entropy protocol to build. Best fit for G1–G4/N1–N4.
- **A2. CRDT-first convergence.** Would give automatic convergence, but
  none of the four candidate families (Automerge, Diamond Types, Willow,
  OCapN) had been evaluated on common criteria at design time, and
  silent-merge risk directly violates G3. Deferred to conditional V4
  (#9143 research matrix; "adopt none" is a valid outcome).
- **A3. Gossip/any-trust background sync.** Cheapest convergence, but no
  scope authorization, no provenance, no conflict artifact — violates
  N2 and G2/G3. Rejected.

## 4. Chosen approach

(Technical core: part-B `versioned-design.md` §§V2/V4 + migration;
pointers, not duplication.)

- **Trust classes as policy labels:** `PeerPolicy { peer_id, class ∈
  {fully_trusted, send_only, accept_signed_from}, operations, relay,
  scope, policy_version, valid_from, expires_at }`. "Fully trusted"
  widens configured operations only — it never bypasses validation.
  Unknown peers reject by default.
- **Wire model:** `SignedEnvelope { suite, signer_id, key_id, event_id,
  object_id, sequence?, parent_ids[], issued_at, payload_digest,
  payload_len, scope, signature }`, Ed25519 over deterministic-CBOR Body
  with domain prefix; versioned suite allowlist, fail closed. Ingest
  pipeline `received → parsed → signature_valid → identity_bound →
  policy_allowed → accepted`; local producers sign before admission
  through the same gate.
- **Conflict artifact:** `ConflictReport { report_id, object_id,
  versions[], signer/source[], scope, policy_digest, detected_at,
  evidence_refs[], status }` — durable/addressable; creation precedes
  any merge; resolution is a new signed event. Conflicts never block
  unrelated objects (B07, landed as #10248).
- **Convergence scope ("within reason" defined):** sync converges only
  inside authorized scope/capacity/retention/tenant/trust-domain
  boundaries; divergence detection + exclusions are explicit (B08,
  landed as #10209). Deletion/tombstone is a signed scoped event;
  report `deleted_by_policy` separately from `missing`/`not_authorized`.
- **Anti-entropy without merge:** scoped divergence detection and delta
  exchange; report-before-resolution invariant enforced (B09, landed as
  #10262).
- **Security/capability notes:** emergency key/peer revocation is
  V2-crucial (immediate block + quarantine, never silent rewrite);
  signer wall-clock does not prove historical validity (may stay
  `indeterminate`); quarantine preserves bytes/provenance while
  withholding release pending revalidation. No first-party privilege;
  hosted service is an ordinary configured peer.

## 5. Dependency-ordered build slices (each one lane/session, with acceptance)

All three slices are **done**; recorded here as the as-built order for
auditability:

1. **B08 — scope/tolerance** (#10209, CLOSED). Defines authorized
   convergence scope and the explicit 1:1-within-reason boundary.
   Acceptance: scope/exclusion rules implemented per statements
   S1.26–S1.31; out-of-scope records excluded by rule with typed reason.
2. **B07 — conflict-report artifact** (#10248, CLOSED; depends on B08).
   First-class durable report per S1.34–S1.38. Acceptance: report
   created before any resolution event in the audit log; reports
   independently reviewable with versions/signers/evidence refs.
3. **B09 — divergence detection + anti-entropy** (#10262, CLOSED;
   depends on B07/B08). Detection and reconciliation-exchange
   requirements per S1.32–S1.40. Acceptance: scoped divergence detected
   and anti-entropy deltas exchanged with no silent merge; unrelated
   sync continues during open conflicts.

No further #9142 implementation slice is open. Any future V4 merge work
belongs under #9143-gated follow-ups, not #9142.

## 6. Rollout + rollback

Rollout (per mill migration §, #9142-scoped):
1. Inventory current ID sync paths; preserve existing world/event/group contracts.
2. Ship envelope parser + canonical vectors in shadow mode; admit nothing new.
3. Activate the mandatory signed gate for the new sync API (not configurable off);
   backfill legacy bytes as `legacy_unverified`, quarantined until authorized
   re-attestation over exact legacy bytes — never silent rehash/re-sign.
4. Enable conflict-report persistence + anti-entropy without auto-merge.
5. Add sync as an opt-in link kind with conservative caps; old peers are not
   implicitly relays or accepted sources.

Rollback: disable the sync link kind per peer (config change, no code);
already-admitted data stays valid under its admission-time policy evidence;
open conflict reports are retained (they are the audit trail, not to be
rolled back); affected material can be quarantined pending revalidation.
No silent byte rewrites in either direction.

## 7. Verification plan

- **Executable model:** `python3 -I
  doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/federation_model.py`
  — disjoint sub-budget reservation, retry reuse, changed-binding
  rejection, revocation-before-cache-release, no-rerun-after-expiry,
  plus 2,000 seeded allocation-tree traces. (Models authenticated-edge
  grants; not crypto or global-budget proof — stated limit.)
- **Oracles:** PBT envelope (any signed-field mutation fails
  verification); PBT budget accounting (monotone spend, `spent + held ≤
  cap`); PBT dedup; MBT ingest/rotate/revoke/replay state machine; MBT
  network traces terminate under caps; **conflict-first oracle** (audit
  log contains report before any merge/resolution; unrelated sync
  proceeds); receipt/claims oracle.
- **Live proof:** the three landed slices' own tests/vectors
  (#10209/#10248/#10262); cross-language canonical-byte vectors
  including the legacy JSON preimage-mismatch guard.

## 8. Open questions (each with a named decider)

1. Exact historical revocation policy (quarantine-which-interval vs
   preserve-and-flag)? **Decider: Drew** (owner). Default: quarantine
   affected interval, preserve bytes; wall-clock alone proves nothing.
2. Dedup TTL/capacity bounds and hard maxima? **Decider: #9111 config
   owner.** Default: finite config, bounded hard maximum, binding
   tombstone through origin expiry.
3. Are `complete=true` responses allowed by default? **Decider: #9111
   config owner.** Default: opt-in.
4. #9084 transfer/retention boundary for sync payloads? **Deciders:
   #9084 owner + Drew.** Default: digests/references only until agreed.
5. Cross-peer takeover attribution + reputation (V4)? **Decider: Drew.**
   Default: V2 local disable/quarantine only; V4 work gated on #9143
   outcomes. None of these blocks V2 sync operation.

## 9. Record, tracking, queue

- **Dated record:** this file
  (`doc/2026-11-24T01-10-00Z_design_9142-implicit-trust-sync-sufficiency.md`)
  plus the mill core `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/`
  (part B, PR #9238).
- **Tracking issue:** #9142 (stays OPEN as federation parent/meta per
  triage notes; link recorded via agent-note comment).
- **Queue item:** task 21567 (`repo-health-code-pilot`), completed on
  merge of this record's PR.
- **Amendment policy:** future #9142-scope corrections land as dated
  amendments referencing this file; the mill core is frozen history and
  is cited, not rewritten.
