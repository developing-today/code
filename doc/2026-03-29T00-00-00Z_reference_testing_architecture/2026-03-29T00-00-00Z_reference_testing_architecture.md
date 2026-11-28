# Testing Architecture

Comprehensive reference for the `id` project's multi-layered test infrastructure: what each test set covers, how to run it, where it runs, and browser/environment compatibility.

## Quick Reference

```bash
# ─── Developer workflow (in nix dev shell) ────────────────────────
just test-unit              # Rust unit tests only (~500 tests, fast)
just test-int               # Rust integration tests (~85 tests)
just test                   # All fast tests (Rust + TS unit + typecheck)
just test-rust              # All Rust tests (unit + integration)
just test-web-unit          # TypeScript unit tests (bun test)
just test-web-typecheck     # TypeScript type checking
just test-e2e               # Playwright E2E — Chromium + Firefox (146 tests)
just test-e2e-chromium      # Playwright E2E — Chromium only (73 tests)
just test-e2e-firefox       # Playwright E2E — Firefox only (73 tests)
just test-nix               # nix flake check (27 checks — runs everything)
just ci                     # CI check suite (lint + test + build, no E2E)
just check                  # fix + ci (auto-fix then verify)

# ─── Nix checks (reproducible, sandboxed) ─────────────────────────
just test-nix                       # nix flake check — all 27 checks (includes everything)
just check-test-e2e                 # Playwright E2E (Firefox in sandbox)
just test-nixos-e2e                 # NixOS VM: Chromium --dump-dom DOM test (NOT Playwright)
just test-nixos-serve               # NixOS VM: HTTP API test (curl, no browser)
just test-nixos-integration         # NixOS VM: full integration tests including serve_tests
just test-nixos-playwright-e2e      # NixOS 4-VM Playwright (both browsers, ~3 min)
just check-one <name>               # Any individual check (nix build .#checks.x86_64-linux.<name>)

# ─── Without entering dev shell ───────────────────────────────────
# Every just recipe is also available as: nix run .#<recipe-name>
# Example: nix run .#test-e2e, nix run .#ci
```

---

## Test Layers

The project has six distinct test layers, from fastest/narrowest to slowest/broadest:

### 1. Rust Unit Tests (~500 tests)

**What:** Pure logic tests embedded in each Rust source file (`#[cfg(test)] mod tests`).

**Where defined:** Bottom of each `.rs` file in `src/`.

**What they test:**
| File | Tests | Coverage |
|------|-------|----------|
| `src/cli.rs` | 93 | CLI argument parsing, command routing, help text |
| `src/tags.rs` | 63 | Tag CRUD, alpha/omega dual-index, search syntax |
| `src/tuple.rs` | 42 | Sort-preserving binary key encoding/decoding |
| `src/discovery.rs` | 33 | Peer discovery protocol, node ID parsing |
| `src/protocol.rs` | 29 | P2P request/response serialization |
| `src/web/templates.rs` | 29 | HTML template rendering, escaping |
| `src/commands/find.rs` | 26 | Pattern matching, glob expansion |
| `src/repl/input.rs` | 26 | Shell preprocessing: pipes, subshells, heredocs |
| `src/repl/runner.rs` | 23 | REPL command dispatch, help system |
| `src/web/content_mode.rs` | 20 | Content type detection (text, markdown, image, etc.) |
| `src/web/markdown.rs` | 20 | Markdown → HTML rendering |
| `src/web/collab.rs` | 19 | WebSocket collaboration server logic |
| `src/web/routes.rs` | 19 | Axum HTTP handler logic |
| `src/helpers.rs` | 16 | Parsing and formatting utilities |
| `src/lib.rs` | 12 | Library exports, node bootstrap |
| `src/web/assets.rs` | 8 | Static asset serving, content types |
| Other files | ~50 | Various (put, get, list, peers, tag, serve, mod) |

**How to run:**
```bash
just test-unit              # cargo test --all-features --lib
just check-one test-unit    # Sandboxed (nix build .#checks...test-unit)
```

**Runtime:** ~5–10 seconds.

---

### 2. Rust Integration Tests (~85 tests)

**What:** CLI-level integration tests that exercise the full command pipeline.

**Where defined:** `tests/cli_integration.rs`

**What they test:**
- File operations: put, get, find, list, cat, show, peek
- Tag operations: set, delete, list, search with structured query syntax
- Remote operations: node ID detection, client/server protocol
- REPL: command dispatch, pipe handling, subshell expansion
- Error handling: invalid arguments, missing files, permission errors

**Network requirement:** The `serve_tests` subset requires network access (bind/listen on loopback). These are **skipped in the sandbox** `test-int` check but **run in the VM** via `nixos-integration` — so `just test-nix` (`nix flake check`) covers everything.

