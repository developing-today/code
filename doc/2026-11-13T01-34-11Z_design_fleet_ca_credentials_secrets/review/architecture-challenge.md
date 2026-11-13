# Architecture challenge — Fleet credentials and CA

**Scope:** Read-only challenge of the parent decisions in `architecture-review.md`
against the controlling intake, addendum, handoffs and v0.3 specification. The
domain designs are still being written and are not assessed here as final.
No implementation, provider, client, or production capability was exercised.

## Overall judgment

I **agree with the parent architecture's direction**, conditional on resolving
the first two architecture blockers below before treating the design as settled:

1. Do not claim safe automatic updates against a provider that lacks a write
   precondition honored by all writers. A Fleet-local lock cannot serialize
   external editors, and readback detects some races only after an overwrite.
2. Make the root signing authorization interface bind approval to the exact
   signer identity and operation, not merely require an explicit signer
   reference plus a generally authorized principal.

Both are design-contract gaps, not requests to perform live operations. Their
eventual code-level proofs are implementation evidence and can be deferred to
synthetic tests. Other observations below are conditions on the detailed domain
designs or implementation tests, not blockers to the high-level architecture.

## Findings

### A-1 — No-CAS provider writes can still lose concurrent edits

**Severity:** Critical safety gap. **Blocks:** settling the automatic-write
contract; not synthetic adapter/model work.

**Evidence:** Parent ADR-S02 correctly says a preflight read is not CAS, but
allows a “narrowest lease Fleet can actually enforce” and proposes stopping on
divergent readback (`architecture-review.md` lines 42–58). The governing spec
requires preserving independently changed fields and native revision
preconditions (§§20.3, 20.5; lines 1125–1134, 1154–1156); it also says to
explicitly expose the race if CAS is absent (§35.1; lines 2191–2203). An
external editor need not honor Fleet's lease. If it changes a value after the
preflight and before Fleet's unconditional update, post-write divergence may
find that Fleet already destroyed the peer's value. A saved old snapshot is
not necessarily the concurrently written value.

**Required design fix:** Define a per-adapter/per-operation safety capability
for conditional mutation. If the provider cannot atomically reject a write
against the observed native revision (or an equivalent serialization
mechanism that every writer must honor), automatic writes to conflict-sensitive
fields must stop for review or use a separately approved, explicitly weaker
policy. Do not present a Fleet-only lease or after-the-fact readback as
serialization. Preserve the planned value and mark the outcome uncertain on
timeouts; never blind-retry.

**Acceptance oracle:** A deterministic generated schedule performs preflight,
an external concurrent update, then Fleet apply. The oracle compares both
providers and the last-common baseline: the external value must remain
recoverable and the operation must report conflict/unsupported-guard rather
than successful update. Also test provider timeout after commit and after
non-commit; replay must not duplicate history or overwrite a later value.

### A-2 — Root signer authorization needs transaction binding and anti-replay

**Severity:** High security design gap. **Blocks:** settling the signing
interface; concrete token format and ceremony implementation can be deferred.

**Evidence:** Parent ADR-S01 separates capability, authorization grant and
operation plan (lines 10–40); ADR-S04 models signer identity separately
(lines 95–123). Spec §24.3 requires an explicitly authorized root source for
each signing operation and rejects ambient/default signing (lines 1459–1474);
the handoff also requires explicit signer source and no silent substitution
(handoff lines 23–25, 208–212). The parent establishes separation, but does not
yet state which exact operation fields the grant authorizes or how a grant is
prevented from being replayed for a different certificate, algorithm, validity,
destination, or signer.

**Required design fix:** The external signing seam should consume a
transaction-scoped authorization bound at least to the root key fingerprint /
handle, requested signature and subject-key identity, profile/purpose,
validity, artifact-plan digest, and any authorized output/escrow destinations.
Specify expiry, one-operation or bounded-use semantics, and how offline/HSM
adapters attest the executed plan. Public inventory lookup and escrow access
must never mint this authorization. Keep authorization distinct from key
access and from a valid certificate path.

**Acceptance oracle:** Negative interface/model tests reject ambient signer
discovery, signer substitution, changed-plan digest, wrong purpose, expired or
replayed approval, and a signer-capability handle used as a general secret
reference. A valid authorization succeeds only for its bound synthetic
transaction; the receipt identifies that transaction without exposing key
material.

### A-3 — Three-destination root custody must remain a narrow exception

**Severity:** High; design condition, not a disagreement with ADR-S05.

