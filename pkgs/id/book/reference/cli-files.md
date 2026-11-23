# File commands: put, get, find and kin

Source: `pkgs/id/crates/id-cli/src/cli/mod.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/ — provenance only; this chapter is complete without following it.

This chapter covers the commands that store, retrieve and search for files: `put`, `put-hash`, `send`, `get`, `get-hash`, `cat`, `list` and `migrate-tags`, followed by the search family `find`, `search`, `show` and `peek`. The global flags accepted by every command are defined in [The id command line](cli.md). Tags attached to stored files are covered in [The tag family](cli-tags.md).

## The remote-node rule

Every command in this chapter except `put-hash`, `send` and `migrate-tags` accepts a remote target by position:

```
id get 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef config.json
```

If the first positional argument is a 64-character hex node ID, it is consumed as the remote node and the rest of the arguments operate on that node. A first positional that is not 64 hex characters is treated as a local file, name or query. Each of these commands also defines `--no-relay`, which disables relay servers for the remote operation and restricts the connection to direct paths. The search family additionally accepts `--node <NODE_ID>` as an explicit alternative to the positional rule.

A malformed node ID where a node ID is parsed fails with `invalid node ID: must be 64 hex characters` and exit 1.

## `put`

```
id put [FILES]...
id put --content NAME
id in|add|store|import [FILES]...
```

Aliases: `in`, `add`, `store`, `import`.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--content` (visible alias `--data`) | none (switch) | off | Read the content from stdin instead of from file paths. Requires exactly one name argument. Conflicts with `--stdin`. |
| `--stdin` | none (switch) | off | Read more file paths from stdin, split on newline, tab or comma. Conflicts with `--content`. |
| `--hash-only` | none (switch) | off | Store the content without creating a named tag. The content is retrievable by hash only. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote operation. |

Behaviour notes:

- Each `FILES` entry is a path. The form `path:name` renames the file during storage: the bytes come from `path` and the stored name is `name`.
- With `--content`, the single positional argument is the stored name, not a path. Supplying zero or more than one name fails with `--content requires exactly one name argument`. Discrepancy CLI-14: the `--help` summary line for `--content` ("Read content from stdin instead of file paths") omits the one-name requirement; the requirement is stated in the field documentation and enforced by the handler.
- With no files and no stdin input, `put` fails with `no files provided`.
- When several files are stored and some fail, the failures are collected into one aggregated error, `some puts failed:` followed by the individual failures, and the command exits 1.

Worked example (toy data):

```
$ printf 'rice, lentils, tea\n' > shopping.txt
$ id put shopping.txt
$ id put shopping.txt:pantry-list.txt
$ printf 'oats, honey\n' | id put --content staples.txt
$ id list
pantry-list.txt
shopping.txt
staples.txt
```

Failure shapes: a missing input file is a handler error (exit 1); `--content` combined with `--stdin` is a clap conflict (exit 2); `--content` with the wrong number of names fails with `--content requires exactly one name argument` (exit 1).

## `put-hash`

