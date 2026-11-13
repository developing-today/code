# Querying local and remote ID servers — v1 design

Design record for issue #5675 ("id: design: Querying local and remote ID
servers v1"). Written per `doc/DOCUMENTATION_PROTOCOL.md` (dated record in
`doc/<UTC>_design_<name>/`). Origin issue: #5675. Sibling and dependent
records/contracts: #2048 (ID data/zettelkasten layer — the tables being
queried), #5732 (ID recommendations — _depends on_ this record; its §3
"extends #5675"), #5683 (residency & boundaries — sensitivity /
selective-sharing grants), #5669 (atlas staging tier). Status: v1 design,
settled at the parent level except the items in §10, which are owned by
the decomposition children. No implementation is authorized by this record
alone; §9 states the child map and the parent-fixed decisions each child
must honour.

This record makes the umbrella design of #5675 durable in `doc/` so the
parent-fixed decisions the children cite (filtering model, the
override/encryption boundary, parity, read-only split, result rules) have a
stable, citable home instead of living only in the issue body. It re-decides
nothing that the decomposition children own; §9 and §12 say what is theirs.

---

## 1. Problem

ID servers hold data in tables. There should be a first-class, designed way
to **query** those tables — on a local server and on remote servers — from
scripts, from agents, and from the human CLI, with the same semantics in all
three. Remote queries are filtered by the data's sensitivity classification;
the owner of a server keeps a deliberate override for their _own_ server's
unencrypted data. This design defines the query surfaces, the filtering model
and its hard boundary, and result shaping; it does not design the individual
mechanisms (those are the children in §9).

## 2. Proposal in one paragraph

One canonical **query object** (table selection, conjunctive field
predicates, projection, ordering, pagination) executes against a **target**
(local or remote) with **identical semantics everywhere** — local and remote
differ only in where execution happens and in the sensitivity filter applied.
A remote server executes the query itself, server-side, under the
requester's clearance, filtering results **before** anything leaves the
server; filtering is a policy filter, never client-side cosmetics. The local
owner may deliberately override sensitivity filtering for their **own,
unencrypted** server data — and only there, because encryption is a
cryptographic boundary that policy never outranks. The v1 query surface is
**read-only by construction**; mutating operations, if exposed at all, are a
separate surface with separate permissions and audit (or an explicit v1
"read-only only" decision record).

## 3. Query surfaces and the query object (parent-fixed)

- **Local queries:** querying tables on a local ID server — in-process /
  embedded where possible, no network round trip, full local performance.
- **Remote queries:** querying tables on a remote ID server over ID transport
  (including over fallback transports), with the **remote server executing
  the query**. "Replicate everything and query locally" is _not_ the only
  option and is not the remote model — it contradicts server-side filtering
  (§4).
- **Query shape (v1):** table selection; filters; projections; ordering;
  and joins **explicitly out of scope** — powerful enough for real use,
  constrained enough to be safe to execute on someone else's server. The
  precise grammar and operator set are **#5816**'s design (landed; see §9).
- **Parity:** the same query, unchanged, is runnable against local and remote
  targets; only the target and the filtering outcome differ.

## 4. Sensitivity classification & filtered remote queries (parent-fixed model; mechanisms in §9)

- **Classification exists:** every table/collection (and, where needed,
  object/row or field) carries a sensitivity classification. The **levels,
  who assigns them, and the default for unclassified data** are **#5817**'s
  design; the parent fixes only that the default for unclassified data is
  **safe, not permissive**.
- **Clearance-scoped execution:** a remote query executes **under the
  requester's clearance**; results are filtered to what that identity is
  cleared for, **server-side**, before anything leaves the server.
- **Filtered-result honesty:** whether the requester learns that results were
  withheld (counts, existence of withheld fields, or nothing) is chosen
  **per classification level** and stated — silent filtering and existence
  leaks are both failure modes. The choice and the field-level behaviour
  (redaction vs. whole-row exclusion vs. rejection) are **#5818**'s design.
