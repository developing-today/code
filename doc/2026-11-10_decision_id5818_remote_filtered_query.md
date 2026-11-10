# Decision: server-side filtered remote query execution

- **Date:** 2026-11-10
- **Status:** design decision; no implementation is included.
- **Issue:** [#5818](https://github.com/developing-today/code/issues/5818),
  server-side filtered remote query execution (from #5675).
- **Parent:** [#5675](https://github.com/developing-today/code/issues/5675)
  fixes the filtering model, the safe default for unclassified data, the
  owner-override boundary, and the parity/pagination requirements this
  record composes with. Its design record is on open PR
  [#7361](https://github.com/developing-today/code/pull/7361)
  (`doc/2026-11-08T06-45-00Z_design_id_query_local_remote_v1/`); §4 and
  §10 of that record assign the mechanisms decided here to #5818. This
  record does not reopen any parent decision.
- **Composes with:**
  [#5816](https://github.com/developing-today/code/issues/5816) (CLOSED —
  v1 structured query object, server-side execution, parity invariant;
  decision record on that issue),
  [#5799](https://github.com/developing-today/code/issues/5799) (CLOSED —
  selective-sharing grants: object/collection units, no per-field grants
  in v1; decision record on that issue),
  [#5909](https://github.com/developing-today/code/issues/5909) (CLOSED —
  PGQ boundary; remote execution and sensitivity filtering remain
  server-side).
- **Depends provisionally on:**
  [#5817](https://github.com/developing-today/code/issues/5817) (OPEN —
  sensitivity classification levels, assignment, defaults). Every place
  this record depends on the classification model is marked
  **PROVISIONAL (#5817)** and listed in one section below; the shapes
  decided here are parameterized by level rank so #5817's levels can
  land without re-deciding them.
- **Evidence baseline:** `origin/main` at `69b4906b8f`, checked
  2026-11-10. At that point #5818, #5817, and #5675 were OPEN; #5816 and
  #5799 were CLOSED with their records on-issue; PR #7361 was OPEN.
  In `pkgs/id`, remote data access today runs over the `MetaRequest`
  transport (e.g. `MetaRequest::Find` / `MetaRequest::Get` in
  `src/commands/find.rs`, `src/commands/get.rs`); there is no
  filtered-query executor and no remote-query audit log yet. This
  record designs both; it implements neither.

## Decision summary

1. **All filtering happens on the serving server, before anything
   leaves it.** A remote query arrives as the #5816 structured query
   object, executes under the requester's identity, and only the
   filtered result crosses the transport. There is no mode in which
   unfiltered rows reach the requester "to be filtered client-side",
   including for debugging, previews, or counts.
2. **Effective visibility is the intersection of grant reach and
   clearance filtering — deny wins.** This records the composition
   rule the parent (§10) assigns to this design: a selective-sharing
   grant (#5799) decides whether a peer may *reach* a collection at
   all; classification filtering decides what a query over reached
   data *returns*. A grant never raises a requester's clearance, and
   clearance never creates reach. When the two disagree, the data is
   not returned.
3. **Row-level exclusion is the default; field-level redaction is the
   exception.** A row whose classification exceeds the requester's
   clearance is excluded whole. A field above clearance inside an
   otherwise visible row is **redacted** (present as a withheld
   marker, value never sent) — never silently dropped from the row
   shape, which would corrupt the #5816 `select` contract. A query
   whose `where` predicates reference a field above the requester's
   clearance is a structured validation error (`field_withheld`):
   evaluating such a predicate server-side and returning matches
   would make the response an oracle for the withheld values.
4. **Withheld-result honesty is a gradient by level rank, not a
   switch.** Every filtered result carries a `withheld` envelope:
   exact withheld-row and withheld-field counts for data at the
   lowest above-clearance level; a boolean `withheld_present` (no
   counts) for data at higher levels; and **nothing at all** — no
   count, no boolean — for the top level and for encrypted data.
   Silent filtering and existence leaks are both failure modes
   (parent §4); the gradient is the per-level choice the parent
   assigns to this record. **PROVISIONAL (#5817)** in level naming
   only: the gradient is defined over level *rank*, so it survives
   whatever level set #5817 lands.
5. **Unclassified data is treated as above-clearance for remote
   requesters.** The parent fixes the default for unclassified data
   as safe, not permissive; concretely, unclassified rows are
   withheld from remote execution (visible in the honesty envelope
   at the boolean tier, never the count tier) until #5817's model
   classifies them. Local owner execution is unaffected (subject to
   #5819's override mechanics).
6. **Every remote query is audited on the serving server, before its
   results are released.** If the audit write fails, a query whose
   scan touched any above-baseline (non-public) data is refused —
   audit-before-release for classified data. The audit record's
   fields are decided below; it is visible to the serving server's
   owner and to no one else. The requester never sees the audit;
   they see only their result and its honesty envelope.
7. **Filtering composes with pagination, not after it.** Predicates,
   grant reach, and clearance filtering are applied before ordering,
   limiting, and cursor minting, so a cursor only ever positions
   within the filtered stream and page sizes count returned rows.
   Truncation and cost semantics remain #5821's and #5816's; this
   record fixes only that sensitivity filtering is part of the
   stream being paginated, and that withheld counts are reported
   per result, not per page.
8. **Parity holds.** Per #5816, filtering outcome under the
   requester's clearance is one of the two permitted local/remote
   behavioural differences. Validation errors, predicate semantics,
   ordering, and error codes — including `field_withheld` — are
   identical on the local and remote executors; the honesty envelope
   is produced by both whenever filtering removes anything.

## Execution pipeline (remote target)

For a remote query object arriving at a serving server:

1. **Authenticate** the requesting identity (ID transport identity;
   a query carries no caller-supplied actor — the identity is the
   transport's, never a field of the query object).
2. **Validate** the query object exactly as #5816 specifies (known
   table/field, operator/type match, limit cap), plus the
   `field_withheld` predicate rule of decision 3. Validation errors
   are identical to local execution.
3. **Authorize reach**: the target collection must be within a
   #5799 grant to this identity (or be the owner's own data under
   #5819's rules). No reach → structured `not_authorized` error;
   the error does not distinguish "collection does not exist" from
   "no grant" (that distinction is itself an existence leak).
4. **Execute under clearance**: scan with the server-side cost
   guardrails #5816 requires (scan cap, timeout); apply
   classification filtering (decision 3/5) to produce the result
   stream; collect the honesty counts and the audit facts (levels
   touched, rows scanned/returned/withheld) during the same pass —
   no second scan for bookkeeping.
5. **Audit** (decision 6): append the audit record; on failure,
   refuse if classified data was touched.
6. **Shape and release**: order, limit, and cursor per #5816/#5821
   over the filtered stream; attach the `withheld` envelope
   (decision 4); return.

Local execution runs the same pipeline minus transport steps, under
the local identity's clearance and #5819's override mechanics.

## The honesty envelope

```text
withheld: {
  rows: <count at the lowest above-clearance level only>,
  fields: <count at the lowest above-clearance level only>,
  present: <true if anything at any higher level was withheld>,
}
```

- The envelope is omitted entirely when nothing was withheld —
  absence means "complete within your clearance", and a result never
  claims more than that.
- Counts are exact only at the lowest above-clearance level: that
  a peer can learn "3 rows just above your clearance exist" is the
  deliberate price of truncation honesty (parent §8); corpus shape
  at higher levels is not disclosed.
- `present` carries no count and no level name.
- For the top level and for encrypted data there is no envelope
  trace at all (decision 4). Encrypted data is outside this design
  entirely per the parent's hard boundary: policy never outranks
  cryptography, and no override in #5819 changes that.
- `count`-mode queries (#5816) apply the same rules: the returned
  cardinality is the filtered cardinality, and the envelope rides
  along under the same gradient.

## The audit record

One append-only record per remote query, written by the serving
server:

| Field | Content |
|---|---|
| `at` | Serving-server timestamp |
| `requester` | Authenticated transport identity (never a query field) |
| `query_hash` | Hash of the canonical query object |
| `query` | The canonical #5816 query object (it is data, not secrets) |
| `target` | Table/collection queried |
| `outcome` | `ok` / structured error code (incl. `not_authorized`, `field_withheld`, cost refusals) |
| `rows_scanned`, `rows_returned` | Execution counts |
| `rows_withheld`, `fields_withheld` | Exact counts (the audit is owner-visible only; the gradient applies to what the *requester* sees, not to the owner's own audit) |
| `levels_touched` | The set of classification levels present in scanned data |
| `override` | Whether an owner override was in effect (mechanics: #5819; the field exists from v1 so the log is never ambiguous about it) |
| `duration_ms`, `cost` | Elapsed time and scan-cap consumption |

Audit records are retained under the serving owner's policy and are
never shipped to the requester or to third parties by this design.
A query refused at validation or authorization is audited too, with
its error outcome — refused queries are exactly the ones an owner
needs to see.

## PROVISIONAL assumptions on #5817 (reconcile before finalizing)

1. **Level set and names.** This record assumes a totally ordered
   level set with a distinguishable baseline (public) level and a
   top level. #5817 may name and number them differently; the
   honesty gradient (decision 4) is defined over rank and the
   baseline/top anchors, and must be re-pointed at #5817's set when
   it lands.
2. **Granularity.** This record assumes classification can attach
   at row and field granularity (the parent's §4 contemplates both).
   If #5817 lands table/row-only, the field-redaction and
   `field_withheld` rules apply vacuously until field granularity
   exists; the row rules are unaffected.
3. **Unclassified default.** Decision 5 fixes the remote-execution
   *behaviour* (withhold, boolean tier). If #5817 assigns
   unclassified data a named level instead, the behaviour maps to
   that level's rank; the safe direction may not be relaxed by the
   mapping.
4. **Assignment authority.** Who may classify or reclassify is
   #5817's; this record assumes only that the executor can read the
   classification of every row/field it scans, atomically with the
   scan (no classification that changes meaning mid-query).

## Alternatives rejected

- **Client-side filtering** (ship rows, filter at the requester):
  rejected — it is the leak the parent design exists to prevent;
  "the client discards it" is not a boundary.
- **Silent filtering** (return the filtered result with no signal):
  rejected — the parent names it a failure mode; a requester who
  cannot tell their answer is incomplete will act on it as complete.
- **Full honesty at every level** (exact counts everywhere):
  rejected — counts at high levels disclose corpus shape and
  existence to precisely the identities the classification excludes.
- **Grants override classification** (a grant returns everything it
  reaches): rejected — it turns every sharing grant into a clearance
  escalation; deny-wins (decision 2) keeps the two models orthogonal.
- **Classification creates reach** (clearance alone opens a
  collection): rejected — reach is the owner's explicit act (a
  grant); ambient clearance silently widening it is the same
  escalation in the other direction.
- **Filter after pagination** (page first, then drop withheld rows):
  rejected — pages would shrink unpredictably, cursors would encode
  positions in a stream the requester may not see, and per-page
  counts would leak the withheld distribution.
- **Audit on the requester / audit optional**: rejected — the
  serving owner is the party accountable for what left their
  server; an optional audit is an audit that is missing exactly
  when it matters (hence audit-before-release, decision 6).
- **Redact by dropping fields from the row shape**: rejected —
  #5816's `select` contract promises a row shape; a field that
  vanishes silently is indistinguishable from a schema change.
  The withheld marker keeps the shape honest.

## Implementation invariants

1. No unfiltered row, field value, or count derived from them ever
   crosses the transport in a remote result, in any mode.
2. Effective visibility = grant reach ∩ clearance filter, always;
   neither mechanism widens the other.
3. A predicate over a field above the requester's clearance is the
   structured error `field_withheld` on both executors — never a
   silently-false predicate and never an evaluated one.
4. The honesty envelope follows the rank gradient exactly: counts
   only at the lowest above-clearance level, presence-only above
   it, nothing for the top level or encrypted data.
5. Unclassified data is withheld from remote requesters until
   classified (safe default), whatever #5817 names that state.
6. Filtering precedes ordering/limit/cursor; a cursor never encodes
   a position in the unfiltered stream.
7. Every remote query — served or refused — produces exactly one
   audit record on the serving server; classified-touching queries
   are refused if the audit write fails.
8. The audit is visible to the serving owner only; the requester's
   only filtering signal is the honesty envelope.
9. Validation, predicate semantics, ordering, and error codes are
   identical on local and remote executors (#5816 parity); the
   filtering outcome under clearance is the permitted difference.
10. Encryption is untouched: nothing in this record returns
    plaintext an identity's keys do not entitle it to, and no
    filtering or override rule may be implemented in a way that
    weakens that boundary (parent §5).

## Evidence and sources

- Parent record (PR #7361, branch
  `queue/12752-5675-id-query-design-record`):
  `doc/2026-11-08T06-45-00Z_design_id_query_local_remote_v1/` §4
  (filtering model; honesty and audit mechanics assigned to #5818),
  §5 (encryption boundary), §8 (truncation honesty), §10
  (composition rule recorded by #5818; withheld signalling owned by
  #5818), §9 decomposition map.
- #5816 decision record (on issue #5816, 2026-11-05): structured
  query object, single-table, closed operator set, server-side
  execution, parity invariant, validation-before-execution, cost
  guardrails.
- #5799 decision record (on issue #5799, 2026-11-05): grant units
  (object, stable collection), selector expansion at grant creation,
  no per-field or redacted-field grants in v1.
- #5817 issue body: levels/assignment/defaults and field-vs-row
  behaviour are its scope; it was OPEN at this record's baseline.
- Task 10221 triage verdict on #5818 (2026-11-08, comment 6723779432):
  the scoped follow-on this record executes — filtering before
  results leave the server, withheld honesty, serving-side audit,
  provisional assumptions for #5817.
- Code baseline `69b4906b8f`: `pkgs/id/src/commands/find.rs` and
  `src/commands/get.rs` (MetaRequest transport precedent);
  absence of a filtered-query executor or query audit in `pkgs/id`
  at baseline (repository search, 2026-11-10).

No build or test was run: this change is a design record only. No
implementation was started.
