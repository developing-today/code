# Identity, login & delegation — parent synthesis for #5684

- Date: 2026-11-23
- Status: **synthesis record** — design only; no implementation is authorised
  by this record alone. The one implementation filing this record makes is
  named in §11.
- Issue: [#5684](https://github.com/developing-today/code/issues/5684) —
  design(id): identity, login & delegation — simplicity-first key custody,
  security-code recovery, and pseudo-login identities. Programme epic
  [#2056](https://github.com/developing-today/code/issues/2056).
- Inputs, all closed with decision records landed:
  [#5884](https://github.com/developing-today/code/issues/5884) setup ceremony
  (`doc/2026-11-08T23-17-06Z_design_id_setup_ceremony/`),
  [#5885](https://github.com/developing-today/code/issues/5885) keyless-custody
  feasibility (decision record on the issue, 2026-11-08),
  [#5886](https://github.com/developing-today/code/issues/5886) variant (a)
  (`doc/2026-11-13_design_id5886.md`),
  [#5887](https://github.com/developing-today/code/issues/5887) variant (b)
  (`doc/2026-11-10_design_id5887.md`),
  [#5888](https://github.com/developing-today/code/issues/5888) variant (c)
  (`doc/2026-11-08T10-03-00Z_design_id_recovery_codes/`),
  [#5889](https://github.com/developing-today/code/issues/5889) variant (d)
  (decision record on the issue, 2026-11-08),
  [#5890](https://github.com/developing-today/code/issues/5890) management UX
  (decision record on the issue, 2026-11-09).
  Issuer rotation/compromise mechanics are
  [#2804](https://github.com/developing-today/code/issues/2804) (closed),
  consumed as an input throughout. Group-delegation security guarantees are
  [#6379](https://github.com/developing-today/code/issues/6379)
  (`doc/2026-11-23T17-47-48Z_design_id_delegation_threat_model/`); §9 states
  how the two delegation layers relate.

## 1. What this record does

The 2026-11-23 re-packet triage of #5684 (task 16820) verified that all
seven decomposition children have landed their decision records, and named
the parent remainder as **integration, not more design**: (1) one synthesis
record linking the child records against this issue's six acceptance items,
naming any genuine gap rather than assuming coverage; (2) the implementation
issue the #5885 record assigns to this parent. This document is (1); §11
records (2).

This record introduces **no new variant decision**. Where the child records
overlap or conflict at their seams, §8 records a reconciliation and flags it
as a decision of this synthesis, so a reader can tell parent-level rulings
from child-level ones. Everything else is citation.

## 2. The settled model, in one page

- **The native ID is a keypair.** Identity = an Ed25519 keypair generated on
  the user's device from the OS CSPRNG; the public key (or its hash) is the
  identifier; no registry mints or revokes it (#5884 §3).
- **Four concepts are kept visibly separate** (#5890 §1): the ID (the key),
  custody (who can reach the private key), sign-in (how a person proves
  themselves to a service), and delegated logins (separate pseudo-login
  identities acting under a grant). A Google/OIDC sign-in is not evidence
  that a service holds the ID key.
- **The real key signs rarely:** the genesis artefacts (#5884 §5),
  delegation grants, adoptions and revocations (#5889), recovery and
  regeneration authorisations (#5888), and rotation statements (#2804).
  Day-to-day activity is signed by pseudo-login identities under scoped,
  expiring, revocable grants (#5889 §1–§2).
- **Login is ordinary; custody is disclosed.** The user sees Google sign-in,
  username/password, or OIDC. For every custody variant, the product states
  in plain language who can sign as the user, who can lock them out, and who
  can recover them — including when the answer is "nobody can" (§6).
- **Acceptance never comes from custody.** Later acceptance verifies a
  signature against the established public key and signed artefacts (§4),
  never from a party holding the private key. That is what makes variants
  (b) and (d) possible at all — and it cannot recover a key that no longer
  exists anywhere (#5885).

## 3. Acceptance map — #5684's six items against the records

| #5684 acceptance item | Satisfied by | State |
|---|---|---|
| Design document covering the native ID, variants (a)–(d), and the seven flows, linked from the issue | This synthesis (§§2, 5–7), over #5884 (native ID), #5886 (a), #5887 (b), #5888 (c), #5889 (d) | Done — this record |
| Feasibility matrix: login method × custody variant, classified, construction or blocker named | #5885 decision record (reproduced in §5) | Done |
| Signed-hash construction: what is signed at setup, what is stored where, verification flows | #5884 §§4, 8 (genesis); #5885 (login binding); reconciled in §4 and §8 R2 | Done |
| Per-variant plain-language "who can sign / lock out / recover" statements, incl. the all-codes-lost statement for (c) verbatim | #5886 §5 (a); #5887 §§4–5 + #5890 §3 (b); #5888 §7 (c); #5889 §3 + #5890 §3 (d); collected in §6 | Done |
| Delegation model for pseudo-login identities: ownership forms, scope, expiry, revocation, raw-view provenance | #5889 §§1–6; management view #5890 §2 | Done |
| Follow-on design tickets filed from the decomposition sketch | #5884–#5890, filed 2026-11-05, all now closed with records landed | Done |

No acceptance item is left unsatisfied. The genuine gaps this synthesis
found are below the acceptance line — cross-record seams (§8) and residual
open questions (§10) — and are named there rather than silently absorbed.

## 4. The signed-hash construction, as one construction

Two signed layers, kept distinct (reconciliation R2, §8):

1. **Genesis — the `SetupStatement`** (#5884 §4, canonical). At setup the new
   key signs a domain-separated statement
   (`SHA-256("id-setup-v1" ‖ canonical(SetupStatement))`, signed as
   `"id-setup-v1" ‖ setup_hash`) binding: the identity public key, the
   ceremony version, the chosen custody variant, and the SHA-256 of the
   generation-1 `RecoverySet`. `prev: null` marks genesis; exactly one may
   exist per identity. It is stored and replicated as a public signed
   ID-graph object alongside the `RecoverySet` (#5884 §8). Verification is
   five checks, offline given the artefacts, with no partial credit and no
   server attestation substituting for any check (#5884 §8).
2. **Login binding** (#5885). The client also signs a domain-separated
   binding statement (`"id/login-binding/v1"`) committing the ID public key
   to the authentication account — OIDC `(iss, sub)` with the subject
   hashed, or the OPAQUE account identifier — plus the custody profile and
   the recovery commitment hash. The authorizer stores the public key, the
   statement, and the signature; it never stores an unwrapped key.

Verification flows that rely on these artefacts:

- **Login / ID-authorised operations:** the authorizer issues a fresh
  challenge (nonce, audience, expiry); the client-held ID key signs it;
  acceptance is signature verification against the bound public key
  (#5885; specialised for variant (b) in #5887 §3). Authentication success
  alone never registers, replaces, or rebinds a key (#5885 invariants
  2–3, 5).
- **Recovery:** a code-derived recovery key signs a claim against the
  `RecoverySet` commitments; settlement after the 72-hour window installs a
  new key by rotation (#5888 §5). No login event substitutes.
- **Delegation:** the principal's real key signs the grant; verifiers
  thereafter verify the pseudo-login's signatures against the grant, never
  against anyone's custody of the real key (#5889 §2).

What the construction does **not** do, stated in the records and repeated
here because the parent requires the honesty: it does not prove where a key
was generated or that no copy exists (#5885); it cannot recover a key that
no longer exists anywhere (#5885, #5887 §3); and a browser authorizer can
serve hostile client code — a non-extractable WebCrypto key prevents
ordinary export, not use by malicious same-origin code (#5885). §10 carries
that boundary as a residual.

## 5. Feasibility matrix (from #5885, the record of authority)

"Keyless" means the authorizer never possesses or can use the user's ID
signing key. Proven-feasible = the construction is established (the
ID-specific integration is not thereby implemented);
feasible-with-caveats = a known composition works if the stated custody and
recovery conditions hold. A password-derived ID signing key without
independent high-entropy protection is **speculative/unsafe** and is not an
approved construction in any cell.

| Login | (a) Service-held key | (b) User-held key | (c) Security-code recovery | (d) Pseudo-login + delegation |
|---|---|---|---|---|
| Google / OIDC | Proven-feasible, but custodial; outside the keyless boundary | Feasible-with-caveats: OIDC authenticates the account; a client-held key signs. A new device needs the old device, a user-held backup secret, or another recovery path | Feasible-with-caveats: OIDC authenticates the account; codes are independent recovery authority — high-entropy, rate-limited at redemption, under #5888's threshold/rotation rules | Feasible-with-caveats, only when the pseudo-login has a client-generated key and the ID key signs a scoped delegation. OIDC alone cannot mint or revoke that delegation |
| Username / password | Proven-feasible, but custodial; outside the keyless boundary | Feasible-with-caveats with OPAQUE (RFC 9807) plus a client-held key/backup. Directly deriving the ID key from a human password is speculative/unsafe: a stolen verifier or encrypted blob is an offline guessing target and a KDF cannot add entropy | Feasible-with-caveats: password authentication coexists with independent codes; password reset must not silently rebind or recover the ID key | Feasible-with-caveats if the PAKE authenticates the account and the client holds the pseudo-login key; the parent ID signs the least-privilege delegation |

Answers to the parent's investigation questions, in one line each:
**OIDC with no key custody — yes**, feasible-with-caveats: the provider
authenticates, the client holds and signs with the key, the relying party
verifies challenge signatures against the bound public key (#5885).
**Username/password with no authorizer key custody — yes, via OPAQUE**,
feasible-with-caveats: the server stores an OPAQUE registration record and
ciphertext only; the RFC 9807 `export_key` composition wraps a *random* ID
key client-side. The brute-force exposure is named, not waved away: a
corrupted server holding the OPRF seed, or a stolen Argon2id-wrapped blob,
admits offline guessing whose cost the KDF raises but cannot make a weak
password strong (#5885, #5887 §6).

## 6. The four variants and their plain-language statements

### (a) Service-held keys — #5886

The service generates the key inside a non-exportable HSM/KMS-backed
signer, signs only on an authenticated user's request (step-up for
sensitive classes), and writes every signature to an append-only,
hash-chained, user-visible audit log. Variant (a) exists as the default
on-ramp, with the disclosure shown at setup and a real exit (§7 flow 6);
refusing it would exclude the convenience-first users the parent centres
(#5886 §8). User-facing statements (#5886 §5):

- **Sign as you:** "The service can sign as you. When you are signed in, it
  signs only what you ask it to, and every signature is recorded in your
  activity. If the service itself were compromised or acted against you,
  it could sign without asking — the record would show it afterwards, but
  the signature would be real."
- **Lock you out:** "The service can. If your account is suspended,
  deleted, or the service shuts down, you cannot sign until access is
  restored or you have migrated out. Nobody else can lock you out."
- **Recover you:** "The service can, through ordinary account recovery. If
  you lose your login, support processes can restore your access — the
  same process an attacker would target, which is why sensitive changes
  ask for a fresh sign-in."

The audit log **detects** signing the user did not intend; it cannot
**prevent** the service from signing, and no copy may present it as
prevention (#5886 §3).

### (b) User-held keys — #5887

The key is generated on the client and lives only as an encrypted blob
(Argon2id-wrapped under a key passphrase distinct from the login password)
in storage the user already has — by default the app-data folder of the
user's Google Drive, always with a downloadable file copy offered. The
authorizer authenticates the person, issues challenges, and holds only
public artefacts and at most blob ciphertext; decryption and signing happen
only on the client (#5887 §§1–3). A compromised authorizer is a
denial-of-service and metadata event, not an identity compromise (#5887
§6). Statements (#5890 §3, with #5887 §5):

- **Sign as you:** "The sign-in service checks who you are but never
  receives your signing key. Only someone with a copy of that key can sign
  as you."
- **Lock you out:** "The service can block its own sign-in, but it cannot
  sign for you or restore a key you have lost." Losing one blob copy is
  recoverable from the other; losing login/Drive access with a surviving
  copy is recoverable by re-binding a new login to the same public key,
  authorised by a signature from the unlocked key (#5887 §5).
- **Recover you:** "The person with another key copy or a configured
  recovery path — not the authorizer." If every blob copy is gone, or the
  passphrase is forgotten with no codes set up, the identity is
  unrecoverable — nobody can unlock it, not support, not the service, not
  the developers — and setup surfaces exactly those rows as an informed
  choice before the user commits (#5887 §5).

### (c) Recovery via 10 security codes — #5888

Ten independent, single-use codes, 100 bits each, Crockford base32 in five
groups of four. **Any one unused code recovers; there is no quorum, and the
threshold is not configurable per identity** (#5888 §4). At rest, only the
signed `RecoverySet` of slot commitments exists. Redemption is an anchored
claim, a 72-hour settle window in which the current key may cancel (cancel
wins), then a settle that installs a **new** key — recovery is rotation,
never rebind — and issues a fresh set of ten, invalidating the old set
whole (#5888 §5). Codes recover the root identity, never a pseudo-login,
and a key-generation change revokes delegations issued under the previous
generation (#5888 §§6.3, 8).

The parent-required statement, verbatim, mandatory in setup and recovery
copy (#5890 §3):

> **If all codes are lost (and no other recovery path was set up), nobody
> can unlock the account. Not support, not the service, not the
> developers. The identity is unrecoverable.**

The full setup-screen copy is #5888 §7.1 (shown before the codes are handed
over, gated by typing back one randomly chosen code — a real informed
choice, not a checkbox), and the at-loss screen is #5888 §7.2, which
forbids offering any support escape hatch the product cannot honour.

### (d) Pseudo-login identities & scoped do-as delegation — #5889

A pseudo-login is its own real ID. The canonical form is **user-owned**:
created under the principal's control, one per device or login method,
holding a **delegation grant** — the one new artefact in this design set —
signed by the principal's real key, naming scope (worlds × capabilities),
with mandatory expiry, optional use bounds, and tree revocation
(#5889 §§2–4). Every action resolves a 4-tuple — authenticating principal,
acting pseudo-login, grant, context — delegation only ever narrows, and
the raw record always carries the full tuple even where the friendly UI
renders the action as the user's (#5889 §§1, 6). **Login-derived /
nobody-owned** identities are session identities only: no standing scope,
no renewal without a fresh authentication event, revoked by deleting the
server-side session record — stated plainly: *nobody holds a key for this
login; it ends when the session ends, and it cannot be extended — only
replaced by logging in again* (#5889 §3). A login-derived identity may be
**adopted** — converted to user-owned by an explicit, user-visible grant
from the principal — never automatically. The service-held pseudo-login
form inherits variant (a)'s honesty statement verbatim (#5889 §3).

## 7. The seven flows, mapped end-to-end

1. **First-time setup via Google sign-in.** The ceremony is #5884's for
   every variant: generate on-device → attest the variant choice under its
   §6 disclosure → generate codes + transcription proof → sign the
   `SetupStatement` → sign and publish the `RecoverySet` → only then is the
   identity presented as created. Variant surfaces: (a) #5886 §4.1 (the
   service generates inside the signer instead, one disclosure screen, the
   choice recorded in the setup artefact); (b) #5887 §4.1 (blob built and
   stored, file backup and codes offered). Variant (d) is post-genesis:
   the first grant is an optional artefact after the identity exists
   (#5884 §5).
2. **First-time setup via username/password.** Identical from key
   generation on; the account is created through OPAQUE so the server
   never sees a reusable password verifier (#5885). The login password and
   the variant-(b) key passphrase are separate secrets, and the UX says
   which is which at the moment each is created (#5887 §4.1).
3. **Everyday login on a new device.** (b) #5887 §4.3: sign in → fetch the
   blob → enter the passphrase → the client unwraps and signs the
   enrolment challenge naming the requested delegation scope → the new
   pseudo-login appears in the management view. (a) #5886 §4.2: ordinary
   login; the live delegation does the work. On the same device, the live
   pseudo-login delegation carries day-to-day use and the real key is not
   needed until a sensitive operation (#5887 §4.2). Session activation
   under a grant is the pseudo-login signing the verifier's challenge —
   distinct from request authentication and from invite proofs, and none
   of the three substitutes for another (#5889 §5).
4. **Recovery with a security code.** #5888 §5 end-to-end: enter one code
   on the recovering device → anchored claim (the code itself is not
   consumed by claiming) → notification to every registered channel →
   72-hour window, cancellable by the current key → settle installs the
   new key, consumes the slot, publishes a fresh set of ten. The
   all-codes-lost dead end is #5888 §7.2's screen, verbatim in §6 above:
   no reset exists and none may be offered.
5. **Creating, viewing, scoping, revoking a pseudo-login delegation.**
   Creation and the plain-language confirmation ("this login can do X in
   worlds Y until Z") are #5889 §4; the management view — one row per
   pseudo-login with form, scope, expiry, last use, and a revoke control,
   session-only logins in a separate "Active sessions" section — is
   #5890 §2. Revocation is effective at next verification; actions already
   taken stand, with provenance (#5889 §4). Killing a compromised
   pseudo-login touches nothing about the real identity or its other
   pseudo-logins.
6. **Migrating between custody variants.** a→b and any→a are required by
   the parent; the mechanism is reconciled in §8 R1: **a→b is a rotation**
   under #2804 (#5886 §6) — the service signs the rotation statement on
   the user's step-up-authenticated request, the service key retires and
   refuses all further signatures, and the user verifies the successor key
   before the flow reports success. b→a is the same construction in
   reverse with the §6(a) disclosure shown again (#5886 §6), or a same-key
   handoff where the user hands over the key they hold (#5890 §1). Two
   limits are stated, not smoothed over: migration is not a lockout remedy
   (an account that cannot authenticate cannot migrate, #5886 §6), and
   **no guaranteed exit from a hostile variant-(a) custodian exists in
   this design set** — the service can refuse the rotation, and can cancel
   a code-recovery claim before it settles (#5888 §8). Promising one would
   require a new parent-level decision.
7. **Issuer / privileged identities.** This design set invents no parallel
   mechanism: issuer rotation and compromise recovery are #2804's, and the
   user-facing records compose with them — variant (a)'s migration and
   rogue-signature recovery are #2804 cases with the §3 audit log as the
   evidence trail (#5886 §§6–7), and #5888's settle uses #2804's
   successor-proof pattern from `issuer_rotation.rs`. Privileged
   identities hold their keys under the same custody variants; what
   differs is what their signatures authorise, not how the keys are kept.

## 8. Cross-record reconciliations (decisions of this synthesis)

The child records were written in parallel and three seams plus one
cross-link needed a parent-level ruling. Each ruling below is flagged so
it can be revisited without disturbing the child records it reconciles.

- **R1 — a→b migration is rotation, not same-key handoff.** #5890 §1
  defines a custody migration as a same-key handoff; #5886 §§2, 9 make the
  variant-(a) key non-exportable — no path returns it in any form — so a
  same-key handoff out of (a) is impossible by construction. **Ruling:**
  #5890's handoff governs where the source key is exportable in the user's
  hands (b→a transfer; moving a (b) key between devices). a→b uses the
  #2804 rotation construction (#5886 §6), and recovery-settle is the same
  rotation entered through #5888. This satisfies, rather than violates,
  #5890's underlying rule — a changed key must not silently keep the ID
  label — because #2804 rotation is exactly the explicit
  succession/continuity mechanism #5890 requires, and #5888 §6.3 supplies
  the consequence #5890 states: grants do not carry across a key change;
  prior-generation delegations are revoked at settle. (#5890's caution on
  this point was written while #5888's PR #7622 was still open; that
  record has since landed, and this ruling consumes it as settled.)
- **R2 — one genesis artefact.** Four records sketch a setup statement
  (#5884 §4; #5885's binding hash; #5886 §4.1; #5887 §3) with overlapping
  but different field sets. **Ruling:** #5884's `SetupStatement` is the
  single genesis artefact — it owns the ceremony and the construction by
  the parent's decomposition. #5885's issuer/subject commitment and
  #5887's blob commitment are fields of the separate signed **login
  binding** and variant-(b) enrolment records layered after genesis
  (§4), not competing genesis statements. Implementation must produce
  exactly one genesis shape, with the cross-implementation `setup_hash`
  agreement oracle #5884 §12 sets.
- **R3 — codes do not unwrap an existing key.** #5887 §5 describes a
  forgotten passphrase being recovered through a "code-wrapped second
  blob" of the same key. #5888 — the variant-(c) owner — settles
  otherwise: a code's only power is to replace the key (§§5, 6.2), via
  claim and settle, ending with a fresh set. **Ruling:** #5888 governs.
  The forgotten-passphrase-with-codes path is: code → claim → settle
  installs a new key → the user sets a new passphrase and a fresh blob is
  created for the new key. The user-visible outcome #5887 promised (back
  in, with a new passphrase) is unchanged; the mechanism is #5888's, and
  no code-wrapped copy of an existing key is created.
- **R4 — the one-time login token lands in variant (d)'s session form.**
  Drew's mechanism, cross-linked onto #5684 (2026-11-05): an
  already-authenticated surface (CLI or web login) mints a one-time token
  to carry that authentication into a browser session. #5889 §7 already
  decided the composition, and this synthesis adopts it at parent level:
  redeeming the token creates (or attaches to) a **login-derived
  pseudo-login session** — single-use, short-lived, no standing do-as
  scope, revocable by ending the session. Standing scope for that browser
  requires adoption (#5889 §3). The token transports an authentication
  that already happened; it changes no custody answer.

## 9. Consistency with #6379 (group delegation threat model)

#6379 and this design set govern **different delegation layers**, and the
synthesis states the boundary so neither is read as the other:

- #6379 covers **group grants** judged by the fold in `id-events` from the
  signed group log. #5889 covers **login delegation grants** verified at
  session/action time. They share principles, deliberately: attenuation —
  a grant conveys no scope its grantor did not hold (G1) — is #5889's
  invariant 3 ("delegation only narrows"); #6379's D2 grantor veto
  parallels the principal's revocation authority in #5889 §4; both keep
  provenance in the raw record.
- **Expiry differs, on purpose.** #6379 D1 adopts an *optional* signed
  `expires_at` for group grants, with interim guarantee G9 — a group grant
  has no expiry until #13534 lands. #5889 makes expiry **mandatory** for
  login delegation grants. Neither satisfies the other: an implementation
  must not treat group-grant behaviour as evidence the login-grant rule
  holds, or vice versa.
- **Determinism vs liveness.** #6379 G8 forbids judging inputs that are
  not signed log fields. #5889's verification-time liveness check (with
  only short-TTL caching) is a session-layer property of login grants, not
  a fold input; the two do not conflict because they judge different
  artefacts.
- **Custody boundary.** #6379's TB2 — the account is its keypair file, and
  anything that can read the file can act as the account — is the ground
  fact variants (b) and (c) are designed around: (b) wraps that key so the
  file alone is ciphertext, and (c) exists for when the key is gone.

## 10. Parent open questions — dispositions

| #5684 open question | Disposition |
|---|---|
| Is Google Drive an acceptable key home for variant (b), or does file-based custody fail simplicity? | Answered — #5887 §8: Drive as the default home passes (the user never handles a file; the blob's residence is invisible plumbing over ciphertext); file-only custody as the normal path would fail. Residual wording risk goes to non-technical tester validation in implementation |
| Password-derived keys: disqualifying, or do PAKE/KDF constructions make it feasible-with-caveats? | Answered — #5885: direct derivation is speculative/unsafe and rejected; OPAQUE over a random client-held key is feasible-with-caveats, with the offline-guessing exposure stated |
| Nobody-owned pseudo-logins: in what sense is the delegation still a cryptographic artefact; does it weaken raw provenance? | Answered — #5889 §3: it is not a delegation holder at all; it is session state, holds no standing scope, and the provenance guarantee applies in full to grant-backed actions. Session actions carry session provenance, labelled as such in the management view (#5890 §2) |
| How many pseudo-logins will a user accumulate before the management view breaks simplicity? | **Partially answered — the one residual.** #5890 §2 designs the view to scale by shape (one row per pseudo-login, grants grouped, sessions separated) but no record sets a numeric bound or a consolidation rule. Carried to the management-UX implementation as a validation item, not silently assumed away |
| Should variant (a) exist at all? | Answered — #5886 §8: yes, as the disclosed default on-ramp with audit and a real exit |
| What threshold for the 10 codes — any-one vs quorum — and can the user choose? | Answered — #5888 §4: any-one, single-use, not per-identity configurable; the stolen-code case is bought down by the settle window, cancellation, and notification instead |
| Does the signed-hash setup artefact create a permanent linkage/fingerprinting surface? | Answered — #5884 §9: the artefact binds one identity's genesis and introduces no identifier beyond the public key the identity already is; mitigations are domain separation, hash-not-contents commitments, and day-granularity `created_at`. Residual (genesis timing is visible to chain holders) accepted and stated |

One further residual, from #5885 rather than the parent's list: the
**browser trust boundary** (§4) — a web authorizer serving hostile client
code can use, though not export, a non-extractable key. No construction in
this design set removes it; native clients do not have it. It is recorded
here so the implementation programme cannot claim it away.

Also closed by landing: #5884's one provisional point. It consumed
#5888's `RecoverySet` shape from then-open PR #7622 and flagged its §6 for
revisiting if the shape changed in review. PR #7622 has merged; the landed
`RecoverySet { identity, set_generation, slots[10] of SHA-256 commitments,
created_at }` is the shape #5884 consumed. No revisit is owed.

## 11. Implementation programme (named slices; one filing made)

**Filed by this synthesis** — the follow-on #5885's record assigns to this
parent ("#5684 must link an implementation issue that chooses the supported
OIDC and/or OPAQUE flow and tests the invariants above"):
**[#13710](https://github.com/developing-today/code/issues/13710) —
impl(id): keyless-custody login — OIDC account-pointer binding and OPAQUE
password login over a client-held ID key** (enqueued in the same pass as
queue task 21509). The choice from the §5 matrix: both
flows, over the one client-held-key shape — OIDC (Google) as the primary
sign-in, OPAQUE for username/password — covering binding-statement
enrolment and verification, challenge sign/verify at login and new-device
enrolment, and the six #5885 invariants as test oracles. (The idp-provider
scaffold under #13658 is the *provider* side — id acting as an issuer —
and is a different workstream from this login-consumption slice.)

Named here, filed separately as the programme is sequenced (each child
record names its own; this list is the parent's index, not new tickets):

1. Setup ceremony implementation — #5884 §13 items 1–5: the explicit setup
   flow over `load_or_create_keypair`, the `SetupStatement` type with
   sign/verify and its property-test oracles, the ceremony state machine
   enforcing the §7 invariants, the codes print screen consuming #5888
   §7.1 copy verbatim, and verifier support for the five-check setup
   validation.
2. Variant (a) signing service — #5886 §11 item 1: per-identity HSM/KMS
   keys, signing policy (ordinary vs step-up classes), retirement, and
   the audit log with user activity view and export.
3. Custody migration flows — #5886 §11 item 2, as reconciled by §8 R1:
   the #2804 rotation construction in both directions, including the
   successor-key verification step.
4. Variant (b) blob and unlock — #5887 §10: the blob format in `pkgs/id`,
   KDF parameter selection measured on target devices, lock-on-idle and
   memory hygiene, wrap/unwrap and tamper-refusal property tests; plus
   the setup/new-device UX validated with non-technical testers.
5. Recovery codes implementation — #5888 §§3, 5, 13: code derivation and
   the `RecoverySet`, claim/settle with anchored ordering and the 2-of-3
   witness time certificates, cancellation, rate limits, notification,
   and the §7 screens with their verbatim copy.
6. Pseudo-login delegation — #5889: the delegation grant artefact and its
   verification (the §1 4-tuple check), challenge signing kept separate
   from request authentication and invite proofs, adoption, and
   tree revocation wired to the management view.
7. Identity management UX — #5890: the "Devices & delegated logins" view,
   custody-change flows under §8 R1, the §6 disclosures at setup and at
   every custody change, and the §10 residual (pseudo-login accumulation)
   as an explicit validation item.

## 12. What this record does not do

It does not authorise implementation beyond the §11 filing, re-decide any
child record, or change any parent decision of #5684. The three §8 rulings
are integration decisions at the seams; if a future record needs to
reverse one, it reverses it here, at the parent, where the conflict lives
— not by quietly forking a child record.
