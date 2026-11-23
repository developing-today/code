# idp-provider

Minimal OIDC provider for the self-hosted identity provider program —
Slice 3 of #13658, implementing #13409. This first step is the scaffold:
OIDC discovery plus a JWKS endpoint serving the **real Slice 2 signing
certificate** (serial 1001) published by `pkgs/idp-pki`. Authorize /
token / userinfo land in the next step of the slice.

This slice uses only the **public** certificate. No private key
material is read, and none ever belongs in the repo.

## Configuration (environment)

| Variable | Default | Meaning |
| --- | --- | --- |
| `IDP_PROVIDER_ISSUER` | `http://127.0.0.1:8911` | Issuer URL used in discovery and endpoint URLs. |
| `IDP_PROVIDER_BIND` | `127.0.0.1:8911` | Listen address. |
| `IDP_PROVIDER_OIDC_CERT` | `pkgs/idp-pki/trust/oidc-signing.crt.pem` (relative to the repo checkout) | Path to the OIDC signing certificate PEM the JWKS is built from. |

## Endpoints (so far)

- `GET /healthz` — `{"status":"ok"}`.
- `GET /.well-known/openid-configuration` — OIDC discovery document
  (code flow, PKCE S256, RS256).
- `GET /jwks` — `{"keys":[JWK]}` built at startup by parsing the
  configured certificate.

## kid convention

The JWKS `kid` is the certificate's SHA-256 thumbprint in base64url
form — the `x5t#S256` convention recorded on #13686 — and is also
served as the key's `x5t#S256` field, so relying parties can pin
either form. `n`/`e` are the base64url RSA modulus/exponent taken
from the certificate's public key.
