//! Home/discovery page rendering and feed queries.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn homepage_has_world_explorer_live_chat_and_discovery_feed() {
        let html = render_home(&[], "lobby");
        assert!(html.contains("World explorer"));
        assert!(html.contains("Global chat"));
        assert!(html.contains("Recent documents and tags"));
        assert!(html.contains("Popular documents and tags"));
        assert!(html.contains("/world?world=lobby"));
        assert!(html.contains("this server"));
    }

    #[test]
    fn discovery_feed_only_renders_tagged_documents_and_explains_popularity() {
        let html = render_feed(&[
            FeedDocument {
                name: "public.md".to_owned(),
                hash: "abc".to_owned(),
                updated_at: 42,
                tag_count: 3,
                tags: vec!["rust".to_owned()],
            },
        ]);
        assert!(html.contains("public.md"));
        assert!(html.contains("3 global tags"));
        assert!(html.contains("Popularity is the number of global tags"));
        assert!(!html.contains("private.md"));
    }
}
