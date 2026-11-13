# Tag scope: local-only vs shared claims (#1793)

## 2026-11-08T06-25-00Z Design: tag scopes for pkgs/id

### Request

Issue #1793 bullet 2: tags (and petnames) can be scoped local-only or
shared, chosen at create/edit time, honoured by replication, and
existing tags can be reviewed and organised by scope. Bullet 1 (file
origin in listings) landed separately in PR #5191; this record covers
only the scope work, implemented in PR #7344.

### Design approach

- `TagScope` (`shared` | `local`) is carried on every live `Claim`,
  defaulting to `shared`, so already-landed claims decode and behave
  exactly as before. No migration reclassifies an existing claim.
- The signed `tag.set` wire form is untouched: no field is added, so
  v1 event IDs and encodings are unchanged and a shared claim is
  replicated exactly as it was before scopes existed.

### Persistence boundary

- Shared claims live in the signed event store, as before.
- Local claims live in a separate store beside the event store
  (`local-claims.jsonl`, rewritten atomically). A local claim is never
  signed and never enters the event store, so it never enters the
  range-sync set and cannot reach another node. A test pins the sync
  set unchanged across a local set/clear.

### Publication behaviour

- `tags_on`, `search` and the review listing merge shared and local
  claims; every row carries its scope, so the two kinds are never
  silently conflated.
- CLI: `id claim set --scope local|shared` selects scope at create
  time; `id claim scope SUBJECT NAME SCOPE` moves a claim between
  scopes; `id claim review` lists every visible claim with its scope.

### Scope-conversion trade-offs

- Moving shared → local copies the claim into the local store and
  then clears only this node's own shared claims, by claim ID, so
  another author's same-named claims are untouched.
- Moving local → shared publishes only this node's own local claims
  (filtered by author) under its signing key, and then removes only
  its own local originals. Publishing another identity's local claims
  — possible when two identities share one `--event-store` with
  different data directories — would leak claims their owner chose to
  keep private; the author filter is the guard, and it matches the
  shared → local direction.
- A move changes where a claim lives, never its name or value.

### Petnames

Petnames were already viewer-private by construction (local journal,
shown only to the owning account, never replicated). No scope choice
was added for them; existing tests pin the private behaviour.

---

## 2026-11-08T06-25-00Z Update: record created during review

This record was written when PR #7344's Codex review flagged its
absence (pkgs/id/AGENTS.md requires the record for new commands, a
new persistent store, and scope-conversion semantics). It documents
the design as implemented in that PR, plus the author-filter guard
for local → shared conversion added in the same review round.
