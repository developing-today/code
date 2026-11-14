//! Native reference shell for `id` — F13.6, design #788
//! (`doc/2026-11-01_design_id788.md`).
//!
//! This crate is the Bevy reference client for the F13.1 shell contract
//! (`pkgs/id/contract/shell-contract.v1.json`). It is a client only: it
//! connects to a world host over the existing session protocol, renders
//! the host's `view` frames, and emits only the client frames the
//! contract already names (`input`, `paste`, `resize`, `chat`). It adds
//! no frame kinds, needs no host change, and — per the design's WIT
//! decision — no shell frame depends on the guest ABI (F12.8/#1065 is a
//! separate layer a shell never sees).
//!
//! Step 1 (this code) lands the isolated app package, the identity /
//! connection bootstrap, the contract frame builders, and the view
//! fallback rule, with a transcript fixture checked against the
//! machine-readable contract. Step 2 adds the window/render plugins and
//! the live contract-parity session harness.

mod session;

pub use session::{ServerUpdate, SessionState, SessionView, join_frame};

use id::world_presentation::WorldSurface;
use serde_json::{Value, json};

/// Shell-contract name this client speaks (F13.1).
pub const CONTRACT_NAME: &str = "shell";
/// Shell-contract major version implemented by this crate.
pub const CONTRACT_VERSION: u64 = 1;

/// Maximum `paste` payload in bytes (contract v1: `paste.text`).
pub const MAX_PASTE_BYTES: usize = 4096;
/// Maximum `resize` width in columns (contract v1: `resize.width`).
pub const MAX_RESIZE_WIDTH: u16 = 500;
/// Maximum `resize` height in rows (contract v1: `resize.height`).
pub const MAX_RESIZE_HEIGHT: u16 = 200;

/// Why a client frame could not be built within the contract bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Paste text exceeds the contract's per-frame byte bound.
    PasteTooLarge {
        /// Actual UTF-8 byte length of the rejected text.
        bytes: usize,
    },
    /// Resize dimensions fall outside the contract's bounds.
    ResizeOutOfRange {
        /// Requested width in columns.
        width: u16,
        /// Requested height in rows.
        height: u16,
    },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PasteTooLarge { bytes } => {
                write!(
                    f,
                    "paste of {bytes} bytes exceeds the {MAX_PASTE_BYTES}-byte contract bound"
                )
            }
            Self::ResizeOutOfRange { width, height } => {
                write!(
                    f,
                    "resize {width}x{height} is outside the contract bounds (width 1..={MAX_RESIZE_WIDTH}, height 1..={MAX_RESIZE_HEIGHT})"
                )
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Connection configuration for one shell session.
///
/// The client identity is the CLI's own: the keypair lives in
/// [`id::CLIENT_KEY_FILE`] and is loaded (or created) through
/// [`id::load_or_create_keypair`], so the native shell never mints a
/// second identity or copies CLI key policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellConfig {
    /// World (routing) name to join.
    pub world: String,
    /// Client key file name, relative to the working directory.
    pub key_file: String,
}

impl ShellConfig {
    /// Configuration for `world` using the standard client key file.
    #[must_use]
    pub fn new(world: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            key_file: id::CLIENT_KEY_FILE.to_string(),
        }
    }
}

/// Load (or create) this client's keypair via the shared `id` helper.
///
/// # Errors
/// Propagates whatever [`id::load_or_create_keypair`] returns when the
/// key file cannot be read or created.
pub async fn load_client_key(config: &ShellConfig) -> anyhow::Result<iroh::SecretKey> {
    id::load_or_create_keypair(&config.key_file).await
}

/// Build the contract `input` frame for raw key bytes.
///
/// Bytes travel hex-encoded in `data_hex`, exactly as the terminal and
/// web shells send them.
#[must_use]
pub fn input_frame(bytes: &[u8]) -> Value {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    json!({ "type": "input", "data_hex": hex })
}

