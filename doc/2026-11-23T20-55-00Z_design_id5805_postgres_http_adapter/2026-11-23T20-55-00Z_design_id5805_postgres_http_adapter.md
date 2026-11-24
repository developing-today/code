# Design: Postgres-over-HTTP adapter — query translation scope and honest refusals

- **Date:** 2026-11-23
- **Status:** decision record; no adapter implementation is included.
- **Issue:** [#5805](https://github.com/developing-today/code/issues/5805),
  “id: design: Postgres-over-HTTP adapter (from #5672)”.
- **Parent:** [#5672](https://github.com/developing-today/code/issues/5672),
  “ID selective sharing and foreign data-store adapters v1” (closed; its
  remainder lives in the child design tickets).
- **Contract:** [`2026-11-08_design_id5803.md`](../2026-11-08_design_id5803.md)
  (shared foreign adapter contract, #5803). This record applies it; it does not
  restate it.
- **Composes with:** [#5818 remote filtered query](../2026-11-10_decision_id5818_remote_filtered_query.md),
  [#5909 SQL/PGQ boundary](../2026-11-08_decision_id5909_sql_pgq_capability.md)
  (v1 query object is single-table, no joins, no raw SQL), the libsql HTTP
  decision on #5806, and the adapter record for #5812
  ([`2026-11-14T13-10-00Z_design_id5812_adapter_record.md`](../2026-11-14T13-10-00Z_design_id5812_adapter_record.md)).
- **Evidence baseline:** `origin/main` at `dfd7d9eb7b4` (2026-11-23). No
  Postgres HTTP adapter, PostgREST client, or query translator exists in
  `pkgs/`. The queue broker in `pkgs/fleet/crates/fleet-queue/src/queue_broker.rs`
  does not use PostgREST, so it is not precedent for this adapter. The portable-core rule in the Atlas
  scaler revision (`doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision/`,
  around lines 455–480) is the in-repo statement about Postgres over REST:
  the portable core uses standard Postgres behind a specified REST contract,
  PostgREST-shaped, with no Supabase-only features.

## Decision summary

| Question                                                                                            | Decision                                                                                                                                                                                                                                                                                                                                     |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Which HTTP interface is canonical for Postgres?                                                     | **A PostgREST-compatible REST surface** over one exposed schema. It is the only interface the adapter speaks. It is open source, runs against any Postgres, and is already the in-repo portable-core choice.                                                                                                                                 |
| Is arbitrary SQL over HTTP supported?                                                               | **No.** Neither a `/query` SQL endpoint nor a PostgREST `/rpc/*` function call is a v1 shape. Both break the typed-shape rule in #5803 §2.2 and the data-shaped query object of #5816.                                                                                                                                                       |
| Where no PostgREST-compatible endpoint exists (bare Postgres on a host with only the wire protocol) | The adapter is **unavailable for that endpoint**. It reports `adapter_unavailable` with the reason `no_rest_surface`. It does not fall back to SQL-over-HTTP, and it does not speak the native wire protocol (out of scope for a browser- or sandbox-reachable adapter). The owner can deploy PostgREST in front of the database to qualify. |
| Query translation scope                                                                             | A **single collection** (one table or view, per #5803 §2.1), **conjunctive** filters over a closed operator set, a **declared projection**, **ordering on declared keys only**, **keyset** pagination only, and **no joins, writes, RPC, counts, or embeds**. Everything else is refused by name (§3).                                       |
| Read modes                                                                                          | **Live** and **materialized** are both advertised; the owner must choose per collection with no default (#5803 §4). Materialized is **not atomic** over PostgREST pages and is reported with `consistency: per_request` (§5).                                                                                                                |
| Where grants are enforced                                                                           | At **two** layers: the adapter validates the query against the collection's allowlist before sending it (§3), and the **database role** the adapter uses can `SELECT` only from the exposed views (§4). The adapter is not trusted alone.                                                                                                    |
| How refusal is surfaced                                                                             | As `query_shape_refused` with a stable feature name. It is surfaced identically for unknown columns and withheld columns, so a refusal reveals nothing about the data (§6).                                                                                                                                                                  |

## 1. Parent and contract decisions applied, not re-decided

Taken as fixed from #5672 and #5803:

1. A collection is the grantable unit. The adapter exposes exactly the owner's
   allowlist of collections; nothing else is enumerable.
2. Reads run inside the caller's grant. The restriction is pushed into the
   source query where it can be. If it cannot be enforced safely, the query is
   refused before any source row is returned. Fetch-then-filter is never used.
3. Grantee-visible outcomes collapse “not granted”, “does not exist”, and
   “unavailable” to `not_available`, except where the collapse would hide a
   shape refusal that depends only on the query text (§6).
4. Read-only in v1. Materialization copies into ID and does not write back.
5. The credential is a local `credential_ref`. It never appears in an ID
   URI, a record, a log, or an error.

Nothing in this record changes a parent decision. §8 lists the points where
the parent leaves an open question and how this record answers it locally.

## 2. Endpoint, addressing, and transport

- **Addressing.** The ID-level reference is
  `id://foreign/postgres/<source-alias>/<schema>.<relation>`, where `<relation>`
  is a table or view exposed to PostgREST. The alias resolves only in local
  endpoint configuration (#5803 §2.1). The URI carries no host, user, or
  password. Rebinding an alias to another database is a source-identity change.
- **Native locator.** The base URL and the schema name stay in local endpoint
  configuration. They never go into shared provenance unless the owner
  discloses them under a grant.
- **Transport.** HTTPS only. Plain `http://` is accepted only for a loopback
  address, so that a local PostgREST for development works. Redirects are
  refused. Requests carry a per-request deadline and a concurrency cap per
  endpoint.
- **Schema pinning.** The request sets `Accept-Profile` to the one exposed
  schema. A request that names any other schema is refused by the adapter.
  The PostgREST `db-schemas` setting must list only that schema. The adapter
  checks this at registration by refusing an endpoint whose schema probe
  returns more than one schema.
- **Credentials.** A JWT or API key is resolved from `credential_ref` just
  before each request and sent as a bearer token in the `Authorization`
  header. It is never placed in the URL, never logged, and never copied into
  an error body. Responses that echo the request headers are treated as
  secret-bearing and dropped.

## 3. Query translation scope (v1)

The adapter accepts a #5816-shaped query object for one collection and
translates it to a PostgREST request. Anything outside the list below is
refused before any network call.

### 3.1 Supported

| Query element | Translation                                                                                                | Notes                                                                                                                                            |
| ------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Collection    | Exactly one relation, from the owner's allowlist                                                           | Unknown or unlisted relation → `not_available` (§6)                                                                                              |
| Projection    | `select=` with the declared column names only                                                              | `*` is not expanded from the source. The adapter sends the declared column list, so a column added to the view later is not exposed by accident. |
| Filters       | Conjunction (AND) of `eq`, `neq`, `lt`, `lte`, `gt`, `gte`, `in`, `is.null` on declared filterable columns | Each becomes a PostgREST `col=op.value` term. Values are typed by the mapping (§5) before serialization.                                         |
| Ordering      | `order=` on declared orderable columns, with the declared unique key appended as the final tiebreak        | Without the appended key, ordering is not total and pagination would drift. The adapter adds the key itself.                                     |
| Pagination    | **Keyset** only: the cursor is the last row's ordering key tuple, sent as a filter on the same ordering    | Offset pagination is refused (`offset_unsupported`). Keyset gives stable pages under concurrent writes, matching #5821.                          |
| Limit         | `limit=` capped at a declared per-collection maximum, default 500 rows                                     | A client `limit` above the cap is refused, not clamped. A silent clamp would hide truncation.                                                    |
| Freshness     | Live or pinned-materialized (§5)                                                                           | Freshness hints may narrow, never widen.                                                                                                         |

### 3.2 Refused by name

| Refused shape                                                | Feature name in the error  | Why                                                                             |
| ------------------------------------------------------------ | -------------------------- | ------------------------------------------------------------------------------- |
| Joins and PostgREST embeds (`select=*,other(*)`)             | `join_unsupported`         | Embeds read a second relation under one grant. #5909 keeps v1 single-table.     |
| OR, NOT, or nested boolean trees                             | `boolean_unsupported`      | Keeps predicate pushdown and the grant check exact.                             |
| `like`, `ilike`, `match`, full-text (`fts`, `plfts`, `wfts`) | `text_pattern_unsupported` | Unindexed patterns are scans, and a scan is a cost-limit issue (#5821).         |
| Offset or page-number pagination                             | `offset_unsupported`       | Drifts under concurrent writes.                                                 |
| Exact or planned counts (`Prefer: count=exact`)              | `count_unsupported`        | A count discloses the size of a withheld set (§6, invariant 3).                 |
| RPC (`/rpc/*`)                                               | `rpc_unsupported`          | A function can read any relation its role can reach. It bypasses the allowlist. |
| Writes (`POST`, `PATCH`, `PUT`, `DELETE`, upsert)            | `write_unsupported`        | Read-only in v1 (#5803 §5).                                                     |
| Schema or OpenAPI discovery (`GET /`)                        | `discovery_unsupported`    | A grantee must not see the schema beyond the collection's own declared shape.   |
| Ordering or filtering on an undeclared column                | `field_unsupported`        | Same wording as for a withheld column (§6).                                     |
| Raw SQL, or any `/query` endpoint                            | `sql_unsupported`          | Rejected by #5803 §8.5 and #5909.                                               |
| A limit above the declared cap                               | `limit_exceeded`           | Refused, not clamped.                                                           |

## 4. Enforcement layers

Each layer is a check on its own. A bug in one does not open the others.

1. **Adapter allowlist.** Path, `Accept-Profile`, columns, operators, and
   limits are checked against the owner's collection config before the request
   leaves the process. The check is pure and can be property-tested (§9).
2. **Database role.** The credential maps to a role whose only privilege is
   `SELECT` on the exposed views, and it has no `EXECUTE` on any function. The
   owner creates the exposed views. A view can carry a row filter, so the
   row-level grant lives in SQL the owner controls, not in the adapter.
3. **PostgREST config.** `db-schemas` lists the one exposed schema.
   `db-anon-role` is set to the same read-only role or to none. `db-max-rows`
   is set to the declared cap, so a misconfigured request cannot pull more
   rows than the collection allows.

A grantee query therefore cannot reach an unlisted relation even if the
adapter check has a defect, and cannot read rows the view hides even if the
role has broader privileges than intended. The owner config is checked at
registration: a role with `INSERT`, `UPDATE`, `DELETE`, or `EXECUTE` on the
exposed schema is a registration error.

## 5. Typed mapping, live reads, and materialized snapshots

### 5.1 Type mapping (mapping version 1)

PostgREST returns JSON, and JSON loses type information. The mapping
therefore uses the column types the owner declares, not whatever JSON
happens to say:

| Postgres type                        | ID value                                                                                               | Rule                                                                                                                                   |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| `boolean`                            | `boolean`                                                                                              | Direct.                                                                                                                                |
| `int2`, `int4`, `int8`               | `integer`                                                                                              | Parsed from the JSON number text, never through `f64`. Postgres `int8` is already within signed 64-bit range, so no tagging is needed. |
| `float4`, `float8`                   | `float64`                                                                                              | `NaN` and infinities are rejected (`type_mapping_failed`).                                                                             |
| `numeric`                            | `tagged` (`type_id: pg.numeric`, decimal text payload)                                                 | A decimal string is not a `float64`. The tag keeps precision and marks that native ordering is lost (#5803 §3.3).                      |
| `text`, `varchar`, `char`, enums     | `string`                                                                                               | Must be valid UTF-8. Enums arrive as text and are validated against the declared label set.                                            |
| `bytea`                              | `bytes`                                                                                                | PostgREST encodes bytea as `\x` hex. The adapter decodes it. It is never read as text.                                                 |
| `timestamptz`                        | `timestamp`                                                                                            | ISO-8601 with offset, converted to epoch microseconds. Postgres stores microseconds, so no precision is lost.                          |
| `date`, `time`, `timetz`, `interval` | `tagged`                                                                                               | Kept as their own type IDs. Date arithmetic is not promised.                                                                           |
| `json`, `jsonb`                      | `record` or `list` **only** when the owner declares the shape; otherwise `tagged` (`pg.jsonb`, opaque) | Undeclared JSON is never flattened into records.                                                                                       |
| `uuid`                               | `string`                                                                                               | Canonical lowercase text.                                                                                                              |
| Arrays                               | `list` of the element mapping                                                                          | Element type must itself be mapped.                                                                                                    |
| `NULL`                               | `null`                                                                                                 | Distinct from absent, per #5803 §3.                                                                                                    |

Any value that does not match its declared type is `type_mapping_failed`,
with the column name and the failing type, but **not** the value. The
mapping version is stored with each materialized snapshot and each live
response. A change to the mapping is a new version (#5803 §3.5).

The JSON decoder must keep numbers as text until the declared type is known.
The adapter does not use a parser that rounds integers above 2^53.

### 5.2 Live reads

- One PostgREST request per page. The response reports the observation time
  of the last page, the consistency boundary, and whether the result is
  complete.
- A page is complete when `Content-Range` says so or when the row count is
  below the limit. A truncated page carries an explicit `truncated: true`
  marker. A failed page is an error, never a short success (#5803 §6).
- No live read falls back to an older snapshot. A timeout or a source error is
  an error (#5803 §4).
- **Consistency:** `per_request`. Two pages of one live query can observe
  different database states. The adapter says so in every response and does
  not claim snapshot semantics.

### 5.3 Materialized snapshots

- A snapshot is built by keyset paging over the whole allowed set, under the
  declared row and byte caps. The caps are checked before rows are copied.
  Exceeding a cap is `snapshot_too_large`, and no partial snapshot is kept.
- **PostgREST cannot pin one database snapshot across requests.** So the
  snapshot is **not atomic**. Its consistency is `per_request`, which is
  reported in provenance. It must not be described as a point-in-time copy.
  A point-in-time copy needs either a database-side export or a snapshot
  endpoint, both out of scope for v1.
- The snapshot is atomically replaced in ID only when every page succeeded.
  An interrupted build leaves the previous complete snapshot in place (#5803
  §4).

### 5.4 Owner mode choice

Both modes are advertised. The owner must pick one per collection. Until the
owner picks, the collection is unavailable. The adapter has no default. This
follows #5803 §4, which allows an adapter to refuse until the owner decides.
For Postgres the recommended choice is **live**, because the consistency
gap of materialization is larger than for the libsql case.

## 6. Refusal and error surface

### 6.1 Codes

Every error uses a stable code from #5803 §6. Postgres-specific mappings:

| Source condition                                      | Code                                                                              | Grantee-visible                                                                          |
| ----------------------------------------------------- | --------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Shape not in §3.1                                     | `query_shape_refused` (with a feature name from §3.2)                             | Yes. The shape is known from the query text alone, so it reveals nothing about the data. |
| Unknown column, unlisted relation, or withheld column | `query_shape_refused` with `field_unsupported`, or `not_available` for a relation | Same wording for “unknown” and “withheld”.                                               |
| 401 or 403 from PostgREST                             | `credential_unavailable` to the owner; `not_available` to a grantee               | Owner diagnostics may name the cause.                                                    |
| 404 for the relation or path                          | `not_available`                                                                   | Collapsed.                                                                               |
| 429 or a configured capacity limit                    | `rate_limited`                                                                    | Yes; a rate limit depends on load, not on data.                                          |
| Deadline exceeded                                     | `timeout`                                                                         | Yes. Never an empty result.                                                              |
| Connection failure, 5xx, or a non-JSON body           | `adapter_unavailable`                                                             | Yes. A non-JSON 200 is an error, not a success.                                          |
| Value outside mapping                                 | `type_mapping_failed`                                                             | Column and type named; value not included.                                               |

### 6.2 Collapse rule

Refusals that depend only on query text (`query_shape_refused`,
`limit_exceeded`) are shown as they are. Anything that depends on the data or
the grant is collapsed to `not_available`. The collapse covers status, result
shape, metadata, and error text. This rule is the reason an undeclared column
and a withheld column produce the same `field_unsupported` text.

## 7. Alternatives considered and rejected

1. **Raw SQL over HTTP (a `/query` endpoint).** Rejected.
   It is the pass-through #5803 §8.5 forbids. Grants could only be enforced by
   parsing SQL, which is not a safe thing to rely on. It also makes refusals
   depend on SQL dialect details.
2. **Expose `/rpc/*` functions.** Rejected for v1. A function is an opaque
   query under the function owner's rights, so it cannot be checked against a
   collection grant. Revisit only with a per-function declaration and a
   separate design.
3. **Speak the native Postgres wire protocol from the adapter.** Rejected for
   v1. It is not HTTP, so it cannot serve browser or HTTP-only sandbox
   callers. It is listed in the parent as the reason for HTTP in the first
   place.
4. **Use a Supabase- or Neon-specific data API.** Rejected. The in-repo
   portable-core rule forbids Supabase-only features in the core, and a
   provider-specific API would make the adapter depend on one host.
5. **Clamp oversized limits instead of refusing them.** Rejected. A clamp
   would silently truncate, which #5803 §6 forbids.
6. **Offset pagination for simplicity.** Rejected. Pages drift under writes,
   and drift leaks information about rows a grantee cannot see.
7. **Materialize with a claimed point-in-time copy.** Rejected. PostgREST
   cannot pin a snapshot across pages, so the copy would be false provenance.

## 8. Open questions from the parent, answered here

| Parent open question                                                                 | Answer in this record                                                                                                                     |
| ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Which HTTP interface is canonical for Postgres, and what do we do where none exists? | PostgREST-compatible surface. Where none exists, the endpoint is `adapter_unavailable` with `no_rest_surface`. No SQL-over-HTTP fallback. |
| Existence privacy vs. useful errors                                                  | Shape refusals are named. Data- and grant-dependent outcomes collapse to `not_available` (§6.2).                                          |
| Are adapters live, materialized, or both, and who decides?                           | Both are advertised; the owner decides per collection with no default (§5.4).                                                             |

## 9. Invariants implementation must hold

1. The adapter sends a request only when the query passes the §3.1 allowlist.
   Every refusal in §3.2 happens before any network call.
2. The request names exactly one schema, through `Accept-Profile`, and that
   schema is the one the endpoint registration checked.
3. A grantee error never distinguishes withheld from absent, and never reports
   a count of withheld rows. Counts are refused outright.
4. The credential is never in a URL, record, log, or error body. Echoed
   request headers are dropped.
5. Every numeric column is parsed from JSON text under its declared type. No
   integer is passed through `f64`.
6. A value that fails its declared type is `type_mapping_failed` and is not
   reported by value.
7. A live response states `per_request` consistency and the observation time.
   A snapshot is never described as atomic.
8. A timeout, a 5xx, or a non-JSON body is an error. It is never an empty
   result and never a fallback to an older snapshot.
9. The registration check refuses a role that holds write or `EXECUTE`
   privileges on the exposed schema, and refuses an endpoint whose
   `db-schemas` lists more than the one exposed schema.
10. The adapter has no write path. Its HTTP client has no `POST`, `PATCH`,
    `PUT`, or `DELETE` method, so a write cannot be expressed.

## 10. Verification plan for the implementation

These are the tests the implementation issues must add. This record adds none.

- **Property tests** over the query object: for every generated query in the
  supported set, the translated request uses only declared columns and
  operators, and the pagination key is total. Oracle: the §3 allowlist.
- **Model-based tests** of the refusal table: each §3.2 shape maps to its
  feature name, and an undeclared column and a withheld column produce
  identical error bodies. Oracle: the §6 table.
- **Contract tests** against a real PostgREST container in CI, with an
  injectable transport for the unit layer. Cases: each §5.1 type, a `bigint`
  near 2^63, `NaN` in a float, a `bytea` value, a deadline, a 5xx, a non-JSON
  200, and a 401. Oracle: §5 and §6 expected outcomes.
- **Enforcement tests**: a role with extra privileges is refused at
  registration, and a request for an unlisted relation returns `not_available`
  with the same body shape as a missing one.

The docs build for this record was checked as part of the PR (see the PR
outcome on #5805 for the exact commands and counts).

## 11. Implementation slices filed

Each slice is a separate implementation issue, linked from #5805. None is
started here.

- [#13758](https://github.com/developing-today/code/issues/13758): adapter registration and owner config (§4 checks).
- [#13760](https://github.com/developing-today/code/issues/13760): query translator and allowlist (§3, §10 property and model tests).
- [#13761](https://github.com/developing-today/code/issues/13761): typed mapping v1, number-as-text rule (§5.1).
- [#13762](https://github.com/developing-today/code/issues/13762): error mapping and grantee collapse (§6).
- [#13763](https://github.com/developing-today/code/issues/13763): live and materialized read paths, snapshot caps (§5.2, §5.3).
- [#13764](https://github.com/developing-today/code/issues/13764): PostgREST conformance fixtures (§10 contract tests).

## Sources checked

- Issue #5805 and its comments (read 2026-11-23, count-confirmed at 11 comments).
- Parent #5672 (closed; body read 2026-11-23).
- [`2026-11-08_design_id5803.md`](../2026-11-08_design_id5803.md) (§§2–8, the adapter contract).
- [`2026-11-10_decision_id5818_remote_filtered_query.md`](../2026-11-10_decision_id5818_remote_filtered_query.md).
- [`2026-11-08_decision_id5909_sql_pgq_capability.md`](../2026-11-08_decision_id5909_sql_pgq_capability.md) (single-table, no-join v1 boundary).
- [`2026-11-08T06-45-00Z_design_id_query_local_remote_v1`](../2026-11-08T06-45-00Z_design_id_query_local_remote_v1/2026-11-08T06-45-00Z_design_id_query_local_remote_v1.md) (§§3, 8).
- [`2026-11-14T13-10-00Z_design_id5812_adapter_record.md`](../2026-11-14T13-10-00Z_design_id5812_adapter_record.md) (adapter record shape).
- Atlas scaler revision, portable-core Postgres rule (around lines 455–480).
- `pkgs/fleet/crates/fleet-queue/src/queue_broker.rs`: searched for PostgREST and `rest/v1` usage; none found, so it is not precedent.
