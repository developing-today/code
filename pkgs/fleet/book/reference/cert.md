# `fleet cert`

`fleet cert` (alias `fleet pki`) is the CLI over the library-first
`fleet-cert` PKI/X.509 API. It manages CA hierarchy material, leaf
certificates, chains, bundles and read-only inventory. The built surface is:

```text
fleet cert <group> <verb> [options]
fleet pki  <group> <verb> [options]

root       create | inspect | list | validate
issuer     create | inspect | list
leaf       issue | inspect | list
ocsp       sign | inspect | test
chain      build | test
bundle     inspect
inventory  list | show | search | audit
key        generate | inspect
publish    plan | verify
profile    list | show
algorithms list | describe
explain    [topic]
doctor
```

Use `fleet cert --help` for the usage summary, `fleet cert explain <topic>`
for a topic, and `fleet cert doctor` to distinguish implemented capabilities
from deferred ones. Unknown groups, verbs, options or malformed values are
usage errors (exit 2); operational, validation and policy failures exit 1.

## Common options and signing policy

| Option                 | Meaning                                                                        |
| ---------------------- | ------------------------------------------------------------------------------ |
| `--json`               | Emit one JSON value instead of text.                                           |
| `--not-before RFC3339` | Set validity start; defaults to now.                                           |
| `--not-after RFC3339`  | Set an exact validity end; mutually exclusive with `--valid-for`.              |
| `--valid-for DURATION` | Set lifetime from not-before (for example `20y`, `90d`, or `15 years 7 days`). |

Signing never discovers keys from ambient locations. Root and issuer creation
must name their key destination/source explicitly; issuer creation also names
its root and root key directory, and leaf issuance names an issuer key
directory. Consult `fleet cert explain signer-source` for the exact rule.

## Group guide

- **`root`** creates, inspects, lists and validates root sets, including their
  cross-signing graph.
- **`issuer`** creates, inspects and lists issuer sets under a named root.
- **`leaf`** issues, inspects and lists leaves for the built-in `server`,
  `client`, `saml-signing` and `oidc-signing` profiles.
- **`ocsp`** signs, inspects and tests RFC 6960 responses. Signing can use an
  issuer directly or a delegated `ocsp-responder` leaf.
- **`chain`** builds and tests signature-verified certificate paths under
  selected trust sets.
- **`bundle inspect`** verifies and reads a bundle manifest and its indexed
  artifacts.
- **`inventory`** provides read-only `list`, `show`, `search` and `audit`
  across certificate bundles. Audit reports drift, missing/orphaned material
  and upcoming expiries; it exits 1 when expired material or drift is found.
- **`key`** generates or inspects keys. Import/export, escrow, restore and
  rotation are not part of the currently built surface.
- **`publish plan|verify`** are read-only publication planning and
  verification. `apply` and `status` remain production-gated/deferred.
- **`profile`** and **`algorithms`** list/show profiles and list/describe
  supported signature algorithms.

Validity uses RFC 3339 instants and Gregorian calendar arithmetic. The crate
clamps issuer/leaf validity to the signer's validity window and records the
clamp as a manifest warning. Bundle manifests index artifacts with SHA-256;
tampering is rejected on load. See the [`fleet-cert` crate record](../library/crates.md#fleet-cert)
for supported algorithms, bundle layout, deferred verbs and crate-level test
evidence.
