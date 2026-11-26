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

use bevy::input::keyboard::Key;
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

/// Convert a window's logical size to contract resize dimensions,
/// given one cell's logical size (the Bevy window reports logical
/// pixels, and the renderer's cell metrics are logical too).
///
/// Floor division, clamped into the contract bounds exactly like
/// [`resize_dimensions`]. A non-finite or non-positive cell size, or a
/// non-finite window size, maps to the minimum 1 rather than a
/// garbage grid.
#[must_use]
pub fn resize_dimensions_logical(
    window_width: f32,
    window_height: f32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn cells(window: f32, cell: f32, max: u16) -> u16 {
        if !(window.is_finite() && cell.is_finite() && cell > 0.0) {
            return 1;
        }
        // Clamped into 1..=max first, so the cast is exact.
        (window / cell).floor().clamp(1.0, f32::from(max)) as u16
    }
    (
        cells(window_width, cell_width, MAX_RESIZE_WIDTH),
        cells(window_height, cell_height, MAX_RESIZE_HEIGHT),
    )
}

/// The grid size last sent to one session, so the session gets a
/// `resize` frame when its window grid changes (and once when it
/// opens) but never twice for the same size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeTracker {
    sent: Option<(u16, u16)>,
}

impl ResizeTracker {
    /// The resize event the session still needs for the current grid,
    /// or `None` when it already has this size.
    #[must_use]
    pub fn pending(&self, grid: (u16, u16)) -> Option<NativeEvent> {
        (self.sent != Some(grid)).then_some(NativeEvent::Resize {
            width: grid.0,
            height: grid.1,
        })
    }

    /// Record that a resize to `grid` was queued for the session. A
    /// resize that could not be queued is not recorded, so it is
    /// offered again on the next frame.
    pub const fn mark_sent(&mut self, grid: (u16, u16)) {
        self.sent = Some(grid);
    }
}

/// Modifier keys held during one key press, as far as the terminal
/// byte mapping and the paste chord care. Shift is already folded
/// into the produced text by the platform, so the byte mapping
/// ignores it; it only tells the paste chord apart from a plain
/// Ctrl+V or Insert. Super/Meta have no terminal encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    /// A Control key is held.
    pub ctrl: bool,
    /// An Alt key is held.
    pub alt: bool,
    /// A Shift key is held.
    pub shift: bool,
}

/// Whether a key press is the paste chord rather than a key to send.
///
/// The chords are the terminal emulators' own: Ctrl+Shift+V, since a
/// plain Ctrl+V is the literal-next control byte a world may want,
/// and Shift+Insert. Alt with either is not a paste, so the chords
/// never shadow an Alt-prefixed key.
#[must_use]
pub fn is_paste_chord(key: &Key, modifiers: KeyModifiers) -> bool {
    if modifiers.alt || !modifiers.shift {
        return false;
    }
    match key {
        Key::Character(character) => modifiers.ctrl && character.eq_ignore_ascii_case("v"),
        Key::Insert => !modifiers.ctrl,
        _ => false,
    }
}

/// The native event for one clipboard read on the paste chord, or
/// `None` for an empty clipboard (nothing to paste, so nothing is
/// sent). Text over [`crate::MAX_PASTE_BYTES`] is still returned:
/// [`native_event_frame`] refuses it, and the caller surfaces that
/// refusal, because the contract delivers a paste as one bounded
/// event and never as a split or truncated one.
#[must_use]
pub fn paste_event(clipboard_text: &str) -> Option<NativeEvent> {
    (!clipboard_text.is_empty()).then(|| NativeEvent::Paste(clipboard_text.to_string()))
}