**How to run:**
```bash
just test-int               # cargo test --all-features --test cli_integration (includes serve_tests)
just test-int-sandbox       # Same but skip serve_tests (for sandbox environments)
just check-one test-int     # Sandboxed, skips serve_tests (nix build .#checks...test-int)
just test-nixos-integration # VM: runs serve_tests (nix build .#checks...nixos-integration)
```

**Runtime:** ~15–30 seconds.

---

### 3. TypeScript Unit Tests (~116 test assertions)

**What:** Bun-native unit tests for frontend TypeScript code.

**Where defined:**
- `web/src/cursor-utils.test.ts` — 76 test/it blocks (cursor position calculation, selection ranges)
- `web/src/editor.test.ts` — 40 test/it blocks (ProseMirror editor initialization, state management)

**What they test:**
- Cursor position calculation across ProseMirror document nodes
- Selection range handling for collaborative editing
- Editor state initialization and document model

**How to run:**
```bash
just test-web-unit          # cd web && bun test
just check-one test-web-unit  # Sandboxed (nix build .#checks...test-web-unit)
```

**Runtime:** ~2–3 seconds.

---

### 4. TypeScript Type Checking

**What:** Full TypeScript type checking via `tsc --noEmit`.

**Where defined:** `web/tsconfig.json`

**How to run:**
```bash
just test-web-typecheck     # cd web && bun run typecheck
just check-one test-web-typecheck  # Sandboxed (nix build .#checks...test-web-typecheck)
```

**Runtime:** ~3–5 seconds.

---

### 5. Playwright E2E Tests (38 tests × 2 browsers = 146 total)

**What:** Full browser automation tests against a running server.

**Where defined:** `e2e/tests/basic.spec.ts` (19 tests) and `e2e/tests/websocket.spec.ts` (19 tests)

**Configuration:** `e2e/playwright.config.ts`

**Architecture:**
- Each browser project (Chromium, Firefox) gets its own ephemeral server instance on a different port to avoid shared state
- Chromium server: port 4173
- Firefox server: port 4174
- Server command: `target/debug/id serve --web --port <PORT> --ephemeral`
- Tests run sequentially within each project (`fullyParallel: false`, `workers: 1`)
- 1 retry on failure (2 retries in CI)

#### basic.spec.ts — Web UI Fundamentals (19 tests)

| Test Group | Tests | What It Covers |
|-----------|-------|----------------|
| **Home Page** | 7 | Page title, file list card, new file form, search input, show-deleted checkbox, theme toggle in footer, empty state |
| **File Creation** | 5 | Create file → navigate to editor, editor has rename/copy buttons, tag panel, ProseMirror editor element |
| **Navigation** | 2 | Editor → file list navigation, created file appears in list |
| **Theme** | 2 | Default `data-theme="sneak"` attribute, theme switcher buttons on editor page |
| **Editor Features** | 3 | Save button, download dropdown, editor container data attributes |

#### websocket.spec.ts — WebSocket + Real-time Collaboration (19 tests)

| Test Group | Tests | What It Covers |
|-----------|-------|----------------|
| **WS Connection + Editor Ready** | 4 | Editor status shows "connected" after WS handshake, ProseMirror is interactive after WS init, connecting state shown initially, initial document received via WS Init message |
| **WS Disconnect + Reconnect** | 4 | Detects disconnect → shows "disconnected" status, automatic reconnect, editor functional after reconnect, survives multiple disconnect/reconnect cycles |
| **Tag WS Live Updates** | 3 | Tag added via API appears without reload, tag removed disappears without reload, tag changes for different file don't affect current editor |
| **Editor Typing + Save** | 3 | Type into ProseMirror, save file with content persistence, Ctrl+S triggers save |
| **Error Recovery** | 2 | Editor recovers from WS error event, clean disconnect (code 1000) doesn't trigger reconnect |
| **Multi-User Collab** | 3 | Two tabs open same file simultaneously, edits from one user appear in other's editor, bidirectional editing works |

#### Fresh Browser Fixture (websocket.spec.ts only)

Firefox's WS upgrade hangs after ~54 prior tests in the same browser process (the basic.spec.ts suite runs first alphabetically). The `websocket.spec.ts` file overrides Playwright's worker-scoped `browser` fixture to launch a **fresh browser instance**, bypassing the degraded networking stack. Cost: ~1–2 seconds launch overhead. All tests in the file automatically use the fresh browser via Playwright's fixture inheritance (no individual test code changes needed).

#### IS_NIX_BUILD Detection

```typescript
const IS_NIX_BUILD = !!process.env.NIX_BUILD_TOP;
```

