//! Unicode properties shared across the `id-core` and `id` crates.

/// Whether `c` is a Unicode 16 format character (general category Cf).
///
/// This includes zero-width characters, bidi controls and isolates, soft
/// hyphen, and other format controls. The predicate is shared by name
/// validation and plain-text rendering so every surface uses the same set.
#[must_use]
pub fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}'
        | '\u{0600}'..='\u{0605}'
        | '\u{061C}'
        | '\u{06DD}'
        | '\u{070F}'
        | '\u{0890}'..='\u{0891}'
        | '\u{08E2}'
        | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}'
        | '\u{110BD}'
        | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0001}'
        | '\u{E0020}'..='\u{E007F}')
}

#[cfg(test)]
mod tests {
    use super::is_format_char;

    #[test]
    fn format_char_matches_unicode_16_range_boundaries() {
        for c in [
            '\u{00AD}',
            '\u{0600}',
            '\u{0605}',
            '\u{061C}',
            '\u{06DD}',
            '\u{070F}',
            '\u{0890}',
            '\u{0891}',
            '\u{08E2}',
            '\u{180E}',
            '\u{200B}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2060}',
            '\u{2064}',
            '\u{2066}',
            '\u{206F}',
            '\u{FEFF}',
            '\u{FFF9}',
            '\u{FFFB}',
            '\u{110BD}',
            '\u{110CD}',
            '\u{13430}',
            '\u{1343F}',
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{1D173}',
            '\u{1D17A}',
            '\u{E0001}',
            '\u{E0020}',
            '\u{E007F}',
        ] {
            assert!(is_format_char(c), "{c:?} should be Unicode 16 Cf");
        }

        for c in [
            '\u{005F}',
            '\u{061B}',
            '\u{061D}',
            '\u{1D172}',
            '\u{1D17B}',
            '\u{1D182}',
            '\u{E0080}',
        ] {
            assert!(!is_format_char(c), "{c:?} should not be Unicode 16 Cf");
        }
    }

    /// Oracle: the Unicode 16 general-category Cf codepoint set, generated
    /// from Unicode 16 data (Python `unicodedata`, unidata_version 16.0.0)
    /// for issue #7343. The boundary test above samples range edges only;
    /// this test scans every codepoint, so a range interior that shrinks,
    /// a dropped singleton, or a widened range fails the suite — the
    /// omission class that shipped once as #4070 (U+061C missing).
    #[test]
    fn format_char_matches_unicode_16_cf_set_exhaustively() {
        const UNICODE_16_CF: &[(char, char)] = &[
            ('\u{00AD}', '\u{00AD}'),
            ('\u{0600}', '\u{0605}'),
            ('\u{061C}', '\u{061C}'),
            ('\u{06DD}', '\u{06DD}'),
            ('\u{070F}', '\u{070F}'),
            ('\u{0890}', '\u{0891}'),
            ('\u{08E2}', '\u{08E2}'),
            ('\u{180E}', '\u{180E}'),
            ('\u{200B}', '\u{200F}'),
            ('\u{202A}', '\u{202E}'),
            ('\u{2060}', '\u{2064}'),
            ('\u{2066}', '\u{206F}'),
            ('\u{FEFF}', '\u{FEFF}'),
            ('\u{FFF9}', '\u{FFFB}'),
            ('\u{110BD}', '\u{110BD}'),
            ('\u{110CD}', '\u{110CD}'),
            ('\u{13430}', '\u{1343F}'),
            ('\u{1BCA0}', '\u{1BCA3}'),
            ('\u{1D173}', '\u{1D17A}'),
            ('\u{E0001}', '\u{E0001}'),
            ('\u{E0020}', '\u{E007F}'),
        ];
        let expected = |c: char| UNICODE_16_CF.iter().any(|&(lo, hi)| (lo..=hi).contains(&c));
        for c in (0..=0x10FFFF).filter_map(char::from_u32) {
            assert_eq!(
                is_format_char(c),
                expected(c),
                "{c:?} (U+{:04X}) disagrees with the Unicode 16 Cf set",
                u32::from(c)
            );
        }
    }
}
