# Tutorial: organise a library with tags and claims

This tutorial organises a small library of stored files with two different metadata systems that the command line keeps separate. Tags (`id tag`) are file metadata held in the node's tag store: key/value pairs attached to a file, with multiple values allowed per key. Claims (`id claim`) are statements about a subject held in the event store: a name and an optional value, in either the `shared` scope or the `local` scope. Tags answer "what did I label this file"; claims answer "what is asserted about this subject, and who else can see it". All names and values in this tutorial are toy data.

The tag family is documented in [The tag family](../reference/cli-tags.md); claims, documents, and groups are documented in [Groups, documents, claims and keys](../reference/cli-groups-identity.md). Claims live in the event store, selected by the global flag `--event-store <DIR>`, whose runtime default is the directory `events` inside the data dir.

## Step 1: store three files

```sh
id --new library put shopping.txt reading.txt photos.txt
```

Create the three files first with any content; the content does not matter for this tutorial. The command form is the one from [Tutorial: store, share and fetch a file between two nodes](share-a-file.md): `--new <NAME>` creates `.iroh/library/` once, and later commands in this tutorial reuse it with `--data-dir .iroh/library`.

## Step 2: set tags

`id tag set <FILE> <KEY> [VALUE]` attaches one value. Run it again with a different value to attach a second value under the same key; a key with no value is a key-only tag.

```sh
id --data-dir .iroh/library tag set shopping.txt kind list
id --data-dir .iroh/library tag set shopping.txt topic food
id --data-dir .iroh/library tag set reading.txt kind list
id --data-dir .iroh/library tag set reading.txt topic books
id --data-dir .iroh/library tag set photos.txt kind photo
id --data-dir .iroh/library tag set photos.txt pinned
```

The last command sets a key-only tag named `pinned` on `photos.txt`. Binary values are supported without putting raw bytes on the command line: `--value-hex <HEX>` takes a hex value, and `--value-file <PATH>` reads the value from a file (`-` reads stdin). The two flags conflict with each other and with the positional value.

## Step 3: read tags back

```sh
id --data-dir .iroh/library tag list
id --data-dir .iroh/library tag list shopping.txt
```

With a file argument, `tag list` prints that file's tags; without one, it prints tags for every file. Display defaults are literal: a non-UTF-8 value is shown as `<binary N bytes>`, and values are truncated at 256 bytes. The flags `--hex` and `--binary` change the value display, and `--no-truncate` prints full values.

## Step 4: search by tags

`id tag search` takes structured terms, and all terms must match (they are ANDed). A term is one of `key:`, `:value`, `key:value`, a `"literal"` quoted string, or a bare word matched case-insensitively across subject, key, and value.

```sh
id --data-dir .iroh/library tag search kind:list
id --data-dir .iroh/library tag search kind:list topic:books
id --data-dir .iroh/library tag search pinned
```

The first search lists `shopping.txt` and `reading.txt`. The second lists only `reading.txt`. The third lists `photos.txt`, matched on its key-only tag. The same query forms work against stored files in general search: `id search kind:list` is not a tag search — for file search by name and hash, see [File commands: put, get, find and kin](../reference/cli-files.md). Keep the two apart: `tag search` queries the tag store; `find` and `search` query file names and hashes.

## Step 5: remove a tag value

`id tag del <FILE> <KEY> [VALUE]` with a value removes that one value. Without a value it deletes every value for the key, plus the key-only tag.

```sh
id --data-dir .iroh/library tag del photos.txt pinned
id --data-dir .iroh/library tag list photos.txt
```

The list for `photos.txt` now shows `kind` with value `photo`, and no `pinned` entry.

## Step 6: set claims, shared and local

`id claim set <SUBJECT> <NAME> [VALUE]` records a claim about a subject. The default scope is `shared`. The flag `--scope <shared|local>` selects the scope explicitly, and a `local:` prefix on the name is shorthand for the local scope when `--scope` is absent.

```sh
id --data-dir .iroh/library claim set reading.txt status finished
id --data-dir .iroh/library claim set reading.txt local:rating 5
id --data-dir .iroh/library claim set photos.txt status unreviewed
```

Read them back and search them:

```sh
id --data-dir .iroh/library claim list reading.txt
id --data-dir .iroh/library claim search status
id --data-dir .iroh/library claim search status finished
```

`claim list <SUBJECT>` prints every claim about one subject. `claim search <NAME> [VALUE]` finds claims by name, optionally narrowed to one value: the first search lists both `status` claims, the second lists only the claim about `reading.txt`. `id claim review` lists claims awaiting review and takes no arguments.

## Step 7: move a claim between scopes, deliberately

`id claim scope <SUBJECT> <NAME> <shared|local>` moves an existing claim. Moving a local claim to the shared scope publishes it, and that direction requires the confirmation flag `--yes`:

```sh
id --data-dir .iroh/library claim scope reading.txt rating shared --yes
id --data-dir .iroh/library claim clear photos.txt status
```

`claim clear <SUBJECT> <NAME>` removes a claim outright; after the second command, `claim search status` lists only the claim about `reading.txt`.

## What success looks like

- `tag search kind:list` lists exactly `shopping.txt` and `reading.txt`.
- `tag list photos.txt` shows `kind = photo` and, after Step 5, no `pinned` entry.
- `claim list reading.txt` shows `status = finished` and `rating = 5`, with `rating` in the shared scope after Step 7.
- `claim search status` lists one claim: the one about `reading.txt`.

## One failure shape

Omit `--yes` in Step 7:

```sh
id --data-dir .iroh/library claim scope reading.txt rating shared
```

The handler refuses the move, because it would publish a local claim, and exits with code 1. The claim stays in the local scope — `claim list reading.txt` still shows it as local — and no partial move is recorded. Add `--yes` to confirm the publication and re-run. This refusal is the scope system's guard working as designed: a local claim never becomes shared as a side effect of a command that did not confirm publication.