When `NIX_BUILD_TOP` is set (automatically by nix inside the build sandbox):
- `projects = [firefoxProject]` — Chromium is excluded entirely
- `webServers = [firefoxServer]` — only Firefox's server starts
- Offline flags added: `--no-mdns --no-relay --no-gossip`

When NOT set (dev machine, `just test-e2e`):
- Both Chromium and Firefox projects run
- No offline flags

**How to run:**
```bash
# Developer (both browsers, 146 tests)
just test-e2e                   # Builds binary first, then runs Playwright
just test-e2e-chromium          # Chromium only (73 tests)
just test-e2e-firefox           # Firefox only (73 tests)

# Nix check (Firefox only, sandboxed, 78 tests)
just check-test-e2e                 # nix build .#checks.x86_64-linux.test-e2e
```

**Runtime:** ~1.5 minutes (both browsers), ~45 seconds (single browser).

---

### 6. NixOS VM Integration Tests

**What:** Full NixOS virtual machine tests that validate the complete deployment stack: systemd service management → Iroh node → Axum web server → browser rendering. These use `pkgs.testers.runNixOSTest` to spin up a real NixOS VM with KVM.

**Requires:** Linux with KVM support, ~2GB RAM per VM.

#### nixos-serve — HTTP API Validation (~15 assertions)

**Where defined:** `nix/tests/serve-test.nix`

**What it tests:**
1. **Boot & readiness:** systemd unit starts, port opens
2. **Home page:** HTML contains "Files" heading
3. **Static assets:** `/assets/manifest.json` served correctly
4. **File CRUD via API:**
   - `POST /api/new` — create file, verify JSON response with hash
   - File appears in list HTML
   - File accessible by name (`/file/hello.txt`) and hash (`/edit/<hash>`)
5. **Save content:** `POST /api/save` with ProseMirror doc format
6. **Rename:** `POST /api/rename` — verify response, renamed file accessible
7. **Copy:** `POST /api/copy` — both files exist
8. **Delete:** `POST /api/delete`
9. **Blob content:** `/blob/<hash>` returns saved content ("Hello, NixOS!")
10. **Health:** server still responds after all operations

**No browser used.** All tests use `curl` for HTTP and `json.loads` for response parsing.

#### nixos-e2e — Browser DOM Rendering (~10 assertions)

**Where defined:** `nix/tests/e2e-test.nix`

**Important: This is NOT Playwright.** It uses `chromium --headless --dump-dom` which renders the page (including JavaScript execution), then prints the final DOM as HTML to stdout. The test parses this HTML with string matching. There is **no interactivity** — no clicking, no typing, no WebSocket connections, no page navigation, no multi-tab scenarios.

**What it tests (using `chromium --headless --dump-dom`):**
1. Home page renders with "Files" heading and `new-file-name` form
2. Created file appears in rendered file list
3. Editor page has `editor` element and filename
4. Edit-by-hash page has editor element
5. JS-dependent UI elements rendered (rename/copy buttons)
6. Default theme (`sneak`) applied in `data-theme` attribute

**Browser:** Chromium only, via `chromium --headless --disable-gpu --no-sandbox --dump-dom`. This validates JS execution in a real browser but is NOT interactive (no clicks, no WebSocket, no typing).

**How to run:**
```bash
just test-nixos-serve           # nix build -L .#checks.x86_64-linux.nixos-serve
just test-nixos-e2e             # nix build -L .#checks.x86_64-linux.nixos-e2e
just test-nixos                 # All 4 VM tests (serve + e2e + playwright + integration)

# Also runs as part of:
just test-nix                   # nix flake check — all 27 checks including VM tests
```

**Runtime:** ~3–5 minutes each (VM boot + test execution).

#### nixos-playwright-e2e — Full Interactive Browser Tests (146 tests)

**Where defined:** `nix/tests/playwright-e2e-test.nix`

**What it is:** A 4-VM NixOS test that runs the **complete** Playwright E2E suite (all spec files, both browsers) inside NixOS virtual machines. This is the only fully hermetic, reproducible way to run interactive Chromium Playwright tests — the nix build sandbox blocks Chromium, but VMs have a full Linux kernel with no restrictions.

**Architecture:**

```
┌─────────────────────────────────────────────────────────┐
│                   Virtual Network                        │
│                                                         │
│  ┌─────────────────┐     ┌─────────────────┐            │
│  │ chromium_server  │     │ firefox_server   │           │
│  │ id serve :4173   │     │ id serve :4174   │           │
│  │ systemd service  │     │ systemd service  │           │
│  └────────┬────────┘     └────────┬────────┘            │
│           │                       │                      │
│  ┌────────┴────────┐     ┌────────┴────────┐            │
│  │ chromium_client  │     │ firefox_client   │           │
│  │ Playwright +     │     │ Playwright +     │           │
│  │ Chromium browser │     │ Firefox browser  │           │
│  │ 4GB RAM, 2 cores │     │ 4GB RAM, 2 cores │           │
│  └─────────────────┘     └─────────────────┘            │
└─────────────────────────────────────────────────────────┘
```

