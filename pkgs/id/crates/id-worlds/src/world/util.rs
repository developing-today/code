use super::*;

pub(crate) use crate::text::sanitize_chat;

use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::watch;

/// Incremental event page for an authorized participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPage {
    /// Latest committed sequence at response time.
    pub current_sequence: u64,
    /// Earliest retained sequence.
    pub oldest_retained_sequence: u64,
    /// The event cursor fell behind retention and a full snapshot is required.
    pub needs_snapshot: bool,
    /// Events newer than the requested cursor, if it is still retained.
    pub events: Vec<WorldEvent>,
}

pub(crate) fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub(crate) fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    bool::from(left.ct_eq(right))
}

/// Lower-case hex encoding of a byte slice; the definition lives
/// in `id-core` ([`id_core::hex::hex_encode`], crate split step 6a,
/// #4409) and is re-exported at its historical path.
pub use id_core::hex::hex_encode;

/// Publish a new records snapshot through the watch channel when it differs.
pub fn publish_records(
    sender: &watch::Sender<(u64, Arc<Records>)>,
    sequence: u64,
    records: Records,
) {
    sender.send_if_modified(|current| {
        if *current.1 == records {
            false
        } else {
            *current = (sequence, Arc::new(records));
            true
        }
    });
}

pub(crate) fn hex_decode_32(encoded: &str) -> Option<[u8; 32]> {
    let bytes = encoded.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let nibble = |b: u8| char::from(b).to_digit(16);
        *slot = u8::try_from(nibble(pair[0])? << 4 | nibble(pair[1])?).ok()?;
    }
    Some(out)
}

pub(crate) fn sanitize_display_name(name: &str) -> String {
    // #4711: trim again after the 48-char cap. The pre-cap trim alone
    // let truncation land on a whitespace character, minting a 48-char
    // name ending in whitespace — a fixed point of neither sanitizer,
    // which the #2960 restore check then refused, leaving the journal
    // unrestorable. The final trim makes every minted name a fixed
    // point of this function.
    pre_fix_sanitize_display_name(name).trim().to_owned()
}

/// The display-name sanitizer as it existed before #4711 added the
/// post-truncation trim: control and format characters stripped,
/// trimmed, then capped at 48 chars — with no trim after the cap, so a
/// name whose 48th character was whitespace was minted with trailing
/// whitespace. Journals written by that mint can legitimately carry
/// such names, and restore accepts those for journal compatibility
/// (see the #2960 check in `WorldCore::restore`).
pub(crate) fn pre_fix_sanitize_display_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !ch.is_control() && !crate::petname::is_format_char(*ch))
        .collect::<String>()
        .trim()
        .chars()
        .take(48)
        .collect()
}

/// The display-name sanitizer as it existed before #2112 (commit
/// 872aa09f) added format-character stripping: control characters
/// stripped, 48-char cap, trimmed. Journals written by that earlier
/// mint can legitimately carry names that are fixed points of this
/// version but not of the current one — ZWJ/ZWNJ sequences such as
/// family emoji are the common case — and restore accepts those for
/// journal compatibility (see the #2960 check in `WorldCore::restore`).
pub(crate) fn legacy_sanitize_display_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !ch.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Whether `name` is a display name the pre-#4711 mint could have
/// produced. That mint returned the first 48 chars of a control- and
/// format-stripped, trimmed string, so a producible name contains no
/// stripped characters, never starts with whitespace, is at most 48
/// chars, and — unless it fills the whole 48-char budget — is fully
/// trimmed: trailing whitespace was only possible when truncation
/// landed on it at exactly 48 chars. (Such names are not fixed points
/// of the pre-fix sanitizer — re-applying it trims them — so restore
/// cannot recognize them by equality; see the #2960 check in
/// `WorldCore::restore`.)
pub(crate) fn is_pre_fix_mint_display_name(name: &str) -> bool {
    if name
        .chars()
        .next()
        .is_none_or(|first| first.is_whitespace())
    {
        return false;
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || crate::petname::is_format_char(ch))
    {
        return false;
    }
    let count = name.chars().count();
    count == 48 || (count < 48 && name.trim() == name)
}

#[cfg(test)]
mod tests {
    use super::{
        is_pre_fix_mint_display_name, legacy_sanitize_display_name, pre_fix_sanitize_display_name,
        sanitize_chat, sanitize_display_name,
    };

    #[test]
    fn display_name_strips_bidi_override() {
        assert_eq!(sanitize_display_name("evil\u{202e}name"), "evilname");
    }

