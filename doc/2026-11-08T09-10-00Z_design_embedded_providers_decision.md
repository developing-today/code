# Decision record: embedded providers — engine & smartphone instantiation of the #5686 provider model

- **Issue:** #5903 (design split 6 of #5686)
- **Decided:** 2026-11-08, queue task 12853 (repo-worker-code, lane code-g)
- **Status:** decided for the embedded face. Model-level mechanisms stay with their
  owning splits, all still open at decision time: declaration #5898, discovery &
  negotiation #5899, grants #5900, offline/degraded #5904, security model #5905.
  This record decides only what those splits do not own: how the one provider
  model is instantiated on embedded hosts (game engine, smartphone), how OS
  permissions compose with `id` grants there, and how session-bound lifecycles
  work there. Where this record touches a sibling's layer, it states an
  interface or a constraint, not a mechanism.
- **Inputs:** parent #5686 (the fixed model: signed, typed, versioned
  declarations offered into the ceiling model of #789; grants explicit,
  revocable, auditable; observation ≠ actuation) and the field survey #5931
  §2 (smartphone/Android), §4 (Bevy/engine), §6 (cross-cutting layers), §7
  (flags). The survey makes no recommendations; the verdicts below are this
  record's, argued from the survey's evidence.

## The decision in one paragraph