| VM | Role | Details |
|----|------|---------|
| `chromium_server` | `id` service on :4173 | systemd-managed, ephemeral, P2P disabled |
| `firefox_server` | `id` service on :4174 | systemd-managed, ephemeral, P2P disabled |
| `chromium_client` | Runs Chromium Playwright | 4GB RAM, 2 cores, Node.js + Playwright |
| `firefox_client` | Runs Firefox Playwright | 4GB RAM, 2 cores, Node.js + Playwright |

**What it tests:**
- The full Playwright test suite (basic.spec.ts + websocket.spec.ts) against both browsers
- Server-client communication across VMs over the virtual network
- WebSocket collaboration: connections, ProseMirror steps broadcast, cursor sharing
- The nix-packaged binary running as a systemd service
- All the same things `just test-e2e` tests, but hermetically in VMs

**What enables this:**
- `playwright.config.ts` has a **VM test mode** (`PLAYWRIGHT_VM_TEST=1`): enables both browsers, disables local webServer (servers are on separate VMs), uses `CHROMIUM_BASE_URL`/`FIREFOX_BASE_URL` env vars for cross-VM URLs
- `e2eTestRunner` nix derivation: pre-built `e2e/` directory with bun dependencies resolved offline, copied to a writable `/tmp/e2e` at test time
- `playwrightBrowsers`: nix-provided browser binaries (from `playwright-driver.browsers`)
- Chromium uses `--no-sandbox --no-zygote --disable-dev-shm-usage --disable-gpu` flags (set via `CHROMIUM_NIX_ARGS` in config)

**Test flow:**
1. All 4 VMs boot and join the virtual network
2. Wait for `id.service` on both servers, verify HTTP reachable via `curl`
3. Copy `e2eTestRunner` to writable `/tmp/e2e` on each client VM
4. Run `node node_modules/@playwright/test/cli.js test --project=chromium` on chromium_client
5. Run `node node_modules/@playwright/test/cli.js test --project=firefox` on firefox_client
6. 10-minute global timeout (actual runtime: ~170 seconds)

**How to run:**
```bash
just test-nixos-playwright-e2e  # nix build .#checks...nixos-playwright-e2e

# Also runs as part of:
just test-nix                   # nix flake check — all 27 checks
```

**Runtime:** ~3 minutes (VM boot: ~15s, Chromium tests: ~58s, Firefox tests: ~94s).

**Note:** Playwright stdout is only visible if a test fails (`must_succeed` captures it). Server-side collab activity logs (WebSocket connections, steps broadcast, cursor events) are always visible in `nix log`.

---

## Browser Coverage Matrix

Each cell shows: ✅ works / ⛔ skipped / ❌ crashes / — not applicable

| Test Set | `just test-e2e` | `just check-test-e2e` (sandbox) | `just test-nixos-e2e` (VM) | `just test-nixos-playwright-e2e` (4-VM) |
|----------|-----------------|--------------------------------|----------------------------|-----------------------------------------|
| basic.spec.ts (19 tests) | ✅ Chromium ✅ Firefox | ⛔ Chromium ✅ Firefox | — | ✅ Chromium ✅ Firefox |
| websocket.spec.ts (19 tests) | ✅ Chromium ✅ Firefox | ⛔ Chromium ✅ Firefox | — | ✅ Chromium ✅ Firefox |
| nixos-e2e DOM dumps (~10) | — | — | ✅ Chromium (`--dump-dom`, not interactive) | — |
| **Total per-browser** | **38 + 38 = 76** | **0 + 38 = 38** | **~10 Chromium** | **38 + 38 = 76** |
| **Grand total** | **146** | **78** (Firefox) | **~10** | **146** (Chromium + Firefox) |

### Why Chromium Fails in Nix Build Sandbox

Chromium's multi-process architecture relies on kernel features that the nix build sandbox restricts:
- **Namespaces:** Chromium spawns sandboxed renderer processes using user namespaces. Nix sandbox already uses namespaces, creating conflicts.
- **Process management:** Chromium's zygote process forks renderers. The sandbox's PID namespace and seccomp filters interfere.
- **/proc access:** Chromium reads `/proc/self/exe` and other procfs entries that behave differently in the sandbox.

