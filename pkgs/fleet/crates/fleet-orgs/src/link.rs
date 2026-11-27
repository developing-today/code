//! RFC 8288 `Link` header parsing for GitHub's pagination.

/// The `rel="next"` target of a `Link` header, if there is one.
pub fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let target = pieces.next()?.trim();
        let url = target.strip_prefix('<')?.strip_suffix('>')?;
        pieces
            .any(|p| {
                let p = p.trim();
                p == r#"rel="next""# || p == "rel=next"
            })
            .then(|| url.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_next_among_other_rels() {
        let h = r#"<https://api.github.com/app/installations?per_page=100&page=3>; rel="next", <https://api.github.com/app/installations?per_page=100&page=9>; rel="last""#;
        assert_eq!(
            next_link(h).as_deref(),
            Some("https://api.github.com/app/installations?per_page=100&page=3")
        );
    }

    #[test]
    fn last_page_has_no_next() {
        let h = r#"<https://api.github.com/app/installations?page=1>; rel="prev", <https://api.github.com/app/installations?page=1>; rel="first""#;
        assert_eq!(next_link(h), None);
        assert_eq!(next_link(""), None);
    }
}