/// Build the contract `paste` frame for clipboard text.
///
/// # Errors
/// [`FrameError::PasteTooLarge`] when `text` exceeds
/// [`MAX_PASTE_BYTES`] UTF-8 bytes; the contract delivers a paste as
/// one bounded durable event, never a stream of input frames.
pub fn paste_frame(text: &str) -> Result<Value, FrameError> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(FrameError::PasteTooLarge { bytes: text.len() });
    }
    Ok(json!({ "type": "paste", "text": text }))
}

/// Build the contract `resize` frame (ephemeral; never journaled).
///
/// # Errors
/// [`FrameError::ResizeOutOfRange`] when a dimension is zero or above
/// the contract maximum.
pub fn resize_frame(width: u16, height: u16) -> Result<Value, FrameError> {
    if width == 0 || width > MAX_RESIZE_WIDTH || height == 0 || height > MAX_RESIZE_HEIGHT {
        return Err(FrameError::ResizeOutOfRange { width, height });
    }
    Ok(json!({ "type": "resize", "width": width, "height": height }))
}

/// Build the contract `chat` frame.
#[must_use]
pub fn chat_frame(text: &str) -> Value {
    json!({ "type": "chat", "text": text })
}

/// Decide what the presenter shows for one host `view` frame.
///
/// The contract's fallback rule: a shell that has a validated typed
/// `surface` renders it; otherwise it renders the legacy `data` text.
/// Here the surface's own deterministic text projection stands in for
/// the rendered cells until step 2's renderer consumes the surface
/// directly — the projection is escape-free and never interprets
/// guest-provided markup.
#[must_use]
pub fn present(data: &str, surface: Option<&WorldSurface>) -> String {
    match surface {
        Some(surface) => surface.text_projection(),
        None => data.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id::world_presentation::parse_surface;

    #[test]
    fn input_frame_hex_encodes_bytes() {
        assert_eq!(
            input_frame(b"a\x1b"),
            json!({ "type": "input", "data_hex": "611b" })
        );
        assert_eq!(input_frame(b""), json!({ "type": "input", "data_hex": "" }));
    }

    #[test]
    fn paste_frame_enforces_contract_bound() {
        assert_eq!(
            paste_frame("hello").unwrap(),
            json!({ "type": "paste", "text": "hello" })
        );
        let at_bound = "x".repeat(MAX_PASTE_BYTES);
        assert!(paste_frame(&at_bound).is_ok());
        let over = "x".repeat(MAX_PASTE_BYTES + 1);
        assert_eq!(
            paste_frame(&over),
            Err(FrameError::PasteTooLarge {
                bytes: MAX_PASTE_BYTES + 1
            })
        );
    }

    #[test]
    fn resize_frame_enforces_contract_bounds() {
        assert_eq!(
            resize_frame(80, 24).unwrap(),
            json!({ "type": "resize", "width": 80, "height": 24 })
        );
        assert!(resize_frame(0, 24).is_err());
        assert!(resize_frame(80, 0).is_err());
        assert!(resize_frame(MAX_RESIZE_WIDTH + 1, 24).is_err());
        assert!(resize_frame(80, MAX_RESIZE_HEIGHT + 1).is_err());
        assert!(resize_frame(MAX_RESIZE_WIDTH, MAX_RESIZE_HEIGHT).is_ok());
    }

    #[test]
    fn present_falls_back_to_data_without_surface() {
        assert_eq!(present("legacy text", None), "legacy text");
    }

    #[test]
    fn present_uses_text_surface_projection() {
        let surface = parse_surface(r#"{"kind":"text","version":1,"lines":["line one"]}"#)
            .unwrap()
            .expect("text surface");
        assert_eq!(present("ignored data", Some(&surface)), "line one");
    }

    #[test]
    fn present_uses_screen_surface_projection() {
        let surface =
            parse_surface(r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["hi"]}"#)
                .unwrap()
                .expect("screen surface");
        assert_eq!(present("ignored data", Some(&surface)), "hi");
    }

    #[test]
    fn config_uses_cli_client_key_file() {
        let config = ShellConfig::new("some-world");
        assert_eq!(config.key_file, id::CLIENT_KEY_FILE);
        assert_eq!(config.world, "some-world");
    }
}
