# SAML 2.0 for the minimal IdP — design record

- **Issue:** #13687 (SAML design for the minimal provider, story #13658)
- **Date:** 2026-11-23
- **Status:** decided; build slices filed from §7
- **Scope of this record:** design only. No SAML code lands with it.

## 1. Context

The minimal provider (`pkgs/idp-provider`) speaks OIDC today: one
account row, four roles from the parent design #1253 (`owner`,
`orchestrator`, `agent`, `read-only`), a deny-by-default allowlist,
and an append-only audit log of allowed *and* denied events. Slice 2
of the PKI (`pkgs/idp-pki`) already issued a dedicated SAML signing
key in anticipation of this work — serial 1002, certificate at
`pkgs/idp-pki/trust/saml-signing.crt.pem`, encrypted private key
escrowed in the vault as `idp.saml-signing.key`. SAML therefore
grows inside the same provider, the same account store, and the
same audit log as OIDC; it does not get a second service, a second
account model, or a separate product beside the provider (the
composed-model evaluation on #13658 settled that shape: the
minimal provider is the core).

What SAML is *for* here: the long tail of applications that speak
SAML 2.0 and not OIDC. Until a concrete SP is named, this is
capability-building, sequenced design → core → conformance →
first real SP.

## 2. Q1 — Component choice

Candidates evaluated from their own repositories and registries on
2026-11-23:

| Candidate | State | XML signature handling | IdP side | Notes |
|---|---|---|---|---|
| **samael** (njaremko/samael, crates.io) | v0.0.22, MIT, actively maintained (commits through 2026-07), ~46k crate downloads, 109 stars | libxmlsec via the default `xmlsec` feature (libxml2/libxmlsec1 C libraries) | Yes — SP and IdP roles, SP- and IdP-initiated SSO, metadata types | Oldest Rust SAML library; SP-first heritage, so its IdP surface is the less exercised half |
| **`saml`** (crates.io) | v0.1.0 **pre-release**; its own README: APIs "subject to change until v1.0" | Pure Rust reimplementation (quick-xml + RustCrypto), structural XSW resistance, interop corpus from Okta/Entra/Auth0/Google/OneLogin/Keycloak/ADFS/Shibboleth fixtures | Yes, plus a proxy role | Best architecture on paper; not yet a stable dependency. MSRV 1.91 |
| **saml-rs** (terminaloutcomes) / **risaml** fork | saml-rs is a one-author project ("doing SAML things, terribly", aimed at Kanidm); risaml is a young maintained fork (from saml-rs 0.5.0) with a typed `Saml<Idp>` facade | Delegated to bergshamra (RustCrypto or aws-lc backends) | Yes | Modern API shape, but the lineage is the youngest of the three |
| Hand-rolled XML-DSig in the provider | — | Would be ours | — | **Rejected outright** (below) |

**Hand-rolled is rejected** for the reason the OIDC side's
minimalism does not generalize: JWS verification is a b64 decode
and one RSA check, but XML-DSig involves canonicalization,
reference resolution, and signature-wrapping defenses — the exact
class of protocol machinery where a subtle bug is a silent
authentication bypass. The provider stays minimal in *surface*,
not in re-implementing XML cryptography.

**Decision: samael**, pinned to an exact version, for the first
build slices — behind an isolation boundary (all SAML protocol
calls live in one `saml` module of the provider; nothing else in
the crate names samael types). Rationale:

- It is the only candidate that is simultaneously published with
  years of history, actively maintained, both-roles, and backed
  for its signature work by **libxmlsec** — the implementation
  the wider SAML ecosystem (including the test harnesses in §6)
  validates against. For a *first interop proof*, the signature
  stack's battle-testing outweighs dependency purity.
- The costs are known and accepted: a native libxmlsec
  dependency in the provider's build closure (the Nix packaging
  supplies it), and a 0.0.x API (the pin + the isolation module
  bound the blast radius).
- **Successor trigger:** when the `saml` crate reaches 1.0, re-
  evaluate swapping the isolation module to it (pure-Rust stack,
  no C dependencies). Recorded here so the choice is revisited on
  evidence, not by drift. risaml is the fallback if samael's
  maintenance lapses first (archived repo or 12 months without a
  commit are the tripwires).

## 3. Q2 — Keys, identity, roles

- **Signing key:** the Slice 2 SAML key (serial 1002) and no
  other. Loaded exactly like the OIDC key: passphrase-encrypted
  on-host, passphrase from the vault by name into process memory
  (`IDP_PROVIDER_SAML_KEY`, `IDP_CA_PASSPHRASE`), cert published
  from `pkgs/idp-pki/trust/`. Assertions are signed
  (RSA-SHA256, SHA-256 digest); responses are signed as well.
  Metadata (§5) is signed with the same key. A SAML key rollover
  follows the Slice 5 OIDC pattern when its turn comes (slice
  S4, §7) — until then the served set is the single key.
