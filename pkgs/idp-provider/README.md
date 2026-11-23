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
| `IDP_PROVIDER_IDENTITY_DB` | _(registry config's `identity_db_path`, else in-memory)_ | Identity store (#13415) SQLite path override. |
| `IDP_GITHUB_CLIENT_SECRET` / `IDP_GOOGLE_CLIENT_SECRET` / `IDP_MICROSOFT_CLIENT_SECRET` | _(unset)_ | Federated-door client secrets, in process memory only (vault names `idp.github.client-secret` / `idp.google.client-secret` / `idp.microsoft.client-secret`). A door whose secret is unset stays disabled. |

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
- `GET /door/{name}/start` — federated door entry: re-validates the authorize request, parks it, redirects upstream.
- `GET /door/{name}/callback` — upstream return: verifies the upstream identity, enforces the allowlist and link state, mints our code, redirects to the relying party.

## Federated doors (#13411 / #13412 / #13414)

Configured doors appear on the login form as "Sign in with …"
links to `GET /door/{name}/start`, which re-validates the relying
party's authorize request, parks it server-side under a random
single-use state handle (300 s), and redirects to the upstream.
`GET /door/{name}/callback` consumes the handle, exchanges the
upstream code, and verifies the upstream identity:

- **OIDC doors** (Google, Microsoft): the upstream ID token's
  signature is verified against the upstream JWKS, and `iss` /
  `aud` / `exp` / `nonce` are checked. The recorded subject is the
  configured claim (`sub`; Microsoft records `oid`, falling back
  to `sub`). The provider also sends PKCE (S256) upstream.
- **GitHub door**: the code is exchanged and the user document is
  fetched; the subject is the numeric user `id`.

The deny-by-default allowlist is then enforced in the provider,
exactly as for the local door, and a code of *our own* is minted
into the same code store — a door login's tokens are
indistinguishable from a local login's (same account `sub`, same
role claims). Door client secrets are never in the config file:
each door names an environment variable (`client_secret_env`) the
launcher fills from the vault by name (`idp.github.client-secret`
→ `IDP_GITHUB_CLIENT_SECRET`, `idp.google.client-secret` →
`IDP_GOOGLE_CLIENT_SECRET`, `idp.microsoft.client-secret` →
`IDP_MICROSOFT_CLIENT_SECRET`). A door whose secret is unset — or
an OIDC door without an issuer/JWKS configured — is disabled: its
start endpoint denies with an audit record and the rest of the
provider is unaffected.

## Account linking (#13415)

Link state lives in a runtime identity store (SQLite,
`identity_db_path`), not in the config: the config's
`account.identities` list is only the seed. The store keeps one
row per `(provider, subject)` with `active | unlinked` status and
an append-only event history (`seed` / `link` / `relink` /
`unlink`, trigger-protected like the audit table). Rows are never
hard-deleted while the account lives.

- An allowlisted identity arriving at a door for the first time is
  **linked by that login** (actor `door-login`); the event lands in
  the identity history *and* the audit record.
- An operator unlinks with `idp-provider unlink <provider>
  <subject>` (relink: `link`; inspect: `links`). An unlinked
  identity's door login is denied (`identity_unlinked`, audited) —
  a login never silently undoes an unlink — while the account
  stays reachable via the other doors.

## kid convention

The JWKS `kid` is the certificate's SHA-256 thumbprint in base64url
form — the `x5t#S256` convention recorded on #13686 — and is also
served as the key's `x5t#S256` field, so relying parties can pin
either form. `n`/`e` are the base64url RSA modulus/exponent taken
from the certificate's public key. Tokens are signed with that same
`kid`.

## Loop proof (step 3)

`examples/test-rp.rs` is a minimal relying party that completes the
full code + PKCE loop over real HTTP against a running provider —
discovery, JWKS, authorize through the local door, token exchange,
ID-token verification against the JWKS key, and userinfo — and
prints a one-line JSON verdict with the in-process loop latency
(`RP_MODE=deny` runs the non-allowlisted attempt instead). It is the
harness behind step 3's proof record on #13409.