```
id put-hash <SOURCE>
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| (none) | — | — | `put-hash` defines no flags of its own; the global flags apply. |

Behaviour notes:

- `SOURCE` is a single file path, or `-` for stdin. The content is stored by hash only, with no named tag — the single-source form of `put --hash-only`.
- `put-hash` takes no remote-node positional: its one argument is always the source.

Worked example (toy data):

```
$ printf 'release checklist v3\n' | id put-hash -
```

Failure shapes: a missing source file is a handler error (exit 1). A missing `SOURCE` argument is a clap error (exit 2).

## `send`

```
id send <SOURCE_NODE> <NAME> --to <DEST_NODE> [--as <NAME>] [--replace] [--no-relay]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--to` | `DEST_NODE` | required (no default) | Destination server node ID (64 hex characters). |
| `--as` | `NAME` | the source name | Name to register on the destination. |
| `--replace` | none (switch) | off | Replace an existing, different destination file; the old destination file is archived under an archive name first. |
| `--no-relay` | none (switch) | off | Disable relay servers for the connection to the source. |

Behaviour notes:

- `send` is a server-to-server copy. The source server performs the transfer with its own identity: it pushes the file's blob to the destination and registers the name there. No file bytes pass through the client running `send`.
- The destination's write policy must allow the source server's node ID; the source node is recorded as the copy's origin.
- An invalid destination node ID fails with `invalid node ID: must be 64 hex characters`.

Worked example (toy data; both node IDs are invented):

```
$ id send 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef report.txt --to fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 --as summary.txt
```

Failure shapes: omitting `--to` is a clap error (exit 2). A name that does not exist on the source, or a destination that refuses the source's writes, is a handler error (exit 1).

## `get`

```
id get [SOURCES]...
id get --hash [SOURCES]...
id get --name-only [SOURCES]...
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--stdin` | none (switch) | off | Read more sources from stdin, split on newline, tab or comma. |
| `--hash` | none (switch) | off | Treat all sources as hashes. A source that matches no known hash fails. Conflicts with `--name-only`. |
| `--name-only` | none (switch) | off | Treat all sources as names only; hash interpretation is skipped even for 64-character hex strings. Conflicts with `--hash`. |
| `--stdout` | none (switch) | off | Write all retrieved files to stdout, concatenated. Overrides the per-item output specifications. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote operation. |

Behaviour notes:

- Each source is a name or a hash. Source resolution tries, in order: exact tag name; then hash, if the source is 64 hex characters. `--hash` and `--name-only` force one interpretation.
- The form `source:output` writes that item to `output` instead of to a file of the source's name. An output of `-` means stdout.
- With no sources and no stdin input, `get` fails with `no sources provided`.
- When several sources are retrieved and some fail, the failures are collected into one aggregated error, `some gets failed:` followed by the individual failures, and the command exits 1.

Worked example (toy data):

```
$ id get shopping.txt
$ id get shopping.txt:groceries-copy.txt
$ id get shopping.txt:-
rice, lentils, tea
```

Failure shapes: a name or hash with no stored content fails with `file not found` locally, or `file not found on remote` for a remote operation (exit 1). `--hash` together with `--name-only` is a clap conflict (exit 2).

## `get-hash`

```
id get-hash <HASH> <OUTPUT>
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| (none) | — | — | `get-hash` defines no flags of its own; the global flags apply. |

Behaviour notes:

- A shortcut for `get --hash`, with the output path as a required second positional. `OUTPUT` may be `-` for stdout.
- `HASH` is the 64-hex-character blob hash.

Worked example (toy data; the hash is invented):

```
$ id get-hash fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 checklist.txt
```

Failure shapes: a hash with no stored content is a handler error (exit 1). A missing `HASH` or `OUTPUT` is a clap error (exit 2).

## `cat`

```
id cat [SOURCES]...
id output|out [SOURCES]...
```

