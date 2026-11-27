# fleet-cert — the `fleet cert` domain (alias `pki`)

Phase 1 foundation slice of the fleet CA program
([#9155](https://github.com/developing-today/code/issues/9155),
tracking [#9157](https://github.com/developing-today/code/issues/9157)).
Implements the fleet architecture spec v0.3
(`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/sources/fleet_architecture_spec_v0_3.md`)
§§3, 5.4, 6, 7.1, 24, 26 for the core surface. Library-first per
§6.4: every CLI verb is a thin shell over this crate's API.

## Landed surface

- `fleet cert root create|inspect|list|validate` — root sets with
  the §3.2 cross-signing graph (default set: 4 keys, 13
  certificates), ceremony-named key destination (§24.3).
- `fleet cert issuer create|inspect|list` — issuer sets per the
  §24.2 matrix (default set, A+B: 8 keys, 26 certificates);
  requires `--root` and `--root-key-dir`.
- `fleet cert leaf issue|inspect|list` — profiles mirroring
  `pkgs/idp-pki`'s idp-ca: `server`, `client`, `saml-signing`,
  `oidc-signing`; requires `--issuer-key-dir`. `--ip` adds IP SANs and
  `--csr <file>` certifies a key made elsewhere (no key is generated or
  written; the manifest records `host-local` custody). The broker TLS
  profiles `broker-server`/`broker-client` (P-256, 90d, renewed with
  30d left) take a CSR only, issue only from a Base (`general`) issuer
  set, and require `.internal` DNS names plus a tailnet IP SAN, with
  wildcards and public names refused (#14399).
- `fleet cert chain build|test` — signature-verified path building
  (shortest path, path-length pruned) and validation under
  all/classical/PQ trust selections.
- `fleet cert ocsp sign|inspect|test` — RFC 6960 responses from
  issuer bundles: direct issuer signing or a delegated responder
  certificate (leaf profile `ocsp-responder`: EKU id-kp-OCSPSigning
  - NoCheck, 30d default), verified against the crate chain code.
    This is the §4.3 functional responder core; it embeds no OCSP URL.
- `fleet cert trust export|stage|ack|activate|retire|status` —
  the §3.5 trust bundle and the broker-client trust store (#14400):
  export assembles a versioned trust bundle (roots-classical /
  roots-all / roots-pq / cross-certs) from verified root sets;
  the store runs the §27.3 item 9 two-phase rollover — stage
  (verify; a failed distribution never touches the active
  anchors), acknowledge (binds version to content hash),
  activate (refused without a matching ack; the displaced
  version is kept as last-known-good), retire.
- `fleet cert key generate|inspect|import|export|rotate` — standalone key
  ceremonies (§6.1). `generate` and `import` write PKCS#8 DER to a
  new owner-only file and never replace one; `import` takes PKCS#8
  DER or a PEM `PRIVATE KEY` block and refuses a key that does not
  load under the named `--algorithm`. `export` writes the public
  half only, as SubjectPublicKeyInfo PEM (default) or DER
  (`--format der`); private-key export stays behind the #9155
  custody gates. `rotate` checks that the `--in` key loads under
  `--algorithm`, then writes a fresh key of that algorithm to a new
  owner-only `--out` file and reports both public-key fingerprints;
  the old key is kept, and retiring it is a separate step.
- `fleet cert root|issuer|leaf renew|rotate` — the
  §5.2 lifecycle verbs. `renew` reissues on the same key (only with
  `--allow-key-reuse`), keeps the generation and bumps the revision
  (`r2`); `rotate` makes new keys in the next generation (`g2`) with
  the same names, slots and policy. Each new bundle records the one
  it follows (`predecessor`). A CSR-issued leaf rotates from a new
  CSR (`--csr`). A renewed root set still anchors the issuer sets of
  its predecessor; a rotated one starts fresh, with the old set left
  in place for its issuers and leaves. `issuer rotate --root <new>`
  moves an issuer set onto a rotated root set, and with
  `--overlap-root <old> --overlap-root-key-dir <keys>` the old roots
  also certify each new issuer key, so relying parties that still
  trust only the old roots validate the new leaves during the move.
- `fleet cert root replace` and `issuer replace` — the §5.2
  orchestration in one step. `root replace` rotates the root set,
  moves every `--issuer` set onto it with the overlap, verifies every
  new chain (primaries to the new roots, overlap certificates to the
  old ones) and prints the distribution, leaf-rotation and retirement
  steps with the date the old material can retire. `issuer replace`
  does the same for issuer sets under their current roots. Output
  goes to one new directory, removed again if any step fails.
- `fleet cert bundle inspect`, `profile list|show`,
  `algorithms list|describe`, `explain`, `doctor`.
- `fleet cert inventory list|show|search|audit` — read-only
  cross-bundle inventory (§6.1): drift (checksum/size/missing/
  orphaned/unreferenced), upcoming expiries (CA 5y, leaf 90d
  Gregorian horizons, overridable per run), orphaned material;
  audit exits 1 iff expired>0 or drift>0. Nothing writes.
- `fleet cert format convert|verify` — certificate PEM/DER conversion
  plus password-protected PKCS#12 export from a verified bundle and
  archive verification. Export requires exactly one in-bundle identity
  and a `--password-file`; output is created owner-only and never
  overwrites an existing file. The API requires a matching cert/key pair,
  preserves additional certificates as the CA chain, and rejects malformed
  archives. Verification redacts keys.
- Text and `--json` output; fleet exit-code contract (0/2/1).

Not yet built (tracked as child issues under #9157): `revoke`/`crl`,
`dns`, `key` escrow/restore and private export,
profile validate/scaffold, algorithm benchmark/doctor,
`batch`, bundle pack/unpack/diff/migrate,
publish apply/status (production-gated; slice 1 ships plan +
verify read-only).

## Design decisions worth knowing

- **Engine**: rcgen 0.14 + aws-lc-rs for all five algorithms
  (ed25519, p256, rsa-4096, gated rsa-2048, ml-dsa-44). ML-DSA-44
  is real FIPS 204 with the RFC 9881 identifiers, verified
  independently by OpenSSL 3.5.
- **rcgen gap, worked around**: rcgen 0.14 cannot reload ML-DSA
  keys (its PKCS#8 loaders have no PQ branch; the typed loader
  panics). `MlDsa44Signer` implements rcgen's public
  `SigningKey`/`PublicKeyData` traits over
  `aws_lc_rs::signature::PqdsaKeyPair`, so reloaded PQ roots and
  issuers sign through the same machinery as classical keys.
- **Cross-signing is data**: the signer→subject edge tables live
  in `graph.rs` (`GraphPolicy`), shared by root creation, issuer
  creation, bundle inventory validation, and tests. The §3.2
  asymmetry (ML-DSA signs no classical subject) is a table entry,
  not a loop condition.
- **Signer source (§24.3)**: signing requests carry a required
  `SignerSource`; there is no ambient key discovery anywhere in
  the crate. Missing keys are policy errors naming the rule.
- **Path building**: iterative-deepening DFS with per-link
  signature verification and path-length pruning. A root
  cross-certificate is a CA certificate in the path, so an anchor
  with pathLenConstraint 1 cannot top a leaf → issuer → cross →
  root path; the validator enforces anchor constraints (RFC 5280
  §6.1 reading) and prefers the shorter directly-cross-signed
  issuer variant.
- **Time (§5.4)**: Gregorian calendar arithmetic on RFC 3339
  instants (Feb 29 + 1y = Feb 28); `--valid-for` and `--not-after`
  are mutually exclusive; leaf/issuer validity clamps to the
  signer's window with the clamp recorded as a manifest warning.
- **Bundles (§7.1)**: `manifest.json` (`fleet-cert-manifest/v1`)
  with per-certificate records parsed back from the issued DER
  and a SHA-256 index over every artifact; loading a bundle
  verifies the index, so tampering is a hard error. Private keys
  are PKCS#8, mode 0600, under `private/` (0700).

## Evidence

`cargo test -p fleet-cert` — 30 unit tests (registry, time
grammar, naming, graph arithmetic, profiles, bundle round-trip,
inventory horizons) and 17 integration tests
(`tests/hierarchy.rs` 7, `tests/ocsp.rs` 3, `tests/publish.rs` 2,
`tests/inventory.rs` 5), each naming its
oracle: spec inventory arithmetic (§§3.2/24.2), the crate
validator, the system `openssl` as an independent implementation
(classical, PQ, and cross-signed paths), the §24.1/§24.3 policy
gates, §5.4 clamping, and §7.1 tamper rejection. A PQ-policy
negative test proves a classical leaf is rejected under PQ-only
trust while an all-ML-DSA path validates.
