//! The directory in a browser: the views and actions of the JSON, text and SSH
//! transports as HTML forms. The page runs no script.

use std::collections::BTreeMap;

use axum::{
    Form, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};

use crate::directory::{Envelope, Member, Received, Refusal};
use crate::directory_auth::{Caller, SESSION_TTL_MS};
use crate::directory_view::{
    AccountView, DeliveryView, DirectoryAction, DirectoryOutcome, DirectoryView, GroupView,
    account_labels, action_from_fields, level_name,
};
use crate::invite::Invite;
use crate::petname::Label;
use crate::qr;
use crate::world_hub::{ResolveError, WorldHub};

use super::security::secure_attribute;
use super::templates::html_escape;
use super::world_ws::{WorldWebState, refusal_status};

const COOKIE: &str = "id_explore";
const COOKIE_PATH: &str = "/explore";
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
                   frame-ancestors 'none'; base-uri 'none'";
const STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:52rem;margin:2rem auto;\
                     padding:0 1rem;line-height:1.4}\
                     code{word-break:break-all}\
                     form{border:1px solid #888;padding:.5rem .75rem;margin:.75rem 0}\
                     label{display:block;margin:.25rem 0}\
                     .note{background:#eef;padding:.5rem}\
                     .alleged{font-style:italic;color:#8a5a00}\
                     .error{background:#fee;padding:.5rem}";

const MAX_ENVELOPE_BYTES: usize = 64 * 1024;

pub(super) const ENVELOPE_PATH: &str = "/envelope";

pub(super) fn explore_routes() -> Router<WorldWebState> {
    Router::new()
        .route("/explore", get(page_handler))
        .route("/explore/login", post(login_handler))
        .route("/explore/act", post(act_handler))
        .route("/invite", get(invite_handler))
        .route(
            ENVELOPE_PATH,
            post(envelope_handler).layer(DefaultBodyLimit::max(MAX_ENVELOPE_BYTES)),
        )
}

async fn envelope_handler(State(state): State<WorldWebState>, body: Bytes) -> Response {
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let envelope: Envelope = match serde_json::from_slice(&body) {
        Ok(envelope) => envelope,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, format!("not an envelope: {error}")).into_response();
        }
    };
    let Ok(lease) = hub.lease(None, None).await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the server is busy; try again",
        )
            .into_response();
    };
    let action = DirectoryAction::Receive { envelope };
    match lease.service().directory(Caller::default(), action).await {
        Ok(DirectoryOutcome {
            received: Some(Received::Applied(_)),
            ..
        }) => applied_json("applied"),
        Ok(DirectoryOutcome {
            received: Some(Received::Duplicate),
            ..
        }) => applied_json("duplicate"),
        Ok(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "envelope was not received",
        )
            .into_response(),
        Err(error) => (envelope_status(&error), format!("{error:#}")).into_response(),
    }
}

fn applied_json(status: &'static str) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        format!("{{\"status\":\"{status}\"}}"),
    )
        .into_response()
}

