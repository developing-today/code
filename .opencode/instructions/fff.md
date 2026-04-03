Use `fff_*` MCP tools for file search and grep operations. FFF provides frecency-ranked, git-aware results — frequently/recently opened files are boosted, git-dirty files are prioritized. Prefer fff tools over built-in glob/grep when searching for files or code patterns. If fff tools fail or return errors, fall back to built-in tools (grep, glob, read).

## Tools

| Tool | Use for |
|---|---|
| `fff_grep` | **Default tool.** Search file contents — definitions, usage, patterns. Use when you have a specific identifier or pattern. |
| `fff_find_files` | Explore which files/modules exist for a topic. Use when you DON'T have a specific identifier or are looking for a file. |
| `fff_multi_grep` | OR logic across multiple patterns in one call. Use for case variants (e.g. `['PrepareUpload', 'prepare_upload']`) or searching 2+ identifiers at once. |

## Core Rules

1. **Search BARE IDENTIFIERS only.** Grep matches single lines. Search for ONE identifier per query:
   - `'InProgressQuote'` — finds definition + all usages
   - `'ActorAuth'` — finds enum, struct, all call sites
   - NOT `'load.*metadata.*InProgressQuote'` (complex regex, 0 results)
   - NOT `'struct ActorAuth'` (too specific, misses enums/traits/type aliases)

2. **NEVER use regex** unless you truly need alternation. Plain text is faster and more reliable. For OR logic, use `fff_multi_grep` with literal patterns.

3. **Stop searching after 2 greps — READ the code.** After 2 grep calls, you have enough file paths. Read the top result. More greps != better understanding.

4. **Use multi_grep for multiple identifiers.** One `fff_multi_grep(['ActorAuth', 'PopulatedActorAuth', 'actor_auth'])` instead of 3 sequential greps.

## Constraint Syntax

Constraints go inline, prepended before the search text (for grep) or in the separate `constraints` parameter (for multi_grep).

| Format | Example | Meaning |
|---|---|---|
| Extension | `*.rs`, `*.{ts,tsx}` | Filter by file type |
| Directory | `src/`, `quotes/` | Filter to directory |
| Filename | `schema.rs`, `src/main.rs` | Filter to specific file |
| Exclude | `!test/`, `!*.spec.ts` | Exclude matches |

Bare words without extensions are NOT constraints. `'quote TODO'` searches for literal text "quote TODO", not TODO in quote files. Use `'quotes/ TODO'` instead.

## Output Format

Grep results auto-expand definitions with body context (struct fields, function signatures). Lines marked with `|` are definition body context. `[def]` marks definition files. `->` Read suggestions point to the most relevant file.

## When to use fff vs codedb

- **fff**: File search, grep, fuzzy matching, frecency-ranked results — "find me the right file fast"
- **codedb**: Structured queries — symbol definitions, dependency graphs, outlines, tree views
- Both complement each other. Use fff for search, codedb for structural navigation.
