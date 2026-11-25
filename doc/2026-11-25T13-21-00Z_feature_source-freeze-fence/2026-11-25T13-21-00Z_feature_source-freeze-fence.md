# Source freeze fence for jurisdiction migration (#3404, hosted DO slice)

See [original plan](../../.opencode/plans/) (plan lives with the parent lane; this record is the hosted-DO slice design).

## 2026-11-25T13-21-00Z Request and intent

#3404 needs the source side of fenced jurisdiction migration in the hosted
data plane (`pkgs/id/hosted-worker`): a durable per-user freeze fence so the
coordinator can fence a source, wait for a trustworthy acknowledgement, and
only then export. An earlier incomplete draft (`user-do.ts`, `package.json`
in the worktree) took `watermark`/`drained` from the caller over public HTTP
routes, which lets any caller — including a stale coordinator or the account
holder — dictate the snapshot cut. This record replaces that with a
server-computed fence before any further implementation.

Scope: `pkgs/id/hosted-worker` (DO fence + ack + tests) and this design note
under `pkgs/id/docs`. No production deployment, no Rust/core changes, no
coordinator/directory/export-copy wiring (that is #3402 and later slices).

## 2026-11-25T13-21-00Z Design

### Freeze is an internal RPC method, not a public route

The draft exposed `POST /v1/migration/freeze` and `GET /v1/migration/freeze-ack`
through `UserDO.fetch`. The front door (`worker.ts`) forwards every path to the
object, so any HTTP freeze route is publicly reachable by construction. The
freeze is therefore **not an HTTP route at all**: it is public methods on
`UserDO` (`freezeSource`, `freezeAckFor`), callable over the internal object
interface (and directly in tests). No fetch path touches the fence table, and
the routing-boundary test (`idFromName`/`.jurisdiction()`/`USERS` confinement)
is unaffected — no new namespace access is introduced.

### The source computes the watermark; the caller never supplies it

The draft's `freeze` took `watermark` and `drained` query params. That is a
false-ack factory: a coordinator (or anyone) can claim any cut. Instead:

- The DO keeps a durable, monotonically increasing **op sequence** (`meta`
  key `op_seq`, starting at 0, bumped once per committed user-data mutation:
  KV put/delete, blob reference insert/delete, bridge ingest batches that
  store anything — accepted, fork, or waiting-held).
- `freezeSource({ migrationId, sourceJurisdiction, epoch })` validates and
  inserts the fence row synchronously before its first await. It then waits for
  already-started mutation requests to leave their tracked sections, reads
  `op_seq` as the watermark, and records `drained = 1`.
- Because JavaScript runs each synchronous section to completion, no mutation
  commit can interleave with fence insertion. A request that starts later is
  refused at entry; a request already in flight (streamed KV body, R2
  `head`/`put`, proof verification, WS batch decode) either completes its
  commit before freeze or resumes, hits the pre-commit fence recheck, and is
  refused with 423 `source_frozen`. `drained = 1` is recorded only after all
  tracked mutation requests have exited, not from a caller-supplied claim.

### Stale-coordinator protection is durable

The fence row is `(migration_id, source_binding, source_jurisdiction, epoch)`
in SQLite — it survives object restarts (eviction/cold start replays from the
same database). Refusals, all fail-closed:

- `sourceJurisdiction` must equal the object's persisted bound jurisdiction
  (the guard's fail-closed anchor); a fresh object with no bound identity
  refuses.
- One frozen fence per object: a freeze naming a different migration while
  fenced is refused (`frozen_by_other`).
- Same migration, different epoch is refused (`epoch_mismatch`): the epoch is
  the directory routing epoch the coordinator saw, so a stale coordinator
  cannot re-fence or reinterpret the wrong attempt.
- Same migration, same epoch is idempotent and returns the recorded ack.

There is deliberately no unfreeze in this slice: after the switch the source
stays fenced; recovery is forward or a fresh reverse migration. Abort/release
is #3402's slice.

### The write gate, and what stays available

`isFenced()` (any frozen row) gates exactly the user-data commits, each
rechecked immediately before its commit point (after the last await), plus an
upfront early refusal on the streaming write routes to avoid buffering bodies
for doomed requests:

- refused when fenced: bridge `push`, KV PUT/DELETE, blob PUT/DELETE
  reference commits, WS batch ingest (the existing `guardBatch`, which now
  throws 423 `source_frozen` instead of masquerading as a ceiling refusal;
  `onBatch` maps the fence error to its code).
- stays available when fenced: `usage`, `state`, `export`, `bridge/export`,
  KV GET, blob GET, and the whole bridge authentication path (`challenge`,
  `heads`, `pull`, `export` legs plus nonce/resume bookkeeping). Rationale:
  export is the read the fence exists to protect, and bridge auth writes
  (nonce mint/consume) are bookkeeping, not user data — fencing `challenge`
  would lock out the very reads the fence must preserve. `op_seq` is bumped
  only at user-data commits, never by auth bookkeeping or the `requests`
  counter.

### Verified when

`test/migration-fence.test.ts` (the file `package.json` already names)
proves: watermark equals the committed op count with no caller input;
idempotent re-freeze; stale-epoch and wrong-migration refusals; jurisdiction
mismatch and unbound-identity refusals; post-freeze 423s on every mutation
route with reads/export/auth intact; restart durability (new `UserDO` over the
same database keeps the fence and the watermark); a streamed KV body racing
freeze is refused and excluded from the watermark; an R2 `put` pending across
freeze never commits a reference; bridge `push` and WS batch ingest racing or
following freeze land nothing. `tsc --noEmit` and the neighbouring suites
(`bridge-do`, `blobs`, `export`, `routing-boundary`) stay green.

---

## 2026-11-25T13-21-00Z Decision: no caller watermark, no HTTP freeze surface

Alternatives considered: (a) keep the HTTP routes and sign the freeze with the
ID key — rejected, because authentication is not the defect; a duly
authenticated but stale coordinator (or the account holder themselves) can
still name a wrong cut, and the front door forwards everything; (b) an
immediate acknowledgement based only on the pre-commit check — not selected,
because waiting for already-started mutation requests makes the drained
acknowledgement explicit and testable. The implementation tracks those
requests, fences future entrants, and retains the pre-commit recheck as the
last safety check. RPC-only entry and a server-computed watermark make a false
ack structurally impossible rather than merely unauthorized.
