# Fleet transport & pooling design

**Date:** 2026-11-23 · **Direction:** Drew, 2026-11-23 15:07 CST · **Tracking issue:** #13752
**Status:** design record — no build is authorized by this document; each work item lands as its own slice issue.

Three threads, one design:

1. Choosing hosts and reaching them **without Tailscale**.
2. **Direct iroh** as a transport for non-Muse fleet agents.
3. A **generic pool primitive** underneath both — and underneath the model pools and placement that already reinvent it.

The threads compose into one stack:

```
        policy (domain)          weights, free-first, reserves, pace guards
              │
        pool primitive (thread 3) state machine, scope-keyed walls, strategies,
              │                   attempt semantics, recovery, decision log
              │
        members                  model legs · (host × transport-form) pairs ·
              │                   machines · compile slots
              │
        transports (threads 1–2) commander-pattern door · laptop-agent ·
                                 ssh · tsnet · iroh
              │
        identity (existing)      fleet CA (#9155) / interim IdP CA — one host
                                 identity binding every transport credential
```

## 0. Premise check — what already exists

This design was written against the record, not from memory. Findings:

- **#13717 (filed today)** codifies the laptop route forms: 3 methods (commander / http-direct / ssh) × 2 shapes (fleet subcommand / raw command) + the `laptop-ssh` wrapper (F7), the `fleet laptop` walker (attempts F1 → F3 → F5 → F7, advancing only on failure-to-serve), timeout-by-construction, and form visibility in every result. Thread 1 generalizes its matrix from *the laptop* to *any host*; nothing here re-opens it. (#13654 is the family's F1.)
- **#10356 (2026-11-14)** designs `fleet connect` — explicit transport *method* selection (`--method exec|ssh|job|auto`), a session model generalized from the daemon's job spool, and drop semantics per path. It is the method-choice layer on a known host; this design is the host-choice layer above it. #10356 also records the constraint that shapes thread 1: **the sandbox has no inbound path at all** — every connection is initiated from the sandbox side or rendezvoused through the session proxy / brokers. Hosts cannot dial agents; every no-overlay route needs a named rendezvous.
- **#2179** already specifies the target this design needs: onboard machines against a standard and redesign placement around "a machine registry (identity, capabilities, …)". Thread 1 extends that registry; it does not invent another.
- **#1967 is closed**, with a recommendation this design preserves: state the requirement — (1) reach a daemon by stable cryptographic identity, not address; (2) NAT traversal without port forwarding, with relay fallback; (3) end-to-end encryption and mutual authentication from that identity — then pick the transport on merit. For mcpx (Go), tsnet won on merit because iroh is Rust: cgo against `iroh-ffi` poisons a pure-Go build, and the sidecar option (B) was the honest iroh path. **The fleet binary is Rust.** The fact that decided #1967 inverts for fleet-side components, and only for them.
- **opencode#2256 is closed** (fork PR #2598): the wall doctrine — wall when the binding window measures <1% remaining AND the provider returns quota-class errors; rejoin on measured increase; never `disabled_providers` for quota. The live scoping layer is **#13693**: walls at the narrowest true scope, alias providers for mixed plan+free, scope inference from error shape and sibling evidence, config declaration as fallback override. Thread 3 is that doctrine generalized, written once.
- The fleet orchestration architecture record (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/architecture.md`) contains no host-selection or transport material. The gap is real, not a missed cross-reference.

## 1. Thread 1 — Host selection and access without Tailscale

### 1.1 Problem

Every route the estate has to a host was built for one host (`amd`, "the laptop") and most lean on Tailscale somewhere: the laptop-agent's HTTP legs run over pooled CONNECT tunnels through the session proxy and hairpin a DERP relay (#10356 measured: warm call ~0.39–0.45 s, cold CONNECT ~1.8 s); real SSH rides an HTTP CONNECT ProxyCommand; the daemon is also reachable over the tailnet directly. An agent that is not on the tailnet — a seat that never enrolled, an overlay outage, a non-Muse agent with no tailnet client — has *no* route, even to a healthy host.

The commander endpoint (#13404) is the working counterexample: a Cloudflare Worker as gate + proxy, a tunnel from the host out to Cloudflare, OAuth/bearer at the door, the origin locked to the tunnel. No overlay anywhere in the path. It was built for one service on one host; thread 1 is its generalization.

### 1.2 The host record

A host is a record in the #2179 machine registry, extended:

| Field | Content |
|---|---|
| `id` | Fleet identity — a certificate identity under the fleet CA program (#9155); until that lands, the interim IdP CA. Every other credential a host holds is *bound to* this identity, never a replacement for it. |
| `names` | Human/DNS names (`amd`, `amd-6.…ts.net`, …) — labels, not identity. |
| `capabilities` | What the host can do (queue worker, opencode server, compile slots, …) — #2179's field, unchanged. |
| `transports` | **Transport offers.** For each transport the host serves: kind (`commander-door`, `laptop-agent`, `ssh`, `tsnet`, `iroh`), endpoint data (door URL, daemon address, `NodeId`, port), and the authenticating credential **by name only**. |
| `enrollment` | Which agent classes may use which offers (Muse seats, opencode lanes, external MCP clients, …). |

The **live overlay** — last-seen, load, per-offer health — is *not* in the registry file. The queue broker already tracks seat liveness; the overlay lives there, keyed by registry `id`. Rationale: the registry is the static source of truth (reviewable, in git, applied by the flake); liveness is runtime state and dies with the runtime. Putting liveness in git is how registries become lies.

**Cold start.** An agent with nothing configured needs one rendezvous that works from bare internet: DNS under our own zones publishing the *stable public doors only* (commander-pattern endpoints). DNS carries offers, never secrets, and is never a liveness source — a door that stops answering is a pool event (§3), not a DNS emergency.

### 1.3 Selection

Resolving a host name produces candidate members: one per (transport offer × invocation form) — the #13717 matrix as data. For the laptop today that is F1–F7; for an iroh-offered host it includes the iroh forms; for a host with no public door it may be tailnet forms only. Selection among candidates is a pool decision (§3): strategies and walls live there, not in a per-host hand-written ladder. `fleet laptop` is `fleet host amd` with the registry consulted — the N=1 case stops being special the day a second machine onboards (#2179's hardware already exists).

### 1.4 Access without Tailscale — the options, honestly

| Option | Works with no overlay? | Rendezvous | Auth hops | Verdict |
|---|---|---|---|---|
| **Commander-pattern public door** (Worker gate + host-initiated tunnel) | Yes | Cloudflare (DNS name) | Door: OAuth or machine bearer (secrets broker, by name) → host: the bridged service's own auth; origin accepts tunnel ingress only | **Primary.** Proven by #13404; the pattern to repeat per host/service that needs a public door. Cost: Cloudflare dependence; the host must hold a tunnel open outbound. |
| **iroh** (thread 2) | Yes | Relay (self-hosted or n0) + pkarr/DNS discovery | Door *is* the host: peer NodeId authenticated by the QUIC/TLS handshake; authorization = application allowlist | **Primary for agent↔host.** No public door to defend; identity is cryptographic end to end. |
| **Direct public SSH/HTTP** | Only where the host has public reachability | The address itself | sshd / daemon token | Niche. Most estate hosts sit behind NAT; listed so nobody designs for it by accident. |
| **Headscale** (self-hosted tailnet control plane) | **No** — removes the vendor, keeps the overlay | Headscale server | Tailnet identity, as today | Recorded and set aside: it answers "not Tailscale-the-company", not Drew's "without Tailscale". Remains a legitimate fallback if overlay semantics are ever the requirement rather than the obstacle. |
| **Keep tailnet legs where they exist** | n/a | DERP / direct | Tailnet identity | Unchanged. Nothing in this design removes a working leg; the pool (§3) holds overlay and non-overlay offers side by side and the walls sort out which serve. |

### 1.5 Recommendation

Registry hybrid (§1.2) + commander-pattern as the general no-overlay door + iroh as the agent door (§2) + DNS cold-start for public doors. One identity per host under the fleet CA, binding commander bearers, daemon tokens, sshd keys, and the iroh NodeId into a single revocable subject — so decommissioning a host is one revocation, not a scavenger hunt.

## 2. Thread 2 — Direct iroh for non-Muse agents

### 2.1 Who, and why not the Muse stack

Non-Muse seats — opencode lanes, grok, felix, xaltha — currently reach hosts through machinery built for Muse sessions: the session proxy, the brokers, Muse's enrollment and bridge tokens. That machinery is a dependency they inherit, not a transport chosen for them; when it wedges (the channel-repair history is long), they wedge with it. Thread 2 gives them a direct transport: seat → host, identity-addressed, no session stack in the path.

### 2.2 iroh, from its own documentation

Facts below are from iroh's docs (docs.rs `iroh` / `iroh-net`) and n0's published design writing, checked 2026-11-23.

- **Identity is the address.** Each node holds an ed25519 `SecretKey`; its public key is the **NodeId**. QUIC's TLS handshake is keyed to that secret — no certificate chain, no CA. Dialing a NodeId means the connection can only be established with the holder of that key, and an accepting endpoint learns the peer's authenticated NodeId from the handshake. Authentication is therefore intrinsic; **authorization is the application's** — the endpoint decides which NodeIds may connect, per protocol (ALPN). This is exactly #1967's split: transport provides 1–3; who-may-do-what is a separate layer.
- **How a connection happens.** The endpoint connects to its closest relay and designates it its **home relay**. A peer connects via that relay first; the two then attempt a direct path (hole punching; in the current iroh line, QUIC-native address discovery + NAT-traversal frames inside the established connection) and migrate off the relay when a direct path sticks. If none sticks — symmetric NAT, hostile firewall — the relay path carries the connection for its whole life. Relays forward **encrypted** packets addressed by NodeId; a relay cannot decode payloads.
- **Discovery is a trait, and services compose.** Shipped options: **DNS discovery** — pkarr packets (signed DNS records keyed by NodeId) published through a pkarr relay and served over DNS by `iroh-dns-server`; **pkarr resolution over HTTP**; **DHT discovery** over the BitTorrent Mainline; **local swarm discovery** (mDNS-like, LAN scope). Multiple services run concurrently and race. **Tickets** — an encodable bundle of NodeId + relay URL + direct addresses — bypass lookup entirely and are the natural bootstrap for a registry-driven fleet: the registry (§1.2) can carry ticket-equivalent data as the iroh transport offer.
- **Relay dependence, stated plainly.** A relay (or pre-known direct addresses) is needed to *start* most connections. It is rendezvous + fallback, not a through-route for healthy connections. n0 operates public relays and a public pkarr/DNS instance; **both server components are self-hostable** (`iroh-relay`: ACME or manual TLS, per-client rate limiting, metrics). Depending on n0 is a choice, not a requirement.
- **Embedding.** iroh is a Rust library: `Endpoint::builder()…bind()`, an ALPN router for protocols, streams and datagrams over the QUIC connection. In the fleet binary it is a dependency, not a process. The Go story is #1967's and has not changed: `iroh-ffi` + cgo (rejected there), a sidecar (option B, still the honest Go path), or nothing. A third-party Go implementation has appeared since; its maturity is unverified — noted, not planned on.

### 2.3 Replaces vs complements

| | |
|---|---|
| **Replaces** (for non-Muse seats) | The session-proxy/DERP hairpin as their rendezvous+traversal mechanism; the Cloudflare tunnel for point-to-point seat↔host traffic. |
| **Complements** | The commander door — hosted MCP clients (ChatGPT, Claude connectors) speak HTTPS or nothing; they will never run iroh, so the public door is permanent infrastructure, not a stopgap. tsnet — mcpx's Go daemon keeps #1967's answer; Muse's own session stack is untouched by this thread. iroh-blobs (BLAKE3-verified, resumable, content-addressed transfer) is the convergence #1967 already flagged: if prepared dependencies / artefact closures ever move peer-to-peer, the same mechanism carries them — recorded as a horizon, not a pilot goal. |

### 2.4 Risks and costs

- **API youth.** iroh renamed its core crate (`iroh-net` → `iroh`) and restructured discovery across the 0.2x–0.9x line; a 1.0 was still ahead at last check. Mitigation: pin the version, and expose iroh only behind fleet's own transport trait so an upgrade is one adapter, not a refactor.
- **Relay operations.** Self-hosting a relay + pkarr/DNS is one more public service to run — but the estate already runs exactly this shape (the commander Worker + tunnel, the Access gates). The relay holds no keys and sees no plaintext; its compromise is an availability event, not a confidentiality one.
- **A second identity system, if done carelessly.** NodeIds must be *bound to* fleet identities in the registry (§1.2) and issued/rotated under the same custody rules as other host keys (#9155) — a NodeId that lives only in a seat's local config is a shadow identity and is out of design.

### 2.5 Recommendation

Pilot, scoped hard: the fleet binary hosts an iroh endpoint; **one** non-Muse seat class (opencode lanes) reaches **one** host (amd) over it, as a new transport offer in the registry alongside the existing legs. Self-host relay + pkarr/DNS under our zones from day one (n0 public infrastructure as bootstrap fallback only, recorded when used). Pre-registered measures, against #10356's baseline: direct-path rate (share of connections that leave the relay), connection setup p50/p99, warm-call latency vs ~0.4 s hairpin, and failure classes observed. The pilot passes, fails, or extends on those numbers — the performance doctrine applies to transports too: a multiplier is proven by benchmark, never asserted.

## 3. Thread 3 — A generic pooling system

### 3.1 The reinvention census

| Instance | Members | What it re-learned the hard way |
|---|---|---|
| Model route pools (opencode fork + route proxy) | (provider, model) legs | Walls must bind at the cap's scope, not the reader's shape (#13693 RC-3); a reset-less wall must still gate (RC-1/RC-2); all-walled pools must stop walking (RC-4). |
| google-free round-robin | 4 project aliases × models | Caps are per-model-per-project — the pool *is* the quota multiplier; rotation without state is not enough once one project walls. |
| Route-form walker (#13717) | F1–F7 forms | Advance on failure-to-**serve**, never on a served result; name the serving form in every result. |
| Fleet placement | Machines / seats | Unread load is **unknown, never zero** (#13611); placement on a blind zero piles work onto a loaded host. |
| Compile dispatch | Build slots across seats | Cross-owner claims need atomicity a per-seat view can't give — the one place a *central* coordinator is directed (separate build; see §3.4). |
| Id spill (#7572) | World holders | Lease/fairness semantics are real and domain-shaped — the cautionary boundary for §3.3. |

Five implementations, one machine. Thread 3 writes the machine once.

### 3.2 What the primitive owns

- **Members and groups.** A member is opaque to the primitive; groups nest (member ⊂ provider ⊂ account; form ⊂ method ⊂ host) because walls bind at group scope as often as at member scope.
- **The state machine.** Per member (and per group, for walls): `serving → demoted → walled(scope, until, evidence) → probing → serving`. Transitions are driven by *classified outcomes* (§3.3) and by probes.
- **Scope-keyed walls with inference rules.** A wall is recorded at the narrowest scope the evidence supports. Widening: repeated failures across siblings at a shared scope, with no sibling success, widen the wall to that scope. Narrowing: a sibling's success during a wide wall narrows it (the walled member stays walled; the group re-opens). These are #13693's rules, stated once, generically — its alias-provider pattern becomes a grouping declaration instead of a config trick.
- **Attempt semantics.** `serve-failure` (transport/auth/connect/timeout — advance) vs `served-failure` (the member answered; the classifier decides whether it advances, demotes, or walls). This is #13717's walker rule, generalized — and it is the rule whose absence made walled models "keep getting hit indefinitely" (#13693 RC-1).
- **Strategies.** Ordered walk; round-robin; weighted; free-first (ordering by declared cost class); least-loaded (with unknown-load as a first-class value that never sorts as zero, #13611). Strategies compose: free-first *then* weighted within class is the model-pool shape; ordered *then* round-robin within tier is the host shape.
- **Recovery.** Three doors back, all automatic: recorded reset expiry; measured increase (opencode#2256's rejoin); and **probe re-admission** — a bounded, cheap probe whose success re-seats the member at demoted weight before full traffic. A wall with no recorded reset still gates, via a bounded inferred wall with its own probe schedule (the #13735 slice-P shape), so "no reset parsed" can never again mean "no wall".
- **The decision log.** Every selection emits (member, state, strategy, reason). The #13717 form-visibility rule is the local case of a general law: **a pool decision you cannot read back is a bug.** Stewardship and debugging consume this log; it is not optional telemetry.

### 3.3 What stays domain-specific

- **Member identity and grouping** — declared by the domain.
- **Measurement** — quota gatherers, latency/load probes, health checks. The primitive schedules probes; it does not define them.
- **Classification** — turning a raw failure into (class, scope-evidence, reset-hint). Quota-vs-transport-vs-auth is knowledge only the domain has; #13693's classifier work stays in the proxy/fork and *feeds* the primitive.
- **Policy** — weights, cost classes, reserve rules (OpenAI last), pace guards (#2732), fairness/leases (#7572). Policy configures; it does not re-implement state.

### 3.4 Options

- **(a) Shared crate (`fleet-pool`, Rust, fleet workspace).** Traits: `Member`, `Classifier`, `Strategy`, `Probe`; the state machine and decision log in the crate. Consumers link it and keep their own process shape. *Recommended.*
- **(b) Pattern spec only.** Every domain re-implements against a written contract. Zero migration — and precisely the drift that produced five divergent implementations, two of which re-learned the wall lesson in production.
- **(c) Central pool service.** One decision point for all domains. Right exactly where cross-owner atomicity is the requirement — the compile coordinator, already directed as its own build, is that case and should use the primitive's state machine *inside* its service. As the general shape it makes every route decision a network hop and a shared failure point; rejected as the default.

### 3.5 Adoption order

1. **Route-form walker** — smallest surface, already specified (#13717); its acceptance cases become the crate's first oracle tests.
2. **Host×transport selection** (thread 1) — born on the primitive; no legacy to migrate.
3. **Model-route boundary** — the proxy/fork keeps its classifiers and plants; the primitive consumes recorded outcomes and returns selections. Sequenced *after* #13693's scoping layer lands, and explicitly not a rewrite of it.
4. **Placement** — reviewed last, against #2179; a "not yet" conclusion is acceptable and recordable, because placement's fairness semantics are the least generic part of the estate.

Property tests name their oracles: *a wall never widens past the scope its evidence supports; a sibling success never leaves a group wall wider than the failing member; an unknown load never wins a least-loaded selection against a known non-zero load; a served failure never advances the walk unless its classifier says so.*

## 4. Sequencing across the threads

1. **3a → 1a**: the pool crate's skeleton and the registry schema are independent paperwork-and-code slices; either can start first, but host selection (1c) needs both.
2. **2a (iroh pilot)** needs only 1a's registry field to record its offer — it must not wait for 1c.
3. **1d (DNS cold-start)** is independent and small; it also de-risks 2b (same zones, same publication discipline).
4. Nothing here touches the Muse session stack, mcpx's tsnet, or #13693's in-flight slices.

## 5. Cross-links

#13752 (tracking) · #13654 (commander subcommand — form F1) · #13717 (route-form family) · #10356 (fleet connect — method selection, sessions, drop semantics) · #2179 (machine pool + registry) · #1967 (iroh and the Go-binding question — closed) · #13669 (mcpx→Rust pilot) · #13693 (wall scoping) · opencode#2256 (wall doctrine — closed) · #13404 (commander endpoint) · #9155 (fleet CA) · #8991, #9399 (pool routing epics) · #13611 (load reads as unknown) · #7572 (id spill — fairness boundary) · #2732 (pace guards)
