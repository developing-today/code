# Fleet GitHub organization registry

`fleet orgs list` reads the stored Turso snapshot; it does not call GitHub.
Use `--json` for machine-readable output, or `--app`, `--org`, and
`--permission` to narrow the rows. The output includes the last recorded sync
time.

`fleet orgs sync` records a sync run and refreshes orgs, Apps, and
installations idempotently. Installation coverage is sourced only from each
App's JWT-authenticated `GET /app/installations` response, following all
pagination links. OAuth-token organization installation reads are explicitly
not coverage evidence: they have produced false negatives.

The registry stores organization login and verification time; role and
repository count are nullable because the permitted App installation endpoint
does not provide them. App slug, id, owner, permissions and events are stored,
along with each App × account installation id, repository selection,
installed permissions and verification time. The command is capability-gated
as `orgs`; builds without that capability retain the normal Fleet exit-69 stub
behavior.

Tests use fixtures and a fixed registry snapshot only. They do not contact
GitHub or Turso.
