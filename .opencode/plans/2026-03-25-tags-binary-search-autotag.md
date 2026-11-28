# Tags: Binary Keys/Values, Search Syntax, Auto-Tagging & CLI/REPL Parity

**Date:** 2026-03-25
**Branch:** `e2e-nix`
**Predecessor:** [Tags v2 Iroh Docs Plan](2026-03-24-tags-v2-iroh-docs.md)

---

## Original Request (verbatim)

> - repl and cli should be 1:1, ensure any diff in one is copied into the other, such as the aliases etc.
> - ensure docs keys can be bytes/binary allowing structured data. either key or value can be bytes and not just string. review the original tags v2 plan at .opencode/plans/2026-03-24-tags-v2-iroh-docs.md
> - ensure tag keys and values can be any length and any kind of bytes, but try to parse as utf8. cutoff in the ui/etc after a certain length. if it doesn't parse as utf8 then exclude as binary without a flag.
> - allow searching keys and values in the ui. strings ending with : mean keys, starting with : mean values key:value means that specific key. allow quotes to do regular search so ":key" looks for any file/key/value that is literally ":key". "key:":":value" looks for a key named "key:" with a value ":value". a file uploaded should automatically tag name,file,path tags to the file. all name/file/path keys should show as the name in the ui, but they should dedupe if they have the exact same hash, tag values, name of the file. make a migrate flag that automatically adds the name/file/path labels for all existing files which don't have them.
> - when you are done add appropriate unit/integration/playwright tests as-needed.
> - when you are done review all .md files across the repo and ensure they are up-to-date.
> - when you are done, nix flake check -L should pass 100%. fix any errors even if you dont think you made them.
> - when you are done, update all docstrings, comments, help text for all functions and objects in rust and typescript.

---

## 1. Overview & Intent

This plan transforms the tag system from string-only to arbitrary-bytes-capable, adds structured search syntax across CLI/REPL/Web, auto-tags uploaded files with name/file/path metadata, and ensures CLI and REPL are exact mirrors. The design philosophy: tags are arbitrary key-value byte pairs that are *displayed* as UTF-8 when possible, with graceful fallback for binary data.

---

## 2. Architecture: Before & After

### Before

| Layer | Current State |
|---|---|
| **Tag struct** (`tags.rs`) | `subject: String, key: String, value: Option<String>` |
| **Tuple encoding** (`tags.rs`) | Always uses `enc.string()` — binary data rejected |
| **Protocol** (`protocol.rs`) | `SetTag/DelTag/GetTags/SearchTags` — all `String` fields |
| **CLI** (`cli.rs`) | `TagCommand` variants use `String` — no binary support |
| **REPL** (`runner.rs`) | Pattern-matched tag commands — no `find` alias for search, has `tags` shorthand |
| **Web API** (`tags_ws.rs`) | REST/WS endpoints — all `String`, no search endpoint |
| **Put command** (`commands/put.rs`) | Stores blob + sets iroh tag — NO metadata auto-tagging |
| **Search** | Basic key/value string match only — no structured syntax |
| **Display** | No length cutoff, no binary indicator |

### After

| Layer | Target State |
|---|---|
| **Tag struct** (`tags.rs`) | `subject: Vec<u8>, key: Vec<u8>, value: Option<Vec<u8>>` with `Display` that tries UTF-8 |
| **Tuple encoding** (`tags.rs`) | Uses `enc.bytes()` for raw data; decode tries `as_str()` then falls back to `as_bytes()` |
| **Protocol** (`protocol.rs`) | Tag fields become `Vec<u8>`; serde as base64 for JSON compat; postcard handles natively |
| **CLI** (`cli.rs`) | Accepts string args; `--hex` flag for inputting binary; displays with UTF-8-try logic |
| **REPL** (`runner.rs`) | Exact 1:1 with CLI: same aliases, same subcommands, same flags |
| **Web API** (`tags_ws.rs`) | Base64-encoded byte fields; new `/api/tags/search` endpoint with query syntax |
| **Put command** (`commands/put.rs`) | Auto-creates `name`, `file`, `path` metadata tags after successful upload |
| **Search** | Structured syntax: `key:` (key filter), `:value` (value filter), `key:value` (pair), `"quoted"` (literal) |
| **Display** | UTF-8 attempted; binary shown as `<binary N bytes>` unless `--binary` flag; 256-char cutoff in UI |
| **Migration** | `id migrate-tags` command to backfill name/file/path for existing entries |

---

## 3. Detailed Design

### 3.1 Binary Keys & Values

