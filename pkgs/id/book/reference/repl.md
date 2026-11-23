# The interactive REPL

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/repl/runner.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/repl/input.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/commands/repl.rs

The interactive REPL is a second command language inside the `id` binary. It does not reuse the clap parser described in [The id command line](cli.md). A line is whitespace-split and matched against literal patterns in `runner.rs`, so clap flags, clap defaults and `--help` do not exist inside the REPL except where a hand parser re-implements them. Global flags such as `--data-dir` must be given to `id repl` itself, before entering; they cannot be set from inside a session.

## Entry

| Invocation | Behaviour |
|---|---|
| `id` (no subcommand) | Starts the REPL with no session target. |
| `id repl` | Starts the REPL with no session target. |
| `id repl [NODE]` | Starts the REPL with `NODE` as the session-level remote target. `NODE` must be a 64-character hex node ID. |
| `id shell` | Hidden alias for `id repl`. |

An invalid session target fails startup before the prompt appears:

```text
Error: invalid node ID: must be 64 hex characters
```

Startup failures of this kind, and failures to open the store, exit with status `1`, like any other `id` handler error. Once the prompt is running, command failures never change the exit status; see "Error behaviour" below.

## Modes

The mode is detected at startup and printed in the banner as `id repl (<mode>)`.

| Mode | Condition | Where operations run |
|---|---|---|
| `Local` | No serve is running for the data directory. | Directly against the local store. |
| `Remote` | The serve lock `.iroh-serve.lock` is present in the data directory. | Proxied to the local `id serve` process. |
| `RemoteNode` | Started as `id repl <NODE_ID>` with a valid node ID. | Against that remote node for the whole session. |

The session target is fixed at startup. In local-serve (`Remote`) mode, individual commands can additionally be aimed at another node with the `@NODE_ID` syntax described below.

### The banner, and what it omits (CLI-12)

At startup the REPL prints a one-line command summary:

```text
commands: list, put, get, cat, gethash, peers, help, quit
```

That line lists 8 commands. The dispatch in `runner.rs` implements 20 command groups. The banner omits `delete`, `rename`, `copy`, `send`, `find`, `search`, `show`, `peek`, the whole `tag` family, `tags`, and `migrate-tags`, all of which work and all of which the REPL's own `help` command lists. This is disagreement CLI-12: the banner is the incomplete side; the dispatch table in this chapter is the complete one.

## Command list

In the table, ◐ marks the commands that also accept an `@NODE_ID` target as the second token. All other commands run against the session target only.

| Command | Arguments | Behaviour |
|---|---|---|
| `list` / `ls` ◐ | — | List stored files. |
| `put <FILE> [NAME]` ◐ | File path, optional stored name | Store a file. `NAME` defaults to the file name. Only the `put` spelling accepts `@NODE_ID`; the `in` alias does not. |
| `in <FILE> [NAME]` | File path, optional stored name | Alias of `put` (no `@NODE_ID` form). |
| `get <NAME> [OUTPUT]` ◐ | Stored name, optional output path | Fetch a file. `OUTPUT` defaults to `NAME`; `-` writes to stdout. |
| `cat <NAME>` ◐ | Stored name | Print a file to stdout. Only the `cat` spelling accepts `@NODE_ID`. |
| `output <NAME>` / `out <NAME>` | Stored name | Aliases of `cat` (no `@NODE_ID` form). |
| `gethash <HASH> <OUTPUT>` | 64-hex content hash, output path | Fetch by hash. Both arguments are required. `-` writes to stdout. No `@NODE_ID` form. |
| `delete <NAME>` / `rm <NAME>` ◐ | Stored name | Delete a stored file. REPL-only: the clap CLI has no delete command. |
| `rename <FROM> <TO>` | Two stored names | Rename a stored file. REPL-only. No `@NODE_ID` form. |
| `copy <FROM> <TO>` / `cp <FROM> <TO>` | Two stored names | Copy a stored file to a new name. REPL-only. |
| `send <NAME> <DEST_NODE> [DEST_NAME]` | Stored name, destination node ID, optional destination name | Server-to-server copy: the source server pushes with its own identity and no bytes pass through this client. |
| `send --replace <NAME> <DEST_NODE> [DEST_NAME]` | As above | As above, archiving the old destination file first. `--replace` is recognised only in that exact second position. |
| `peers` ◐ | — | List peers. |
| `find <QUERY>... [flags]` | Queries plus hand-parsed flags | Find and fetch matches; see "Search commands" below. |
| `search <QUERY>... [flags]` | Queries plus hand-parsed flags | List matches; fetches nothing. |
| `show <QUERY>... [flags]` / `view ...` | Queries plus hand-parsed flags | Show match content. |
| `peek <QUERY>... [flags]` | Queries plus hand-parsed flags | Preview match content. |
| `tag set <FILE> <KEY> [VALUE]` | File, key, optional value | Set a file metadata tag. The head word may be `tag`, `label` or `link`; `set` may be `add`. |
| `tag del <FILE> <KEY> [VALUE]` | File, key, optional value | Delete a tag value, or every value for the key when `VALUE` is omitted. `del` may be `rm`, `remove`, `rem`, `delete` or `unset`. |
| `tag list [FILE]` / `tag ls [FILE]` | Optional file | List tags. |
| `tags [FILE]` / `labels [FILE]` / `links [FILE]` | Optional file | Bare forms of `tag list`. |
| `tag search <QUERY>...` / `tag find <QUERY>...` | Structured query terms | Search tags; same structured syntax as the clap `tag search` in [The tag family](cli-tags.md). A bare `tag search` with no query prints usage instead of erroring. |
| `migrate-tags` / `migrate` | — | Add the `name`/`file` auto-tags where missing; never overwrites. |
| `help` / `?` | — | Print the REPL help text. |
| `quit` / `exit` / `q` | — | Leave the REPL; exit status `0`. |
| `!<cmd>` | Shell command line | Shell escape; see below. |

