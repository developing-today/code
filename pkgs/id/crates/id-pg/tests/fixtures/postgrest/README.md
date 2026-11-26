# id-pg PostgREST contract fixture

`init.sql` is loaded into a disposable PostgreSQL database before PostgREST
starts. Expose only schema `api` with `PGRST_DB_SCHEMAS=api`, use
`authenticator` as the authenticator role and `fixture_reader` as the anonymous
role, and set `PGRST_JWT_SECRET` to a test-only secret of at least 32 bytes.
The authenticator's database password is `fixture_password`.

The runtime URL is supplied as `ID_PG_POSTGREST_URL`. Run the normal test
inventory without a database using `cargo test -p id-pg --test postgrest_unit`;
the live cases are ignored by default and must be explicitly enabled with
`cargo test -p id-pg --test postgrest_contract -- --ignored`. The latter fails
if its URL is missing. Every live success and source-side failure goes through
real PostgREST/PostgreSQL. Only the impossible “HTTP 200 with HTML” case uses
the harness's one-shot local HTTP server, since PostgREST does not produce that
success response.

Contract views:

- `contract_values`: one deterministic row covering every `PgType`, including
  JSON/JSONB opaque and declared shapes, a nullable value, and a 64-bit integer
  that JSON parsers must not round.
- `nan_value`: PostgreSQL's non-finite double precision JSON representation.
- `sql_failure`: raises SQLSTATE `PT503`, which PostgREST maps to HTTP 503.
- `slow_value`: sleeps long enough for the live curl deadline to expire.

The script initializes a test fixture only; the user-facing HTTP credential
case supplies a deliberately invalid bearer JWT. Do not use these credentials
outside disposable CI containers.