**Evidence:** The addendum explicitly asks for initial copies in encrypted
laptop storage, authorized BWS, and authorized 1Password, while forbidding
automatic expansion to future providers/caches/backups (addendum §5, lines
42–55). The parent preserves those three and separates escrow from ordinary
replication (ADR-S01 lines 21–28; ADR-S05 lines 125–141), in accordance with
the precedence rule. This is the right resolution of the apparent conflict
with the older blanket no-root-replication rule.

**Condition for detailed design:** Represent these as three explicit,
independently verified custody destinations under a dedicated root-escrow
policy, not as a generic provider list or inferred “all authorized replicas”
set. Distinguish active signer custody from escrow copies and make the
recovery-identity dependency graph explicit. An incomplete destination set or
one failed restore must not be reported as complete custody. This preserves
the requested policy without authorizing execution in this design pass.

**Acceptance oracle:** Synthetic policy tests reject wildcard/future-provider
expansion, generic sync/cache enrollment and missing destinations; isolated
restore tests prove each declared copy recoverable with the other destinations
unavailable, using separately protected recovery material. Production remains
approval-gated.

### A-4 — Keep the compatibility graph separate from client support claims

**Severity:** Medium; implementation evidence deferred.

**Evidence:** Parent ADR-S04 correctly calculates the reference inventory and
states that 65 certificate variants are not 65 trust anchors; it requires an
ML-DSA-only anchor and all-ML-DSA path for a strict PQ authentication claim
(lines 95–123). This matches spec §§3.2–3.5, §24.2, §33.3 and §48.2 V3-S15–17
(spec lines 114–155, 1423–1436, 2120–2122, 2669–2671). It also correctly
keeps MTC optional (handoff lines 53–64; source precedence lines 29–30).

**Condition for detailed design:** Preserve the distinction among (a) a
synthetic graph being constructible, (b) an implementation supporting each
signature/OID, (c) a client building the intended path, and (d) a verifier
actually enforcing PQ-only policy. Client rows must be versioned evidence, not
inferred from cross-sign availability. MTC and future crypto experiments must
not alter the baseline graph, silently fall back to classical authentication,
or gate ordinary V1/V2/V3 progress.

**Acceptance oracle:** Independent path validation plus a client-capability
matrix; negative tests cover missing ML-DSA support, classical anchor/path,
classical leaf authentication, stale/missing MTC landmarks, and fallback that
would otherwise be misreported as PQ. These are implementation/research gates,
not prerequisites for this high-level architecture to proceed.

### A-5 — Preserve scaling and reliability as measured constraints, not promises

**Severity:** Medium; evidence and concrete budgets deferred to benchmark /
operations work.

**Evidence:** Parent ADR-S02 calls for bounded concurrency, supported paging,
coalesced reads, resumable cursors and a scope/pagination/grant completeness
token (lines 42–70); ADR-S03 makes freshness part of the result and rejects
silent stale fallback (lines 72–93). ADR-S06 correctly treats same-host
OpenBao containers as a lab and future federation as nonblocking (lines
143–155). These align with addendum §§2, 7–9 and spec §§20.3, 35.4, 37.1–37.3.

**Condition for detailed design:** Keep incremental lookback separate from
periodic full-census coverage; bind a completeness result to account, grants,
filters, pagination and snapshot/cursor generation so scope shrinkage or a
stale cursor cannot masquerade as deletion. Give each operation bounded
concurrency, cancellation and resumable state. Derive limits and freshness
budgets from real-provider and broker measurements; do not block the baseline
on OpenBao, fnox adoption, MTC, or an unmeasured latency target.

**Acceptance oracle:** Generated partial-page, permission-change, expired-token,
rate-limit, cursor-reset, timeout-after-write and process-restart sequences
must never advance a complete-scope checkpoint or classify absence as deletion
without authoritative evidence. Benchmarks report methodology and p50/p95/p99,
resource use and provider limits separately; no unmeasured SLA is claimed.

## Disposition

Retain ADR-S01–S06 with the two required refinements A-1 and A-2. A-3 through
A-5 are acceptance conditions for the domain designs and later implementation
evidence; they do not justify waiting for production permissions, a live
provider migration, or future infrastructure. The specified exact PKI graph,
source-precedence decisions, three-copy custody goal, and optional/nonblocking
MTC/future programs remain consistent with the reviewed inputs.

## Re-review — 2026-11-12

I re-read the complete current `architecture-review.md` and the cited source
sections: spec §§20.3, 20.5, 24.3, 35.1 and addendum §5. This is a review of
the parent text as it now stands, not a review of the still-evolving domain
designs.