### REPL-only commands

`delete`, `rename`, `copy`/`cp`, `gethash` (one word), the bare `tags` form, and the interactive match-selection prompt have no clap counterpart. The top-level CLI cannot delete, rename or copy a stored file at all; those operations exist only here and in the shared FileOps layer used by the web UI (see [Data stores and schemas](data-stores.md)).

Conversely, the REPL has no `serve`, `world`, `group`, `doc`, `claim`, `key`, `keyset`, `gallery` or `walk` commands.

### Tag commands: what the hand parser drops

The REPL tag commands accept no value-encoding flags. `--value-hex` and `--value-file` on `tag set`, `--value-hex` on `tag del`, and `--hex`, `--binary` and `--no-truncate` on `tag list` exist only in the clap commands in [The tag family](cli-tags.md).

## Search commands and their hand-parsed flags

`find` and `search` share one hand parser. `show` and `peek` have their own, smaller flag sets.

| Flag | `find` / `search` | `show` / `view` | `peek` |
|---|---|---|---|
| `--name` | yes | yes | — |
| `--all` | yes | yes | yes |
| `--first [N]` | yes | yes | yes |
| `--last [N]` | yes | yes | yes |
| `--count` | yes | — | — |
| `--exclude <PAT>` | yes | yes | yes |
| `--dir <DIR>` | yes (`find` output directory) | — | — |
| `--format <tag\|group\|union>` | yes | — | — |
| `--file` | yes | — | — |
| `>FILE` redirect token | yes | — | yes |
| `-o <FILE>` | — | yes | yes |
| `--lines <N>` / `-n <N>` | — | — | yes; default `5` |
| `--head-only` / `--tail-only` | — | — | yes |
| `--chars` / `--words` | — | — | yes |
| `--quiet` / `-q` | — | — | yes |

Queries are case-insensitive. Match quality is exact, then prefix, then contains, over name and hash. There is no glob matching.

### The format default differs from the clap CLI (CLI-11)

In the REPL, `find` and `search` default to `--format union`. The clap `id find` and `id search` commands default to `--format tag`. The same query therefore prints union-deduplicated output interactively and tag-grouped output non-interactively. Neither side's help mentions the other's default, and the REPL `help` text lists no `--format` flag at all even though the hand parser accepts one. This is disagreement CLI-11; the code behaviour is as stated here.

### Interactive match selection

With multiple matches and no `--all`, `find` and `search` prompt before fetching:

```text
select numbers (e.g. '1 3 5' or '1,2,3') or enter to cancel
```

Entering numbers selects those matches. Pressing Enter with no input cancels the fetch.

## Remote targeting with `@NODE_ID`

Syntax: `@<64-hex>` as the **second** token of the line.

```text
list @NODE_ID
put @NODE_ID FILE
get @NODE_ID NAME
cat @NODE_ID NAME
delete @NODE_ID NAME
peers @NODE_ID
```

Rules:

- Only `list`, `put`, `get`, `cat`, `delete` and `peers` accept the form.
- Any other command written with an `@NODE_ID` second token falls into a catch-all that reports it as unsupported for remote targeting.
- A malformed `@…` token is not treated as a target at all. It stays in the command words, so the line is then parsed as the plain command with that token as an ordinary argument.
- Per-command targeting works in local-serve mode. A session started as `id repl <NODE_ID>` already has its target fixed; see "Modes" above.

