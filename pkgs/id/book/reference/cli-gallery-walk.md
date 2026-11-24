# Gallery, match-audit and walk

Source: `pkgs/id/crates/id-cli/src/cli/gallery.rs`, `pkgs/id/crates/id-cli/src/cli/match_audit.rs`, `pkgs/id/crates/id-cli/src/cli/walk.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/cli/gallery.rs — provenance only; this chapter is complete without following it.

Three unrelated command groups share this chapter. `id gallery` administers the program gallery directory that `id serve --gallery-dir` serves. `id match-audit` exports and verifies tic-tac-toe match logs. `id walk` is the walking skeleton: stored nodes, signed records at `id://` addresses, sync between two nodes, and conflict reports.

Exit behaviour, all commands in this chapter unless a command states otherwise: success exits `0`; a handler error prints `Error: <message>` to stderr and exits `1`; a clap parse error (unknown subcommand or flag, missing required argument, conflicting flags, bad value) prints the error and usage to stderr and exits `2`.

Feature gating differs per group. `id gallery` and `id walk` are not feature-gated: both work in every build, including a `--no-default-features` build. `id match-audit` parses in every build but needs the `sandbox` cargo feature at dispatch: in a build without `sandbox` it fails with `this build has no sandbox support`, exit `1` — the same parse-everywhere, fail-at-dispatch pattern as `id world`, which parses without its feature and fails only at runtime (disagreement CLI-10, resolved; see [World commands: joining and playing](cli-worlds.md)).

## `gallery`

```
id gallery --dir <DIR> <SUBCOMMAND>
```

| Flag    | Argument       | Default                         | Meaning                                                                                                                                                                                                              |
| ------- | -------------- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--dir` | directory path | required (env `ID_GALLERY_DIR`) | The gallery directory: the one `serve --gallery-dir` serves (see [serve and publications](cli-serve.md)). It is a flag on the `gallery` parent, given before the subcommand. Environment fallback: `ID_GALLERY_DIR`. |

Behaviour common to all gallery subcommands: the subcommands edit the directory on disk. A running server reads the directory when it starts, so restart the server to serve a change. Admission is this endpoint's policy only: a removed record keeps verifying, keeps its bundle file here, and stays listed on any other host. Every removal, refusal and reinstatement is appended to the public transparency list.

Failure common to all gallery subcommands: a `--dir` that is not a gallery directory fails with `<dir> is not a gallery directory`, exit `1`.

The toy content hash used in the examples is the invented value `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`.

### `gallery remove`

Stops listing a record here and writes the removal to the public transparency list. The bundle file is kept, and the record stays valid on other hosts.

```
id gallery --dir <DIR> remove <CONTENT_HASH> --reason <REASON> [--note <TEXT>]
```

| Flag             | Argument                                 | Default             | Meaning                                                                                                                                                    |
| ---------------- | ---------------------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<CONTENT_HASH>` | 64 hex characters                        | required positional | The record's content hash.                                                                                                                                 |
| `--reason`       | `illegal_content\|malware\|spam\|policy` | required            | The reason class for the transparency entry. The parser also accepts the hyphen spelling `illegal-content`. Omitting `--reason` is a clap error, exit `2`. |
| `--note`         | text                                     | — (none)            | A short public note for the transparency list.                                                                                                             |

Example:

```
$ id gallery --dir ./toy-gallery remove \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --reason spam --note "toy example removal"
```

Failures: an unknown `--reason` value is a clap error, exit `2`. A malformed content hash fails with `invalid hash: expected 64 hex characters`, exit `1`.

### `gallery refuse`

Refuses a record waiting for review, publicly. The refusal goes on the transparency list, like a removal.

```
id gallery --dir <DIR> refuse <CONTENT_HASH> --reason <REASON> [--note <TEXT>]
```

| Flag             | Argument                                 | Default             | Meaning                                                                                                     |
| ---------------- | ---------------------------------------- | ------------------- | ----------------------------------------------------------------------------------------------------------- |
| `<CONTENT_HASH>` | 64 hex characters                        | required positional | The record's content hash.                                                                                  |
| `--reason`       | `illegal_content\|malware\|spam\|policy` | required            | The reason class for the transparency entry. The parser also accepts the hyphen spelling `illegal-content`. |
| `--note`         | text                                     | — (none)            | A short public note for the transparency list.                                                              |

