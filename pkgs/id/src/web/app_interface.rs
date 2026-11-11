//! The `[interfaces.web]` section of an app's `app.toml` (#2026 §2,
//! #2325): how a packaged app binds the host's generic web surfaces.
//!
//! In adapter mode (§4.1) the app ships no web code of its own; the
//! host's world page and websocket bridge serve it, parameterised by
//! this section — `route` is the page route, `socket` the
//! live-channel route. In direct mode (§4.2) the app ships its own
//! frontend; parsing accepts the declaration, but the adapter
//! binding in `routes` / `world_ws` applies to adapter mode only.
//!
//! This is the binding layer only: discovering app folders and
//! reading their `app.toml` files is the host-discovery work (#2324).

use std::collections::HashSet;

use serde::Deserialize;

/// How an app's web interface is rendered (#2026 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebMode {
    /// The host's generic surfaces render the app (§4.1).
    Adapter,
    /// The app ships its own frontend (§4.2).
    Direct,
}

/// A parsed `[interfaces.web]` section: the binding for one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebInterface {
    /// Adapter or direct.
    pub mode: WebMode,
    /// Page route when the app is active.
    pub route: String,
    /// Live-channel (websocket) route, if the app has one.
    pub socket: Option<String>,
}

impl WebInterface {
    /// The binding the built-in world app has always used: adapter
    /// mode, page at `/world`, live channel at `/ws/world`. Routers
    /// built with this interface are byte-identical to the routers
    /// built before parameterisation existed.
    #[must_use]
    pub fn world_app() -> Self {
        Self {
            mode: WebMode::Adapter,
            route: "/world".to_owned(),
            socket: Some("/ws/world".to_owned()),
        }
    }

    /// Whether the adapter binding applies to this interface.
    #[must_use]
    pub fn is_adapter(&self) -> bool {
        self.mode == WebMode::Adapter
    }
}

#[derive(Deserialize)]
struct AppToml {
    interfaces: Option<Interfaces>,
}

#[derive(Deserialize)]
struct Interfaces {
    web: Option<WebSection>,
}

#[derive(Deserialize)]
struct WebSection {
    mode: Option<String>,
    route: Option<String>,
    socket: Option<String>,
}

/// Why an `[interfaces.web]` section could not be read.
#[derive(Debug)]
pub enum WebInterfaceError {
    /// The TOML did not parse as an app manifest.
    Toml(toml::de::Error),
    /// `mode` named something other than `adapter` or `direct`.
    UnknownMode(String),
    /// `route` does not start with `/` (axum panics on such paths).
    RouteMustStartWithSlash(String),
    /// `socket` does not start with `/` (axum panics on such paths).
    SocketMustStartWithSlash(String),
    /// `route` collides with a host-registered route, so binding it
    /// would panic router construction with a duplicate route. This
    /// covers exact hits and structural (renamed-capture or wildcard)
    /// overlaps with a host capture route (#4795).
    RouteReserved(String),
    /// `socket` collides with a host-registered route, so binding it
    /// would panic router construction with a duplicate route
    /// (exact or structural, #4795).
    SocketReserved(String),
    /// `route` is not a valid axum path pattern — unbalanced braces,
    /// an empty or invalid capture name, a wildcard outside the final
    /// segment, or a repeated capture name — so binding it would panic
    /// router construction (#4795).
    RouteInvalid(String),
    /// `socket` is not a valid axum path pattern (#4795).
    SocketInvalid(String),
    /// `route` and `socket` name the same path, so binding both would
    /// panic router construction with a duplicate route.
    RouteSocketOverlap(String),
}

