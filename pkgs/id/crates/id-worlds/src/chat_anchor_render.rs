//! Text projection for chat anchors in terminal surfaces (#5873 section 5).

use id_store::chat_anchor::LinkState;

/// Render one projected anchor as a terminal header line.
///
/// `surface` and `part_label` are local display data, not protocol state. If a
/// target cannot be resolved for this viewer, pass `None`; the output then
/// contains only the canonical reference and does not disclose why resolution
/// failed. `fragment` is the already-validated #5870 `pa1` selector.
#[must_use]
pub fn render_anchor_header(
    target_id: &str,
    relation: &str,
    state: LinkState,
    surface: Option<&str>,
    part_label: Option<&str>,
    fragment: Option<&str>,
) -> Option<String> {
    if state == LinkState::Unbound {
        return None;
    }

    let reference = match fragment {
        Some(fragment) => format!("id://{target_id}#{fragment}"),
        None => format!("id://{target_id}"),
    };
    let label = if state == LinkState::Tombstoned {
        "deleted".to_owned()
    } else if let Some(surface) = surface {
        let surface = single_line(surface);
        match part_label {
            Some(part) => format!("{surface} {}", single_line(part)),
            None => surface,
        }
    } else {
        // Do not reveal whether the target is absent or merely inaccessible.
        return Some(format!("anchored: {reference} ({relation}){}", state_marker(state)));
    };

    Some(format!(
        "anchored: [{label}]({reference}) ({relation}){}",
        state_marker(state)
    ))
}

fn state_marker(state: LinkState) -> &'static str {
    match state {
        LinkState::Disputed => " [disputed]",
        LinkState::Tombstoned => " [tombstoned]",
        LinkState::Current | LinkState::Relocated | LinkState::Unbound => "",
    }
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::render_anchor_header;
    use id_store::chat_anchor::LinkState;

    #[test]
    fn resolved_anchor_uses_the_canonical_reference_form() {
        assert_eq!(
            render_anchor_header(
                "abc",
                "about",
                LinkState::Current,
                Some("Design doc"),
                None,
                None,
            )
            .as_deref(),
            Some("anchored: [Design doc](id://abc) (about)"),
        );
    }

    #[test]
    fn unresolved_and_inaccessible_targets_have_the_same_plain_reference() {
        let missing = render_anchor_header("abc", "about", LinkState::Current, None, None, None);
        let hidden = render_anchor_header("abc", "about", LinkState::Current, None, None, None);
        assert_eq!(missing, hidden);
        assert_eq!(missing.as_deref(), Some("anchored: id://abc (about)"));
    }

    #[test]
    fn part_selectors_and_local_labels_are_rendered_as_references() {
        assert_eq!(
            render_anchor_header(
                "abc",
                "anchored-to-part",
                LinkState::Current,
                Some("Design doc"),
                Some("§3"),
                Some("pa1~b~r1~p~block"),
            )
            .as_deref(),
            Some("anchored: [Design doc §3](id://abc#pa1~b~r1~p~block) (anchored-to-part)"),
        );
    }

    #[test]
    fn disputed_and_tombstoned_links_are_never_silently_dropped() {
        assert_eq!(
            render_anchor_header("abc", "about", LinkState::Disputed, Some("Doc"), None, None)
                .as_deref(),
            Some("anchored: [Doc](id://abc) (about) [disputed]"),
        );
        assert_eq!(
            render_anchor_header("abc", "about", LinkState::Tombstoned, Some("Doc"), None, None)
                .as_deref(),
            Some("anchored: [deleted](id://abc) (about) [tombstoned]"),
        );
        assert_eq!(
            render_anchor_header("abc", "about", LinkState::Unbound, Some("Doc"), None, None),
            None,
        );
    }

    #[test]
    fn local_labels_cannot_inject_terminal_lines() {
        assert_eq!(
            render_anchor_header("abc", "about", LinkState::Current, Some("Doc\nforged"), None, None)
                .as_deref(),
            Some("anchored: [Doc forged](id://abc) (about)"),
        );
    }
}