Example:

```
$ id gallery --dir ./toy-gallery refuse \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --reason malware
```

Failures: an unknown `--reason` value is a clap error, exit `2`. A malformed content hash fails with `invalid hash: expected 64 hex characters`, exit `1`.

### `gallery reinstate`

Reverses a removal or refusal (an appeal upheld) and lists the record again. The reinstatement goes on the transparency list.

```
id gallery --dir <DIR> reinstate <CONTENT_HASH> [--note <TEXT>]
```

| Flag             | Argument          | Default             | Meaning                                        |
| ---------------- | ----------------- | ------------------- | ---------------------------------------------- |
| `<CONTENT_HASH>` | 64 hex characters | required positional | The record's content hash.                     |
| `--note`         | text              | — (none)            | A short public note for the transparency list. |

Example:

```
$ id gallery --dir ./toy-gallery reinstate \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --note "toy appeal upheld"
```

Failures: a malformed content hash fails with `invalid hash: expected 64 hex characters`, exit `1`. There is no `--reason` flag on this subcommand; passing one is a clap error, exit `2`.

### `gallery approve`

Lists a record waiting for review. Under the `reviewed` posture (see `gallery posture` below), approval is what makes a record listed.

```
id gallery --dir <DIR> approve <CONTENT_HASH>
```

| Flag             | Argument          | Default             | Meaning                    |
| ---------------- | ----------------- | ------------------- | -------------------------- |
| `<CONTENT_HASH>` | 64 hex characters | required positional | The record's content hash. |

This subcommand takes no flags of its own.

Example:

```
$ id gallery --dir ./toy-gallery approve \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Failures: a malformed content hash fails with `invalid hash: expected 64 hex characters`, exit `1`.

### `gallery pending`

Shows the records waiting for review.

```
id gallery --dir <DIR> pending
```

| Flag    | Argument       | Default                         | Meaning                                                         |
| ------- | -------------- | ------------------------------- | --------------------------------------------------------------- |
| `--dir` | directory path | required (env `ID_GALLERY_DIR`) | The parent `gallery` flag; the only flag this subcommand takes. |

Example:

```
$ id gallery --dir ./toy-gallery pending
```

Failures: only the common gallery failure applies (`<dir> is not a gallery directory`, exit `1`).

### `gallery transparency`

Shows the transparency list: every removal, refusal and reinstatement, in order, with the record, the time and the reason class.

```
id gallery --dir <DIR> transparency
```

| Flag    | Argument       | Default                         | Meaning                                                         |
| ------- | -------------- | ------------------------------- | --------------------------------------------------------------- |
| `--dir` | directory path | required (env `ID_GALLERY_DIR`) | The parent `gallery` flag; the only flag this subcommand takes. |

Example:

```
$ id gallery --dir ./toy-gallery transparency
```

Failures: only the common gallery failure applies (`<dir> is not a gallery directory`, exit `1`).

### `gallery posture`

Sets the admission posture.

```
id gallery --dir <DIR> posture <open|reviewed>
```

| Flag        | Argument         | Default             | Meaning                                                                                                                                                               |
| ----------- | ---------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<POSTURE>` | `open\|reviewed` | required positional | `open`: every verified record is listed until it is removed. `reviewed`: a record is listed once the operator approves it. Any other value is a clap error, exit `2`. |

Behaviour: the gallery's default posture, before any operator ruling is stored, is `open`. Switching posture changes what is listed at once, because a record's visibility is computed from the posture, the approvals and the transparency list.

Example:

```
$ id gallery --dir ./toy-gallery posture reviewed
```

Failures: an unknown posture value is a clap error, exit `2`.

### `gallery response-hours`

States, or withdraws, the response time published for one reason class.

```
id gallery --dir <DIR> response-hours <REASON> [HOURS]
```

| Flag       | Argument                                 | Default             | Meaning                                                                                               |
| ---------- | ---------------------------------------- | ------------------- | ----------------------------------------------------------------------------------------------------- |
| `<REASON>` | `illegal_content\|malware\|spam\|policy` | required positional | The reason class the statement covers. The parser also accepts the hyphen spelling `illegal-content`. |
| `[HOURS]`  | integer                                  | — (none)            | The published response time in hours. Omit it to withdraw the statement for that reason class.        |

