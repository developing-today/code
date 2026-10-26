# C012: G: global directives and report context

[Main reference](../main.md#c012)

## Relevant transcript excerpts

<!-- workflow:excerpt C012 1 28 -->

[Transcript lines 1-28](../transcript.txt#L1-L28)

```text
# Platform world: state report

**Date:** 2026-10-18
**Scope:** the `id` platform world stack, its example apps, the SSH input
decision and how to implement it, and the open GitHub work.
**Basis:** session context, the companion transcription
[`2026-10-18T00-00-00Z_reference_platform_state`](../2026-10-18T00-00-00Z_reference_platform_state/2026-10-18T00-00-00Z_reference_platform_state.md),
live repo/GitHub checks at 18:36–19:00Z, and four code-level research passes
(SSH I/O path, PR #211 diff, CLI/frame protocol, plans + apps).

Source plans (committed on `origin/main`, `.opencode/plans/`):

- [Phase 11 — foundations](../../.opencode/plans/phase-11-foundations.md)
- [Phase 12 — platform flexibility](../../.opencode/plans/phase-12-platform-flexibility.md)
- [Phase 9/10 — portable platform](../../.opencode/plans/phase-9-portable-platform.md)

> **Checkout note:** the verified state is `origin/main` at `60f7a634`. The
> local `main` ref and the `platform-world-apps-migration` worktree lag it by
> ~145 commits (they lack `Record`/`Replay`/`Measure` and the plan files).
> Facts below are stated against `origin/main` unless noted.

---

## Executive summary

1. **The platform stack is merged into `main`** (#209–#256 plus #210, #212,
   #213, #224). Verified at `60f7a634`: **1988 Rust tests, 0 failures, 36
   suites**; web vitest **349 passed, 9 skipped**; fmt clean.
```

<!-- workflow:excerpt C012 38 41 -->

[Transcript lines 38-41](../transcript.txt#L38-L41)

```text
   with `id world`. No content auto-detection. Long-term, agreed: every
   variation — raw/buffered × primary/alternate, split UI, dual channel —
   lands eventually, each for a different context (§13–§15). §7 is a concrete
   implementation plan grounded in file/line.
```

<!-- workflow:excerpt C012 45 45 -->

[Transcript lines 45-45](../transcript.txt#L45-L45)

```text
5. **Phase 11/12 plans are committed and detailed** — 18 F11 tasks and 18 F12
```

<!-- workflow:excerpt C012 49 128 -->

[Transcript lines 49-128](../transcript.txt#L49-L128)

```text
6. **CI:** only the Firefox E2E check is red on `main` (#229 — leave alone).
   The `id` PR gate is the slowest in the repo: ~116 min average per PR
   (#292), with a hard-coded check list (#300) and intermittent disk
   exhaustion (#257).
7. **Where next (§12):** decide the mode shape → reconcile #211 in three
   steps (keep-stack, then line mode as a separate entry, then optional
   toggle) → Phase 11 order F11.17, F11.18, F11.14, F11.5, F11.10 → CI slice
   (#292/#300/#257) → Phase 12 first slice (F12.4, F12.1, F12.7).

---

## 1. What has landed

Merged as one stack top (#234) plus siblings merged individually with `main`
merged in and tested first.

| PR | Adds | Enables |
|---|---|---|
| #209 | Roc source compilation from the world page | Worlds built from source in the browser |
| #214 | Portable screen presenters (web + SSH) | One structured grid renders identically in browser and terminal |
| #215 | Source-only install policy | Server can refuse raw artifacts, forcing build-from-source |
| #216 | Terminal keyboard input for web worlds | Games playable from the browser with real key events |
| #219 | Per-session viewport and paste | Connections size independently; bounded clipboard paste |
| #220 | Terminal buffer capture interface | Ratatui-style buffers feed the same screen contract |
| #221 | Bounded pure Roc `RocJson` | Apps parse/encode JSON without host-specific shortcuts |
| #224 | LogSync deadline + CI disk cap + `--keep-going` | Tests no longer race 30s; CI reports every check |
| #225 | Module build provenance | Compiler/platform/source digests recorded on install (observation, not attestation) |
| #226 | Per-world resource profiles | Optional requests clamped to server hard ceilings |
| #228 | Durable per-world resource profiles | Profile survives restart; applies on next install |
| #231 | Capability ceilings | Server and per-world ceilings bound which capabilities reach authority |
| #233 | Operator CLI for ceilings and profiles | `id world caps --ceiling`, `id world profile` |
| #234 | Structured RocJson/Screen app migration | `territory`, `lounge`, `roc-grid`, `roc-screen` on shared libraries |
| #239 | Session recording and replay | `id world record` / `replay` fixtures; record-match verdict |
| #243 | Journaled scheduled events | `schedule.at` / `schedule.cancel`, restart-safe, `late:true` on catch-up |
| #244 | Journal refuses unreadable complete lines | Fail-loud rollback instead of silent truncation |
| #246 | Phase 11/12 plans | Foundations and platform-flexibility roadmaps |
| #247 | Per-call fuel metrics and budgets | `id world measure`; budget tests guard regressions |
| #248 | Territory view fits 8 seats | 1.6M fuel (16% of 10M) vs 7.1M before; `Screen.render_rows_json` |
| #256 | Chat visibility + `history` scope | Join-only viewers no longer see chat; `history` opts in to read |

Independents also merged: #210 (signed doc revisions), #212 (home discovery),
#213 (Workers pkarr doc). Resolved and closed: #238 (relay kit, settled by
#249 — vendored under `pkgs/id/tests/fixtures/relay` with a drift test),
#240 (install seed leak, fixed by #250/#256).

## 2. The apps' present state

All eight example dirs sit on the shared platform
(`pkgs/id/examples/roc-world/platform.roc`, freestanding Zig host adapter).

### Shared guest surface

- **Screen contract:** one structured grid; web, SSH and captured buffers
  present the same structure. No guest HTML/CSS/escape sequences.
- **Libraries:** `RocJson`, `RocGrid`, `RocCap`, `RocText` (proven string
  operations only — Unicode casing/trimming deferred to a Roc nightly bump).
  **`RocText` is currently imported by no example** — it exists in the
  platform only (`platform.roc:14`, `roc-world/README.md:99`).
- **`platform.roc`** requires `layout : model, U64, U64 -> Str` (guest-aware
  reflow). `Key.parse` decodes raw terminal input bytes into a `Key` tag union
  (`Char/Ctrl/Enter/Tab/Backspace/Escape/Delete/arrows/Home/End/PageUp/
  PageDown/Unknown`) matching the browser encoder's sequences.
- **Not built:** table/list/meter `Screen` widgets — deferred until two apps
  need the same one.

### The examples

| App | What it is | Shared libs | Notes / gaps |
|---|---|---|---|
| `territory` | Multiplayer 16×8 grid, 8 seats (A–H + cursor), persistent seats; journal replays grid/seats | `Key`, `Screen`, `RocJson`, `RocGrid` | Reference app. Full house = 1.6M fuel (16% of 10M) after #248. `territory.wasm` checked in (`--wasm-memory=8454144` to fit the 16 MiB sandbox) |
| `lounge` | Capability showcase: `players` + `time.tick:5000`, greets arrivals via `chat.say`, refusals counted `denied=N`, polls `time.now` | `RocJson`, `RocCap` | 178 lines |
| `roc-chat` | Minimal chat on `chat.history`: type a line + Enter → `chat.say`; `hist` returns newest 8 lines | `Key`, `RocCap`, `RocJson` | 149 lines; matches the #256 visibility rules |
| `roc-grid` | `RocGrid` demo: 4×3 grid, `set` writes `A` at centre; reports untouched count / cursor cell / OOB read | `RocGrid`, `Screen` | 61 lines |
| `roc-screen` | Portable screen + `Key.parse` demo: counter on `+`, cell grid in a bordered `"counter"` box | `Screen`, `Key` | 47 lines |
| `tic-tac-toe` | Two-player game, digits 0–8, ply parity X/O, pure `update` for exact replay; publishes records | none | **Older pattern:** records JSON hand-built by string interpolation (`main.roc:62`) — no `RocJson` |
| `roc-counter` | Tiny pure counter (`inc` → `count=N`), `records` export | none | **Older pattern:** legacy plain-text view (`main.roc:14`). Needs Zig 0.16 + pinned Roc `nightly-2026-10-04-130536d` |
| `roc-world` | The shared platform + Zig host adapter (`platform.roc`, `Screen/Key/RocJson/RocGrid/RocCap/RocText.roc`) | — | ABI docs require the pinned compiler for `roc_platform_abi.zig` |

Migration leftovers worth doing eventually: `tic-tac-toe` → `RocJson`;
`roc-counter` → `Screen`; adopt `RocText` where a real use appears.
```

<!-- workflow:excerpt C012 358 361 -->

[Transcript lines 358-361](../transcript.txt#L358-L361)

```text
- **Bad:** auto-detect ("if it looks like text, buffer it") — non-deterministic
  and untestable.
- **Good:** explicit, one-way routing. Mode chosen at login (or by a visible
  prefix-key toggle later); mode is visible; exit returns to raw.
```

<!-- workflow:excerpt C012 378 379 -->

[Transcript lines 378-379](../transcript.txt#L378-L379)

```text
Do not auto-detect from content. Do not build two SSH channels until someone
has a real client for it.
```

<!-- workflow:excerpt C012 477 563 -->

[Transcript lines 477-563](../transcript.txt#L477-L563)

```text
## 8. What is done / not done / planned / proposed

### Done

Portable screens (web + SSH + captured buffers); shared Roc libraries;
seven apps on the shared platform (two still on older patterns); source-only
installs; resource profiles and capability ceilings; roles and scopes;
install-seed redaction and chat visibility via one projection (`#250/#256`);
fuel metrics and budgets (`#247`); territory full house at 16% (`#248`);
journaled scheduled events (`#243`); session record/replay (`#239`); journal
fail-loud (`#244`); module provenance (`#225`); operator CLI incl. `record`,
`replay`, `measure` (on `origin/main`); Phase 11/12 plans (committed).

### Not done (open work)

- **#211** — SSH worlds: awaiting the mode decision; reconcilable per §7.
- **CI** — Firefox E2E (#229, leave alone); `id` gate ~116 min/PR (#292);
  hard-coded check list (#300); disk exhaustion (#257); world test flake
  (#190).
- **#227** — journal rollback compatibility: pre-#244 binaries truncate an
  unreadable last line; back up before rolling back.
- **#241** — LogSync: deadline aligned to 60s (#224), but the ~23s bulk
  transfer and the mid-stream reset under `--keep-going` are unexplained;
  #255 (your sweep) is the open fix attempt.
- **#242** — host-originated events (presence, capability results, ticks) not
  retained in `events_after`; `territory` fixtures cannot contain them.
- **#230** — capability ceilings: mechanism landed (#231/#233); issue still
  open — close or scope the remainder.
- **Input modes** — not in either plan; add as a Phase 11 task when landed.

### Planned — Phase 11 (foundations, F11 tasks with exit criteria)

- **CI repair:** F11.1 relay kit (**done** via #249; drift only caught in a
  full checkout); F11.2 Firefox E2E isolation (per-spec-file `id serve
  --ephemeral`, browser restart per file; E2E derivation <10 min, two
  consecutive greens); F11.3 explain LogSync (bulk <10s, 50 consecutive runs
  without retry); F11.4 fast lane (median PR feedback <15 min over ten runs);
  F11.5 injected `Clock` (no `sleep` in `world.rs` tests; schedule tests <1s).
- **Cost visibility:** F11.6 fuel budgets (**done** #247, budget per example,
  fails >25% growth); F11.7 host metrics + tracing (<2% of the F11.8
  benchmark); F11.8 benchmarks (committed baselines; >20% regression reported
  non-blocking); F11.9 bytes-per-frame table (80×24 and 200×60 per example —
  the baseline F12.11 must beat).
- **Structure:** F11.10 split `world.rs` (6,120), `world_session.rs` (4,017),
  `cli.rs` (3,958) — no file >2,500 lines, moves only (`cmd_world` has a
  730 KB stack frame — box large command variants); F11.11 one client
  projection (mostly **done** #250/#256; host events #242 open); F11.12
  journal format management (future-version journal → clear refusal; `backup`
  restores byte-for-byte); F11.13 error taxonomy (every `CapError` documented,
  incl. why `schedule.at` drops).
- **Guest cost:** F11.14 cheaper `Screen` (territory view ≤40% of the fuel
  limit — row strings + style runs); F11.15 modules ≤400 KB with instantiate
  time measured (`wasm-opt -Os` + strip in each `build.sh`); F11.16 replay
  self-check (`id world verify` replays twice, a nondeterministic guest must
  fail); F11.17 a view trap must not poison the world (error screen naming the
  failing call, state kept, retry on next event); F11.18 pinned toolchain for
  checked-in artifacts (the checked-in `territory.wasm` was built by an older
  system `roc` than `nix/roc.nix` pins, costing ~4.2M fuel/view; rebuild all
  examples; `build.sh` refuses a foreign compiler; compare **behavior, never
  hashes**).

### Proposed — Phase 12 (platform flexibility, F12 tasks)

Whole-phase exit: <5 min empty-dir→joinable world; a new capability = provider
+ manifest with **no `world.rs` edits**; ≥3 guest languages pass conformance;
state-preserving upgrade with rollback; 1,000 idle / 100 active worlds per
node (20 ev/s, p99 <50 ms); `world.toml` + module hash reproduces a world;
redraw bandwidth <10% of a full frame; no new authority path outside named
ceilinged providers.

F12.1 declarative `world.toml` manifest + `id world apply`/`diff`; F12.2
forking and templates; F12.3 roles v2 (builder, moderator); F12.4 provider
registry (test provider added without editing `world.rs`); F12.5 first
providers `kv`, `blob`, `http.fetch`, `notify` (bounds/refusal/replay tests;
`http.fetch` refuses unlisted hosts); F12.6 observability/audit (admin frames
→ audit lines, replayable); F12.7 ABI spec + conformance suite
(`plaza_abi_version`); F12.8 second/third guest SDKs (Rust + TS/Zig); F12.9
program upgrade with state migration; F12.10 fair scheduling and quotas
(fuel-spinning world raises no other world's p99 >10%); F12.11 delta and
binary frames; F12.12 many worlds per node; F12.13 journal replication
(tampered stream refused); F12.14 portals (both sides grant); F12.15 `id world
dev` (edit-to-run <3s; fuzz harness); F12.16 `id world doctor` (toolchain,
platform build, ceilings, profile, journal format, clock — each check with a
failing-case test); F12.17 module trust and signing; F12.18 DoS tests.

Sequencing per the plans: Phase 11 first needs F11.1, F11.4, F11.10, F11.11,
F11.12; recommended first Phase 12 slice: **F12.4, F12.1, F12.7**.
```

<!-- workflow:excerpt C012 565 615 -->

[Transcript lines 565-615](../transcript.txt#L565-L615)

```text
## 9. Open GitHub issues (live, 2026-10-18)

### Platform and `id`

| Item | Title | State / note |
|---|---|---|
| PR #211 | feat(ssh): add chat and document viewing to worlds | OPEN, CONFLICTING; four `SshIo` regions; reconcilable per §7 |
| PR #255 | fix(id): coalesce LogSync ingest batches (#241) | Your sweep; left alone |
| #227 | Define rollback compatibility for world profile journal entries | Open; backup before rollback |
| #229 | Stabilize shared-state failures in Firefox E2E CI | Open; leave alone per instruction |
| #230 | Define server and per-world world capability ceilings | Open though #231/#233 landed; close or re-scope |
| #241 | Bulk LogSync test retries a mid-stream reset instead of explaining it | Open; 7 comments; deadline aligned, cause unexplained |
| #242 | Host-originated events are not retained in the event stream | Open; blocks `territory` fixtures with presence/ticks |
| #190 | Serve stalls before printing node in `test_world_module_shared_over_iroh_and_web_presentations` | Open; times out after 30s waiting for the `node:` line; predates Phase 4 |

### CI infrastructure (affects the `id` PR gate)

| Item | Title | Note |
|---|---|---|
| #292 | The `id` workflow gates PRs for ~2 hours — split e2e out and fail fast | Last 4 runs averaged 116 min/PR (max 137), 465 min total; every other workflow ~11 min |
| #300 | PR gate's hard-coded flake-check list can silently drift from `pkgs/id/flake.nix` | New checks get no PR coverage |
| #257 | `nix flake check` intermittently dies with 'No space left on device' | Partially mitigated by `mkCheck` settings |
| #293 | Add `timeout-minutes` caps to every job | |
| #288 | Cache Go build cache by `go.sum` hash | |
| #287 | Consolidate the three Jules workflows into one | |
| #176 | Nix Playwright E2E suite fails after vendoring dependencies | Adjacent to #229 |

### Long tail (open, outside this report's scope)

Phase 2–5 sync/auth/tag correctness (#159–#195: membership fold, grants, tag
claims, sync batch bounds, blob gate wiring, alias display rules, …); mcpx
(#237, #290, #301); Renovate security PRs (#200–#204); a long queue of draft
editor-bot PRs (Palette/Bolt). Recorded here so the backlog is visible.

## 10. Verification and CI state

- `cargo test --all-features --no-fail-fast -- --skip serve_tests`: 1988
  passed, 0 failed, 36 suites (at `60f7a634`).
- `bun run test` in `pkgs/id/web`: 349 passed, 9 skipped. `cargo fmt --check`
  clean.
- Plain `cargo clippy` fails with E0514 (system rustc vs pinned toolchain);
  the rustup 1.97.0 cargo in a separate `CARGO_TARGET_DIR` exits 0 — not a
  code problem.
- `cargo check --all-targets --all-features` needs `web/dist` first
  (`bun install && bun run build`); full `cargo test --lib` needs
  `RUST_MIN_STACK` (`pkgs/id/.cargo/config.toml`) and must run from `pkgs/id`.
- Nix checks: `CARGO_INCREMENTAL=0`, `*_DEBUG=line-tables-only`,
  `nix flake check ./pkgs/id/ -L --keep-going`.
- **Housekeeping found during this research:** local `main` and several
  worktrees lag `origin/main` by ~145 commits. `git fetch` + fast-forward, or
  re-create worktrees from `origin/main`, before building on them.
```

<!-- workflow:excerpt C012 617 637 -->

[Transcript lines 617-637](../transcript.txt#L617-L637)

```text
## 11. Corrections made during this work

1. Said territory's styled view fit all eight seats. It trapped at six or
   more. Fixed in #248 (1.6M fuel full house; the plan also records 78% of
   the limit at two seats pre-#248).
2. Said rollback to a pre-change binary fails loudly. Only non-final
   unreadable entries fail loudly; a final one is truncated. Fixed for future
   binaries in #244; pre-existing ones still truncate.
3. Said join-only participants cannot read chat. They could, via the snapshot
   and `events_after`. Fixed in #256.
4. Said a join-only watcher heard chat live on `main`. It did not. The raw
   broadcast does carry the real seed and chat; #256 pins that.
5. Said PR #211 has a `/chat` command. It does not: plain lines chat
   implicitly; commands are `/input`, `/records`, `/documents`, `/document`,
   `/help`, `/quit`.
6. Said raw keystrokes in alternate mode were `main`'s default. `main` never
   uses the alternate screen (`\x1b[?1049` is absent); it clear-repaints the
   primary screen per view. Alternate screen is an improvement to make.
7. A worktree-based check claimed `record`/`replay`/`measure` do not exist.
   They exist on `origin/main` (`cli.rs:1654/1719/1741`); the worktree was
   ~145 commits stale.
```

<!-- workflow:excerpt C012 639 660 -->

[Transcript lines 639-660](../transcript.txt#L639-L660)

```text
## 12. Where we should go next

**Immediate (decisions + #211):**

1. Decide the mode shape — recommended: raw default + line mode as a separate
   entry (login-time choice), shared grammar (§5–§7).
2. Reconcile #211 in three steps: keep-stack PR → line-mode PR → optional
   toggle. Fix the resize-drop and total-records-cap gaps while moving.
3. Housekeeping: fast-forward worktrees to `origin/main`; close or re-scope
   #230; add an input-mode task to the Phase 11 plan (it is in neither plan).

**Next (Phase 11 order):** F11.17 (view traps), F11.18 (pinned toolchain),
F11.14 (cheaper `Screen`), F11.5 (injected clock); then F11.10 (module split —
before F11.12, and after the open PRs land since they touch those files);
F11.12/F11.13 alongside. In parallel, the CI slice: #292 (fail fast / split
e2e), #300 (check list from `flake.nix`), #257 (disk) — these multiply the
speed of everything else.

**Later:** the optional in-session toggle; `--json` transcript output for
agents; documents as a protocol frame pair; the Roc partial-line surface; then
Phase 12's first slice **F12.4 (provider registry), F12.1 (world manifest),
F12.7 (ABI conformance)**.
```

<!-- workflow:excerpt C012 779 780 -->

[Transcript lines 779-780](../transcript.txt#L779-L780)

```text
## 16. Open questions for you

```

<!-- workflow:excerpt C012 794 829 -->

[Transcript lines 794-829](../transcript.txt#L794-L829)

```text
7. These doc files are uncommitted. Commit them to
   `platform-world-apps-migration` (or a fresh branch off `origin/main`)?

---

## References

- Companion transcription:
  [`doc/2026-10-18T00-00-00Z_reference_platform_state/`](../2026-10-18T00-00-00Z_reference_platform_state/2026-10-18T00-00-00Z_reference_platform_state.md)
- Plans: `.opencode/plans/phase-11-foundations.md`,
  `phase-12-platform-flexibility.md`, `phase-9-portable-platform.md`
  (committed on `origin/main`)
- Key code: `pkgs/id/src/world_ssh.rs` (SSH I/O, `SshIo`, `SshInputFrame`,
  explorer `Typing`), `pkgs/id/src/world_session.rs` (`ClientFrame`:107-258,
  `ServerFrame`:264-362, dispatch 1103-2468), `pkgs/id/src/world.rs`
  (events, `events_after`, journal), `pkgs/id/src/sandbox.rs` (`plaza_update`,
  UTF-8 input rule), `pkgs/id/src/world_net.rs` (`WorldClient`),
  `pkgs/id/src/directory_view.rs` (`parse_line`, `HELP`),
  `pkgs/id/src/commands/world.rs` (CLI verbs, `join` pump),
  `pkgs/id/src/world_presentation.rs` (`render_ansi`).
- Guest surface: `pkgs/id/examples/roc-world/` (`platform.roc`, `Screen.roc`,
  `Key.roc`, `RocJson.roc`, `RocGrid.roc`, `RocCap.roc`, `RocText.roc`),
  `pkgs/id/examples/territory/` etc.
- PR #211 branch `origin/platform-ssh-world` (merge-base `292d3c7a`).
- Transport: `pkgs/id/src/frames.rs` (length-prefixed JSON over Iroh),
  `pkgs/id/src/web/world_ws.rs` (WebSocket), `pkgs/id/Cargo.toml` (`iroh = "1"`,
  mdns/mainline address lookup). ssh3 status: `francoismichel/ssh3`,
  IETF `draft-michel-ssh3-00` (research stage, checked 2026-10-18).
- Decision records in `doc/`: `2026-10-18T04-00-00Z_fix_world_chat_visibility/`,
  `2026-10-17T16-30-00Z_fix_territory_view_fuel/`,
  `2026-10-17T15-30-00Z_feature_world_perf_budgets/`,
  `2026-10-17T15-00-00Z_architecture_next_stage_platform/`,
  `2026-10-16T13-00-00Z_feature_world_apps_structured_migration/`
- Live state checked 2026-10-18: `origin/main` `60f7a634`; branch
  `platform-world-apps-migration` `52c662f2` (stale vs `origin/main`);
  research notes `/tmp/opencode/report-notes-2026-10-18.md`.
```

## Normalized purpose

Stand-in for the global bucket G, which the tool on main lacks. Holds the report's framing, the constraints every concept follows, and the material outside the client concepts: what landed, app state, CI, Phase 11 and 12 plans, open issues, verification and corrections. Not researched at S1.

## Initial thoughts

- Facts are stated against origin/main 60f7a634 (L17-L20); main is now 59ba8cf1 and #211 has merged, so every concept re-checks its claims.
- Decided: raw default plus a separate line-mode entry; every variation lands eventually, each for a different context (L38-L41, L781-L783).
- Never auto-detect the input mode from content (L358-L361); do not build two SSH channels until a real client exists (L378-L379).
- SSH servers in id stay rare and opt-in, and classic SSH is supported indefinitely (L725-L726, L772-L773).
- Outside this package and already filed: the CI slice (#292, #300, #257, #229), Phase 11 tasks (#539-#544, #547, #550-#555), Phase 12 tasks (#534, #556-#560), example migration (#565), journal and LogSync issues (#227, #241, #242, #230).

## Inferred suggestions

- Inference: Most implementation targets share pkgs/id/src/world_ssh.rs, so S7 PRs for the SSH concepts should be serialized.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
