//! Byte-level text helpers mirroring `RocText.roc`: total functions that
//! never trap. Operations work on UTF-8 bytes where the Roc originals do;
//! `lines` follows `Str.split_on "\n"` semantics used by the examples.

/// Trim ASCII whitespace (space, tab, newline, carriage return) from
/// both ends, like `RocText.trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// ASCII lowercase, like `RocText.lowercase`.
pub fn lowercase(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_lowercase()).collect()
}

pub fn starts_with(s: &str, prefix: &str) -> bool {
    s.as_bytes().starts_with(prefix.as_bytes())
}

pub fn drop_prefix<'a>(s: &'a str, prefix: &str) -> &'a str {
    s.strip_prefix(prefix).unwrap_or(s)
}

pub fn contains(s: &str, needle: &str) -> bool {
    s.as_bytes()
        .windows(needle.len().max(1))
        .any(|w| w == needle.as_bytes())
        || needle.is_empty()
}

/// Split on '\n', like `RocText.lines`. A trailing newline does not
/// produce a trailing empty line beyond what `str::split` yields after
/// the final separator is dropped by the Roc helper's shape; this keeps
/// the exact `split` pieces, matching how the examples consume them.
pub fn lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('\n').collect()
    }
}

pub fn join_lines(parts: &[String]) -> String {
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_matches_roc_text() {
        assert_eq!(trim("  inc \n"), "inc");
        assert_eq!(trim("inc"), "inc");
        assert_eq!(trim(" \t\r\n"), "");
    }

    #[test]
    fn helpers_are_total() {
        assert_eq!(lowercase("AbC-XYz"), "abc-xyz");
        assert!(starts_with("hello", "he"));
        assert!(!starts_with("hi", "hello"));
        assert_eq!(drop_prefix("pre:fix", "pre:"), "fix");
        assert_eq!(drop_prefix("fix", "pre:"), "fix");
        assert!(contains("a-b-c", "-b-"));
        assert!(!contains("abc", "d"));
        assert_eq!(lines("a\nb"), vec!["a", "b"]);
        assert_eq!(lines(""), Vec::<&str>::new());
        assert_eq!(join_lines(&["a".into(), "b".into()]), "a\nb");
    }
}
