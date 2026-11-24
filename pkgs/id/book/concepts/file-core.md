# The file core: content addressing, names and tags

## Content addressing

The file core has two halves: immutable bytes and mutable names. Bytes are stored once, in the blob store, and addressed by hash. A name points at a hash. Changing a file writes new bytes under a new hash and moves the name; the old bytes stay in the store until garbage collection.

The store is iroh-blobs. On disk it is an `FsStore` (redb plus data files) in `.iroh-store/`, with the iroh-docs store under `.iroh-store/docs/`. `id serve --ephemeral` uses the in-memory variant and loses content on exit. `STORE_PATH = ".iroh-store"` and `open_store` / `StoreType` (`Persistent(FsStore)` or `Ephemeral(MemStore)`) are the library entry points in `id-store`.

A hash is the only content identity the CLI exposes. Commands that take a hash take 64 hex characters: `id get-hash` takes a hash and an output path, `id put-hash` stores a single source hash-only, and the gallery commands reject a hash that is not 64 hex characters with `invalid hash: expected 64 hex characters`. Storing the same bytes twice produces the same hash and one stored blob. Two store modes exist for names: `id put` stores bytes and binds a name; `id put --hash-only` and `id put-hash <SOURCE>` store bytes with no named tag. In the second mode the hash is the only handle the caller holds.

One hash fact is pinned in the surveys and one is not. The signed event chain in `id-store` computes an operation ID as BLAKE3 over the received event bytes (web-config survey §5.2). The surveys do not name the blob hash algorithm for the iroh-blobs store itself. This chapter therefore states the blob hash as the iroh-blobs hash, printed as 64 hex characters, and marks the algorithm name **unverified** against the survey sources.

## Names

A file, in the FileOps definition, is a name bound to an immutable blob hash, plus tag-store metadata for that name (web-config survey §5.4). The name binding is an iroh-blobs tag. `id put notes.txt` stores the file `notes.txt` under the name `notes.txt`; `id put notes.txt:archive-copy` stores the same bytes under the name `archive-copy`, because the put syntax is `path:name`. Retrieval resolves the name to its current hash: `id get archive-copy` writes the current bytes for that name.

