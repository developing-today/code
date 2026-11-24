# ID Program — Full Investigation, Design Completion & Build-Out Prompt

**You are the coordinator of a program-scale investigation and build-out of the ID project** in
`developing-today/code` (`pkgs/id`). This prompt is a **starting point, not a conclusive or final
stopping point**. Work the design end to end, begin implementation wherever you are unblocked, and
when a pass finishes, review what remains, expand the plan, and repeat — until nothing more can be
done with the access and authority you have. Use subtasks and orchestration heavily: launch many
parallel agents (analyze, research, design, document, implement) and keep them pointed at the
frontier. A phase finishing is a signal to start the next one, not to stop and ask.

---

## 1. What ID is (ground truth — verify, don't re-derive)

ID is a local-first, federated, capability-based application platform: content-addressed objects in
a signed event graph, apps defined against a standard capability set, and nodes that are
simultaneously clients and servers with different platform/capability profiles. Governing
principles (from #6304 / #1240 / #6312): **local-first; hostable by anyone; peer-to-peer ≡ hosted;
a hosted instance is a backstop, never a requirement.**

- Program epics: [#2056](https://github.com/developing-today/code/issues/2056) (core build, first
  apps, comms, currency, hosting, notes, chat, boards — 27/29 sub-issues closed; open:
  [#2036](https://github.com/developing-today/code/issues/2036) messaging/relay protocol v1,
  [#2039](https://github.com/developing-today/code/issues/2039) match attestation/audit) and
  [#9106](https://github.com/developing-today/code/issues/9106) (extensible primitives, federation,
  namespaces — Part A core, Part B federation/trust, Part C Atlas-facing).
- There is **no single charter doc**. Design docs live in the repo-root `doc/` archive (~2,900
  timestamped paths; `pkgs/id/doc` and `pkgs/id/docs` symlink there) plus `pkgs/id/README.md`,
  `pkgs/id/ARCHITECTURE.md`, `pkgs/id/WEB.md`. Producing the charter is part of your job (§5).
- Code: 16 crates under `pkgs/id/crates/`; the root `id` crate is the only public surface,
  `id-core` the shared core. `id-currency-registry` and `id-render` are flagged no-dependents in
  ARCHITECTURE.md — resolve or retire that flag as part of the architecture review.
- Shipped today: Iroh P2P file CLI + REPL + collaborative web UI; `id walk` federation-walking
  skeleton; a hosted Cloudflare Worker (per-user Durable Object, bridge store/auth, currency,
  export/import — both halves landed 2026-11-23); an MCP bridge with the reconciled
  `mcp_policy::PolicySession` in id-caps (#7916 closed; #7917 open); invites with the depth-cap
  fix (#13217); BoundaryGate at signup/login; **Roc guest apps as a shipped pattern**
  (tic-tac-toe + 13 examples); a Bevy native shell (step 2a of #12657 landed).
- Scale program #6304, measured: frame cost is **FLAT at ≈958,616 fuel** across 10k/100k/1M
  events (#13085); resident memory ≈**18–22 KB per world blended** at 1x/10x, compaction spike
  89 MiB at 100k entries, no wall (#13086). Open tension: the codified **256-participant cap** vs
  the 320 / 3,200 tier targets (#13083). #13084 (join-time benches) is flag-held; #13087/#13088
  are blocked behind the scale chain.
- Backlog census (2026-11-23, Search API): **208 open `app/id`, 191 `epic/id-program`,
  36 `epic/id-platform`**; of app/id: 44 `status/blocked`, 28 `status/in-review`,
  19 `queue/queued`, 41 `kind/design`. The design backlog is the centre of gravity: Phase-14 core
  topics #6276–#6282 + outlines, the #5955 worlds program (Roc libraries, dynamic user functions,
  presentation assets), the #567x cluster (wire format, currency scaling, knowledge layer, Bevy
  incorporation #5781, ESP32 #5783), federation/trust designs #9129–#9148.
- Blocker chains (heads first): scale chain (§above); store conformance #9082/#9110 (slices
  #10191 + #10213 closed — the only remaining step is #9082's adopt/defer/reject call on the
  fixture's `reserved` IndexedDB entry); #6377 → #6381; #10237 → #10252 → #10269 (implementation
  landed for #10237; its close is flag-held); hosted worlds #10693 → #10697 (all blocked);
  discovery #10706/#10708/#10712 (blocked); jurisdiction #3399 → #3400–#3404 → #5528/#9935;
  Atlas D1 chain #13100 → #13101 → #13105 → #13107 (#13106 flag-held). **Corrections to common
  assumptions:** #9084 is a pm-stack issue, not an ID chain head; the PM-orchestration DAG
  (#9198–#9513) does not touch ID except operationally (task 16081 / #9455 dep-blocked on #9200).
- Platform priority (Drew, 2026-11-22): **Linux → Android → Web → macOS → Windows**; mobile
  native adopted, high priority; per-platform releases with granular tags.
- Atlas relationship (stated): [#6062](https://github.com/developing-today/code/issues/6062)
  (ID-graph publication format design, open) + unstarted projectors #11182–#11186 + #9106 Part C
  [#9109](https://github.com/developing-today/code/issues/9109) (storage economics, ID search,
  recommendations). "Atlas baked into ID" is **this program's direction to design**, not a
  recorded decision — design it, don't assume it.

## 2. Research seeds (Drew's threads — start here, go further)

- **Falcor / BLAS further development** — Falcor's virtual JSON Graph, path addressing
  (`get`/`set`/`call`), normalized cache. The library is dormant (v2.x, years without releases):
  steal the *model* for native graph navigation, add typed querying as a view layer, do not adopt
  the library. BLAS-class numerics belong host-side: faer (decompositions), nalgebra
  (game/geometry), ndarray (interchange), fastembed-rs/lancedb for embeddings — exposed to apps
  as capabilities, never linked into pure app definitions.
- **App platforms; new interfaces and I/O equivalencies across platforms** — define what an "app"
  is on each tier (CLI, TUI, GUI, web, Android, embedded/ESP32) and the equivalency rules that let
  one app definition render/interact on all of them.
- **Roc** — the platform model is an unusually exact fit: *the platform supplies all I/O, so a
  platform literally is a capability set.* Define the Roc-side API once; implement hosts per
  target; the same pure app runs everywhere; purity is syntactic (effectful hosted functions carry
  `!` in the new compiler); hosts can meter/refuse each hosted call — a natural fuel hook.
  Read all of Luke Boswell's work:
  - https://www.reddit.com/r/roc_lang/s/3nfBm2CheC (could not be fetched in pre-research — read it
    first-hand if reachable; it most plausibly announces roc-native-android)
  - https://lukewilliamboswell.github.io/roc-ray-experiment/ and
    https://lukewilliamboswell.github.io/roc-native-android/ and the whole blog
    https://lukewilliamboswell.github.io/ (multi-entrypoint hosts + roc PR #8214; Action-State GUI
    with zig + raylib; roc-pga2d physics; basic-cli TUI text editor; fullstack Roc + htmx ×3)
  - **roc-ray** (release 0.12.1, featured on roc-lang.org examples; init!/update!/render! model),
    **roc-tui**, **roc-gui**, **roc-signals** (reactive engine; the thing ported to Android),
    **roc-native-android** (Zig JNI bridge + Kotlin host, APK scaffolding; author-stated gaps,
    emulator-only — but a real existence proof for the Android tier), roc-awesome,
    Hasnep's roc-packages index.
  - Caveats to design around: Roc is pre-1.0, compiler rewritten in Zig, two dialects in the
    wild, most platforms single-maintainer experiments. ID's Roc layer must be insulated from
    compiler churn (pin, abstract, or vendor the boundary).
- **Rust + Roc libraries to improve the id binary and id-as-a-library** — audit every crate
  boundary: what should be a reusable library (storage, caps, graph, wire, fuel, render) vs
  binary-only. The end state: other apps embed ID pieces as libraries (see MCP, §3).
- **A standard capability set tied to Roc libraries** — one capability catalogue, programmed
  against once, implemented by every host (Rust native, web/WASM, Android, ESP32). Each
  capability: name, signature, purity class, fuel cost model, failure modes, per-platform notes.
- **Pure vs impure app definitions** — formalize the split: pure definitions (deterministic,
  replayable, fuel-metered) vs impure/hosted effects; how the split shows up in the type system,
  the wire format, and the permission prompt a user sees.
- **Tokens: a generic trading/market system; tokens tied to app fuel and in-app items** — prior
  art: Agoric ERTP (Mint/Issuer/Brand/Purse/Payment with framework-enforced conservation) + Zoe
  (escrow, offer safety, metering); Wasmtime `consume_fuel` as the deterministic metering
  primitive beneath any fuel token. Convergent shape: metered execution → conserved asset
  objects → capability-held, attenuable/revocable budgets — one design unifying app fuel,
  in-app items, and trades. Build on the existing currency work (#2056 currency sub-issues,
  `id-currency-registry`, hosted-worker currency) — do not start a second currency design.
- **Sprites and basic text/2D games** (Dwarf Fortress, Space Invaders, Dig Dug), **basic
  simulations, sketching, mind mapping** — the first-app suite that proves the platform: each is
  a design + a working app, exercising sprites, text grids, input, timing, persistence.
- **Visualizing and searching the ID object graph** — Cytoscape.js (analysis, algorithms,
  compound nodes) vs Sigma.js v3 + Graphology (WebGL scale); internal model = property graph,
  interchange = JSON-LD at the boundary. Search = the Atlas thread (§3).
- **Embeddings for search — the Atlas project baked in** — derived-index pattern: index keyed by
  content hash + `model_id`; query → KNN → graph expansion → rerank/RRF fusion; the index is
  rebuildable, never a source of truth. Storage: libSQL/Turso native vectors (`F32_BLOB` +
  DiskANN `vector_top_k`) — sqlite-vec can't load on remote sqld. Local models via fastembed-rs
  (ONNX) or candle. Coordinate with the Atlas program prompt — the two programs meet here.
- **Bevy platform support and ECS for complex simulations** — `bevy_ecs` is usable standalone;
  headless server = `bevy` with `default-features=false` + MinimalPlugins + ScheduleRunnerPlugin.
  ECS consumes projections of the graph; it never owns the graph. Build on #5781 and the landed
  Bevy shell (#12657 step 2a).
- **Cleaner server/client capability separation** — one node binary, capability profiles:
  what a pure rendering client holds vs a storage/signing node vs a hosted backstop.
- **Full OCapN and CRDT support** — OCapN (Spritely: CapTP, Syrup, `ocapn://` sturdyrefs,
  netlayers) is *pre-standardization and protocol-unstable*, with Guile/Racket reference
  implementations and no production Rust implementation found: adopt the **semantics** (promise
  pipelining, distributed GC, certified third-party handoffs) into ID's Rust cap layer; track the
  standard, don't block on it. CRDTs (Loro — Fugue text, MovableTree, full history DAG, closest
  to event sourcing; Yrs — Yjs ecosystem/wire compat; Automerge — git-like change DAG, per-change
  attribution) sign nothing and enforce no capabilities: they sit **inside** ID's signed-event
  envelope, for content convergence only. Decide per content class which engine (or ID's own log)
  applies.
- **Distinguish the major components**: pure clients rendering the graph; content-addressed
  storage; signing; the **iroh-native approach** (dial-by-public-key QUIC; iroh-blobs = BLAKE3
  CAS transfer; iroh-gossip = live notification only, no catch-up; iroh-docs = signed KV,
  NamespaceId as write capability, range-based set reconciliation — the closest existing shape
  to ID's store; weigh adopt-vs-mirror per component).
- **Text/property objects as structured data; binary formats (images/video)** — object model
  completion: typed properties, schemas, migrations; media objects with streaming, thumbnails,
  and capability-gated decode.
- **MCP, and using each part as a library in other apps** — MCP servers sit *behind* ID's
  capability layer (MCP's own auth is young; tool hints are untrusted; calls arrive holding a
  capability). Every major component ships as a library with a stable API; the binary is one
  composition of them.
- **One distributed federated graph** — servers and clients are technically the same thing with
  different platforms/capabilities. Any design that forks "the server version" from "the client
  version" of a concept is wrong; fix the design.

## 3. Workstreams (run in parallel; each owns its issues end to end)

- **A. Charter & architecture** — write the missing charter (purpose, principles, component map);
  reconcile ARCHITECTURE.md with the code as it is; produce the core architecture diagrams
  (mermaid in the book; SVG/graphviz where mermaid can't carry it): component map, event/object
  lifecycle, capability flow, federation topology, app execution (pure core ↔ hosts).
- **B. Design completion & new designs** — review and organize **all** existing designs in the
  `doc/` archive and the 41 open `kind/design` issues; touch up and improve every one (current
  terminology, links, status, open questions resolved or explicitly parked); create the missing
  designs for the obvious next steps — v1 ideas taken to a **fully featured platform, not an
  MVP**: identity & key recovery, namespaces, permissions UX, offline/merge semantics per
  content class, media pipeline, app distribution & updates, developer tooling, observability.
- **C. Capability platform & Roc** — the standard capability catalogue (§2); the Roc platform
  definition + host matrix (Linux first, Android second per priority); I/O equivalency rules;
  pure/impure formalism; fuel metering per capability.
- **D. Data, graph & search** — object/property model completion; graph visualization &
  navigation (Falcor-model path API); Atlas integration design (#6062 + projectors #11182–#11186
  + #9109 Part C) with the derived-index pattern; CRDT-per-content-class decisions.
- **E. Tokens & fuel economy** — ERTP/Zoe-shaped asset design on Wasmtime fuel metering;
  market/trading mechanics; in-app items; budgets as capabilities; abuse/Spam economics.
- **F. Clients, games & simulations** — mockups/sketches (ASCII + rendered) of how the system
  looks now and how next steps will look; the first-app suite (text/2D games, sims, sketching,
  mind mapping) as designs + working apps; Bevy ECS simulation tier; sprite/text-grid rendering
  across TUI/GUI/Android hosts.
- **G. Federation & protocols** — OCapN semantics adoption plan; wire format (#567x); relay/
  messaging v1 (#2036); attestation/audit (#2039); iroh component decisions; hosted backstop
  parity proofs (p2p ≡ hosted, tested).
- **H. Implementation** — begin every unblocked slice the designs unlock, in dependency order;
  small PRs, verified, merged. Designs that can't be implemented yet get a filed, linked
  implementation issue with a named next action — nothing sits silently.

## 4. Deliverables (the program is done when these exist and stay current)

1. **A comprehensive document series in book form** — an ID book built with the **bookwright**
   method (survey before structure; coordinator-fixed skeleton; mechanical coverage proof;
   zero-warning build), joining the existing code-repo Pages deploy under
   **https://developing-today.github.io/code/id/** (one Pages site per repo — never a standalone
   deploy). Sources in `pkgs/id/book/` + `pkgs/id/book.toml`, built by a flake package like
   `pkgs/fleet/docs.nix`. Include a **DOC-01-style docs-sync contract** in `pkgs/id/README.md`:
   any crate/capability/protocol change updates its chapter in the same change. The fleet book
   (https://developing-today.github.io/code/fleet/) is the form reference.
2. **An issue-by-issue map of ALL ID issues** — every open `app/id` / `epic/id-program` /
   `epic/id-platform` issue, prioritized in best working order, **with all blockers marked both
   on the issues themselves and in the priority list** (a chapter in the book + the tracking
   issue kept current). Blockers are worked, not just listed: when a blocker is itself workable,
   it goes first.
3. **Many designs** — completed, improved, and new (§3B/§3C/§3E), each landed in the `doc/`
   archive by PR and linked from the book.
4. **Many PRs merged** — implementation slices (§3H), docs, and the book itself; CI runs on main
   only; merge when verified.
5. **Diagrams & mockups** — architecture diagrams (§3A) and UI/interaction mockups for current
   state and next steps (§3F), embedded in the book.

## 5. How to work (fleet rules — binding)

- GitHub via the **gh CLI / REST only** (the GitHub connector is banned). File a GitHub issue in
  the owning repo the same pass for any problem found; naming it in a doc is not a substitute.
- Execution on the **laptop** (`~/git/code` primary stays on main + clean; all work in worktrees
  on branches, landed by PR). Never `git stash` in shared clones. Never force-push main.
- Before implementing any issue: premise check (is it still true on current main?) +
  singular-ownership check (fresh reads; a claim comment from another agent is a hard stop).
  Re-read an issue's newest comments immediately before merging its PR.
- Builds/tests dispatch-first (`fleet queue dispatch`), laptop fallback; a container environment
  failure is not a test result — record which route verified. Tests name their oracle.
- Evidence standard: every factual claim carries its checked source or is labelled unverified;
  counts computed from a named source, never estimated in prose.
- Respect standing holds: items flagged as awaiting Drew's item-specific approval are **not**
  yours to start (the scale chain's #13084 is one). Items needing Drew's decision get surfaced
  with the decision's substance (options, ramifications, recommendation) — never re-ask a
  decision he already recorded.
- Secrets by name only, never values. Never touch the `vertexinc` org. External communications
  (anything outside our own repos/channels under Drew's identity) need per-item pre-approval.
- Quota doctrine: plan quotas only, never funds; a 402 is a wall; burn-by-expiry; measure before
  large burns.

## 6. The iteration rule

This prompt is the **starting point**. When every workstream has reported, do not conclude:
re-survey (issues, designs, code), reconcile what the passes learned, expand the design set and
the issue map, and run the next pass. The program ends only when a full pass finds nothing more
that can be designed, documented, or implemented with the authority available — and even then,
the book, the issue map, and the docs-sync contract remain as the living record the next pass
starts from.
