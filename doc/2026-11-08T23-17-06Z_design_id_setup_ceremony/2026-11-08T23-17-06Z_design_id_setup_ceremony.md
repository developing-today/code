# Native ID creation & setup ceremony — decision record

**Date:** 2026-11-08 · **Issue:** [#5884](https://github.com/developing-today/code/issues/5884)
**Parent:** [#5684](https://github.com/developing-today/code/issues/5684) (identity, login & delegation)
**Programme:** epic [#2056](https://github.com/developing-today/code/issues/2056)
**Siblings:** #5885 (keyless feasibility), #5886 (variant a), #5887 (variant b),
#5888 (variant c — recovery codes, record in open PR #7622), #5889 (variant d),
#5890 (identity management UX)
**Status:** v1 decision record (2026-11-08). Design only — no implementation is
authorised by this record alone, and implementation timing is a separate decision
per the filing.

Verified at `origin/main` `10da0c4ed2` (2026-11-08): `pkgs/id/src/store.rs`
exposes `load_or_create_keypair` (lazy Ed25519 creation, 32-byte raw secret in
`KEY_FILE`); no #5884 decision record exists on main. Sibling record for #5888
read from open PR #7622 (branch `queue/12847-5888-recovery-codes`) — treated as
proposed, not merged; where this record consumes its `RecoverySet` interface it
says so explicitly.

---

## 1. What this record settles, and what it does not

The parent (#5684) already rules the direction: identity is an iroh-style
keypair; simplicity-first is a hard requirement; custody variants (a)–(d) and
their honesty statements are the parent's; recovery via 10 security codes is
variant (c), designed by #5888. The parent's acceptance names this ticket
(decomposition item 1) to settle key generation, first signatures, and the
signed-hash setup artefact.

This record settles, for native ID creation:

1. the ceremony shape — the ordered steps from "no identity" to "identity
   presented as created", and where each step happens;
2. native key generation — algorithm, entropy source, and where initial
   custody begins;
3. the setup statement and its hash — what is signed at setup, in what order
   the first signed artefacts are produced, and how they are stored and
   verified;
4. recovery/failure invariants for a ceremony that stops halfway;
5. reconciliation with sibling #5888's `RecoverySet` (open PR #7622).

It does **not** re-decide any parent custody/recovery/delegation choice,
duplicate #5888's recovery-code design (format, KDF, threshold, regeneration
stay there), design variant (a)'s service-side generation internals (#5886),
or start any implementation.

## 2. Chosen shape, in one paragraph

Creating a native ID is an **explicit ceremony on the user's device**, never a
lazy side effect of first run. The client generates a fresh **Ed25519 keypair
from the operating system's CSPRNG**; the secret key is written exactly once,
to the local key file, and **never leaves the device** — initial custody
begins device-local, and any later custody arrangement (parent variants a–d)
is a separate, attested step. The user picks a custody variant under the
parent's plain-language disclosures, the recovery codes are generated and
transcription-proven per #5888, and only then does the new key produce its
first signatures, in a fixed order: the **setup statement** (a genesis
self-certificate whose hash binds the identity, the ceremony version, the
chosen variant, and the hash of the generation-1 `RecoverySet`), then the
**signed `RecoverySet` itself**. The identity is presented as created **only
after the signed `RecoverySet` is published** — an identity's first moment of
existence always has a recovery path, or the ceremony has not finished and
nothing is presented as created.

## 3. Key generation and initial custody

- **Algorithm:** Ed25519, matching the existing primitive in
  `pkgs/id/src/store.rs` (`load_or_create_keypair`, iroh `SecretKey`) and the
  rest of the ID graph. No new signature scheme is introduced by setup.
- **Entropy:** the OS CSPRNG via the platform keypair generator. Keys are
  never derived from a password, a login event, an OIDC token, or any
  server-supplied value. This consumes #5885's settled recommendation
  (random ID signing key, generated and held client-side) rather than
  re-opening it.
- **Where the key first exists:** in client memory during the ceremony, then
  in exactly one place at rest — the local key file (32-byte raw secret,
  mode `0600`, the `KEY_FILE` convention already in `store.rs`). It is never
  transmitted, never included in a sync payload, never logged.
- **Initial custody begins device-local.** The setup statement attests which
  parent variant the user chose; for variant (b) that is also the steady
  state. For variant (a), service custody begins only through #5886's own
  enrolment — the ceremony never silently starts in service custody, and a
  variant-(a) setup still records `custody_variant: a` in the same artefact
  shape (no forked artefact). For variant (d), pseudo-logins are delegated
  *after* genesis (§5), never instead of it.
- **An existing key file is a hard stop, not an overwrite.** If a key file
  already exists, setup refuses to generate: the operator either resumes an
  incomplete ceremony for that key (§7) or abandons it (§7). Silent rotation
  at setup is forbidden — rotation is #2804/#5888 settle mechanics, never a
  setup side effect.

## 4. The setup statement and its hash

The signed-hash construction the parent's acceptance requires:

```
SetupStatement {
  version:         1,
  identity:        <identity public key / id>,
  ceremony:        "native-setup-v1",
  custody_variant: <a | b | c | d-ready>,   // parent #5684 variants, as chosen
  recovery_set:    <SHA-256 of the canonical RecoverySet, generation 1>,
  prev:            null,                     // genesis: no predecessor
  created_at:      <timestamp, advisory only>,
}
setup_hash  = SHA-256( "id-setup-v1" || canonical_encode(SetupStatement) )
setup_sig   = Ed25519_sign( identity_secret, "id-setup-v1" || setup_hash )
```

- **Domain separation** (`"id-setup-v1"` prefix on both hash and signature)
  means a setup signature can never be replayed as any other ID-graph
  signature, and no other artefact's signature verifies as a setup.
- **Canonical encoding** is a stated implementation requirement: one byte
  string per statement, so independent implementations compute the same
  `setup_hash`. (Encoding choice — the repo's existing canonical form for
  signed ID-graph objects — is an implementation detail with this oracle:
  two conforming implementations must agree on `setup_hash` for the same
  statement.)
- **`prev: null` is the genesis marker.** Exactly one setup statement may
  exist per identity; a second one is not a re-setup, it is either a
  rotation (which links `prev` to the prior generation, per #2804/#5888) or
  invalid.
- **`created_at` is advisory** and recorded at coarse (day) granularity in
  any published form: ordering comes from the signed chain, not wall clock,
  and a precise timestamp is a fingerprinting surface (§9).

## 5. First signed artefacts, in order

The new key signs exactly these artefacts during the ceremony, in this
order, and nothing else:

1. **The setup statement** (§4) — the genesis self-certificate.
2. **The `RecoverySet`, generation 1** — format, slot commitments, and rules
   are #5888's; the ceremony owns only the ordering (§6) and the print
   screen.
3. **(Optional, post-genesis) the first delegation or binding** — e.g. a
   variant-(d) pseudo-login "do-as" grant (#5889) or a directory/hosted
   binding. Optional artefacts never block genesis and are never required
   for the identity to exist.

No other signature is produced before these: in particular the key never
signs content, challenges, or delegations "to test it works" before the
recovery set exists. A liveness check is a local sign/verify in memory,
never a published artefact.

## 6. Reconciliation with #5888 / PR #7622 (RecoverySet)

- This record **consumes the `RecoverySet` interface exactly as published**
  by the #5888 record in open PR #7622:
  `RecoverySet { identity, set_generation, slots[10] of SHA-256 commitments,
  created_at }`, signed by the identity key. No field is added, removed, or
  reinterpreted here; the setup statement references it **by hash** (§4)
  rather than embedding it, so #5888's object stays the single definition.
- #5888 §9 assigns this ticket ceremony ordering and the print screen, and
  requires: **the recovery set is published before the identity is
  presented as created.** §2 and §7 adopt that as a hard invariant.
- #5888 §13 assigns the print-screen implementation to this ticket. Decision:
  the codes are shown once, grouped exactly as #5888 specifies (five groups
  of four, slot-labelled), with a **print / save-as-plain-text** action and
  the #5888 §7.1 setup copy verbatim — no paraphrase, no in-app persistent
  copy. Transcription proof (#5888 invariant 15: type back a randomly
  chosen code) happens in the ceremony before the `RecoverySet` is signed.
- PR #7622 is **open, not merged**: if its record changes in review, the
  interface this record depends on is the `RecoverySet` shape and the
  publish-before-presented rule; anything beyond that is #5888's to change
  and this record inherits it by reference, not by copy.

## 7. Ceremony ordering and failure invariants

Ordered steps: **generate → attest variant choice under the parent's
plain-language disclosure → generate codes + transcription proof → sign
setup statement → sign and publish `RecoverySet` → present identity as
created.**

Failure and recovery invariants implementation must hold:

1. **Nothing is created until everything is.** Before the `RecoverySet` is
   published, no surface may present the identity as existing — no profile,
   no handle claim, no delegation, no "your ID is ready" state.
2. **Abort is clean.** Abandoning the ceremony before publication destroys
   the generated secret (delete the key file) and leaves no artefact; a
   later setup starts from generation, and the abandoned key's public key
   was never published, so nothing dangles.
3. **Resume is same-key or nothing.** A key file with no published setup
   statement means an incomplete ceremony: resume completes the remaining
   steps *for that key*, or abort per (2). Generating a second key over an
   incomplete ceremony is forbidden (§3).
4. **Published is never rolled back.** Once the `RecoverySet` is published,
   the identity exists; later failure (e.g. a binding step fails) is a
   post-genesis problem handled by ordinary flows, not by deleting the
   setup artefacts.
5. **Loss before recovery is stated, not fixed.** If the key file is lost
   before the `RecoverySet` is published, the identity is unrecoverable
   and was never presented as created; if lost after, #5888 recovery is the
   path. There is no support reset at any point (§5888 invariant 10,
   consumed unchanged).
6. **One genesis per identity.** A published setup statement with
   `prev: null` for an identity that already has one is invalid and must be
   rejected by verifiers.

## 8. Storage and verification

- **Secret material:** only the local key file (§3). The setup statement,
  its signature, and the `RecoverySet` are public signed objects.
- **Artefact storage:** the setup statement and `RecoverySet` are stored
  and replicated exactly like any other signed ID-graph object (the
  journal/durable store that holds the identity's chain, #2819 pattern via
  #5888 §13) — the service, when one is involved, stores these public
  objects and never any secret.
- **Verification (any verifier, offline given the artefacts):** recompute
  `setup_hash`; check `setup_sig` against the identity public key; check
  `prev == null`; recompute the `RecoverySet` hash and check it equals the
  commitment in the statement; check the `RecoverySet`'s own signature and
  `set_generation == 1`. All five checks pass, or the setup is not valid —
  there is no partial credit and no server attestation substitutes for any
  check.

## 9. Risks and mitigations

- **Linkage / fingerprinting (parent open question):** the setup artefact
  binds one identity's genesis; it introduces no identifier beyond the
  public key the identity already is. Mitigations: domain separation,
  hashes-not-contents for the recovery commitment, coarse `created_at`.
  Residual risk accepted: genesis timing is visible to anyone holding the
  chain — inherent to a signed genesis, and the alternative (no signed
  genesis) loses verifiable creation.
- **Half-ceremony confusion:** the biggest UX risk is a user believing an
  ID exists before it does. §7 invariant 1 exists for this; #5890 owns the
  screens but consumes the rule unchanged.
- **Key-file fragility:** a single local file is the identity until the
  recovery set is published — a window of minutes inside one ceremony.
  Accepted: shortening the window (publish earlier) would break the
  publish-before-presented rule's purpose.
- **Variant drift:** the attested `custody_variant` could go stale after a
  later migration (#5890). The statement records the variant *at genesis*;
  migrations are later signed objects, never edits to this statement.

## 10. Alternatives rejected, and why

| Alternative | Why rejected |
|---|---|
| Lazy creation via `load_or_create_keypair` on first run as the ceremony | No informed variant choice, no recovery at the first moment of existence, identity exists before its genesis is attested. The primitive stays as storage plumbing *inside* the ceremony's generate step. |
| Server-side key generation (all variants) | Makes every identity variant-(a) at birth regardless of the user's choice; contradicts #5885's settled client-side recommendation and the parent's custody honesty. |
| Password- or login-derived key | Entropy and custody failures settled by #5885; a derived key makes the login credential a signing key in disguise. |
| Present the identity, publish the `RecoverySet` later | #5888 §9 forbids it: the first moment of existence would have no recovery path. |
| Embed the full `RecoverySet` inside the setup statement | Two definitions of one object; the statement references it by hash so #5888's record stays canonical. |
| Support-assisted setup completion for a stuck ceremony | A completion path that does not need the key is a backdoor against §7/§5888 invariant 10. Stuck means resume-same-key or abort, for the user, on the device. |

## 11. Parent (#5684) decisions — preserved, none changed

This record changes **no** parent decision: variants (a)–(d), the
simplicity-first requirement, the honesty statements (including variant
(c)'s all-codes-lost statement, consumed verbatim through #5888 §7), and
implementation timing as a separate decision all stand as the parent left
them. The one parent open question this record answers is the setup-time
half of the signed-hash construction (parent acceptance item 3); the
fingerprinting question is addressed in §9 with its residual risk stated.
No call-out on #5684 is therefore required beyond the linking note that
this ticket is settled.

## 12. Open questions for implementation (with defaults)

| Question | Default this record sets |
|---|---|
| Canonical encoding for `SetupStatement` | The repo's existing canonical encoding for signed ID-graph objects; oracle = cross-implementation `setup_hash` agreement. |
| Where does the ceremony live (CLI, app, web)? | The client that owns the key file; web-only setup that cannot hold a key file device-locally is out of scope until variant design says otherwise. |
| Must the optional first delegation (#5889) wait for its record? | Yes — genesis ships without it; it is post-genesis by design (§5). |
| Coarse `created_at` granularity | Day. Finer only with a stated verifier need. |
| Key file already exists and belongs to a *node* (non-user) identity | Out of scope: this ceremony is for user native IDs; node bootstrap keeps the existing lazy path. |

## 13. Follow-on implementation work (separately timed)

1. An explicit setup command/flow that wraps `load_or_create_keypair` in
   the §7 ordering and refuses to generate over an existing key file.
2. `SetupStatement` type, canonical encoding, sign/verify per §4/§8, with
   property tests naming their oracles: genesis uniqueness, hash-commitment
   match against the `RecoverySet`, domain-separation non-replay.
3. Ceremony state machine enforcing §7 invariants 1–6, including
   resume/abort for an incomplete ceremony.
4. The codes print screen per §6, consuming #5888 §7.1 copy verbatim.
5. Verifier support for the §8 five-check setup validation.

## 14. Definition of done for this design ticket

- This record is linked from #5884, stating the decision and what remains
  provisional (the #5888 interface is consumed from open PR #7622; if that
  record's `RecoverySet` shape changes in review, §6 is the only section
  that needs revisiting).
- No production code is changed; implementation timing stays a separate
  decision.
