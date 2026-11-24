# Tutorial: store, share and fetch a file between two nodes

This tutorial stores a file on one node, starts that node as a server, and fetches the file from a second node. Both nodes run on one machine, in two separate data directories, so every step is repeatable without a second computer. The two nodes find each other through the default gossip discovery and mDNS, which is on by default. All data in this tutorial is toy data: one text file named `notes.txt` containing one line.

Commands used here are documented in [File commands: put, get, find and kin](../reference/cli-files.md) and [serve and publications](../reference/cli-serve.md). The global flags `--new` and `--data-dir` are documented in [The id command line](../reference/cli.md).

## Step 1: create the first node's data directory and store the file

`--new <NAME>` creates `.iroh/<NAME>/` under the current directory and runs the command inside it. It fails if the directory already exists, so use it exactly once per node.

```sh
printf 'hello from node a\n' > notes.txt
id --new node-a put notes.txt
```

`id put` stores the file's bytes as a content-addressed blob and records the name `notes.txt` for it. A file argument of the form `path:name` stores the file under a different name; this tutorial keeps the file's own name.

Confirm the store:

```sh
id --data-dir .iroh/node-a list
```

`--data-dir <DIR>` reuses an existing data directory; it is the documented way to run later commands against a directory that `--new` created. The list prints `notes.txt`.

## Step 2: print the first node's ID

```sh
id --data-dir .iroh/node-a id
```

`id id` prints the node's server identity: a 64-character hex node ID. Copy it exactly; the next steps use it as `<NODE_A_ID>`. A mistyped ID fails with the error `invalid node ID: must be 64 hex characters` and exit code 1.

## Step 3: serve the first node

In a terminal that stays open:

```sh
id --data-dir .iroh/node-a serve
```

`id serve` runs indefinitely. It hosts the blobs in the store and answers remote file operations from other nodes. Gossip discovery and mDNS local discovery are both on by default; this tutorial passes neither `--no-gossip` nor `--no-mdns`. Leave this terminal running for the rest of the tutorial.

## Step 4: create the second node and fetch the file

In a second terminal, create the second node's data directory by storing nothing yet — `id list` against a fresh directory is enough to create it:

```sh
id --new node-b list
```

The list is empty. Now fetch the file from the first node. In `id get`, when the first positional argument is a 64-character hex node ID, it is consumed as the remote node and the remaining arguments operate there:

```sh
id --data-dir .iroh/node-b get `<NODE_A_ID>` notes.txt
```

This downloads the blob named `notes.txt` from node A and writes it to a file named `notes.txt` in the current directory. A get argument of the form `source:output` chooses a different output name, and an output of `-` writes to stdout.

Confirm the fetch in three ways:

```sh
cat notes.txt
id --data-dir .iroh/node-b list
id --data-dir .iroh/node-b cat notes.txt
```

The first command prints `hello from node a`. The list now shows `notes.txt` in node B's own store: a successful `get` stores the blob locally, it does not stream it on every read. `id cat` is `get` forced to stdout, and prints the same line from node B's store.

## Step 5 (alternative): push the file server-to-server with send

Fetching is a pull. `id send` is the push form: the source server copies the file to the destination server with its own identity, and no bytes pass through the client issuing the command.

First print node B's ID:

```sh
id --data-dir .iroh/node-b id
```

Then, with node A still serving, store a second file on node A and send it:

```sh
printf 'second file\n' > second.txt
id --data-dir .iroh/node-a put second.txt
id send `<NODE_A_ID>` second.txt --to `<NODE_B_ID>`
```

`--to <DEST_NODE>` is required. The flag `--as <NAME>` sets a different destination name; the flag `--replace` archives the old destination file first when the destination already holds that name. Afterwards, `id --data-dir .iroh/node-b list` shows `second.txt` without node B having run a `get`.

## What success looks like

- `id --data-dir .iroh/node-b list` lists both `notes.txt` and `second.txt`.
- `id --data-dir .iroh/node-b cat notes.txt` prints `hello from node a`.
- The fetched files are byte-identical copies in node B's own store, listed and readable after node A's `serve` is stopped.

## One failure shape

Run the fetch with node A's `serve` stopped, or with a name node A does not hold:

```sh
id --data-dir .iroh/node-b get `<NODE_A_ID>` missing.txt
```

The command fails with a not-found error — `file not found on remote` for a missing name on a reachable node — printed as `Error: <message>` on stderr with exit code 1. Nothing is written locally: no partial file and no entry in node B's list. Start node A's `serve` again, check the name with `id --data-dir .iroh/node-a list`, and re-run the `get`.
