//! The read-only `/groups` page: this node's local group event log
//! (`crate::groups`), folded into a [`Projection`] and rendered — groups,
//! their members and roles, and the grants in force.
//!
//! This is NOT the hosted-world directory group list at `/explore`
//! (a `id_directory::directory_view::DirectoryView`); it is the local
//! `groups.jsonl` event log the group commands and `id serve` maintain.
//!
//! Read-only is load-bearing: [`GroupLog::open`](id_groups::groups::log::GroupLog::open)
//! truncates a torn final line, which is a mutation — a page GET must
//! never repair the file. [`read_projection`] therefore parses the file
//! itself with the same torn-tail tolerance (every unterminated final
//! entry is dropped, parseable or not, exactly as `GroupLog::open` drops
//! it) but writes nothing, ever.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use super::AppState;
use super::templates::{html_escape, render_page};
use id_events::groups::{Event, Key, Projection, fold};

/// Resolves the group-log path the web page reads: the configured
/// path when one was passed in, else the default file name the group
/// CLI uses (`groups.jsonl`).
pub fn resolve_groups_log(configured: Option<PathBuf>) -> PathBuf {
    configured.unwrap_or_else(|| PathBuf::from(id_groups::groups::log::FILE_NAME))
}

/// The group-log path for this state: the path `id serve` resolved
/// from `--groups-log` (or its default) and passed into the web state.
pub fn groups_log_path(state: &AppState) -> PathBuf {
    resolve_groups_log(state.groups_log.clone())
}

/// Reads the group log at `path` and folds it, without modifying the
/// file in any way (see the module docs). A missing file is an empty
/// log. A newline-terminated line that does not parse is corruption
/// and fails, mirroring `GroupLog::open`; a torn final line is dropped
/// from the read, never from the file.
///
/// # Errors
///
/// Fails if the file cannot be read, a complete line is not an event,
/// or the events cannot be folded.
pub fn read_projection(path: &Path) -> Result<Projection> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let mut events: Vec<Event> = Vec::new();
    for (number, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !line.ends_with(b"\n") {
            // Torn final entry: `GroupLog::open` drops every unterminated
            // final entry, parseable or not -- mirror that here, without
            // truncating the file.
            break;
        }
        let text = std::str::from_utf8(line)
            .with_context(|| format!("{}: line {} is not an event", path.display(), number + 1))?;
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(text.trim_end()) {
            Ok(event) => events.push(event),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("{}: line {} is not an event", path.display(), number + 1)
                });
            }
        }
    }
    Ok(fold(&events)?)
}

fn key_hex(key: &Key) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// Renders the groups page content (without the page shell) from a
/// folded projection: every group with its ID and active/pending/
/// dissolved status, its direct members and roles, and the grants in force in
/// it — via [`Projection::grants_in_force`], so a grant whose grantor
/// was later removed still shows while it remains in force.
pub fn render_groups(projection: &Projection) -> String {
    let grants: Vec<&id_events::groups::Grant> = projection.grants_in_force().collect();
    let mut html = String::from(
        "<h1>Groups</h1>\
         <p>This node's local groups from its group event log, read-only. \
         The hosted-world directory groups live at <a href=\"/explore\">/explore</a>.</p>",
    );
    let mut any = false;
    for (id, view) in projection.groups() {
        any = true;
        let status = if view.dissolved_by.is_some() {
            "dissolved"
        } else if view.is_active() {
            "active"
        } else {
            // Not dissolved, but not yet in effect -- e.g. a friendship
            // still awaiting acceptance.
            "pending"
        };
        html.push_str(&format!(
            "<section class=\"group\"><h2>Group {}</h2><p>Status: {}</p>",
            id, status
        ));
        if view.members.is_empty() {
            html.push_str("<p>No members.</p>");
        } else {
            html.push_str("<table><thead><tr><th>Member</th><th>Role</th></tr></thead><tbody>");
            for (member, role) in &view.members {
                html.push_str(&format!(
                    "<tr><td>{}</td><td>{:?}</td></tr>",
                    key_hex(member),
                    role
                ));
            }
            html.push_str("</tbody></table>");
        }
        let group_grants: Vec<&&id_events::groups::Grant> =
            grants.iter().filter(|grant| grant.group == *id).collect();
        if group_grants.is_empty() {
            html.push_str("<p>No grants in force.</p>");
        } else {
            html.push_str(
                "<table><thead><tr><th>Grantor</th><th>Grantee</th><th>Scope</th>\
                 <th>Redelegable</th><th>Target world</th></tr></thead><tbody>",
            );
            for grant in group_grants {
                html.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{:?}</td><td>{}</td><td>{}</td></tr>",
                    grant
                        .from
                        .as_ref()
                        .map(key_hex)
                        .unwrap_or_else(|| "(group)".to_owned()),
                    key_hex(&grant.to),
                    grant.scope,
                    if grant.redelegate { "yes" } else { "no" },
                    grant
                        .target
                        .as_deref()
                        .map(html_escape)
                        .unwrap_or_else(|| "-".to_owned()),
                ));
            }
            html.push_str("</tbody></table>");
        }
        html.push_str("</section>");
    }
    if !any {
        html.push_str("<p>No groups in this node's group log yet.</p>");
    }
    html
}