On both embedded hosts the authoritative declaration is `id`'s own signed,
versioned provider manifest (the #5898 / #1061 writer-set artefact) — never a
platform registry. Platform surfaces (Android's AppFunctions registry, a Bevy
engine's ECS registration) are **projections** of that manifest: generated from
it, regenerable, and never reverse-authoritative. Authority is checked at the
writer-set layer on every invocation; transports (Binder, in-process ECS
calls, an outbound phone relay) carry no authority. On the phone, an `id` grant
and an Android runtime permission are **two gates, both of which must pass**,
with intersection semantics. In the engine, providers are session-bound:
capabilities exist only while the world/session runs, and grants bind to the
session instance by default.

## D1 — One declaration, per-host projections (both hosts)

- The manifest is the sole declaration artefact for embedded providers, in the
  same shape as every other provider class. An embedded-specific declaration
  format is rejected: #5686's test is that the model needs no per-target
  special case, and a second format is exactly that special case.
- **Smartphone projection:** capabilities may additionally be registered with
  Android AppFunctions so OS-mediated callers (system agents) can discover and
  invoke them. Rejected as the *authoritative* registry: at survey time
  AppFunctions is beta/alpha with a private-preview caller set (#5931 §2.1,
  §7), the registry is the OS's, and its authority is Android permissions, not
  owner-controlled signed objects — adopting it as the model would replace
  `id`'s layer-4 authority with Google's (#5931 §6.7's standing warning).
  Also rejected: ignoring AppFunctions entirely — it is the first OS-native
  declaration+discovery surface on a phone, and the projection costs little
  because it is generated. Rule: the projection is derived from the manifest;
  drift is resolved by regenerating the projection, never by importing the
  registry's state back into the manifest.
- **Engine projection:** provider surfaces (render, input, audio, entity data)
  register into the running engine (components/resources in Bevy's ECS) from
  the same manifest. The engine-specific part is confined to *how surfaces are
  produced*, never to what is declared (#5931 §4, decider question 1 —
  answered: yes, same manifest).

## D2 — Engine provider shape: native at the boundary, scripted only inside it

- Built-in provider surfaces are **native Bevy plugins** (the engine's own
  plugin architecture): compile-time, full reflection, enforceable per-tick
  budgets.
- **Scripted providers** (Lua/Rhai via the bevy_mod_scripting pattern) are
  permitted only for session-scoped, world/user content, and only through a
  host-curated API surface — the embedded-scripting pattern of #5931 §6.5:
  scripts receive exactly the host functions the hosting plugin exposes, and
  never an actuation surface beyond that exposure. Rejected as the general
  provider mechanism: weaker isolation than the native boundary, a
  self-described work-in-progress bridge with version-locked churn, and Rune
  support on hold in the rewrite (#5931 §4.2, §7).
- **Wasm components inside the engine are deferred, not rejected.** The
  component model is the cross-cutting provider-component question owned by
  #5898/#1061 (#5931 §6.2); this record keeps the engine face compatible with
  it (provider interfaces stay in a WIT-shaped, typed form) without making
  tick-loop performance depend on an unproven in-engine component runtime.
- **Tick discipline:** invocations are marshalled onto the ECS schedule; no
  provider call blocks the tick thread. Rate/size limits in the manifest are
  interpreted on this host as per-tick budgets and enforced by the host
  plugin, not by provider goodwill.
- **Replication boundary:** `id`'s journal (writer-set) carries shared-world
  authority. Engine replication (replicon/lightyear class, #5931 §4.3)
  replicates ephemeral engine state only; it is a transport, and a replicated
  engine state change is never by itself an authorised journal write. The two
  systems answer different authority questions and must not be conflated
  (#5931 §4, decider question 2 — answered for the provider face this way).
  Bevy version pin/track/abstract policy stays with #5781.

## D3 — Smartphone: OS permissions compose with `id` grants as two gates

This is the composition #5686's smartphone class names ("two gates, both must
pass"); this record fixes the semantics:

1. **Gate order and independence.** Gate 1 is the `id` grant (#5900): scoped to
   capability, provider, consumer, purpose, rate/volume, expiry. Gate 2 is the
   Android runtime permission, held by the host app. Neither gate's pass
   implies the other's: the app holding a permission does not authorise a
   consumer, and an `id` grant does not create an OS permission (the app must
   still request it at runtime, per-use/one-time where the platform offers
   that shape).
2. **Intersection semantics.** Effective authority for an invocation is the
   intersection of the two gates. A denial at either gate is final for that
   invocation and is reported as a denial naming the gate — never silently
   retried through the other gate.
3. **Revocation is immediate.** If the OS/user revokes (or the platform
   auto-resets) a permission, every capability depending on it is suspended at
   once: the declaration is updated, discovery shows the capability as
   suspended (not merely failing at call time), and outstanding grants against
   it are inert until the permission is re-granted *and* the capability is
   re-declared. Rejected: install-time / app-level consent as a grant
   substitute — its granularity is the app, not the consumer or the grant
   (#5931 §2.2). Rejected: building the provider on accessibility services —
   it inherits a platform-policy fight and bypasses the permission model this
   composition exists to respect (#5931 §2.2).
4. **Sensitivity scaling is inherited, not re-invented:** observation vs
   actuation remain different risk classes (#5686, #5900). On this host that
   means actuation-class capabilities default to per-use approval on the `id`
   side *and* a foreground, user-visible context on the OS side; standing
   grants are for low-sensitivity observation only.

## D4 — Session-bound lifecycles (engine first, phone by the same rule)

- A declaration carries a **lifetime scope**. Engine providers are
  session-scoped by default: the capability exists only while its world/
  session runs. Phone providers are app-state-scoped: alive in the foreground,
  and in the background only inside the declared WorkManager / foreground-
  service envelope (#5931 §2.3) — a provider never promises availability
  beyond the OS battery envelope, and its declared rates are battery-aware
  caps, enforced by the host app.
- **Death is explicit.** When a session ends or the app leaves its declared
  envelope, the provider tombstones its discovery entries (#5686 stale-
  discovery rule); consumers see provider-gone, not a hang.
- **Grants bind to the instance.** A grant names (provider identity,
  capability, session instance). Session death makes grants against that
  instance dormant; they do not transfer automatically to a later session of
  the "same" provider. A standing grant may opt in to renewal against a new
  instance of the same provider identity, but the default is no renewal.
  (This answers #5686's open question — do grants outlive a session-bound
  provider? — for the embedded classes: no, not by default.)
- **What wakes a phone provider** (push vs WorkManager windows) is #5904's
  mechanism to settle; the embedded constraint recorded here is that wakeups
  can only restore the provider inside its declared OS envelope, and a wakeup
  never widens a grant or a permission.

## D5 — Identity binding and transports on embedded hosts

- **Smartphone:** the provider identity is a **device-bound delegated
  identity** under the user who owns the phone: the device is the provider,
  the user is its ceiling owner, and the delegation is revocable by the user.
  (Answers #5686's identity question for this class; a $5 sensor's answer is
  #5901's, not this record's.)
- **Engine:** the provider identity is a **session-instance identity**
  delegated from the hosting user's (or world host's) identity, created at
  session start and retired at session end — matching D4's instance binding.
- **Invocation transport carries no authority** (#5931 §6.1, §6.7): on-device
  phone calls go over Binder (via AppFunctionManager for projected functions,
  or the client's own IPC); off-device reachability uses the **outbound-relay
  pattern** (the phone initiates; no inbound listening — the Phone MCP
  precedent, #5931 §2.1), because carrier/NAT reality makes direct inbound
  reachability the exception. Engine calls are in-process. In all three, the
  grant check happens at the writer-set layer on every invocation; a
  transport-level credential (an OAuth token, a Binder caller identity, an
  ECS-internal call) identifies a principal but is never itself the grant.

## Invariants implementation must hold

1. **Manifest sole authority.** Platform registries (AppFunctions, ECS
   registration) are projections: generated from the manifest, regenerable,
   never a source of authority or of declaration state.
2. **Two-gate composition on the phone.** `id` grant ∩ OS permission;
   either denial is final and named; OS revocation suspends the capability
   and inerts its grants immediately.
3. **Ceiling composition.** No embedded mechanism raises a ceiling: a
   provider offers at most its delegated identity's ceiling (#789 via #5686),
   a projection can only ever expose a subset of the manifest, and the
   manifest only ever exposes a subset of the ceiling.
4. **Instance-bound grants.** Grants scope to (provider, capability, session
   instance) on session-bound hosts — which is also the capability-confusion
   defence for this class: a grant cannot be replayed against a later session
   or a re-registered projection (the #5905 audit should verify this claim).
5. **Lifecycle honesty.** Declared lifetime scope and OS/tick/battery
   envelopes are part of the declaration; death tombstones discovery;
   availability is never promised beyond the envelope.
6. **Data uniformity and provenance.** Data emitted by an embedded provider
   lands in the same model as any other class, with provenance recording at
   minimum provider identity, capability, the grant it was obtained under,
   session instance, and time (#5686 cross-cutting).
7. **Actuation is never ambient.** On either host, actuation-class
   capabilities need the per-use approval flow of D3(4); scripts (D2) and
   background phone work (D4) cannot hold actuation beyond an explicitly
   granted, host-curated surface.

## Conflicts with the parent (#5686) — explicit check

- **No parent decision is changed.** #5686 fixes the model (declaration,
  discovery, negotiation, grants, ceiling composition) and leaves the
  embedded instantiation open as decomposition item 6; this record is that
  item, decided inside the parent's constraints.
- Two of the parent's *open questions* are answered **for the embedded
  classes only** (grants outliving session-bound providers → D4; provider
  identity for a phone → D5). The general answers remain the parent's and
  #5900's to state.
- One position is flagged as **input to #5899, not a parent fork**: embedded
  negotiation takes transport profiles (in-process negotiation is the
  degenerate single-step case of the same shape; Binder/relay negotiation is
  multi-step). If #5899 settles on a single transport-less protocol, the
  embedded faces conform — the profile claim here is about message
  pattern, not about a second negotiation model.
- Sibling splits were all open with no recorded decisions at decision time
  (verified 2026-11-08: #5898, #5899, #5900, #5901, #5902, #5904, #5905,
  #5780, #5781), so this record forks no sibling decision; where layers
  touch, it defers as noted.

## Splits and follow-ons

- **No further splits filed.** The design fits in this one record; the
  remaining embedded work is implementation-facing and already owned:
  the Android client's provider face by #5780, the Bevy incorporation's
  provider face by #5781, each consuming this record.
- iOS is out of scope: the filed class is Android (#5780), and the survey
  made no iOS pass (#5931 §8). If an iOS client is ever filed, this record's
  two-gate and projection rules are the starting hypothesis, not a decision
  about that platform.

## Alternatives rejected — summary

| Alternative | Why not |
|---|---|
| AppFunctions as authoritative registry | Beta/alpha + gated caller set; authority would be Google's, not the owner's (#5931 §2.1, §6.7) |
| Ignore AppFunctions entirely | Loses the OS-native discovery/invocation surface the projection gets almost free |
| Embedded-specific declaration format | A per-target special case — the failure #5686's one-model test exists to catch |
| Scripts as the general engine provider mechanism | Weak isolation, bridge churn, Rune on hold (#5931 §4.2, §7) |
| Engine replication as world authority | Replicates engine state, not journal authority; different question (#5931 §4.3) |
| OS permission as the grant | App-level granularity; no per-consumer, per-purpose, revocable-by-owner semantics (#5931 §2.2) |
| Provider on accessibility services | Platform-policy fight; bypasses the permission model (D3) |
| Grants that silently follow a provider across sessions | Breaks instance binding; replays authority into a context nobody granted (D4) |
| Inbound-reachable phone provider | Carrier/NAT reality; outbound relay is the proven pattern (#5931 §2.1) |