- **Audit:** remote queries (who, what query, what levels were touched /
  withheld) are logged on the **serving** server, visible to its owner.
  Mechanics are **#5818**'s.
- **Composition with grants:** selective-sharing grants decide _whether_ a
  peer may reach data; sensitivity filtering decides _what a query returns_.
  Their composition when they disagree is a **parent open question** (§10,
  deny-wins is the working assumption); the decided rule is recorded by the
  child that owns it.

## 5. The owner override — and its hard boundary (parent-fixed boundary; mechanics in §9)

- **Override exists:** the local owner may override sensitivity filtering for
  data on their **own** server.
- **Hard boundary (fixed here, exactly):** the override applies only when the
  data is **not encrypted**. Encryption defeats the override — if data is
  encrypted under keys the querying identity does not hold, no override,
  admin flag, or local access returns plaintext. **Classification is a policy
  filter; encryption is a cryptographic boundary, and policy never outranks
  cryptography.**
- **Scope:** own server only. Owning server A grants no override on server B,
  even for data A originally shared to B.
- **Mechanics** (explicit/deliberate/audited, per-query or per-session,
  never a permanent silent setting) and the **multi-principal server** rule
  (whose override applies to whose data; whether operator and data owner can
  be separated at all) are **#5819**'s design.

## 6. Parity across front doors (parent-fixed: one model; presentation in §9)

- **One query model, three front doors:** a scripting/API surface, an
  agent-facing surface (structured in/out, machine-readable errors, stable
  result schema), and the human CLI — all executing the same queries with the
  same filtering.
- **No privileged back door:** agents and scripts get no broader access than
  their identity's clearance; the CLI gets no magic local bypass except the
  explicit owner override (§5).
- **Non-interactive safety:** scripts/agents can run read-only queries
  without prompts; mutating operations (§7) require explicit intent visible
  in audit.
- Presentation details across the three surfaces are **#5820**'s design.

## 7. Read-only by default vs. mutating surfaces (parent-fixed default; the separation is §9)

- The query surface is **read-only by default and by design**; the v1 query
  language **must not be able to mutate data as a side effect**.
- Mutating operations, if exposed at all, are a **separate** surface —
  distinct commands/endpoints, distinct permissions, distinct audit —
  designed explicitly, not "query language plus write verbs when you're
  local".
- Whether a remote principal may ever mutate via this surface, and under
  what grants, is decided (even if the v1 answer is "no") by **#5822**.

## 8. Result shaping & pagination (parent-fixed requirements; the record is §9)

- **Projection, ordering, limits** are pushed to the executing server, never
  done client-side over a full dump.
- **Pagination** is stable **cursor-based** (no offset drift under concurrent
  writes), with page-size limits enforced server-side.
