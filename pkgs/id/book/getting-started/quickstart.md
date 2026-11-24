# Quickstart

This chapter runs one complete first pass with the `id` binary built in
[Installation and provisioning](installation.md): store a
file, list it, fetch it back, tag it, and find it again by search. It
ends with a one-command world smoke test that runs entirely offline. No
server is started and no network is used; every command below operates
on the local store in the current directory.

All examples use toy data. Create the file first:

```bash
mkdir demo-node && cd demo-node
printf 'hello from the quickstart\n' > hello.txt
```

Running the commands inside an empty directory keeps the node's data —
`.iroh-store/`, `.iroh-key`, and `.iroh-meta/`, as laid out in
[Installation and provisioning](installation.md) — in one
place you can delete afterwards.

## 1. Store the file

```bash
id put hello.txt
```

`put` stores the file's bytes in the blob store under the hash of the
content and records the name `hello.txt` pointing at that hash. The
command's aliases are `in`, `add`, `store`, and `import`. A store under
a different name uses the `path:name` form:

```bash
id put hello.txt:greeting.txt
```

## 2. List what the node holds

```bash
id list
```

`list` prints the stored files with their names and hashes. The list you
see contains `hello.txt` (and `greeting.txt` if you ran the rename form
above). There is no `ls` alias for this command in the parser
(disagreement CLI-1): `id ls` fails with an unrecognized-subcommand
error. The `ls` spelling exists only inside the REPL and as an alias of
`id tag list`.

## 3. Fetch the file back

```bash
id get hello.txt:copy.txt
```

`get` takes `source:output` pairs: the command above resolves the name
`hello.txt` to its hash, reads the blob, and writes it to `copy.txt` in
the current directory. Two shorter forms do related work:

```bash
id cat hello.txt        # write the content to stdout
id get-hash <HASH> out.txt
```

`cat` (aliases `output`, `out`) is `get` forced to stdout. `get-hash`
fetches by the 64-hex content hash directly and requires an explicit
output path; the output path `-` means stdout.

Fetching a name the node does not hold is an error, not an empty result:
the command prints `Error: file not found` (or `not found: <name>`) and
exits with code 1.

## 4. Tag the file

Tags are key-value metadata attached to a stored file, held in the tag
store rather than in the blob. Set one, then read it back:

```bash
id tag set hello.txt topic greeting
id tag list hello.txt
```

`tag` has the aliases `label` and `link`, and `tag set` has the alias
`add`. A tag key can hold multiple values, and a tag with no value is a
key-only tag. Remove the tag again with:

```bash
id tag del hello.txt topic greeting
```

`tag del` deletes every value for the key plus the key-only tag when no
value is given. Tagging is covered in full in
[The tag family](../reference/cli-tags.md).

## 5. Find the file again

Two different searches exist, and they search different things.

Search by file name with `search`, which lists matches and retrieves
nothing:

```bash
id search hello
```

Queries are case-insensitive. Match quality is ordered exact, then
prefix, then contains, over the file name and the hash. There is no glob
matching (disagreement CLI-8): the README example `id find "*.rs"` does
not match every `.rs` file, because the query is a substring query, not
a pattern. The literal query `*.rs` matches only a name or hash
containing that character sequence.

Retrieve the best match in one step with `find`, which writes the best
matching file to a file of its own name by default:

```bash
id find hello
```

Search the tags instead with `tag search`, whose terms are ANDed
together:

```bash
id tag set hello.txt topic greeting
id tag search topic:greeting
```

The structured term forms are `key:` (the key exists), `:value` (the
value exists), `key:value` (the pair), a quoted `"literal"`, and a bare
term, which matches case-insensitively across subject, key, and value.

A search with no matches in the `find`, `search`, or `show` commands is
an error: `Error: no matches found for: <queries>`, exit code 1.

## 6. One-command world smoke test

```bash
id world doctor
```

`world doctor` takes no node argument and contacts no host. It checks
the local world toolchain — the Roc compiler (flag `--roc-bin`,
environment variable `ID_ROC_BIN`, falling back to `roc` on the `PATH`)
and the Roc platform (flag `--platform`, environment variable
`ID_ROC_PLATFORM`, falling back to `./examples/roc-world`) — and prints
a report. Add `--json` for the machine-readable form. The exit rule is
exact: the command exits non-zero only when a check fails; warnings do
not fail it.

This is a toolchain check, not a world session. Hosting a world requires
`id serve --world` with a world admin token, and joining one requires a
capability minted by that host's admin; no command in this chapter
claims a live world service exists. The full path — scaffold a world
with `id world new <DIR>`, run the offline scratch-world loop with
`id world dev <DIR>` (which prints one JSON report and needs no host),
host with `serve`, and join with `id world join <NODE>` — is covered in
[World commands: joining and playing](../reference/cli-worlds.md) and the
tutorial [Tutorial: join a world and run a guest app](../tutorials/join-a-world.md).

## What you have after this chapter

One node directory containing a blob store with one stored file under
two names, one metadata tag, and the node's own key file — plus the five
commands (`put`, `list`, `get`, `tag`, `find`/`search`) that every other
file workflow in this book is built from. The reference for the whole
family is [File commands: put, get, find and kin](../reference/cli-files.md).