    #[test]
    fn display_name_strips_zero_width_only_name_to_empty() {
        assert_eq!(sanitize_display_name("\u{200b}\u{200b}"), "");
    }

    #[test]
    fn display_name_strips_joiners() {
        assert_eq!(sanitize_display_name("a\u{200d}b"), "ab");
        assert_eq!(sanitize_display_name("a\u{200c}b"), "ab");
    }

    #[test]
    fn legacy_display_name_keeps_joiners_but_strips_controls() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(legacy_sanitize_display_name(family), family);
        assert_eq!(legacy_sanitize_display_name("a\u{7}b"), "ab");
        assert_eq!(legacy_sanitize_display_name("  Ada  "), "Ada");
    }

    #[test]
    fn display_name_strips_control_chars() {
        assert_eq!(sanitize_display_name("a\u{7}b"), "ab");
    }

    #[test]
    fn display_name_trims_before_applying_character_budget() {
        for spaces in [48, 47, 46] {
            let padded = format!("{}Bob", " ".repeat(spaces));
            assert_eq!(sanitize_display_name(&padded), "Bob");
        }

        let exactly_48 = "x".repeat(48);
        assert_eq!(sanitize_display_name(&format!(" {exactly_48}")), exactly_48);
        assert_eq!(
            sanitize_display_name(&format!("\u{7}{exactly_48}")),
            exactly_48
        );
    }

    #[test]
    fn display_name_truncation_never_leaves_trailing_whitespace() {
        // #4711 boundary shapes: whitespace landing exactly on the
        // 48-char cap is trimmed after truncation, and every minted
        // name is a fixed point of the sanitizer.
        let cases = [
            (
                format!("{} {}", "a".repeat(47), "b".repeat(10)),
                "a".repeat(47),
            ),
            (
                format!("{}  {}", "a".repeat(46), "b".repeat(10)),
                "a".repeat(46),
            ),
            (
                format!("{}\u{a0}{}", "a".repeat(47), "b".repeat(10)),
                "a".repeat(47),
            ),
        ];
        for (input, expected) in cases {
            let minted = sanitize_display_name(&input);
            assert_eq!(minted, expected, "input: {input:?}");
            assert_eq!(sanitize_display_name(&minted), minted);
        }
        // A full 48-char name with no boundary whitespace is untouched.
        let exactly_48 = "x".repeat(48);
        assert_eq!(sanitize_display_name(&exactly_48), exactly_48);
    }

    #[test]
    fn pre_fix_sanitizer_reproduces_boundary_names_restore_must_accept() {
        // The pre-#4711 mint produced exactly this 48-char name ending
        // in a space; journals carrying it must remain restorable.
        let boundary = format!("{} ", "a".repeat(47));
        assert_eq!(
            pre_fix_sanitize_display_name(&format!("{} {}", "a".repeat(47), "b".repeat(10))),
            boundary
        );
        // It is recognizable as a pre-fix mint output even though it
        // is not a fixed point of the pre-fix sanitizer (re-applying
        // that sanitizer trims it), and the current sanitizer maps it
        // to its trim.
        assert!(is_pre_fix_mint_display_name(&boundary));
        assert_eq!(sanitize_display_name(&boundary), "a".repeat(47));
        // Names no mint path could produce are not pre-fix outputs:
        // short-but-untrimmed, over-length, control-bearing, or
        // leading-whitespace names.
        assert!(!is_pre_fix_mint_display_name("Ada "));
        assert!(!is_pre_fix_mint_display_name(&"a".repeat(49)));
        assert!(!is_pre_fix_mint_display_name("a\u{7}b"));
        assert!(!is_pre_fix_mint_display_name(" Ada"));
        assert!(is_pre_fix_mint_display_name("Ada"));
    }

    #[test]
    fn chat_strips_bidi_isolates() {
        assert_eq!(sanitize_chat("a\u{2066}b\u{2069}c"), "abc");
    }

    #[test]
    fn chat_preserves_zwj_emoji_and_zwnj() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(sanitize_chat(family), family);
        assert_eq!(sanitize_chat("a\u{200c}b"), "a\u{200c}b");
    }

    #[test]
    fn chat_strips_other_format_and_control_chars() {
        assert_eq!(sanitize_chat("a\u{200b}b"), "ab");
        assert_eq!(sanitize_chat("a\u{202e}b"), "ab");
        assert_eq!(sanitize_chat("a\u{7}b"), "ab");
    }

    #[test]
    fn chat_preserves_newline_and_tab() {
        assert_eq!(sanitize_chat("a\nb\tc"), "a\nb\tc");
    }
}