/// A refused envelope is a permanent 4xx; any other failure, such as a journal
/// write, is transient and the sender retries.
fn envelope_status(error: &anyhow::Error) -> StatusCode {
    if error.downcast_ref::<Refusal>().is_some() {
        refusal_status(error)
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn page_handler(State(state): State<WorldWebState>, headers: HeaderMap) -> Response {
    let secure = state.cookie_secure;
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let caller = Caller {
        session: session_of(&headers),
        ..Caller::default()
    };
    match run(&hub, caller, DirectoryAction::View).await {
        Ok(outcome) => page(StatusCode::OK, &outcome.view, &[], None),
        Err(error) => failure(&error, secure),
    }
}

async fn login_handler(
    State(state): State<WorldWebState>,
    Form(fields): Form<BTreeMap<String, String>>,
) -> Response {
    let secure = state.cookie_secure;
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let caller = match fields.get("admin_token").filter(|token| !token.is_empty()) {
        Some(token) if hub.admin_ok(token) => Caller {
            admin: true,
            ..Caller::default()
        },
        Some(_) => {
            return failure(
                &Refusal::Unauthenticated("that admin token is not right".to_owned()).into(),
                secure,
            );
        }
        None => Caller::from_secret(fields.get("credential").map_or("", String::as_str)),
    };
    match run(&hub, caller, DirectoryAction::OpenSession).await {
        Ok(outcome) => match outcome.session {
            Some(token) => with_cookie(
                (StatusCode::SEE_OTHER, [(header::LOCATION, COOKIE_PATH)]).into_response(),
                Some(session_cookie(&token, secure)),
            ),
            None => failure(&anyhow::anyhow!("no session was opened"), secure),
        },
        Err(error) => failure(&error, secure),
    }
}

async fn act_handler(
    State(state): State<WorldWebState>,
    headers: HeaderMap,
    Form(fields): Form<BTreeMap<String, String>>,
) -> Response {
    let secure = state.cookie_secure;
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let action = match action_from_fields(&fields) {
        Ok(action) => action,
        Err(error) => return failure(&error, secure),
    };
    let signs_out = matches!(action, DirectoryAction::SignOut);
    let sends = action.sends_envelope();
    let mailed_to = match &action {
        DirectoryAction::AddEmail { address } | DirectoryAction::SignInEmail { address } => {
            Some(address.clone())
        }
        _ => None,
    };
    let credential = fields
        .get("credential")
        .filter(|credential| !credential.is_empty())
        .cloned();
    let caller = Caller {
        session: session_of(&headers),
        credential: credential.filter(|_| sends),
        ..Caller::default()
    };
    let outcome = match run(&hub, caller, action).await {
        Ok(outcome) => outcome,
        Err(error) => return failure(&error, secure),
    };
    let mut notes = Vec::new();
    let mut cookie = outcome
        .session
        .as_deref()
        .map(|token| session_cookie(token, secure));
    if let Some(credential) = &outcome.credential {
        notes.push(format!("Credential, shown once: {credential}"));
        if let Ok(signed_in) = run(
            &hub,
            Caller::from_secret(credential),
            DirectoryAction::OpenSession,
        )
        .await
        {
            cookie = signed_in
                .session
                .as_deref()
                .map(|token| session_cookie(token, secure));
        }
    }
    if signs_out {
        cookie = Some(clear_cookie(secure));
    }
    if let (true, Some(address)) = (outcome.mailed, mailed_to) {
        notes.push(format!("A code was mailed to {address}."));
    }
    page(StatusCode::OK, &outcome.view, &notes, cookie)
}

async fn run(
    hub: &WorldHub,
    caller: Caller,
    action: DirectoryAction,
) -> anyhow::Result<DirectoryOutcome> {
    let lease = hub.lease(None, None).await.map_err(|error| match error {
        ResolveError::Busy | ResolveError::TooManyWorlds => anyhow::Error::from(
            Refusal::RateLimited("the server is busy; try again".to_owned()),
        ),
        _ => anyhow::Error::from(Refusal::NotFound("no such world".to_owned())),
    })?;
    lease.service().directory(caller, action).await
}

fn session_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            pair.trim()
                .split_once('=')
                .filter(|(name, _)| *name == COOKIE)
                .map(|(_, value)| value)
        })
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path={COOKIE_PATH}; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_TTL_MS / 1000,
        secure_attribute(secure)
    )
}

fn clear_cookie(secure: bool) -> String {
    format!(
        "{COOKIE}=; Path={COOKIE_PATH}; HttpOnly; SameSite=Strict; Max-Age=0{}",
        secure_attribute(secure)
    )
}

