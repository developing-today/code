---
session: ses_2af2
updated: 2026-04-03T03:03:05.149Z
---

## Summary

### Goal
Migrate all opencode MCP servers to lootbox. Install deno+git+lootbox via nix. Auto-start lootbox server on repo entry. Update opencode config and instructions.

### What Was Done

**Research (Complete):**
- Analyzed lootbox repo (v0.0.54, Deno 2.x project) — build: `deno task ui:build && deno compile --allow-all --include ui/dist -o lootbox src/lootbox-cli.ts`
- UI build: React+Vite app with heavy npm deps (monaco-editor etc.)
- deno.json imports include jsr:, npm:, and `https://esm.sh/@pothos/core@3.21.0` (URL import)
- Analyzed ibsenware.org deno+nix approach (FOD for deps + pure build)
- Found `deno vendor` doesn't support npm: specifiers; `deno cache` does

**Nix Derivation (In Progress — Stuck):**
- Created `pkgs/lootbox/default.nix` with two-phase approach (FOD for deps, pure build)
- **FOD succeeded** — got source hash `sha256-uY8VETshvwIbGjq10NRVc8ts4IEsKypvdBcjLqOLqu0=` and deps hash `sha256-t9Vzb0e3F4SPN2LD+fOeCP1bcC7Y1IWH8NnIDYct/4M=`
- **Pure build failed** — esm.sh URL import can't resolve from DENO_DIR cache (redirect means cache key differs). JSR deps resolve fine. Tried `DENO_NO_REMOTE=1`, `deno install --frozen`, etc.

### Key Problem
The `esm.sh/@pothos/core@3.21.0` URL import uses HTTP redirects, so the cached version's key doesn't match the import URL in the sandbox. This breaks the pure (no-network) build phase.

### User's Fallback Direction
> "if you can't figure it out then just make a script that installs it correctly if the devshell runs and the binary and deno aren't available... if the server isn't running on the port then stand up the server"

OK with caching everything, don't need `--frozen`. Try vendoring per ibsenware.org blog first, then fall back to shell-hook install script.

### Files Modified
- **Created:** `pkgs/lootbox/default.nix` (has correct hashes but build phase broken)

### Files NOT Yet Modified
- `nix-common.nix` (needs deno + lootbox added)
- `opencode.jsonc` (needs MCP servers removed, lootbox CLI usage added)
- `.opencode/instructions/` (replace MCP-specific docs with lootbox instructions)
- `justfile`/`root.just` (needs `update-lootbox` recipe)
- `.envrc` or shellHook (auto-start lootbox server)
- `lootbox.config.json` (needs creation with all 3 MCP servers)

### Remaining Work
1. **Fix nix build OR pivot to shell-hook approach** (try vendoring, or fall back to install script)
2. Add deno to `nix-common.nix`
3. Create `lootbox.config.json` with MCP servers: codedb (`/home/user/bin/codedb mcp`), fff (`/home/user/.local/bin/fff-mcp`), chrome-devtools (`npx -y chrome-devtools-mcp@latest`)
4. Update `opencode.jsonc` — remove mcpServers, add lootbox CLI
5. Update `.opencode/instructions/` 
6. Add `just update-lootbox` recipe
7. Update `.envrc`/shellHook to auto-start lootbox server
8. Run `lootbox init`

### Key Context
- `.envrc`: `use flake` + `use flake ./src/id` + `dotenv_if_exists .env.local`
- ALL MCP servers go into lootbox config, NONE remain in opencode
- Opencode calls lootbox CLI directly (not as MCP)
- Lootbox MCP config format: `{ "command": "string", "args": ["string"], "env": {}, "transport": "stdio" }`
