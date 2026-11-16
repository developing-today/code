# Decision: provider security model — identity, admission, impersonation, rogue providers, bridge compromise, capability confusion, ceiling-composition audit

- **Date:** 2026-11-09
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5905](https://github.com/developing-today/code/issues/5905), "id: design: provider security model (from #5686)". Queue task 14150.
- **Parent design:** [#5686](https://github.com/developing-today/code/issues/5686) — the single capability-provider model, in-tree record `doc/2026-10-25T05-37-57Z_design_client_capability_providers/` (F13.7: signed, typed, versioned declarations; the ceiling model of #789; two-key opt-in for client providers; stable F11.13 refusal codes). This record does not re-decide the parent; §1 lists what it takes as given, and the conflicts check at the end is explicit.
- **Consumed, not re-decided:** [#5900](https://github.com/developing-today/code/issues/5900) grants decision (`doc/2026-11-09T00-16-56Z_decision_id5900_provider_capability_grants.md`) — grant shape, sensitivity classes S0–S3, attenuation, and the §5 ceiling-composition invariants; its §6 expressly leaves the threat model to this record. [#5903](https://github.com/developing-today/code/issues/5903) embedded decision (two-gate composition; session-bound engine instantiation; device-bound delegated phone identity; invocation transport carries no authority). [#5904](https://github.com/developing-today/code/issues/5904) offline decision (presence classes, buffering, §5 cached-grant expiry semantics). The attenuated-capabilities decision (`doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/`) and the ceilings record (`doc/2026-10-16T11-00-00Z_feature_world_capability_ceilings/`).
- **Inputs:** the capability-token field survey [#5931](https://github.com/developing-today/code/issues/5931) as cited by #5900 §6.4/D1; the parent record's own security analysis (authority creep, confused deputy, malicious host/guest, replay, forged replies — written for the F13.7 client-provider slice; this record generalises it to every provider class and settles the questions it left open).
- **Ceiling-before-grant:** the existing server/world ceiling-before-grant behaviour is **preserved unchanged**; this record records no amendment to it (task 14150 acceptance).

## Decision summary

| Question             | Decision                                                                                                                                                                                                                                                                                      |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Provider identity    | A provider **is** its keypair in the ID object model. A provider instance is (provider identity, instance, manifest version) — the binding #5900 D1 already grants against (D1)                                                                                                               |
| Authenticity         | Manifests are signed by the provider identity and verified at admission and on every manifest change; transport authentication never substitutes for the signature (D2)                                                                                                                       |
| Admission            | Explicit registry action by the world/server admin only. Every provider class enters at `default_ceiling: excluded`; nobody is routable, let alone granted, by discovery alone (D2)                                                                                                           |
| Impersonation        | Addressing and grants bind keys, never names or transport addresses. A familiar name under a different key is a different provider; a registry name collision is a refusal, never a rebind (D3)                                                                                               |
| Rogue provider       | An admitted-but-malicious provider's blast radius is its ceiling ∩ grant intersection, its bounds, and its classes — nothing ambient. Removal paths: ceiling removal, registry removal, grant revocation, in that order of breadth. Its behaviour is attributable from the journal alone (D4) |
| Bridge compromise    | A bridge/broker is transport, never a principal: it holds no grants, cannot mint or alter signed objects, and its compromise yields drop/delay/replay only — bounded by signatures, nonces, and expiry (D5)                                                                                   |
| Capability confusion | A capability's identity is the tuple (provider identity, capability id, manifest version). Grants name exactly one such tuple; same-named capabilities of different providers are different capabilities (D6)                                                                                 |
| Ceiling composition  | Effective authority is always the #5900 §5 three-way intersection, and it must be **enumerable**: registry + grant ledger + journal suffice to list and audit it, with named audit flags (D7)                                                                                                 |

## 1. What is already fixed (not re-decided here)

1. One provider model for every target class (host, client, engine, smartphone, ESP32/MQTT, terminal, web); a rule needing a per-target special case is wrong, not the table (#5686).
2. Identity never grants authority. A grant names identities for binding and audit; possession plus validity of the grant object is the authority (#5900 §1.5). This record's identity decisions are therefore about _binding and authenticity_, never about authority flowing from identity.
3. Effective authority at any use = grant scope ∩ provider ceiling ∩ consumer ceiling, checked at use, and no layer (OS permission, registry state, session presence, transport authentication) may add to the intersection — layers only subtract or pass through (#5900 §5, invariants 1–3).
4. Actuation is never ambient; classes S0–S3 and their approval flows are manifest facts (#5900 D2/D3). Grants attenuate only; revocation is by ledger append; every issuance, delegation, revocation, approval, and actuation use is journaled with grant id and lineage (#5900 D4).
5. Client providers carry the parent's two-key rule (world grant **and** the client's own per-world opt-in, checked on every request) and default to `default_ceiling: excluded` (F13.7). #5903's two-gate phone composition and session-bound engine instantiation, and #5904's offline semantics (cached snapshots authorise observation only, never actuation; expiry is absolute), stand as decided.

## D1 — Provider identity

- A provider identity is a keypair in ID's native object model — the same signing and writer-set machinery as provider manifests and #5900 grant objects. There is no second identity system for providers.
- The unit everything binds to is the **provider instance**: (provider identity, instance, manifest version). #5900 D1 already binds grants to exactly this; this record makes it the universal referent — registry entries, ceilings, audit entries, and refusals all name instances, not bare names.
- Per-class instantiation is as already decided: smartphone providers are device-bound delegated identities (#5903 D5); engine providers are session-instance identities (#5903 D5); host and client providers are the identities named in their manifests (F13.7). A class may add binding facts (device, session); it may not weaken the rule that the key is the identity.
- Human-readable provider names (including the reserved `client.` prefix space) are **display and routing labels only**. A name is a claim any key can make; it resolves to an instance only through the registry entry an admin admitted (D2). No security decision — admission, grant, ceiling, audit — may key on a name alone.
- **Key rotation and loss.** Rotation is an explicit, journaled re-binding: the old identity signs the successor, or an admin re-admits the new key as a new instance. Grants never transfer silently across a rotation — they are re-issued or explicitly re-bound in the journal. A lost key with no rotation path is a dead provider: its instance is removed from the registry and its grants die with the binding. (Recovery UX is an open question, below; the security default is not negotiable — no recovery path may let a name alone resurrect an identity.)

## D2 — Authenticity and admission

- A provider manifest is a signed object. The registry verifies the manifest signature against the provider identity **at admission and on every manifest change**; an unsigned or wrongly-signed manifest is not routable, with a stable F11.13 refusal.
- **Admission is an admin act.** A provider enters a world's registry only when the world/server admin admits its (identity, manifest) pair. Discovery — hearing a manifest announced, resolving a name, completing a transport handshake — admits nothing and grants nothing. There is no auto-admission, no first-come name registration, and no trust-on-first-use path.
- Every provider of every class enters at **`default_ceiling: excluded`** (the F13.7 client rule, generalised): admission makes a provider _nameable_ in a ceiling, never _included_ in one. Inclusion is a separate, journaled ceiling act.
- **Manifest changes re-run admission for what changed.** A new manifest version whose capability set, classes, or bounds exceed the admitted version is a new admission decision for the delta; grants bound to the superseded version refuse at use with a stable code (#5900 D1 version binding). A manifest change may narrow freely; widening is never inherited silently.
- Authenticity of _responses_ follows the parent's rule, generalised: a reply is accepted only for the session and nonce it answers, within bounds, from the instance the request was routed to; anything else is dropped and audited, and the consumer sees `provider_unavailable`, never forged data (F13.7 forged/stale-reply rule).

## D3 — Impersonation

- Consumers and worlds address providers by instance (D1), never by name or transport address. A transport address (an MQTT topic, a socket, a URL, a session) is routing information; the same name or address presenting a different key is **a different provider**, full stop.
- A registry name already admitted under one key presented under another key is a **collision refusal**, journaled with both identities — never a rebind, never first-wins-by-recency, never a silent shadow entry. Display layers must surface the collision rather than pick one.
- Session and transport authentication (pairing, OS identity, TLS, a signed webhook) are subtract-only gates under §1.3: they can refuse a use, they cannot make an impersonator into the provider, and they cannot substitute for manifest/grant signatures (#5903 D5: invocation transport carries no authority).
- A provider may not sign with one identity and claim another's instance in any field; verifiers check every signed object's signer against the instance it names, and a mismatch is a refusal with a stable code.

## D4 — Rogue providers

A rogue provider is an admitted provider that acts against its consumers' interests — with a valid key, inside or probing at the edge of its admitted surface. The model assumes this case rather than treating admission as trust:

- **Blast radius is structural.** A rogue provider can exercise only capabilities in its ceiling ∩ consumers' grant intersection, within grant bounds and class approvals (§1.3, #5900 D2/D3). It holds no ambient authority: no session, pairing, or presence gives it anything. Its S3 capabilities still need per-use approval; its actuation uses are journaled as they happen.
- **Observation dishonesty is the honest residual risk.** A rogue _observation_ provider can return false data within its bounds; signatures prove origin, not truth. This record does not pretend otherwise: mitigations are consumer-side (cross-checking independent providers, plausibility bounds in the manifest's units) and are policy, not protocol. What the protocol guarantees is attribution — false data is provably _from that instance_ — and containment: observation results enter the journal as that provider's claims, and a consumer's `update` remains the only path by which they affect world state (parent rule).
- **Removal, in order of breadth.** (1) Ceiling removal: takes effect across every outstanding grant at once, no re-issuance (#5900 §5.1). (2) Registry removal: the instance stops being routable; in-flight uses refuse at their next check. (3) Grant revocation: per-grant or per-subtree, for surgical cases (a single consumer relationship). All three are journaled admin acts with stable codes; none requires the provider's cooperation, connectivity, or key.
- **Detection surface.** The #5900 audit invariants are the detection substrate: for any actuation, the journal answers under which grant, approved by whom, within which bounds — and, by this record, _by which instance_. Rate/bounds refusals by a provider's own consumers are journaled too, so probing behaviour is visible rather than silent.

## D5 — Bridge compromise (ESP32/MQTT and relayed classes)

- A bridge, broker, or relay (the MQTT broker and ESP32 bridge of the parent classes, any future store-and-forward hop) is **transport, never a principal**. It holds no grants, appears in no ceiling, and cannot mint, alter, or re-sign manifests, grants, or journal entries: every authority-bearing object is signed end-to-end between its issuer and its verifier, and a bridge that modifies one produces a signature failure, not a modified fact.
- A compromised bridge can therefore do exactly three things: **drop, delay, replay**. Drop and delay are availability attacks, answered by #5904 presence semantics (a silent provider degrades to its presence class; nothing is presumed delivered). Replay is bounded by the D2 response rule and per-request nonces/expiry: a replayed request or reply is dropped and audited, and replay never causes a new actuation (parent rule: replay never contacts a client; generalised — replay never re-executes).
- Bridge credentials (broker passwords, topic ACLs, tunnel keys) are transport hygiene only. Their compromise is an operational incident (rotate them), never an authority breach; any design that would make it one violates §1.3 and is wrong at review, not at incident time.
- Store-and-forward exists only where #5904 allows it: declared by both manifest and grant, observation-class, and never a path by which a bridge queues actuation for later execution.

## D6 — Capability confusion

- A capability's identity is the tuple **(provider identity, capability id, manifest version)**. Capability ids are namespaced by provider (the parent's reserved-prefix rule is one instance); the same id under two providers, or under two manifest versions of one provider, names _different capabilities_.
- Grants already name exactly one capability of one instance (#5900 D1); this record adds the consumer-side rule: no interface — CLI, UI, negotiation, or discovery listing — may present a capability by bare id where two admitted providers offer the same id. Presentation groups by provider instance, and negotiation (#5899's scope) resolves names to tuples before any grant is requested, never at use time.
- A manifest that renames, re-classes, or re-bounds a capability has created a new tuple: existing grants do not follow the rename. This is D2's no-silent-widening rule at capability granularity, and it closes the confusion where a familiar capability name returns with a higher class under an old grant.

## D7 — Ceiling-composition audit

The ceiling model is only a security property if the composed result can be inspected. Decisions:

- **Enumerability invariant.** For any world, the effective provider authority — every (instance, capability, class) in the three-way intersection, the standing grants against it, and the approval flow each class implies — must be computable from three sources alone: the registry, the grant ledger (including `world_caps` ceilings), and the journal. If a fact affecting effective authority lives anywhere else (a config file, a transport ACL, an OS permission store), it is either brought into the journal or it does not affect authority (§1.3 already forbids the latter; this invariant makes it auditable).
- **Standing audit flags.** An audit enumeration must flag, at minimum: any standing actuation grant; any S3 capability present in any ceiling; any grant bound to a superseded manifest version; any admitted provider not at `default_ceiling: excluded` that no ceiling act journaled; any name collision refusal in the journal (D3) not yet resolved by an admin. Flags are findings for a human or a follow-on task, never auto-remediations.
- **Ceiling changes are journaled as compositions, not just edits.** A ceiling act records the resulting delta to effective authority (which instances/capabilities entered or left the intersection), so the audit trail reads as a history of _authority_, not of configuration.
- **Cadence is operational, not protocol.** This record requires the enumeration to exist and be runnable on demand (filed as #7944); how often it runs is a deployment decision. What could reopen this: evidence that on-demand audit misses a class of drift — e.g. transport-level state that demonstrably changed effective authority — would falsify the enumerability invariant and force the missing source into the journal.

## Invariants implementation must hold

1. No provider is routable before an admin admits its signed (identity, manifest) pair; admission defaults every class to `default_ceiling: excluded`.
2. Every authority-bearing object (manifest, grant, revocation, ceiling act) verifies against the instance it names; a signer/instance mismatch is a refusal with a stable F11.13 code, journaled.
3. Names and transport addresses never resolve to authority; collisions refuse, they do not rebind.
4. Effective authority at any use is the #5900 §5 three-way intersection, computed from current registry, ceiling, and ledger state — never cached past the manifest cache TTL, and never widened by any transport, session, OS, or bridge state.
5. A bridge or broker holds no grants and appears in no ceiling; its full compromise yields drop/delay/replay only, and replay never re-executes.
6. Registry removal, ceiling removal, and grant revocation each take effect without the provider's cooperation, and are journaled as authority deltas.
7. For any actuation, the journal alone answers: which instance, under which grant, approved by whom, within which bounds (extends #5900 D4(c) with the instance binding of D1).
8. The D7 enumeration runs from registry + ledger + journal alone; any audit flag it raises is a filed finding, not a silent log line.

## Conflicts with the parent and siblings — explicit check

- **No parent decision is changed.** #5686's model, #789's ceilings, #5900's grant shape/classes/composition invariants, #5903's embedded instantiations, and #5904's offline semantics are all taken as given (§1); every decision above either generalises a rule one of them stated for a single class (default-excluded admission, forged-reply refusal, transport-carries-no-authority) or settles a question they expressly left open (#5900 §6 leaves the threat model here; the parent's security analysis was scoped to the F13.7 slice). Server/world ceiling-before-grant behaviour is preserved verbatim.
- Nothing here re-decides #5931 (a survey, consumed as input) or the declaration/discovery/negotiation scopes of #5898/#5899 — D6 constrains what negotiation must resolve names _to_, and leaves how it does so to #5899.

## Alternatives rejected (summary)

- **External PKI / CA-issued provider identity** — a second identity stack and an external trust root for a system whose object model already signs everything; admission (D2) is the trust act, and a CA would move it, not remove it.
- **Name- or address-based provider addressing** (topic, URL, or display name as identity) — makes impersonation a configuration accident instead of a cryptographic impossibility (D3).
- **Transport authentication as provider authenticity** (a valid TLS client cert or broker login _is_ the provider) — violates the subtract-only layer rule; bridge compromise would become authority compromise (D5).
- **Auto-admission on discovery with reputation scoring** — discovery is unauthenticated by definition here; reputation is un-auditable authority, and the enumerability invariant (D7) could not hold.
- **Shared bridge credentials as a trust boundary** — one secret whose leak is every bridged provider at once; D5 makes bridge secrets operational only.
- **Treating rogue-provider behaviour as out of scope** ("admitted means trusted") — the parent's malicious-host/guest analysis already assumes hostile admitted parties for one class; D4 generalises the assumption rather than designing for a world without it.

## Unresolved, mechanism-dependent questions

- **Key recovery UX.** D1 fixes the security default (no name-only resurrection); the human flow for "the device is gone, its provider must come back" — admin re-admission ceremony, successor linking in the journal — is follow-on design, expected to ride on #7944's implementation findings.
- **Hardware attestation for the ESP32 class.** A provider key in extractable flash is cloneable, and a clone _is_ the identity under D1. Whether the ESP32 class requires secure-element key storage is a hardware decision this record flags but does not take; until it is taken, ESP32-class instances should be ceilinged as if cloneable (bounds and classes set accordingly).
- **Cross-world reputation.** Whether one world's rogue-provider findings should inform another world's admission is deliberately not decided: worlds are sovereign over their registries, and sharing findings is a publication question, not a protocol one.

## What this record does not do

No code, manifest schema, or registry implementation is specified here beyond the vocabulary above. Implementation is filed as [#7944](https://github.com/developing-today/code/issues/7944) (admission signature verification, default-excluded admission for all classes, manifest-change re-admission, the D7 audit enumeration). #5905 stays open until this record is merged, per its acceptance.
