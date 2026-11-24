# Broker TLS — fleet brokers on the program CA (design record for #13685)

- **Issue:** developing-today/code#13685 (fleet-ca: broker TLS — long-term design line)
- **Program:** #9155 (Fleet CA, credentials & secrets); custody #9158 / #9163; issuer
  lifecycle & publication #9164; CLI/crate architecture #9157; companion hierarchy #13658
- **Status:** DESIGN ONLY. No broker configuration change, no certificate issuance,
  no rollout is authorised by this record.
- **Governing ruling (Drew, 2026-11-23):** broker TLS is a **long-term plan line** in
  the fleet CA program. It is **not a Phase 1 blocker**, it **gates nothing** in the
  `fleet cert` build-out, custody, or publication phases, and today's **token auth
  stands unchanged** until this line activates in the program's later order.
  Production rollout, when the design is accepted and built, is a production action
  under the program's production gate on #9155.
- **Queue task:** 23477 (design packet routed by triage, 2026-11-24).

## 1. Why this record exists

The CA-RECON sweep of 2026-11-23 (recorded on #13658 and #9155) found no broker-TLS
provision anywhere in the governing spec
(`doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/sources/fleet_architecture_spec_v0_3.md`):
the fleet brokers appear in the design only as existing token-auth domains. The
fleet's own control plane — the components that hand out secrets, hand out work,
and execute on the laptop — had no path onto the program CA. #13685 is the design
line that closes that gap, on the program's schedule rather than ahead of it.

## 2. Fresh-state revalidation (2026-11-24, this pass)

- #13685: open; two triage verdicts (oc-code-07, oc-code-08, 2026-11-24) both route
  it as a design item; **no design record, no child issues, no PRs** existed before
  this pass (verified by issue read, sub-issue list, and open-PR search).
- #9155: open; Drew's 2026-11-23 ruling is the authority for this line's posture.
- #13658: the IdP CA (classical P-384 hierarchy, live under that story's own
  authority) is the **companion** hierarchy whose relying parties are the SAML/OIDC
  stacks. Its recon record states the same gap from the other side: the spec
  contains no provision for putting the fleet brokers on TLS from the fleet CA.
