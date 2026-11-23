# idp-provider

Minimal OIDC provider for the self-hosted identity provider program —
Slice 3 of #13658, implementing #13409. Step 1 was the scaffold: OIDC
discovery plus a JWKS endpoint serving the **real Slice 2 signing
certificate** (serial 1001) published by `pkgs/idp-pki`. Step 2 adds
the rest of the plan's surface — authorize / token / userinfo — and
completes the authorization-code + PKCE loop.

No private key material is ever in the repo. The provider loads the
Slice 2 OIDC private key from its on-host path under `IDP_CA_HOME`
(`~/idp-ca/issued/oidc-signing/oidc-signing.key`), where it sits
passphrase-encrypted; the passphrase reaches the process in memory
only (the launcher fetches it from the vault by name,
`idp.ca-passphrase`, into the process environment) and the key is
decrypted in memory. At startup the provider proves the loaded key
matches the published certificate behind the JWKS, and fails closed
if it does not. There is no provider-local key generation, ever.

Standing constraints: no pkgs/id keypair custody in the login path;
SSO never rebinds an id key (#2060).

## Flow shape (the whole of it)

- Authorization code + PKCE, **S256 only**. No password grant, no
  implicit flow, no dynamic client registration — clients are a
  static registry in the provider config (see
  `config.example.json`).
- Short-lived (300 s) ID + access tokens, RS256, signed with the
  Slice 2 key. Authorization codes live 120 s and are single-use.
- ID token claims carry the account's **#1253 role set** (owner /
  orchestrator / agent / read-only) from the account row, so
  consumers check roles server-side.
- One account row (the owner) with linked external identities;
  roles live on the account, not the door. The stand-up door is the
  design's *devtoday local* identity: `GET /authorize` with valid
  client parameters serves a login form that POSTs back to
  `/authorize`; the presented credential is verified against a
  PBKDF2-HMAC-SHA256 hash held in the **on-host** provider config
  (never in the repo — generate it on-host with
  `IDP_LOCAL_CREDENTIAL=... idp-provider hash-credential`).
- **Deny-by-default allowlist** holding only the owner's identity
  keys, enforced in the provider before the credential is checked.
  **Every** authorize/token attempt — allowed, denied, or login-form
  challenge — is appended to an append-only audit record (SQLite;
  UPDATE/DELETE are blocked by triggers, and a failed audit write
  fails the attempt closed).

## Configuration (environment)

| Variable | Default | Meaning |
| --- | --- | --- |
| `IDP_PROVIDER_ISSUER` | `http://127.0.0.1:8911` | Issuer URL used in discovery and endpoint URLs. |
| `IDP_PROVIDER_BIND` | `127.0.0.1:8911` | Listen address. |
| `IDP_PROVIDER_OIDC_CERT` | `pkgs/idp-pki/trust/oidc-signing.crt.pem` (relative to the repo checkout) | Path to the OIDC signing certificate PEM the JWKS is built from. |
| `IDP_PROVIDER_CONFIG` | _(unset)_ | Path to the provider registry config JSON. Unset = public-only mode (discovery + JWKS; flow endpoints refuse service). |
| `IDP_PROVIDER_OIDC_KEY` | _(unset)_ | Path to the encrypted Slice 2 OIDC private key. Required when `IDP_PROVIDER_CONFIG` is set. |
| `IDP_CA_PASSPHRASE` | _(unset)_ | Passphrase for the signing key, in process memory only (vault name `idp.ca-passphrase`). Required when `IDP_PROVIDER_CONFIG` is set. Never logged, never written to disk by the provider. |
| `IDP_PROVIDER_AUDIT_DB` | _(registry config's `audit_db_path`, else in-memory)_ | Append-only audit SQLite path override. |

## Endpoints

- `GET /healthz` — `{"status":"ok"}`.
- `GET /.well-known/openid-configuration` — OIDC discovery document
  (code flow, PKCE S256, RS256).
- `GET /jwks` — `{"keys":[JWK]}` built at startup by parsing the
  configured certificate.
- `GET /authorize` — with valid client parameters and no credential,
  serves the local-door login form; with the local credential fields
  (the form's POST), runs the gate and issues a code by redirect.
- `POST /authorize` — the login form's submission target; same
  handler as the credentialed GET.
- `POST /token` — `grant_type=authorization_code` exchange with
  PKCE verification; returns short-lived ID + access tokens.
- `GET /userinfo` — Bearer access token in, account claims out.

## kid convention

The JWKS `kid` is the certificate's SHA-256 thumbprint in base64url
form — the `x5t#S256` convention recorded on #13686 — and is also
served as the key's `x5t#S256` field, so relying parties can pin
either form. `n`/`e` are the base64url RSA modulus/exponent taken
from the certificate's public key. Tokens are signed with that same
`kid`.
