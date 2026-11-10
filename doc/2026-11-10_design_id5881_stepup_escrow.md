# Design: 2FA step-up gates & org-escrowed credential proxying (id)

- Date: 2026-11-10
- Status: **DECIDED** (queue task 14333). Design only; no implementation is included in this change.
- Issue: [#5881](https://github.com/developing-today/code/issues/5881) · Parent: [#5683](https://github.com/developing-today/code/issues/5683) · Programme: epic [#2056](https://github.com/developing-today/code/issues/2056)

## Question

Within the #5683 residency & boundaries framework, how do 2FA step-up gates
and org-escrowed credential proxying work — what may be escrowed, what is
proxied, what is audited, and what can the user see?

## Parent decisions this record relies on (not re-decided)

From #5683 and its framework record (`doc/2026-11-08_design_id5683.md`),
unchanged by this record:

1. **Capabilities, never defaults.** Step-up gates and escrow/proxying are
   opt-in capabilities an org or server admin enables in a declaration.
   Nothing here turns either on by default.
2. **Step-up shape (§6.3).** The declaration explicitly enumerates the events
   requiring step-up and states the granted window's duration; no event or
   duration is implied by enabling a boundary. Step-ups are logged.
3. **Escrow shape (§6.4).** The org escrows credential material under its own
   control; the server receives *assertions*, not raw values, wherever an
   assertion suffices. Every proxy use is audited with claim kind, assurance
   level, issuer, recipient, declaration version, and an opaque assertion
   reference — never the raw value. The user can see what was shared.
   Revocation is possible and the server then re-evaluates.
4. **Consent must be configured (§6.4, §11).** The consent model must be
   explicitly selected before any credential is escrowed or assertion
   proxied; missing consent configuration means no proxy use is authorized.
5. **Evidence vocabulary (#5876).** Assertions carry the framework's
   assurance levels and provenance; a proxied assertion is evidence with an
   issuer, a subject, and a lifetime, not a bare boolean the server must
   take on faith.

**One explicit scoping decision of this record (flagged, not silent):** D4
sets a framework floor inside the parent's consent menu — org-authority
alone never authorizes proxying *user-owned* credential material. The
parent (§11) leaves the consent choice to each declaration; this record
narrows that menu for user-owned material only, and the narrowing is
recorded here and on issue #5881. Org-owned material (service accounts the
org itself issued and owns) is unaffected.

## Decision summary

| Question | Decision |
| --- | --- |
| Step-up gate | D1: a gate is (declaration-listed event, required factor class, window scope); windows are per (subject, server, event class) and expire into re-step-up, never into denial of ordinary activity |
| Escrow contents | D2: org-issued or org-attested material and evidence originals; never the sole copy of any user credential; never user passwords or raw biometrics |
| Proxied assertions | D3: signed, recipient-bound, short-lived, minimal claims (kind + assurance + boolean) — raw values only when no assertion suffices, the declaration's privacy rule names the value and reader, and consent covers it |
| Consent | D4: standing consent scoped per claim kind as the default; per-use consent for raw-value reveals; org-authority alone sufficient only for org-owned material |
| Audit | D5: every step-up (attempt and result) and every proxy use logged with the parent's field set; no raw values in any log |
| User visibility | D6: the user can always enumerate escrowed contents, consent state, proxied assertions, and step-ups for their own account — this visibility is a floor an org policy cannot switch off |
| Degraded behaviour | D7: assertion or step-up unavailable → the *gated action* fails closed, ordinary activity is untouched; break-glass (§6.5) is the only bypass |

## D1 — Step-up gates

- A **gate** is a triple in the enabling declaration: an *event class*, a
  *required factor class*, and a *window*. Event classes are the parent's
  list — entering from a new network, privilege escalation, admin actions —
  extended by two classes this record adds because they spend other
  people's trust: **changing consent or escrow contents**, and **any
  raw-value reveal** under D3. Break-glass use always requires step-up and
  never rides a window granted for anything else.
- The **factor class** names what satisfies the gate: a second factor
  distinct from the factor that opened the session, drawn from the org's
  IdP factors (WebAuthn/passkey preferred; TOTP acceptable; SMS never
  sufficient for admin actions or raw-value reveals). The gate checks
  *recency of a strong authentication*, it does not re-implement 2FA
  inside the server — the IdP performs the factor, the server consumes a
  signed step-up assertion (D3 shape) naming the event class and time.
- A granted **window** is scoped to (subject, server, event class), has
  the duration the declaration states, and is not transferable across
  servers or event classes. Expiry means the next gated action steps up
  again (parent §6.3); it never locks the session or downgrades ordinary
  activity.
- **Failure** is bounded: a failed or abandoned step-up denies the gated
  action, leaves the session alive, and is logged; repeated failures are
  rate-limited per the declaration and surface to the user (D6), so a
  third party fumbling a victim's gate is visible to the victim.
- The step-up result is recorded as framework evidence (issuer = IdP,
  assurance = the factor class achieved, lifetime = the window), so
  downstream boundary evaluations cite it like any other evidence.

## D2 — Escrow contents

Escrow is an **attestation store, not a credential vault for the server's
convenience**. What may be escrowed, per subject, in per-subject
compartments encrypted under org keys:

1. **Org-issued credential material** — credentials the org itself issued
   (org IdP accounts, org certificates), where the org is the issuer and
   could re-issue at will.
2. **Evidence originals** — the documents or provider results behind
   #5876 declarations (e.g. the residency attestation an assertion cites),
   so an assertion's provenance is inspectable by an authorized auditor
   without the server ever holding the original.
3. **Recovery shares** — one share of a split recovery secret for the
   user's factors, where the remaining share(s) stay with the user. The
   org's share alone reconstructs nothing.

Never escrowed:

- The **sole copy** of anything. Escrow never becomes the only holder of
  a user credential; the user always retains independent access. An
  escrow arrangement that would flip ownership of a credential to the
  org is invalid under this design.
- **User passwords or password equivalents** for services the org did
  not issue, and **raw biometric samples** (a factor's public key or
  attestation is not a sample and may be recorded as evidence metadata).

## D3 — Proxied assertions

- An assertion is a signed statement with the framework's evidence
  fields: claim kind, assurance level, issuer (the org), subject,
  recipient (one named server), issue time, expiry, and an opaque
  reference back to the escrow record it rests on.
- **Claim kinds** in the skeleton: `member-of` (group/role),
  `residency-attested` (the subject satisfies a named boundary — a
  boolean, never the underlying value, per the parent's privacy rule),
  `identity-verified` (at a named assurance level), and `stepped-up`
  (event class + window, from D1).
- Assertions are **short-lived** (minutes, set by the declaration),
  **recipient-bound**, and **non-transferable**: a server cannot relay
  one onward as its own evidence; federation peers evaluate under the
  parent's §8 rules, not by trusting a forwarded assertion.
- **Raw-value proxying is the exception path**, available only when all
  three hold: no assertion kind suffices for the server's stated
  requirement; the declaration's explicit privacy rule names the exact
  value and the named reader; and consent (D4) covers that reveal.
  Every raw-value reveal is per-use consented, stepped-up (D1), and
  audited with the value's *identity*, never the value (D5).

## D4 — Consent model

- **Default: standing consent, scoped per claim kind.** The user grants
  (or the enrolment flow records) consent per claim kind and per
  recipient class; a proxy use outside the granted kinds does not
  happen. Consent state is part of the user's visible record (D6) and
  revocable at any time (parent §6.4 un-escrow).
- **Per-use consent** is required for: raw-value reveals (D3), and the
  first proxy use of any newly added claim kind.
- **Org authority alone** authorizes proxying only *org-owned* material
  (D2 class 1, where the org is issuer and owner). For user-owned
  material it is never sufficient (the flagged scoping decision above).
- Consent is recorded as evidence: who consented, when, to which kinds,
  under which declaration version. Expired or absent consent fails the
  proxy use closed (D7), it does not degrade to a weaker claim kind.

## D5 — Audit

- **Proxy uses** record exactly the parent's field set: claim kind,
  assurance level, issuer, recipient, declaration version, opaque
  assertion reference, timestamp, and the consent record it rested on.
- **Step-ups** record: subject, server, event class, factor class
  required and achieved, result (granted / failed / abandoned), window
  granted, timestamp.
- **Escrow administration** (contents added/removed, consent changes,
  revocations) is audited the same way, with the actor named.
- **No raw values, ever**: not credential material, not assertion
  values, not factor secrets. An audit entry that would need a value to
  make sense is mis-designed; the opaque reference is the join key for
  an authorized auditor.
- Retention is stated in the declaration (parent §11 evidence-retention
  choice); the user's own view (D6) covers at least the retention
  window the org itself keeps.

## D6 — User visibility

The user can, at any time and without org approval, see for their own
account:

- what is escrowed (D2 class and item identity, not other subjects'
  contents),
- their consent state per claim kind and when it changed,
- every assertion proxied (kind, recipient, assurance, when),
- every step-up (event class, result, when), including failures,
- and can **revoke** consent and request un-escrow from the same view.

Revocation takes effect for new proxy uses immediately; assertions
already issued run out their short TTL (D3) and are not retroactively
falsified — the server re-evaluates at next use, per the parent. This
visibility is a **floor**: an enabling declaration or org policy may
add visibility, never remove this baseline. A design in which the org
can proxy invisibly is the dossier failure the parent's §3.3 warns
against, and is rejected here by construction.

## D7 — Degraded behaviour

- Escrow or IdP unreachable, assertion service down: the action that
  needed the assertion or step-up **fails closed** — it does not
  proceed on stale or assumed claims. Ordinary, ungated activity is
  unaffected; a proxy outage must not become a server outage.
- The only bypass is the parent's break-glass capability (§6.5):
  separate opt-in, named role, always stepped-up (D1), logged loudly,
  and surfaced to admins and to the affected user (D6).

## Invariants implementation must hold

1. No step-up event or window exists that a declaration did not state.
2. No proxy use without a covering, unexpired consent record.
3. No raw value in any assertion, audit entry, or log.
4. Escrow is never the sole copy of a user credential.
5. A window never crosses servers or event classes.
6. Revocation stops new proxy uses immediately.
7. The user's D6 view is complete: every audit entry about a subject
   is visible to that subject in at least the form the org keeps.

## Alternatives rejected

- **Forward raw credentials to the server.** Rejected by the parent
  (§6.4: assertions wherever an assertion suffices); it makes every
  enabled server a credential holder and a breach target.
- **Per-use consent for everything.** Rejected as the default: it
  trains click-through and makes standing intranet use unusable. Kept
  where the stakes are per-event (raw-value reveals, new claim kinds).
- **Org authority alone for all escrow.** Rejected for user-owned
  material (D4): enrolment in an org is not blanket consent to spend a
  user's credentials invisibly.
- **Server-implemented 2FA.** Rejected: step-up consumes the IdP's
  factor as a signed assertion; a second, server-local factor store
  would split the user's factor lifecycle across every server.

## Open points routed back to #5683

- Whether a stepped-up window may satisfy a *residency* boundary's
  evidence bar, or only intranet-mode gates — this record uses step-up
  only for gates and consent/escrow actions, and leaves boundary
  evidence bars to #5876's model.
- Retention periods for step-up and proxy audit after account closure
  follow the parent's §11 retention choice; no period is invented here.