**Core change:** Replace `String` with `Vec<u8>` throughout the tag pipeline.

**`TagValue` wrapper type** (new in `tags.rs`):
```rust
/// A tag field that stores arbitrary bytes but tries to display as UTF-8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TagValue(Vec<u8>);

impl TagValue {
    pub fn from_str(s: &str) -> Self { Self(s.as_bytes().to_vec()) }
    pub fn from_bytes(b: Vec<u8>) -> Self { Self(b) }
    pub fn as_bytes(&self) -> &[u8] { &self.0 }
    pub fn as_str(&self) -> Option<&str> { std::str::from_utf8(&self.0).ok() }
    pub fn is_utf8(&self) -> bool { std::str::from_utf8(&self.0).is_ok() }
    pub fn display_lossy(&self) -> String { String::from_utf8_lossy(&self.0).into_owned() }
    pub fn display_truncated(&self, max: usize) -> String { /* UTF-8 try, truncate, add "..." */ }
}
```

**Serde:** Serialize as base64 string for JSON (web API), raw bytes for postcard (protocol).

**Tuple encoding changes in `tags.rs`:**
- `encode_alpha_key` / `encode_omega_key`: use `enc.bytes()` instead of `enc.string()`
- `decode_alpha_key` / `decode_omega_key`: try `as_str()`, fall back to `as_bytes()`

**Display rules:**
- Try UTF-8 parse. If valid: display as string, truncated at 256 chars in UI.
- If invalid UTF-8: display as `<binary N bytes>` by default. Show hex with `--binary`/`--hex` flag.
- In web UI: show as `[binary: N bytes]` with a "copy hex" button.

### 3.2 CLI/REPL Parity

**Identified diffs to fix:**

| Feature | CLI | REPL | Fix |
|---|---|---|---|
| Search alias `find` | ✅ `tag search` alias `find` | ❌ Only `tag search` | Add `find` to REPL patterns |
| `tags` shorthand | ❌ No shorthand | ✅ `tags\|labels\|links` lists all | Document as REPL-only interactive convenience (acceptable diff) |
| Help text | Via `--help` | Via `help tag` | Ensure identical descriptions |

**REPL additions:**
- Add `find` as alias: `tag|label|link find|search <key> [value]`
- Ensure all error messages match CLI wording

### 3.3 Search Syntax

**Grammar:**

```
search_term  := quoted_string | key_value | key_only | value_only | bare_word
quoted_string := '"' <any chars> '"'       → literal search across subject/key/value
key_only      := <word> ':'                → filter by key name
value_only    := ':' <word>                → filter by value
key_value     := <word> ':' <word>         → filter by key AND value
bare_word     := <word>                    → search across subject/key/value
```

**Examples:**
- `name:` → all tags with key "name"
- `:myfile.txt` → all tags with value "myfile.txt"
- `name:myfile.txt` → tags with key "name" and value "myfile.txt"
- `":key"` → literal search for ":key" in any field
- `"key:"` → literal search for "key:" in any field
- `"key:":":value"` → key is literally "key:", value is literally ":value"

**Implementation:** New `SearchQuery` parser in `tags.rs` with `parse_search_query(input: &str) -> Vec<SearchTerm>`. Applied in `search_tags()` method.

**Where it applies:**
- CLI: `id tag search <query>` — query is the full search string
- REPL: `tag search <query>`
- Web UI: Search box in tags panel sends to `/api/tags/search?q=<query>`

### 3.4 Auto-Tagging on Upload

**When:** After successful `put` (file or stdin) — both local and remote modes.

**Tags created:**
- `name` → filename (basename, e.g., `photo.jpg`)
- `file` → filename (same as name, for alias compatibility)
- `path` → full original path as provided by user (e.g., `~/photos/photo.jpg`)

For stdin: `name` and `file` = provided name, `path` = `<stdin>`.

**Files affected:**
- `commands/put.rs` — add `auto_tag()` call after successful store
- `tags.rs` — new `auto_tag(subject_hash: &str, filename: &str, path: &str)` method

### 3.5 Display Name from Tags

**In Web UI:** Files with `name`, `file`, or `path` tags display the tag value as the human-readable name instead of the hash.

**Deduplication rule:** In file listings, if multiple entries have the same hash AND the same set of tag values AND the same display name → show only once.

**Implementation:** In `routes.rs` and web frontend, when rendering file lists:
1. Query tags for each hash
2. Use first found of `name` > `file` > `path` as display name
3. Group by (hash, display_name, tag_set) for dedup