Aliases: `output`, `out`.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--stdin` | none (switch) | off | Read more sources from stdin, split on newline, tab or comma. |
| `--hash` | none (switch) | off | Treat all sources as hashes. Conflicts with `--name-only`. |
| `--name-only` | none (switch) | off | Treat all sources as names only, skipping hash interpretation. Conflicts with `--hash`. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote operation. |

Behaviour notes:

- `cat` is `get` forced to stdout: sources resolve by the same name-then-hash rule, and the content is written to stdout. `cat` has no `--stdout` flag, because stdout is its only destination.

Worked example (toy data):

```
$ id cat shopping.txt
rice, lentils, tea
```

Failure shapes: a missing source fails with `file not found` or `file not found on remote` (exit 1). `--hash` together with `--name-only` is a clap conflict (exit 2).

## `list`

```
id list [NODE]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--no-relay` | none (switch) | off | Disable relay servers for a remote listing. |

Behaviour notes:

- Lists all stored files (names) in the local store, or in the remote node's store when `NODE` (a 64-character hex node ID) is given.
- Discrepancy CLI-1: `list` has no `ls` alias in the clap parser. `id ls` fails with `error: unrecognized subcommand 'ls'` and the tip "a similar subcommand exists: 'list'" (exit 2), even though the README command table lists `ls` as an alias. The `ls` spelling exists only inside the REPL, and as the alias of `tag list` (see [The tag family](cli-tags.md)).

Worked example (toy data):

```
$ id list
pantry-list.txt
shopping.txt
staples.txt
```

Failure shapes: an unreachable remote node is a handler error (exit 1). `id ls` is a clap error (exit 2).

## `migrate-tags`

```
id migrate-tags
id migrate
```

Alias: `migrate`.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| (none) | — | — | `migrate-tags` takes no arguments and defines no flags of its own; the global flags apply. |

Behaviour notes:

- Scans all blob tags in the store and adds the `name` and `file` auto-tags to any subject that does not already have them. Existing tags are never overwritten.
- The command reports the number migrated, in the form `migrated 42 files`.

Worked example (toy data):

```
$ id migrate-tags
migrated 3 files
```

Failure shapes: a store that cannot be opened is a handler error (exit 1).

## Search-family shared semantics

`find`, `search`, `show` and `peek` share one matching model:

- Queries are case-insensitive and match against both names and hashes.
- Match quality is ordered: exact match ranks above prefix match, which ranks above contains (substring) match.
- Discrepancy CLI-8: there is no glob or wildcard matching anywhere in the search implementation. A query of `*.rs` is the literal two-character substring `*.` followed by `rs`; in practice it matches nothing, despite the README usage example `id find "*.rs"` being labelled "Find files by pattern".
- `--first [N]` and `--last [N]` are optional-value flags on all four commands. Each takes zero or one value (`num_args = 0..=1`); a bare `--first` or `--last` means `1`. The two flags conflict with each other. A value of `0` is rejected by the value parser with `zero is not a valid limit: use at least 1` (clap error, exit 2); a non-numeric value fails in the same parser.
- `--exclude <PAT>` is repeatable on all four commands and drops matches whose name or hash contains the pattern.
- `--name` is accepted by all four commands and makes name matches rank above hash matches.
- `--node <NODE_ID>` names a remote node explicitly, as an alternative to the positional rule; `--no-relay` applies to that remote search.
- A query with no matches fails in the CLI handlers with `no matches found for: <queries>` (exit 1). Inside the REPL the same condition prints a line and the session continues; see [The interactive REPL](repl.md).

### `--format` and the REPL difference

`--format` exists on `find` and `search` only:

| Value | Meaning |
|---|---|
| `tag` | Each match printed with its query. |
| `group` | Matches grouped by query. |
| `union` | Matches deduplicated by hash. |

The clap default is `tag`. Discrepancy CLI-11: the REPL's `find` and `search` default to `union`, because the REPL hand parser enters with `union` as its default format. The same query therefore prints tag-grouped output from the command line and union-deduplicated output in the REPL, and neither side's help mentions the other's default.

## `find`

```
id find <QUERIES>...
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--name` | none (switch) | off | Prefer name matches over hash matches. |
| `--stdout` | none (switch) | off | Write the best match to stdout instead of to a file. |
| `--all` (visible aliases `--out`, `--export`, `--save`, `--full`) | none (switch) | off | Output all matches instead of only the best match. |
| `--dir` | `DIR` | none | Output directory for `--all`; each match is saved by name. |
| `--format` | `tag`, `group` or `union` | `tag` | Output format (see the `--format` table above). |
| `--first` | `N` (optional) | bare flag = `1` | Return only the first N matches. Conflicts with `--last`. |
| `--last` | `N` (optional) | bare flag = `1` | Return only the last N matches. Conflicts with `--first`. |
| `--count` | none (switch) | off | Print the count of matches instead of the matches. |
| `--exclude` | `PAT` | none | Exclude matches whose name or hash contains `PAT`. Repeatable. |
| `--node` | `NODE_ID` | none | Remote node to search. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote search. |

Behaviour notes:

- Default output mode: the best match for each query is written to a file of its own name. `--stdout` sends the best match to stdout instead; `--all` writes every match (to stdout, or into `--dir` by name).
- Multiple queries are accepted; each query is matched separately.

Worked example (toy data):

```
$ id find shopping
$ id find --count shop
2
$ id find --first 1 --stdout pantry
rice, lentils, tea
```

Failure shapes: no matches fails with `no matches found for: <queries>` (exit 1). `--first 0` is a value-parser error (exit 2). `--first` together with `--last` is a clap conflict (exit 2).

## `search`

```
id search <QUERIES>...
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--name` | none (switch) | off | Prefer name matches over hash matches. |
| `--all` (visible aliases `--out`, `--export`, `--save`, `--full`) | none (switch) | off | Include all matches in the output. |
| `--dir` | `DIR` | none | Output directory for `--all`. |
| `--format` | `tag`, `group` or `union` | `tag` | Output format (see the `--format` table above). |
| `--first` | `N` (optional) | bare flag = `1` | Return only the first N matches. Conflicts with `--last`. |
| `--last` | `N` (optional) | bare flag = `1` | Return only the last N matches. Conflicts with `--first`. |
| `--count` | none (switch) | off | Print the count of matches instead of the matches. |
| `--exclude` | `PAT` | none | Exclude matches whose name or hash contains `PAT`. Repeatable. |
| `--node` | `NODE_ID` | none | Remote node to search. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote search. |

Behaviour notes:

- `search` takes the same flags as `find` except `--stdout`, and differs in effect: it lists matches and retrieves nothing.

Worked example (toy data):

```
$ id search shop
shopping.txt
$ id search --format group shop pantry
```

Failure shapes: identical to `find` — no matches exits 1 with `no matches found for: <queries>`; `--first 0` and `--first` with `--last` exit 2.

## `show`

```
id show <QUERIES>...
id view <QUERIES>...
```

Alias: `view`.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--name` | none (switch) | off | Prefer name matches over hash matches. |
| `--all` | none (switch) | off | Output all matches instead of only the first. |
| `-o`, `--output` | `FILE` | stdout | Write the matched content to `FILE` instead of stdout. |
| `--first` | `N` (optional) | bare flag = `1` | Return only the first N matches. Conflicts with `--last`. |
| `--last` | `N` (optional) | bare flag = `1` | Return only the last N matches. Conflicts with `--first`. |
| `--exclude` | `PAT` | none | Exclude matches whose name or hash contains `PAT`. Repeatable. |
| `--node` | `NODE_ID` | none | Remote node to search. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote search. |

