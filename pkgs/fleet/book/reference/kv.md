# fleet kv

`fleet kv` is identity-scoped key/value bytes: a host-local store per
session, plus a central plane (#13316) that hosts reach through the
hosted kv door — never by connecting to the central database directly.
For operating the central plane see
[Running the `fleet kv` central plane](../operations/kv-central.md);
for backups see [Backing up `fleet kv`](../operations/kv-backup.md).

Keys are scoped to the session's launch identity (owner, context and
session) and to a `--namespace`. With no trusted launch-identity record
every verb but help refuses with `identity_unavailable` (exit 69).
Parsing is strict and finishes before any store access: an unknown
flag or a missing `--namespace` is a usage error (exit 2) and touches
nothing. Typed store refusals print `error: <code>: <message>` and
exit 1.

Values of any size are accepted: up to the namespace's inline
threshold (default 256 KiB, at most 1 MiB) they are stored inline, and
larger values are placed as blobs when the namespace has a blob
placement (see `namespace` below).

## The two planes

- **Local (default).** Reads see this host's overlay first, then —
  after a sync — the central head, cached for at most 1 s. Writes
  store a new local version and queue it for the sweep (`sync`).
- **Central (`--global`).** `--global` pins `get`, `set`, `list` and
  `delete` to the central store: it reads the authoritative head (no
  cache) and writes by compare-and-swap against the head it read, or
  against `--expect VERSION`. A moved head is a typed `conflict`
  (exit 1) whose proposed value central keeps. The central store is
  `FLEET_KV_URL` with a bearer <redacted> in the file named by
  `FLEET_KV_TOKEN_FILE`; without them `--global` and `sync` exit 69
  with `central_not_configured`, before the local store is opened.

## fleet kv get

```
fleet kv get [--global | --allow-stale] [--verbose] --namespace NS KEY
```

Prints the value's bytes exactly. A missing or masked key exits 1
with `not_found`. Central unreachable: `unavailable`, or `stale` for
an expired cached head (exit 69); `--allow-stale` reads that head
anyway. `--verbose` prints the source to stderr. A central value
stored as a blob streams through the blob door and is checked against
its reference: a length or digest mismatch exits 1
(`blob_length_mismatch`, `blob_digest_mismatch`) after the bytes read
so far, which are not the value.

## fleet kv set

```
fleet kv set [--global [--expect VERSION]] --namespace NS KEY (VALUE | --stdin)
```

Stores bytes as a new local version and queues it for the sweep;
prints the version. `--force-blob` marks the value for blob placement
at any size when sync sends it; with `--global` it goes through the
upload door at once, as does any `--global` value above the inline
threshold (`no_blob_placement`, exit 69, when the namespace has no
blob placement).

## fleet kv list

```
fleet kv list [--global | --allow-stale] [--verbose] --namespace NS [--include-tombstones]
```

One line per key: `KEY<TAB>VERSION<TAB>ORIGIN`, where ORIGIN is
`local`, `central`, `both` or `masked`. `--verbose` adds each central
row a local row hides (`shadowed-by-local`). `--allow-stale` lists
local rows when central is unreachable.

## fleet kv delete

```
fleet kv delete [--global [--expect VERSION]] --namespace NS KEY
```

Writes a masking tombstone: the key reads absent here and sweeps as a
mask. `--global` writes the tombstone to central directly by
compare-and-swap.

## fleet kv clear-override

```
fleet kv clear-override --namespace NS KEY
```

Drops the local overlay row, advances the key's fence and cancels
unsent sweep intents. History is kept. Busy for over 5 s:
`clear_busy_try_again`.

## fleet kv rebase

```
fleet kv rebase --namespace NS KEY
```

Resolves a conflicted key: reads central's head, makes it the key's
base, cancels the key's pending intents and queues one new intent for
this session's overlay value against that base (none after
clear-override: the key follows central). Prints
`rebased onto=VERSION requeued=VERSION|none`. Run `fleet kv sync`
next.

## fleet kv revert

```
fleet kv revert --namespace NS KEY --to VERSION
```

Appends a new version equal to VERSION from this key's local history.

## fleet kv sync

```
fleet kv sync [--archive]
```

Pulls this host's namespace definitions from central, then sweeps
this session's pending intents into central by compare-and-swap.
Prints a summary line, then one line per conflict or waiting key.
When the last archive checkpoint is over 15 min old (or `--archive`),
it then archives the writes that did not sweep. After the first sync
this host knows which namespaces exist: any other namespace is
refused with `unknown_namespace`.

## fleet kv backup

```
fleet kv backup
```

The same as `fleet kv sync --archive`: sync, then checkpoint this
session's unswept writes to the central survival archive now.

## fleet kv snapshot

```
fleet kv snapshot DEST
```

Writes a consistent one-file snapshot of this host's whole store
(WAL included) to DEST, with a manifest row inside it. Never
overwrites DEST. Prints the manifest.

## fleet kv restore

```
fleet kv restore SNAPSHOT
fleet kv restore --archive
```

Restores SNAPSHOT as this host's store after checking its manifest,
schema version and integrity. Refuses when a store (or its WAL)
already exists. `--archive` instead adopts this session's last
central archive checkpoint as new local writes (queued to sweep); no
checkpoint: `archive_not_found`.

## fleet kv namespace

```
fleet kv namespace define NS (--shared SCOPE | --private) [--global-flag] [--blob-threshold BYTES] [--blob-partition JUR/TIER]
fleet kv namespace list
```

Operator verbs over central namespaces; they need the central store
and an admin credential (the hosted door's kv-admin role) and never
open a local store. `--shared SCOPE` gives every agent one shared
scope; `--private` gives each (owner, context) its own. Defining NS
again keeps its settings unless a flag changes them, raises its
policy epoch, and refuses to change shared to private or back
(`namespace_scope_change`). `--global-flag` marks NS as global (a
designation only). Hosts see a change after their next
`fleet kv sync`.

## fleet kv grant

```
fleet kv grant list GROUP
fleet kv grant revoke GROUP DB NS
```

Operator verbs over a group's central grants; they need the central
store and an admin credential (the hosted door's kv-admin role) and
never open a local store. `list` prints one line per grant row:
`GROUP<TAB>DB<TAB>NS<TAB>OPS<TAB>EPOCH`, where OPS is a
comma-separated subset of `read,write,delete`. A group with no grant
rows is refused with `unknown_group` (fail-closed: no grant, no group
database). `revoke` removes GROUP's grant for NS in DB and prints
`revoked group=GROUP database=DB namespace=NS`; an ungranted
namespace refuses `namespace_not_granted`, and revoking the group's
last grant dissolves the group. Admitting a grant stays with the
kv-admin door statement (`grants.upsert`): this verb never creates a
grant row. (#13316, PR #16384)

## fleet kv blob

```
fleet kv blob gc JURISDICTION/TIER [--grace-ms MS] [--purge]
```

Runs one garbage-collection pass over a central blob partition
(admin credential; the local store is not opened). The pass marks
published blobs that no key references as candidates, clears
candidates that gained a reference, then fences the delete of every
candidate unreferenced for at least the grace (`--grace-ms`, default
24 hours). `--purge` then asks the blob door to remove the bytes of
every deleted blob that no published row names.

## Exit codes

0 ok · 1 typed refusal (`malformed_key`, `not_found`, `conflict`,
`unknown_namespace`, `backup_target_exists`, `not_a_snapshot`, …) ·
2 usage error · 69 unavailable (`central_not_configured`,
`identity_unavailable`, `no_blob_placement`, …).
