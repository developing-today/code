# Research: prior art — capability systems, OCAP, Spritely Goblins/OCapN, CRDT shared worlds

- **Date:** 2026-11-09
- **Status:** research deliverable for [#5989](https://github.com/developing-today/code/issues/5989); docs-only, no implementation is included here.
- **Issue:** [#5989](https://github.com/developing-today/code/issues/5989), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S77 and S110 of the 2026-11-04 brief).
- **Deliverable shape (per #5989):** a per-system entry for each system in scope, each closing with an explicit **adopt / adapt / reject, because** line for our design; a dedicated section drawing the CRDT-shaped vs authority-shaped line for world state; and a synthesis mapping the findings to the #5955 children they feed — the programming-model child (our "pass the platform in, nothing ambient" model), the identity/invite system ([#5751](https://github.com/developing-today/code/issues/5751)), writer-set authority ([#5545](https://github.com/developing-today/code/issues/5545)), and the self-hosting child.
- **Coordination:** the capability-provider survey [#5931](https://github.com/developing-today/code/issues/5931) covers provider *classes* (which providers exist and what they vend); this survey covers capability *theory and protocols* (what a capability is, how it moves, how it is revoked). Where the two touch — WASI/component model — this doc states the theory position and defers provider enumeration to #5931.
- **Method and honesty notes:** entries are written from the public documentary record of each system (papers, manuals, project documentation, post-mortems). Where a system's claims are its designers' rather than independently verified results, the entry says so. Our model's own terms are used as stated in #5989 and the #5955 children: capabilities arrive by being *passed in* (no ambient authority), with compile-time platform ceilings as a deliberate deviation from classic ocap, noted where it matters.

## Why this research exists

Our worlds model is an object-capability design whether we call it one or not: user code runs inside worlds, holds references to things, and can do exactly what the references it was handed allow — nothing ambient, nothing by name alone. That idea has a fifty-year lineage with sharp theorems, at least two protocol traditions (CapTP and OCapN), a working distributed-actor implementation (Goblins), and a parallel tradition (CRDTs) that solves a *different* problem — convergence without authority — which is easy to confuse with ours. This survey separates the three questions the lineage keeps distinct: **who may act** (capabilities), **how actors reach each other across machines** (protocols and naming), and **how shared state converges** (CRDTs vs authority). The #5955 children each inherit one of those questions; mixing them is how designs go wrong.

---

## Part 1 — Object-capability systems (the theory)

### 1.1 E (Electric Communities / ERights, 1990s–2000s)

- **What it is:** a programming language (Mark Miller and others, growing out of Electric Communities' Habitat successor work) built entirely on object-capability discipline, with distributed computation via CapTP and promise pipelining.
- **Core mechanism:** a reference *is* the authority: holding an object reference is the only way to invoke it, and there is no global namespace to reach around it. E adds two things classic ocap statements often omit: **promises** (a reference to a not-yet-computed value, sendable immediately — "promise pipelining" collapses distributed round trips) and **revocation by caretaker** (interpose a forwarding object you can later switch off; revocation is a pattern, not a primitive).
- **Map against our model:** our "pass the platform in" *is* E's discipline at the world boundary: a program's platform parameter is its initial reference set, and everything else must arrive by introduction. Our compile-time platform ceilings, however, are **not** E: E has no static ceiling concept — authority is purely dynamic and reference-shaped. The ceiling is a static bound on what *can be passed in*, which is closer to a loader policy than to anything in E.
- **Verdict: adopt** the reference-is-authority discipline and the caretaker pattern for revocable grants, **because** they are the proven vocabulary for exactly our boundary; **adapt** promise pipelining only if our cross-world calls turn out latency-bound, because its debugging cost is real and documented by E's own community.

### 1.2 Joule (Agorics, 1990s)

- **What it is:** a concurrent capability language by Dean Tribble, Mark Miller and others at Agorics, designed for distributed agoric (market) systems; a direct ancestor of E.
- **Core mechanism:** **channels and distributors**: all communication is by passing references over channels; Joule's contribution to the lineage is treating *resource management* (who pays for computation, how a channel's capacity is allocated) as part of the capability fabric rather than an afterthought, and its insistence that a program's behavior be analyzable from its reference graph alone.
- **Map against our model:** Joule is the ancestor that took metering seriously as a language concern — the same concern our per-program ceilings and budgets address statically. Its analyzability goal matches why we want platform ceilings legible at compile time: a reviewer should be able to bound what a program can reach without running it.
- **Verdict: adapt** — Joule itself is a historical source, not an implementable dependency, **because** its durable lesson is the one we already encode (authority and resource bounds must be visible in the program's interface, not in runtime folklore); cite it as lineage, build nothing on it.

### 1.3 Cap'n Proto and CapTP (2013– )

- **What it is:** Kenton Varda's serialization/RPC system; its RPC layer is a CapTP implementation (capability transport protocol, inherited from E) with promise pipelining over capability references, used in production (e.g. inside Cloudflare Workers' bindings).
- **Core mechanism:** RPC references are unforgeable, exportable/importable across connections, and support **third-party introduction** (handing a peer a reference to a third party, which the protocol resolves directly) and distributed garbage collection of remote references.
- **Map against our model:** this is the strongest candidate shape for how capabilities cross *our* machine boundary — between user-hosted worlds and the hosted backstop, where Phase-14 principles require identical behavior. CapTP's introduction semantics match our invite flows (#5751): an invite is precisely an introduction of a capability to a third party.
- **Verdict: adopt** the CapTP semantics (unforgeable references, introduction, promise pipelining) as the reference model for any cross-host capability protocol we design, **because** it is the only capability protocol with both a formal lineage (E) and a maintained production implementation; whether we adopt the Cap'n Proto wire format itself is an implementation decision for the protocol child, not settled here.

### 1.4 seL4-style capability hardware/OS work (seL4, CHERI)

- **What it is:** the kernel/OS end of the lineage: seL4 (formally verified microkernel) derives all authority from unforgeable capability objects managed in CSpaces; CHERI extends the idea into hardware (capability pointers in registers/memory, provenance and monotonicity enforced by the CPU).
- **Core mechanism:** **derivation trees and monotonic attenuation**: a capability can only be copied in weakened form (fewer rights, narrower range); seL4 tracks derivation so revocation can reclaim children. The kernel itself holds no ambient authority over user objects beyond the capabilities it is handed.
- **Map against our model:** two imports. (1) *Attenuation on delegation* is the rule our invite/sharing flows need: what you pass on is never stronger than what you hold — our writer-set authority (#5545) should state this as an invariant, not a hope. (2) *Derivation tracking* is what makes revocation tractable; E's caretaker does it by pattern, seL4 by bookkeeping. Our model currently has no stated revocation story for passed-in platform capabilities; this is the gap this survey most wants the capability-model child to close.
- **Verdict: adapt** — adopt attenuation and derivation-tracked revocation as *design invariants*, **because** they are proven at the layer where mistakes are unfixable; reject any literal dependence on seL4/CHERI as a runtime requirement, because our worlds must run hosted by anyone on commodity stacks (Phase-14: hostable by anyone, at any scale).

---

## Part 2 — Spritely Goblins and OCapN (the living protocol tradition)

### 2.1 Spritely Goblins (Spritely Institute, 2019– )

- **What it is:** Christine Lemmer-Webber's transactional actor environment (Racket, with Guile/Wisp ports), the reference environment for OCapN; actors ("objects") hold capabilities, turns are transactional, and the system is built for networked virtual worlds — Spritely's own framing is explicitly the Habitat lineage continued.
- **Core mechanism:** **transactional turns**: an actor's turn either commits fully or rolls back, which makes distributed-object programming dramatically less error-prone than raw eventual-send; **near/far references** unify local and remote objects under one reference discipline; persistence is via orthogonal persistence of actor state (Goblins' "persistence via rehydration" model).
- **Map against our model:** Goblins is the closest living relative of our worlds: persistent user objects, capability authority, worlds as social spaces. The mapping that matters most is *transactional turns vs our journal/revision model*: our object model (in the importer lineage, #2048) is event/revision based, not turn based — a world author's rules execute against committed revisions. Goblins demonstrates the alternative is viable at world scale; it does not demonstrate it is compatible with our revision history, and it isn't, directly.
- **Verdict: adapt** — adopt near/far reference unification as a *goal* for our cross-world references (a reference should not change meaning when its target moves hosts), **because** Phase-14 requires p2p behavior identical to hosted behavior; reject importing transactional turns wholesale, because our committed-revision model already chose its consistency story and the two do not compose without a redesign no child has asked for.

### 2.2 OCapN (Open Capability Network protocols)

- **What it is:** the protocol family Spritely is standardizing for interoperable capability networks: CapTP (the transport, as in §1.3), **Netstring/Goblin-style sturdy references** (unforgeable, offline-holdable references that can be rehydrated into live connections), and locator/identity conventions so independently-run nodes interoperate.
- **Core mechanism:** a sturdy reference is data: it can sit in a message, a file, or a QR code while its target is offline, and becomes a live capability when presented to a node that can rehydrate it. Identity of a node is cryptographic (key-based), not DNS-based.
- **Map against our model:** sturdy references are almost exactly our **invite** shape (#5751): an invite must survive being carried out-of-band, presented later, and redeemed against a world that may have been offline — and must be attenuable (view-only invite vs join invite) and revocable. OCapN's work is the prior art that has already made the mistakes here (replay, confused-deputy at rehydration, locator spoofing).
- **Verdict: adopt** sturdy-reference semantics as the design baseline for #5751 invites, **because** the invite *is* a sturdy reference and re-deriving its failure modes from scratch would repeat a decade of protocol bugs; adapt the concrete locator scheme to our key-based identity rather than importing OCapN's whole stack.

### 2.3 Petnames (Zooko's triangle; Spritely petname systems)

- **What it is:** the naming layer of the capability tradition: global names are either secure or memorable, not both (Zooko's triangle); petnames solve it socially — *your* local, memorable name for a key-identified entity, with introductions carrying suggested names ("edge names") you may accept or replace.
- **Core mechanism:** names are local claims, never global facts; an introduction binds a cryptographic identity, a suggested name, and the introducer — the same shape our object model already uses for names/aliases as claims in the #2048 lineage.
- **Map against our model:** our identity/invite system (#5751) will be judged by users entirely through names: world names, member names, object names. If any of those silently become global identifiers, we inherit phishing as a platform feature — the exact failure petnames exist to prevent.
- **Verdict: adopt** — petname discipline (local names over key identity, introducer-attributed edge names) is a requirement for #5751, **because** it is the only naming design in the lineage that is both usable and spoof-resistant, and it composes with our names-as-claims object model without translation.

---

## Part 3 — CRDTs and shared state: drawing the line

### 3.1 CRDTs proper (Shapiro et al. 2011; Automerge, Yjs, Loro)

- **What it is:** conflict-free replicated data types: state-based (CvRDT) or operation-based (CmRDT) structures with mathematically guaranteed strong eventual convergence — replicas that have seen the same updates are in the same state, with no coordination and no authority.
- **Core mechanism:** convergence is a property of the *data type* (commutative operations or a join-semilattice), bought by giving up invariants that span replicas: a CRDT cannot enforce "only one winner", "balance never negative", or "this parcel has exactly one owner" while partitioned — those require coordination or authority.
- **Map against our model — the line #5989 asks for:**
  - **CRDT-shaped problems** (convergence is the whole requirement; no fact of the matter beyond merge): presence and cursors, shared text/document editing, collaborative building *drafts*, annotation layers, chat backscroll, any per-user state that others merely observe. Use CRDTs; do not spend authority machinery here.
  - **Authority-shaped problems** (an invariant someone must be *able to violate the merge over*): game state under a world author's rules, ownership and transfer of scarce objects/land, economy balances, moderation actions, writer-set membership itself. These belong to the authority that owns the invariant — in our model, the writer set (#5545) and its committed revision order. A CRDT here is not a simplification; it is the invariant's quiet deletion.
  - **The hybrid that actually ships:** authority-owned state *references* CRDT documents (the world commits "the current version of document D is v", while D's interior converges by CRDT) — this is the shape both Automerge-in-production and our revision model point at, and it keeps each mechanism on its own side of the line.
- **Verdict: adopt** CRDTs for the CRDT-shaped class with the line above stated as a design rule, **because** convergence without coordination is genuinely free there; **reject** CRDTs as the substrate for owned world state, because scarcity and authorship are authority facts, and a merge function cannot mint them.

### 3.2 Shared virtual worlds and "real estate" (Habitat 1986; Second Life 2003; modern attempts)

- **What it is:** the social end of the lineage. Habitat (Lucasfilm/Quantum Link, Morningstar & Farmer) invented the graphical virtual world and its governance lessons ("Cyberspace Innkeeping"); Second Life industrialized virtual land: parcels with owners, permissions, an official land market and currency exchange; modern attempts (crypto-land platforms) repeated the land market with speculation first and worlds second.
- **Core mechanism (Habitat):** objects with behaviors, avatars, regions under operators — and the documented discovery that *the social contract is the platform*: Habitat's operators learned that scarcity, theft, and dispute resolution are governance features that must be designed, because users generate them whether or not the system names them.
- **Core mechanism (Second Life):** land as the unit of scarcity and rent: parcel ownership gates building rights; the platform (Linden Lab) is landlord, mint, and court of last resort. It worked as a business and a community — under a single operator who could be all three.
- **Map against our model:** our worlds are **user-hosted** (self-hosting child of #5955): there is no platform landlord. So "real estate" in our model cannot mean platform-issued parcels; it can only mean **authority over a region of a world, delegated by that world's writer set** — land as an attenuated capability bundle (build/enter/transfer rights over a region), revocable and re-delegable under the world's own rules. The Habitat lesson transfers whole: disputes over that bundle will happen inside worlds, so the world's governance child needs the dispute surface, not the platform. The crypto-land lesson is the negative space: when transfer is frictionless and ownership is the product, speculation eats the world; our transfer rights should be world-policy, not platform default.
- **Verdict: adapt** — adopt Habitat's governance-first lesson and Second Life's parcel-as-rights-bundle shape, re-founded on writer-set delegation instead of a platform landlord, **because** user hosting removes the landlord the original design relied on; **reject** platform-issued land and default-frictionless transfer, because both contradict self-hosting and invite the speculation failure mode respectively.

### 3.3 Croquet (and the replicated-world alternative)

- **What it is:** Croquet (David A. Reed, Andreas Raab, David P. Reed et al.): replicated virtual worlds where every peer runs the same deterministic computation over replicated objects, coordinated by reflectors — shared worlds *without* a state authority, by making computation itself replicated and deterministic.
- **Core mechanism:** identical inputs + deterministic execution = identical worlds on every peer; latecomers snapshot and replay. Authority dissolves into the protocol.
- **Map against our model:** Croquet is the strongest version of "p2p identical to hosted" — and it achieves it by demanding determinism from all world code, forever. Our programs are user code under ceilings, not a deterministic closed world; adopting Croquet's requirement would be a programming-model decision far larger than this survey.
- **Verdict: reject** as our world's consistency model, **because** its determinism requirement is incompatible with our user-code and persistence model; **adapt** its snapshot-plus-replay idea for world *join/sync* (a joining host catches up from a snapshot plus committed revisions), which our revision model already resembles.

---

## Part 4 — Adjacent modern work: WASI and the WebAssembly component model

- **What it is:** WASI (WebAssembly System Interface) and the component model: sandboxed components whose entire host access is the set of typed interface instances (capabilities) they are linked with — no ambient syscalls; a component's imports *are* its platform.
- **Core mechanism:** capability-based security as a linking discipline: authority is enumerated in the component's type signature, attenuated by the host at instantiation, and auditable statically.
- **Map against our model:** this is our "pass the platform in, nothing ambient" model, industrialized — and our **compile-time platform ceilings are exactly a component-model import bound**: they state, statically, the maximum import set a program may be linked with. Where classic ocap (§1.1) is purely dynamic, the component model shows the static form is coherent, toolable, and shippable — it is the evidence that our deliberate deviation from classic ocap is a deviation *toward* a proven neighbor, not into the void. Provider enumeration (which providers vend which interfaces) belongs to #5931; the theory point is this doc's.
- **Verdict: adopt** the component model's import-as-platform vocabulary for stating our ceilings, **because** it gives the ceiling concept a standard grammar reviewers already read; coordinate with #5931 rather than duplicating its provider survey.

---

## Synthesis — what each #5955 child takes from this survey

| #5955 child | Takes | From |
|---|---|---|
| Programming-model child ("pass the platform in") | Reference-is-authority discipline; caretaker revocation pattern; ceilings stated as component-model import bounds | §1.1 E; §4 WASI |
| Capability-model child | Attenuation-on-delegation and derivation-tracked revocation as invariants; **open gap: our revocation story for passed-in capabilities must be written** | §1.4 seL4/CHERI |
| Cross-world / protocol work | CapTP semantics (introduction, pipelining) as reference model; near/far reference unification as a goal | §1.3 Cap'n Proto; §2.1 Goblins |
| Identity / invites (#5751) | Sturdy references as the invite baseline; petname discipline as a requirement | §2.2 OCapN; §2.3 petnames |
| Writer-set authority (#5545) | Authority-shaped state stays under writer-set committed order; CRDTs only inside the line | §3.1 CRDTs |
| Self-hosting child | Land/regions as writer-set-delegated capability bundles; no platform landlord; snapshot+replay join | §3.2 Habitat/SL; §3.3 Croquet |
| Shared-editing surfaces | CRDTs for presence, documents, drafts — never for ownership or economy | §3.1 CRDTs |

**The one-sentence version:** capabilities decide *who may act* (E, seL4, WASI), sturdy references and petnames decide *how actors find and name each other* (OCapN, #5751), writer sets decide *what is true* for owned state (#5545), and CRDTs decide *how the rest converges* — and the design errors this survey exists to prevent are all crossings of those four jurisdictions.
