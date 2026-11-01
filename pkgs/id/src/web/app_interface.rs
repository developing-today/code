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
/// Returns [`WebInterfaceError`] when the TOML is malformed or the
/// mode is neither `adapter` nor `direct`.
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
    Ok(Some(WebInterface {
        mode,
        route: section.route.unwrap_or_else(|| "/world".to_owned()),
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
}