Example — publish a 24-hour response statement for malware, then withdraw it:

```
$ id gallery --dir ./toy-gallery response-hours malware 24
$ id gallery --dir ./toy-gallery response-hours malware
```

Failures: an unknown reason class is a clap error, exit `2`.

### `gallery tag`

Sets the operator's own tags for a program or an author. Tags are operator speech, served apart from signed records.

```
id gallery --dir <DIR> tag [TAGS]... [--program <HASH> | --author <KEY>]
```

| Flag        | Argument          | Default                             | Meaning                                                                                                             |
| ----------- | ----------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `[TAGS]...` | tag strings       | — (none)                            | The tags to set, in the portable form: lower-case letters, digits and `-`. Giving no tags clears the target's tags. |
| `--program` | 64 hex characters | required unless `--author` is given | A program's content hash. Conflicts with `--author`.                                                                |
| `--author`  | public key        | — (none)                            | An author's public key. Conflicts with `--program`.                                                                 |

Example — tag a toy program, then clear its tags:

```
$ id gallery --dir ./toy-gallery tag toy-favourite editors-pick \
    --program 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
$ id gallery --dir ./toy-gallery tag \
    --program 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Failures: giving neither `--program` nor `--author` is a clap error, exit `2` (the handler's form of the same refusal is `name a --program or an --author`). Giving both is a clap error, exit `2`. A malformed program hash fails with `invalid hash: expected 64 hex characters`, exit `1`.

## `match-audit`

Exports and verifies match-audit v0 logs for tic-tac-toe. The whole command needs the `sandbox` cargo feature at dispatch; see the gating note at the top of this chapter.

### `match-audit export-tic-tac-toe`

Exports a reference tic-tac-toe transcript using synthetic demo keys, as a binary v0 log.

```
id match-audit export-tic-tac-toe --output <FILE>
```

| Flag       | Argument  | Default  | Meaning                                                                      |
| ---------- | --------- | -------- | ---------------------------------------------------------------------------- |
| `--output` | file path | required | Write the binary v0 log to this path. Omitting it is a clap error, exit `2`. |

Example:

```
$ id match-audit export-tic-tac-toe --output ./toy-match.log
```

Failures: an unwritable output path fails the command, exit `1`. In a build without the `sandbox` feature the command fails at dispatch with `this build has no sandbox support`, exit `1`.

### `match-audit verify`

Verifies a v0 log against the pinned bundled tic-tac-toe program.

```
id match-audit verify <LOG>
```

| Flag    | Argument  | Default             | Meaning                                                        |
| ------- | --------- | ------------------- | -------------------------------------------------------------- |
| `<LOG>` | file path | required positional | Binary match-audit v0 log, as written by `export-tic-tac-toe`. |

Example:

```
$ id match-audit verify ./toy-match.log
```

Failures: a log that does not verify against the pinned program fails the command, exit `1`. A missing log file fails the command, exit `1`. In a build without the `sandbox` feature the command fails at dispatch with `this build has no sandbox support`, exit `1`.

## `walk`

The #9106 walking skeleton, driven by hand. One stored node per directory; the path is: `init` a node, `write` a signed record at an `id://` address, `serve` the node so a peer can sync with it, `sync` to the peer, `read` the record back from the other node's store, and use `reports` for the conflict a diverged write opens.

Every walk command takes `<DIR>`, the node directory, as its first argument. One process may hold a node directory at a time: opening a directory that a `walk serve` process holds reports that the directory is held rather than waiting.

`walk` is not feature-gated anywhere: it works in every build, including a `--no-default-features` build.

### `walk init`

Creates a walk node in a directory.

```
id walk init <DIR> --name <NAME>
```

| Flag     | Argument       | Default             | Meaning                                                                                                                                        |
| -------- | -------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `<DIR>`  | directory path | required positional | The node directory.                                                                                                                            |
| `--name` | string         | required            | The node's name: the resolver name in the addresses it mints, and the source label on what it produces. Omitting it is a clap error, exit `2`. |

Behaviour: re-running `init` on an existing node keeps its records. The recorded name must match the `--name` given.

Example:

```
$ id walk init ./toy-node-a --name toy-a
```

Failures: re-initialising with a different name than the recorded one fails the command, exit `1`.

### `walk write`

