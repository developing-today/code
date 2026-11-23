# First ID app — PoC: sticky notes + did-it tracker

**This is a proof of concept, not a feature.** It exists to prove the
[#2031](https://github.com/developing-today/code/issues/2031) bar before
any permanent app work is planned: two identities exchange notes and
did-it events in one shared group timeline, and one identity's did-it
view correctly excludes a third party's claims while keeping them
retrievable on demand.

**Review-round gate:** no permanent feature is built on this PoC without
a review round accepting the schema, the trust behaviour, and the open
privacy decisions recorded in the design
(`doc/2026-11-01_design_id2031.md`, "Questions for Drew"). That gate is
#2031's, and this PoC does not lift it.

## What it composes

Nothing here is a new protocol or an app-local event shape. The PoC is a
composition of library slices that landed independently:

- **Notes** are `document.put` events in the shared group; a logical
  note is the `note.id` tag on each revision, and an edit is a new
  document linked by a `note.supersedes` tag, so concurrent heads stay
  visible instead of collapsing last-writer-wins.
- **The did-it feed** is `didit.*` events (schema + pure evaluator,
  #3788) in the same timeline, their canonical CBOR bodies persisted and
  retrievable through the generic body store (#4837).
- **The counted view** is the writer-set projection (#3789): only
  records whose author is in the viewer's writer set reach the
  evaluator. An uncounted record stays stored, is listed with its
  author and a not-counted mark, and its body is retrievable on demand.

## Where the executable artifact lives

The acceptance is an event-store integration property, so the PoC runs
as a Rust integration test rather than a Roc example (the `examples/`
Roc programs are world/capability demos; #3790's dispatch note reserves
the A/B/C test for `pkgs/id/tests/`):

```bash
cd pkgs/id
cargo test --test first_app_poc
```

The test (`pkgs/id/tests/first_app_poc.rs`) exchanges every event
between two replicas until they converge, then checks: the notes view
(two logical notes, B's revision is note-1's head, C's note present),
the did-it view (three records listed, exactly C's uncounted), the
evaluator verdict over counted records only (2 of 3 walks — still due),
a control verdict over all records (3 of 3 — satisfied, proving the
exclusion is what held the view at two), and C's record body retrieved
byte-exact from A's store after a restart.
