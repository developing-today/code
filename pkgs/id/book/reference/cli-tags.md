# The tag family

Source: `pkgs/id/crates/id-cli/src/cli/tag.rs` and `pkgs/id/crates/id-cli/src/cli/mod.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/ — provenance only; this chapter is complete without following it.

`id tag` manages metadata tags: key/value pairs attached to stored files. The parent command has two aliases, `label` and `link`, so every command in this chapter also runs as `id label …` and `id link …`.

## Storage note

Tags are stored in iroh-docs CRDT documents and replicate conflict-free across peers. A tag is a triple: a subject (the file), a key, and an optional value. Multiple values can exist under one key on one file, and a key can also exist with no value at all (a key-only tag).

Tags are a different mechanism from claims. The `claim` family records signed statements about subjects in the event store; the `tag` family records file metadata in the docs store. Claims are covered in [Groups, documents, claims and keys](cli-groups-identity.md); the file commands that create the subjects tags attach to are covered in [File commands: put, get, find and kin](cli-files.md).

Display rules shared by `tag list` and `tag search`:

- A non-UTF-8 value is displayed as `<binary N bytes>` by default, where `N` is the byte count.
- `--hex` displays a non-UTF-8 value as hex (`0x...`) instead.
- `--binary` displays a non-UTF-8 value as lossy text instead.
- Values are truncated at 256 bytes by default; `--no-truncate` disables the truncation.

## `tag set`

```
id tag set <FILE> <KEY> [VALUE]
id tag add <FILE> <KEY> [VALUE]
```

Alias: `add`.

| Flag           | Argument | Default | Meaning                                                                                                          |
| -------------- | -------- | ------- | ---------------------------------------------------------------------------------------------------------------- |
| `--value-hex`  | `HEX`    | none    | Tag value as hex-encoded raw bytes (binary-safe). Conflicts with the positional `VALUE` and with `--value-file`. |
| `--value-file` | `PATH`   | none    | Read the raw tag value from `PATH`; `-` reads stdin. Conflicts with `--value-hex`.                               |

Behaviour notes:

- With a positional `VALUE`, the tag is `(FILE, KEY, VALUE)`. With no value from any source, the tag is key-only: `(FILE, KEY)` with no value.
- Setting a second value under an existing key adds it; multiple values per key are stored side by side. Setting a tag never removes other values of the same key.
- The value source is exactly one of: positional `VALUE`, `--value-hex`, `--value-file`, or none.

Worked example (toy data):

```
$ id put field-notes.txt
$ id tag set field-notes.txt priority high
$ id tag set field-notes.txt pinned
$ printf '\x01\x02' > sig.bin
$ id tag set field-notes.txt marker --value-file sig.bin
$ id tag list field-notes.txt
priority: high
pinned
marker: <binary 2 bytes>
```

Failure shapes: `--value-hex` together with a positional value or with `--value-file` is a clap conflict (exit 2). Missing `FILE` or `KEY` positionals are clap errors (exit 2). Handler errors exit 1 under the process-wide rules in [The id command line](cli.md); whether `set` requires the file to already exist in the store is unverified against the source for this chapter.

## `tag del`

```
id tag del <FILE> <KEY> [VALUE]
```

Aliases: `rm`, `remove`, `rem`, `delete`, `unset`.

| Flag          | Argument | Default | Meaning                                                                                    |
| ------------- | -------- | ------- | ------------------------------------------------------------------------------------------ |
| `--value-hex` | `HEX`    | none    | Specific value to remove, as hex-encoded raw bytes. Conflicts with the positional `VALUE`. |

Behaviour notes:

- With a value (positional or `--value-hex`), `del` removes exactly that `(KEY, VALUE)` pair from the file.
- With no value, `del` removes every value stored under the key, plus the key-only tag for that key if one exists.

Worked example (toy data):

```
$ id tag set field-notes.txt priority high
$ id tag set field-notes.txt priority low
$ id tag del field-notes.txt priority high
$ id tag list field-notes.txt
priority: low
$ id tag del field-notes.txt priority
$ id tag list field-notes.txt
```

The first `del` removes only `priority=high` and leaves `priority=low`. The second, with no value, removes the key entirely.

Failure shapes: `--value-hex` together with a positional value is a clap conflict (exit 2). Missing `FILE` or `KEY` positionals are clap errors (exit 2). Handler errors exit 1 under the process-wide rules in [The id command line](cli.md).

## `tag list`

```
id tag list [FILE]
id tag ls [FILE]
```

Alias: `ls`.

| Flag            | Argument      | Default | Meaning                                                                  |
| --------------- | ------------- | ------- | ------------------------------------------------------------------------ |
| `--hex`         | none (switch) | off     | Show non-UTF-8 values as hex (`0x...`) instead of `<binary N bytes>`.    |
| `--binary`      | none (switch) | off     | Show non-UTF-8 values as lossy text instead of `<binary N bytes>`.       |
| `--no-truncate` | none (switch) | off     | Do not truncate long values; the display default truncates at 256 bytes. |

Behaviour notes:

- With `FILE`, lists the tags on that file. Without `FILE`, lists all tags in the store.
- The `ls` alias belongs to `tag list` itself. It does not extend to the top-level `list` command, which has no `ls` alias (discrepancy CLI-1, covered in [File commands: put, get, find and kin](cli-files.md)).

Worked example (toy data):

```
$ id tag list
field-notes.txt  priority: high
field-notes.txt  pinned
$ id tag ls field-notes.txt
priority: high
pinned
```

Failure shapes: handler errors exit 1 under the process-wide rules in [The id command line](cli.md). The display flags never change what is stored; they change only the rendering of non-UTF-8 and long values.

## `tag search`

```
id tag search <QUERY>...
id tag find <QUERY>...
```

Alias: `find`. At least one query term is required.

| Flag            | Argument      | Default | Meaning                                                                  |
| --------------- | ------------- | ------- | ------------------------------------------------------------------------ |
| `--hex`         | none (switch) | off     | Show non-UTF-8 values as hex (`0x...`) instead of `<binary N bytes>`.    |
| `--binary`      | none (switch) | off     | Show non-UTF-8 values as lossy text instead of `<binary N bytes>`.       |
| `--no-truncate` | none (switch) | off     | Do not truncate long values; the display default truncates at 256 bytes. |

Behaviour notes — the structured query syntax. Each query term takes one of five forms:

| Term form   | Matches                                                                  |
| ----------- | ------------------------------------------------------------------------ |
| `key:`      | Tags whose key is `key` (any value, or key-only).                        |
| `:value`    | Tags whose value is `value` (any key).                                   |
| `key:value` | Tags with exactly that key and that value.                               |
| `"literal"` | Tags where the literal text occurs in any field (subject, key or value). |
| `bare`      | Case-insensitive search across subject, key and value.                   |

- Multiple terms are ANDed: a tag must satisfy every term to be returned.
- Quoted strings are accepted in the key and value positions of a `key:value` term: `"key:":":value"` matches the key `key:` with the value `:value`.

Worked example (toy data):

```
$ id tag search priority:
field-notes.txt  priority: high
$ id tag search :high
field-notes.txt  priority: high
$ id tag search priority:high
field-notes.txt  priority: high
$ id tag search "field-notes"
field-notes.txt  priority: high
field-notes.txt  pinned
$ id tag search priority pinned
```

The last search returns nothing: `priority` and `pinned` are separate tags on the file, and ANDed terms must all be satisfied by one tag.

Failure shapes: running `tag search` with no query term is a clap error (exit 2), because the parser requires at least one term. The exit behaviour of a search that matches nothing is unverified against the source for this chapter; the file search family, by contrast, fails on no matches (see [File commands: put, get, find and kin](cli-files.md)).

## Exit behaviour for this chapter

All commands in this chapter follow the process-wide rules in [The id command line](cli.md): success exits 0, a handler error prints `Error: <message>` on stderr and exits 1, and a clap parse error (missing required argument, conflicting value sources) exits 2.
