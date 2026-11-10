# Local groups and grants page at /groups — design record

Issue: #2143. Implementation: PR #4107 (`pkgs/id/src/web/groups.rs`).
This record is the dated design document required by `pkgs/id/AGENTS.md`
("Documenting Design & Architecture Decisions") for the /groups web feature.
It records the three behavioral decisions a reader of the page cannot infer
from the routes table alone.

## Intent

`id serve` already exposes the hosted-world directory at `/explore`. The
group CLI (`id group ...`) maintains a *local* group log — friendships,
groups, and grants recorded on this machine — but had no web surface: an
operator could not see the local groups and grants from the same server
that serves the worlds. #2143 asks for a read-only local groups page.

## Decisions

1. **The page reads the local group log, not the hosted-world directory.**
   `/groups` and `/explore` are different data with different owners:
   `/explore` lists hosted-world directory groups (server-side directory
   state), while `/groups` renders the projection of the local
   append-only group log (`groups.jsonl`) that the group CLI writes. The
   page says so in its own header and links to `/explore` for the
   directory view, so the two are never conflated. Membership status is
   rendered as dissolved only when `dissolved_by` is set, active when
   `GroupView::is_active()`, and otherwise pending — a friendship awaiting
   acceptance renders as pending, not dissolved.

2. **Read-only is load-bearing, including torn-tail handling.**
   The writer-side reader, `GroupLog::open`, repairs the log on open: it
   drops an unterminated final entry (a torn append interrupted before
   its closing newline). A web page must never mutate the file it merely
   displays, so `read_projection` in `web/groups.rs` parses the file
   *without* opening it through `GroupLog::open`. To stay consistent with
   the writer's view of the world it mirrors the repair semantics
   exactly, in memory only: the raw bytes are checked for a trailing
   newline first, every unterminated final entry is dropped — parseable
   or not — and the tail is never decoded before that check, so a torn
   multibyte character at the end of the file cannot fail the whole page.
   A genuinely corrupt *terminated* entry is an error, mirroring
   `GroupLog::open`, rather than being silently skipped.

3. **The log path is the serve command's configured resolution.**
   `AppState.groups_log` carries the resolved group-log path into the web
   layer: `id serve` passes its `--groups-log` resolution
   (`Some(groups_log)` in `commands/serve.rs`), so the page shows the
   same log the operator configured for the server. When no path is
   configured (`None` — tests and embedders), the page reads the default
   `groups.jsonl`, exactly as the group CLI does. There is one source of
   truth for "which log" per surface, and it is the operator's flag, not
   a web-layer default that could drift from the CLI's.

## Route reservation

`/groups` is a host page route: it is registered in `web/routes.rs` and
reserved in `HOST_RESERVED_ROUTES` (`web/app_interface.rs`), so a packaged
app manifest declaring `route = "/groups"` or `socket = "/groups"` is
rejected during interface validation instead of shadowing the host page.

## References

- Issue #2143; PR #4107 (head ecec92cae7 at the time of this record,
  plus the follow-up commit adding this document).
- `pkgs/id/docs/DOCUMENTATION_PROTOCOL.md` — the protocol this record
  follows (`docs/<UTC_RFC_DATETIME>_<kind>_<name>/`).
- Related: [accounts, groups, contacts and delegation](../2026-10-09T12-00-00Z_feature_directory_accounts_groups/2026-10-09T12-00-00Z_feature_directory_accounts_groups.md)
  — the directory/group model the local log belongs to.
