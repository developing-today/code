# Design: a native Android client for ID

- **Date:** 2026-11-08
- **Status:** decision-ready proposal; platform-specific key-storage and Rust/Android binding details have explicit implementation spikes below.
- **Issue:** [#5780 — native Android client](https://github.com/developing-today/code/issues/5780)
- **Baseline:** `origin/main` at `af4e603e896a92bb3ddfbacb9d1089d83571f401`.
- **Consumes:** [#1831](https://github.com/developing-today/code/issues/1831) and its input-event design; [#5684](https://github.com/developing-today/code/issues/5684) identity custody; [#5686](https://github.com/developing-today/code/issues/5686) provider model; [#5931](https://github.com/developing-today/code/issues/5931) §2 smartphone survey; [#5903](https://github.com/developing-today/code/issues/5903) embedded-provider decisions; [#5904](https://github.com/developing-today/code/issues/5904) offline/degraded rules; [#788](https://github.com/developing-today/code/issues/788) shell contract/native reference shell; [#790](https://github.com/developing-today/code/issues/790) client-only boundary; [#3447](https://github.com/developing-today/code/issues/3447) crate split; [#5671](https://github.com/developing-today/code/issues/5671) transport fallback.

## Decision in one paragraph

Build a **thick, local-first Android client**: the phone owns its local journal and device state, can read cached worlds and continue local work without a network, and synchronizes through ID's existing journal/sync seams when connected. It remains a **client-only** process—no world listener or host starts implicitly. Use a native **Kotlin/Jetpack Compose** shell around a Rust client core; the Rust core owns identity operations, journal rules, sync, and the shared shell/provider protocols. Treat the phone as a device-bound delegated provider: an ID grant and the Android runtime permission are independent gates and both must allow an invocation. Android platform APIs are adapters/projections, never the source of ID authority. Presence is honest and foreground-first: Android cannot promise always-on peer presence or immediate wakeups without a reachable push service, so v1 does not promise either.

This is a design for the product and its boundaries, not an implementation. #1831 remains the remote-drive feature and input-model precursor; its event vocabulary is reused through the versioned shell contract, not forked into a second protocol.

## Decisions at a glance

| Question | Decision |
|---|---|
| Thin, thick, or per-world client? | **Thick/local-first client.** No thin/thick selector in v1; local-only, cached, connected, and synchronized are lifecycle states of the same world model, not different client products. |
| Play, sideload, or both? | **Both**, from one source and package identity when signing permits. Start with signed sideload artifacts for the personal fleet; publish the same release lineage through Play when its signing and policy setup is ready. Both update paths use Android's package installer/Play; no silent self-installer. |
| What survives from #1831? | Its accepted input-event model survives as the **Android producer mapping** for remote-drive mode. The shell contract owns framing, versioning, refusal behavior, and conformance; no Android-only wire protocol. |
| What wakes the app without a central server? | **No guaranteed event wake.** Foreground activity syncs immediately; opt-in WorkManager jobs are best-effort. No always-on socket, false `online` presence, or required central push service in v1. |
| Android UI vs shared Rust UI? | **Kotlin/Compose** for Android UI and platform integrations; Rust remains the shared domain/protocol core. #788's Bevy recommendation is for a separate native reference shell, not an Android product UI. |

## 1. Context and constraints

### 1.1 What already exists

- #1831's input-event design defines a common pointer model, Android gesture mapping, explicit refusals, and the remote-drive boundary. It is a producer/interface design; its later manifest/shell-contract slice must land before an Android remote-drive client can emit those frames. See [`doc/2026-11-08T05-48-47Z_design_input_event_model/2026-11-08T05-48-47Z_design_input_event_model.md`](../2026-11-08T05-48-47Z_design_input_event_model/2026-11-08T05-48-47Z_design_input_event_model.md).
- #788 selects Bevy as the reference **native shell** and says shells consume the versioned client↔host contract; it separately excludes server startup and keeps Bevy out of the default `id` build. That record does not select an Android app framework. Its current `id-native-shell` package depends on `id` with `default-features = false, features = ["world"]`; it is a useful contract precedent, not a ready-made Android client.
- #790 establishes the client-only boundary: only an explicit `id serve` hosts a world. An Android client does not start a server merely because it has local data or is serving a provider.
- #3447 has already made `pkgs/id` a workspace with `id-core` and `id-native-shell`, but the root `id` package still owns the broad facade and default `sandbox`/`world` feature set. The Android product needs a narrower dependency surface than `id` with `world` enabled; the crate split is the seam to create it.
- #5686 fixes the uniform signed, typed, versioned provider model, ceiling composition, explicit grants, and the observation/actuation distinction. #5903 D3 and D5 apply that model to phones: ID grants and Android permissions are two gates; the phone provider identity is device-bound and delegated under its owner. #5904 treats a smartphone as session-bound: when the app is gone, the provider is unavailable, not an indefinitely buffering bridge.
- The #5931 survey §2 (surveyed 2026-11-04) inventories AppFunctions / “Android MCP,” classic intents and permissions, Phone MCP, WorkManager/foreground services, Termux, and on-device agent runtimes. It explicitly makes no recommendation. AppFunctions was described there as beta/alpha with a private-preview caller set; this design therefore does not make it authoritative or a v1 dependency.

### 1.2 System shape

```text
Kotlin / Jetpack Compose app
  ├─ Android adapters: Keystore, permissions, lifecycle, notifications,
  │  content picker, WorkManager
  └─ narrow JNI/FFI boundary
       └─ Rust ID client core
            ├─ device identity + provider manifest/grant checks
            ├─ local world journal + materialized views
            ├─ shell-contract session client / remote-drive producer
            └─ existing event-sync and transport abstractions
                 ├─ local app-private storage
                 └─ remote ID worlds/peers (transport fallback is #5671)
```

No component in this diagram starts a world host implicitly. Android lifecycle adapters may pause or resume client work; they do not change world authority, grant scope, or journal semantics.

## 2. Identity custody on-device

### 2.1 Identity and key storage

The ID identity remains an Ed25519 identity under the custody rules being designed by #5684. Android biometrics are an **unlock convenience**, not an identity, a recovery factor, or proof of a remote grant. The phone's provider identity is a device-bound delegated identity under the user's ceiling, following #5903 D5; each enrolled device is independently revocable and its delegation never raises the user's ceiling.

The Android implementation must not assume that every Android Keystore/StrongBox combination can generate and use ID's Ed25519 key directly. The design boundary is:

1. Prefer a hardware-backed, non-exportable signing key only on devices where the supported algorithm and API level are verified by a real-device spike.
2. Otherwise keep the ID key in an encrypted key envelope in app-private storage, with a Keystore-generated AES wrapping key (request StrongBox when present; otherwise use the platform Keystore and report the actual security level). Decrypt only for signing, keep plaintext lifetime short, and zeroize Rust-side secret buffers where possible.
3. If hardware-backed protection is unavailable, disclose that posture; do not label software-backed storage “StrongBox.” Whether the app permits that fallback is an explicit setup choice, not a silent downgrade.

The platform boundary exposes sign/verify operations or a short-lived key handle to Rust; it does not pass a long-lived plaintext seed through Kotlin. The first implementation slice must verify algorithm support, hardware level, lock-screen reset behavior, process-death behavior, and Android backup behavior on representative devices before committing to a concrete Keystore API.

### 2.2 Enrollment, recovery, and device loss

- Enrollment creates or imports the user's ID according to #5684's selected custody variant, then creates the phone's scoped device delegation. A phone is not silently made the user's only recovery copy.
- Key envelopes, recovery secrets, and delegation signing material are excluded from Android cloud/device backup. Local journal backup/export is a separate choice and never implies key recovery.
- BiometricPrompt may gate use of the local wrapping key. A changed biometric set, lost device, cleared app data, or invalidated Keystore key is handled as a key-unavailable/re-enrollment state; the app must not claim that biometrics or Google account sign-in can recover the ID.
- Recovery ceremony, security-code count/threshold, and migration between #5684 custody variants remain **owned by #5684**. This record chooses the mobile boundary and the device delegation; it does not invent a competing recovery protocol. Until #5684 settles recovery, onboarding must state plainly that loss of every authorized recovery path can make the identity unrecoverable.

### 2.3 Custody claims

| Actor | Can sign as the user's root identity? | Can block access? | Recovery |
|---|---|---|---|
| Android OS / device owner | Merely holding the phone does not export the key; a compromised OS or app process may still invoke an authorized signer, so hardware backing is not protection from a hostile live OS. Root-key extraction resistance depends on verified Keystore support. | The OS can lock, wipe, invalidate, or—if compromised—misuse local key operations. | No recovery by the OS alone. Use the #5684 recovery path. |
| ID service / remote world host | No key custody and no authority from an Android permission or login token. | Can deny connectivity or reject a request under its existing authority rules. | Cannot recover the user's private key. |
| User's other enrolled device / recovery material | Only within the delegation or recovery authority explicitly granted under #5684. | Can revoke that device's delegation if authorized. | Exact process is #5684's decision. |

## 3. Offline worlds and synchronization

### 3.1 Local-first, not “remote mode versus local mode”

The Android client is thick enough to retain its local journal, identity state, and materialized world views in app-private storage. A world can be local-only, cached from a peer, connected, or awaiting synchronization, but those are explicit states of one client model. V1 does not ask users to choose “thin” or “thick” independently for every world: that multiplies permission, cache, and failure behavior without evidence that it is needed. The deciding evidence to revisit this is measured device storage/sync pressure or a real deployment that cannot use the local journal.

“Local-first” does not mean “silently commit remote writes while disconnected.” A local-owned world can append through its local journal/append gate without opening a network listener; it is not reachable by peers until an explicit sync/export path shares it. For a remote world, offline reads use the last verified local materialization; edits that cannot pass the existing writer-set/append authority are shown as local drafts or pending intents, not as committed world state. The UI distinguishes **saved locally**, **accepted by the world**, and **synchronized**. No optimistic state is presented as a journal-confirmed remote fact.

### 3.2 Journal and sync rules

- Use the existing ID journal, event store, and `event_sync`/crate-split seams. Do not create a second Android database schema as an authority source or a last-writer-wins merge layer.
- The journal is the durable record; a materialized view is rebuildable. Synchronization exchanges signed journal/event material under the existing peer and writer-set rules. The implementation must preserve provenance, ordering/causal metadata, and explicit refusals through sync.
- Concurrent offline events are not silently overwritten. Preserve valid signed events and let existing ID projection/append rules determine acceptance and visible conflict state. If the existing sync seams cannot express a concrete conflict, file a focused design issue with a reproducible trace rather than adding a phone-only merge rule.
- Persist atomically in the app-private directory. Disk exhaustion, migration failure, and interrupted sync must leave the last valid journal recoverable and visibly report unsynchronized work. Never delete the only local copy merely because a remote peer has a newer snapshot.
- Offline capability-provider observations follow #5904: the phone's interactive provider does not promise bridge-style buffering while its session is gone. A fresh observation needs a live grant and OS permission; a gap/offline state is explicit. Actuation is never queued for later execution by this client.

### 3.3 Connectivity

The client uses the existing transport API and protocol. UDP-denied mobile networks are a #5671 transport-fallback concern, not a new Android transport or a reason to weaken end-to-end authority. A lost network is not evidence that a provider is revoked; it changes freshness/presence according to the existing session rules.

## 4. Shell contract and #1831 remote drive

The normal Android app is a native shell over ID's world/client APIs. It renders the current portable `Screen`/text presentation and adds native surfaces only where Android owns a useful affordance: share intents, system content selection, notifications, and platform permission prompts. A shell that cannot render an optional presentation falls back to the contract's portable text/data surface; guest content is never interpreted as executable Android UI.

Remote-drive mode is a feature inside that shell, not the whole product:

- Reuse #1831's accepted event model and Android gesture mapping: tap → press/release, long press → `long_press`, drag → drag sequence, pan/fling → scroll. V1 does not promise pinch, stylus hover, or multi-touch beyond the designed primary pointer.
- Encode those events only after the #1831 interface/contract slice defines them in the versioned shell contract. The producer declares its environment/capabilities and returns explicit refusal for unsupported or undeclared events; it never silently drops an event.
- Do not retain a private “Android remote control” frame schema. Shell conformance is against the same host contract as terminal, web, and the native reference shell. No host change is needed merely because the producer is Android.
- Keep the client-only contract from #790: a phone joining, viewing, or remotely driving a world never starts a world server. Explicit hosting, if ever wanted on Android, requires a separate design, lifecycle/thermal analysis, and an explicit user action.

## 5. Phone as a capability provider

### 5.1 One declaration, two independent gates

The phone declares capabilities through the same signed, typed, versioned ID provider manifest used by other provider classes. Platform discovery APIs are projections of that manifest, never alternate authority. AppFunctions may become a generated Android projection later, but its beta/alpha and gated-caller status in the #5931 §2.1 survey makes it unsuitable as a required v1 surface. Do not build on Accessibility Services as a generic control path.

Every invocation requires both:

1. an ID grant scoped to provider, capability, consumer, purpose, rate/volume, and expiry; and
2. the relevant Android runtime permission or system-mediated selection for the operation.

Effective authority is their intersection. Denial or revocation by either gate stops the invocation and suspends the affected provider capability in discovery; an Android permission does not create an ID grant. The host checks ID authority on every invocation. A transport identity, Binder caller, AppFunctions entry, or notification token is not itself a grant.

### 5.2 Risk classes and lifecycle

- **Observation** (for example, a user-selected photo or an explicitly granted sensor/location sample) is opt-in, purpose-scoped, rate-bounded, and may require per-use Android mediation. The system photo picker is preferred over broad media-library access when it meets the use case.
- **Actuation** (for example, taking a photo, sending a notification, or controlling a device) requires an explicit ID grant and the platform's own permission/foreground context. Default to per-use approval and visible user action; no background actuation or ambient access.
- Android permission revocation, provider shutdown, and app leaving its declared execution envelope immediately suspend dependent capabilities and tombstone discovery per #5903. Availability is not promised outside the OS lifecycle/battery envelope.
- AppFunctions registration, where later adopted, is generated from the ID manifest and can only expose a subset. It never writes authority back into ID. No foreign caller is authorized merely because Android can discover the function.

## 6. App framework, Rust core, and distribution

### 6.1 Kotlin/Compose shell; Rust domain core

Choose Kotlin/Jetpack Compose for Android screens and platform integration. It matches Android's native lifecycle, accessibility, permission, content-picker, and notification APIs without forcing those concerns into a cross-platform renderer. Share the ID domain and protocol behavior in Rust, not the whole UI toolkit. #788's Bevy recommendation concerns the separate F13.6 reference native shell; its own record keeps Bevy in a non-default package and says nothing about Android's native product UX. Reusing Bevy as the Android UI would add engine/window lifecycle and packaging without evidence it improves this app's platform integration.

Use a narrow, typed JNI/FFI boundary. The implementation spike may choose UniFFI-generated bindings or a small JNI/C ABI after checking async Iroh runtime ownership, cancellation, panic containment, and secret-buffer lifetime; that binding choice must not leak Android types into the Rust domain API.

### 6.2 Client-only Rust build variant

The Android app depends on a dedicated client facade assembled through #3447's crate seams (working name `id-client`), not the root `id` binary with its broad defaults. The facade includes only the world/client protocol, local journal/store, identity operations, provider/grant checks, and event sync needed by the app. It excludes CLI/REPL, `id serve` and server listeners, `sandbox`/Wasmtime, web/Axum, and SSH serving. The exact module extraction order belongs to #3447; a Cargo feature named `client-only` on the current monolithic root is not sufficient unless dependency-tree checks prove the server-only graph is actually absent.

The Android artifact is a separate app target. Start with Android arm64-v8a; add x86_64 for emulator/CI coverage if the Rust/Iroh dependency graph supports it. Keep build artifacts and signing outside ordinary root/default-member builds. A first vertical slice must prove the packaged app can connect to a test world, persist/reopen the journal, and pass the same shell-contract checks as other clients before adding provider surfaces.

### 6.3 Distribution and updates

Support both a signed sideload channel for the personal fleet and Google Play for users who want managed installation. Use one package identity and signing lineage where the Play signing arrangement permits it; establish release-key custody before enabling Play App Signing. Promote the same versioned source release through both channels rather than maintaining a fleet-only fork.

- Play-installed copies update through Play's normal update mechanism.
- Sideloaded copies can check a signed release manifest, but installation is handed to Android's package installer and requires the user's approval. The app never grants itself unknown-source installation permission or silently replaces itself.
- Keep signing certificate and monotonically increasing version compatibility so an authorized user can move between channels without losing app-private data. If Play's signing arrangement makes a shared signing lineage impossible, stop and document the explicit channel migration/data-boundary decision before shipping two package IDs.
- The APK update is not an identity-recovery or journal-backup mechanism. #5684 owns identity recovery; journal export/restore is an independent feature.

## 7. Presence and wakeups without a central server

There is no reliable way for a sleeping Android process behind Doze/NAT to learn that an arbitrary peer's world changed without an always-reachable rendezvous/push path. V1 is honest about that limit:

1. Opening/resuming the app triggers authenticated sync immediately.
2. A user may opt into periodic WorkManager sync for selected worlds. It is best-effort, respects Android's scheduling/battery constraints, and is not a real-time subscription or SLA.
3. An explicit, user-started live remote-drive session may use a foreground service with a visible notification for as long as Android policy permits. It ends or becomes offline when that declared envelope ends.
4. Presence is session/lifecycle-derived per #5904. When the app sleeps or the session expires, peers see offline/stale—not online because a periodic job might eventually run.
5. Do not add FCM or another central push dependency in v1. A future push integration may be a non-authoritative wake hint only: it must not carry a grant, execute an action, or replace authenticated sync, and its operator/credential/availability model must be a separately reviewed decision. WorkManager is not presented as event-triggered push.

Thus the answer to “what wakes the app when a world changes?” is: **nothing is guaranteed without a central push/rendezvous service; the user opening the app is the reliable v1 wake, with opt-in best-effort scheduled sync as a convenience.** This is an intentional limitation, not a silent offline promise.

## 8. Open questions from #5780 — resolved or explicitly deferred

1. **Thin vs thick vs per-world:** resolved to one thick/local-first client; per-world connectivity/cache states are not a product-mode switch. Revisit only on measured storage/sync pressure or a concrete deployment requirement.
2. **Play vs sideload:** resolved to both, with sideload for the personal fleet first and Play as a managed distribution channel. Update behavior is channel-native and user-approved; common package/signing lineage is a release gate.
3. **#1831 event model vs shell contract:** resolved: #1831 owns the producer semantics/mapping; the versioned shell contract owns frames, versions, refusals, and conformance. Remote drive is one mode of the full Android client, not an Android-specific protocol. It cannot ship before the #1831 contract slice lands.
4. **Push/presence without a central server:** resolved for v1 as no guaranteed wake, no continuous background presence, foreground sync plus opt-in best-effort WorkManager. Any central push service is deferred to a separate decision with owner/operator, privacy, and availability evidence named.
5. **Kotlin/Compose vs shared Rust UI:** resolved to Kotlin/Compose for Android UI and Rust for shared core/protocol. #788's Bevy decision is not an Android framework decision. Revisit shared UI only if another mobile platform is actually in scope and a parity/accessibility spike shows a net benefit.
6. **Exact recovery ceremony and multi-device household model:** deferred to #5684, which remains the owner of the custody variants and recovery semantics. This design adopts #5903's device-bound delegated provider identity and forbids the Android app from inventing a parallel recovery system.
7. **Exact Rust FFI generator and minimum Android API/ABI matrix:** implementation spikes owned by the Android client follow-on. They must establish buildability, runtime ownership, panic/cancellation safety, actual Keystore support, and package size on representative devices; they do not reopen the Kotlin/Compose decision.

## 9. Implementation follow-ons

These are named next actions; they are not implementation performed by this design PR. The parent issue #5780 remains the design record until these slices are accepted and the owner chooses whether to split them into tracked child issues.

1. **Crate-split client facade and Android target:** extract `id-client` along #3447 seams; prove server-only dependencies (`wasmtime`, web, SSH, serve/listeners) are absent from its Android graph; build arm64-v8a and exercise one authenticated client session.
2. **Android identity/enrollment spike:** test direct Ed25519 Keystore/StrongBox support and encrypted-key-envelope fallback on representative API levels/devices; specify app backup exclusions, biometric invalidation, key zeroization, and the #5684 recovery handoff.
3. **Local journal and reconnect sync:** persist/reopen a local world, sync through existing event-sync seams, inject interruption/storage-full cases, and prove no remote event is shown committed before authority accepts it. Do not add a new merge algorithm absent a failing trace.
4. **Kotlin shell and contract conformance:** implement the portable-screen/text client, lifecycle cancellation, and versioned session transport; run the shared contract corpus against Android. Keep native-only surfaces behind typed adapters.
5. **#1831 remote-drive integration:** after the input-event contract slice lands, implement Android gesture mapping and explicit refusal UX; test tap/long-press/drag/scroll replay and unsupported multi-touch/refusal cases.
6. **Phone provider v1:** choose a minimal, user-mediated provider capability set; implement the two-gate ID-grant ∩ Android-permission check, revoke/suspend behavior, and foreground-only actuation. Do not depend on AppFunctions for authority.
7. **Lifecycle, presence, and release channels:** verify WorkManager behavior and session expiry on real devices; package signed sideload updates and Play-track updates with the agreed signing lineage. No push service is included without a separate approved decision.

## 10. Invariants and acceptance mapping

1. **No root-key upload:** authentication, Android permissions, AppFunctions, FCM, and transport credentials never substitute for ID signing authority.
2. **Two gates:** every phone-provider invocation passes both the ID grant and the Android permission/mediated selection; either denial is final and named.
3. **Journal truth:** a local cache/draft is never mislabeled as a remote committed event; no last-writer-wins layer silently deletes signed journal history.
4. **Client-only:** opening the app, a local world, a provider, or a remote-drive session never starts a world host or grants ambient host authority.
5. **Protocol reuse:** remote-drive and ordinary shell traffic use the shared versioned shell contract; unsupported events produce explicit refusal.
6. **Lifecycle honesty:** presence expires when the app/session leaves its supported envelope; scheduled work is best-effort and does not promise immediate wake.
7. **Build isolation:** the Android app consumes a client-only Rust facade and does not pull CLI, sandbox, web, or server-listener features into the shipped app.
8. **Platform provenance:** Android permission and AppFunctions state are projections/second gates; ID's signed declaration and grant checks remain authoritative.

| #5780 acceptance | Covered by |
|---|---|
| Keystore/StrongBox custody applying #5684 to mobile | §2, including the explicit real-device Keystore spike and #5684-owned recovery handoff |
| Offline local journal + sync through #3447 seams | §3 and follow-on 3 |
| Shell contract on mobile and native surfaces | §4 and §6.1 |
| Phone as #5686 provider with Android runtime permissions | §5, adopting #5903's two-gate and lifecycle decisions |
| Rust client-only build shape and Android UI decision | §6; dedicated `id-client` seam, Kotlin/Compose UI |
| Four open questions | §§3, 6, 7, and explicit answer table in §8 |
| Implementation next actions do not sit silently | §9 |

## Alternatives rejected

| Alternative | Why rejected |
|---|---|
| Thin remote-only app | Contradicts the local journal/offline-world requirement and makes network access the condition for reading local work. |
| User-selectable thin/thick mode for each world in v1 | Adds a second state/permission model before storage or sync evidence demonstrates the need. |
| Treat Android permission/AppFunctions as the provider grant | Android permissions are app-level/platform authority; #5686/#5903 require independent, consumer- and purpose-scoped ID grants. |
| Keep a long-lived socket and report continuous presence | Android background/battery rules and peer NAT make that promise false without an always-reachable service. |
| Reimplement pointer events in an Android-only protocol | Duplicates #1831 and breaks parity with terminal/web/native shells. |
| Build the Android UI in Bevy/shared Rust immediately | #788 selects Bevy for a different native-shell goal; no Android accessibility/lifecycle evidence supports sharing the whole UI. Keep the domain and protocol shared instead. |
| Automatically host local worlds from the phone | Violates #790's client-only default and creates unreviewed listener, battery, and authority behavior. |
