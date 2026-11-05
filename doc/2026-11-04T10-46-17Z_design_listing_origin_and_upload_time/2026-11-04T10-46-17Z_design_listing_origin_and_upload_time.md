# File listing provenance: origin node and upload time in List

**Status:** as-built decision record (retrospective). The change described
here shipped in PR #5191 ("id: show file origin node + upload time in
listings", implementing issue #4973, part 1 of issue #1793), merged
2026-11-03 at commit `e81e2028b50a553120d6ac34ac3d5f2d539c2067`. The
documentation protocol in `docs/DOCUMENTATION_PROTOCOL.md` and
`pkgs/id/AGENTS.md` requires this record to be written _before_ an
interface or API change is implemented; for this change it was not, a
gap raised as a P1 finding by Codex review on PR #5191 and tracked in
issue #5384. This record therefore documents the shipped behavior and
the rationale embodied in it, verified against current `main`; it does
not propose any new behavior, and no code changes accompany it.

## Intent

A file listing showed only a file's hash and name. A user looking at a
listing could not tell _which node_ a file came from or _when_ it was
uploaded, even though the store already recorded `created`/`modified`
metadata tags on writes. Issue #4973 asked for the origin node and the
upload time to be carried end to end: stamped at write time, returned
in the List wire payload, and printed by the CLI/REPL and web listings.

## Decision

### Provenance is a write-time stamp, set if absent

`FileOps::stamp` (in `pkgs/id/src/fileops.rs`) sets a set-if-absent
`origin` tag alongside the existing `created`/`modified` tags when an
origin is configured. The stampers are the write entry points:
`LocalNode::ops` stamps the local node, `MetaProtocol::handle` stamps
the requesting (writing) node, and the web save/upload paths stamp the
web node's id next to their existing `created` write. Because the tag
is set-if-absent, the first writer stays the file's origin across
later rewrites and renames; provenance is a fact about the first
write, not about the most recent holder. No new storage mechanism was
introduced — `origin` reuses the existing metadata-tag mechanism, and
`origin` is registered among the system tag keys so it does not leak
into user tag pills in the web UI.

Origin is stamped only on a genuine first write — one with no
`created` tag recorded yet. A file that already has a `created` tag
but no `origin` tag is a legacy file (below), and a later overwrite
must not stamp it: absence of an `origin` tag alone does not prove
that this write is the file's first.

### Legacy files: unknown, never guessed, never backfilled

Files written before origin stamping existed carry no `origin` tag,
and files predating the `created`-derived upload time carry no upload
time either. In the read model both fields are `Option`s and are
`None` for such files; every surface renders `None` as **unknown**. No
backfill or migration was performed, and no value is inferred from the
node that currently holds or serves the file: a guessed origin would
be fabricated provenance, which is worse than an honest unknown. The
protocol test `list_shows_unknown_origin_for_pre_change_files` pins
this: a name-tag-only file lists with both fields `None`.

### The List wire payload changed, so the ALPN bumped

`MetaResponse::List` items changed from bare entries to:

```rust
pub struct ListEntry {
    pub hash: Hash,
    pub name: String,
    /// Node id of the file's origin node, if recorded.
    pub origin: Option<String>,
    /// Upload time (unix seconds), if recorded.
    pub uploaded_at: Option<u64>,
}
```

enriched server-side from the `origin` and `created` metadata tags.

Postcard encodes enum variants and payloads by position, and the
protocol's compatibility rule (documented on `MetaRequest` in
`pkgs/id/src/protocol.rs` and pinned by the `wire_format_is_pinned`
test) is that within one protocol version new variants may only be
_appended_ — reordering, inserting, or changing a payload requires a
new ALPN. Changing the List payload in place under the old ALPN would
have let mixed-version peers silently mis-decode each other, so
`META_ALPN` bumped from `b"/iroh-meta/2"` to `b"/iroh-meta/3"` in the
same change. The accepted cost, stated in PR #5191, is that
mixed-version peers fail to decode List responses until both sides
update; that hard failure is acceptable for this single-binary
protocol and is preferable to silent mis-decoding.

### CLI/REPL output contract

`id list` and the REPL `list` command (local, connected, and `@node`
forms) print one tab-separated row per file:

```
<hash>\t<name>\t<origin>\t<uploaded_at>
```

`uploaded_at` is unix seconds. A field with no recorded value prints
`unknown` (the `list_tsv_row_*` tests in `pkgs/id/src/commands/text.rs`
pin both the recorded and the unknown rendering). The REPL local path
goes through the in-process meta handler so its enrichment matches a
server exactly, rather than re-implementing it. The direct-store
fallback (no metadata store open) prints `unknown` for both new
columns instead of omitting them, so the column shape is stable
across modes.

### Web listing

`FileInfo.origin_node` is populated in `get_file_list`; each file row
renders `origin <node> · uploaded <date>`; the list supports
`?sort=uploaded|origin` and `?origin=<node-id|unknown>` (so legacy
files with unknown origin are directly filterable), and pagination
links preserve both parameters.

## Alternatives considered, as embodied in the shipped change

- **Backfill or infer origin for pre-existing files** — rejected.
  The true origin of a legacy file is unknowable from current state;
  stamping the current holder would fabricate provenance. Legacy
  files stay `unknown`, including when overwritten later (see the
  first-write rule above).
- **Change the List payload without an ALPN bump** — rejected. It
  violates the pinned positional wire-format rule and risks silent
  mis-decoding between mixed-version peers.
- **A separate provenance query alongside the old List payload** —
  not chosen. Every listing consumer (CLI, REPL, web) needs origin
  and upload time inline with the listing; enriching `ListEntry`
  server-side keeps one contract for all of them.

## Compatibility summary

- Wire: `MetaResponse::List` payload shape changed; ALPN
  `/iroh-meta/3` is required on both peers. Within `/iroh-meta/3`,
  the append-only variant rule continues to apply.
- Data: fully additive. Existing files, tags, and journals decode
  unchanged; the new tags are written only by post-change writes.
- Output: CLI/REPL list output gained two trailing tab-separated
  columns; consumers parsing rows must expect four fields, with
  `unknown` as the explicit no-value marker.

## Validation shipped with PR #5191

- `protocol::tests::list_shows_origin_node_and_upload_time_for_new_files`
  — a write via the meta protocol lists with the writing node and a
  nonzero upload time.
- `protocol::tests::list_shows_unknown_origin_for_pre_change_files`
  — a pre-change file lists with both new fields `None`.
- `commands::text` `list_tsv_row_*` — recorded and unknown TSV
  rendering.
- Updated List serialization round-trip, the 30k-item >1 MiB List
  response test, and `tests/remote_write_access.rs` for the
  `ListEntry` shape. PR #5191 reported `cargo check --tests` clean,
  68 library tests passed, and 5 `remote_write_access` tests passed.

## Out of scope

Backfilling origin for pre-existing files, and the local-vs-shared
tag-scope decisions (the remaining scope of parent issue #1793), are
explicitly not decided by this record.

## References

- Issue #4973 — the feature request this change implemented.
- PR #5191 — the shipped implementation, merge commit
  `e81e2028b50a553120d6ac34ac3d5f2d539c2067`.
- Issue #5384 — the documentation-compliance gap this record closes.
- `pkgs/id/src/protocol.rs` — `ListEntry`, `MetaResponse::List`, the
  append-only compatibility rule, `META_ALPN`.
- `pkgs/id/src/fileops.rs` — `FileOps::stamp` and the first-write
  origin rule.