## The input language: substitution, pipes, here-strings, heredocs

Before dispatch, `preprocess_repl_line` in `input.rs` rewrites a line in this order: heredoc detection, then here-string, then the `|>` pipe, then `$(…)` and backtick substitution. The scans are quote- and nesting-aware: a `<<` or `|>` inside a `$(…)` or backtick span is shell source, not REPL syntax.

| Form | Example | Behaviour |
|---|---|---|
| Command substitution | `put $(date +%F) name` | Runs `sh -c` on the inner command, captures stdout, and stores it under `name`. The captured text travels through the line as a `__STDIN_CONTENT__:<text>` marker token. |
| Backtick substitution | `` put `date +%F` name `` | Same capture as `$(…)`. |
| Pipe-in | `cmd \|> put - name` | Runs the left side as a shell command; its output becomes the `put` content. The `-` placeholder is the content slot. Only the `\|>` spelling exists. |
| Here-string | `put - name <<< 'text'` | Stores the literal text under `name`. Single and double quotes around the text are stripped. |
| Heredoc | `put - name <<EOF` | Enters continuation mode; see below. |

Capture is bounded. Oversized output and a non-zero exit from the substituted command are errors (`ShellCaptureError`) and the `put` does not happen.

Content with no name is an error:

```text
error: content requires a name (e.g., put $(cmd) name.txt)
```

### Heredocs

`put - name <<EOF` returns a need-more-input state. The runner then reads continuation lines and shows the prompt note:

```text
(heredoc: type 'EOF' on its own line to end, Ctrl+C to cancel)
```

- The delimiter may be quoted: `<<'EOF'` or `<<"EOF"`.
- Trailing text after the delimiter is an error: `unexpected trailing text after heredoc delimiter`.
- An empty delimiter is an error: `heredoc delimiter cannot be empty`.
- Ctrl+C cancels the heredoc. Ctrl+D ends it with what was read.

### The pipe that does not exist (CLI-7)

The README's REPL section shows:

```text
> cat myfile.txt | head -5   # Pipe output to shell
```

That example does not work. The REPL implements exactly four operators — `<<DELIM` heredoc, `<<<` here-string, `|>` pipe-in, and `$(…)`/backtick capture — and dispatch splits on whitespace with no `|` handling. The line parses as `cat` with the extra arguments `|`, `head` and `-5`, matches no pattern, and is reported as an unknown command. To send REPL output to a shell tool, fetch the file with `get` first and run the tool outside the REPL, or use the shell escape below. This is disagreement CLI-7; the code behaviour is as stated here.

## Shell escape

A line beginning with `!` runs the rest of the line as a shell command:

```text
!ls -la
```

The command runs as `sh -c <cmd>` with **no** REPL preprocessing: no substitution, pipe or heredoc handling is applied to it. A non-zero shell exit prints:

```text
exit: <code>
```

The REPL continues after either outcome.

## Leaving the REPL

| Input | Behaviour |
|---|---|
| `quit`, `exit`, `q` | Exit, status `0`. |
| Ctrl+D (EOF) | Exit, status `0`. |
| Ctrl+C once | Warns. |
| Ctrl+C twice | Exits, status `0`. |
| Ctrl+C during a heredoc | Cancels the heredoc; the REPL stays open. |

History is provided by rustyline and is in-memory only. No history file is configured, so history does not survive the session.

## Error behaviour

- An unrecognised line prints `unknown command: <line>` followed by `type 'help' for available commands`, and the loop continues.
- A command that fails prints `error: <e>` and the loop continues.
- The REPL never exits non-zero because a command failed. The only non-zero exits are startup failures, which exit `1` before the prompt appears.

## Worked example

Toy data: a node whose ID is written here as `NODE_ID`, and two small text files.

```text
$ id repl
id repl (Local)
> put notes.txt shopping-list
> put $(printf 'milk\neggs\n') groceries.txt
> list
groceries.txt
shopping-list
> cat groceries.txt
milk
eggs
> printf 'bread\n' |> put - bakery.txt
> put - motto.txt <<< 'buy local'
> rename motto.txt slogan.txt
> copy slogan.txt slogan-copy.txt
> delete slogan-copy.txt
> tags slogan.txt
> !echo done
done
> quit
```

In a local-serve (`Remote`) session, the delete line could instead be aimed at another node as `delete @NODE_ID slogan-copy.txt`; in the `Local` session shown, that spelling would be reported as unsupported for remote targeting.
