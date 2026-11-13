# Design: one identity across the pm-stack — SSO propagation and roles (pm-stack)

- **Issue:** #1253 (epic #1184, PM stack coherence design series)
- **Depends on, taken as parameters (not re-decided):**
  - #1182 (one public URL behind Cloudflare Access, open): the public
    entry exists and terminates Cloudflare Access. This record assumes
    the entry presents a validated Access identity to the stack; if
    #1182 lands a different entry shape, §2's first hop changes, the
    rest does not.
  - #941 (aggregator comment API accepts self-asserted author identity,
    open): the defect this design removes. The migration in §5 is
    written against the comment/registry records #941 describes.
- **Status:** design record. No implementation in this document's PR.
  Open questions that are Drew's are named in §7 and are *not* decided
  here.

## 1. Problem

Authorship on dashboard writes is caller-asserted (#941): a write names
an author and the stack believes it. Each layer — the aggregator, Kaneo,
Pad, Grist — additionally keeps its own account story, so "who did this"
is unenforceable, per-person permission does not exist, and fleet agents
act under Drew's identity or under no identity at all. The ideal system
(#1253): one identity — the Cloudflare Access identity at the public
entry — propagates through every layer, roles are checked server-side
on every write, and asserted authorship disappears.

## 2. Identity propagation per layer

The rule that makes the rest mechanical: **identity enters the stack
exactly once, at the entry, as a verified claim; every hop inside the
stack carries that claim in a form the next layer can verify without
trusting the caller; no layer ever accepts an identity from a request
body.**

| Hop | Mechanism | Verification |
|---|---|---|
| Browser → entry (#1182) | Cloudflare Access login (IdP-backed) | Access validates the IdP session and issues its JWT (`Cf-Access-Jwt-Assertion`) |
| Entry → aggregator | The Access JWT is forwarded unchanged on proxied requests; direct (non-entry) routes to the aggregator do not exist publicly | Aggregator validates the JWT against the Access team's public keys (issuer, audience, expiry) on every request — no session cache longer than the JWT's own lifetime |
| Aggregator → Kaneo / Pad / Grist (writes the aggregator brokers) | The aggregator mints a short-lived (≤ 5 min) **stack token**: a signed JWT whose claims are `sub` (the Access identity), `role`, `aud` (the target layer), `exp`, and `act` (the action class: comment, triage, dispatch, admin) | Target layer verifies the aggregator's signing key (published in the stack's shared config), audience, and expiry. The signing key is the aggregator's service credential — the one service secret this design introduces |
| Human → Kaneo / Pad / Grist native UI (not via the aggregator) | Those tools sit behind the same Access application (per #1182's single-entry rule they have no separate public route); where a tool supports OIDC/SSO natively it consumes the same IdP; where it does not, it is reachable only through the entry proxy and receives the stack token like any brokered write | Same as above; a layer with neither native SSO nor entry proxying is not exposed until it has one of them — that gap is an implementation issue, not an exception |
| Reads | Anonymous reads stay anonymous (§5). Authenticated reads carry the same claims so per-role read filtering is possible later without a second propagation design | Same validation as writes |

What this refuses: header trust ("the proxy said so" — an
`X-User: alice` header is spoofable by anything that can reach the
layer directly, so layers bind to loopback/stack-network only *and*
verify signatures anyway; network position is defence in depth, never
the check), shared API tokens per role (a token *is* an identity —
per-role tokens would recreate asserted authorship with extra steps),
and per-layer account databases (the thing being removed).

## 3. Role model

Four roles, checked server-side on every write, per action class:

| Role | May write | May not |
|---|---|---|
| `owner` | Everything, including role mapping changes and admin actions | — |
| `orchestrator` | Comments, triage actions (labels, priorities, queue operations), dispatches | Role mapping, admin |
| `agent` | Comments and triage actions **attributed to its own agent identity** (§4), dispatches only within its lane's grants | Anything as another identity; role mapping; admin |
| `read-only` | Nothing (reads only) | All writes |

- **Where the mapping lives:** a single registry table in the
  aggregator's store — the stack's one identity store — mapping
  identity (`sub`, plus agent ids from §4) → role. It is data, edited
  through an owner-only admin action, versioned (every change is an
  audit event), and cached by layers for at most the stack token's
  lifetime, so a role revocation takes effect within 5 minutes
  everywhere. No layer keeps its own copy of the mapping as truth.
- **Default deny:** an identity with no mapping row is `read-only`.
  An action class a role's row does not name is denied. Denials are
  logged with the claim that was presented, so "why was I denied" is
  answerable from the audit trail.
- Roles are coarse on purpose: four roles cover the stated need
  (#1253). Per-resource ACLs are a later design if a real case appears;
  this record does not build them.

## 4. Agent identity

Fleet agents must act as themselves, or attribution — the point of the
exercise — is lost for exactly the actors who write the most.

- Each fleet agent/lane has its own identity in the same registry:
  `agent:<lane-or-service-id>` (e.g. `agent:repo-worker-code`), role
  `agent`, granted by an owner action like any other mapping row.
- Authentication: the agent presents its stack credential — the
  fleet-secrets-issued per-agent token (fetched through the existing
  secrets broker route, never committed, never in a payload) — to the
  aggregator's token endpoint, which exchanges it for a stack token
  (§2) with `sub = agent:<id>`. The exchange is the only place the
  long-lived credential is used; layers only ever see 5-minute stack
  tokens.
- Attribution: every write carries `sub` end to end, and every layer's
  audit event records `sub`, `role`, `act`, and the object written.
  "The fleet did it" stops being an answer; the lane and the run are.
- An agent acting *for* Drew (a dispatch Drew ordered) is still
  attributed to the agent identity, with the dispatch reference in the
  audit event. Delegation is recorded, never impersonated: there is no
  mechanism in this design for a write to carry Drew's `sub` unless
  Drew's own Access session presented it.

## 5. Migration of existing records

- Existing dashboard comments and registry records carry asserted
  author strings (#941). On cutover they are **not rewritten**: each
  record gains a provenance marker distinguishing `asserted-legacy`
  (pre-cutover, author is a claim, displayed as-is with that caveat in
  the audit view) from `verified` (post-cutover, author is a `sub`).
  Rewriting history to look verified would falsify the audit trail the
  design exists to create.
- Asserted names that match a registry identity (same person, now
  with an Access identity) are linked by an owner-confirmed mapping
  pass — a display convenience (one profile page), never a change to
  the stored provenance.
- Anonymous reads: unchanged. The dashboard's public read surface
  stays readable without login where it is today; identity gates
  writes and non-public reads, not the public backstop view
  (consistent with the Phase-14 local-first/public-backstop posture
  in the wider programme).

## 6. Break-glass

When Access or the IdP is down, the stack must not choose between
"closed" and "asserted authorship returns". The posture:

- **Reads keep working** from the last-known state (the stack is
  local-first; the aggregator and layers run on the laptop, entry
  outage does not stop local reads).
- **Writes queue, attributed-but-unverified:** a write attempted
  during an entry outage is accepted only from the stack-local
  network, stamped `provenance: outage-queued` with the *claimed*
  identity kept visibly a claim, and held out of any surface that
  requires verified authorship until the entry recovers and the
  claimant's identity is confirmed by a fresh login — at which point
  the write is re-stamped verified or rejected by its claimant.
  Nothing outage-queued is ever silently promoted.
- **Owner break-glass:** the owner can, from the laptop directly,
  issue a single-use, ≤ 1 hour admin stack token via the aggregator's
  local CLI (the credential never leaves the laptop; the issuance is
  itself an audit event written before the token works). This is the
  recovery path for a broken role registry, and it is deliberately
  inconvenient: physical/local access is the factor.
- Asserted authorship as a fallback mode does not exist. If none of
  the above applies, writes fail closed and say why.

## 7. Open questions for Drew (named, not decided)

1. **IdP choice for Access:** which identity provider backs Cloudflare
   Access for the stack (the one #1182 assumes), and are non-Drew
   human users expected at all in v1, or is the human set exactly one
   owner plus agents?
2. **Grist/Pad native SSO gap:** if either cannot consume the IdP or
   the entry proxy acceptably, is the answer replacing the tool,
   accepting read-only exposure for it, or funding the proxy work?
3. **Agent credential custody:** per-agent tokens via the existing
   secrets broker (this record's assumption) vs per-lane credentials
   with separate rotation — a custody/rotation-policy call.
4. **Outage-queued writes:** is the queue-and-confirm posture in §6
   acceptable for comments specifically, or should comments fail
   closed during an outage and only triage/dispatch queue?

## 8. Implementation splits (for the follow-on filing)

To be filed as sub-issues of #1184 when this record lands:

1. Aggregator: Access JWT validation middleware + default-deny role
   check on every write route (closes the server-side half of #941).
2. Aggregator: identity registry table + owner admin actions + audit
   events for mapping changes.
3. Aggregator: stack-token minting (per-layer audience) + the agent
   token-exchange endpoint; layers: stack-token verification in Kaneo,
   Pad, Grist write paths.
4. Comment API: stop accepting author from the request body; stamp
   provenance (`verified` / `asserted-legacy`) — closes #941.
5. Migration pass: provenance markers on existing records + the
   owner-confirmed name-linking pass (§5).
6. Break-glass: outage queueing + the local owner token CLI (§6),
   gated on Drew's answer to question 4 for the comment surface.
