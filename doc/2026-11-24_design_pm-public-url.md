# Design: one public URL for the PM dashboard behind Cloudflare Access (#1182)

Date: 2026-11-24. Design lane: felix. Issue: #1182, child of epic #1184.
Related: #1253 (SSO propagation — starts from the Access identity settled
here), #1254 (PWA/push — delivery path assumes behind-Access),
`doc/2026-11-24T06-06-29Z_design_pm-cloudflare-migration` (names
`pm.security.cab` the one front door; this doc is the access-model record
underneath that migration, not a re-decision of it).

Status of this record: **design of record for #1182.** It does two things
the issue's history left undone. First, it records the settled access
model as a repo document (acceptance criterion 1) — a 2026-11-24 comment
on #1182 cites a `doc/2026-11-24_design_id1182_pm-public-url.md`, but
that file never landed on main; this document supersedes the citation.
Second, it reconciles the design with what has since shipped: the
2026-11-01 triage sketch proposed a direct tunnel ingress
`pm.security.cab -> http://localhost:4870`, but the console work shipped
a better shape (one front door through the console, §2), and this design
ratifies the shipped shape instead of the sketch. **The implementation
gate in #1182 stands**: nothing in this doc authorises implementation;
the remaining implementation slices (§6) start only when Drew's explicit
go is recorded on #1182.

## 1. The settled access model (Drew, 2026-10-25, per #1182)

1. The dashboard may be public **only** behind an authentication gate —
   Cloudflare Access. No gate, no public route.
2. The identity is **Drew's Google SSO identity**. The gate admits the
   owner and denies everyone else by default.
3. The **aggregator itself does not change**: it keeps binding loopback +
   tailnet only. The public path reaches it through intermediaries; it
   never gets its own public route, hostname, or tunnel ingress.
4. The **tailnet path is the break-glass** and stays unaffected and
   documented: `amd-6.raptor-pumpkinseed.ts.net:4870` (and
   `tailscale serve` → 4870) works with no Cloudflare involvement.

These four points are settled and are not re-opened by this design.
Everything below is mechanism in service of them.

## 2. As built on main today (verified 2026-11-24)

The hostname the issue proposed already exists, as the console's front
door:

- **`pm.security.cab`** is the stack's one public name
  (`pm-stack/README.md`: console "public behind sign-in as
  `pm.security.cab` (console at `/console`, dashboard at `/` through the
  console's gate)"; aggregator still "no public route").
- **Two cooperating paths serve the one hostname:**
  - The **`pm-console-edge` Worker** sits on the hostname's route
    (`pm-stack/console/edge/wrangler.jsonc`, `PUBLIC_HOST =
pm.security.cab`). It serves the app, its state and notifications
    from Cloudflare, and checks the caller with the same identity code
    the origin uses (`pm-stack/console/src/auth.ts`). The route is
    placed on the Worker by `scripts/expose.py` once the Worker answers
    its health check — and removing the route is the whole rollback.
  - The **`amd-6-pm-stack` Cloudflare Tunnel** (remotely managed;
    connector = `pm-tunnel` user unit on amd-6) sends `pm.security.cab`
    traffic to **pm-console at `127.0.0.1:4872`**, and the console
    passes the dashboard's paths through to the **aggregator at
    `127.0.0.1:4870`** for a signed-in owner (`pm-stack/PORTS.md` row
    4872). The aggregator's listener is untouched.
- **Cloudflare Access already covers the hostname**, declared in code:
  `pm-stack/console/scripts/expose.py` (run on a timer by
  `pm-console-expose.service`) reconciles the Access application,
  policy, sign-in methods and the edge route to a declared state —
  "changes only what is not already so." Its header states the model:
  one Access application covering the hostname, **one policy: the
  owner's address**; sign-in methods are the one-time PIN, plus Google
  when the secrets exist (§3). The console's environment file names the
  Access issuer and the owner, and `auth.ts` verifies the Access-issued
  JWT (signature, issuer, audience, times, via JWKS) and asks one
  question: is this identity the owner? A request that never came
  through the edge (loopback, no edge headers) is the owner's own
  process and is treated as the owner.
- **The aggregator already knows how to consume Access identity**
  (#1253): `pm-stack/aggregator/src/access-identity.ts` derives write
  authorship from a verified `Cf-Access-Jwt-Assertion` when
  `PM_AGG_ACCESS_TEAM_DOMAIN` + `PM_AGG_ACCESS_AUD` are configured,
  failing closed on owner claims; it is inert (legacy path) when
  unconfigured.
- **Dependents already assume this URL**: the message-bridge owner link
  redirects to `https://pm.security.cab/console/#more/messages`; the
  Android app is the console's Trusted Web Activity over the same
  hostname; the Cloudflare-migration design routes every human surface
  under this one name.

So the dashboard's public path, as built, is:

```
browser / Android TWA
  │  HTTPS
Cloudflare Access ── sign-in (one-time PIN today; Google once provisioned)
  │
pm-console-edge Worker ── serves console + state itself; checks caller
  │  same hostname, paths it does not own fall through the tunnel
pm-console :4872 (amd-6) ── owner gate (auth.ts); dashboard paths pass through
  │
aggregator :4870 ── loopback + tailnet only, unchanged
```

## 3. Decisions

- **D1 — One URL, and it is `pm.security.cab`.** The dashboard's public
  URL is `https://pm.security.cab/` (dashboard at `/` through the
  console's gate, console at `/console`). No `dashboard.` or `agg.`
  hostname, no second Access application. This matches the name Drew
  and the Android app already use.
- **D2 — The console is the front door; the aggregator is never routed
  directly.** The 2026-11-01 sketch's direct tunnel ingress to :4870 is
  **rejected**: it would put a second, thinner gate in front of a
  service with no auth of its own, and split the identity check across
  two places. Routing through the console keeps exactly one origin-side
  owner check (`auth.ts`, shared verbatim with the edge) and one
  session. The tunnel ingress stays as it is: the whole hostname to
  :4872.
- **D3 — Access policy shape (as declared in `expose.py`; this design
  ratifies it).** One self-hosted Access application covering the
  hostname; default-deny; a **single Allow policy for the owner's email
  identity only** — never a domain-wide, group, or link-sharing rule,
  because behind Access the console/aggregator trust the identity
  completely, so the policy is load-bearing. Application session
  duration 24h. The policy has exactly one writer: `expose.py`'s
  declared state, reconciled by the expose timer. Hand-edits in the
  Cloudflare dashboard are drift and will be reconciled away; changes
  to the policy go through `expose.py`.
- **D4 — Identity: Google is the settled IdP; the one-time PIN is the
  interim.** Today the Access app offers the one-time PIN (email code)
  only. Google sign-in is fully wired in `expose.py` and turns on
  automatically once the secrets `google.access-sso.client-id` and
  `google.access-sso.client-secret` exist in the secret store — it is a
  provisioning gap, not a code gap (§6, slice S1). Until then the
  settled model is met in substance (single-identity gate) but not in
  the letter (Google SSO); this doc records that gap explicitly rather
  than calling the model implemented. After Google is on, the PIN stays
  as a fallback method on the same single-identity policy — it admits
  the same one address, so it does not widen the gate.
- **D5 — Origin-side verification stays mandatory.** Access is the outer
  gate, not the only check: the edge and the console both verify the
  Access JWT before answering (a request the edge is unsure of is
  passed to the origin, which decides as it always has — a stranger
  gets what a stranger got before). The `workers.dev` address of the
  edge Worker answers health and public files only; owner tokens are
  issued only for the routed hostname. Loopback remains owner-equivalent
  by definition (the caller's process already has the owner's shell).
- **D6 — Aggregator Access-JWT author derivation gets enabled on the
  public path (slice S3).** With the console passing dashboard paths
  through, configuring `PM_AGG_ACCESS_TEAM_DOMAIN` / `PM_AGG_ACCESS_AUD`
  / `PM_AGG_OWNER_IDS` makes dashboard _writes_ derive authorship from
  the verified JWT instead of a caller-supplied author — closing the
  #1253 follow-through on exactly the path this design creates. Reads
  are unaffected; unconfigured deployments keep today's behaviour.

## 4. Failure modes and break-glass

- **Tunnel connector down** → hostname requests that need the origin
  return a Cloudflare 530/no-route error; the edge still serves what it
  holds (console core, last machine report, marked with its age).
  Nothing is exposed; the tailnet path is unaffected.
- **Edge Worker down / route removed** → the hostname falls back to the
  tunnel path alone (that is also the rollback for any edge change).
- **Access misconfiguration / owner lockout** → break-glass is the
  tailnet URL (`amd-6.raptor-pumpkinseed.ts.net:4870`) or loopback on
  amd-6; neither depends on Cloudflare. Policy repair goes through
  `expose.py`, not the dashboard UI, so a bad policy cannot lock out
  its own repair path (the expose timer runs on amd-6).
- **Tunnel-token compromise alone does not expose the dashboard**: a
  caller still has to pass the Access identity check and the origin's
  JWT verification.
- **Secret-store absence is fail-visible, not fail-open**: without the
  Google secrets, `expose.py` logs that Google sign-in is not on yet
  and leaves the PIN method in place; the gate never opens wider
  because a secret is missing.

## 5. Migration steps (from as-built to design-complete)

Most of the migration already happened under the console work; the
remaining steps are small and ordered:

1. **Ratify** this design on #1182 (this doc) — no runtime change.
2. **Drew's go** recorded on #1182 (the issue's own gate).
3. **Enable Google SSO** (slice S1): create the Google OAuth client,
   store the two secrets, let the expose timer reconcile, verify a
   Google sign-in end to end.
4. **Acceptance verification** (slice S2), from off-tailnet: an
   unauthenticated request is rejected by Access; after sign-in,
   `https://pm.security.cab/` serves the dashboard; the tailnet
   break-glass still serves; evidence recorded on #1182.
5. **Aggregator JWT derivation on** (slice S3, §3 D6).
6. **Close-out** (slice S4): confirm the `pm-stack/README.md` /
   `PORTS.md` topology rows match this design (they already describe
   the shipped shape), then close #1182 and update epic #1184.

No DNS change, no aggregator change, and no new tunnel ingress are
needed at any step.

## 6. Implementation slices (listed here for Felix to file; not filed by this design)

- **S1 — Enable Google SSO on the `pm.security.cab` Access app.**
  Provision the Google OAuth client; store
  `google.access-sso.client-id` / `google.access-sso.client-secret` in
  the secret store; confirm the next expose run turns Google sign-in
  on. Acceptance: Google appears as a sign-in method on the Access
  login page; a Google sign-in as the owner succeeds. Gated on Drew's
  go on #1182.
- **S2 — #1182 acceptance verification pass.** Off-tailnet checks from
  §5 step 4, recorded as evidence on #1182. Acceptance: all four checks
  pass and are linked from the issue.
- **S3 — Aggregator Access-JWT author derivation on the console
  pass-through path.** Set `PM_AGG_ACCESS_TEAM_DOMAIN`,
  `PM_AGG_ACCESS_AUD`, `PM_AGG_OWNER_IDS` for the aggregator on amd-6;
  verify a dashboard write through `pm.security.cab` derives its author
  from the verified JWT and a forged author claim is rejected (403).
  Traces to #1253.
- **S4 — Close-out.** README/PORTS row check against §2, #1182 closure
  with the S2 evidence, epic #1184 bookkeeping.

## 7. Risks

- **Policy broadening is the standing risk.** The console and aggregator
  trust the Access identity completely, so any future change that
  admits a second identity or a domain rule silently re-scopes who can
  drive the fleet's dashboard. Mitigations: single-writer policy
  (D3), and any `expose.py` policy diff deserves owner review.
- **Single-identity availability.** If the owner's Google account is
  unreachable, the PIN method (same single address) is the fallback;
  tailnet/loopback is the final break-glass. No second human identity
  is added for availability — that would change the settled model.
- **TWA/cookie behaviour.** The Android app rides the same hostname in a
  browser context; Access session changes (duration, IdP) must be
  verified against the app in S2, not only a desktop browser.
- **Design drift.** The topology moved twice while this issue waited
  (aggregator-only sketch → console front door → edge Worker). This doc
  pins the design to named files (`expose.py`, `auth.ts`,
  `access-identity.ts`, PORTS rows) so the next drift is visible as a
  diff against §2.