### Disposition by prior finding

- **A-1 — Closed, with a required implementation distinction.** ADR-S02 now
  explicitly defines provider-enforced atomic preconditions, serialization
  honored by every writer, and unguarded writes; it rejects Fleet-only locks
  for externally writable scopes and defaults unguarded steps to
  `needs-review` (parent lines 73–97). This closes the lost-update design gap
  for the default path. Its separately approved weaker policy is clearly
  labelled as accepting a residual lost-update window, rather than being
  represented as safe serialization.

  **Source compatibility:** The gate does not inherently contradict the
  mandatory in-place stale-password procedure in spec §20.5. That procedure
  says what the accepted rotation saga must do—preserve history, update the
  existing native item ID, and verify—while §§20.3 and 35.1 require use of
  native preconditions where present and explicit exposure of a no-CAS race.
  The new gate is a stricter default, and its named weaker-policy path leaves
  room for the specified operation. However, because the current provider
  adapters are deliberately classified `unguarded` until evidence exists,
  implementation acceptance must not count a permanently `needs-review`
  plan as having implemented routine bidirectional reconciliation or §20.5.
  The domain design must state how the approved/operational policy enables the
  required saga, retains its native-ID in-place update, and reports the
  residual risk. This is an implementation/operating-policy acceptance
  condition, not an open A-1 architecture finding.

  **Oracle scope:** The preflight → external edit → apply test should assert
  conflict/no success for the default guarded contract. If testing the
  separately approved weaker policy, it must assert the explicit risk status
  and must not claim the concurrent value is preserved when the provider
  cannot guarantee that. This qualification avoids promising an impossible
  oracle for an explicitly weaker mode.

- **A-2 — Closed.** ADR-S01 now requires a transaction-scoped authorization
  bound to signer identity, subject-key identity, signature algorithm,
  purpose, validity, artifact-plan digest and authorized destinations; it
  specifies expiry, single use by default, enumerated bounded use, refusal on
  mismatch, receipt, and negative cases (parent lines 30–46). That closes the
  authorization-vs-capability gap and rejects ambient discovery/replay at the
  architecture level. Token format, durable consumption state and offline/HSM
  attestation remain appropriately deferred as implementation details, with
  their required behavior stated.

  **65-variant compatibility:** Single-use authorization does not conflict
  with the reference graph. The complete signature set can be precomputed
  and authorized as enumerated bounded use, or split into scoped grants per
  signer/operation; the plan digest can bind the full bundle. Spec §§24.2–24.5
  requires 65 certificate variants and explicit signer transactions, not a
  single unbounded approval. The current paragraph would be clearer if the
  enumerated plan were explicitly described as a finite set of
  `(signer, subject-key, signature-algorithm, purpose, validity, artifact)`
  tuples, because its listed subject and algorithm fields are singular while
  a bundle contains many signatures and multiple root signers. That is a
  precision improvement, not an open A-2 finding: per-signature grants already
  satisfy the stated contract.

- **A-3 — Closed.** ADR-S05 makes the three destinations a closed enumerated
  set and makes custody incomplete if any declared copy or isolated restore
  fails (parent lines 168–187). This matches addendum §5's three named
  destinations and explicit prohibition on automatic replication expansion.
  It does not grant authority to perform production copying, and I find no
  contradiction with the source.

### New issues / contradictions check

No new blocker found in the changed text. The main precision note is the
authorization tuple above. The gate for “any field both sides may edit” is
conservative but not a source contradiction: §20.3 requires per-field conflict
handling, while §35.1 expressly recognizes that no-CAS adapters retain a race
window. The source's requested stale-password update remains a required saga,
not permission to describe an unguarded automatic write as race-free. The
weaker policy and an explicit implementation acceptance condition are
important so this safety default does not silently erase that deliverable.

The three-copy closure is consistent with the addendum's initially requested
encrypted laptop, authorized BWS project and authorized 1Password vault;
additional R2/future providers remain out of the set. A-2's bounded-use rule
can support the reference run only for a finite pre-enumerated signature set;
it must not turn the bundle digest into blanket authority for later or
unspecified signatures.

**Overall:** I **agree** with the revised parent architecture. A-1, A-2 and A-3
are closed at the architecture level. I do not soften the operational
condition: default unguarded updates must stop, weaker mode must disclose its
race, and implementation must still deliver the required history-preserving,
native-ID in-place rotation workflow under an explicitly defined policy.
