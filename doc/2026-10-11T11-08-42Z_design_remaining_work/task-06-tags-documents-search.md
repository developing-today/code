# Task 06: Tags, documents, index, and search

- Status: Designed. S8 (observed-remove property tests) is open and gates the tag design.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md) (the store) and [task 01](./task-01-phase0-fixes.md) (the tuple fix).
- Design: [design §9](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md). Evidence: S6 (index on redb), the tuple bug (task 01).

## Goal

Tags and documents become events. Search reads a local index, rebuilt from events, that lives in redb. The existing `tags.rs` storage is discarded, since there is no data to keep.

## Scope

1. **Tags.** `tag.set{subject, name, value?}` and `tag.clear{claims}`. Anyone may tag anything. Claims are attributed to the signing key.
   - Observed-remove: a clear removes only the claims its author had seen. The clear names them, and the clear carries the author's per-author max-seq clock.
   - Dots are `(author, seq)`. An add survives if its seq is greater than its author's entry in the clear's clock.
   - A concurrent set and clear on the same claim leave the set in place (add wins).
   - Precedence against strong remove: tags follow the content policy (task 03, decision 4), not membership voidness. This is a proposal.
2. **Index.** The α/Ω layout in redb: `(subject, name, value|null, claim)` and `(value|null, name, subject, claim)`, with prefix queries that use explicit upper bounds (see task 01). Also an author index.
3. **Search.** By subject, by name and value, and by author. Multi-term AND, as today.
4. **Documents.** `document.put{media_type, payload}`, where the payload is exactly one of: inline CBOR up to 64 KiB (`MAX_DOCUMENT_BYTES`), or a blob reference (hash and size) above that. `media_type` is at most 128 bytes.
5. **CLI.** `tag set` and `tag del` publish events. `tag list` and `tag search` read the index. Output shows the author and the event ID, so provenance is visible.
6. **Live editing.** The ProseMirror collab server stays the live authority. A document event is produced from a snapshot. The trigger (for example, on idle close) is open (task 10).

## Acceptance

- S8, to run first: property tests showing observed-remove holds under random delivery orders, including duplicates, and that add-wins holds for concurrent set and clear.
- Index tests on the real store, including the NUL-containing values from task 01.
- Search results match a reference computed from the events.
- A document above 64 KiB is stored as a blob and its event carries only the hash.

## Gotchas

- The old `del` deleted every value of a key. The new clear names the observed claims instead, which is a behaviour change to call out in the CLI help.
- `after` cannot hold an explicit tag list, so the clock is needed.
- Full-text search is out of scope. Only exact-match and prefix queries are planned.

## Files

New module, for example `pkgs/id/src/tags/` (events and index). Retired: `pkgs/id/src/tags.rs` storage. `tuple.rs` stays, with the fix from task 01.

## Dependencies

None beyond those already in task 02.

## 2026-10-11T12-49-09Z Update: observed-remove tags (S8)

- Convergence over 3 to 5 replicas, partitions, any order and duplicates: pass (300 cases, 3,000 in a soak).
- Add-wins: pass. Concurrent adds survive a clear that did not see them.
- A clear removes a dot only if the dot is named and covered by the clock. A planted bug that ignored the clock failed with `seen=0, unseen=1`.
- Merge laws (commutative, associative, idempotent) pass on reachable states. A wrong but symmetric join passes the laws, so only the oracle check catches it.
- Metadata: state stays flat. The log costs about 131 B per operation, excluding signatures.
- Not tested: strong-remove precedence, real headers and signatures, persistence and the index, network behaviour.
