//! Text sanitation shared by world chat and the general chat module.

/// Remove disallowed controls and format characters from chat text.
///
/// Newlines and tabs remain valid; zero-width joiners are retained for
/// grapheme sequences. The result is trimmed to match world chat behavior.
pub fn sanitize_chat(text: &str) -> String {
    text.chars()
        .filter(|ch| {
            (!ch.is_control() || *ch == '\n' || *ch == '\t')
                && (!crate::is_format_char(*ch) || *ch == '\u{200c}' || *ch == '\u{200d}')
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::sanitize_chat;

    #[test]
    fn sanitize_chat_keeps_allowed_whitespace_and_joiners() {
        let text = "\u{7}hello\n\t\u{200d}world\u{202e} ";
        assert_eq!(sanitize_chat(text), "hello\n\t\u{200d}world");
    }
}

/// Truncate `s` to at most `max` bytes **on a character boundary**, appending `…`.
pub fn truncate_on_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