/// Paths the host always registers in `create_router_inner` and the
/// shared world HTTP surface merged from `world_http_routes` (page
/// routes, `/api/*`, `/ws/tags`, `/ws/collab/{doc_id}`, the explore
/// and world-bridge routes, and the asset/blob catch-alls). A
/// packaged app binding that reuses any of these would panic router
/// construction — axum rejects duplicate/conflicting routes — so
/// the parser rejects them instead (#4779). Keep this list mirrored
/// with `routes.rs` (`create_router_inner`) and `world_ws.rs`
/// (`world_http_routes`).
pub(crate) const HOST_RESERVED_ROUTES: &[&str] = &[
    "/",
    "/files",
    "/settings",
    "/peers",
    "/blocks",
    "/groups",
    "/edit/{*name}",
    "/hash/{hash}",
    "/view/{*name}",
    "/blob/{hash}",
    "/api/files",
    "/api/home/feed",
    "/api/peers",
    "/api/blocks",
    "/api/blocks/remove",
    "/api/save",
    "/api/new",
    "/api/rename",
    "/api/copy",
    "/api/download",
    "/api/delete",
    "/api/restore",
    "/api/hard-delete",
    "/api/upload",
    "/api/images",
    "/api/tags",
    "/api/tags/search",
    "/api/identity/register",
    "/api/identity/me",
    "/api/identity/name",
    "/ws/collab/{doc_id}",
    "/ws/tags",
    "/assets/{*path}",
    "/explore",
    "/explore/login",
    "/explore/act",
    "/invite",
    "/api/world/invite",
    "/api/world/host-view",
    "/api/world/metrics",
    "/api/world/directory",
];

/// Whether `path` collides with the host's own route table: an exact
/// hit on [`HOST_RESERVED_ROUTES`], or anything on the shared
/// `/api/world/*` surface (which grows with the world bridge, so it
/// is guarded by prefix rather than enumerated).
fn is_host_reserved(path: &str) -> bool {
    path == "/api/world" || path.starts_with("/api/world/") || HOST_RESERVED_ROUTES.contains(&path)
}

/// One segment of an axum path pattern. The capture name is kept so the
/// parser can reject repeats; structural comparison ignores the names —
/// a renamed capture still collides (#4795).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSegment {
    /// A literal segment (`"edit"`, `""` from `//`, `"bar:baz"`, ...).
    /// Segments starting with `:` or `*` are rejected by
    /// [`parse_path_shape`]: axum panics on the old `:capture` / `*glob`
    /// syntax and only accepts `{capture}` / `{*wildcard}`.
    Static(String),
    /// A single-segment capture (`{name}`).
    Param(String),
    /// A catch-all capture (`{*name}`), valid only as the final segment.
    Wildcard(String),
}

/// Split `path` (which must start with `/`) into its axum path shape,
/// rejecting patterns axum itself would panic on: unbalanced braces, an
/// empty or invalid capture name, a segment starting with the old
/// `:capture` / `*glob` syntax, a wildcard outside the final segment,
/// or a capture name used twice (#4795).
fn parse_path_shape(path: &str) -> Result<Vec<PathSegment>, &'static str> {
    let inner = path.strip_prefix('/').unwrap_or(path);
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    let mut names = HashSet::new();
    for raw in inner.split('/') {
        if raw.contains('{') || raw.contains('}') {
            let Some(body) = raw
                .strip_prefix('{')
                .and_then(|stripped| stripped.strip_suffix('}'))
            else {
                return Err("unbalanced braces");
            };
            if body.contains('{') || body.contains('}') {
                return Err("unbalanced braces");
            }
            let (name, is_wildcard) = match body.strip_prefix('*') {
                Some(name) => (name, true),
                None => (body, false),
            };
            if name.is_empty() {
                return Err("empty capture name");
            }
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err("invalid capture name");
            }
            if !names.insert(name.to_owned()) {
                return Err("repeated capture name");
            }
            segments.push(if is_wildcard {
                PathSegment::Wildcard(name.to_owned())
            } else {
                PathSegment::Param(name.to_owned())
            });
        } else if raw.starts_with(':') || raw.starts_with('*') {
            // Axum 0.8 only accepts brace captures: a leading colon
            // or asterisk panics router construction, so those
            // segments are invalid too (#4795).
            return Err("must not start with ':' or '*'");
        } else {
            segments.push(PathSegment::Static(raw.to_owned()));
        }
    }
    if segments
        .iter()
        .position(|segment| matches!(segment, PathSegment::Wildcard(_)))
        .is_some_and(|index| index + 1 != segments.len())
    {
        return Err("wildcard capture must be the final segment");
    }
    Ok(segments)
}

