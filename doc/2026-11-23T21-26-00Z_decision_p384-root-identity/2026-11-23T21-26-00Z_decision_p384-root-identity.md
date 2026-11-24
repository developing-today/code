# Decision: adopt the IdP P-384 root as a fifth root identity, bridged by cross-signing behind the production gate

- **Date:** 2026-11-23
- **Status:** Decision record; no implementation is included here.
- **Issues:**
  [#9163](https://github.com/developing-today/code/issues/9163)
  (PKI roots, cross-signing, custody design),
  [#9155](https://github.com/developing-today/code/issues/9155)
  (fleet CA program epic, design mill),
  [#13658](https://github.com/developing-today/code/issues/13658)
  (IdP CA build story).
- **Parent design:**
  `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets`,
  governing spec
  `sources/fleet_architecture_spec_v0_3.md`
  (§3 trust architecture, §24 PKI defaults/graph/custody/publication).
- **Supersedes nothing; decided by:**
  Drew's rulings on
  [#9155](https://github.com/developing-today/code/issues/9155)
  (comments `7113177173`, `7113495110`),
  recorded in summary in the adoption record on
  [#9163](https://github.com/developing-today/code/issues/9163)
  (comment `7114322761`),
  against the CA-RECON divergence record on
  [#13658](https://github.com/developing-today/code/issues/13658)
  (comment `7112612951`).
  This document carries that recorded decision; it does not make or
  revisit it.

## Decision

1. **The IdP P-384 root is ADOPTED as a fifth root identity in the
   fleet CA program's root registry** — via the spec's import/adopt
   path (`fleet_architecture_spec_v0_3.md` §24.1 item 7:
   "Import/adopt existing CAs, CSRs, public certificates and keys").
   The adopted root:
   - Subject `CN=Developing Today IdP Root CA, O=Developing Today`;
     EC P-384 (SHA-384 signatures).
   - SHA-256 fingerprint
     `45:49:3A:F7:04:28:59:A6:E2:ED:73:7D:2C:7D:99:A7:BE:29:A3:0A:D6:BC:4F:E9:32:77:98:2B:9D:90:D1:3D`.
   - Valid 2026-11-23 → 2046-11-18 (20-year root window, matching the
     design's root default).
   - Registered with its issuing intermediate
     (`CN=Developing Today IdP Issuing CA, O=Developing Today`,
     EC P-384, SHA-256 fingerprint
     `B6:9F:25:C5:DE:AE:18:2B:A6:28:33:56:EC:99:54:DA:7C:56:BC:AC:B0:A6:AD:0F:86:6A:79:05:47:07:6A:42`,
     valid 2026-11-23 → 2036-11-20); tooling `pkgs/idp-pki`
     (`idp-ca`); public certificates in its `trust/`.
     Keys never moved: the laptop originals remain the working copies;
     escrow copies sit in the approved destinations
     (adoption record [#9163](https://github.com/developing-today/code/issues/9163)
     comment `7114322761`).
   - After adoption the program's classical root identities read:
     Ed25519, P-256, **P-384 (adopted)**, RSA-4096 —
     with ML-DSA-44 remaining the post-quantum baseline identity.
2. **The adopted root will additionally be BRIDGED by cross-signing
   into the design graph** (spec §§3.2/24.2: the asymmetric
   root-level cross-sign graph and the A/B direct cross-sign matrix)
   **when the program's design roots are generated.** That bridging
   step sits behind the production gate — design roots are not
   generated until the build-out completes, per Drew's ruling
   (comment `7113495110`) — and is **out of scope for this document**:
   no cross-signing, root generation, escrow, or production signing is
   performed or authorized here.

## Context

- The design's reference root set has four identities (Ed25519,
  P-256, RSA-4096, ML-DSA-44 as the baseline signature scheme),
  with 9 asymmetric root cross-certificates and a 13-certificate
  root-level reference inventory (CA-RECON record,
  [#13658](https://github.com/developing-today/code/issues/13658)
  comment `7112612951`).
- The IdP CA was built 2026-11-23 under
  [#13658](https://github.com/developing-today/code/issues/13658),
  is live, and per Drew's ruling (comment `7113495110`) is the
  **interim primary CA for all other work until design roots exist**:
  its issuing intermediate generates certificates for agents and
  other authentication needs.
- Its custody is **3/3 approved destinations with restoration
  tested**: encrypted laptop storage (working copies, read-only
  access), Bitwarden Secrets (`idp.root-ca.key`,
  `idp.issuing-ca.key`, `idp.ca-passphrase`), and 1Password vault
  Dev (three Document items mirroring the BWS names, hash-verified
  on read-back; restoration from the 1Password copies alone proven
  in a fresh temp dir — decrypt, pubkey-to-certificate match,
  throwaway-leaf chain verification — then shredded). Full evidence
  in the adoption record (comment `7114322761`).
- The P-384 divergence was named in the CA-RECON record
  (comment `7112612951`, divergence 1: the IdP root is EC P-384, not
  one of the design's four root identities, classical-only while the
  baseline is ML-DSA-44) and settled by Drew's ruling
  (comment `7113177173`).
- Landing history: the CERT-P2 lane completed the adoption record,
  escrow, and restoration test, then stopped the ADR document itself
  on a platform execution-risk flag with a clean prepared worktree
  (`~/wt-certp2-p384`, branch `certprog/p384-adr`, at main) and
  routed the document to a laptop-local session
  ([#9155](https://github.com/developing-today/code/issues/9155)
  comment `7114341514`). This record is that routed document.

## Rationale

- **Drew supports P-384 indefinitely and accepts either route** —
  accepting the IdP root as a fifth root identity or bridging it by
  cross-signing (comment `7113177173`); the choice is made in the
  design pass and recorded here. Cross-signing proceeds regardless.
- **The CA is live with relying parties today** as the interim
  primary CA (comment `7113495110`); adoption recognizes an
  operating, escrowed, restoration-tested hierarchy rather than
  carrying it as an undocumented exception.
- **P-384's security margin exceeds P-256's**, and P-256 was chosen
  for minor performance reasons (comment `7113177173`); a root's
  signing frequency makes that minor performance difference
  immaterial.
- **Adoption keeps one inventory and one custody story** instead of
  a permanent parallel hierarchy; bridging is retained as a
  complement (decision item 2), not the mechanism of belonging.

## Consequences

- The fleet-cert cross-signing graph (data/policy, PR #13720) gains
  a fifth identity **as data** when the reference set is next
  touched; the graph remains data/policy, not a hardcoded four
  (CA-RECON record step 2 anticipated exactly this ADR).
- The design's own reference inventory (13 root-level certificates:
  four roots + 9 asymmetric root cross-certificates) is **unchanged
  by this document** — the adopted root is counted additionally in
  the program inventory, and its future cross-certificates land with
  the bridging step, not here.
- The **5-year issuer default is unchanged** (Drew's ruling,
  comment `7113177173`); the IdP issuing CA's 10-year validity
  (→ 2036-11-20) stands as the **adopted exception, not the
  template**.
- **No key material moves and no production signing is authorized by
  this ADR.** The only signing in scope of the cited evidence was
  the restoration test's throwaway leaf, destroyed with its temp dir
  (comment `7114322761`). Bridging (cross-signing the adopted root
  into the design graph, adding the ML-DSA track beside it,
  publication, and tooling convergence per the CA-RECON steps 3–8)
  remains behind the production gate.
- [#9163](https://github.com/developing-today/code/issues/9163)
  and [#9155](https://github.com/developing-today/code/issues/9155)
  remain open; the bridging work and the program build-out continue
  under them.

## Alternatives considered

- **Bridge-only (cross-sign without adoption): rejected as the sole
  mechanism.** It leaves a permanent second-class hierarchy and two
  inventory stories — the operating interim primary CA would remain
  outside the registry it serves. Bridging is kept, but as the
  complement in decision item 2.
- **Regenerate a design-conformant successor root (abandon the IdP
  P-384 hierarchy for a fresh four-identity root): rejected.** It
  destroys a live, escrowed, restoration-tested hierarchy with active
  relying parties for a naming preference, against Drew's recorded
  support for P-384 indefinitely and his acceptance of the
  accept-as-fifth route.

## Evidence and traceability

Every factual claim above traces to the cited comments:

- Identity particulars (subjects, algorithms, fingerprints,
  validity, tooling, keys-never-moved), custody 3/3 destinations,
  hash values, restoration-test procedure and outcome, interim-
  primary role, and the P-384 decision summary:
  [#9163](https://github.com/developing-today/code/issues/9163)
  comment `7114322761`.
- P-384 supported indefinitely; either route acceptable; choice by
  design pass recorded as ADR; 5-year issuers stand with the IdP
  10-year as exception; cross-signing proceeds; program executed end
  to end starting with the `fleet cert` build-out; design + synthetic
  proof proceeds everywhere with the single stop point at the
  program's own production design roots:
  [#9155](https://github.com/developing-today/code/issues/9155)
  comment `7113177173`.
- Production gate simplified to a standing rule (design roots not
  generated until the build-out completes; dependent steps including
  IdP cross-signing out of scope until lifted); interim-primary-CA
  workstream and raised priority of custody completion:
  [#9155](https://github.com/developing-today/code/issues/9155)
  comment `7113495110`.
- Four-identity reference set, 13-certificate root inventory,
  20-pairs/65-variants issuer reference, import/adopt as the
  designed path (§24.1(7)), cross-signing machinery (§§3.2/24.2),
  and divergences/steps 1–8 including this ADR as step 2:
  [#13658](https://github.com/developing-today/code/issues/13658)
  comment `7112612951`.
- Custody/adoption completion, gate state untouched, clean prepared
  worktree and routing of this document:
  [#9155](https://github.com/developing-today/code/issues/9155)
  comment `7114341514`.