Writes a signed record (a note) on a node, and prints the record's `id://` address and event ID.

```
id walk write <DIR> --as <alice|bob> --text <TEXT> [--parent <HEX>]...
```

| Flag       | Argument       | Default                  | Meaning                                                                                                                                                                          |
| ---------- | -------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<DIR>`    | directory path | required positional      | The node directory.                                                                                                                                                              |
| `--as`     | `alice\|bob`   | required                 | The demo writer signing the record. Writers are demo keys derived from the name; `alice` and `bob` are the two demo writers. The flag is spelled `--as`.                         |
| `--text`   | text           | required                 | The note's text.                                                                                                                                                                 |
| `--parent` | hex event ID   | the node's current heads | A parent version the record follows; repeatable. Omitting it follows the node's current heads for the demo graph. Naming two or more parents resolves the conflict between them. |

Example — Alice writes a toy note, then Bob writes a follow-up:

```
$ id walk write ./toy-node-a --as alice --text "toy note one"
$ id walk write ./toy-node-a --as bob --text "toy note two"
```

The first command prints an address of the form `id://toy-a/...`; that address is the argument to `walk read`.

Failures: omitting `--as` or `--text` is a clap error, exit `2`. Writing to a directory with no initialised node fails the command, exit `1`.

### `walk read`

Reads the record an `id://` address names, from a node's store. The address may have been minted by any node: the address's target names the record, and the serving node must hold the version.

```
id walk read <DIR> <ADDRESS>
```

| Flag        | Argument          | Default             | Meaning                                  |
| ----------- | ----------------- | ------------------- | ---------------------------------------- |
| `<DIR>`     | directory path    | required positional | The node directory.                      |
| `<ADDRESS>` | `id://...` string | required positional | The address, as printed by `walk write`. |

Example:

```
$ id walk read ./toy-node-b id://toy-a/toy-record-address
```

(The address above is a shortened toy form; use the full address `walk write` printed.)

Failures: reading a record the node does not hold is an error, not a fetch: the command fails with a message of the form `this node does not hold the record at id://...`, exit `1`. Sync first (see `walk sync`) if the record lives on another node.

### `walk reports`

Shows a node's heads and conflict reports for the demo graph.

```
id walk reports <DIR>
```

| Flag    | Argument       | Default             | Meaning                                                                 |
| ------- | -------------- | ------------------- | ----------------------------------------------------------------------- |
| `<DIR>` | directory path | required positional | The node directory. This command takes no flags beyond this positional. |

Example:

```
$ id walk reports ./toy-node-a
```

Failures: a directory with no initialised node fails the command, exit `1`. A diverged write shows up here as a conflict report; resolve it with a `walk write` that names both sides as `--parent` values.

### `walk serve`

Serves a node: answers federation exchanges from its directory until interrupted, persisting what sessions admit. It prints the peer token a `walk sync` dials, of the form `<endpoint-id>@<socket-address>`.

```
id walk serve <DIR>
```

| Flag    | Argument       | Default             | Meaning                                                                 |
| ------- | -------------- | ------------------- | ----------------------------------------------------------------------- |
| `<DIR>` | directory path | required positional | The node directory. This command takes no flags beyond this positional. | It runs until interrupted (Ctrl+C). |

Example:

```
$ id walk serve ./toy-node-a
```

Failures: while `serve` holds the directory, other walk commands against the same directory report that it is held, exit `1`. Interrupting `serve` ends it; the records it persisted stay in the directory.

### `walk sync`

Runs one exchange from a node to a serving peer, and persists what the node admits.

```
id walk sync <DIR> --peer <TOKEN>
```

| Flag     | Argument       | Default             | Meaning                                                                                                                     |
| -------- | -------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `<DIR>`  | directory path | required positional | The node directory.                                                                                                         |
| `--peer` | peer token     | required            | The peer token printed by the peer's `walk serve`: `<endpoint-id>@<socket-address>`. Omitting it is a clap error, exit `2`. |

Example — sync toy node B from the toy node A that is serving (the token is the invented one printed by that serve):

```
$ id walk sync ./toy-node-b --peer toy-endpoint-id@127.0.0.1:41001
```

Failures: an unreachable peer fails the exchange, exit `1`. A malformed peer token fails the command, exit `1`. Sync is one exchange per invocation; run it again for a later state.