/// The host's route table as path shapes: every entry of
/// [`HOST_RESERVED_ROUTES`] plus the guarded `/api/world` prefix root
/// (#4795).
fn host_shapes() -> Vec<Vec<PathSegment>> {
    let mut shapes = Vec::with_capacity(HOST_RESERVED_ROUTES.len() + 1);
    for route in HOST_RESERVED_ROUTES {
        if let Ok(shape) = parse_path_shape(route) {
            shapes.push(shape);
        }
    }
    if let Ok(shape) = parse_path_shape("/api/world") {
        shapes.push(shape);
    }
    shapes
}

/// Whether `candidate` overlaps `host` the way axum (via `matchit`
/// 0.8) panics on at router construction: two captures at the same
/// position — params or wildcards, whatever their names — while a
/// static sibling or a pure prefix on either side coexists and returns
/// `false` (#4795). Verified against a router built from the full host
/// table: wildcards swallow no static route, but capture-vs-capture at
/// one level panics in either order.
fn shapes_conflict(candidate: &[PathSegment], host: &[PathSegment]) -> bool {
    let mut index = 0;
    loop {
        match (candidate.get(index), host.get(index)) {
            (Some(candidate_segment), Some(host_segment)) => {
                match (candidate_segment, host_segment) {
                    (PathSegment::Static(a), PathSegment::Static(b)) => {
                        if a != b {
                            return false;
                        }
                    }
                    (PathSegment::Static(_), _) | (_, PathSegment::Static(_)) => {}
                    _ => return true,
                }
            }
            (Some(_), None) | (None, Some(_)) | (None, None) => return false,
        }
        index += 1;
    }
}

/// Whether `candidate` structurally overlaps any host route: the same
/// capture shape under different capture names (#4795). A lone root
/// catch-all (`/{*name}`) is also rejected: axum registers it without
/// panicking, but it would claim every otherwise-unmatched request, so
/// no packaged app may bind it. Callers validate syntax first.
fn shape_conflicts_with_host(candidate: &[PathSegment]) -> bool {
    if matches!(candidate, [PathSegment::Wildcard(_)]) {
        return true;
    }
    host_shapes()
        .iter()
        .any(|host| shapes_conflict(candidate, host))
}

impl std::fmt::Display for WebInterfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toml(err) => write!(f, "app.toml did not parse: {err}"),
            Self::UnknownMode(mode) => {
                write!(
                    f,
                    "[interfaces.web] mode {mode:?} is not \"adapter\" or \"direct\""
                )
            }
            Self::RouteMustStartWithSlash(route) => {
                write!(f, "[interfaces.web] route {route:?} must start with '/'")
            }
            Self::SocketMustStartWithSlash(socket) => {
                write!(f, "[interfaces.web] socket {socket:?} must start with '/'")
            }
            Self::RouteReserved(route) => {
                write!(
                    f,
                    "[interfaces.web] route {route:?} collides with a host-registered route"
                )
            }
            Self::SocketReserved(socket) => {
                write!(
                    f,
                    "[interfaces.web] socket {socket:?} collides with a host-registered route"
                )
            }
            Self::RouteInvalid(route) => {
                write!(
                    f,
                    "[interfaces.web] route {route:?} is not a valid axum path pattern"
                )
            }
            Self::SocketInvalid(socket) => {
                write!(
                    f,
                    "[interfaces.web] socket {socket:?} is not a valid axum path pattern"
                )
            }
            Self::RouteSocketOverlap(path) => {
                write!(
                    f,
                    "[interfaces.web] route and socket both bind {path:?}; they must differ"
                )
            }
        }
    }
}

impl std::error::Error for WebInterfaceError {}

impl From<toml::de::Error> for WebInterfaceError {
    fn from(err: toml::de::Error) -> Self {
        Self::Toml(err)
    }
}

