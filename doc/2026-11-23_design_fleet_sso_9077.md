# Fleet SSO design note (#9077)

Date: 2026-11-23 · Lane oc-code-05 (task 21351) · Status: settled per FDR-0013
(Drew may override §6; the override point is named there, not parked on.)

## 1. Question settled

**Is pkgs/id the OIDC provider, or a separate minimal provider?**

**Settled: a separate minimal OIDC provider first, with an explicit,
designed fold path into pkgs/id — not drift.** Fleet services get boring,
standard OIDC now; id remains the long-term identity system of record and
absorbs the provider's accounts through its own binding ceremonies when it
is ready to be an issuer.

## 2. Evidence (all verified fresh 2026-11-23)

- **No OIDC surface exists in pkgs/id.** Code search over `pkgs/id` for
  `oidc|openid|jwks` returns nothing; `issuer` hits are currency issuers,
  evidence issuers, and rotation — no protocol surface to reuse. Casting
  id as the provider is greenfield protocol work, not a small extension.
- **id's key-custody model is unresolved.** #5684 (simplicity-first custody,
  login & delegation) is OPEN and owned; #9077's own body names this as the
  cost of id-as-provider ("more exotic than a conventional OIDC issuer").
  Fleet logins must not wait on it.
- **The binding side is half-built.** #2852 slices 1–2 (account + binding
  records, ceremony, delayed rebind) landed (PRs #4854, #4885); slice 3 is
  partial (metering/budget remainder in #3811) and slice 4 (export/import,
  #3810) is untouched. The ceremony the fold path needs *exists*; the rest
  does not gate a separate provider.
- **The consumer contract is in progress, not landed.** #1253 (one identity
  across the PM stack, roles owner/orchestrator/agent/read-only checked
  server-side) is OPEN/in-progress; #1182 (Cloudflare Access entry) is
  OPEN/triaged. The provider must therefore serve the #1253 role model as
  its first consumer contract without assuming the entry migration is done.
- **Serverless id is unbuilt.** #5524 (Workers edition) is OPEN. A minimal
  provider has no such dependency.

## 3. The settled shape

- **Issuer:** a new minimal OIDC provider ("devtoday SSO" button) —
  standard code + PKCE flows, upstream federated logins GitHub / Google /
  Microsoft, signing keys with rotation, JWKS endpoint, short-lived ID +
  access tokens. No id keypair custody inside the login path.
- **Linking model:** one account row, N linked external identities
  (GitHub sub / Google sub / Microsoft oid / devtoday local). Any linked
  door opens the same account interchangeably; linking and unlinking are
  audited account events. Every external identity ever linked is retained
  in history (never hard-deleted while the account lives) so audit keeps
  continuity of who-did-what.
- **Role set (v1, consumed server-side):** owner / orchestrator / agent /
  read-only, per #1253. Roles live on the account (not the login door), are
  checked server-side on every write, and propagate as token claims the
  entry point can forward. Agent identity (agents as themselves, not Drew)
  follows #1253 §agent-identity; the provider mints agent credentials as
  linked identities of the same account shape.
- **Drew-only gate (v1):** deny-by-default allowlist containing only Drew's
  GitHub/Google/Microsoft account IDs, enforced in the provider (not the
  apps), every attempt audited (allowed and denied). Lifting the gate is a
  separate ruled change, not a config edit.
- **Fold path into id (designed, not drifted):** when #5684 settles custody
  and id can act as an issuer, provider accounts bind to id accounts via the
  #2852 ceremony (external-identity-as-bound-identity), the provider becomes
  a relying party of id (or id absorbs issuance), and the linking history
  migrates as the audit trail. The child issue for the provider MUST record
  this fold as acceptance debt, not an afterthought.

## 4. Non-goals (unchanged from #9077)

MFA/passkeys/OTP policy, workload identity, capabilities/attenuation beyond
basic roles, and the broader Fleet v0.3 identity vision (CA mill #9155
boundary note) — recorded as out of scope, not covered here.

## 5. Open questions still needing Drew (override points)

1. Provider choice itself (§1) — override by ruling for id-as-provider.
2. Whether hosting-adjacent costs ever enter any ledger — no.
   (Not this issue's question; listed so the roles/claims shape is not
   later read as answering it.)
3. When the Drew-only gate lifts, and to whom first.

## 6. Build breakdown (filed as child issues of #9077)

1. Provider stand-up (issuer, JWKS, rotation, allowlist gate, audit).
2. GitHub federated login. 3. Google federated login.
4. Microsoft federated login. 5. Account linking UI/flow + history.
6. First consumer integration (PM stack per #1253) proving the full loop:
   Drew signs into a fleet service via each linked door against the same
   account, roles enforced server-side.
