# Exact intake bytes versus the upstream formatted view

## Decision

The authoritative source bytes and line anchors for this design are the
eleven Git blobs at intake commit
`34faffbff26615e313488f6d04705a444342e41e`, now copied without alteration to:

[`source-snapshots/intake-34faffbff2/sources/`](source-snapshots/intake-34faffbff2/sources/).

[`source-snapshots/provenance.json`](source-snapshots/provenance.json) records
the original hash, pinned commit, snapshot path and current upstream-view
hash of every file. No upstream file or change was reset, discarded or
overwritten. The complete upstream formatted variant remains in `sources/`
and in Git history; the original blobs also remain in Git history.

## Why a snapshot is necessary

Upstream commit `8d0921ba60` (`github-actions[bot]`, “style(treefmt): auto-format
on main”, 2026-11-13T01:55:19Z) reformatted eight of the eleven input files.
It padded tables, rewrote emphasis/list numbering and removed blank lines.
For example the coordinator's numbered preflight/staging text was changed
to Markdown's repeated `1.` form. These are not a new user requirement set,
but they change bytes, line offsets and the supplied checksum results.

The formatting commit arrived with the ordinary merge of current `main`
into this branch (`735ff56781`). Source-dependent generation was paused;
the original blobs were copied to new paths with exclusive creation, not
restored over another author's changes. New immutable snapshots are excluded
from treefmt so their hashes, excerpts and line-number witnesses stay stable.

The original package's `SHA256SUMS` is itself a preserved input. It has **not**
been rewritten to bless formatted bytes. Therefore running it in the
formatted `sources/` view is not a valid original-package integrity check.
Run it against the immutable snapshot instead:

```bash
cd doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-snapshots/intake-34faffbff2/sources
sha256sum -c SHA256SUMS
```

Summit executed that command after copying the blobs: all **five covered
files pass**. The manifest never covered the other six files; their complete
snapshot hashes are in the provenance record, not falsely attributed to the
supplied manifest. The original ZIP was not re-fetched by this continuation;
its earlier verified receipt remains in `note.md`.

The original intake `note.md` is also preserved one level above the snapshot's
`sources/` directory, with its own auxiliary provenance entry. This supports
the corrective #6767 evidence row's exact original-note witness; it is not a
twelfth package input or a new independently verified deployment claim.

## Citation and validation contract

- `source_file_and_section` filenames and source line ranges refer to this
  **pinned original intake**, not an arbitrary formatted checkout or a later
  source revision. Cite the source commit and/or snapshot path with them.
- Validators and matrix builders read exact excerpts from the snapshot and
  compare source/excerpt hashes. They do not silently re-hash the same range
  from a changed source and call it verified.
- A formatting-only upstream update is not permission to change semantic
  requirements, classify future work as mandatory, renumber stable source IDs
  or erase source precedence. A substantive future source revision needs its
  own snapshot, change analysis and trace update.
- The legacy `sources/` view remains useful for reading rendered Markdown,
  but is not used as the byte-level oracle for this inventory. The snapshot
  is still a repository artifact available on `main` after this PR lands;
  no temporary external staging path is needed to reproduce its checks.