/// Reads the `[interfaces.web]` section from an `app.toml` document.
///
/// Returns `Ok(None)` when the document declares no web interface
/// (a pure CLI app is valid, #2026 §2). Defaults inside a present
/// section: `mode` defaults to `adapter`, `route` to `/world`;
/// `socket` has no default — a section without one binds no live
/// channel.
///
/// # Errors
///
/// Returns [`WebInterfaceError`] when the TOML is malformed, the mode
/// is neither `adapter` nor `direct`, a route/socket value does not
/// start with `/`, is not a valid axum path pattern, both sides of the
/// binding name the same (or pattern-conflicting) path, or either side
/// collides — exactly or structurally — with a host-registered route
/// (any of which would panic router construction, #4779 and #4795).
pub fn web_interface_from_app_toml(
    source: &str,
) -> Result<Option<WebInterface>, WebInterfaceError> {
    let parsed: AppToml = toml::from_str(source)?;
    let Some(section) = parsed.interfaces.and_then(|i| i.web) else {
        return Ok(None);
    };
    let mode = match section.mode.as_deref() {
        None | Some("adapter") => WebMode::Adapter,
        Some("direct") => WebMode::Direct,
        Some(other) => return Err(WebInterfaceError::UnknownMode(other.to_owned())),
    };
    let route = section.route.unwrap_or_else(|| "/world".to_owned());
    if !route.starts_with('/') {
        return Err(WebInterfaceError::RouteMustStartWithSlash(route));
    }
    if let Some(ref socket) = section.socket
        && !socket.starts_with('/')
    {
        return Err(WebInterfaceError::SocketMustStartWithSlash(socket.clone()));
    }
    let route_shape = match parse_path_shape(&route) {
        Ok(shape) => shape,
        Err(_) => return Err(WebInterfaceError::RouteInvalid(route)),
    };
    let socket_shape = match section.socket {
        Some(ref socket) => match parse_path_shape(socket) {
            Ok(shape) => Some(shape),
            Err(_) => {
                return Err(WebInterfaceError::SocketInvalid(socket.clone()));
            }
        },
        None => None,
    };
    if let Some(ref socket) = section.socket
        && *socket == route
    {
        return Err(WebInterfaceError::RouteSocketOverlap(route.clone()));
    }
    if is_host_reserved(&route) {
        return Err(WebInterfaceError::RouteReserved(route));
    }
    if let Some(ref socket) = section.socket
        && is_host_reserved(socket)
    {
        return Err(WebInterfaceError::SocketReserved(socket.clone()));
    }
    if shape_conflicts_with_host(&route_shape) {
        return Err(WebInterfaceError::RouteReserved(route));
    }
    if let Some(ref socket_shape) = socket_shape
        && shape_conflicts_with_host(socket_shape)
    {
        let socket = section.socket.clone().unwrap_or_default();
        return Err(WebInterfaceError::SocketReserved(socket));
    }
    if let Some(ref socket_shape) = socket_shape
        && shapes_conflict(&route_shape, socket_shape)
    {
        return Err(WebInterfaceError::RouteSocketOverlap(route.clone()));
    }
    Ok(Some(WebInterface {
        mode,
        route,
        socket: section.socket,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_adapter_binding_with_custom_routes() {
        let source = r#"
id = "arena"

[interfaces.web]
mode = "adapter"
route = "/arena"
socket = "/ws/arena"
"#;
        let iface = web_interface_from_app_toml(source)
            .expect("parses")
            .expect("web section present");
        assert_eq!(iface.mode, WebMode::Adapter);
        assert_eq!(iface.route, "/arena");
        assert_eq!(iface.socket.as_deref(), Some("/ws/arena"));
        assert!(iface.is_adapter());
    }

    #[test]
    fn defaults_apply_inside_a_present_section() {
        let iface = web_interface_from_app_toml("[interfaces.web]\n")
            .expect("parses")
            .expect("web section present");
        assert_eq!(
            iface,
            WebInterface {
                mode: WebMode::Adapter,
                route: "/world".to_owned(),
                socket: None,
            }
        );
    }

    #[test]
    fn no_web_section_is_none_not_an_error() {
        let source = "id = \"cli-only\"\n\n[interfaces.cli]\ncommands = [\"world\"]\n";
        assert_eq!(web_interface_from_app_toml(source).expect("parses"), None);
    }

    #[test]
    fn direct_mode_parses_and_is_not_adapter() {
        let source = "[interfaces.web]\nmode = \"direct\"\nroute = \"/app\"\n";
        let iface = web_interface_from_app_toml(source)
            .expect("parses")
            .expect("web section present");
        assert_eq!(iface.mode, WebMode::Direct);
        assert!(!iface.is_adapter());
    }

    #[test]
    fn unknown_mode_is_an_error() {
        let source = "[interfaces.web]\nmode = \"telepathic\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::UnknownMode(_))
        ));
    }

    #[test]
    fn route_without_a_leading_slash_is_rejected() {
        // Packet case 1: `route = "arena"` parsed Ok and panicked in
        // `create_router_for_app` (routes.rs:322).
        let source = "[interfaces.web]\nroute = \"arena\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::RouteMustStartWithSlash(value))
                if value == "arena"
        ));
    }

    #[test]
    fn host_page_routes_are_reserved() {
        // Packet cases 2-3: `route = "/"` and `route = "/files"` parsed
        // Ok and panicked on the duplicate host route (routes.rs:322).
        for reserved in ["/", "/files", "/settings", "/peers", "/groups"] {
            let source = format!("[interfaces.web]\nroute = {reserved:?}\n");
            assert!(
                matches!(
                    web_interface_from_app_toml(&source),
                    Err(WebInterfaceError::RouteReserved(_))
                ),
                "{reserved} must be rejected as host-reserved"
            );
        }
    }

    #[test]
    fn socket_without_a_leading_slash_is_rejected() {
        // Packet case 4: `socket = "ws/arena"` parsed Ok and panicked
        // in `world_routes_at` (world_ws.rs:60).
        let source = "[interfaces.web]\nroute = \"/arena\"\nsocket = \"ws/arena\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::SocketMustStartWithSlash(value))
                if value == "ws/arena"
        ));
    }

    #[test]
    fn host_socket_routes_are_reserved() {
        // Packet case 5: `socket = "/ws/tags"` parsed Ok and panicked
        // on the duplicate host route (routes.rs:382 via
        // world_ws.rs:60).
        let source = "[interfaces.web]\nroute = \"/arena\"\nsocket = \"/ws/tags\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::SocketReserved(value)) if value == "/ws/tags"
        ));
    }

    #[test]
    fn world_api_surface_is_reserved_for_both_sides() {
        let source = "[interfaces.web]\nroute = \"/api/world/invite\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::RouteReserved(_))
        ));
        let source = "[interfaces.web]\nroute = \"/arena\"\nsocket = \"/api/world/metrics\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::SocketReserved(_))
        ));
    }

    #[test]
    fn route_and_socket_naming_the_same_path_is_rejected() {
        let source = "[interfaces.web]\nroute = \"/arena\"\nsocket = \"/arena\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::RouteSocketOverlap(value)) if value == "/arena"
        ));
    }

    #[test]
    fn invalid_capture_syntax_in_route_is_rejected() {
        // #4795: each of these parsed Ok before and panicked axum in
        // `create_router_inner` (routes.rs:322). The last three use the
        // old `:capture` / `*glob` spellings, which axum 0.8 rejects
        // outright, so a lone colon segment panics too, despite the issue draft
        // listing it as safe.
        for bad in [
            "/foo/{bar",
            "/foo/{}",
            "/foo/{*}",
            "/foo/{bar}/{bar}",
            "/foo/{*name}/bar",
            "/:",
            "/foo/:bar",
            "/foo/*rest",
        ] {
            let source = format!("[interfaces.web]\nroute = {bad:?}\n");
            assert!(
                matches!(
                    web_interface_from_app_toml(&source),
                    Err(WebInterfaceError::RouteInvalid(_))
                ),
                "{bad} must be rejected as invalid axum syntax"
            );
        }
    }

    #[test]
    fn renamed_capture_conflicts_with_host_routes_are_reserved() {
        // #4795: the same shape as a host capture route under a different
        // capture name — invisible to the exact-string reserved list —
        // panics in `create_router_inner`.
        for bad in [
            "/{*path}",
            "/edit/{*other}",
            "/view/{*other}",
            "/hash/{other}",
            "/blob/{other}",
            "/assets/{*other}",
            "/ws/collab/{other}",
        ] {
            let source = format!("[interfaces.web]\nroute = {bad:?}\n");
            assert!(
                matches!(
                    web_interface_from_app_toml(&source),
                    Err(WebInterfaceError::RouteReserved(_))
                ),
                "{bad} must be rejected as colliding with a host route"
            );
        }
    }

    #[test]
    fn invalid_capture_syntax_in_socket_is_rejected() {
        // #4795: the socket side merges via `world_routes_at`
        // (world_ws.rs:60) and had the same gap.
        for bad in ["/ws/{oops", "/ws/{}", "/ws/{*}", "/ws/{a}/{a}"] {
            let source = format!("[interfaces.web]\nroute = \"/arena\"\nsocket = {bad:?}\n");
            assert!(
                matches!(
                    web_interface_from_app_toml(&source),
                    Err(WebInterfaceError::SocketInvalid(_))
                ),
                "{bad} must be rejected as invalid axum syntax"
            );
        }
    }

    #[test]
    fn renamed_capture_conflicts_in_socket_are_reserved() {
        // `/api/{*rest}` is deliberately absent here: a wildcard with no
        // same-level host capture registers fine, so it stays accepted.
        for bad in ["/ws/collab/{other}", "/{*path}", "/edit/{*other}"] {
            let source = format!("[interfaces.web]\nroute = \"/arena\"\nsocket = {bad:?}\n");
            assert!(
                matches!(
                    web_interface_from_app_toml(&source),
                    Err(WebInterfaceError::SocketReserved(_))
                ),
                "{bad} must be rejected as colliding with a host route"
            );
        }
    }

    #[test]
    fn route_and_socket_with_conflicting_patterns_are_rejected() {
        // Same shape under different capture names on the two sides of
        // one binding: both land in one router, so axum would panic.
        let source = "[interfaces.web]\nroute = \"/arena/{name}\"\nsocket = \"/arena/{other}\"\n";
        assert!(matches!(
            web_interface_from_app_toml(source),
            Err(WebInterfaceError::RouteSocketOverlap(value)) if value == "/arena/{name}"
        ));
    }

    #[test]
    fn non_conflicting_capture_shapes_still_parse() {
        // #4795: these were verified not to panic against the full host
        // table (page routes plus the merged world HTTP surface), so the
        // parser must keep accepting them.
        for (route, socket) in [
            ("/api/{name}", None),
            ("/api/{*rest}", None),
            ("/foo//bar", None),
            ("/arena/{*rest}", None),
            ("/arena", Some("/ws/arena")),
        ] {
            let mut source = format!("[interfaces.web]\nroute = {route:?}\n");
            if let Some(socket) = socket {
                source.push_str(&format!("socket = {socket:?}\n"));
            }
            assert!(
                web_interface_from_app_toml(&source).is_ok(),
                "{route} must still parse"
            );
        }
    }

    #[test]
    fn world_app_binding_still_parses() {
        // The built-in world app's binding (`/world` + `/ws/world`)
        // and the shipped tic-tac-toe app must keep working.
        let world = WebInterface::world_app();
        let source = format!(
            "[interfaces.web]\nroute = {:?}\nsocket = {:?}\n",
            world.route,
            world.socket.as_deref().unwrap_or_default()
        );
        let parsed = web_interface_from_app_toml(&source)
            .expect("world binding parses")
            .expect("web section present");
        assert_eq!(parsed, world);
        let source = "[interfaces.web]\nmode = \"adapter\"\nroute = \"/tic-tac-toe\"\n";
        let parsed = web_interface_from_app_toml(source)
            .expect("tic-tac-toe binding parses")
            .expect("web section present");
        assert_eq!(parsed.route, "/tic-tac-toe");
        assert_eq!(parsed.socket, None);
    }
}
