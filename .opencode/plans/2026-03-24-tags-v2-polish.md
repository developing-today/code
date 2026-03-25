# Tags V2 Polish: Aliases, Tests, Playwright, Nix, Docs

> **Status:** Draft
> **Date:** 2026-03-24
> **Predecessor:** [2026-03-24-tags-v2-iroh-docs.md](./2026-03-24-tags-v2-iroh-docs.md)

## Overview

Complete the Tags V2 work with: REPL/CLI aliases (`tag`/`label`/`link`), comprehensive test coverage (unit + integration + Playwright E2E), Nix infrastructure for browser testing, fix nix flake check -L to 100%, documentation updates, and build.sh log level fix.

---

## Task 1: REPL Tag Aliases (`label`, `link`)

**Goal:** Allow `label` and `link` as interchangeable prefixes for all `tag` commands in both CLI and REPL.

### 1a. CLI aliases (src/cli.rs)

- **Line 807:** Change `#[command(subcommand)]` to `#[command(subcommand, aliases = ["label", "link"])]` on the `Tag(TagCommand)` variant
- **Line ~27 in docstring:** Add `label` and `link` to the CLI structure comment

### 1b. REPL aliases (src/repl/runner.rs)

- **Lines 356-404:** Every `"tag"` pattern match must also accept `"label"` and `"link"`:
  - Line 356: `["tag", "set" | "add", ...]` → `["tag" | "label" | "link", "set" | "add", ...]`
  - Line 360: Same pattern
  - Line 364-371: Same for `del` variants
  - Line 376-384: Same for `del` with value
  - Line 397: `["tag", "search", key]` → `["tag" | "label" | "link", "search", key]`
  - Line 401: Same for search with value
- **Lines 389-395:** The standalone `tags` command → also add `labels` and `links` as aliases
- **Help text (lines 505-509):** Update to mention `label`/`link` as aliases

### 1c. Tests

- **Unit test in cli.rs tests:** Verify `id label set X Y Z` and `id link set X Y Z` parse correctly
- **Unit test in runner.rs tests:** Verify `label set ...` and `link set ...` dispatch correctly

---

## Task 2: Fix nix flake check -L (Critical - Unblocks Everything)

**Goal:** Get `nix flake check -L` to 100% pass.

### Root cause
`src/commands/tag.rs` is locally present but **untracked** by git. Nix copies only git-tracked files (`src = ./.;`), so `mod tag` in `commands/mod.rs` fails to resolve.

### Fix
```bash
git add src/commands/tag.rs  # Track the file
git add src/tags.rs src/tuple.rs  # Track other new files
# Also track all other new/modified files from Tags V2 work
```

Then run `nix flake check -L` and fix any remaining failures iteratively. Pre-existing failures must be fixed (not deleted), tests can be rewritten if incorrect.

---

## Task 3: New Unit Tests

**Goal:** Add unit tests for all new code from Tags V2.

### 3a. tags.rs (src/tags.rs)
Already has 22 tests. Verify coverage of `copy_all_tags()` — add if missing.

### 3b. commands/tag.rs (src/commands/tag.rs)
Add tests for:
- `cmd_tag_set` argument parsing
- `cmd_tag_del` argument parsing  
- `cmd_tag_list` argument parsing
- `cmd_tag_search` argument parsing

### 3c. web/routes.rs — copy_handler
Add tests for:
- `CopyRequest`/`CopyResponse` serialization
- `copy_handler` success path
- `copy_handler` when source doesn't exist
- `copy_handler` when target already exists (archives)

### 3d. web/templates.rs
Add tests for:
- `render_media_viewer` includes rename/copy buttons
- `render_binary_viewer` includes rename/copy buttons  
- `render_editor` includes copy button

### 3e. repl/runner.rs
Add tests for:
- `label` and `link` alias dispatch
- `labels`/`links` standalone aliases

### 3f. cli.rs
Add tests for:
- `id label set X Y Z` parses correctly
- `id link del X Y` parses correctly
- `id label list` parses correctly
- `id link search K V` parses correctly

---

## Task 4: New Integration Tests

**Goal:** Add integration tests in `tests/cli_integration.rs` for tag commands.

### 4a. Tag command tests (new module: `tag_tests`)
- `test_tag_set_and_list`: Put a file, set tags, verify `tag list` output
- `test_tag_del`: Set then delete a tag, verify gone
- `test_tag_search`: Set tags on multiple files, search by key/value
- `test_tag_label_alias`: Verify `id label set ...` works same as `id tag set ...`
- `test_tag_link_alias`: Verify `id link search ...` works same as `id tag search ...`

### 4b. Copy command test (if CLI copy exists)
Check if `copy` is exposed as a CLI command — if not, test via REPL integration or skip.

---

## Task 5: Playwright E2E Tests Setup

**Goal:** Set up basic Playwright infrastructure and write foundational E2E tests for the web UI.

### 5a. Infrastructure setup

**Package setup:**
```bash
cd web && bun add -d @playwright/test
```

**Config file:** `web/playwright.config.ts`
- Projects: `chromium` and `firefox`
- `webServer` config to start `id serve --web` before tests
- Use environment variable for browser executable path (Nix compatibility):
  ```typescript
  use: {
    channel: undefined,  // Let Nix provide the browser
    launchOptions: {
      executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined,
    }
  }
  ```

