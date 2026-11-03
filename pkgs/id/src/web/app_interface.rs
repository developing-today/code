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
    /// would panic router construction with a duplicate route.
    RouteReserved(String),
    /// `socket` collides with a host-registered route, so binding it
    /// would panic router construction with a duplicate route.
    SocketReserved(String),
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
    "/edit/{*name}",
    "/hash/{hash}",
    "/view/{*name}",
    "/blob/{hash}",
    "/api/files",
    "/api/home/feed",
    "/api/peers",
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
/// start with `/`, both sides of the binding name the same path, or
/// either side collides with a host-registered route (any of which
/// would panic router construction, #4779).
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
    Ok(Some(WebInterface {
        mode,
        route,
        socket: section.socket,
    }))
}

#[cfg(test)]
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
        for reserved in ["/", "/files", "/settings", "/peers"] {
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
