# Decision: provider declaration model — manifest contents, signing, versioning, dynamic vs static capabilities

- **Date:** 2026-11-11
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5898](https://github.com/developing-today/code/issues/5898), "id: design: provider declaration model (from #5686)". Queue task 14785.
- **Parent design:** [#5686](https://github.com/developing-today/code/issues/5686) — the single capability-provider model, in-tree record `doc/2026-10-25T05-37-57Z_design_client_capability_providers/` (F13.7: signed, typed, versioned declarations offered into the ceiling model of #789; grants explicit, revocable, auditable; observation ≠ actuation; a client provider is a named, versioned provider with an F12.4 manifest plus the `side` field). This record does not re-decide the parent; §1 lists what it takes as given, open questions that would change a parent decision are routed back to #5686 in §8, and the conflicts check at the end is explicit.
- **Consumed, not re-decided:** [#5899](https://github.com/developing-today/code/issues/5899) discovery & negotiation record (PR #7603, in flight at decision time) — discovery is a filtered view over this record's artefacts, and §3.1 of that record requires the declaration artefact to carry `scope`, `ttl_class`, and `epoch`, or else carry them in a sidecar envelope; D2 discharges that requirement inside the artefact. [#5900](https://github.com/developing-today/code/issues/5900) grants decision (`doc/2026-11-09T00-16-56Z_decision_id5900_provider_capability_grants.md`) — grants bind the tuple (provider identity, capability id, manifest version); sensitivity classes S0–S3 are manifest facts; its §6 assumed this record would spell the `expires_at` and `bounds` fields, discharged in D2. [#5903](https://github.com/developing-today/code/issues/5903) embedded decision (`doc/2026-11-08T09-10-00Z_design_embedded_providers_decision.md`) — the authoritative declaration is `id`'s own signed, versioned provider manifest (the #5898 / #1061 writer-set artefact); platform registries are projections, never reverse-authoritative. [#5904](https://github.com/developing-today/code/issues/5904) offline/degraded decision — presence classes and cached-grant expiry semantics consume the TTL and expiry fields fixed here. [#5905](https://github.com/developing-today/code/issues/5905) security decision (`doc/2026-11-09_decision_id5905_provider_security_model.md`) — a provider is its keypair; manifests are signed by the provider identity and verified at admission and on every manifest change; capability identity is the tuple (provider identity, capability id, manifest version); admission is an explicit registry action and every provider enters at `default_ceiling: excluded`. This record gives those statements their concrete artefact.
- **Inputs:** the capability-provider field survey [#5931](https://github.com/developing-today/code/issues/5931) §6.7 + §6.2 (declaration surfaces in the wild: MCP `tools/list`, AppFunctions' OS registry, WebMCP registration, WIT interfaces, Matter clusters, set against the manifest model — an inventory, not a decision); the current code at origin/main `effdff265b`: `ProviderManifest` in `pkgs/id/crates/id-core/src/world_caps.rs` (name, contract version, kind, argument schema, bounds, required scope, default-ceiling flag) behind a static `ProviderRegistry` — the static catalog this model must subsume as its degenerate case.
- **Ceiling-before-grant:** unchanged. A declaration is an offer into the ceiling model, never an authority; nothing in this record widens any ceiling.

## Decision summary

| Question          | Decision                                                                                                                                                                                                                                                                                          |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The artefact      | One signed provider manifest per (provider identity, instance) is the sole declaration artefact for every provider class; platform registries are generated projections of it, never the reverse (D1)                                                                                             |
| Manifest contents | A fixed signed field set: identity/instance/name/`side`/kind, three version axes, epoch, scope, TTL class, issue/expiry times, and per-capability entries carrying id, contract version, typed argument schema, bounds, required scope, sensitivity class, and observation/actuation class (D2)   |
| Signing           | Signed by the provider identity key over the canonical encoding of the whole manifest; verified at admission, on every change, and when a grant binds its tuple. Registry admission is a separate registry record referencing the manifest digest — never a countersignature on the manifest (D3) |
| Versioning        | Three axes, never conflated: `format_version` (declaration format), monotonic integer `manifest_version` (immutable once filed; amendment = new version), per-capability `contract_version` (same-major = compatible). `epoch` generations guard rollback (D4)                                    |
| Dynamic vs static | The capability **set** is static within one `manifest_version`; **availability** is dynamic (liveness, suspension, which version is live). A changed offer is a new signed `manifest_version`, never a runtime mutation (D5)                                                                      |
| Lifecycle         | filed → admitted → live → suspended → superseded / stale → tombstoned; withdrawal is itself a signed object, so a bridge or registry cannot forge it (D6)                                                                                                                                         |

## 1. Taken as given from the parent and siblings

1. One provider model for every class — ESP32/MQTT, terminal, Bevy, smartphone, web (#5686). A per-target declaration format is a parent-level change, not a choice available to this record.
2. Declarations are signed, typed, and versioned, offered into the #789 ceiling model; every provider enters at `default_ceiling: excluded` until a ceiling admits it (#5686 F13.7, #5905 D2, and the #1061 registry test-provider proof in current code).
3. Grants are explicit, revocable, and auditable, and bind exactly one capability tuple (provider identity, capability id, manifest version) (#5900, #5905 D6). Declaration therefore owes grants a stable, immutable binding target.
4. Observation ≠ actuation, and sensitivity classes S0–S3 are manifest facts, not per-request judgements (#5686, #5900).
5. A provider is its keypair; names and transport addresses are never identity (#5905 D1/D3).
6. Discovery is a filtered view over declaration records; there is no second discovery database and no announcement channel (#5899 §3.1).
7. Transports and bridges carry no authority and cannot mint or alter signed objects (#5903, #5905 D5).

## 2. The decisions

### D1 — One artefact: the signed provider manifest

- The declaration artefact is a single signed object in the ID object model — the same object and signature stack the grant object uses (#5900 D1 rejected a foreign token wire format for grants; the same reasoning binds here: a second object/signature stack for declarations would be paid for in verification code and audit joins, for interop nobody consumes yet). Foreign declaration surfaces (MCP `tools/list`, AppFunctions, WebMCP, WIT, Matter) are import/projection targets per #5903 D1 and survey #5931 §6.7, never the canonical form.
- One manifest declares one provider instance's whole offer. There are no partial or per-capability declaration documents: a consumer, a registry, or an auditor reading one manifest version sees the complete offer that version makes, which is what makes the capability tuple in a grant a complete binding.
- **The static catalog is the degenerate case, not a second model.** Today's compile-time `ProviderManifest` entries in `world_caps.rs` are declarations whose provenance is the binary itself: at registry load they are projected into the D2 field shape (host-attested provenance, `ttl_class: static`, world scope, no expiry). Adding a built-in provider must remain exactly "one manifest entry" — the property the #1061 test provider already proves — and a built-in and a runtime-filed provider become indistinguishable to grants, discovery, and audit downstream of admission.

### D2 — Manifest contents

The signed field set (every field below is inside the signature unless marked otherwise):

**Provider block**

| Field                     | Content                                                                                                                                                                                                                                                        |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `format_version`          | Version of the declaration format itself (D4, axis 1). The crypto suite is named by the format version, never negotiated per manifest (no downgrade surface).                                                                                                  |
| `provider`                | The provider identity (keypair identity in the ID object model, #5905 D1).                                                                                                                                                                                     |
| `instance`                | Instance discriminator: one identity may run several instances; grants and audit bind (provider, instance) as #5905's provider-instance binding requires.                                                                                                      |
| `name`                    | Human/registry name. A label, never identity: a familiar name under a different key is a different provider, and a registry name collision is a refusal, never a rebind (#5905 D3). Client providers live under the reserved `client.` prefix (parent F13.7).  |
| `side`                    | Who holds the resource (the parent's F13.7 addition to the F12.4 fields).                                                                                                                                                                                      |
| `kind`                    | Provider kind class (host request / subscription / platform module in current code; the kind class discovery cards expose — constrained-device / host / web / engine / phone — derives from it, #5899 tier B).                                                 |
| `manifest_version`        | Monotonic integer version of this manifest (D4, axis 2).                                                                                                                                                                                                       |
| `epoch`                   | Declaration epoch: monotonic per provider identity, incremented on re-declaration after stale/tombstone and on instance re-establishment (D4; required in the artefact by #5899 §3.1/§5.3 — discharged here, not in a sidecar envelope).                       |
| `scope`                   | Where this declaration is filed: world id / server id / federation intent (#5899 §3.1 — discharged here). Promotion between scopes is governed by #5899, not by re-filing a different manifest.                                                                |
| `ttl_class`               | Liveness class per #5899 §5 / #5904 presence classes; `static` is the built-in class (D1).                                                                                                                                                                     |
| `issued_at`, `expires_at` | Issue time and absolute expiry. Expiry is absolute and use never renews it (the #5904 §5 cached-grant rule, applied to the artefact it was waiting on).                                                                                                        |
| `capabilities`            | The capability entries below; the complete set for this version (D1, D5).                                                                                                                                                                                      |
| `extensions`              | Namespaced optional fields. An unknown **optional** extension is ignored; an unknown **required** extension is a refusal with a stable F11.13-class code. No extension may widen bounds, scope, class, or ceiling — extensions describe, they never authorise. |

**Per-capability entry**

| Field               | Content                                                                                                                                                     |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capability_id`     | Stable within the provider across versions; capability identity is the tuple (provider identity, `capability_id`, `manifest_version`) (#5905 D6).           |
| `contract_version`  | This capability's contract version (D4, axis 3).                                                                                                            |
| `argument_schema`   | Typed argument schema in WIT-shaped typed form (the form #5903 D2 keeps engine providers compatible with); free-form untyped schemas are not a declaration. |
| `bounds`            | The existing bound vocabulary (argument bytes, items, identifier bytes, response bytes, horizon, world limits, rates).                                      |
| `required_scope`    | Participant scope required for the effect (join / chat / chat-read in current code).                                                                        |
| `sensitivity_class` | S0–S3, a manifest fact (#5900).                                                                                                                             |
| `effect_class`      | Observation or actuation (parent: the two are different risk classes, always distinguishable from the declaration alone).                                   |
| `default_ceiling`   | Per-manifest ceiling posture inherited from the provider block's admission state; entries never enter above `excluded` by declaration alone (#5905 D2).     |
| `store_and_forward` | Whether actuation may be queued for later execution; honoured only when the grant also declares it (#5904). Absent = never.                                 |

**Prohibited contents.** A manifest never carries: credentials or secrets of any kind; grant objects or consumer identities (a declaration is an offer, not an authorisation — the audience is whoever the scope admits, per #5899's tiers); executable code (declarations are data; scripted providers per #5903 D2 declare surfaces, and their code arrives by its own channel); or transport addresses as identity (endpoint hints, where needed, are non-authoritative routing data and are never what a grant or an audit entry binds).

### D3 — Signing

- The manifest is signed by the provider identity key over the canonical encoding of the complete field set above (the signature field itself is excluded by the canonical form, in the ID object model's usual way). The manifest digest is the content address every other record cites: registry admission, discovery cards, terms records, grants.
- **Verification points** (fixing _what_ is verified where; #5905 D2 already fixed _that_ verification happens): (1) at admission — an unsigned, badly signed, or unverifiable manifest is not filed, and the refusal is a stable code, not a silent drop; (2) on every manifest change — a new version is a new signed object, verified independently of its predecessor; (3) at grant issuance — the grant binds the digest-verified tuple, so a manifest substituted after admission cannot inherit grants made against another.
- **Admission is not a countersignature.** The registry's admission act (admitted-by, ceiling posture, admitted-at) is a separate registry record referencing the manifest digest. The manifest remains single-author — its provider — which keeps provenance and blame exact: the provider answers for what it offered, the registry answers for what it admitted, and neither signature can be mistaken for the other.
- Transport authentication never substitutes for the signature (#5905 D2): a manifest arriving over an authenticated channel with a bad signature is refused; a correctly signed manifest arriving over a bridge is exactly as valid as a local one (bridges are transport, #5905 D5).

### D4 — Versioning: three axes, never conflated

1. **`format_version`** — the declaration format's own version. A reader refuses an unknown major format version with a stable refusal code; it never best-effort parses a format it does not know, because a partially understood declaration is a partially understood offer.
2. **`manifest_version`** — a monotonic integer per (provider, instance, epoch). Immutable once filed: any change to any signed field is a new `manifest_version`, never an edit in place. This is the version grants and terms records bind (#5900, #5899 §4.3). Semantic-version strings may be carried as display metadata in `extensions`; the binding is the integer.
3. **`contract_version`** — per capability. Major = breaking (schema or bounds semantics change incompatibly); minor = compatible narrowing or addition of optional arguments. Compatibility is the parent's rule: same major = compatible; a session advertises (name, version) pairs and is routed only to compatible versions (parent F13.7). Negotiation (#5899 §4) narrows within compatibility; it never manufactures it.

- **Rollback protection.** A registry or discovery view refuses a `manifest_version` lower than the highest seen for its (identity, instance, epoch), and refuses an epoch rollback absolutely. A provider that must return to earlier _contents_ files them as a new, higher `manifest_version` with the old contents — history moves forward only.
- **Superseded versions stay readable.** A superseded manifest version remains fetchable and verifiable until every grant bound to it has expired: audit must be able to reconstruct exactly what was offered when a grant was made (#5905 D4 — behaviour attributable from the journal alone).
- **Deprecation and removal.** A new version may mark a capability `deprecated` (still invocable, shown as deprecated in discovery) or omit it (removed). Removal takes effect immediately for new terms records and new grants. An invocation under a still-live grant against a removed capability fails with a stable `capability_removed` refusal and is journalled as such — removal is a provider's right over its own offer, exercised in the open, never by silently degrading the capability.

### D5 — Dynamic vs static: the set is static, availability is dynamic

This is the split the issue names, and the record settles it as a rule about _which layer changes_:

- **Static — the offer.** Within one `manifest_version`, the capability set, schemas, bounds, classes, and required scopes never change. There is no runtime self-modification and no capability appears without a new signed `manifest_version`. (The fully dynamic alternative — runtime registration in the style of a mutable `tools/list` — is rejected in §7: it makes the grant's tuple binding point at a moving target and makes audit reconstruction depend on provider goodwill.)
- **Dynamic — the liveness.** Everything about _whether_ a declared capability can currently be used is dynamic state held outside the signed manifest: presence and heartbeat per `ttl_class`, the #5899 lifecycle (live → suspect → stale → tombstoned), and suspension (e.g. an OS permission is revoked on a phone: the capability is suspended in the registry/discovery view at once per #5903 D3, the manifest is unchanged, and re-declaration on re-grant resumes the same `manifest_version` if its contents are unchanged).
- **Dynamic — which version is live.** A provider changes its offer at runtime by filing a new signed manifest version; admission re-verifies it and the discovery view updates atomically to the new version (there is never a moment where discovery shows a half-old, half-new offer). Existing grants against the old tuple are not migrated and are not revoked by the version change (the #5899 stale-rule analogue: a version change voids prior _terms records_, not grants); a provider may serve several manifest versions concurrently during a transition, and every invocation is checked against the tuple it cites.
- **Session advertisement is a reference, not a declaration.** A client session advertising `(name, version)` pairs (parent F13.7) is citing a filed manifest by digest; advertisement never creates, amends, or re-signs a declaration. Session-bound providers (engine, #5903 D4) file their declaration at session start with session scope and a finite `ttl_class` — the static-per-version rule holds unchanged inside the session.

### D6 — Declaration lifecycle

`filed → admitted → live → (suspended) → superseded / stale → tombstoned`, where:

- **filed** = signature-verified and stored, not yet admitted (D3); a filed declaration is discoverable by nobody.
- **admitted** = the registry's admission record exists (D3); the provider enters at `default_ceiling: excluded` (#5905 D2).
- **live** = within TTL/heartbeat and not suspended; only live declarations appear in discovery views (#5899).
- **suspended** = registry-side state (D5), contents unchanged.
- **superseded** = a higher `manifest_version` in the same epoch is live.
- **stale → tombstoned** = per #5899 §5's TTL semantics; re-declaration after tombstone starts a new `epoch` (D4).
- **Withdrawal** before expiry is itself a signed object naming (provider, instance, epoch, manifest_version): a withdrawal is an act of the provider identity, so neither a bridge nor a compromised registry can forge one (#5905 D5). A registry-side tombstone on expiry/withdrawal is bookkeeping over that act, not a substitute for it.

## 3. Invariants implementation must hold

1. **I1 — One artefact.** Every provider class declares through the same signed manifest shape; no per-target declaration format exists. (Parent one-model test.)
2. **I2 — No unsigned authority.** An unsigned or unverifiable manifest is never filed, discovered, negotiated against, or granted against.
3. **I3 — Immutability.** A filed manifest version never changes; amendment is a new version. Any store that can mutate a filed manifest is defective by definition.
4. **I4 — Tuple stability.** (provider identity, capability id, manifest version) names one immutable offer forever; audit can always fetch and verify the exact manifest a grant was issued against, until the grant's expiry.
5. **I5 — Declaration precedes discovery.** Anything discoverable traces to exactly one filed declaration record; there is no discovery state without a declaration under it (#5899 §3.1).
6. **I6 — Offers are not authority.** Holding, reading, or filing a declaration confers no invocation right; the ceiling and a grant decide, in that order, unchanged.
7. **I7 — No widening by declaration.** A new manifest version can offer more, but no ceiling, grant, or terms record widens as a side effect of a declaration change; consumers opt into a new version explicitly through #5899 negotiation and #5900 grants.
8. **I8 — Forward-only history.** Manifest versions and epochs never roll back; returning to earlier contents is a new version carrying those contents.
9. **I9 — Availability is not contents.** Liveness, suspension, and staleness are computed outside the signed manifest; no availability event ever edits a signed field.
10. **I10 — Names are not identity.** No security decision (admission, grant, routing, audit attribution) is ever made on `name` or on a transport address.

## 4. Alternatives rejected (summary)

- **Platform registry as the authoritative declaration** (AppFunctions / OS registries) — already rejected for the embedded face by #5903 D1 and binding here for all classes: the registry would be the platform's, its authority the platform's permissions, and `id`'s layer-4 authority would be replaced by the platform's (survey #5931 §6.7's standing warning). Projections from the manifest: yes; reverse-authoritative: never.
- **Unsigned or self-asserted declarations** (a provider's word over an authenticated channel) — channel authentication authenticates a connection, not an offer; it cannot survive a bridge, a cache, or an audit (#5905 D2/D5).
- **Fully dynamic capability registration** (mutable runtime `tools/list`-style sets under one version) — the grant tuple would bind a moving target; the static-set/dynamic-availability split (D5) keeps every dynamic property the use cases actually need.
- **Registry countersignature on the manifest** — conflates two different assertions (offer vs admission) into one artefact and makes either side's later revocation ambiguous; separate records citing one digest keep both exact (D3).
- **Semantic-version strings as the binding version** — string ordering and pre-release rules are a human convention, not a monotone; bindings use the integer `manifest_version` (D4).
- **Per-target manifest formats** (a phone manifest, an engine manifest, …) — precisely the special case the parent's one-model test exists to refuse (#5903 D1 for the embedded face; generalised here).
- **In-manifest endpoint/credential fields** — addresses are not identity (#5905 D3) and credentials in a widely replicated, audit-retained offer document are a leak by construction (D2 prohibited contents).

## 5. Open questions routed to the parent (#5686)

These cannot be settled inside this record without forking a parent decision; they are recorded here as explicit open questions for #5686, and nothing in D1–D6 depends on their answers:

1. **Key rotation and provider identity.** #5905 D1 fixes "a provider is its keypair". If an identity key rotates, is the rotated provider the _same_ provider (existing grants, epoch, and manifest lineage carry over, under a rotation proof) or a _new_ provider that must be re-admitted from `excluded`? This record's machinery works either way (rotation-as-new-provider is the conservative default until the parent rules), but the ruling belongs to the ID object model / #5686, not to the declaration layer.
2. **Federation-scope admission.** D2 files declarations with federation intent and #5905 D5 bounds what a bridge can do, but whether a declaration admitted in one administrative domain is _admissible_ in another on its home admission alone, or needs a local admission act per domain, is a cross-domain trust decision for #5686.

## 6. Conflicts check

- **#5899:** its §3.1 fallback ("if #5898 settles the artefact without `scope` / `ttl_class` / `epoch`, they ride in the discovery envelope") is not taken: all three fields are in the artefact (D2), so no envelope sidecar exists and there is exactly one signed source for them. Its stale-semantics (§5) and terms-record rules (§4) are consumed unchanged; D5's version-change rule is stated as their analogue, not an amendment.
- **#5900:** the field spellings its §6 assumed (`expires_at`, `bounds`, sensitivity class as a manifest fact) are discharged verbatim in D2; the grant object and its shapes are untouched.
- **#5903:** the projection rule and the two-gate phone composition are untouched; D5's suspension state is the registry-side mechanism its D3 revocation paragraph describes ("the declaration is updated, discovery shows the capability as suspended") — read here as a discovery/registry state change, since a signed manifest cannot be edited (I3/I9). If #5903 intended a re-filed manifest on suspension, that is a wording clarification for its record, not a behavioural difference: contents, version, and bindings are identical under either reading.
- **#5904:** cached-grant and presence semantics consume `ttl_class` / `expires_at` as fixed in D2; no #5904 rule is amended.
- **#5905:** D1–D6 of that record are the premises of §1 here, not re-decided; this record adds the artefact those premises quantify over. Provider-instance binding (its D1) is carried by the `instance` field.
- **Parent #5686:** the F12.4 field set (name, version, argument schema, bounds, required scope, default ceiling) plus `side` is carried entire in D2; the F13.7 session-advertisement and same-major routing rules are consumed in D4/D5. #5686 itself remains open; as with its other splits (#5900, #5903–#5905, all decided while the parent is open), this record decides only the layer its issue owns.

## 7. What this record does not do

No code, no wire encoding, and no `world_caps` changes are specified here beyond the field vocabulary of D2. The canonical-encoding choice inside the ID object model, the registry's dynamic-filing path (today's registry is static, D1), and the projection of compile-time manifests into D2 shape are implementation work, to be filed as follow-on issues from this record. Issue #5898 closes when this record is merged, per its acceptance.
