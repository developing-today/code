# Design decision record: residency evidence model (id)

- Date: 2026-11-09
- Status: **DECIDED** (queue task 13936). Design only; no implementation is included in this change.
- Issue: [#5876](https://github.com/developing-today/code/issues/5876) · Parent: [#5683](https://github.com/developing-today/code/issues/5683) · Programme: epic [#2056](https://github.com/developing-today/code/issues/2056)

## Question

What counts as residency evidence — the declarations, assurance levels,
provenance, lifetime, and visibility rules that every boundary decision in
the #5683 framework cites?

## Parent decisions this record relies on (not re-decided)

From #5683 (Drew-ruled), unchanged by this record:

1. **Capabilities, never defaults.** Residency evidence and boundaries are
   opt-in capabilities a server/world admin or user enables. Nothing here
   turns a restriction on by default, and no deployment inherits a boundary
   it did not choose.
2. **Basic frameworks first.** This record specifies the declaration,
   evidence, and enforcement skeleton in its simplest useful form — no
   policy language, automated classification, or jurisdiction-specific rule
   packs.
3. **Framework vocabulary.** Declarations are evidence with provenance;
   self-declared, org-attested, and document/provider-verified
   declarations are different assurance levels; IP evidence is weak
   evidence with known error modes.

Nothing in this record changes a parent decision. Where an implementation
choice would have to, it is flagged as a parent-level question, not forked
here. No divergence from #5683.

## Grounding (verified fresh 2026-11-09, task 13936)

- The evaluation vocabulary this model plugs into is
  `pkgs/id/crates/id-core/src/residency.rs` (the #3398 step-1 evaluator):
  an `Evidence` is an issuer-signed assertion bound to
  (resource, plane, jurisdiction, source, expires_at, serial); a `Policy`
  trusts evidence sources per plane (`trust_evidence_source`) and issuers
  as trust roots; evaluation is default-deny and a plane with no acceptable
  evidence is unknown/denied, never assumed.
- No declaration/assurance/provenance handling exists anywhere in `pkgs/id`
  today (grep over `pkgs/id/src` and `pkgs/id/crates/id-core/src` for
  declaration/assurance/attestation finds only unrelated hits). This record
  is the first artifact defining it.
- Siblings consumed: #5877 (public-IP evidence) defines IP observations as
  an observation class mapping to the lowest assurance tier — this record
  does not re-decide that, it assigns the tier; #5883
  (`doc/2026-11-08T09-15-00Z_design_boundary_audit_appeals.md`) logs
  evidence _classes_ with assurance levels, never the dossier — this
  record defines what those classes are.

## Decision

### 1. Declaration record (chosen shape)

A residency declaration is a signed record with exactly these fields:

- **subject:** the user, org, server, or world the claim is about.
- **claim:** claim type (`residency` or `citizenship`) plus one
  jurisdiction value (e.g. `us`, `eu`). Multi-jurisdiction subjects hold
  multiple declarations, never one declaration with a list.
- **assurance:** one of §2's three levels. Stored, never inferred.
- **provenance:** who declared it, when, on what basis, and what (if
  anything) verified it — declarant identity, attester/verifier identity
  (a configured trust root), basis reference (e.g. document type checked,
  attestation reference — a reference, never the raw document), and
  `verified_at`. Maps onto `Evidence{issuer, source, serial, signature}`:
  the issuer's signature over these fields is what makes the declaration
  an `Evidence` the evaluator accepts.
- **lifetime:** `expires_at` (mandatory), `serial` for revocation checks
  via `EvidenceVerifier::revoked`. Renewal is a new superseding
  declaration; there is no in-place lifetime extension.
- **visibility class:** §5's disclosure rule for this declaration,
  chosen at issuance from the fixed set (exact / boolean / sealed).

A boundary declares, per scope, which claim types and minimum assurance
levels it requires. A declaration satisfies a boundary only when subject,
claim type, jurisdiction, assurance floor, provenance trust, and lifetime
all check out at evaluation time.

### 2. Assurance levels (chosen: three, ordered)

- **L0 — self-declared:** the subject asserts their own
  residency/citizenship. No independent verification. Cheapest to issue,
  weakest to rely on.
- **L1 — org-attested:** an organisation the operator trusts attests the
  subject's claim (employer, school, member org). The attester identity is
  part of provenance; the operator decides which attesters are trust
  roots.
- **L2 — document/provider-verified:** a document check or provider
  verification the operator trusts (government ID check, verified provider
  record). The basis reference names the check; raw documents are never
  stored in the evidence store.

Higher assurance satisfies a lower floor; lower never satisfies higher.
Public-IP observations (sibling #5877) sit **below L0**: corroboration and
step-up signal only, never a declaration, and never satisfying a boundary
that requires a declaration.

### 3. Provenance (chosen)

Every declaration records: declarant (who made the claim), verifier (which
trust-rooted issuer signed it — for L0, the subject's own identity with a
marker that no third party verified it), basis (what was checked: "self
assertion", "org attestation ref X", "document check type Y" — references,
not contents), `verified_at`, and `serial`. The evaluator checks exactly
what `residency.rs` already checks: issuer in trust roots, signature valid,
not revoked, bound to the right resource/plane, source trusted for the
plane. A peer's self-assertion presented as third-party evidence fails the
trust-root check — the #3398 rule that a signature proves authorship, not
location, applies unchanged.

### 4. Lifetime (chosen)

- Every declaration has a mandatory `expires_at`. No indefinite
  declarations. Recommended defaults are an implementation choice, but the
  field must always be present and enforced (`now >= expires_at` denies,
  per the existing evaluator).
- **Renewal** issues a new declaration with a new serial superseding the
  old; the old serial may be revoked at renewal or left to expire.
- **Revocation** is via `EvidenceVerifier::revoked` on the serial (issuer
  revokes, org withdraws attestation, document invalidated). Revoked
  denies immediately, including mid-session re-checks.
- **Expiry/revocation effect:** deny new grants relying on the declaration
  and re-evaluate existing access at the next enforcement point
  (login, join, grant exercise, policy re-check). There is no silent
  grandfathering of access beyond expiry: continued access needs a current
  declaration. The audit log records the expiry-driven denial as a new
  entry (per #5883), so the subject sees what changed and what would
  restore it (renew, supply higher assurance).

### 5. Visibility (chosen: boolean by default, exact by exception)

- **Default — satisfies-boundary boolean:** checkers, enforcement points,
  and peer scopes learn only whether the subject satisfies a named boundary
  ("satisfies scope S residency requirement: yes/no"). They never see the
  jurisdiction value, the basis, or other declarations.
- **Exact value — need-based only:** the jurisdiction value (and provenance
  metadata) is visible to the subject themselves and to a scope admin
  reviewing a decision about that subject — and only inside the scope that
  stores the declaration. This is what makes admin review and appeal (#5883)
  possible without leaking the value everywhere.
- **No global dossier:** declarations live in the scope that collected
  them (server / world / account scope). There is no cross-scope or global
  residency registry. The audit log references evidence _classes and
  assurance levels_ ("L1 org-attested declaration"), never raw values.
- Raw identity documents are never stored in the evidence store at all —
  only the verifier's signed assertion that a check passed.

## Alternatives rejected and why

| Option                                                                                             | Why rejected                                                                                                                                                                                                                                                             |
| -------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Bare boolean/flag ("is-resident: true") with no provenance or assurance                            | Collapses L0/L1/L2 into one bit; boundaries cannot state an assurance floor, appeals cannot distinguish "renew" from "supply stronger evidence", and audit cannot reproduce the decision. Directly contradicts the parent's "stores the level of assurance" requirement. |
| Central global residency registry shared across scopes                                             | Creates the sensitive dossier the parent explicitly forbids; violates scope-local visibility and the "no global log" rule #5883 already decided.                                                                                                                         |
| Storing raw identity documents in the evidence store                                               | Turns the evidence store into an identity-theft target and a dossier by construction; unnecessary — the evaluator needs the issuer's assertion, not the document.                                                                                                        |
| Auto-promotion (a declaration "ages into" higher assurance, or IP corroboration upgrades L0 to L1) | Assurance measures verification actually performed, not confidence felt. Silent upgrades let weak evidence satisfy strong boundaries. Upgrades happen only by issuing a new declaration at the higher level.                                                             |
| Indefinite-lived declarations                                                                      | Access granted on stale facts never lapses; moves, expiries, and withdrawn attestations silently persist. Mandatory expiry plus revocation is the mechanism the evaluator already enforces.                                                                              |
| Exact-value disclosure to every checker                                                            | Every enforcement point becomes a copy of the dossier. The boolean-by-default rule gives checkers exactly what they need (admit/deny/step-up) and nothing more.                                                                                                          |
| IP-alone as a declaration                                                                          | Re-decides sibling #5877, which this record must not fork: IP is an observation class with known spoofing/NAT/egress error modes, usable for corroboration and step-up, never load-bearing.                                                                              |

## Invariants implementation must hold

1. Opt-in only: no declaration is collected and no boundary is enforced for a scope that did not enable it. Default state is off.
2. Assurance is stored and ordered (L2 > L1 > L0 > IP-observation); higher satisfies lower, never the reverse; nothing auto-promotes.
3. Every declaration carries provenance (declarant, verifier/trust root, basis reference, verified_at, serial) and mandatory expiry; missing fields deny.
4. Expiry and revocation are enforced at every check, including mid-session re-checks and queued retries; expired/revoked denies.
5. Checkers see the satisfies-boundary boolean by default; exact values only for the subject and the deciding scope's admin.
6. Declarations are scope-local; no global registry; audit entries cite classes and levels, never raw values or other subjects' evidence.
7. A signature proves authorship, not location; untrusted issuers and peer self-assertions presented as third-party evidence deny.
8. Every denial or step-up cites the evidence classes consulted and what would change the outcome (renew, higher assurance, correct the record).

## Splits

None. Implementation timing is the issue's own reserved decision, per its
framing. If sanctions (#5878), intranet (#5879), escrow/proxy (#5881), or
enforcement (#5882) designs need richer attestation formats, they file
their own follow-ons and link back here — this issue does not grow to hold
them.

## Worked mini-example

A server enables a sanctions boundary requiring L1+ residency declarations
for membership. Alice supplies an L1 org-attested declaration (issuer: her
employer, a configured trust root; expires in 1 year). The join check sees
only "satisfies: yes". Six months later the employer withdraws the
attestation → serial revoked → next re-check denies with reason
"org-attested declaration revoked; renew or supply L2". The audit log
records the class ("L1 declaration, revoked"), not Alice's jurisdiction
value. Bob, who has only an IP observation and no declaration, cannot join
on IP alone; the explanation tells him what would change it (supply an L1+
declaration).