- **Identity:** assertions are issued for the provider's one
  account row — the same row OIDC tokens name. There is no
  SAML-specific user store, and the allowlist check is the same
  one: an identity that is not allowlisted, or whose link state
  is `unlinked` (#13415), gets a denied audit record and no
  assertion.
- **NameID:** `urn:oasis:names:tc:SAML:2.0:nameid-format:persistent`
  with the **account_id** as the value — opaque, stable across
  doors and key rollovers, never an email address or a door-
  specific subject. (Consistent with the standing rule that SSO
  never rebinds an id key: the NameID names the account, not the
  door the user arrived through.)
- **Attributes:** an AttributeStatement carrying `roles`
  (multi-valued, the #1253 role strings verbatim) and
  `displayName`. Per-SP attribute release is configuration (§5);
  the default release set is exactly these two.
- **Audit:** assertion issuance and every denial are events in
  the provider's existing append-only audit log, with a `saml`
  surface tag and reason codes mirroring the OIDC vocabulary
  (`sp_not_registered`, `not_allowlisted`, `identity_unlinked`,
  `bad_request_signature`, `acs_mismatch`, `assertion_issued`).

## 4. Q3 — Profiles and bindings

- **Web Browser SSO only.** SP-initiated: AuthnRequest via
  HTTP-Redirect binding, Response via HTTP-POST binding.
  IdP-initiated (unsolicited) responses are **off** in the first
  slices — they weaken the replay story and no consumer needs
  them yet.
- **Signed assertions: mandatory** (both assertion and response
  signed, §3). **Signed AuthnRequests: accepted, not required**
  by default — many SPs cannot sign; a per-SP flag can require
  it. When an AuthnRequest *is* signed and the SP has a
  registered certificate, the signature must verify or the
  request is denied and audited.
- **Assertion encryption: not in the first slices.** Assertions
  travel over the TLS POST binding; encryption adds an SP key-
  management story no named consumer requires yet. Revisit when
  a consumer's policy demands it.
- **Single Logout: not in the first slices.** Sessions are the
  provider's existing short-lived shape; SLO is filed as later
  work if a consumer needs it, not built speculatively.
- Conditions discipline: `NotBefore`/`NotOnOrAfter` with a
  5-minute assertion lifetime, `InResponseTo` echo, Audience =
  the SP's entityID, Recipient = the registered ACS URL, and a
  server-side assertion-ID replay cache (an assertion ID is
  accepted once).

## 5. Q4 — Metadata and trust

- **Publication:** IdP metadata at `GET /saml/metadata`,
  entityID = the provider's issuer string, generated from the
  live configuration and **signed with the SAML key**. The
  metadata advertises exactly §4's profile: SSO service
  (Redirect + POST), persistent NameID, the signing certificate.
- **Trust: static, per-SP, in the provider config** — entityID,
  ACS URL(s), optional request-signing certificate, attribute
  release set, require-signed-requests flag. **No dynamic
  metadata fetching or refresh**: no SSRF surface, and a trust
  change is a config change that goes through the same review as
  code. SP metadata documents are consumed by hand at
  onboarding time and transcribed into config, never fetched by
  the provider at runtime.

## 6. Q5 — Conformance plan

No real SP depends on this IdP until both steps pass, in order:

1. **Local test SP.** An integration test that runs the
   provider's SAML side against a test SP built on samael's own
   SP implementation: SP-initiated SSO completes, the roles
   attribute arrives intact, and the negative suite passes —
   signature-wrapping attempt rejected, replayed assertion
   rejected, expired assertion rejected, wrong-ACS request
   rejected, unregistered SP denied + audited.
2. **SAMLtest.id.** The public conformance harness (the same
   one samael's and saml-rs's own docs point at): the IdP's
   metadata is uploaded, the harness's SP completes SSO against
   us, and its validation report is recorded on the conformance
   slice's issue. Findings become fixes before step 3.
3. Only then is a real SP onboarded (slice S3), one at a time,
   each as a config change with its own verification loop.

## 7. Build slices (filed as issues from this record)

- **S1 — SAML core.** `saml` module on samael (pinned); config
  for SP registrations; `GET /saml/metadata` (signed);
  `GET|POST /saml/sso` (Redirect-in, POST-out); authentication
  via the provider's existing local-door login at the SSO
  endpoint; signed assertion issuance per §§3–5; audit events.
  Acceptance: the §6 step-1 local test SP suite, green.
- **S2 — Conformance.** §6 step 2 (SAMLtest.id) executed and
  recorded; findings fixed; the negative suite extended with
  anything the harness surfaces.
- **S3 — First real SP onboarding.** Blocked until a consumer
  is named (candidates come from the per-app pattern work on
  #13416); per-SP config + a verification loop + an onboarding
  runbook entry. One SP; the second SP is a config change, not
  a project.
- **S4 — Doors + rollover parity (later).** SAML authentication
  via the federation doors (#13411/#13412/#13414 identities,
  same account row), AuthnContext reflecting the door; SAML
  signing-key rollover following the Slice 5 set pattern.

## 8. Non-goals

- No SAML proxying, no artifact binding, no ECP/PAOS, no
  attribute queries, no NameID management in these slices.
- No changes to the OIDC surface, the account model, or the
  allowlist semantics: SAML consumes them as they are.
- No new secrets: the SAML key and its passphrase path already
  exist from Slice 2.
