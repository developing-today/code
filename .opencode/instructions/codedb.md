Use `codedb_*` MCP tools for codebase exploration — they are faster and more token-efficient than grep/glob/read for indexed queries. If codedb tools fail or return errors, fall back to built-in tools (grep, glob, read, bash with ripgrep).

## Local Tools

| Tool | Use for |
|---|---|
| `codedb_tree` | File tree / directory structure |
| `codedb_outline` | List symbols in a file |
| `codedb_symbol` | Find symbol definitions across the project |
| `codedb_search` | Full-text search |
| `codedb_word` | Exact identifier / word lookup |
| `codedb_hot` | Recently modified files |
| `codedb_deps` | Reverse dependency graph |
| `codedb_read` | Read file content |
| `codedb_edit` | Apply edits |
| `codedb_changes` | Changes since a sequence number |
| `codedb_status` | Index status |
| `codedb_snapshot` | Full project snapshot |
| `codedb_bundle` | Batch multiple read-only queries (max 20 ops) to save round-trips |
| `codedb_projects` | List indexed projects |
| `codedb_index` | Index a new project directory |

## Remote Code Lookup

`codedb_remote` searches source code in remote repositories (e.g. GitHub). Use this when you need to look at the **actual source code** of a library or framework — implementation details, internal logic, how something really works. This is faster and more direct than Context7 for raw code. Prefer `codedb_remote` over Context7 when:
- You need to read the source of a known repo (not docs/guides)
- You want to understand how a library implements something internally
- You're debugging behavior that docs don't explain

Context7 is still better for: documentation, usage guides, API references, code examples.

## Worktree Support

When working in a git worktree (different directory from main workspace), call `codedb_index` with the worktree's absolute path first, then pass `project: "<worktree-path>"` to all subsequent codedb queries. The codedb server indexes the main workspace CWD on startup; other directories need explicit indexing. Up to 5 projects are cached simultaneously.

## Auto-Indexing

codedb watches the filesystem (2s polling) and incrementally re-indexes modified files automatically — no manual re-index needed for edits within the active project.

## CLI

The codedb binary is at `/home/user/bin/codedb`. Agents may use it directly from the shell if needed, but should prefer the MCP tools unless they have a reason to deviate. CLI commands can be looked up from the codedb repo.
