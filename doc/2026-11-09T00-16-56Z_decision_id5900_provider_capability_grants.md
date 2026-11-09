# Decision: capability grants for providers — grant shape, sensitivity classes, approval flows

- **Date:** 2026-11-09
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5900](https://github.com/developing-today/code/issues/5900), "id: design: capability grants for providers (from #5686)". Queue task 13837.
- **Parent design:** [#5686](https://github.com/developing-today/code/issues/5686) — the single capability-provider model (signed, typed, versioned declarations; the ceiling model of #789; grants explicit, revocable, auditable; observation ≠ actuation). This record does not re-decide the parent; §1 lists what it takes as given.
- **Consumed, not re-decided:** [#5903](https://github.com/developing-today/code/issues/5903) embedded decision (two-gate composition on the phone; session-bound instantiation in the engine) and [#5904](https://github.com/developing-today/code/issues/5904) offline decision (presence, buffering, and §5 cached-grant expiry semantics). §6 states the boundary with each explicitly.
- **Substrate:** the attenuated-capabilities decision (`doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/`) — authority is carried by capabilities, attenuation only narrows, the grant ledger survives as the world ceiling, identity never grants authority — and its enforcement surface `world_caps` (per-world grant ledger in `pkgs/id/crates/id-core`). This record is the provider-facing generalisation of that decision, not a replacement for it.
- **Inputs:** the capability-token field survey [#5931](https://github.com/developing-today/code/issues/5931) §6.4 (UCAN 1.0, Biscuit 3.x, macaroons, IETF AAT draft: attenuate-only offline-verifiable delegation is a solved, multiply-implemented pattern; the open question was encoding vs ID-native objects — answered in D1).

## Decision summary

| Question | Decision |
| --- | --- |
| Grant shape | A signed, ID-native grant object binding capability + provider + consumer, with purpose, rate/volume bounds, sensitivity class, absolute expiry, and use limits as first-class fields (D1) |
| Encoding | ID-native signed objects with UCAN-isomorphic attenuation semantics; no foreign token wire format in v1 (D1) |
| Classes | Every capability is declared **observation** or **actuation** in the provider manifest; the class is part of the capability, never inferred at use time (D2) |
| Standing grants | Observation may be standing within expiry. Actuation is never ambient and never standing by default; standing actuation requires an explicit standing-grant record under D3 bounds (D2, D3) |
| Approvals | Scaled by sensitivity class S0-S3: policy-auto, approve-once, per-grant + threshold confirm, per-use (D3) |
| Delegation | Attenuate-only: subset of capabilities, narrower-or-equal bounds, earlier-or-equal expiry, never a higher sensitivity class (D4) |
| Revocation | Ledger revocation is immediate wherever a live check happens; offline behaviour is exactly #5904 §5 (D4) |
| Audit | Issuance, delegation, actuation use, and revocation are journaled with grant id and lineage; observation use is journaled where results are consumed (D4) |

## 1. What the parent already fixed (not re-decided here)

From #5686 / F13.7 and the attenuated-capabilities record, taken as given:

1. One provider model for every target class; a rule needing a per-target special case is wrong, not the table.
2. A provider offers nothing above its ceiling; a consumer receives nothing above its grant; client providers default to `default_ceiling: excluded`. Effective authority is always an intersection (§5 below).
3. Two keys for client providers: the world grant **and** the client's own per-world opt-in, checked on every request.
4. Grants are explicit, revocable, and auditable; refusals carry stable error codes (F11.13).
5. Identity never grants authority. A grant names identities for binding and audit; possession plus validity of the grant object is the authority.

## D1 — The grant object

A grant is a signed object in ID's native object model (the same signing and writer-set machinery as provider manifests), carrying:

- `grant_id` and `parent_grant_id` (null for a root grant) — the lineage every audit entry references.
- `capability` — one capability identifier from the provider's manifest (a grant covers exactly one capability; breadth comes from holding several grants, never from a wildcard field).
- `provider` — the provider identity **and instance/manifest version** the grant is against. A grant does not float to a different provider, a different device, or a silently upgraded manifest.
- `consumer` — the world / principal the grant is issued to. Grants are non-transferable except by delegation under D4, which mints a new object rather than passing this one on.
- `purpose` — a short declared purpose string where the capability admits one (required for actuation classes S2+; free text for the audit record, never parsed into authority).
- `bounds` — rate and volume limits in the capability's own units (requests/minute, bytes, samples, actuations/day — the manifest declares which units a capability is bounded in). Bounds are part of the grant, not of the request.
- `sensitivity` — the class of D3, copied from the manifest's declaration for the capability at issuance; a grant can never carry a lower class than the manifest declares.
- `issued_at`, `expires_at` — absolute times. Expiry is a field of the grant; use never renews it (composes with #5904 §5 verbatim).
- `max_uses` — optional use-count limit; required for S3.
- `standing` — boolean, with the D2/D3 restrictions on when it may be true.

**Encoding verdict:** ID-native signed objects, with attenuation semantics isomorphic to UCAN (a delegation chain of signed, narrowing caveats, verifiable offline). Rejected for v1: adopting UCAN/Biscuit/macaroon wire formats directly — the survey (#5931 §6.4) shows the *pattern* is what matters and it is multiply-implemented; importing a foreign format buys a second object model, a second signature stack, and a mapping layer to our manifests, for interop nobody consumes yet. The isomorphism is deliberate: if a future interop decision needs UCAN on the wire, grants translate field-for-field instead of being redesigned. Also rejected: continuing with ledger-only grants (the `world_caps` ACL shape) as the general model — a ledger entry cannot be delegated, cannot be verified offline by a bridge device, and cannot carry bounds or lineage; the ledger survives in its proper role, the ceiling (§5).

## D2 — Observation vs actuation, and the no-ambient rule

- The provider manifest declares each capability as **observation** (its use returns data and changes nothing outside the journal) or **actuation** (its use changes state outside the journal — an output, a write, a physical effect), reusing #5904 §3's definitions verbatim. The class is a property of the capability in the manifest; neither the consumer nor a grant can reclassify it.
- **Observation grants may be standing**: issued once, presented or resolved by reference on each use, valid until expiry or revocation. Standing observation is the normal case (sensor feeds, state reads).
- **Actuation is never ambient.** No actuation happens because a session is open, a device is paired, an OS permission exists, or a grant was approved at some earlier point and forgotten. Every actuation request presents its grant at use time, the grant is validated at use time (signature, expiry, bounds, revocation, ceiling intersection), and the use is journaled. Ambient authority — authority that flows from *context* rather than from a presented grant — is the failure mode this rule exists to exclude; the two-gate composition of #5903 (an `id` grant **and** a platform permission must both pass) is a tightening of this rule, never a substitute for the grant.
- **Actuation grants are not standing by default.** The default actuation grant is purpose-bound and short-lived (D3). A **standing actuation grant** exists only as an explicit record with the `standing` flag set at issuance under the D3 approval for its class, its own (tighter) bounds, and a mandatory expiry — there is no indefinite standing actuation at any class.

## D3 — Sensitivity classes and approval flows

The manifest assigns every capability a sensitivity class; approval requirements scale with it and are properties of issuance, not of individual requests (except S3).

| Class | Meaning | Examples | Approval flow | Standing | Expiry / uses |
| --- | --- | --- | --- | --- | --- |
| **S0** — public observation | data the provider publishes to any consumer | public presence, coarse weather | policy-auto: issuance is a configuration fact, no per-grant human approval | yes | long expiry permitted |
| **S1** — private observation | data about an owner's world/devices | sensor history, device state | approve-once by the grantor (owner or their delegate) at issuance | yes | expiry required |
| **S2** — reversible actuation | effects that can be undone or expire on their own | set a mode, post a message, queue a job | per-grant approval at issuance; per-use confirmation when a single use exceeds a manifest-declared volume threshold | only as an explicit standing record (D2) | expiry required; bounds required |
| **S3** — consequential actuation | irreversible, physical-safety, financial, or outward-facing effects | unlock, dispense, send externally, delete | **per-use approval**: every use is approved by the grantor at use time; the grant authorises asking, not doing | **never** | shortest expiry; `max_uses` required; purpose required |

Rules that cut across the table:

- Approval is by the grantor (the owner of the provider side, or a holder of a delegation whose scope explicitly includes approving — D4). A consumer can never approve its own grant or its own use.
- An approval flow may be *stricter* than the table for a given world or provider (a world may require per-use approval for an S2 capability); it may never be looser. Loosening a class is a manifest change, reviewed as such.
- Denied or expired approval is a refusal with a stable code, journaled like any other refusal; it is never retried silently under a new identity (#5904 §3's rule for expired requests applies unchanged).

## D4 — Issuance, attenuation/delegation, revocation, audit

- **Issuance.** Only a grantor holding authority over the provider (ownership, or a grant whose scope is issuance itself) can mint a root grant, and only within the ceilings of §5. Issuance is journaled with the full grant object.
- **Attenuation / delegation.** A holder may mint a child grant from its own, and a child grant may only ever be **narrower**: a subset relationship on the same single capability, bounds less than or equal, expiry earlier than or equal, `max_uses` less than or equal, sensitivity class **unchanged** (a class is a property of the capability, not a dial a delegator may turn down), and `standing` may be dropped but never gained. Delegation depth is recorded in lineage; a provider manifest may cap it. This is the attenuated-capabilities rule ("attenuation only narrows; nobody can widen") restated for the provider grant shape — one rule, two surfaces, deliberately.
- **Revocation.** The grantor (or any ancestor in the lineage) may revoke a grant or a whole subtree at any time by appending a revocation to the grant ledger. Wherever a use is checked against the live ledger, revocation is effective immediately, including mid-standing-period. Revocation stops future uses; it does not erase history — journaled results stand (parent §1, given). The offline case is not re-decided here: #5904 §5 governs cached snapshots (observation only, until the earlier of grant expiry and manifest cache TTL, never actuation), and a revocation learned later invalidates a snapshot from the moment it is learned.
- **Audit invariants.** (a) Every issuance, delegation, revocation, S2/S3 approval decision, and every actuation use is a journal entry naming `grant_id`, lineage root, capability, provider instance, consumer, and outcome/refusal code. (b) Observation uses are journaled wherever their results are consumed by `update` (parent rule); pure routing checks are not journaled. (c) The journal must be sufficient to answer, for any actuation that ever happened: under which grant, approved by whom, within which bounds — with no other data source required.

## 5. Composition with the ceiling model (invariants implementation must hold)

1. **Three-way intersection.** Effective authority at any use = grant scope ∩ provider ceiling (what the provider offers at all, per its manifest and the world's `world_caps` ceiling) ∩ consumer ceiling (what the world permits its consumers to hold). Removing a capability from a ceiling takes effect across every outstanding grant at once, without reissuing them (attenuated-capabilities rule, unchanged).
2. **Check at use, not at issuance only.** Ceilings move; grants are checked against the current ceilings on every use, alongside signature, expiry, bounds consumption, and revocation state.
3. **No layer grants what another forbids.** An OS/runtime permission (#5903 two-gate), a platform registry state, a session's presence, or a transport's authentication can each only *subtract* from the intersection or pass it through — never add to it.
4. **Refusals are stable and legible.** Every refusal path above (expired, revoked, over bounds, over ceiling, class approval missing, ambient actuation attempted) maps to a stable F11.13 refusal code and a journal entry; a consumer can always distinguish "ask for a grant" from "ask again later" from "never".

## 6. Boundaries with the sibling decisions (consumed, not duplicated)

- **#5903 (embedded).** The `provider` binding of D1 is to provider identity + instance/manifest version generally; the #5903 rule that engine providers are session-bound and grants bind to the session instance by default is the embedded instantiation of that binding, and stands. The #5903 phone rule — an `id` grant and an Android runtime permission are two gates, both must pass, intersection semantics — is an instance of invariant 5.3 and is not restated as a new mechanism here. Nothing in this record changes any #5903 verdict.
- **#5904 (offline/degraded).** Grant *shape* is decided here; grant *behaviour while unreachable* belongs to #5904 and is consumed verbatim: actuation is never queued for later execution except where manifest and grant both explicitly declare store-and-forward; cached grant snapshots authorise observation only, until the earlier of grant expiry and the manifest cache TTL, never actuation; expiry is absolute and use never renews it. The expiry and bounds fields #5904 §5 assumed are the `expires_at` and `bounds` fields of D1 — that dependency is now discharged, with no change to #5904 semantics.
- **#5898 / #5899 / #5905 (declaration, discovery & negotiation, security model)** remain the decisions of their owning splits; this record assumes declarations carry the class/bounds vocabulary of D1-D3 for #5898 to spell, and takes no position on the #5905 threat model beyond the invariants in §5.

## Alternatives rejected (summary)

- **Ledger-only grants as the general model** — cannot delegate, cannot verify offline, cannot carry bounds/lineage (D1).
- **Foreign token wire format (UCAN/Biscuit/macaroon) as the v1 encoding** — the pattern is adopted, the format is not; a second object/signature stack for unconsumed interop (D1).
- **Ambient actuation from session, pairing, or OS permission** — the exact failure D2 exists to exclude.
- **Per-target grant shapes** (a phone grant, an engine grant, ...) — fails the one-model test of the parent; the class table and D3 flows are target-independent by construction.
- **Sensitivity as a per-request judgement** — classes must be manifest facts, or approval flows become negotiation at use time and audit cannot reconstruct why a use was allowed.

## What this record does not do

No code, no manifest schema, and no `world_caps` changes are specified here beyond the field vocabulary of D1. Implementation work (grant object in the ID object model, ledger revocation surface, journal entry shapes, the #5898 manifest spelling of class/bounds) is follow-on work to be filed from this record; #5900 stays open until this record is merged, per its acceptance.