- The issuance machinery this design consumes already exists on main:
  `pkgs/fleet/crates/fleet-cert` (the `fleet cert` domain, #9157) — algorithm
  registry, cross-signing graph as data/policy, leaf profile registry mirroring
  `pkgs/idp-pki`'s `idp-ca` (`server`, `client`, `saml-signing`, `ocsp-responder`),
  bundle/manifest handling, and the safety rail that signing always requires an
  explicitly passed signer source (no ambient key discovery).

## 3. The brokers today (census, from source)

| Broker | Crate | Transports today | Auth today |
| --- | --- | --- | --- |
| Secrets broker | `fleet-secrets` | Unix sockets: session broker socket + per-role socket (dual-bind, #6216/#6218); off-host reach via the session-proxy leg | Bearer token from a token file; socket access gated by filesystem permissions |
| Queue broker | `fleet-queue` | TCP on the tailnet (`QUEUE_BROKER_BIND`, default `100.99.113.103:4801`); local UDS via the shared dual-bind machinery (`uds_http.rs`) | Bearer token from a token file (`~/.config/queue-broker/token`) |
| Laptop-agent | `fleet-laptop-agent` | TCP agent endpoint on the laptop; legs to the other brokers over tailnet/proxy (one leg also carries a Cloudflare Access service token) | Bearer token from a token file |

Properties of the current state that the design must respect:

1. **All confidentiality/integrity off-host comes from the tailnet** (WireGuard)
   plus bearer-token possession. There is no per-component identity: any holder
   of a broker's token is that broker's client, indistinguishably.
2. **UDS legs are already strongly scoped** by filesystem permissions and are
   not network surfaces. They are not the problem this design solves.
3. **The secrets broker is the recovery path for the other two.** Every other
   component's credentials ultimately resolve through it. Any design that makes
   the secrets broker depend on material it cannot obtain without itself is a
   lockout machine; §6 orders the migration around this fact.

## 4. TLS identity for the three brokers

Issuer-set choice and profile are decided in the ADR filed alongside this record
(`adr-0001-broker-issuer-and-profiles.md`). Summary:

- **Issuer:** the fleet CA's **Base issuer set**, A/B partitions both eligible
  (brokers verify against both partitions; issuance normally from the active
  partition). Not the Internal Web set (its profile is web-SAN service leaves),
  and not the IdP CA of #13658 (its relying parties are SAML/OIDC stacks; brokers
  are not relying parties, and Drew's ruling keeps that hierarchy on its own
  backing-in path).
- **New leaf profiles** in the `fleet-cert` profile registry, derived from the
  existing `server`/`client` profiles (P-256, `serverAuth`/`clientAuth`):
  - `broker-server` — TLS server leaf for a broker's TCP leg.
  - `broker-client` — TLS client leaf for a fleet component dialling a broker.
  - Differences from the base profiles: **90-day default validity** (the spec's
    §5.4 leaf default, not idp-ca's 730 days — broker leaves are machine-managed
    and must be cheap to lose), mandatory SAN set (below), and renewal automated
    from first issuance (a broker leaf is never a hand-renewed artifact).
- **SAN plan (per broker, recorded in its issuance request):** the broker's
  internal DNS name(s) under the program's `.internal` naming, plus its tailnet
  IP as an IP SAN. No public DNS names, no wildcards: a broker is a single named
  component, and its certificate says exactly that.
- **Key custody for leaves:** keys are generated **on the host that runs the
  broker**, the CSR goes to the issuer, the private key never moves. Leaf keys
  are a machine-local credential class under #9158 (see §7 review): not escrowed,
  not replicated, worthless off the host by construction of the migration rules
  in §5 (a stolen leaf key alone never suffices — the token is still checked).
- **Algorithms:** P-256 leaves are the primary track (matches the profile
  registry and the fleet's rustls/aws-lc-rs stack). Ed25519 issuer partitions
  are eligible issuers when active. **ML-DSA leaves are deferred**: the CA
  design's baseline scheme becomes the broker track when the TLS stack can
  verify ML-DSA chains end to end; until then PQ-only remains a verifier-policy
  matter (§3.2 of the spec), and the cross-signed issuer graph is what lets
  broker chains build into both trust selections later without re-issuance of
  roots. This is a deferral, not a rejection, and slice BT-1 records the
  re-check as an explicit acceptance item.

Per-broker application:

| Broker | Server cert | Client cert | Notes |
| --- | --- | --- | --- |
| Queue broker | `broker-server` on the TCP leg (`:4801`) | `broker-client` when it dials the secrets broker / laptop-agent | First mover (§5): its clients are numerous but its own recovery does not depend on another broker's TLS |
| Laptop-agent | `broker-server` on its TCP endpoint | `broker-client` on its broker legs | Second: it is a client of both other brokers and a server to fleet clients |
| Secrets broker | `broker-server` on its off-host leg only | `broker-client` for broker peer sync if/when that lands (#13948's line) | **Last** (§5, rule M4). UDS legs never take TLS (decision D-UDS, §5) |

## 5. mTLS posture and migration path

### 5.1 The posture decision

Certificates and tokens do different jobs in this design, permanently:

- **Certificate = component identity.** Which named fleet component is on the
  other end of this connection, attested by the program CA.
- **Token = authorisation credential.** The existing per-broker bearer token,
  unchanged in semantics, remains the authorisation check.

Certificates **augment** token auth; they do not replace it at any phase of
this design. Retiring a broker token is a separate future decision, per broker,
with its own record — it is not a phase of this migration. This keeps the
blast radius of a CA incident disjoint from the blast radius of a token leak:
either factor alone is insufficient on the TCP legs once Phase 3 lands.

### 5.2 Phases (per broker, independently)

- **Phase 0 — today.** Token only. No code path knows what a certificate is.
- **Phase 1 — server authentication.** The broker presents its `broker-server`
  leaf on the TCP leg; clients validate against the distributed trust bundle
  (§6) and fail closed, with a named operator override for break-glass. Token
  auth unchanged. Clients without bundle support keep working over UDS or the
  tailnet leg only until they are bundle-capable — Phase 1 never *removes* a
  working path, it adds a verified one and makes it the default for capable
  clients.
- **Phase 2 — dual-accept client authentication.** Clients present
  `broker-client` leaves where they have them. The broker logs the certified
  identity alongside the token identity and accepts either presentation
  (token always required; cert optional). Mismatched cert/token identity
  pairs are logged, not yet denied — this phase exists to surface every
  legitimate pairing in the estate before any denial is switched on.
- **Phase 3 — required client certificates on TCP legs.** On the TCP legs, a
  valid `broker-client` leaf **and** the token are both required, and the
  cert identity must be on the broker's recorded pairing set from Phase 2.
  UDS legs are unchanged at every phase.

### 5.3 Migration rules (the never-lockout contract)

- **M1 — Additive flags, permissive defaults.** Every phase transition is a
  config flag whose default is the more permissive posture. A bad config push
  degrades toward Phase 0 behaviour, never toward denial.
- **M2 — Dual-accept windows are measured in releases.** A phase's accept-both
  window stays in the binary for at least one full release cycle after the
  estate reports full adoption (Phase 2 logs are the adoption evidence).
- **M3 — UDS break-glass is permanent.** Local UDS access with the token is a
  designed, permanent recovery path on every broker. It is not a migration
  leftover and no phase removes it.
- **M4 — The secrets broker migrates last, and never depends on itself.** Its
  leaf renewal path is local (on-host key, CSR over the UDS leg / direct
  `fleet cert` invocation with an explicitly passed signer source). At no
  point may obtaining or renewing the secrets broker's own certificate
  require a secret that only the secrets broker can hand out.
- **M5 — One broker at a time, in census order.** Queue broker → laptop-agent
  → secrets broker. A broker does not enter Phase *n+1* until the previous
  broker has completed Phase *n* without a rollback event.

## 6. Trust distribution

Broker clients get their trust anchors through the program's machinery, never
through a broker-specific side channel:

- **Anchors** are the program trust bundles of the spec (§3.5): broker clients
  install the **classical compatibility bundle** (`roots-classical`, or
  `roots-all` where the runtime supports it). The PQ-only bundle (`roots-pq`)
  is not a broker bundle until ML-DSA broker leaves exist (§4).
- **Publication** is #9164's: AIA `caIssuers` pointing at the program mirrors
  (the two public mirrors and the `.internal` mirror — broker chains resolve
  from the internal mirror first), explicit valid chains distributed with the
  bundle because clients may ignore or stop early on AIA, CDP entries only
  where real signed CRLs exist, and **no OCSP URL in a broker leaf** unless a
  functional responder for the Base set exists (broker revocation checking is
  CRL-based by profile default).
- **Delivery and rollover** follow the trust-bundle controller pattern of spec
  §27.3 item 9: versioned bundles, **two-phase rollover** (distribute the new
  anchor set → collect acknowledgements → activate → retire the old set), and
  per-runtime tracking. Until the general controller exists, the broker line
  uses its minimal form: bundles produced by `fleet cert trust export`,
  delivered with the fleet binary/config deployment, version-stamped, and
  **acknowledged in the broker handshake health surface** — a broker reports
  the bundle version each connected client presented with, so a stale client
  is visible before it is broken by a rollover. Building the general
  controller stays #9164/§27.3 work; slice BT-2 builds only the broker-scoped
  consumption and acknowledgement, against the same versioned format, so the
  controller adopts it rather than replacing it.

## 7. Review against #9158 (custody) and #9164 (publication)

Reviewed 2026-11-24 against the #9158 and #9164 scope/acceptance seeds and the
governing spec sections they refine (§§3, 5.4, 13, 24, 27.3, 43–44) and the
addendum's custody refinements.

**Custody (#9158) — outcome: consistent, two recorded notes.**

1. Broker leaf private keys are a **new credential class** for the canonical
   model: machine-generated, machine-local, single-host, never replicated,
   never escrowed, fail-closed on export (there is no export). This fits
   #9158's per-credential replication policy and its fail-closed classes; the
   class must be enumerated in #9158's schema work when this line activates —
   slice BT-1 carries that as an explicit cross-reference, so the class is
   adopted by the custody model rather than invented beside it.
2. Nothing in this design touches root or issuer custody: the three-destination
   escrow (encrypted laptop storage, authorised BWS project, authorised
   1Password vault), tested restoration, and explicit-signer-source rules all
   stand; `fleet-cert` already enforces the signer-source rail in code.
   Broker tokens remain canonical Credential records under the existing model,
   by name only, exactly as today.

**Publication (#9164) — outcome: consistent, one recorded dependency.**

1. Broker leaves consume the standard publication profile — mirror set,
   AIA/CDP discipline (real signed CRLs before any URL is embedded), explicit
   chain distribution — with one profile-level tightening #9164's framework
   permits: no OCSP URL until a functional Base-set responder exists.
2. **Dependency, stated plainly:** the §27.3 trust-bundle controller is
   roadmap, not built. This design does not silently assume it. Slice BT-2 is
   scoped to the minimal broker consumption path in the same versioned format
   (§6), and the general controller remains a #9164-line deliverable that
   later absorbs it. If #9164's publication work reorders, BT-2 is the only
   broker slice whose schedule moves.

No conflict with either issue's safety gates was found: this record proposes
no production CA operation, no vault write, no live escrow, and no rotation.

## 8. Failure and rollback story

| Failure | Detection | Response | Rollback |
| --- | --- | --- | --- |
| Broker leaf expires | Renewal is automated at 50% of the 90-day life; health surface reports leaf age; alert at 75% | On-host renewal (M4 path for the secrets broker); overlap window: the broker can present the renewed leaf without dropping the old one mid-flight | Expiry degrades that broker's TCP leg to the M1 permissive posture by flag, not to silence; UDS unaffected |
| Issuer partition compromise / mis-issuance | #9164 chain-matrix tests + CRL issuance | Brokers verify against **both** A/B partitions; issuance fails over to the clean partition; compromised partition's leaves revoked via real signed CRLs on the mirrors | Remove the compromised partition from the bundle by two-phase rollover (§6); tokens were never dependent on it, so authorisation is untouched |
| Trust-bundle distribution failure | Bundle-version acknowledgements in the health surface (§6) show stale clients | Rollover pauses at the distribute phase — activation never proceeds without acknowledgements; clients keep last-known-good anchors | Old anchor set remains active; nothing to roll back because activation is the last step |
| Bad config push (phase flag) | Broker startup self-check + health surface | M1: defaults are permissive; a flag that fails validation is refused at load and the previous posture kept | Set the flag back; the previous phase's accept path is still in the binary (M2) |
| CA unavailable at renewal time | Renewal attempts from 50% life leave a 45-day runway | Retry with backoff across both partitions; alert well before expiry | The runway *is* the rollback: no same-day expiry cliff exists in this design |
| Full retreat needed | Operator decision | — | Phase flags return the estate to Phase 0 (token-only) per broker, in reverse census order (secrets broker first on retreat). The Phase 0 accept path is retained in the binary until a separate removal decision is recorded — removal is not part of this design |

## 9. Slice map (implementation issues filed under #13685)

Filed as native sub-issues of #13685 when this record was written. All are
build slices for the program's later order — none is a production action, and
production enablement of any phase remains subject to #9155's production gate.

- **BT-1** — `fleet-cert`: `broker-server` / `broker-client` leaf profiles,
  broker SAN policy, 90-day validity, ML-DSA re-check item; register the
  machine-local leaf-key credential class with #9158's model.
- **BT-2** — Trust bundle, broker consumption: versioned bundle install via
  `fleet cert trust export`, two-phase rollover client behaviour, bundle-version
  acknowledgement in broker health surfaces (§6; absorbs into the §27.3
  controller when it lands).
- **BT-3** — Queue broker Phase 1: TLS server authentication on the TCP leg,
  token unchanged, break-glass override named and logged.
- **BT-4** — Laptop-agent Phase 1: TLS server authentication on its endpoint;
  client-side bundle validation on its broker legs.
- **BT-5** — Secrets broker Phase 1: TLS server authentication on the off-host
  leg only, migrated last, renewal path provably independent of the broker
  itself (M4 test).
- **BT-6** — Phase 2 across brokers: optional client certificates, identity
  logging, cert↔token pairing census (the Phase 3 admission list).
- **BT-7** — Phase 3 across brokers: required client certificates on TCP legs,
  pairing enforcement, failure-matrix tests (§8 rows as test cases), rollback
  drill in a synthetic estate. Production enablement explicitly out of scope.

Dependency order: BT-1 → BT-2 → BT-3/BT-4/BT-5 (census order, M5) → BT-6 → BT-7.

## 10. Non-goals

- No production rollout, no live broker reconfiguration, no certificate
  issuance — all gated on #9155's production gate and this design's acceptance.
- No change to the IdP CA (#13658) or its backing-in path.
- No TLS on UDS legs (decision recorded in §5: filesystem scoping already
  provides the property TLS would add, and wrapping UDS in TLS would spend
  complexity for no identity gain — peer credentials, not certificates, are
  the UDS identity mechanism if one is ever needed).
- No token retirement decision (§5.1).
- No SPIFFE/SPIRE adoption: spec §27.3 items 1 and 5 keep workload-identity
  frameworks as a studied future track; this design's component identities are
  program-CA leaves, and nothing here forecloses a later SPIFFE mapping.