No combination of flags (`--no-sandbox`, `--no-zygote`, `--disable-gpu`, `--disable-setuid-sandbox`, `--disable-dev-shm-usage`, `--disable-software-rasterizer`) works around these kernel-level restrictions. Chromium hangs (doesn't crash immediately), which Playwright reports as "Page crashed" after timeout.

**Firefox works** because it uses a simpler process model without Chrome's zygote/renderer architecture.

**Mitigation:** `just test-nixos-playwright-e2e` runs the **full interactive Playwright suite** (both Chromium and Firefox, 146 tests) inside 4 NixOS VMs where there are no kernel restrictions. `just test-nixos-e2e` validates Chromium DOM rendering via `--dump-dom` for fast smoke testing. For fastest development iteration, `just test-e2e` runs both browsers on the host. `just test-nix` (aliases: `test-full`, `check-nix`) runs `nix flake check` — all 27 checks, every test layer is covered.

---

## Lint & Format Checks

These run as part of `just ci` and `just test-nix` (`nix flake check`):

| Check | Tool | What | just command | nix check name |
|-------|------|------|-------------|----------------|
| Rust formatting | rustfmt | `*.rs` files | `just cargo-fmt-check` | `rustfmt-check` |
| TS/JS/CSS/JSON formatting | biome | `*.ts`, `*.js`, `*.css`, `*.json` | `just web-fmt-check` | `biome-check` |
| Rust linting | clippy | All clippy lints | `just clippy-lint` | `clippy-lint` |
| TS/JS linting | biome | lint rules | `just web-lint` | `web-lint` |
| Nix formatting | nixfmt | `*.nix` files | via `nix fmt` | `nix-fmt-check` |
| Nix linting | statix | Anti-patterns in nix | — | `statix-check` |
| Shell formatting | shfmt | `*.sh` files | — | `shfmt-check` |
| Shell linting | shellcheck | `*.sh` files | — | `shellcheck-check` |
| TOML validation | taplo | `*.toml` files | — | `taplo-check` |
| Multi-formatter | treefmt | Orchestrates all formatters | `just treefmt` | `treefmt-check` |
| Cargo check | cargo check | Type checking without codegen | `just cargo-check` | `cargo-check` |
| Rust docs | cargo doc | Build documentation | `just doc` | `doc` |

---

## Combined Commands

| Command | What It Runs | E2E? | Network? |
|---------|-------------|------|----------|
| `just check` | `fix` (auto-format) → `ci` | No | Yes (serve_tests) |
| `just ci` | fmt checks → clippy → web-lint → test-sandbox → web-unit → typecheck → doc → build → release | No | No (sandbox-safe) |
| `just test-nix` | `nix flake check` — all 27 checks (lint + test + E2E Firefox + NixOS VM tests + VM Playwright + VM integration — runs everything) | Yes (Firefox sandbox + VM both browsers + Chromium DOM dump) | No (sandboxed, serve_tests run in VM) |

---

## `just test-nix` / `nix flake check` — Complete Check List

All 27 checks that run in the nix build sandbox:

| # | Check Name | What It Does |
|---|-----------|-------------|
| 1 | `default` | Runs `just ci` (combined lint + test + build) |
| 2 | `cargo-fmt-check` | rustfmt `--check` on all `.rs` files |
| 3 | `web-fmt-check` | biome format check on TS/JS/CSS/JSON |
| 4 | `clippy-lint` | Clippy lints on all Rust code |
| 5 | `web-lint` | biome lint check on TS/JS |
| 6 | `test` | `cargo test --skip serve_tests` (all Rust tests, sandbox-safe) |
| 7 | `test-unit` | `cargo test --lib` (unit tests only) |
| 8 | `test-int` | `cargo test --test cli_integration --skip serve_tests` |
| 9 | `test-web` | TS unit + typecheck + Rust tests (sandbox-safe) |
| 10 | `test-web-unit` | `bun test` (TypeScript unit tests) |
| 11 | `test-web-typecheck` | `tsc --noEmit` |
| 12 | `doc` | `cargo doc --no-deps` |
| 13 | `cargo-check` | `cargo check` |
| 14 | `test-e2e` | Full Playwright suite — **Firefox only** in sandbox (78 tests) |
| 15 | `nix-fmt-check` | `nixfmt --check` on all `.nix` files |
| 16 | `treefmt-check` | treefmt `--ci` orchestrated format check |
| 17 | `biome-check` | biome format on TS/JS/CSS/JSON (standalone) |
| 18 | `rustfmt-check` | rustfmt `--check` (standalone) |
| 19 | `statix-check` | statix lint on `.nix` files |
| 20 | `shfmt-check` | shfmt format check on `.sh` files |
| 21 | `shellcheck-check` | shellcheck lint on `.sh` files |
| 22 | `taplo-check` | taplo validation on `.toml` files |
| 23 | `nixos-serve` | NixOS VM: HTTP API test (~15 assertions) |
| 24 | `nixos-e2e` | NixOS VM: Chromium `--dump-dom` DOM rendering (~10 assertions, **not Playwright**) |
| 25 | `nixos-playwright-e2e` | NixOS 4-VM: Full Playwright suite, Chromium + Firefox (146 interactive tests) |
| 26 | `nixos-integration` | NixOS VM: Full cli_integration test suite including serve_tests (~83 tests) |
| 27 | — | (Nix evaluates the build derivations as implicit checks) |

---

## Environment Comparison

| Property | `just test-*` (dev shell) | `just test-nix` (`nix flake check`) | `just check-one *` (sandbox) | `just test-nixos-*` (VM) | `just test-nixos-playwright-e2e` (4-VM) |
|----------|--------------------------|-------------------------------------|------------------------------|---------------------------|-------------------------------|
| **Runs in** | Host OS | Nix build sandbox | Nix build sandbox | NixOS VM (QEMU/KVM) | 4 NixOS VMs (QEMU/KVM) |
| **Network** | Full | None | None | Loopback only | Virtual network (inter-VM) |
| **Filesystem** | Full | Read-only source + tmp | Read-only source + tmp | Full VM filesystem | Full VM filesystem |
| **Kernel features** | Full | Restricted (namespaces, seccomp) | Restricted (namespaces, seccomp) | Full Linux kernel | Full Linux kernel |
| **Chromium** | ✅ Works | ❌ Hangs (kernel restrictions) | ❌ Hangs (kernel restrictions) | ✅ Works (`--dump-dom` only) | ✅ Works (full Playwright) |
| **Firefox** | ✅ Works | ✅ Works | ✅ Works | Not installed | ✅ Works (full Playwright) |
| **What runs** | Individual test sets | All 27 checks (lint + test + E2E + VM) | Individual check | NixOS VM test | Full Playwright suite (146 tests) |
| **Binary source** | `cargo build` (local) | `cargo build` (in sandbox) | `cargo build` (in sandbox) | `just build-nix` (nix package) | `just build-nix` (nix package) |
| **Browser source** | System/nix dev shell | `playwright-driver.browsers` (nix store) | `playwright-driver.browsers` (nix store) | `pkgs.chromium` (system package) | `playwright-driver.browsers` (nix store) |
| **Server mode** | `--ephemeral` | `--ephemeral --no-mdns --no-relay --no-gossip` | `--ephemeral --no-mdns --no-relay --no-gossip` | systemd `services.id` (ephemeral, no P2P) | systemd `services.id` (ephemeral, no P2P) |
| **Reproducible** | Mostly (nix shell tools) | Yes (hermetic) | Yes (hermetic) | Yes (hermetic VM) | Yes (hermetic VM) |
| **RAM required** | ~500MB | ~2GB+ (cargo build + VM tests) | ~2GB (cargo build) | ~2GB per VM | ~4GB per client VM (×2), ~512MB per server VM (×2) |
| **Runtime** | ~1.5 min (E2E) | ~15+ min (all checks + VM tests) | ~10+ min (cargo build + E2E) | ~3–5 min per VM test | ~3 min (boot + 146 tests) |

---

## Nix App vs Nix Check — When to Use Which

**`just test-nix`** (maximum coverage, aliases: `test-full`, `check-nix`, `nix-check`):
- Runs `nix flake check` — all 27 checks including VM Playwright and VM integration (both browsers, serve_tests)
- Hermetic and reproducible — **runs everything**
- ~15+ min (cargo build + tests + VM boot)
- **Use for:** Pre-release verification, maximum confidence, CI

**`just test-e2e`** (fast development, both browsers):
- Both Chromium and Firefox on the host
- Uses locally built binary (`cargo build`)
- Full 146 tests
- Fast if binary is already built
- **Use for:** Full browser coverage during development

**`just check-test-e2e`** (sandbox E2E, Firefox only):
- Builds everything from scratch inside nix sandbox (`nix build .#checks...test-e2e`)
- Firefox only (Chromium crashes in sandbox)
- 78 tests
- Slow (must compile Rust + build web assets)
- **Use for:** CI/reproducible verification that E2E tests pass

**`just test-nixos-e2e`** (VM DOM smoke test):
- Spins up full NixOS VM (`nix build .#checks...nixos-e2e`)
- Chromium `--dump-dom` only (~10 DOM structure assertions, **not interactive**)
- No clicking, no typing, no WebSocket, no navigation — just renders page and checks HTML output
- Tests the nix-packaged binary with systemd
- **Use for:** Fast smoke test — validating the NixOS module, systemd integration, and that JS renders correct DOM

**`just test-nixos-playwright-e2e`** (4-VM Playwright):
- Spins up 4 NixOS VMs: 2 servers + 2 clients (`nix build .#checks...nixos-playwright-e2e`)
- Full Playwright suite, both Chromium AND Firefox (146 interactive tests)
- Exercises WebSocket collaboration, multi-user editing, all UI interactions
- Hermetic and reproducible — no host dependencies
- ~3 minutes (boot + all tests)
- **Use for:** Complete E2E validation when you need hermetic, reproducible, both-browser Playwright coverage

**`just test-nixos-integration`** (VM integration):
- Spins up 1 NixOS VM (2GB RAM, 2 cores) (`nix build .#checks...nixos-integration`)
- Runs the full `cli_integration` test binary including `serve_tests` (~83 tests)
- Uses pre-built test binary (`integrationTestRunner`) with `ID_BINARY` env var pointing to nix-built `id`
- `serve_tests` need real networking (bind/listen on ports) — the VM provides this
- ~15 seconds (boot + tests)
- **Use for:** Running serve_tests in nix — the only way to test `id serve` subprocess spawning hermetically

---

## Test File Locations

```
tests/
└── cli_integration.rs              # 85 Rust integration tests

src/
├── cli.rs                          # 93 unit tests (CLI parsing)
├── tags.rs                         # 63 unit tests (tag system)
├── tuple.rs                        # 42 unit tests (binary encoding)
├── discovery.rs                    # 33 unit tests (peer discovery)
├── protocol.rs                     # 29 unit tests (P2P protocol)
├── helpers.rs                      # 16 unit tests (utilities)
├── lib.rs                          # 12 unit tests (bootstrap)
├── commands/
│   ├── find.rs                     # 26 unit tests
│   ├── serve.rs                    # 5 unit tests
│   ├── tag.rs                      # 6 unit tests
│   ├── peers.rs                    # 6 unit tests
│   ├── put.rs                      # 2 unit tests
│   ├── get.rs                      # 2 unit tests
│   └── list.rs                     # 1 unit test
├── repl/
│   ├── input.rs                    # 26 unit tests (shell preprocessing)
│   └── runner.rs                   # 23 unit tests (command dispatch)
└── web/
    ├── templates.rs                # 29 unit tests
    ├── content_mode.rs             # 20 unit tests
    ├── markdown.rs                 # 20 unit tests
    ├── collab.rs                   # 19 unit tests (WS collaboration)
    ├── routes.rs                   # 19 unit tests (HTTP handlers)
    ├── assets.rs                   # 8 unit tests
    └── mod.rs                      # 2 unit tests

web/src/
├── cursor-utils.test.ts            # 76 TS unit tests (cursor math)
└── editor.test.ts                  # 40 TS unit tests (editor state)

e2e/tests/
├── basic.spec.ts                   # 19 Playwright tests (UI fundamentals)
├── websocket.spec.ts               # 19 Playwright tests (WS + collab)
└── playwright.config.ts            # Config: ports, browsers, IS_NIX_BUILD

nix/tests/
├── serve-test.nix                  # NixOS VM: HTTP API (~15 assertions)
├── e2e-test.nix                    # NixOS VM: Chromium DOM (~10 assertions)
├── integration-test.nix            # NixOS VM: Full cli_integration suite (~83 tests, includes serve_tests)
└── playwright-e2e-test.nix         # NixOS 4-VM: Full Playwright (146 tests, Chromium + Firefox)
```

---

## When to Add Tests Where

Use this decision tree when you need to add a new test:

### Pure logic (parsing, encoding, formatting, data structures)
**Add to:** Rust unit tests in the relevant `src/*.rs` file, `#[cfg(test)] mod tests`

Tests here run in milliseconds. No I/O, no network, no filesystem. Examples: tag search query parsing, binary key encoding, CLI argument validation, content type detection, markdown rendering.

### Command behavior (CLI I/O, end-to-end command flow)
**Add to:** `tests/cli_integration.rs`

Tests the full command pipeline: parse args → open store → execute → format output. If the test needs network (e.g., `serve` command), put it in the `serve_tests` module — it will be skipped in the nix sandbox `test-int` check but runs in the `nixos-integration` VM check, so `just test-nix` still covers it.

### Frontend logic (ProseMirror, cursor math, editor state)
**Add to:** `web/src/*.test.ts` files (bun unit tests)

Pure TypeScript logic tests. No browser, no DOM. Fast (~2 seconds for all). Examples: cursor position calculation, selection range handling.

### Web UI interaction (click, type, navigate, visible state)
**Add to:** `e2e/tests/basic.spec.ts`

Tests that exercise the web UI through a real browser. Page loads, form submissions, navigation, theme switching, visual element presence. No WebSocket or real-time features.

### WebSocket, collaboration, real-time features
**Add to:** `e2e/tests/websocket.spec.ts`

Tests that need WebSocket connections: connect/disconnect/reconnect, collaborative editing, cursor sharing, tag live updates, multi-user scenarios. This file uses a fresh browser fixture to avoid Firefox WS degradation.

### Deployment and systemd integration
**Add to:** `nix/tests/serve-test.nix` or `nix/tests/e2e-test.nix`

Tests that validate the nix-packaged binary works correctly as a systemd service. `serve-test.nix` for HTTP API behavior (curl-based), `e2e-test.nix` for DOM rendering (Chromium `--dump-dom`). These run in a single NixOS VM.

### Adding new Playwright spec files
When you add a new `e2e/tests/*.spec.ts` file, it is automatically picked up by all three Playwright execution modes (local, nix sandbox, VM). The VM Playwright test (`playwright-e2e-test.nix`) runs `--project=chromium` and `--project=firefox` without specifying individual test files, so new spec files are included automatically.

### Do I still need local E2E (`just test-e2e`)?
**Yes, for development speed.** Local E2E runs in ~90 seconds with both browsers and gives immediate feedback. The VM Playwright test takes ~3 minutes and requires a full nix rebuild if any source changed. Use `just test-e2e` during development, rely on `just test-nix` (which includes VM Playwright) for pre-push verification.

### What are the limits of each test set?

| Test set | Limitation |
|----------|-----------|
| Rust unit tests | No I/O, no network, no browser |
| Rust integration | No browser, no web UI. `serve_tests` subset needs network (skipped in sandbox, runs in `nixos-integration` VM) |
| TS unit tests | No browser, no DOM, no WebSocket — pure logic only |
| Playwright sandbox (`just check-test-e2e`) | **Firefox only** — Chromium crashes in nix build sandbox |
| nixos-e2e | Chromium `--dump-dom` only — **no interactivity** (no clicks, typing, WS, navigation) |
| nixos-serve | **No browser** at all — curl + JSON parsing only |
| nixos-playwright-e2e | Full coverage, but **~3 min** and requires KVM. Playwright stdout only visible on failure |
| nixos-integration | Full cli_integration suite in VM, but requires KVM. 2 flaky web serve tests skipped (pre-existing) |
| Local E2E (`just test-e2e`) | Both browsers, full coverage, but **not hermetic** — depends on host state |

---

## Known Issues & Workarounds

### Firefox WebSocket Degradation (websocket.spec.ts)

**Problem:** After ~54 prior tests in the same Firefox browser process (basic.spec.ts runs first alphabetically), Firefox's networking stack degrades. WebSocket upgrade requests hang at the HTTP level — the server receives the upgrade request but the WS handshake never completes.

**Evidence:** Server logs show a ~40-second gap between upgrade request and timeout. The test passes on retry (fresh browser process).

**Fix:** `websocket.spec.ts` overrides Playwright's worker-scoped `browser` fixture to launch a fresh browser instance. All tests in the file automatically inherit the fresh browser via fixture scope. Cost: ~1–2 seconds launch overhead.

### Chromium in Nix Build Sandbox

**Problem:** All Chromium tests crash with "Page crashed" in the nix build sandbox, despite `--no-sandbox`, `--no-zygote`, `--disable-gpu`, and other flags.

**Root cause:** Nix build sandbox's kernel-level namespace and seccomp restrictions are fundamentally incompatible with Chromium's multi-process architecture.

**Fix:** `playwright.config.ts` conditionally runs Firefox-only when `IS_NIX_BUILD` is detected. Chromium coverage is provided by `just test-nixos-e2e` (NixOS VM with real kernel access).

### `just test-nix` / `nix flake check` with -L flag

**Problem:** `just test-nix` (`nix flake check -L`, verbose logs) crashes with `fatal runtime error: assertion failed: output.write(&bytes).is_ok()` in PTY mode. This is a nix 2.28.5 bug.

**Workaround:** Run `just test-nix` (which uses `nix flake check` without `-L`). Build logs are still available via `nix log` after the fact. Individual checks can be run with `just check-one <name>` (which does support `-L` internally).

### treefmt --check vs --fail-on-change

**Problem:** treefmt v2 renamed `--check` to `--fail-on-change`. The `nix fmt -- --check` invocation fails.

**Fix:** The justfile `treefmt` and `treefmt-no-cache` recipes translate `--check` to `--fail-on-change` automatically using a bash argument loop.

### git add requirement for nix flake commands

**Important:** Nix flakes only see **staged** files. After editing any file, you must `git add .` before running `just test-nix`, `just check-one *`, or any nix-based just command. Without staging, nix uses cached source and your changes won't be tested.