fn with_cookie(mut response: Response, cookie: Option<String>) -> Response {
    if let Some(value) = cookie.and_then(|cookie| HeaderValue::from_str(&cookie).ok()) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn page(
    status: StatusCode,
    view: &DirectoryView,
    notes: &[String],
    cookie: Option<String>,
) -> Response {
    let notices: String = notes
        .iter()
        .map(|note| format!("<p class=\"note\">{}</p>", html_escape(note)))
        .collect::<Vec<_>>()
        .concat();
    let body = format!(
        "<h1>Directory</h1>{notices}{}{}",
        view_html(view),
        actions_html(&view.viewer)
    );
    respond(status, &body, cookie)
}

fn failure(error: &anyhow::Error, secure: bool) -> Response {
    let status = refusal_status(error);
    let cookie = (status == StatusCode::UNAUTHORIZED).then(|| clear_cookie(secure));
    let body = format!(
        "<h1>Directory</h1><p class=\"error\">{}</p><p><a href=\"{COOKIE_PATH}\">Back to the directory</a></p>",
        html_escape(&format!("{error:#}"))
    );
    respond(status, &body, cookie)
}

fn respond(status: StatusCode, body: &str, cookie: Option<String>) -> Response {
    let response = (
        status,
        [
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(document(body)),
    )
        .into_response();
    with_cookie(response, cookie)
}

fn document(body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Directory</title><style>{STYLE}</style></head><body>{body}</body></html>"
    )
}

fn view_html(view: &DirectoryView) -> String {
    let accounts: String = if view.accounts.is_empty() {
        "<p>(none)</p>".to_owned()
    } else {
        let items: String = view
            .accounts
            .iter()
            .zip(account_labels(&view.accounts))
            .map(|(account, label)| account_html(account, &label))
            .collect();
        format!("<ul>{items}</ul>")
    };
    let groups: String = if view.groups.is_empty() {
        "<p>(none)</p>".to_owned()
    } else {
        let items: String = view.groups.iter().map(group_html).collect();
        format!("<ul>{items}</ul>")
    };
    format!(
        "<p>viewer: <code>{}</code></p><h2>Accounts</h2>{accounts}<h2>Groups</h2>{groups}{}",
        html_escape(&view.viewer),
        deliveries_html(&view.deliveries)
    )
}

fn deliveries_html(deliveries: &[DeliveryView]) -> String {
    if deliveries.is_empty() {
        return String::new();
    }
    let items: String = deliveries
        .iter()
        .map(|delivery| {
            format!(
                "<li>{} to <code>{}</code>: {}</li>",
                html_escape(&delivery.kind),
                html_escape(&delivery.to),
                html_escape(&delivery.status)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!("<h2>Sent to other servers</h2><ul>{items}</ul>")
}

fn account_html(account: &AccountView, label: &Label) -> String {
    let mut lines = vec![format!(
        "<code>{}</code> {}",
        html_escape(&account.id),
        label_html(label)
    )];
    if let Some(verified) = account.verified {
        lines.push(format!("verified: {}", if verified { "yes" } else { "no" }));
    }
    if let Some(keys) = &account.keys {
        lines.push(format!("keys: {}", codes(keys)));
    }
    if let Some(emails) = &account.emails {
        lines.push(format!("emails: {}", codes(emails)));
    }
    if let Some(scopes) = &account.scopes {
        lines.push(format!("scopes: {}", plain(scopes)));
    }
    if let Some(friends) = &account.friends {
        lines.push(format!("friends: {}", codes(friends)));
    }
    if let Some(incoming) = &account.incoming {
        lines.push(format!("requests in: {}", codes(incoming)));
    }
    if let Some(outgoing) = &account.outgoing {
        lines.push(format!("requests out: {}", codes(outgoing)));
    }
    format!("<li>{}</li>", lines.join("<br>"))
}

fn label_html(label: &Label) -> String {
    match label {
        Label::Petname(name) => format!("<strong>{}</strong>", html_escape(name)),
        Label::SharedPetname { name, fingerprint } => format!(
            "<strong>{}</strong> <code>{}</code>",
            html_escape(name),
            html_escape(fingerprint)
        ),
        Label::Alleged { name, fingerprint } => format!(
            "<em class=\"alleged\">{}</em> <span class=\"alleged\">unverified {}</span>",
            html_escape(name),
            html_escape(fingerprint)
        ),
    }
}

async fn invite_handler(Query(fields): Query<BTreeMap<String, String>>) -> Response {
    let field = |name: &str| fields.get(name).map_or("", String::as_str);
    let page = Invite::new(field("key"), field("node"), field("name"))
        .and_then(|invite| invite_page(&invite));
    match page {
        Ok(body) => respond(StatusCode::OK, &body, None),
        Err(error) => respond(
            StatusCode::BAD_REQUEST,
            &format!(
                "<h1>Invite</h1><p class=\"error\">{}</p>",
                html_escape(&format!("{error:#}"))
            ),
            None,
        ),
    }
}

fn invite_page(invite: &Invite) -> anyhow::Result<String> {
    let url = invite.url();
    let name = html_escape(&invite.name);
    let key = html_escape(&invite.key);
    Ok(format!(
        "<h1>Add {name}</h1>\
         <p>Account: <code>{key}</code></p><p>Server: <code>{}</code></p>\
         <div>{}</div>\
         <p>Invite: <code>{}</code></p>\
         <form method=\"post\" action=\"/explore/act\"><h3>Petname</h3>\
         <input type=\"hidden\" name=\"action\" value=\"set_petname\">\
         <input type=\"hidden\" name=\"account\" value=\"{key}\">\
         <label>Petname <input name=\"name\" value=\"{name}\" autocomplete=\"off\"></label>\
         <button>Add {name}</button></form>\
         <p><a href=\"{COOKIE_PATH}\">Sign in to the directory</a> first. The petname is only yours.</p>",
        html_escape(&invite.node),
        qr::svg(&url)?,
        html_escape(&url),
    ))
}

fn group_html(group: &GroupView) -> String {
    let mut lines = vec![format!(
        "<code>{}</code> <strong>{}</strong> {}",
        group.id,
        html_escape(&group.name),
        html_escape(&group.description)
    )];
    lines.push(format!(
        "public: {}",
        if group.public { "yes" } else { "no" }
    ));
    lines.push(format!("scopes: {}", plain(&group.scopes)));
    lines.push(format!("you: {}", group.level.map_or("none", level_name)));
    if let Some(members) = &group.members {
        let listed: Vec<String> = members
            .iter()
            .map(|member| {
                format!(
                    "{} {} ({})",
                    member_label(&member.member),
                    html_escape(&member.name),
                    level_name(member.level)
                )
            })
            .collect();
        lines.push(format!("members: {}", listed.join("; ")));
    }
    format!("<li>{}</li>", lines.join("<br>"))
}

fn member_label(member: &Member) -> String {
    match member {
        Member::Account { id } => format!("account:{}", html_escape(id)),
        Member::Group { id } => format!("group:{id}"),
    }
}

fn codes(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items
            .iter()
            .map(|item| format!("<code>{}</code>", html_escape(item)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn plain(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        html_escape(&items.join(" "))
    }
}

struct ActionForm {
    title: &'static str,
    action: &'static str,
    fields: &'static [(&'static str, &'static str)],
}

const SIGN_OUT: ActionForm = ActionForm {
    title: "Sign out",
    action: "sign_out",
    fields: &[],
};

const ANONYMOUS_FORMS: &[ActionForm] = &[
    ActionForm {
        title: "Sign up",
        action: "sign_up",
        fields: &[("name", "Name")],
    },
    ActionForm {
        title: "Sign in by email",
        action: "sign_in_email",
        fields: &[("address", "Email")],
    },
    ActionForm {
        title: "Confirm sign-in",
        action: "confirm_sign_in",
        fields: &[("address", "Email"), ("code", "Code")],
    },
];

const ACCOUNT_FORMS: &[ActionForm] = &[
    ActionForm {
        title: "Set petname",
        action: "set_petname",
        fields: &[("account", "Account ID"), ("name", "Petname")],
    },
    ActionForm {
        title: "Rename petname",
        action: "rename_petname",
        fields: &[("account", "Account ID"), ("name", "Petname")],
    },
    ActionForm {
        title: "Remove petname",
        action: "remove_petname",
        fields: &[("account", "Account ID")],
    },
    ActionForm {
        title: "Add email",
        action: "add_email",
        fields: &[("address", "Email")],
    },
    ActionForm {
        title: "Confirm email",
        action: "confirm_email",
        fields: &[("address", "Email"), ("code", "Code")],
    },
    ActionForm {
        title: "Remove email",
        action: "remove_email",
        fields: &[("address", "Email")],
    },
    ActionForm {
        title: "Remove key",
        action: "remove_key",
        fields: &[("key", "Key (hex)")],
    },
    ActionForm {
        title: "Request friend",
        action: "request_friend",
        fields: &[
            ("to", "Account ID"),
            ("server", "Their server URL (blank for this server)"),
            ("audience", "Their world ID (with a server URL)"),
            ("credential", "Your credential (with a server URL)"),
        ],
    },
    ActionForm {
        title: "Accept friend",
        action: "accept_friend",
        fields: &[
            ("from", "Account ID"),
            ("server", "Their server URL (blank for this server)"),
            ("audience", "Their world ID (with a server URL)"),
            ("credential", "Your credential (with a server URL)"),
        ],
    },
    ActionForm {
        title: "Remove friend",
        action: "remove_friend",
        fields: &[("other", "Account ID")],
    },
    ActionForm {
        title: "New group",
        action: "create_group",
        fields: &[
            ("name", "Name"),
            ("description", "Description"),
            ("scopes", "Scopes (comma list or -)"),
        ],
    },
    ActionForm {
        title: "Update group",
        action: "update_group",
        fields: &[
            ("group", "Group ID"),
            ("name", "Name"),
            ("description", "Description"),
            ("scopes", "Scopes (comma list or -)"),
        ],
    },
    ActionForm {
        title: "Public group",
        action: "set_public",
        fields: &[("group", "Group ID"), ("public", "Public (yes or no)")],
    },
    ActionForm {
        title: "Delete group",
        action: "delete_group",
        fields: &[("group", "Group ID")],
    },
    ActionForm {
        title: "Group member",
        action: "set_member",
        fields: &[
            ("group", "Group ID"),
            ("member", "account:ID or group:N"),
            (
                "level",
                "Level (access, read, write, manage, moderator, admin, none)",
            ),
        ],
    },
];

const ADMIN_FORMS: &[ActionForm] = &[ActionForm {
    title: "Verify account",
    action: "verify",
    fields: &[("account", "Account ID")],
}];

const RECEIVE_FORM: ActionForm = ActionForm {
    title: "Receive friend envelope",
    action: "receive",
    fields: &[("envelope", "Envelope (JSON)")],
};

fn actions_html(viewer: &str) -> String {
    let mut html = String::from("<h2>Actions</h2>");
    html.push_str(&act_form(&RECEIVE_FORM));
    match viewer {
        "anonymous" => {
            html.push_str(&login_forms());
            html.extend(ANONYMOUS_FORMS.iter().map(act_form));
        }
        "admin" => {
            html.push_str(&act_form(&SIGN_OUT));
            html.extend(ADMIN_FORMS.iter().map(act_form));
        }
        _ => {
            html.push_str(&act_form(&SIGN_OUT));
            html.extend(ACCOUNT_FORMS.iter().map(act_form));
        }
    }
    html
}

fn act_form(form: &ActionForm) -> String {
    let fields: String = form
        .fields
        .iter()
        .map(|(name, label)| {
            format!(
                "<label>{} <input name=\"{}\" autocomplete=\"off\"></label>",
                html_escape(label),
                html_escape(name)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!(
        "<form method=\"post\" action=\"/explore/act\"><h3>{}</h3>\
         <input type=\"hidden\" name=\"action\" value=\"{}\">{fields}<button>Send</button></form>",
        html_escape(form.title),
        html_escape(form.action)
    )
}

fn login_forms() -> String {
    "<form method=\"post\" action=\"/explore/login\"><h3>Sign in with a credential</h3>\
     <label>Credential <input name=\"credential\" autocomplete=\"off\"></label>\
     <button>Sign in</button></form>\
     <form method=\"post\" action=\"/explore/login\"><h3>Admin sign-in</h3>\
     <p>Send the admin token only over HTTPS.</p>\
     <label>Admin token <input name=\"admin_token\" type=\"password\" autocomplete=\"off\"></label>\
     <button>Sign in</button></form>"
        .to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::directory::Directory;
    use crate::directory_view::render_text;
    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn app(admin: Option<&str>) -> Router {
        app_with_cookie_secure(admin, false)
    }

    fn app_with_cookie_secure(admin: Option<&str>, cookie_secure: bool) -> Router {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let hub = WorldHub::single(WorldService::new(world, admin.map(str::to_owned)));
        explore_routes().with_state(WorldWebState {
            hub: Some(hub),
            cookie_secure,
        })
    }

    async fn lobby_with_bo() -> (Router, String) {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, None);
        let bo = service
            .directory(
                Caller::default(),
                DirectoryAction::SignUp {
                    name: "Bo".to_owned(),
                },
            )
            .await
            .unwrap()
            .view
            .viewer;
        let hub = WorldHub::single(service);
        let app = explore_routes().with_state(WorldWebState {
            hub: Some(hub),
            cookie_secure: false,
        });
        (app, bo)
    }

    fn sent_from_home(to: &str, audience: &str, at: u64) -> Envelope {
        let mut home = Directory::new();
        let (credential, _) = home.sign_up("Ann").unwrap();
        home.request_friend(&credential, to, audience, at)
            .unwrap()
            .0
    }

    fn push(envelope: &Envelope) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(ENVELOPE_PATH)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(envelope).unwrap()))
            .unwrap()
    }

    #[tokio::test]
    async fn a_web_token_does_not_block_peer_envelopes_but_guards_other_routes() {
        let (app, bo) = lobby_with_bo().await;
        let sec = std::sync::Arc::new(crate::web::security::WebSecurity {
            token: Some("s3cret".to_owned()),
            ..Default::default()
        });
        let app = app.layer(axum::middleware::from_fn_with_state(
            sec,
            crate::web::security::guard,
        ));

        let envelope = sent_from_home(&bo, "lobby", crate::world::unix_ms());
        let (status, _, body) = send(&app, push(&envelope)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("applied"), "{body}");

        let (status, _, _) = send(&app, get("/explore", None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_pushed_envelope_is_applied_once_and_refusals_are_permanent() {
        let (app, bo) = lobby_with_bo().await;
        let at = crate::world::unix_ms();
        let envelope = sent_from_home(&bo, "lobby", at);

        let (status, _, body) = send(&app, push(&envelope)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("applied"), "{body}");
        let (status, _, body) = send(&app, push(&envelope)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("duplicate"), "{body}");

        let elsewhere = sent_from_home(&bo, "elsewhere", at);
        let (status, _, _) = send(&app, push(&elsewhere)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "wrong audience");

        let mut forged = sent_from_home(&bo, "lobby", at);
        forged.at = at + 1;
        let (status, _, _) = send(&app, push(&forged)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bad signature");

        let stranger = sent_from_home(&"ab".repeat(32), "lobby", at);
        let (status, _, _) = send(&app, push(&stranger)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "unknown account");
    }

    #[tokio::test]
    async fn an_oversized_envelope_is_refused_before_it_is_read() {
        let (app, _) = lobby_with_bo().await;
        let oversized = Request::builder()
            .method("POST")
            .uri("/envelope")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(vec![b' '; MAX_ENVELOPE_BYTES + 1]))
            .unwrap();
        let (status, _, _) = send(&app, oversized).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn only_refusals_are_permanent_and_anything_else_is_retried() {
        assert_eq!(
            envelope_status(&anyhow::anyhow!("journal write failed")),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            envelope_status(&anyhow::Error::from(Refusal::Conflict("stale".to_owned()))),
            StatusCode::CONFLICT
        );
    }

    fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("GET").uri(path);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        builder.body(Body::empty()).unwrap()
    }

    fn post(path: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    fn set_cookie(headers: &HeaderMap) -> Option<String> {
        headers
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::to_owned)
    }

    fn full_set_cookie(headers: &HeaderMap) -> String {
        headers[header::SET_COOKIE].to_str().unwrap().to_owned()
    }

    fn attributes(set_cookie: &str) -> Vec<&str> {
        set_cookie.split(';').skip(1).map(str::trim).collect()
    }

    fn credential_in(body: &str) -> String {
        body.split("Credential, shown once: ")
            .nth(1)
            .and_then(|rest| rest.split(['<', ' ']).next())
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn the_page_is_html_with_a_locked_down_policy_and_no_script() {
        let app = app(None);
        let (status, headers, body) = send(&app, get("/explore", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && !csp.contains("script-src"));
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(body.contains("action=\"/explore/act\"") && body.contains("value=\"sign_up\""));
        assert!(!body.contains("<script"));
    }

    #[tokio::test]
    async fn signing_up_in_the_browser_signs_the_browser_in_and_sign_out_clears_it() {
        let app = app(None);
        let (status, headers, body) =
            send(&app, post("/explore/act", "action=sign_up&name=Cy", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Credential, shown once: acct."), "{body}");
        let cookie = set_cookie(&headers).unwrap();
        assert!(cookie.starts_with("id_explore=sess."), "{cookie}");

        let (_, _, body) = send(&app, get("/explore", Some(&cookie))).await;
        assert!(
            body.contains("Cy") && body.contains("value=\"sign_out\""),
            "{body}"
        );
        assert!(!body.contains("value=\"sign_up\""));

        let (_, headers, body) =
            send(&app, post("/explore/act", "action=sign_out", Some(&cookie))).await;
        assert!(body.contains("value=\"sign_up\""), "{body}");
        assert!(
            headers[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }

    #[tokio::test]
    async fn explorer_cookies_are_http_only_strict_and_not_secure_by_default() {
        let app = app(None);
        let (_, headers, _) =
            send(&app, post("/explore/act", "action=sign_up&name=Cy", None)).await;
        let session = set_cookie(&headers).unwrap();
        let cookie = full_set_cookie(&headers);
        let attrs = attributes(&cookie);
        assert!(attrs.contains(&"HttpOnly") && attrs.contains(&"SameSite=Strict"));
        assert!(!attrs.contains(&"Secure"), "{attrs:?}");

        let (_, headers, _) = send(
            &app,
            post("/explore/act", "action=sign_out", Some(&session)),
        )
        .await;
        let cookie = full_set_cookie(&headers);
        let attrs = attributes(&cookie);
        assert!(attrs.contains(&"HttpOnly") && attrs.contains(&"Max-Age=0"));
        assert!(!attrs.contains(&"Secure"), "{attrs:?}");
    }

    #[tokio::test]
    async fn explorer_cookies_are_secure_when_https_is_terminated_in_front() {
        let app = app_with_cookie_secure(Some("s3cret"), true);
        let (status, headers, _) =
            send(&app, post("/explore/login", "admin_token=s3cret", None)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let cookie = full_set_cookie(&headers);
        let attrs = attributes(&cookie);
        assert!(attrs.contains(&"HttpOnly") && attrs.contains(&"SameSite=Strict"));
        assert!(attrs.contains(&"Secure"), "{attrs:?}");
        let session = set_cookie(&headers).unwrap();

        let (_, headers, _) = send(
            &app,
            post("/explore/act", "action=sign_out", Some(&session)),
        )
        .await;
        assert!(attributes(&full_set_cookie(&headers)).contains(&"Secure"));

        let (status, headers, _) =
            send(&app, post("/explore/login", "admin_token=wrong", None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(attributes(&full_set_cookie(&headers)).contains(&"Secure"));
    }

    #[tokio::test]
    async fn names_and_refusals_are_escaped_before_they_reach_the_page() {
        let app = app(None);
        let (_, _, body) = send(
            &app,
            post(
                "/explore/act",
                "action=sign_up&name=%3Cb%3EBo%3C%2Fb%3E",
                None,
            ),
        )
        .await;
        assert!(
            body.contains("&lt;b&gt;Bo&lt;/b&gt;") && !body.contains("<b>Bo</b>"),
            "{body}"
        );

        let (status, _, body) = send(&app, post("/explore/act", "action=explode", None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("<p class=\"error\">"), "{body}");
    }

    #[tokio::test]
    async fn a_credential_signs_in_and_a_wrong_admin_token_does_not() {
        let app = app(Some("s3cret"));
        let (_, _, body) = send(&app, post("/explore/act", "action=sign_up&name=Di", None)).await;
        let credential = credential_in(&body);

        let (status, headers, _) = send(
            &app,
            post("/explore/login", &format!("credential={credential}"), None),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(headers[header::LOCATION], "/explore");
        assert!(
            set_cookie(&headers)
                .unwrap()
                .starts_with("id_explore=sess.")
        );

        let (status, headers, _) =
            send(&app, post("/explore/login", "admin_token=wrong", None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            headers[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }

    #[tokio::test]
    async fn the_admin_token_opens_an_admin_session_with_the_verify_form() {
        let app = app(Some("s3cret"));
        let (status, headers, _) =
            send(&app, post("/explore/login", "admin_token=s3cret", None)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let cookie = set_cookie(&headers).unwrap();
        let (_, _, body) = send(&app, get("/explore", Some(&cookie))).await;
        assert!(body.contains("viewer: <code>admin</code>"), "{body}");
        assert!(body.contains("value=\"verify\""), "{body}");
    }

    #[tokio::test]
    async fn an_ended_session_is_refused_and_its_cookie_cleared() {
        let app = app(None);
        let (status, headers, _) =
            send(&app, get("/explore", Some("id_explore=sess.00000000"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            headers[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
    }

    #[test]
    fn every_form_builds_the_action_it_names() {
        for form in ANONYMOUS_FORMS
            .iter()
            .chain(ACCOUNT_FORMS)
            .chain(ADMIN_FORMS)
            .chain([&SIGN_OUT, &RECEIVE_FORM])
        {
            let mut fields: BTreeMap<String, String> = BTreeMap::new();
            fields.insert("action".to_owned(), form.action.to_owned());
            for (name, _) in form.fields {
                let value = match *name {
                    "group" => "1",
                    "public" => "yes",
                    "scopes" => "-",
                    "level" => "read",
                    "member" => "account:abc",
                    "server" => "https://example.com",
                    "envelope" => {
                        r#"{"kind":"friend_request","from":"a","to":"b","audience":"c","id":"d","at":1,"signature":"e"}"#
                    }
                    _ => "x",
                };
                fields.insert((*name).to_owned(), value.to_owned());
            }
            assert!(
                action_from_fields(&fields).is_ok(),
                "{} does not build its action",
                form.action
            );
        }
    }

    #[tokio::test]
    async fn html_text_and_json_show_the_same_accounts_and_groups() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let ada = world
            .directory(
                Caller::default(),
                DirectoryAction::SignUp {
                    name: "Ada & <Co>".to_owned(),
                },
            )
            .await
            .unwrap();
        let ada_caller = Caller::from_secret(ada.credential.as_deref().unwrap());
        world
            .directory(
                Caller::default(),
                DirectoryAction::SignUp {
                    name: "<b>Bo</b>".to_owned(),
                },
            )
            .await
            .unwrap();
        world
            .directory(
                ada_caller.clone(),
                DirectoryAction::CreateGroup {
                    name: "Club & <Crew>".to_owned(),
                    description: "members \"only\"".to_owned(),
                    scopes: Vec::new(),
                },
            )
            .await
            .unwrap();
        let view = world
            .directory(ada_caller, DirectoryAction::View)
            .await
            .unwrap()
            .view;
        let html = view_html(&view);
        let text = render_text(&view);
        let json = serde_json::to_string(&view).unwrap();

        assert_eq!(view.accounts.len(), 2);
        assert_eq!(view.groups.len(), 1);
        for account in &view.accounts {
            assert!(
                html.contains(&html_escape(&account.id))
                    && html.contains(&html_escape(&account.name))
            );
            assert!(text.contains(&account.id) && text.contains(&account.name));
            assert!(json.contains(&account.id) && json.contains(&account.name));
        }
        assert!(html.contains("&lt;b&gt;Bo&lt;/b&gt;") && !html.contains("<b>Bo</b>"));
        let group = &view.groups[0];
        assert!(html.contains(&html_escape(&group.name)));
        assert!(text.contains(&group.name) && json.contains(&group.name));
    }

    #[tokio::test]
    async fn the_invite_page_names_the_account_and_carries_its_qr_code() {
        let app = app(None);
        let key = "aa11".repeat(16);
        let node = "bb22".repeat(16);
        let uri = format!("/invite?key={key}&node={node}&name=Ann%20Lee");
        let (status, _, body) = send(
            &app,
            Request::builder().uri(&uri).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<h1>Add Ann Lee</h1>"));
        assert!(body.contains("<svg "));
        assert!(body.contains("value=\"Ann Lee\""));
        let url = Invite::new(&key, &node, "Ann Lee").unwrap().url();
        assert!(body.contains(&html_escape(&url)));
        let bad = format!("/invite?key=zz&node={node}&name=Ann");
        let (status, _, _) = send(
            &app,
            Request::builder().uri(&bad).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
