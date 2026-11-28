# `cursor` family

Contract owner: HYD-06 (#9508), `fleet-queue/src/work/activity.rs`
(`refresh_activity`, `dedupe_replay`, `ActivityRecord`).

`cases.json` holds one refresh case and one replay case as wire
JSON. The refresh case commits four `Present` records, then
refetches: `c5` edited behind the creation watermark (same native
ID, new revision + digest + later source time), `c9`
byte-identical but re-observed, `c10` brand new and duplicated in
the same refetch (first occurrence wins), `c6` moved
`Present` → `Deleted`, `c7` first observed `Minimized`, and `c8`
absent with no tombstone. The replay case redelivers two
committed `(native ID, revision)` pairs out of order plus one new
revision of the same native ID.

The dedupe key is the `(native_id, revision)` pair, never the
creation: an edit behind the cursor is one edit, not a second
creation; absence with no tombstone is a visibility gap, never an
inferred delete.