**Browser path handling for Nix:**
- In `nix-common.nix`, set env vars:
  ```nix
  PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH = "${pkgs.chromium}/bin/chromium";
  PLAYWRIGHT_FIREFOX_EXECUTABLE_PATH = "${pkgs.firefox}/bin/firefox";
  PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
  ```
- In playwright config, read these env vars for `executablePath`

### 5b. Nix dependencies (nix-common.nix)

Add to `nativeBuildInputs`:
- `playwright-driver` (or `playwright-test` if available)
- `chromium`
- `firefox`

Add shell hook environment variables:
```nix
PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = "1";
```

### 5c. Justfile commands

```just
# Run Playwright E2E tests
test-e2e:
    cd web && bunx playwright test

# Run Playwright tests for specific browser
test-e2e-chromium:
    cd web && bunx playwright test --project=chromium

test-e2e-firefox:
    cd web && bunx playwright test --project=firefox
```

Add corresponding `nix run` apps in `flake.nix`.

### 5d. Basic E2E test files

**`web/e2e/home.spec.ts`** — Home page tests:
- Page loads with title containing "id"
- File list is visible
- New file form exists
- Can create a new file
- Search/filter input works

**`web/e2e/editor.spec.ts`** — Editor tests:
- Navigate to a file and editor loads
- Rename button visible
- Copy button visible
- Can rename a file via prompt

**`web/e2e/tags.spec.ts`** — Tag UI tests:
- Tags display on file list items
- Tag panel visible on editor page
- Can add a tag via inline form

Each test runs against both Chromium and Firefox (via Playwright projects).

### 5e. Flake.nix check integration

Add `test-e2e` check if feasible (may need headless browser in nix sandbox — might need to skip or mark as `passthru` if sandboxing is an issue).

---

## Task 6: build.sh Log Level Fix

**Goal:** Update build.sh to set `RUST_LOG=info` instead of debug.

### Analysis
`scripts/build.sh` (and `build.sh` which is a symlink to it) does NOT currently set any log level. The user likely wants the serve/run commands to default to `info` rather than `debug`.

### Approach
- Check if `RUST_LOG` is set in the build script or justfile serve commands
- If the justfile `serve` command or `build.sh` sets `RUST_LOG=debug`, change it to `RUST_LOG=info`
- If neither sets it, add `export RUST_LOG="${RUST_LOG:-info}"` to `build.sh` so the built binary defaults to info unless overridden
- Also check `justfile` serve recipes for any `RUST_LOG=debug` settings

---

## Task 7: Documentation Updates

### 7a. README.md (new — replaces deleted brainstorm notes)

Create a proper project README with:
- Project name and one-line description
- Feature list (P2P file sharing, metadata tags, web UI, REPL, etc.)
- Quick start / installation
- Usage examples (CLI, REPL, serve)
- Architecture overview (link to ARCHITECTURE.md)
- Development setup (nix develop)
- License

### 7b. ARCHITECTURE.md (new)

Create comprehensive architecture document:
- System overview diagram
- Module structure (src/ tree with descriptions)
- Data flow: put → store → protocol → peers
- Storage layer: iroh-blobs (content) + iroh-docs (metadata tags)
- Tag system: Alpha/Omega CRDT documents, tuple encoding
- Web UI architecture: Axum routes, templates, WebSocket collaboration
- REPL design: readline + preprocessing + command dispatch
- Peer discovery: DHT + gossip topics
- Build variants: lib vs web

### 7c. WEB.md updates

- Add copy API route (`POST /api/copy`)
- Document rename/copy buttons on all viewer pages
- Update API routes table
- Document tag UI (tag panel, inline add, bulk tag operations)

### 7d. web/README.md updates

- Add `copyFile()` to JavaScript API section
- Document tag-related WebSocket events (`TagEvent`)
- Update file structure if new files were added

### 7e. docs/ feature documentation

Create `docs/2026-03-24T00-00-00Z_feature_tag_aliases/` with:
- Link back to plan file
- Description of `tag`/`label`/`link` alias system
- REPL and CLI alias tables
- References

---

## Task 8: Commit and Verify

### 8a. Stage all files
```bash
git add -A  # Track everything (respecting .gitignore)
```

### 8b. Run full verification
```bash
just check          # Primary quality gate
nix flake check -L  # Nix-level verification
```

### 8c. Commit
```
feat(tags): complete tags v2 with aliases, copy UI, tests, and docs

- Add tag/label/link aliases to CLI and REPL
- Add /api/copy route and copyFile() JS method
- Add rename/copy buttons to all viewer pages (editor, media, binary)
- Add tag UI: pills, inline add, bulk operations, search/filter
- Set up Playwright E2E tests with Chromium + Firefox
- Add unit tests for tag commands, copy handler, aliases
- Add integration tests for tag CLI commands
- Fix nix flake check -L (track all new files)
- Create ARCHITECTURE.md, proper README.md
- Update WEB.md, web/README.md with new features
- Set RUST_LOG default to info in build.sh
```

---

## Execution Order

1. **Task 2** (Fix nix) — unblocks everything, just `git add` untracked files
2. **Task 1** (Aliases) — small code change
3. **Task 6** (build.sh) — small change
4. **Task 3** (Unit tests) — can run with `just test-unit`
5. **Task 4** (Integration tests) — requires binary build
6. **Task 5** (Playwright) — largest new infrastructure
7. **Task 7** (Documentation) — can be parallelized with tests
8. **Task 8** (Commit) — final verification

Tasks 3, 4, 5, 7 can be partially parallelized.
