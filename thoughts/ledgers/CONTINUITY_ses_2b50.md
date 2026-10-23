---
session: ses_2b50
updated: 2026-04-01T23:12:37.579Z
---

## Summary of Work Done This Session

### Main Task
Install and configure **codedb** (justrach/codedb) as an MCP tool for OpenCode, along with instruction files for multiple MCP tools.

### Accomplishments

#### 1. codedb Installation
- Downloaded codedb v0.2.1 — segfaulted (known bug #84)
- Downloaded **v0.2.2** from GitHub releases — works correctly
- Binary installed to `/home/user/bin/codedb` (6.8MB, static ELF x86_64, works on NixOS without patching)
- `~/bin` is NOT on PATH, so full path is used in config

#### 2. MCP Configuration in `.opencode/opencode.jsonc`
- Added codedb as MCP server
- **Found and fixed bug**: OpenCode schema uses `environment` (not `env`) for env vars (`additionalProperties: false` rejects `env`)
- Final working config:
```json
"codedb": {
  "type": "local",
  "command": ["/home/user/bin/codedb", "mcp"],
  "environment": { "CODEDB_NO_TELEMETRY": "1" }
}
```

#### 3. Three Instruction Files Created in `.opencode/instructions/`
All wired into `opencode.jsonc` via `instructions` array:
1. **`codedb.md`** — prefer codedb_* tools for codebase exploration, use `codedb_bundle` for batching, `codedb_index` for worktrees, fallback to grep/glob/read
2. **`chrome-devtools.md`** — prefer for UI verification, write Playwright e2e tests after validating, fallback to curl/webfetch
3. **`context7.md`** — prefer for library docs, max 3 retries then fallback to `webfetch` for official docs, then optionally `btca_ask`

### Key Technical Details
- **codedb**: 16 MCP tools (tree, outline, symbol, search, word, hot, deps, read, edit, changes, status, snapshot, bundle, remote, projects, index). File watcher polls every 2s. `ProjectCache` LRU with MAX_CACHED=5 (hardcoded in `mcp.zig` line 29). Idle timeout: 30min (hardcoded). Data in `~/.codedb/projects/<hash>/`.
- **OpenCode MCP schema fields**: `type`, `command`, `environment` (Record<string,string>), `enabled` (boolean), `timeout` (integer ms, default 5000)
- **context7**: Remote MCP at `https://mcp.context7.com/mcp`, optional `CONTEXT7_API_KEY` env var

### Open/Potential Next Step
- **Rebuilding codedb from source** to increase MAX_CACHED from 5 to 25 was offered but not yet confirmed by user. System has Zig 0.15.2. Would require: clone repo → edit `src/mcp.zig` line 29 → `zig build -Doptimize=ReleaseFast` → replace binary.