/// `GET /groups` — the read-only local groups and grants page.
pub async fn groups_page_handler(State(state): State<AppState>) -> Response {
    let path = groups_log_path(&state);
    match read_projection(&path) {
        Ok(projection) => {
            let content = render_groups(&projection);
            Html(render_page("Groups", &content, "", &state.assets)).into_response()
        }
        Err(error) => {
            let content = format!(
                "<h1>Groups</h1><p>Could not read the group log at {}: {}</p>",
                html_escape(&path.display().to_string()),
                html_escape(&format!("{error:#}"))
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(render_page("Groups", &content, "", &state.assets)),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id_events::groups::policy::{Delegation, Policy, RemoveMode, Template};
    use id_events::groups::{Body, Create, Delegate, MemberOp, Role};
    use id_groups::groups::log::GroupLog;
    use tempfile::TempDir;

    const OWNER: Key = [1; 32];
    const ANN: Key = [2; 32];

    fn create_body(policy: Policy, members: Vec<(Key, Role)>) -> Body {
        Body::Create(Create {
            template: Template::Custom,
            policy,
            members,
            admins_on_create: false,
        })
    }

    /// A group whose grantor grants ANN a scope, then leaves; under
    /// `KeepPrior` the grant made before the removal stands.
    fn log_with_retained_grant(dir: &TempDir) -> PathBuf {
        let path = dir.path().join(id_groups::groups::log::FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let policy = Policy {
            delegation: Some(Delegation::KeepPrior),
            ..Policy::default()
        };
        let group = log
            .record(OWNER, None, create_body(policy, vec![(ANN, Role::Write)]))
            .unwrap()
            .event;
        log.record(
            OWNER,
            Some(group),
            Body::Delegate(Delegate {
                grantee: ANN,
                scope: Role::Read,
                can_redelegate: true,
            }),
        )
        .unwrap();
        log.record(
            OWNER,
            Some(group),
            Body::Member(MemberOp::Remove {
                member: OWNER,
                mode: RemoveMode::KeepPrior,
            }),
        )
        .unwrap();
        path
    }

    #[test]
    fn retained_grant_from_removed_grantor_stays_visible() {
        let dir = TempDir::new().unwrap();
        let path = log_with_retained_grant(&dir);
        let projection = read_projection(&path).unwrap();
        let grants: Vec<_> = projection.grants_in_force().collect();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].from, Some(OWNER));
        assert_eq!(grants[0].to, ANN);
        assert_eq!(grants[0].scope, Role::Read);
        assert!(grants[0].redelegate);

        let html = render_groups(&projection);
        // Grantor (removed) and grantee both render, with the scope.
        assert!(html.contains(&key_hex(&OWNER)));
        assert!(html.contains(&key_hex(&ANN)));
        assert!(html.contains("Read"));
        // ANN's direct membership role renders too.
        assert!(html.contains("Write"));
    }

    #[test]
    fn torn_tail_read_leaves_the_file_byte_identical() {
        let dir = TempDir::new().unwrap();
        let path = log_with_retained_grant(&dir);
        let good = std::fs::read(&path).unwrap();
        let mut torn = good.clone();
        torn.extend_from_slice(b"{\"id\":\"f00");
        std::fs::write(&path, &torn).unwrap();

        let projection = read_projection(&path).unwrap();
        assert_eq!(projection.groups().count(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), torn);
    }

    #[test]
    fn missing_log_file_is_an_empty_projection() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("absent.jsonl");
        let projection = read_projection(&path).unwrap();
        assert_eq!(projection.groups().count(), 0);
        assert!(render_groups(&projection).contains("No groups"));
    }

    #[test]
    fn configured_path_wins_and_default_is_the_cli_file_name() {
        let custom = PathBuf::from("/tmp/somewhere/custom-groups.jsonl");
        assert_eq!(resolve_groups_log(Some(custom.clone())), custom);
        assert_eq!(
            resolve_groups_log(None),
            PathBuf::from(id_groups::groups::log::FILE_NAME)
        );
    }

    #[test]
    fn unterminated_final_entry_is_dropped_even_when_it_parses() {
        let dir = TempDir::new().unwrap();
        let path = log_with_retained_grant(&dir);
        // Strip only the trailing newline: the final (member-remove)
        // event becomes an unterminated torn entry. `GroupLog::open`
        // drops it, so the page must too -- OWNER stays a member.
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.pop(), Some(b'\n'));
        std::fs::write(&path, &bytes).unwrap();

        let projection = read_projection(&path).unwrap();
        let (_, view) = projection.groups().next().unwrap();
        assert!(view.members.contains_key(&OWNER));
    }

    #[test]
    fn torn_multibyte_tail_is_dropped_not_an_error() {
        let dir = TempDir::new().unwrap();
        let path = log_with_retained_grant(&dir);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&[0xE2, 0x82]);
        std::fs::write(&path, &bytes).unwrap();

        let projection = read_projection(&path).unwrap();
        assert_eq!(projection.groups().count(), 1);
    }

    #[test]
    fn pending_friendship_renders_pending_not_dissolved() {
        use id_events::groups::{Pair, friendship_id};

        let dir = TempDir::new().unwrap();
        let path = dir.path().join(id_groups::groups::log::FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        // A friendship's create must name the pair-derived group id, or
        // the fold refuses it (see id_events::groups::tests::request).
        let pair = Pair::of(OWNER, ANN).unwrap();
        let friendship = friendship_id(&pair, 0);
        log.record(
            OWNER,
            Some(friendship),
            Body::Create(Create {
                template: Template::Friendship,
                policy: Policy::friendship(),
                members: vec![(ANN, Role::Manage)],
                admins_on_create: false,
            }),
        )
        .unwrap();

        let projection = read_projection(&path).unwrap();
        let html = render_groups(&projection);
        assert!(html.contains("Status: pending"), "html: {html}");
        assert!(!html.contains("Status: dissolved"), "html: {html}");
    }

    #[test]
    fn corrupt_complete_line_is_an_error_not_a_silent_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(id_groups::groups::log::FILE_NAME);
        std::fs::write(&path, "not json\n").unwrap();
        assert!(read_projection(&path).is_err());
    }
}
