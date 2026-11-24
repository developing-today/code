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
  `oidc-signing`; requires `--issuer-key-dir`.
- `fleet cert chain build|test` — signature-verified path building
  (shortest path, path-length pruned) and validation under
  all/classical/PQ trust selections.
- `fleet cert ocsp sign|inspect|test` — RFC 6960 responses from
  issuer bundles: direct issuer signing or a delegated responder
  certificate (leaf profile `ocsp-responder`: EKU id-kp-OCSPSigning
  + NoCheck, 30d default), verified against the crate chain code.
  This is the §4.3 functional responder core; it embeds no OCSP URL.
- `fleet cert bundle inspect`, `profile list|show`,
  `algorithms list|describe`, `explain`, `doctor`.
- Text and `--json` output; fleet exit-code contract (0/2/1).

Not yet built (tracked as child issues under #9157): `revoke`/`crl`,
`publish`, `dns`, `trust`, `key`, `format`, profile
validate/scaffold, algorithm benchmark/doctor, `batch`,
`inventory`, renew/rotate verbs, bundle pack/unpack/diff/migrate.

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

`cargo test -p fleet-cert` — 19 unit tests (registry, time
grammar, naming, graph arithmetic, profiles, bundle round-trip)
and 7 integration tests in `tests/hierarchy.rs`, each naming its
oracle: spec inventory arithmetic (§§3.2/24.2), the crate
validator, the system `openssl` as an independent implementation
(classical, PQ, and cross-signed paths), the §24.1/§24.3 policy
gates, §5.4 clamping, and §7.1 tamper rejection. A PQ-policy
negative test proves a classical leaf is rejected under PQ-only
trust while an all-ML-DSA path validates.