Behaviour notes:

- `show` is `cat` over the search matcher: it outputs the content of the first (best) match for each query, or of every match with `--all`.
- `show` has no `--count` and no `--format` flag.

Worked example (toy data):

```
$ id show shopping
rice, lentils, tea
$ id show --first 2 -o combined.txt shop
```

Failure shapes: no matches fails with `no matches found for: <queries>` (exit 1). `--first 0` and `--first` with `--last` exit 2.

## `peek`

```
id peek <QUERIES>...
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--name` | none (switch) | off | Prefer name matches over hash matches. |
| `-n`, `--lines` | `N` | `5` | Number of head and tail units to show. Zero is rejected by the value parser. |
| `--head-only` | none (switch) | off | Show only the head. Conflicts with `--tail-only`. |
| `--tail-only` | none (switch) | off | Show only the tail. Conflicts with `--head-only`. |
| `--chars` | none (switch) | off | Count by characters instead of lines. Conflicts with `--words`. |
| `--words` | none (switch) | off | Count by words instead of lines. Conflicts with `--chars`. |
| `-q`, `--quiet` | none (switch) | off | Suppress the header banner; print content only. |
| `-o`, `--output` | `FILE` | stdout | Write the preview to `FILE` instead of stdout. |
| `--all` | none (switch) | off | Peek all matches instead of only the first per query. |
| `--first` | `N` (optional) | bare flag = `1` | Return only the first N matches. Conflicts with `--last`. |
| `--last` | `N` (optional) | bare flag = `1` | Return only the last N matches. Conflicts with `--first`. |
| `--exclude` | `PAT` | none | Exclude matches whose name or hash contains `PAT`. Repeatable. |
| `--node` | `NODE_ID` | none | Remote node to search. |
| `--no-relay` | none (switch) | off | Disable relay servers for a remote search. |

Behaviour notes:

- The default preview shows a header banner, then 5 head lines and 5 tail lines; content of 10 lines or fewer is shown in full.
- `--lines` sets the head and tail size together. With `--chars` or `--words`, the same number counts characters or words instead of lines.
- `--lines 0` is rejected by the same value parser as `--first`/`--last`, with `zero is not a valid limit: use at least 1`.

Worked example (toy data):

```
$ id peek --lines 2 shopping
rice, lentils, tea
$ id peek --head-only --lines 20 --quiet staples
oats, honey
```

Failure shapes: no matches fails with `no matches found for: <queries>` (exit 1). `--lines 0` is a value-parser error (exit 2). `--head-only` with `--tail-only`, `--chars` with `--words`, and `--first` with `--last` are clap conflicts (exit 2).

## Exit behaviour for this chapter

All commands in this chapter follow the process-wide rules in [The id command line](cli.md): success exits 0, a handler error prints `Error: <message>` on stderr and exits 1, and a clap parse error (unknown flag, conflict, invalid value) exits 2. The multi-file commands `put` and `get` aggregate partial failures into a single exit-1 error (`some puts failed:` / `some gets failed:`) rather than exiting on the first failure.
