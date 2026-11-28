# Backup, restore and data directories

A node's whole state lives in its data directory. This chapter names every part of that directory, states which parts are the record and which parts can be derived again, and gives the restore shape that follows from the code. One limit runs through the chapter and is stated here, not buried: **there is no backup verb and no restore verb for a whole node in the `id` CLI.** The exceptions are narrow and are named where they belong: `id world journal backup <DIR> <OUT>` for one world's journal, `id host export` and `id host import` for one world under a world host (see [Moving one hosted world: export and import](#moving-one-hosted-world-export-and-import)), and the key export commands in [Groups, documents, claims and keys](../reference/cli-groups-identity.md). Everything else in this chapter is file copying done by the operator, and each step that the surveys could not verify against code is marked unverified.

The store formats themselves are in [Data stores and schemas](../reference/data-stores.md). Running the node is in [Running a node: serve, worlds and the hosted edition](services.md).

## Where the data directory is

| How the node was started   | Data directory                                                                             |
| -------------------------- | ------------------------------------------------------------------------------------------ |
| No global flags            | The current working directory.                                                             |
| `--data-dir <DIR>`         | `<DIR>`, created if missing.                                                               |
| `--new [<NAME>]`           | `.iroh/<NAME>/` under the current directory; a random 8-hex name when `<NAME>` is omitted. |
| `--ephemeral` (serve only) | No store on disk. The store is in memory and is lost at exit. There is nothing to back up. |

All paths below are relative to the data directory, because the process changes directory into it at startup.

## The directory, item by item

| Path                                                           | What it is                                                                                                                                                                                                                                                                                                                              | Back it up?                                                                                                                                                                                 |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.iroh-store/`                                                 | The blob and name store: iroh-blobs `FsStore` (redb plus data files). The iroh-docs store sits under `.iroh-store/docs/`. This is the content-addressed store: a name points at an immutable blob hash, and the blob bytes are the file content.                                                                                        | Yes. This is the record for stored files.                                                                                                                                                   |
| `.iroh-key`                                                    | The node key file. The keypair for this node's identity. A second file, `.iroh-key-client`, holds the client key where a client identity exists.                                                                                                                                                                                        | Yes. Losing the key loses the identity; the store without the key is content under a name nobody can prove is theirs. Keep the copy access-controlled; this book never prints a key value.  |
| `.iroh-serve.lock`                                             | The serve lock, present while a serve process owns the directory. Constant `SERVE_LOCK` in `id-core`.                                                                                                                                                                                                                                   | No. It is runtime state, recreated by the next serve. Do not copy it into a backup as a record, and do not expect it in a restored directory.                                               |
| `.iroh-allowed`                                                | Optional plain-text file, one node ID per line, naming nodes allowed to write the store in addition to `--allow-node <IDS>` and this machine's own keys.                                                                                                                                                                                | Yes, if present. It is operator configuration that exists nowhere else.                                                                                                                     |
| `groups.jsonl`                                                 | The default group log for the `group`, `delegate` and `revoke` commands. The global flag `--groups-log <FILE>` moves it; a relative path is relative to the data directory.                                                                                                                                                             | Yes. Group membership and delegation history live here.                                                                                                                                     |
| `events/`                                                      | The default event-store directory for the `doc`, `claim` and `writer-sets` commands. The global flag `--event-store <DIR>` moves it; a relative path is relative to the data directory. Inside it, the event store file is `events.redb`, format marker `id.events.store.v9`.                                                           | Yes. Documents, claims and writer sets live here.                                                                                                                                           |
| `.iroh-meta/registry.json`                                     | The tag registry: namespace records for the tag store (global, node-private and custom paired namespaces) plus a one-time migration marker. Registry format version `1`.                                                                                                                                                                | Yes. Tags themselves live in the iroh-docs namespaces under `.iroh-store/docs/`; the registry records which namespaces exist. Back it up together with `.iroh-store/`, never instead of it. |
| `.id-worlds/<NAME>/`                                           | One directory per hosted world, named by `--world-name <NAME>` (default world name `lobby`). Holds the world's journal and its checkpoints: the signed, ordered record of world inputs and the folded state snapshots the host trims to every `--world-checkpoint-every <N>` events (default `1000`; `0` never trims).                  | Yes, for every world the node hosts. A world's journal is its record; see below.                                                                                                            |
| `.id-keyset.jsonl`, `.id-keyset-client.jsonl`                  | Keyset records: device enrolment, guardians, rotation and recovery state for the account keyset.                                                                                                                                                                                                                                        | Yes, if present.                                                                                                                                                                            |
| `directory.jsonl`                                              | The directory journal: one JSON `DirectoryEntry` per line. **Unverified location detail:** the survey records its location as the directory that holds the server-config file given to `--server-config <FILE>`, not as a fixed data-directory path. Back up the copy that sits next to the server-config file the node actually loads. | Yes, if the node runs with a server config.                                                                                                                                                 |
| Gallery directory (`--gallery-dir <DIR>`, or `ID_GALLERY_DIR`) | Verified `<content-hash>.idpb` bundle files plus an `operator/` subdirectory holding collections, tags and admission decisions. The directory is wherever the operator put it; it is not inside the data directory by default.                                                                                                          | Yes, if the node serves a gallery. Back up the directory the flag points at.                                                                                                                |

Files a node writes outside the data directory, and therefore outside a data-directory backup: the log file named by `ID_LOG_FILE` (operator logging, not node state), the compiled-world cache named by `ID_WORLD_COMPILE_CACHE_DIR` (derivable; see below), and the gallery directory above.

## What is the record, and what is derivable

**The store is the record.** Blobs are content-addressed: the hash is computed from the bytes, so a blob either matches its hash or is corrupt, and no index can invent a blob that is not there. Names and tags point at hashes. Copying `.iroh-store/` copies the files; copying any index, cache or listing without it copies pointers to content the restore will not have.

Derivable or replaceable, and therefore not backup-critical:

- **Blob bytes that other nodes hold.** A blob can be fetched again from a peer that has it, by hash, with `id get-hash <HASH> <OUTPUT>` or by name from a remote node. That is replication, not backup: it works only while a peer holding the blob is reachable, and this book does not present it as a substitute for a copy the operator controls.
- **The compiled-world cache** (`ID_WORLD_COMPILE_CACHE_DIR`). Compiled modules are produced from Roc source or precompiled modules by compiling again. Losing the cache costs compile time, not state.
- **A world's folded state, given its journal and checkpoints.** World state is a fold over the journal. The journal plus the module rebuild the state by replay; a checkpoint is a saved fold at one point in that journal, so replay after a checkpoint starts there instead of at event one. That is why the journal and checkpoints are the backup, and a memory image of a running world is not a backup format at all.
- **The serve lock, web assets and the book.** The lock is runtime state. Web assets are embedded in the binary at compile time. Neither is node data.
- **Search or listing output.** Anything `id list`, `id find` or `id search` prints is computed from the store at call time.

Not derivable, and therefore the backup itself: `.iroh-store/`, `.iroh-key` (and `.iroh-key-client` where present), `groups.jsonl`, `events/`, `.iroh-meta/registry.json`, each `.id-worlds/<NAME>/`, the keyset files where present, `.iroh-allowed` where present, and the server-config TOML file itself, which lives wherever the operator put it.

## Backing up: the manual shape

**Unverified as a product procedure:** the steps below are the file-copy shape that follows from the directory layout above. No command in the CLI performs them, and the surveys record no tested whole-node backup procedure in the repository. Treat this section as an operator procedure to test on a toy node before trusting it on a real one.

1. Stop the serve process for the data directory. The store is a single-opener redb database and the world journals are append-only files a running host is writing; a copy taken under a running serve is a copy of files in motion. **Unverified:** the surveys record no online-backup or snapshot mode in the code.
2. Copy the whole data directory, including hidden files. In a toy example with invented names:

```text
cp -a toy-node/ toy-node-backup-01/
```

3. Copy, separately, the files that live outside the data directory and belong to this node: the server-config TOML (if the node uses one), its `directory.jsonl` (see the table above), and the gallery directory (if the node serves one).
4. Verify the copy holds the items in the table: `.iroh-store/`, `.iroh-key`, `groups.jsonl`, `events/`, `.iroh-meta/registry.json` and `.id-worlds/` where those exist. A backup missing `.iroh-key` is a backup of content without its identity.

For one world's journal only, there is a real verb:

```text
id world journal check <DIR>
id world journal backup <DIR> <OUT>
```

`journal backup` requires the output path `<OUT>` to not already exist, and it checks the journal first: a journal that fails `journal check` is not backed up by this verb. `journal check` exits non-zero when the journal is invalid or was written in a newer format than the binary reads. See [World commands: administering and verifying](../reference/cli-worlds-admin.md).

Keys have their own export path, separate from file copying: `id key export <OUTPUT>` writes a key file (passphrases are hidden prompts, never flags), `id key export-seed` prints a 24-word phrase once and writes no file, and `id key import <INPUT>` and `id key import-seed` reverse them. The `--client` flag selects the client key. Details are in [Groups, documents, claims and keys](../reference/cli-groups-identity.md).

## Restoring: the manual shape

**Unverified as a product procedure:** there is no restore verb. The shape below is the reverse of the backup above, and the same warning applies: test it on a toy node first.

1. Install an `id` binary of a build that reads the backed-up formats. Format versions are real constraints here: an event store at `id.events.store.v9` is read by a binary that knows v9 (v8 is also accepted by the current code); a world journal written in a newer format than the binary reads fails `journal check`. An old store binary is a known failure shape in the other direction too; see [Troubleshooting](troubleshooting.md).
2. Place the copied data directory where the restored node will run, with the hidden files intact and the key files access-controlled.
3. Do not restore `.iroh-serve.lock` as a live lock. If a copied lock file is present and no serve is running, the surveys record no code path that clears or ignores a stale lock; that gap is listed in [Troubleshooting](troubleshooting.md) as unverified. **Unverified:** whether a leftover lock blocks the next serve or the REPL's mode detection only.
4. Start the node pointed at the restored directory:

```text
id --data-dir toy-node-restored serve --web
```

5. Check the restore before declaring it done: `id --data-dir <DIR> list` names the stored files, `id world journal check <DIR>` passes for each hosted world directory the node must serve, and a known file fetched by name matches the content the operator expects. A serve that starts is not, by itself, proof that the right directory was restored.

Restoring a single world journal backup made by `id world journal backup <DIR> <OUT>` into a world directory is **unverified**: the surveys record the backup verb and the check verb, and no restore or import verb for a journal backup file. This book does not invent one.

## Moving one hosted world: export and import

A world host (`id host`) can write one world to a single signed file and restore that file into another host root. This is the one restore path for a world journal that the code ships. It covers one world at a time, not a whole node.

1. On the source host, print the host's signing key with `id host key <ROOT>`. The key is created on first use as `<ROOT>/host.key`, readable by its owner only. Keep the printed hex: the restore checks every file against it.
2. With the host running (`id host run <ROOT>`, or a node serving the root with `id serve --world-host`, whose root is `.id-worlds` under its data directory), start the world, because only a world the host holds live can be exported: `id host start <ROOT> <WORLD>`.
3. Write the export: `id host export <ROOT> <WORLD> <FILE>`. The command refuses a `<FILE>` that already exists, writes to a hidden partial file first and renames it into place only when the whole export is on disk.
4. Copy `<FILE>` to the machine that will host the world.
5. Restore it: `id host import <NEW_ROOT> <FILE> --source <KEY_HEX>`, with the hex from step 1. The import refuses a file signed by another key, a file with a changed or missing byte, a cut file, and a world name already recorded in `<NEW_ROOT>`. Nothing is written into the root unless every check passes; a failed import leaves no world directory behind.
6. Run a host on the new root (`id host run <NEW_ROOT>`), start the restored world with `id host start <NEW_ROOT> <WORLD>` and run `id host inspect <NEW_ROOT> <WORLD>` to confirm it opens.

```text
id host key hosts/a
id host run hosts/a          # in its own terminal
id host start hosts/a alpha
id host export hosts/a alpha alpha.idworld
id host import hosts/b alpha.idworld --source <KEY_HEX>
id host run hosts/b          # in its own terminal
id host start hosts/b alpha
```

`export` talks to the running host through its control socket. `import` uses the socket when a host is running on the target root and writes the root directly when none is. The restored world carries its journal, its directory and its module files, and its journal bytes are identical to the source's.

## The hosted edition

There is nothing for an operator to back up in the hosted edition today, because there is no deployed hosted service: the Worker package is a local-only scaffold (see [Running a node: serve, worlds and the hosted edition](services.md)). Inside that scaffold, a user's data would live in one `UserDO` Durable Object (SQLite tables at schema version `3`) with blob bytes in the R2 binding `BLOBS`, and the `UserDO` exposes a `GET /v1/export` route that returns the full key-value and blob index export. **Unverified as a restore path:** the surveys record the export route and no corresponding import or restore route in the Worker. Do not describe hosted export as hosted backup and restore.
