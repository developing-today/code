//! Android remote-drive transport bridge for issue #1831.
//!
//! The bridge speaks the existing `/id-world/1` shell session protocol and
//! stays a client: it never starts a world host or listener.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

/// The newest host view received by the Android remote-drive client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RemoteView {
    /// Durable world sequence reflected by this view.
    pub sequence: u64,
    /// Plain-text screen projection supplied by the host.
    pub data: String,
    /// Optional versioned screen or presentation surface from the host.
    pub surface: Option<Value>,
}

/// Parse and validate one server frame, returning a view when it carries one.
///
/// The shell contract is the oracle for frame kind and shape. Non-view
/// session frames are ignored; malformed frames are errors.
pub fn parse_view_frame(frame: &Value) -> Result<Option<RemoteView>> {
    let _ = frame;
    Ok(None)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use serde_json::json;

    use super::{RemoteView, parse_view_frame};

    #[test]
    fn shell_contract_view_frame_yields_the_host_text_projection() {
        let frame = json!({
            "type": "view",
            "sequence": 7,
            "view_revision": 1,
            "data": "tic-tac-toe: X to move",
        });
        let contract = id_worlds::shell_contract::ShellContract::embedded()
            .expect("embedded shell contract is valid");
        assert_eq!(
            contract
                .check_server_frame(&frame)
                .expect("view frame conforms to the shell contract"),
            "view"
        );

        assert_eq!(
            parse_view_frame(&frame).expect("valid host view parses"),
            Some(RemoteView {
                sequence: 7,
                data: "tic-tac-toe: X to move".to_owned(),
                surface: None,
            })
        );
    }
}
