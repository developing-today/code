# idp-pki — certificate authority for the self-hosted IdP

`idp-ca` is a small openssl-backed certificate authority: a self-signed root
CA, one issuing intermediate, leaf issuance with named profiles, revocation,
and CRL generation. It is the PKI foundation (Slice 1) of the story "our own
OIDC/SAML/SSO identity provider" — the hierarchy built here is the trust
anchor from which the future IdP's OIDC (JWKS) and SAML signing certificates
are issued.

## Relationship to the fleet CA program

The fleet CA / credentials / secrets program (epic #9155, design records in
`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/`, PKI starters
#9163 and #9164) designs the fleet-wide PKI with ML-DSA-44 as its baseline
signature scheme. This package is deliberately separate and classical:

- The IdP's certificates must verify in ordinary relying-party stacks
  (browsers, SAML and OIDC libraries), which do not yet accept ML-DSA chains.
  This CA therefore uses ECDSA P-384 (CAs) with RSA/ECDSA leaves.
- The fleet program remains the place for the post-quantum design; a hybrid
  or cross-signed root (its #9163 already covers cross-signing) is the
  recorded path to converge the two later, not a silent fork.

## Commands

```
idp-ca init-root [--cn NAME] [--days N]
idp-ca create-intermediate [--cn NAME] [--days N]
idp-ca issue-leaf --profile server|client|saml-signing|oidc-signing \
    --name NAME [--cn NAME] [--san DNS:x]... [--days N]
idp-ca revoke --name NAME
idp-ca gencrl [--ca root|issuing]
idp-ca inspect [--cert FILE]
idp-ca verify-chain [--leaf FILE] [--with-crl]
idp-ca status
```

State lives under `IDP_CA_HOME` (default `~/idp-ca`): `root/`, `issuing/`
(each with `ca.crt`, encrypted `ca.key`, `index.txt`, `serial`, `crl.pem`),
and `issued/<name>/` for leaves.

## Chosen defaults

| Thing                              | Choice                                           | Why                                                                                       |
| ---------------------------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| Root CA                            | EC P-384, SHA-384, 7300 days (20 y), pathlen 1   | Long-lived trust anchor; P-384 is the modern, widely accepted CA curve                    |
| Issuing CA                         | EC P-384, SHA-384, 3650 days (10 y), pathlen 0   | Shorter than the root so the working CA can be rotated without touching the anchor        |
| server / client leaves             | EC P-256, 730 days                               | Fast, modern leaf keys; two-year private-CA lifetime                                      |
| saml-signing / oidc-signing leaves | RSA 2048, 1095 days                              | RS256/RSASSA is the most widely accepted signing profile in SAML and OIDC consumer stacks |
| Key encryption                     | AES-256-CBC, passphrase from `IDP_CA_PASSPHRASE` | Passphrase is never taken from argv and never printed                                     |

## Key handling

- Escrowing a PEM value with the `bws` CLI needs the `--`
  separator (`bws secret create -- <KEY> <VALUE> <PROJECT>`):
  a PEM value starts with `-` characters, and without `--` the
  argument parser eats it as an option and echoes it back in
  the error. Pass the value argv-direct (no shell in between).
- Private keys are generated on the CA host only. They are **never**
  committed to git, never printed, and never leave the host except into the
  secrets vault.
- The live hierarchy (generated 2026-11-23 on the laptop `amd`) escrows its
  keys in Bitwarden Secrets under the names `idp.root-ca.key` and
  `idp.issuing-ca.key`; the key-encryption passphrase is vault-held as
  `idp.ca-passphrase`.
- Only public material is published: `trust/` in this directory holds the
  root certificate, the issuing certificate, and the chain, exactly as
  generated, plus the provider signing certificates issued under Slice 2
  of the IdP story (`oidc-signing.crt.pem`, `saml-signing.crt.pem`) —
  plus the Slice 5 rollover successor `oidc-signing-2.crt.pem`
  (serial 1004, SHA-256 fingerprint
  `59:EF:0E:3D:7E:61:91:D4:97:D2:AB:58:54:41:A5:71:B8:41:25:BB:27:E7:3A:A4:C5:1C:CD:84:34:C9:59:7A`;
  its key is escrowed as `idp.oidc-signing-2.key`) —
  public certificates only; their private keys live in the vault, by name. Verify any copy against the fingerprints recorded on the story
  issue before trusting it.

## Verification recipe

```
idp-ca verify-chain                                  # intermediate against root
idp-ca verify-chain --leaf issued/<name>/<name>.crt  # leaf against the chain
idp-ca verify-chain --leaf issued/<name>/<name>.crt --with-crl
```
