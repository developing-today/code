# Nix cache as the content store: derivations + flakes per item

Date: 2026-11-24 05:12 UTC
Status: design record; no benchmarks run, no implementation authorized
Source issue: [developing-today/code#9521](https://github.com/developing-today/code/issues/9521)
Source brief: [2026-11-13 brief](../2026-11-13T11-42-00Z_design_nix_content_store/brief.md) (Drew, 2026-11-13; PR #9520, merged `31718d6bf1`)
Tracking: [developing-today/code#13873](https://github.com/developing-today/code/issues/13873) (queue packet 21569, slice 1)
Supersedes: none. This record upgrades the #9521 brief to the 9-point sufficient-design standard. It does not replace the brief; where they differ, the brief records Drew's intent and this record records the design decision.

## 1. Problem and evidence

The question (brief §§1–5, issue body): can our Nix cache serve as the content store directly — every file and file-set as a content-addressed derivation, one flake per item (sharded by CA prefix or equivalent), `ISO-date.increment` versioning with fork lineage in the version, and constraint-based resolution (newest allowed version wins; documented divergence where pragmatic)? In scope: trustless-Nix options, pruning to final artefacts per consumer kind, the full target matrix (static/dynamic, VM/container, deb/RPM/Arch/snap/Homebrew/Windows installer, embeddable libraries, roc+ID worlds, Wasm/WASI/browser and Cloudflare variants, Android APK, ratatui), an admin platform surface (retention config, usage statistics, backups), trusted store sharing with ACLs, and per-tenant derivation lists as complete backups (brief §§6–8).

Evidence is bounded and must be read as such:

- The brief is transcribed dictation, not a measurement. Its version-suffix algebra is explicitly unpinned ("the exact suffix algebra is for the design to pin down"); its resolver rules are intent statements, not a spec; its tooling claims (Datadog version constraints, "Trust Nix", recursive-Nix/Nix-trees performance) are pointers to investigate, not established facts.
- No benchmark in this record has been run. The benchmark mandate (brief §5) is the gate: per-item derivation + flake cost (space, `nix eval` overhead, complex-flake behavior) across compiler × store-backend × resolver combinations (Nix, Lix, Tvix, Trust Nix / Trustix, Hydra learnings, nixpkgs practice, mixed pairings including hot-cache-over-long-term-store). Until that data exists, every sizing claim below is a hypothesis with a named test, not a result.
- EPIC GUARD applies: #9521 holds work in itself; subtasks completing never closes it. This record is slice 1 (design to standard); it authorizes no build, no store migration, no production cutover.

## 2. Goals and non-goals

### Goals

- Give the #9521 program a decision-grade design: explicit contracts (naming, versioning, resolution, pruning) that a benchmark slice and later build slices can execute against.
- Preserve Drew's decision rule (brief §5): if measurement shows no significant downside — essentially a wash — keep the design for option value; rip it out later if it hurts.
- Bound the "if Nix loses" exit (brief §9): define what failure looks like before benchmarks run, so a negative result retires the program instead of lingering.
- Keep the first executable work benchmark-first and small: inventory + measurement plan before any store-format commitment.

### Non-goals

- No implementation is authorized by this record: no derivation-per-item migration, no flake-per-item rollout, no store-backend change, no resolver replacement, no consumer cutover.
- No benchmark is claimed as done here; §7 specifies the plan, not results.
- No pinning ofunguessed facts: the exact version-suffix algebra, shard function, constraint-file format, and compiler/store/resolver pairing recommendations are deferred to the benchmark + spec slices with named deciders (§8).
- No remote-consumer or deployment coverage is claimed; repository evidence alone never proves store-wide behavior.

## 3. Alternatives considered

| Alternative | Benefits | Costs / risks | Decision |
|---|---|---|---|
| Do nothing; keep the current cache + separate content store | Zero new risk, zero new overhead; no flake-lock or eval-latency exposure. | Leaves the hypothesized option value (uniform addressing, per-item versioning, portable targets) unexplored; the program question stays open. | Rejected as the program answer, kept as the live default until benchmarks + a build slice prove otherwise. The store does not move on the basis of this design alone. |
| Adopt derivation-per-item + flake-per-item without benchmarking | Fastest path to the target matrix. | Commits to unknown space/eval/lock costs across an untested compiler × store × resolver matrix; risks a painful revert. Directly violates the brief's own benchmark gate. | Rejected. Benchmarks (§7, slice 1) precede any adoption decision. |
| Benchmark-first, then conditional adoption (chosen) | Honors Drew's wash-rule with data; bounds the downside before commitment; produces the pairing matrix the brief asks for. | Benchmark work costs time; mixed-pairing results may be inconclusive and force a narrower re-scope. | **Chosen.** Slice 1 measures; slice 2 specs the contracts; later slices build only on a positive gate. |
| Partial adoption: content-addressed derivations without flake-per-item (single monorepo flake or per-directory flakes) | Avoids flake-proliferation and lock-conflict risk; simpler resolution. | Loses the per-item versioning/sharding benefits the brief centers; lock contention moves rather than disappears. | Deferred as a fallback if benchmarks show flake-per-item cost dominates. Named in slice 1 exit criteria. |
| Non-Nix content store (bucket API + delta/profile format, brief §9) | Purpose-built deltas, compression, tiering; no Nix eval in the read path. | Abandons Nix addressing/portability; a second store to operate; does not answer the brief's question. | Named exit path if the gate fails ("if Nix loses"): the benchmark slice must say so explicitly with numbers. |

## 4. Chosen approach

The program proceeds benchmark-first, contracts-second, targets-last. Nothing below moves bytes in production; it defines what the executing slices must pin down.

### 4.1 Identity and layout (to be pinned by slice 2, bounded here)

- One derivation per item; one flake per derivation as the working hypothesis; sharding by content-addressed prefix (or measured-equivalent) to bound flake resolution time and mitigate flake-lock conflicts (brief §2).
- Standard-library references follow (or pin) the core flake; per-use-case overrides allowed; version constraints live in the flake or a named sidecar file — format chosen by slice 2 after surveying the Datadog-style constraint pointer in the brief.
- Pruning rule (brief §6): build-time inputs that do not land in the final artefact are excluded or copied into place per consumer kind (flake app, NixOS program, home-manager service/system packages/user app, overlay, systemd/launchd unit). A home-manager module and a NixOS module expose required settings and surface module conflicts; conflict behavior is specified, not silently resolved.

### 4.2 Versioning and resolution (intent preserved, algebra deferred)

- Version intent (brief §3): `ISO-date.increment` per change-day, starting at 1, with fork lineage visible in the version (original `.1`, first fork `.1`, first change of first fork continuing the pattern). The exact suffix algebra is **not** pinned here — slice 2 proposes it with collision, ordering, and monotonicity properties, decider per §8.
- Resolution intent (brief §4): unconstrained inputs resolve to the newest version allowed by every constraint, root-down; a single-derivation downgrade from the flake lock is permitted where constraints allow; deliberate divergence (one derivation pinned old while the rest move) is permitted only as a recorded, reasoned exception at the decision site. Slice 2 turns these into a resolver spec with conformance tests, including the brief's cross-compiler demand: same expression → same build or same error across evaluated compilers, with incompatibilities mapped.

### 4.3 Target matrix (demand list, not a commitment)

If the gate passes, one derivation family fans out per consumer (brief §7): Darwin/Ubuntu/NixOS/Red Hat binaries; VM or buildkit containers from scratch or base images; static + dynamic variants; with/without-dependencies and library forms; deb/RPM/Arch-Omarchy/snap/Homebrew/Windows-installer packages; raw files and zip/tar/tar.gz/tar.zstd/zstd streams; Rust/Go-embeddable libraries and DLLs; roc+ID worlds (wrapped/unwrapped, tunic and single-file-trace variants); web/Wasm/WASI/Cloudflare/D1/R2/Turso/IndexedDB variants; Android APK flag-flip; ratatui/SSH terminal forms. Scaling behavior (multi-instance, autoscale, cluster balancing, worker/VPS/cloud spillover) is demand context for the benchmark design, not a promise.

### 4.4 Platform and sharing surface (brief §8, bounded)

- Per-tenant derivation lists generatable as compressed, shippable complete backups (with the database or standalone); a second process may merge derivation lists.
- Trusted store sharing with ACLs/access levels; artefact shapes per need (roc library/platform library, Rust/JS libraries, static/dynamic ID binaries, wrappers that fetch on boot, with/without embedded roc compiler, trusted-key sets for build caches/stores, per-file or per-segment content addressing).
- Admin surface for privileged/delegated users: retention config; precompile/recompile/version/metadata updates; usage statistics (downloads, durations, success/error codes and messages, ID versions, geography, user/execution counts and lengths, players/counts); backups to provided paths; base-derivation generation (Windows installer, standalone site, WASI roc variant); bulletproof-build test loop. All of this is specified by later slices only after the gate; this record authorizes none of it.

## 5. Dependency-ordered implementation slices

Each slice is sized for one lane/session. These are proposed executable slices under #9521; this design enqueues none of them and claims none authorized.

| Order | Slice and dependency | Acceptance criteria |
|---:|---|---|
| 1 | **Inventory + benchmark plan and measurements.** No data dependency. Survey the brief's pointers (Trust Nix/Trustix, recursive Nix, Nix trees, Datadog-style constraints, Hydra/nixpkgs learnings); define the compiler × store-backend × resolver matrix incl. mixed pairings (hot cache over long-term store; split build/resolve/serve toolsets); define workloads (single-item eval, full-chain resolution, complex flake of hundreds of objects, space/compression/tiering). | Dated benchmark receipt with space, per-derivation eval overhead, and complex-flake behavior per pairing; explicit wash/fail verdict against pre-registered thresholds; named fallback (partial adoption or §9 exit) if the gate fails. No store change. |
| 2 | **Contract spec: naming, sharding, version algebra, constraints, resolution, pruning.** Depends on slice 1 (needs the cost bounds). Pins shard function, flake-per-item layout, constraint-file format, exact version-suffix algebra with ordering/monotonicity proofs or tests, resolver semantics incl. downgrade/divergence rules, per-consumer pruning + module-conflict behavior. | Machine-checkable spec + conformance tests (same-expression/same-result-or-error across compilers); worked version-lineage examples incl. forks; reviewer sign-off by the named deciders (§8). Still no production data. |
| 3 | **Pilot: one item family end-to-end on an isolated store.** Depends on slices 1–2. Implements derivation+flake per item for a single small family; exercises version bumps, constraint resolution, divergence recording, pruning, and one packaging target. | Pilot receipt: eval timings, space, lock behavior, and a built artefact for the chosen target; rollback = delete the pilot namespace; production store untouched. |
| 4 | **Target-matrix expansion, one target per slice.** Depends on slice 3. Each slice adds one consumer/target family with its pruning + module surface. | Per-target receipt with build, size, and consumer verification; failures do not block other targets. |
| 5 | **Platform/sharing/backup surface.** Depends on slices 3–4. ACLs, retention config, usage statistics, per-tenant backup lists. | Receipts for sharing, retention, stats, and restore-from-backup on the isolated store; no production ACL or retention change without a separate authorized cutover slice. |

## 6. Rollout and rollback

- Rollout is gated: slice 1 (measure) → explicit wash/fail verdict → slice 2 (spec) → slice 3 (isolated pilot) → per-target slices. No slice moves production store data, cache configuration, or consumers; each slice states its own blast radius (isolated store/namespace only) and its evidence receipts.
- Rollback at every stage is deletion, not repair: remove the pilot namespace/spec draft/benchmark fixtures; the live cache and content store are never the rollback target because they are never touched.
- The §9 exit is a first-class outcome: if benchmarks fail the gate, the program records the numbers, selects partial-adoption or non-Nix-store fallback explicitly, and stops — it does not drift into an unmeasured build.

## 7. Verification plan

For every slice, retain evidence with timestamp and exact code/schema revision:

1. **Fidelity oracle:** quotes and section references trace every brief claim into this record or into a named open question; nothing from brief §§1–10 is silently dropped (see §9 mapping note in record metadata).
2. **Benchmark oracle (slice 1):** pre-registered thresholds for space, per-derivation eval overhead, and complex-flake behavior; per-pairing results; the wash/fail verdict is recomputable from the receipt.
3. **Spec oracle (slice 2):** conformance tests proving same-expression/same-result-or-error across evaluated compilers; version-ordering tests incl. fork lineages; constraint-resolution tests incl. downgrade and recorded-divergence cases.
4. **Isolation oracle (slices 1–3):** proof the production cache/store and consumers were untouched (no config, ACL, retention, or consumer diff).
5. **Scope oracle:** no implementation, migration, cutover, or credential change rides along with a design/benchmark slice; full-repo CI status reported separately, never implied from focused tests.

## 8. Open questions and decisions

1. **Version-suffix algebra — decider: Drew.** The brief's fork-lineage sketch is ambiguous by its own admission. **Default:** slice 2 proposes the exact algebra with ordering/monotonicity guarantees; until Drew approves it, no version string is normative.
2. **Shard function and flake-per-item cost ceiling — decider: Drew on the ceiling; executing lane on the function.** **Default:** CA-prefix sharding as the hypothesis; slice 1 sets the measured ceiling and may force the partial-adoption fallback.
3. **Constraint-file format (flake vs sidecar, Datadog pointer) — decider: Drew.** **Default:** slice 2 surveys and recommends; no format is assumed.
4. **Compiler × store × resolver pairings to bless — decider: Drew.** **Default:** slice 1 maps compatibilities (including one-way compat); no pairing is recommended here.
5. **Wash/fail thresholds for the benchmark gate — decider: Drew.** **Default:** slice 1 pre-registers thresholds before measuring; post-hoc threshold movement is recorded as a decision, not a quiet edit.
6. **§9 exit vs partial adoption if the gate fails — decider: Drew.** **Default:** numbers first; the benchmark receipt names the exit explicitly.
7. **Target-matrix priority order — decider: Drew.** **Default:** slice 3 picks the smallest measurable family; no target order is promised by this record.

## 9. Record, tracking, and acceptance

This dated record is the slice-1 design artifact for #9521, delivered under queue packet 21569 (tracking #13873). It links from #9521 for discoverability; per EPIC GUARD it neither closes #9521 nor claims any benchmark, build, or migration fact. Coverage note: brief §§1–5 map to §§1–4.2 + slices 1–2 above; §§6–8 map to §§4.1–4.4 + slices 3–5; §9 maps to the alternatives table and §6 exit; §10 (store holds wrappers/presets) is carried as the §4.4 artefact-shape list.

**Design acceptance:**

- [x] Problem and evidence, goals/non-goals, honest alternatives including do-nothing, chosen contracts and capability boundary.
- [x] Dependency-ordered, lane-sized slices with individual acceptance criteria.
- [x] Rollout, rollback/recovery limits, verification oracles, and named-decider open questions with defaults.
- [x] Date-stamped `doc/` record tied to #9521 and #13873 queue packet 21569; explicitly distinguishes design evidence from benchmark/build/cutover evidence.
- [ ] After merge, add this document's permanent GitHub link to #9521 and verify the issue comment and document.

### Evidence references

- [Source brief (2026-11-13)](../2026-11-13T11-42-00Z_design_nix_content_store/brief.md)
- [#9521 source issue](https://github.com/developing-today/code/issues/9521)
- [#13873 Lane-6 tracking issue](https://github.com/developing-today/code/issues/13873)
