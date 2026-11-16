//! Native input events mapped to F13.1 contract frames (design #788
//! step 2: "mappings for existing input, paste, and resize").
//!
//! The terminal shell turns keystrokes, clipboard pastes, and terminal
//! resizes into the contract's `input` / `paste` / `resize` frames via
//! the builders in [`crate`]. The native shell's windowing layer
//! produces platform events instead — raw key bytes from the OS, a
//! clipboard string, a window size in physical pixels — so this module
//! is the adapter: every native event becomes exactly the frame the
//! terminal builder would produce for the same logical action, which
//! is what the parity tests assert. No new frame kinds are introduced.

use serde_json::Value;

use crate::{
    FrameError, MAX_RESIZE_HEIGHT, MAX_RESIZE_WIDTH, chat_frame, input_frame, paste_frame,
    resize_frame,
};

/// One input event from the native windowing layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeEvent {
    /// Raw key bytes, already translated from the platform key event
    /// (the same bytes a terminal would send for the key).
    KeyInput(Vec<u8>),
    /// Clipboard text delivered as one paste action.
    Paste(String),
    /// Window resized to this many terminal columns / rows.
    Resize {
        /// New width in columns.
        width: u16,
        /// New height in rows.
        height: u16,
    },
    /// A chat line submitted from the shell's own UI.
    Chat(String),
}

/// Build the contract frame for one native event.
///
/// # Errors
/// Propagates the builders' [`FrameError`] for an oversize paste or an
/// out-of-range resize — the native layer must surface those instead
/// of silently truncating, exactly as the terminal shell does.
pub fn native_event_frame(event: &NativeEvent) -> Result<Value, FrameError> {
    match event {
        NativeEvent::KeyInput(bytes) => Ok(input_frame(bytes)),
        NativeEvent::Paste(text) => paste_frame(text),
        NativeEvent::Resize { width, height } => resize_frame(*width, *height),
        NativeEvent::Chat(text) => Ok(chat_frame(text)),
    }
}

/// Convert a window's pixel size to contract resize dimensions, given
/// the pixel size of one terminal cell in the shell's font.
///
/// The result is clamped into the contract bounds
/// (1..=[`MAX_RESIZE_WIDTH`] × 1..=[`MAX_RESIZE_HEIGHT`]): a native
/// window can be dragged smaller than one cell or larger than the
/// contract's maximum grid, and a resize the host would reject must
/// never be emitted — clamping at the mapping layer is the native
/// counterpart of the terminal's own size limits. A zero cell size
/// (font metrics not ready yet) maps to the minimum 1×1 rather than
/// dividing by zero.
#[must_use]
pub fn resize_dimensions(
    window_width_px: u32,
    window_height_px: u32,
    cell_width_px: u32,
    cell_height_px: u32,
) -> (u16, u16) {
    fn cells(window: u32, cell: u32, max: u16) -> u16 {
        if cell == 0 {
            return 1;
        }
        let n = window / cell;
        u16::try_from(n).unwrap_or(max).clamp(1, max)
    }
    (
        cells(window_width_px, cell_width_px, MAX_RESIZE_WIDTH),
        cells(window_height_px, cell_height_px, MAX_RESIZE_HEIGHT),
    )
}

/// Build the `resize` frame for a window pixel size directly.
///
/// # Errors
/// Never in practice (dimensions are clamped into range first), but
/// the signature keeps the builder's error visible for callers.
pub fn resize_frame_for_window(
    window_width_px: u32,
    window_height_px: u32,
    cell_width_px: u32,
    cell_height_px: u32,
) -> Result<Value, FrameError> {
    let (width, height) = resize_dimensions(
        window_width_px,
        window_height_px,
        cell_width_px,
        cell_height_px,
    );
    resize_frame(width, height)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_frames_match_the_terminal_builders() {
        // Parity oracle: for the same logical action, the native
        // mapping must produce byte-identical frames to the builders
        // the terminal shell uses.
        assert_eq!(
            native_event_frame(&NativeEvent::KeyInput(b"a\x1b".to_vec())).unwrap(),
            input_frame(b"a\x1b")
        );
        assert_eq!(
            native_event_frame(&NativeEvent::Paste("hello".to_string())).unwrap(),
            paste_frame("hello").unwrap()
        );
        assert_eq!(
            native_event_frame(&NativeEvent::Resize {
                width: 80,
                height: 24
            })
            .unwrap(),
            resize_frame(80, 24).unwrap()
        );
        assert_eq!(
            native_event_frame(&NativeEvent::Chat("hi".to_string())).unwrap(),
            chat_frame("hi")
        );
    }

    #[test]
    fn native_mapping_keeps_contract_bounds() {
        let over = "x".repeat(crate::MAX_PASTE_BYTES + 1);
        assert!(matches!(
            native_event_frame(&NativeEvent::Paste(over)),
            Err(FrameError::PasteTooLarge { .. })
        ));
        assert_eq!(
            native_event_frame(&NativeEvent::Resize {
                width: 0,
                height: 24
            }),
            Err(FrameError::ResizeOutOfRange {
                width: 0,
                height: 24
            })
        );
    }

    #[test]
    fn window_pixels_convert_and_clamp_to_contract_cells() {
        assert_eq!(resize_dimensions(800, 480, 10, 20), (80, 24));
        // A window smaller than one cell still yields a legal 1x1.
        assert_eq!(resize_dimensions(3, 3, 10, 20), (1, 1));
        // A huge window clamps to the contract maximum, not beyond.
        assert_eq!(
            resize_dimensions(1_000_000, 1_000_000, 1, 1),
            (MAX_RESIZE_WIDTH, MAX_RESIZE_HEIGHT)
        );
        // Font metrics not ready: no divide-by-zero, minimum grid.
        assert_eq!(resize_dimensions(800, 480, 0, 0), (1, 1));
        assert_eq!(
            resize_frame_for_window(800, 480, 10, 20).unwrap(),
            json!({"type": "resize", "width": 80, "height": 24})
        );
    }
}