### 3.6 Migration Command

**New command:** `id migrate-tags`

**Behavior:**
1. Scan all iroh blob tags (the name→hash mappings)
2. For each, check if `name`/`file`/`path` metadata tags exist
3. If missing, create them from the blob tag name
4. Report: `Migrated N files, skipped M already-tagged files`

**CLI definition:** New `MigrateTags` variant in the command enum.
**REPL:** `migrate-tags` command.

### 3.7 Web UI Search

**New endpoint:** `GET /api/tags/search?q=<search_query>`

**Frontend changes (web/):**
- Add search input box to file list / tags panel
- Real-time filtering as user types (debounced 300ms)
- Results show matching files with highlighted matching key/value
- Binary values shown as `[binary: N bytes]`

---

## 4. File-by-File Change List

### Rust (`src/`)

| File | Changes |
|---|---|
| `tags.rs` | Add `TagValue` type; change `Tag` struct fields to `TagValue`; update encode/decode to use `bytes()`; add `SearchQuery` parser; add `auto_tag()` method; add `migrate_tags()` method; update all methods to accept `impl Into<TagValue>` |
| `tuple.rs` | No changes needed (already supports bytes) |
| `protocol.rs` | Change `MetaRequest`/`MetaResponse` tag fields from `String` to `Vec<u8>`; add `MigrateTags` request/response; add `SearchTags` with query string |
| `cli.rs` | Add `--hex`/`--binary` flags to tag commands; add `MigrateTags` command; update help text |
| `commands/tag.rs` | Adapt to `TagValue`; handle display truncation; handle `--hex` flag |
| `commands/put.rs` | Call `auto_tag()` after successful put (both file and stdin paths) |
| `commands/serve.rs` | Handle new `MigrateTags` protocol message |
| `repl/runner.rs` | Add `find` alias for search; add `migrate-tags` command; ensure all aliases match CLI |
| `commands/repl.rs` | Add `migrate_tags()` method to ReplContext |
| `web/tags_ws.rs` | Change API types to use base64 for bytes; add `/api/tags/search` endpoint |
| `web/routes.rs` | Use tag display names for file listings; add dedup logic |
| `web/mod.rs` | No structural changes expected |

### TypeScript (`web/`)

| Area | Changes |
|---|---|
| Tags WebSocket client | Handle base64-encoded byte fields |
| Search component | New search input with structured query syntax |
| File list rendering | Display name from tags; dedup identical entries |
| Binary display | Show `[binary: N bytes]` with copy-hex action |

### Tests

| File | Changes |
|---|---|
| `src/tags.rs` (unit) | Tests for `TagValue`, binary encode/decode, `SearchQuery` parser, auto-tag, migration |
| `tests/cli_integration.rs` | Tag binary round-trip, search syntax, migrate-tags, auto-tag on put |
| `web/` (vitest) | Search component tests, binary display tests |
| Playwright (if configured) | E2E tag search flow, file upload with auto-tags |

---

## 5. Execution Steps

### Phase 1: Core Binary Support
1. Create `TagValue` type in `tags.rs` with UTF-8 try-parse, display, serde
2. Update `Tag` struct to use `TagValue` for subject/key/value
3. Update `encode_alpha_key`/`encode_omega_key` to use `enc.bytes()`
4. Update `decode_alpha_key`/`decode_omega_key` with bytes fallback
5. Update all `tags.rs` methods to accept `TagValue`
6. Run `just check` — fix compile errors across dependent files

### Phase 2: Protocol & Transport
7. Update `MetaRequest`/`MetaResponse` tag variants to `Vec<u8>`
8. Update `commands/tag.rs` to convert between CLI strings and `TagValue`
9. Update `commands/serve.rs` handler for new types
10. Run `just check`

### Phase 3: CLI/REPL Parity
11. Add `find` alias to REPL search command in `runner.rs`
12. Add `migrate-tags` command to REPL
13. Verify all aliases match between CLI and REPL
14. Update all help text to be identical
15. Run `just check`

### Phase 4: Search Syntax
16. Implement `SearchQuery` parser in `tags.rs`
17. Wire `search_tags()` to use `SearchQuery`
18. Update CLI `tag search` to pass raw query string
19. Update REPL `tag search` / `tag find` to use query syntax
20. Add unit tests for parser edge cases
21. Run `just check`

