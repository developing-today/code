All MCP tools are accessed through `mcpx`. The `mcpx` gateway daemon unifies all MCP servers under session isolation and TypeScript Code Mode.

## Always write scripts or use `mcpx_exec` / `mcpx exec`

Write `.ts` scripts or use `mcpx_exec` (via native tool or CLI `mcpx exec '<code'>`) for tool usage.
TypeScript execution filters large payloads on the server side so only relevant results enter your LLM context window.

```bash
# Inline check via CLI
mcpx exec 'console.log(await tools.codedb.status({}))'

# Multi-line script execution via CLI
cat << 'EOF' | mcpx exec -
const results = await tools.fff.grep({ query: "TODO" });
console.log(JSON.stringify((results.matches || []).slice(0, 5), null, 2));
EOF
```

## Available MCP Namespaces

| Namespace | Tools | Description |
|---|---|---|
| `codedb` | tree, outline, symbol, search, word, hot, deps, read, edit, changes, status, snapshot, bundle, projects, index, remote | Codebase exploration, symbol lookup, AST-aware search |
| `fff` | grep, find_files, multi_grep | Frecency-ranked file search and content grep |
| `chrome-devtools` | navigate_page, take_screenshot, take_snapshot, click, fill, press_key, hover, type_text, evaluate_script, wait_for, upload_file, handle_dialog, list_console_messages, get_console_message, list_network_requests, get_network_request | Browser automation, UI verification, screenshots (session-isolated) |
| `context7` | resolve_library_id, query_docs | Library documentation lookup (API refs, usage guides, examples) |
| `jules` | Jules tasks, issues, and orchestration | Google Jules integration |
| `antigravity-jules-orchestration` | Orchestration tasks | Multi-agent coordination |
| `antigravity` | Agent tooling | Antigravity interface |
| `desktop-commander` | Desktop commander | System administration & actions |
| `sequential-thinking` | sequentialthinking | Step-by-step reasoning server |
| `mcp-server-neon` | database operations | Neon serverless Postgres tools |
| `cloudrun` | service management | Google Cloud Run deployment and inspection |
| `gopls-mcp-server` | Go LSP tools | Go language server diagnostics and navigation |

## Script Patterns

```typescript
// Find definitions
const sym = await tools.codedb.symbol({ name: "handleAuth" });
console.log(sym);

// Browser verification (isolated session)
await tools["chrome-devtools"].navigate_page({ url: "http://localhost:3000" });
const snap = await tools["chrome-devtools"].take_snapshot({});
console.log(snap);

// Library documentation
const lib = await tools.context7.resolve_library_id({ query: "how to use React hooks", libraryName: "react" });
const docs = await tools.context7.query_docs({ libraryId: lib.libraryId, query: "useEffect cleanup" });
console.log(docs);

// Chaining tools across servers
const files = await tools.fff.grep({ query: "deprecated" });
for (const f of (files.matches || []).slice(0, 5)) {
  const outline = await tools.codedb.outline({ path: f.path });
  console.log(f.path, outline);
}
```

## Commands

| Command | Description |
|---|---|
| `mcpx ls` | List active server namespaces and tool counts |
| `mcpx types <ns>` | Print TypeScript signatures for a namespace |
| `mcpx exec 'code'` | Run inline TypeScript against all MCP servers |
| `mcpx exec <script>.ts` | Run a script file |
| `mcpx call <server>.<tool> '<args>'` | Call a single tool with JSON arguments |
| `mcpx daemon run` | Run daemon in foreground |
| `mcpx log` | Inspect execution logs |
| `mcpx stats` | Check slowest tools and error counts |
| `mcpx skills list` | List embedded agent skills |
| `mcpx skills install` | Install skills for OpenCode, Claude, Codex, or Antigravity |

## Key Notes

- Configuration: `.mcpx.json` at repo root
- Secrets: loaded from environment / direnv (e.g. `JULES_API_KEY`, `CONTEXT7_API_KEY`) — never put secrets in config files
- Stateful servers (e.g. `chrome-devtools`) are leased exclusively per session