FileOps is the single implementation behind every surface: CLI, REPL, meta protocol and web all call the code in `crates/id-sync/src/fileops.rs` (facade path `src/fileops.rs`). Rename, copy and delete are FileOps operations, and FileOps maintains three identity metadata keys on every rename and copy: `name`, `file` and `path`. An `origin` key is stamped at first write and records the origin node (#4973).

The CLI has no top-level delete, rename or copy command. Those operations exist in the REPL (`delete`, `rename`, `copy`/`cp`), in the web API (`/api/rename`, `/api/copy`, `/api/delete`) and in FileOps itself. The command surface is in [File commands: put, get, find and kin](../reference/cli-files.md).

## Two tag systems with one word

The word tag names two distinct systems, and this book keeps them apart.

The first system is metadata tags, run by `id tag`. A metadata tag is a key/value pair attached to a file name. The store is the tag store in `id-store`, backed by iroh-docs namespace pairs: a global namespace, a node-private namespace, and custom paired or single namespaces. Tag values are arbitrary bytes. On the web wire, a binary value travels as `value_b64`.

The second system is tag claims, run by `id claim`. A claim is a signed statement in the event store that a subject carries a name, with a scope of `shared` or `local`. Claims live in `id-store` (`tag_claims`) and the event log, they replicate with the log, and they carry authorship. Metadata tags carry no signature and no author. The claim commands are in [Groups, documents, claims and keys](../reference/cli-groups-identity.md); the identity and group model behind claims is in [Identity, the directory and groups](identity-directory.md). The design record that models claims as atomic-triple projections (decision of 2026-11-06) and the tag-scope design (#1793) are carried in [Design: capabilities and providers](../design/capabilities-providers.md).

## Metadata tags: set, delete, list

`id tag set <FILE> <KEY> [VALUE]` sets one value. Multiple values per key are allowed: setting the same key twice with two values leaves both values on the file. With no value, the command sets a key-only tag. Two binary-safe input forms exist. `--value-hex <HEX>` takes the value as hex, and `--value-file <PATH>` reads the value from a file, with `-` meaning stdin. The two flags conflict with each other and with the positional value.

`id tag del <FILE> <KEY> [VALUE]` deletes one value, or, with `--value-hex <HEX>`, the value named in hex. With no value at all, it deletes every value for the key plus the key-only tag.

`id tag list [FILE]` lists tags for one file, or for all files when the file argument is omitted. Display is lossy by default: a value that is not valid UTF-8 prints as `<binary N bytes>`, and values truncate at 256 bytes. `--no-truncate` prints full values, `--hex` prints values in hex, and `--binary` selects the binary display. The REPL forms of the tag commands have none of the `--value-hex`, `--value-file`, `--hex`, `--binary` or `--no-truncate` flags; those are clap-only (CLI survey §2.1).

Tag changes broadcast on the web surface as `TagEvent` JSON frames on `/ws/tags`: `set`, `del`, `del_all` and `transfer`, each carrying `ns`, `subject`, `key` and an optional `value`; `transfer` carries `from`, `to` and `count`. The frame format is in [WebSocket protocols](../reference/websockets.md).

## Tag search

`id tag search <QUERY>...` takes structured terms, and all terms must match (logical AND). The term forms are:

- `key:` — the file has the key, with any value.
- `:value` — the file has the value, under any key.
- `key:value` — the file has that key with that value.
- `"literal"` — a quoted literal term.
- A bare term — matched case-insensitively across subject, key and value.

Quoted key and value positions are supported inside the `key:value` forms. The same syntax runs in the REPL (`tag search`, alias `tag find`) and in the web API at `GET /api/tags/search`, which takes `q` and `ns` parameters. The full tag command family is in [The tag family](../reference/cli-tags.md).

File search is a different matcher. `id find` and `id search` match queries against names and hashes with quality ordered exact, then prefix, then contains, case-insensitively. There is no glob or wildcard matching anywhere in `commands/find.rs`. Disagreement CLI-8: the README example `id find "*.rs"` presents the query as a pattern; the literal query `*.rs` matches only a name or hash containing that character run, and in practice matches nothing.

## The tag registry

Namespaces are recorded in a registry file: `.iroh-meta/registry.json`, with `REGISTRY_VERSION = 1` (`crates/id-store/src/tags.rs`). The registry records the namespaces in use and the completion of the one-time migration described below. The registry is local node state. It is not a replicated document, and no command in the CLI survey reads or writes it directly; the tag commands operate on namespaces the registry tracks.

## Auto-tags and `id migrate-tags`

Three keys are maintained by the system, not typed by the user: `name`, `file` and `path` (the identity keys FileOps refreshes on rename and copy), plus the `origin` key stamped at first write. Files stored before the auto-tag rules existed can lack them.

`id migrate-tags` (alias `migrate`) repairs that state. It adds the `name` and `file` auto-tags where they are missing. It never overwrites an existing value. The REPL has the same command as `migrate-tags` / `migrate`. The command is idempotent in effect: a second run finds the tags present and adds nothing.

## Archives and the archive naming rule

Rename and replace do not destroy the previous binding. FileOps archives it. The archive name is `{name}.archive.{unix-ts}`: the live name, the literal segment `.archive.`, and the Unix timestamp in seconds. If that name is taken in the same second, the suffix is bumped (`{ts+1}`, then higher) until a free name is found, claimed under an in-process lock (#1612/#2462).

Archives carry metadata. The live name records `archive.rename` and `archive.replace` keys naming the archives it owns. Each archive records an `archive.owner` key naming its owner. Archives are dropped only with their owner. There is no per-archive delete operation.

The archive namespace is reserved at the writer, and the reservation test is exact. A candidate user file name whose final segment matches `{stem}.archive.{digits}` is refused with `ReservedArchiveName` when two conditions both hold: the characters after `.archive.` are all digits, and those digits parse as a `u64`. A name such as `field-notes.archive.draft` stays valid, because `draft` is not digits. A digit string too large for `u64` fails the second condition and is not an archive name under the rule. This digit-plus-`u64`-fit test is the finding recorded against #13291 and #4767: classification of an existing name as an archive turns on the suffix parsing as a `u64`, not on the `.archive.` segment alone. The web layer builds its soft-delete and restore pair on top of the same FileOps archive machinery; `FileOps::delete` itself is a hard delete of name, metadata and owned archives, while blob bytes survive until garbage collection. The delete, copy and transfer rules in full are in [Sync, replication and FileOps](sync.md).