/// The control byte a terminal sends for Ctrl plus `c`, if any.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' | 'A'..='Z' => u8::try_from(c.to_ascii_lowercase())
            .ok()
            .map(|letter| letter - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Text the platform produced, if it is safe to forward as typed
/// characters: non-empty and free of control characters (those are
/// produced by this mapping alone, from the logical key).
fn printable(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// Map one key press from the native window to the bytes a terminal
/// sends for the same key (xterm conventions), so a world in a
/// native session sees exactly what it would see over the terminal
/// shell (#15022, session-mode input).
///
/// `key` is the logical key (layout applied) and `text` the text
/// the platform produced for the press, preferred for characters
/// because it carries Shift and composed dead keys. Ctrl turns a
/// character into its control byte; Alt prefixes ESC. Keys with no
/// terminal encoding (modifiers alone, dead keys, media keys) and
/// Ctrl chords with no control byte map to `None` rather than to a
/// guess.
#[must_use]
pub fn key_input_bytes(key: &Key, text: Option<&str>, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    let base: Vec<u8> = match key {
        Key::Enter => b"\r".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
        Key::Space if modifiers.ctrl => vec![0],
        Key::Space => b" ".to_vec(),
        Key::ArrowUp => b"\x1b[A".to_vec(),
        Key::ArrowDown => b"\x1b[B".to_vec(),
        Key::ArrowRight => b"\x1b[C".to_vec(),
        Key::ArrowLeft => b"\x1b[D".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::Insert => b"\x1b[2~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => b"\x1b[15~".to_vec(),
        Key::F6 => b"\x1b[17~".to_vec(),
        Key::F7 => b"\x1b[18~".to_vec(),
        Key::F8 => b"\x1b[19~".to_vec(),
        Key::F9 => b"\x1b[20~".to_vec(),
        Key::F10 => b"\x1b[21~".to_vec(),
        Key::F11 => b"\x1b[23~".to_vec(),
        Key::F12 => b"\x1b[24~".to_vec(),
        Key::Character(character) if modifiers.ctrl => {
            let mut chars = character.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            vec![control_byte(c)?]
        }
        Key::Character(character) => printable(text)
            .or_else(|| printable(Some(character.as_str())))?
            .as_bytes()
            .to_vec(),
        Key::Dead(_) => return None,
        _ if modifiers.ctrl => return None,
        _ => printable(text)?.as_bytes().to_vec(),
    };
    if modifiers.alt {
        let mut prefixed = Vec::with_capacity(base.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend_from_slice(&base);
        return Some(prefixed);
    }
    Some(base)
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
    fn logical_window_sizes_convert_and_clamp_to_contract_cells() {
        // 960x480 logical at 12x24 cells is an 80x20 grid; a partial
        // cell is not counted.
        assert_eq!(
            resize_dimensions_logical(960.0, 480.0, 12.0, 24.0),
            (80, 20)
        );
        assert_eq!(
            resize_dimensions_logical(971.9, 503.9, 12.0, 24.0),
            (80, 20)
        );
        // Smaller than a cell, zero, negative or NaN: the 1x1 minimum.
        assert_eq!(resize_dimensions_logical(5.0, 5.0, 12.0, 24.0), (1, 1));
        assert_eq!(resize_dimensions_logical(0.0, -3.0, 12.0, 24.0), (1, 1));
        assert_eq!(
            resize_dimensions_logical(f32::NAN, f32::INFINITY, 12.0, 24.0),
            (1, 1)
        );
        assert_eq!(resize_dimensions_logical(960.0, 480.0, 0.0, -1.0), (1, 1));
        // A huge window clamps to the contract maximum, not beyond.
        assert_eq!(
            resize_dimensions_logical(1.0e9, 1.0e9, 12.0, 24.0),
            (MAX_RESIZE_WIDTH, MAX_RESIZE_HEIGHT)
        );
        // Every output is a frame the contract builder accepts.
        let (width, height) = resize_dimensions_logical(1.0e9, 0.0, 12.0, 24.0);
        assert!(resize_frame(width, height).is_ok());
    }

    #[test]
    fn a_session_gets_each_grid_size_once() {
        let mut tracker = ResizeTracker::default();
        // A fresh session has no size yet, so the current grid is due.
        assert_eq!(
            tracker.pending((80, 24)),
            Some(NativeEvent::Resize {
                width: 80,
                height: 24
            })
        );
        // Not queued (full channel): still due on the next frame.
        assert!(tracker.pending((80, 24)).is_some());
        tracker.mark_sent((80, 24));
        // Same size again: nothing to send.
        assert_eq!(tracker.pending((80, 24)), None);
        // The window changed: the new size is due, and only it.
        assert_eq!(
            tracker.pending((100, 30)),
            Some(NativeEvent::Resize {
                width: 100,
                height: 30
            })
        );
        tracker.mark_sent((100, 30));
        assert_eq!(tracker.pending((100, 30)), None);
        // Back to an earlier size is a change too.
        assert!(tracker.pending((80, 24)).is_some());
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

    fn character(text: &str) -> Key {
        Key::Character(text.into())
    }

    const PLAIN: KeyModifiers = KeyModifiers {
        ctrl: false,
        alt: false,
        shift: false,
    };
    const CTRL: KeyModifiers = KeyModifiers {
        ctrl: true,
        alt: false,
        shift: false,
    };
    const ALT: KeyModifiers = KeyModifiers {
        ctrl: false,
        alt: true,
        shift: false,
    };

    #[test]
    fn characters_forward_the_platform_text() {
        assert_eq!(
            key_input_bytes(&character("a"), Some("a"), PLAIN),
            Some(b"a".to_vec())
        );
        // Shift and dead keys are already in the text: the text
        // wins over the logical key's character.
        assert_eq!(
            key_input_bytes(&character("a"), Some("A"), PLAIN),
            Some(b"A".to_vec())
        );
        assert_eq!(
            key_input_bytes(&character("e"), Some("\u{e9}"), PLAIN),
            Some("\u{e9}".as_bytes().to_vec())
        );
        // No text: fall back to the logical character.
        assert_eq!(
            key_input_bytes(&character("z"), None, PLAIN),
            Some(b"z".to_vec())
        );
        assert_eq!(
            key_input_bytes(&Key::Space, Some(" "), PLAIN),
            Some(b" ".to_vec())
        );
    }

    #[test]
    fn named_keys_send_xterm_sequences() {
        // Oracle: the bytes xterm sends in normal cursor mode, so a
        // world sees the same input from both shells.
        let cases: [(Key, &[u8]); 16] = [
            (Key::Enter, b"\r"),
            (Key::Tab, b"\t"),
            (Key::Backspace, b"\x7f"),
            (Key::Escape, b"\x1b"),
            (Key::ArrowUp, b"\x1b[A"),
            (Key::ArrowDown, b"\x1b[B"),
            (Key::ArrowRight, b"\x1b[C"),
            (Key::ArrowLeft, b"\x1b[D"),
            (Key::Home, b"\x1b[H"),
            (Key::End, b"\x1b[F"),
            (Key::Delete, b"\x1b[3~"),
            (Key::PageUp, b"\x1b[5~"),
            (Key::PageDown, b"\x1b[6~"),
            (Key::F1, b"\x1bOP"),
            (Key::F5, b"\x1b[15~"),
            (Key::F12, b"\x1b[24~"),
        ];
        for (key, bytes) in cases {
            assert_eq!(
                key_input_bytes(&key, None, PLAIN),
                Some(bytes.to_vec()),
                "{key:?}"
            );
        }
    }

    #[test]
    fn ctrl_chords_send_control_bytes() {
        assert_eq!(
            key_input_bytes(&character("c"), Some("\u{3}"), CTRL),
            Some(vec![0x03])
        );
        assert_eq!(
            key_input_bytes(&character("A"), None, CTRL),
            Some(vec![0x01])
        );
        assert_eq!(
            key_input_bytes(&character("["), None, CTRL),
            Some(vec![0x1b])
        );
        assert_eq!(key_input_bytes(&Key::Space, None, CTRL), Some(vec![0x00]));
        // A chord with no control byte, or on a non-character key
        // with no encoding, sends nothing rather than a guess.
        assert_eq!(key_input_bytes(&character("1"), None, CTRL), None);
        assert_eq!(key_input_bytes(&character("ab"), None, CTRL), None);
        // Named keys keep their sequence under Ctrl.
        assert_eq!(
            key_input_bytes(&Key::Enter, None, CTRL),
            Some(b"\r".to_vec())
        );
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(
            key_input_bytes(&character("x"), Some("x"), ALT),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            key_input_bytes(
                &character("x"),
                None,
                KeyModifiers {
                    ctrl: true,
                    alt: true,
                    shift: false,
                }
            ),
            Some(vec![0x1b, 0x18])
        );
    }

    #[test]
    fn keys_without_an_encoding_send_nothing() {
        assert_eq!(key_input_bytes(&Key::Shift, None, PLAIN), None);
        assert_eq!(key_input_bytes(&Key::Control, None, CTRL), None);
        assert_eq!(
            key_input_bytes(&Key::Dead(Some('`')), Some("`"), PLAIN),
            None
        );
        assert_eq!(key_input_bytes(&Key::AudioVolumeUp, None, PLAIN), None);
        // Control characters in platform text never pass through.
        assert_eq!(
            key_input_bytes(&character("a"), Some("\u{1}"), PLAIN),
            Some(b"a".to_vec())
        );
        assert_eq!(
            key_input_bytes(
                &Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                Some("\u{7}"),
                PLAIN
            ),
            None
        );
    }

    const CTRL_SHIFT: KeyModifiers = KeyModifiers {
        ctrl: true,
        alt: false,
        shift: true,
    };
    const SHIFT: KeyModifiers = KeyModifiers {
        ctrl: false,
        alt: false,
        shift: true,
    };

    #[test]
    fn paste_chords_are_ctrl_shift_v_and_shift_insert() {
        assert!(is_paste_chord(&character("V"), CTRL_SHIFT));
        assert!(is_paste_chord(&character("v"), CTRL_SHIFT));
        assert!(is_paste_chord(&Key::Insert, SHIFT));
        // Ctrl+V alone stays the literal-next byte, Insert alone and
        // Shift+V stay keys, and Alt never makes a paste.
        assert!(!is_paste_chord(&character("v"), CTRL));
        assert_eq!(
            key_input_bytes(&character("v"), None, CTRL),
            Some(vec![0x16])
        );
        assert!(!is_paste_chord(&Key::Insert, PLAIN));
        assert!(!is_paste_chord(&character("V"), SHIFT));
        assert!(!is_paste_chord(&Key::Insert, CTRL_SHIFT));
        assert!(!is_paste_chord(
            &character("v"),
            KeyModifiers {
                alt: true,
                ..CTRL_SHIFT
            }
        ));
        assert!(!is_paste_chord(&character("b"), CTRL_SHIFT));
    }

    #[test]
    fn a_clipboard_read_becomes_one_contract_paste_frame() {
        assert_eq!(paste_event(""), None);
        let text = "line one\nline two\t\u{e9}";
        assert_eq!(
            native_event_frame(&paste_event(text).unwrap()).unwrap(),
            json!({"type": "paste", "text": text})
        );
        assert_eq!(
            native_event_frame(&paste_event(text).unwrap()).unwrap(),
            paste_frame(text).unwrap()
        );
        // At the bound it is one frame; one byte over is refused whole,
        // never split into several pastes or truncated.
        let at_bound = "x".repeat(crate::MAX_PASTE_BYTES);
        assert!(native_event_frame(&paste_event(&at_bound).unwrap()).is_ok());
        let over = format!("{at_bound}y");
        assert_eq!(
            native_event_frame(&paste_event(&over).unwrap()),
            Err(FrameError::PasteTooLarge {
                bytes: crate::MAX_PASTE_BYTES + 1
            })
        );
    }

    #[test]
    fn mapped_bytes_build_the_terminal_input_frame() {
        let bytes = key_input_bytes(&Key::ArrowUp, None, PLAIN).unwrap();
        assert_eq!(
            native_event_frame(&NativeEvent::KeyInput(bytes)).unwrap(),
            input_frame(b"\x1b[A")
        );
    }
}