### Phase 5: Auto-Tagging & Migration
22. Add `auto_tag()` to `tags.rs`
23. Call `auto_tag()` from `commands/put.rs` after successful store
24. Add `migrate_tags()` to `tags.rs`
25. Add `MigrateTags` to CLI enum and protocol
26. Add `migrate-tags` command to REPL (from step 12)
27. Run `just check`

### Phase 6: Web UI & API
28. Update `tags_ws.rs` API types for base64 byte fields
29. Add `/api/tags/search` endpoint with query syntax
30. Update web frontend: search box, binary display, display names, dedup
31. Run `just check`

### Phase 7: Display & UX Polish
32. Add `--hex`/`--binary` flags to CLI tag commands
33. Implement 256-char truncation in CLI/REPL display
34. Binary values: `<binary N bytes>` default, hex with flag
35. Web UI: `[binary: N bytes]` with copy-hex button
36. Run `just check`

### Phase 8: Tests
37. Unit tests: `TagValue`, search parser, auto-tag, migration, binary encode/decode
38. Integration tests: tag binary round-trip, search syntax, migrate-tags, auto-tag on put
39. Web tests (vitest): search component, binary display
40. Playwright tests if applicable: E2E search, upload auto-tag
41. Run `just check`

### Phase 9: Documentation & Final Verification
42. Update all Rust docstrings, comments, help text
43. Update all TypeScript docstrings and comments
44. Review and update: README.md, WEB.md, TODO.md, ARCHITECTURE.md, AGENTS.md, web/README.md
45. Run `just check`
46. Run `just kill-serve` (build + deploy)
47. Run `nix flake check -L` — fix any errors
48. Final `just check` confirmation

---

## 6. Expected End Result

- **Tag keys and values** accept arbitrary bytes, stored via tuple `bytes()` encoding
- **Display** tries UTF-8, falls back to `<binary N bytes>`, honors `--hex` flag
- **CLI and REPL** are exact mirrors: same commands, same aliases, same flags, same help text
- **Search syntax** works everywhere: `key:`, `:value`, `key:value`, `"literal"`, bare words
- **File uploads** auto-tag with `name`, `file`, `path` metadata
- **Display names** in UI use name/file/path tag values instead of raw hashes
- **Dedup** in listings: identical hash + tags + name collapsed
- **Migration** command backfills name/file/path for existing files
- **Web UI** has search box, binary value display, display names
- **All tests pass**: `just check`, `nix flake check -L`
- **All docs current**: docstrings, comments, help text, .md files

---

## 7. Todo List

- [ ] **P1** Create `TagValue` type in `tags.rs` with bytes, UTF-8 try, display, serde
- [ ] **P1** Update `Tag` struct and all `tags.rs` methods to use `TagValue`
- [ ] **P1** Update tuple encoding in `tags.rs` to use `enc.bytes()` instead of `enc.string()`
- [ ] **P1** Update `MetaRequest`/`MetaResponse` in `protocol.rs` from `String` to `Vec<u8>`
- [ ] **P1** Update `commands/tag.rs` and `commands/serve.rs` for new types
- [ ] **P2** Add `find` alias and `migrate-tags` command to REPL (`runner.rs`)
- [ ] **P2** Verify and fix all CLI/REPL alias and help text parity
- [ ] **P2** Implement `SearchQuery` parser in `tags.rs` (key:, :value, key:value, "quoted")
- [ ] **P2** Wire search syntax through CLI, REPL, and web API
- [ ] **P3** Add `auto_tag()` in `tags.rs` — creates name/file/path tags
- [ ] **P3** Call `auto_tag()` from `commands/put.rs` after successful store
- [ ] **P3** Add `migrate_tags()` and `MigrateTags` command to CLI/REPL/protocol
- [ ] **P3** Update web API types for base64 byte fields
- [ ] **P3** Add `/api/tags/search` endpoint with query syntax
- [ ] **P4** Update web frontend: search box, binary display, display names, dedup
- [ ] **P4** Add `--hex`/`--binary` CLI flags; implement 256-char truncation
- [ ] **P5** Write unit tests: TagValue, search parser, auto-tag, migration, binary encode/decode
- [ ] **P5** Write integration tests: binary round-trip, search syntax, migrate-tags, auto-tag on put
- [ ] **P5** Write web tests (vitest) and Playwright tests as needed
- [ ] **P6** Update all Rust docstrings, comments, help text
- [ ] **P6** Update all TypeScript docstrings and comments
- [ ] **P6** Review and update all .md files (README, WEB, TODO, ARCHITECTURE, AGENTS, web/README)
- [ ] **P6** Run `just check`, `just kill-serve`, `nix flake check -L` — fix all errors
