# SSO relying-party integration (component H): hosted editions consume #9077, tenancy accounts align with #1253 roles

Date: 2026-11-24 · Lane code-g (queue task 21564, AUDIT-7 Lane 6 packet) · Status: proposed
Source issue: developing-today/code#9086 (component H of epic #9078) · Tracking: #13873 · Umbrella: #13871

Scope fence (binding, from #9086): identity is designed in #9077 — this record
is ONLY the relying-party integration of the hosted editions with it. Any gap
found in #9077's scope is filed ON #9077, never forked into a parallel identity
design here. The tenancy account model aligns to #1253's roles; the hosted ID
edition consumes #2852's binding rather than inventing another.

Consumed settlements (inputs, not decisions):

- #9077 provider shape: a separate minimal OIDC provider first ("devtoday SSO"
  button), upstream federated logins GitHub / Google / Microsoft, one account
  row with N linked external identities, roles owner / orchestrator / agent /
  read-only on the account checked server-side, Drew-only allowlist gate in
  the provider, audited attempts, designed fold path into pkgs/id via the
  #2852 ceremony (`doc/2026-11-23_design_fleet_sso_9077.md`, settled 2026-11-23).
- Component E tenancy schema (#9083 design, `doc/2026-11-17T16-55-00Z_design_hosted-tenancy-architecture/`):
  central tenancy DB holds `account`, `binding`, `tenant_db`, `role_grant`,
  `peer_policy`, `publish_record`, `credential` (metadata only, tokens NEVER
  stored); per-tenant Turso DBs hold content; routing is a pure function of
  (session → `binding_id`, board → tenant DB), fail closed.
- #2852 slices 1–2 landed (account + binding records, ceremony, delayed
  rebind; PRs #4854, #4885); slice 3 partial, slice 4 untouched.

## 1. Problem + context — what breaks today, with evidence

The hosted editions (A: PM service Workers edition #9079, B: ID service hosted
edition #9080, C: fleet tool hosted equivalent #9081) have no authentication
story. The program intake (`doc/2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md`,
S15) states the requirement in one line — "hosted services are relying parties
for #9077" — and every downstream design treats it as an unconsumed seam: the
component A design (2026-11-23) names an "SSO session → `binding_id`
resolution (H/#9077 seam)" front-door module it does not specify, and E's
tenancy design assumes "an authenticated session" at §5.3/§8 without saying
how any surface produces one.

What exists today is the opposite of SSO:

- Laptop PM stack authorship is caller-asserted (tracked in #941, restated in
  #1253): 'who did this' is unenforceable and per-person permissions do not
  exist. There is nothing to federate yet — the relying party must also be
  the first server-side enforcement point for #1253's roles.
- The only doorway designed is the interim one: #1182 (dashboard behind
  Cloudflare Access, identity = Drew's Google SSO, Google-only) is OPEN,
  explicitly gated on Drew's go, and not implemented. It covers one app, one
  upstream, one user — and its relation to #9077 was never stated.
- #9077's provider side settled 2026-11-23 but has no consumer: no relying
  party exists, so the provider's role claims, linking model, and gate have
  no integration proving they are consumable.

Without this design, A/B/C cannot serve any authenticated traffic, E's token
minting has no session to mint from, and #9077's provider is an issuer with no
relying party — the program's auth chain is specified at both ends with the
middle missing.

## 2. Goals and explicit non-goals

Goals (the four questions #9086 demands answers to, plus the #1182 relation):

- G1. Per-surface token/session flows: which flow applies to the browser
  dashboard (A), service-to-service callers (C), and native instance sync (E),
  and where each flow terminates.
- G2. Tenancy link table: where account ↔ identity links live, and exact
  link/unlink semantics including per-user Turso DB access continuity.
- G3. Drew-only gate mechanism and the rule by which it relaxes without a
  redesign.
- G4. Agent credential issuance and scoping against tenancy for #1253's agent
  role, with audit attribution.
- G5. The #1182 interim door's relation to this integration once #9077 exists
  (retire, keep, or keep-as-edge — decided below).

Non-goals (out of scope, not deferred design debt):

- Identity provider design itself (#9077 owns issuer, JWKS, rotation,
  upstream federation, linking ceremony UI). This record consumes its
  contract; a provider gap is filed ON #9077.
- MFA/passkeys/OTP policy, workload identity beyond agent credentials,
  capabilities/attenuation beyond #1253's four roles (all inherited
  non-goals from #9077's settlement §4).
- Per-surface authorization policy content (what orchestrator may do to which
  board lives in #1253 and the surface designs; this record defines where the
  check happens and what claims feed it, not the permission matrix).
- Break-glass when the provider is down beyond "fail closed + tailnet/native
  paths unaffected" (each surface's design owns its degraded mode; §8 notes
  the one open parameter).

## 3. Alternatives considered, with honest tradeoffs, including do-nothing

- **Alt 1 — each surface rolls its own auth.** A verifies tokens its own way,
  C mints its own service keys, E does its own thing. Rejected: three
  session→binding resolutions means three places to get the fail-closed rule
  wrong, three audit trails, and role checks that cannot share #1253's
  `role_grant` table. The front-door seam exists precisely to avoid this.
- **Alt 2 — Cloudflare Access everywhere, permanently (#1182 as the final
  answer).** Keep Access (Google-only, Drew-only) as the hosted auth story.
  Rejected: covers one upstream and one user by construction, has no
  account-linking model, no roles, no service-to-service story, and no answer
  for native sync. Kept a role only as a transitional edge (see §4.6).
- **Alt 3 — wait for pkgs/id as the provider, integrate once.** Rejected as a
  schedule matter, not a technical one: #9077 settled this 2026-11-23 with
  evidence (no OIDC surface in pkgs/id; #5684 custody unresolved; serverless
  id unbuilt). Waiting re-gates every hosted edition on the ID program.
  The fold path (§4.7) keeps this alternative available later without
  redesign of the relying-party side.
- **Alt 4 — do nothing (ship hosted editions unauthenticated / tailnet-only).**
  Rejected: the program's premise is multi-tenant hosted service; without a
  relying-party design the editions cannot leave tailnet, E's per-tenant
  isolation has no authenticated principal to key off, and #1253's
  server-side role checks have no identity to check. Doing nothing preserves
  caller-asserted authorship — the exact condition #1253 exists to end.

## 4. Chosen approach

### 4.1 Trust architecture in one paragraph

The #9077 provider is the sole identity authority. Each hosted surface runs
a **front door**: a stateless handler that validates the provider's tokens
(JWKS), resolves the provider subject to a tenancy `account_id` via the link
table (§4.3), loads the account's active `binding_id` and `role_grant` rows,
and translates the session into the surface's native credential — an httpOnly
session cookie (A), an audience-bound short-lived service token (C), or
nothing bearer at all (E, which uses key-based peer auth). Tenancy authority
(keyed by `binding_id`, E §2–§3) is never re-derived downstream: routing and
role checks consume what the front door resolved, and every denial fails
closed. Token values are never stored in any DB (E §2.3 inherits fully).

### 4.2 Per-surface flows (G1)

| Surface                              | Flow                                                                                                                                                      | Terminates where                                                                                                        | Notes                                                                                                                                                                                             |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A browser dashboard (PM service)     | Authorization Code + PKCE against the devtoday provider (federated GitHub/Google/Microsoft upstream at the provider, invisible here)                      | Front-door session cookie: httpOnly, Secure, SameSite=Lax, 12h idle expiry, bound to `binding_id` + role-claim snapshot | Cookie is a pointer, not a capability: every request re-resolves `binding_id` → live `role_grant` rows, so revocation lands within one request, not at cookie expiry                              |
| C service-to-service (fleet surface) | OAuth client-credentials against the provider using an agent credential (§4.5); returns audience-bound access token, 15-minute TTL                        | Bearer on the API call; validated at the front door per request                                                         | No refresh tokens for services; callers re-authenticate with the agent credential. Token carries `binding_id`, `tenant scope`, `role=agent`, `jti`                                                |
| E native instance sync               | NO bearer flow. Sync uses the existing signed-event `LogSync`/range-sync path with key-based peer auth (`iroh_id` from the `binding` row, #2852 ceremony) | Peer handshake, not HTTP auth                                                                                           | Bearer tokens never cross the sync boundary. A native node proves "I hold the key for this binding" exactly as native peers do today; the hosted tenant is an ordinary peer (E §0 peer principle) |

Why PKCE for a confidential dashboard: the dashboard is server-rendered and
holds a client secret, but PKCE costs nothing and keeps the flow identical if
a public client (native app, CLI login) is added later — one flow to audit,
not two. Device flow is NOT adopted: native instance sync needs no browser
login (key ceremony covers it) and fleet agents use client-credentials; adding
device flow would be a second human-login path with no consumer.

### 4.3 Link table: home and semantics (G2)

**Home:** a new `identity_link` table in the CENTRAL tenancy DB, beside E's
`account`/`binding` tables — it is control-plane data (who may open which
account through which door), never content:

```sql
-- One row per linked login door. The provider's subject is the key;
-- email is contact-only, never a key (E §2.1 rule inherits).
CREATE TABLE identity_link (
  link_id       TEXT PRIMARY KEY,
  account_id    TEXT NOT NULL REFERENCES account(account_id),
  door          TEXT NOT NULL,             -- github | google | microsoft | devtoday
  provider_sub  TEXT NOT NULL,             -- stable subject at that door
  linked_at     TEXT NOT NULL,
  linked_by     TEXT NOT NULL,             -- binding_id that performed the link
  unlinked_at   TEXT,                      -- NULL while active
  UNIQUE (door, provider_sub)              -- one account per door-identity, ever
);
```

Every link AND unlink is an audited account event (who, when, which door).
History rows are retained (never hard-deleted while the account lives) so the
audit trail keeps continuity of who-did-what — same rule as #9077's
settlement §3.

**Link semantics:** linking a new door requires an authenticated session on
the account (prove who you are) PLUS a completed upstream login on the new
door (prove you own it), then inserts the row. First-link (account creation)
is provider-gated (§4.4).

**Unlink semantics:**

- Last-door guard: the single active link for an account cannot be unlinked
  without a replacement linked in the same operation. An account with no
  doors is unrecoverable except by Drew-level intervention — the guard makes
  that state unreachable by API.
- DB access continuity: tenant access keys off `account_id`/`binding_id`
  (E's `tenant_db.tenant_id = binding_id`), never off a door. Linking or
  unlinking changes which doors open an account; it never touches tenant rows,
  never re-keys a database, and never invalidates another session's Turso
  tokens (those die by their own TTL or by `credential.jti` revocation).
  Unlinking a compromised door is therefore safe and instant: the door stops
  opening the account, everything the account owns is untouched.
- Rebind interplay: `binding` row changes (key rotation, #2852 delayed
  rebind) are orthogonal — `identity_link` points at `account_id`, stable
  across rebinds.

### 4.4 Drew-only gate and relaxation (G3)

**Mechanism (v1):** deny-by-default allowlist containing only Drew's
provider subjects, enforced IN THE PROVIDER (first-link/account-creation
path), not in the apps. Every attempt — allowed and denied — is audited.
Apps never see the gate; they see only valid sessions for existing accounts,
so no surface carries gate logic that could drift.

**Relaxation without redesign:** lifting the gate is a ruled change to the
provider's enrollment policy (add allowlist entries, or flip to open
enrollment with default role `read-only`), recorded as a decision with the
decider named (§8). No protocol, schema, or surface change is involved:
`role_grant` already scopes every account, and a newly enrolled account with
only `read-only` on no tenant can do nothing until granted. The gate relaxes
by data + ruling, never by redeploy of the auth path.

### 4.5 Agent credentials (G4)

Fleet/agent callers authenticate as #1253's agent role, as linked identities
of the same account shape (per #9077 settlement §3 — no parallel credential
system):

- Issuance: an account owner (or orchestrator, per #1253's permission matrix)
  mints an agent credential for their account from an authenticated session;
  the credential is recorded in E's `credential` table as metadata
  (`tenant_id`, audience, scope, `jti`, expiry) — the secret itself is shown
  once and never stored.
- Scope: `role=agent`, `scope=tenant_id` (single tenant, never `'*'` at
  issuance; cross-tenant agents hold one credential per tenant). TTL 30 days
  max, rotation by re-issuance; revocation by `jti` with effect within one
  request (front door checks live `credential` rows, same as §4.2).
- Attribution: every action taken under an agent credential is audited with
  both the agent credential's `jti` and the owning `account_id`/`binding_id`,
  satisfying #1253's agent-attribution requirement. Agents authenticate as
  themselves (their credential), never as Drew, never with a shared secret.

### 4.6 #1182 relation (G5)

#1182's Cloudflare Access door (Google-only, Drew-only, unimplemented, gated
on Drew's go) is the INTERIM entry for the laptop dashboard, not for the
hosted editions. Ruling: once slice 6 (§5) proves the full #9077 loop on a
hosted surface, Access retires as the authority for that surface — the
provider is the authority, Access is at most a defense-in-depth edge during
transition. The tailnet break-glass path #1182 documents is unaffected either
way and stays. No hosted surface may treat an Access JWT as sufficient
identity after its slice-6 proof; before that proof, hosted surfaces serve no
production traffic at all (tailnet/dev only), so there is no gap to cover.

### 4.7 #2852 consumption and #9077 gap check

- The hosted ID edition (B) consumes #2852's binding ceremony verbatim for
  SSO→ID binding; `identity_link` rows reference `account_id`s that the
  ceremony binds. No second binding ceremony is specified here by design.
- #9077 gap check (performed 2026-11-24, required by #9086 acceptance): the
  provider settlement covers issuer, JWKS, rotation, linking model, role set,
  gate, and audit — everything this integration consumes. The one item this
  record needs that #9077 leaves open is the exact token-claim shape for
  roles (`binding_id` + role list as claims); filed ON #9077 as a comment
  (claim-shape request), not forked here. No other gap found.

### 4.8 Security and capability analysis

- Blast radius: provider compromise is total (it is the authority) — hence
  minimal provider, standard flows, short TTLs, no custom crypto. Tenancy-DB
  compromise yields metadata + link history, never content or token values
  (E §2.3 holds: nothing new stored here is a bearer secret).
- Confused deputy: service tokens are audience-bound per surface; a token
  minted for C is rejected at A. Agent credentials are single-tenant.
- Downgrade: surfaces accept ONLY provider-issued tokens after slice 6; the
  Access-JWT path is removed, not left as a fallback (a fallback login is a
  second authority).
- Audit: link/unlink, gate denials, issuance/revocation, and agent actions
  are all account events with actor + timestamp. There is no unaudited
  identity-state change in this design.

## 5. Dependency-ordered build slices (each one lane/session)

1. **Slice H1 — claim-shape contract + front-door library.** Agree the exact
   role/binding claims with #9077 (answer to the filed claim-shape request),
   implement ONE shared front-door module (validate → resolve → role-load →
   fail-closed) consumed by all surfaces. Acceptance: unit tests with forged/
   expired/foreign-audience tokens all rejected; a fixture provider token
   resolves to (`account_id`, `binding_id`, roles) in <50ms p99 outline.
2. **Slice H2 — `identity_link` table + link/unlink API.** Migration, link
   ceremony (session + new-door proof), last-door guard, audit events.
   Acceptance: link/unlink round-trip tests incl. guard refusal, history
   retention, and DB-continuity proof (tenant rows untouched across relink).
   Depends on H1 (needs resolution to authenticate linking).
3. **Slice H3 — surface A integration (browser Code+PKCE + session cookie).**
   Dashboard login, cookie issuance, per-request role re-resolution, logout
   (cookie kill + session audit). Acceptance: Drew logs in via each linked
   door to the same account; revocation lands within one request; unauthenticated
   requests rejected. Depends on H1, H2.
4. **Slice H4 — agent credentials + surface C integration.** Issuance UI/API,
   `credential`-metadata recording, client-credentials flow, audience-bound
   tokens, revocation, audit attribution. Acceptance: agent acts as agent role
   on exactly one tenant, revocation effective in one request, audit shows
   `jti` + owner. Depends on H1 (H2 for the owner session that mints).
5. **Slice H5 — native sync peer-auth confirmation.** Conformance note (not new
   code expected): hosted tenant as ordinary LogSync peer via #2852-bound
   keys; bearer tokens absent from the sync path by test. Acceptance: sync
   session test proving key-auth works and bearer tokens are neither sent nor
   accepted on sync endpoints. Depends on H2 (binding rows must exist).
6. **Slice H6 — first-consumer full loop + #1182 retirement.** Drew signs
   into a hosted service via EACH linked door against the same account with
   roles enforced server-side (mirrors #9077's acceptance, proven from the
   relying-party side); #1182 retired as authority per §4.6. Acceptance: live
   proof log + #1182 updated. Depends on H3, H4.
7. **Slice H7 — gate-relaxation drill.** Dry-run the §4.4 relaxation (add a
   test enrollment, verify default read-only-nothing, remove it) without
   changing production policy. Acceptance: drill log + runbook merged.
   Depends on H6. (Policy change itself is Drew's ruling, §8.)

Slices H3/H4/H5 are parallel-safe with each other (all depend only on H1,
H2 for minting sessions); H1 → H2 → {H3, H4, H5} → H6 → H7.

## 6. Rollout + rollback plan

Rollout: H1+H2 land behind no traffic (tables + library, no surface wired);
H3 wires the dashboard on a dev hostname first, then production; H4 then H5;
H6 is the go-live proof per surface; H7 drills relaxation. Each surface
migrates independently — a surface with no slice-6 proof serves no production
traffic (tailnet/dev only), so partial rollout is safe by construction.

Rollback: each slice rolls back by unwiring (surface falls back to
no-production-traffic posture), never by resurrecting a weaker authority.
`identity_link` rows are additive — rollback of H2+ means dropping the table,
which strands no tenant data (access keys off `account_id`, §4.3). There is
no rollback that re-enables caller-asserted authorship: if the provider is
down, surfaces fail closed (deny) and native/tailnet paths continue; the
break-glass posture is "deny hosted writes, keep native working," owned per
surface.

## 7. Verification plan

- Slice tests: forged/expired/foreign-audience token rejection (H1);
  last-door-guard refusal + history retention + tenant-continuity (H2);
  per-door login to one account + one-request revocation (H3); single-tenant
  agent confinement + `jti` attribution in audit (H4); bearer-absent sync
  proof (H5); live multi-door loop log (H6); drill log + runbook (H7).
- Program-level: #9077's acceptance (Drew signs in via each door, roles
  enforced server-side) re-proven from the relying-party side in H6 — the two
  acceptances are the same loop observed from opposite ends, and both must
  hold.
- Negative verification: provider-outage drill (surfaces deny, native works);
  compromised-door drill (unlink door, prove account + data untouched).

## 8. Open questions, each with a named decider

1. Exact role-claim shape in provider tokens (names, nesting, multi-tenant
   representation) — decider: Drew, on #9077 (comment filed from this
   design); H1 proceeds with a proposed shape if unanswered in 7 days,
   recorded as provisional.
2. Session cookie TTL / idle policy values (12h proposed, §4.2) — decider:
   Drew; H3 proceeds with proposed values.
3. Whether Access remains as a defense-in-depth edge post-H6 or is removed
   entirely — decider: Drew, at H6 review.
4. Agent credential max TTL (30 days proposed) and whether orchestrator may
   mint (proposed yes) — decider: Drew, at H4 review; #1253's permission
   matrix is consulted, not overridden.
5. Provider-outage degraded mode beyond fail-closed (read-only dashboard?
   queued writes?) — decider: Drew, per surface at slice review; default is
   deny.

## 9. Record

- This document (dated, in `doc/`) is the design record for #9086.
- Tracking: source issue #9086 (link comment on completion); AUDIT-7 Lane 6
  tracking #13873; umbrella #13871.
- Queue: task 21564 (`repo-health-code-pilot`, kind `feature_slice`,
  slice 1) — completed on merge of this doc.
- Implementation issues: slices H1–H7 to be filed as children of #9086 on
  acceptance of this design (granular-issue acceptance in #9086).