- **Formats:** structured (JSON/rows) for scripts/agents; human-readable
  tables for the CLI; a SQLite-shaped result is a candidate format for
  composability (see #5816's foreign-adapter query-Redis-get-SQLite pattern).
- **Truncation honesty:** when a result is cut by page size or a server
  cap, the result **says so and how to continue** — never a silent partial
  answer presented as complete. A cut caused by sensitivity filtering is
  reported **only as far as the per-classification disclosure policy (§4;
  #5818's design) permits**: where that policy withholds counts or even the
  existence of filtered data, the result does not announce that filtering
  occurred — announcing it would leak exactly what the policy hides.
- **Cost limits:** server-side guardrails (timeout, scan cap) so a remote
  query cannot DoS the serving server.
- The settled result-format and cursor semantics record is **#5821**.

## 9. Decomposition map and status

The parent's decomposition (as fixed in #5675) and current child state at
`origin/main` (2026-11-08). Parent-fixed decisions each child must honour are
named; no child re-decides a parent decision.

| Child | Scope                                                                                                 | Status                                                               | Parent-fixed input it inherits                                      |
| ----- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------- |
| #5816 | ID query language/shape v1 (local + remote execution model, parity)                                   | **CLOSED** — decision record landed 2026-11-05 (record on the issue) | §3 shape rules; §4 remote-is-server-side; §5 boundary; §7 read-only |
| #5817 | Sensitivity classification model (levels, assignment, defaults, field vs. row)                        | OPEN — queued                                                        | §4 model + safe default                                             |
| #5818 | Server-side filtered remote query execution + withheld-result honesty + audit                         | OPEN (parked/assigned)                                               | §4 model; §3 parity; composition rule (§10)                         |
| #5819 | Owner override mechanics (scope, audit, multi-principal) & encryption boundary                        | OPEN (in-progress/claimed)                                           | §5 boundary (verbatim)                                              |
| #5820 | Client surfaces parity — scripting/API, agent, CLI over one model                                     | OPEN (parked/assigned)                                               | §6 + outputs of #5816/#5818/#5821/#5822                             |
| #5821 | Result shaping & cursor pagination (formats, truncation honesty, cost limits)                         | OPEN                                                                 | §8 requirements + #5816's shape                                     |
| #5822 | Mutating surface separation (permissions, audit) — or an explicit v1 "read-only only" decision record | OPEN                                                                 | §7 read-only-by-construction                                        |

**Ordering (from the design's own dependencies).** #5817 (classification
levels/defaults) and #5816 (query shape) are foundational and independent of
each other. #5818 composes #5816 + #5817 (and the grants model — §10).
#5819 depends on #5817's levels. #5821 depends on #5816's shape. #5820
consumes #5816/#5818/#5821 outputs, so it lands last among the read-path
designs. #5822 is an independent decision record whose answer feeds #5820's
permission model. Design prioritisation is independent per ticket;
implementation timing is a separate decision.

## 10. Open questions (owned by the children, not re-decided here)

- **Classification levels:** how many, named how, can owners define their
  own? → **#5817**.
- **Withheld-result signalling:** counts, yes/no, or nothing — per level?
  → **#5818**.
- **Query language form:** a new ID-specific shape, a SQL subset, or both
  (SQL compiled to the ID shape)? → settled by **#5816** (object is the
  semantics; SQL later as sugar) — see its record.
- **Grants vs. classification when they disagree (deny-wins?):** a
  **parent-level** open question, recorded by **#5818**'s execution design as
  it composes the grants model; the parent fixes only that composition must
  be stated, not left implicit.
- **Multi-principal override separation** (operator vs. data owner) and
  whether the model can prevent operator override over others' unencrypted
  data → **#5819**.
- **Cursor stability:** snapshot semantics per query, or best-effort with
  documented drift? → **#5821**.
- **SQLite-shaped result** as a v1 format or follow-on? → **#5821**.

## 11. Dependencies and composition

- **Composes with #5683** (residency & boundaries / sensitivity rules) and
  the selective-sharing grants model: grants decide _reach_, classification
  decides _content returned_ (§4; §10 composition rule).
- **Depends on #2048** (ID data/zettelkasten layer): this design queries the
  tables that layer defines.
- **Is a hard prerequisite of #5732** (recommendations): #5732's v1 record
  names #5675 (visibility semantics) and #5683 (visibility classes) as hard
  prerequisites; its filter-before-ranking contract consumes this design's
  severity/clearance model.
- **Execution tier:** remote-query execution and cost limits align with the
  #5669 staging-tier pattern where computation placement is needed.

## 12. What this record does NOT decide

- The classification level names/assignment and the query grammar — #5817
  and #5816 respectively.
- Filtered-execution mechanics, withheld-result wording, audit schema —
  #5818.
- Override mechanics and multi-principal separation — #5819.
- The three surfaces' presentation — #5820.
- Result formats, cursor semantics, truncation wording — #5821.
- Whether a mutating surface exists in v1 at all — #5822.

## 13. Acceptance (from #5675, met)

A follow-on reader can break this design into the decomposition tickets
without re-deciding: the feature set, the non-goals, the override/encryption
boundary, and the open questions are all stated; the seven children are filed
and mapped (§9); the parent-fixed decisions each child inherits are named.
This record records that decomposition and the landed child design record
(#5816) in `doc/`.
