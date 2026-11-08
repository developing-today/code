//! Home/discovery page rendering and feed queries.

use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
};
use serde::Serialize;

use super::{AppState, templates::html_escape};

const MAX_FEED_ITEMS: usize = 20;
const SYSTEM_KEYS: &[&str] = &["created", "modified", "deleted", "name", "file", "path"];

/// A locally available document carrying public global metadata.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(super) struct FeedDocument {
    name: String,
    hash: String,
    updated_at: u64,
    tag_count: usize,
    tags: Vec<String>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub(super) struct FeedResponse {
    scope: String,
    popularity_metric: String,
    recent: Vec<FeedDocument>,
    popular: Vec<FeedDocument>,
    recent_tags: Vec<FeedTag>,
    popular_tags: Vec<FeedTag>,
}

/// A public tag attached to a locally available document.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(super) struct FeedTag {
    label: String,
    /// Subject is present for recent tags; popular tags aggregate across subjects.
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    /// Tag-store event time in microseconds since the Unix epoch.
    timestamp: u64,
    /// Number of local documents carrying this exact visible global tag.
    occurrences: usize,
}

/// Return only globally tagged documents whose blobs are present on this node.
pub(super) async fn feed_handler(State(state): State<AppState>) -> axum::response::Response {
    let tags = match state.tag_store.list_all(&state.tag_store.global).await {
        Ok(tags) => tags,
        Err(error) => {
            tracing::warn!("home feed global tag query failed: {error:#}");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let mut by_subject: std::collections::BTreeMap<String, Vec<crate::tags::Tag>> =
        std::collections::BTreeMap::new();
    for tag in tags {
        if let Some(subject) = tag.subject.as_str()
            && !subject.is_empty()
        {
            by_subject.entry(subject.to_owned()).or_default().push(tag);
        }
    }

    let mut documents = Vec::new();
    let mut recent_tags = Vec::new();
    let mut tag_occurrences = std::collections::BTreeMap::<String, (usize, u64)>::new();
    for (name, tags) in by_subject {
        if tags.iter().any(|tag| tag.key.as_bytes() == b"deleted") {
            continue;
        }
        let Some(hash) = super::routes::get_hash_for_name(&state.store, &name).await else {
            continue;
        };
        let mut updated_at = 0;
        let mut subject_tag_times = std::collections::BTreeMap::<String, u64>::new();
        for tag in &tags {
            let key = tag.key.display_lossy();
            if key == "created" || key == "modified" {
                updated_at = updated_at.max(
                    tag.value
                        .as_ref()
                        .and_then(|value| value.as_str()?.parse::<u64>().ok())
                        .unwrap_or(tag.timestamp / 1_000_000),
                );
            }
            if !SYSTEM_KEYS.contains(&key.as_str()) && !key.starts_with("archive.") {
                let label = match tag.value.as_ref().and_then(|value| value.as_str()) {
                    Some(value) if !value.is_empty() => format!("{key}:{value}"),
                    _ => key,
                };
                subject_tag_times
                    .entry(label)
                    .and_modify(|timestamp| *timestamp = (*timestamp).max(tag.timestamp))
                    .or_insert(tag.timestamp);
            }
        }
        if updated_at == 0 {
            updated_at = tags
                .iter()
                .map(|tag| tag.timestamp / 1_000_000)
                .max()
                .unwrap_or(0);
        }
        let mut visible_tags = Vec::with_capacity(subject_tag_times.len());
        for (label, timestamp) in subject_tag_times {
            visible_tags.push(label.clone());
            recent_tags.push(FeedTag {
                label: label.clone(),
                subject: Some(name.clone()),
                timestamp,
                occurrences: 1,
            });
            let occurrence = tag_occurrences.entry(label).or_insert((0, 0));
            occurrence.0 += 1;
            occurrence.1 = occurrence.1.max(timestamp);
        }
        documents.push(FeedDocument {
            name,
            hash,
            updated_at,
            tag_count: visible_tags.len(),
            tags: visible_tags,
        });
    }

    let mut recent = documents.clone();
    recent.sort_by_key(|doc| std::cmp::Reverse(doc.updated_at));
    recent.truncate(MAX_FEED_ITEMS);
    let mut popular = documents;
    popular.sort_by(|a, b| {
        b.tag_count
            .cmp(&a.tag_count)
            .then_with(|| a.name.cmp(&b.name))
    });
    popular.truncate(MAX_FEED_ITEMS);
    recent_tags.sort_by_key(|tag| std::cmp::Reverse(tag.timestamp));
    recent_tags.truncate(MAX_FEED_ITEMS);
    let mut popular_tags: Vec<FeedTag> = tag_occurrences
        .into_iter()
        .map(|(label, (occurrences, timestamp))| FeedTag {
            label,
            subject: None,
            timestamp,
            occurrences,
        })
        .collect();
    popular_tags.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.label.cmp(&b.label))
    });
    popular_tags.truncate(MAX_FEED_ITEMS);
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(FeedResponse {
            scope: "this server's local store and global tag namespace".to_owned(),
            popularity_metric: "number of visible global tags on the document".to_owned(),
            recent,
            popular,
            recent_tags,
            popular_tags,
        }),
    )
        .into_response()
}

pub(super) fn render_home(
    worlds: &[crate::world_hub::WorldInfo],
    default_world: &str,
    world_enabled: bool,
    world_route: &str,
) -> String {
    let world_name = html_escape(default_world);
    let world_href = urlencoding::encode(default_world);
    let mut html = String::from(
        "<section class=\"space-y-4\" data-home-app><h1>Home</h1><p class=\"text-sm opacity-70\">Discovery and live activity are scoped to this server; they do not synchronize between devices.</p>",
    );
    html.push_str("<section class=\"card bg-base-200 border border-base-300 p-4\"><h2>World explorer</h2><p>Available worlds on this server. A world marked asleep is stored and opens when selected; unknown worlds are not created just by browsing.</p><ul>");
    if worlds.is_empty() {
        let _ = std::fmt::Write::write_fmt(
            &mut html,
            format_args!(
                "<li><a href=\"{world_route}?world={world_href}\">{world_name}</a> (default world)</li>"
            ),
        );
    } else {
        for world in worlds {
            let name = html_escape(&world.name);
            let href = urlencoding::encode(&world.name);
            let state = if world.open {
                "open"
            } else {
                "stored; opens on selection"
            };
            let _ = std::fmt::Write::write_fmt(
                &mut html,
                format_args!(
                    "<li><a href=\"{world_route}?world={href}\">{name}</a> <span>{state}</span></li>"
                ),
            );
        }
    }
    html.push_str("</ul><p>Unavailable worlds are not listed; selecting a listed world reports if it cannot be opened.</p></section>");
    if world_enabled {
        let _ = std::fmt::Write::write_fmt(
            &mut html,
            format_args!(
                "<section class=\"card bg-base-200 border border-base-300 p-4\"><h2>Global chat</h2><p>This is the authenticated default world ({world_name}). Join with a guest capability to participate. Messages use the existing bounded world chat and are live only within this server.</p><iframe title=\"Live global chat in {world_name}\" src=\"{world_route}?world={world_href}\" loading=\"lazy\" class=\"w-full min-h-[36rem] border border-base-300\"></iframe></section>"
            ),
        );
    } else {
        html.push_str("<section class=\"card bg-base-200 border border-base-300 p-4\"><h2>Global chat</h2><p>World chat is unavailable because this server has no hosted world configured.</p></section>");
    }
    html.push_str("<p><a href=\"/files\" data-nav>Browse files</a></p>");
    html.push_str("<section class=\"card bg-base-200 border border-base-300 p-4\"><h2>Recent documents</h2><ol data-home-recent><li>Loading public local feed…</li></ol><h2>Popular documents</h2><p>Popularity is the number of visible global tags on each document; no view tracking is used.</p><ol data-home-popular><li>Loading public local feed…</li></ol><h2>Recent tags</h2><ol data-home-recent-tags><li>Loading public local feed…</li></ol><h2>Popular tags</h2><p>Popularity is the number of locally available documents carrying each exact global tag.</p><ol data-home-popular-tags><li>Loading public local feed…</li></ol><p data-home-updated aria-live=\"polite\"></p></section></section>");
    html
}

#[cfg(test)]
fn render_feed(documents: &[FeedDocument]) -> String {
    let mut html = String::from("<p>Popularity is the number of global tags.</p><ul>");
    for document in documents {
        let name = html_escape(&document.name);
        let href = urlencoding::encode(&document.name);
        let tags = document
            .tags
            .iter()
            .map(|tag| html_escape(tag))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = std::fmt::Write::write_fmt(
            &mut html,
            format_args!(
                "<li><a href=\"/edit/{href}\">{name}</a> — {} global tags <span>{tags}</span></li>",
                document.tag_count
            ),
        );
    }
    html.push_str("</ul>");
    html
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::{
        body::to_bytes,
        http::{Request, StatusCode},
    };
    use futures::{SinkExt, StreamExt};
    use iroh::endpoint::{Endpoint, RelayMode, presets};
    use iroh_blobs::{api::Store, store::mem::MemStore};
    use iroh_docs::protocol::Docs;
    use iroh_gossip::net::Gossip;
    use tower::ServiceExt;

    async fn test_router() -> (
        axum::Router,
        Store,
        std::sync::Arc<crate::tags::TagStore>,
        tempfile::TempDir,
        Endpoint,
        Docs,
        crate::world::WorldHandle,
    ) {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let docs = Docs::memory()
            .spawn(
                endpoint.clone(),
                store.clone(),
                Gossip::builder().spawn(endpoint.clone()),
            )
            .await
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let tags =
            crate::tags::TagStore::init_in(&docs, "home-test", temp.path().join(".iroh-meta"))
                .await
                .unwrap();
        let tags = std::sync::Arc::new(tags);
        let mut state = AppState::new(
            store.clone(),
            None,
            "test-node".to_owned(),
            tags.clone(),
            [7; 32],
            temp.path().join("identity.redb"),
        )
        .await
        .unwrap();
        let world = crate::world::WorldHandle::spawn(
            crate::world::WorldCore::new("lobby", crate::world::WorldLimits::default()).unwrap(),
        );
        state.world = Some(crate::world_hub::WorldHub::single_named(
            "lobby",
            crate::world_session::WorldService::new(world.clone(), None),
        ));
        (
            super::super::routes::create_router(state),
            store,
            tags,
            temp,
            endpoint,
            docs,
            world,
        )
    }

    #[test]
    fn homepage_has_world_explorer_live_chat_and_discovery_feed() {
        let html = render_home(&[], "lobby", true, "/world");
        assert!(html.contains("World explorer"));
        assert!(html.contains("Global chat"));
        assert!(html.contains("Recent documents"));
        assert!(html.contains("Recent tags"));
        assert!(html.contains("Popular tags"));
        assert!(!html.contains("<main"));
        assert!(html.contains("Popular documents"));
        assert!(html.contains("/world?world=lobby"));
        let custom = render_home(&[], "lobby", true, "/arena");
        assert!(custom.contains("/arena?world=lobby"));
        assert!(!custom.contains("/world?world=lobby"));
        assert!(html.contains("this server"));
        assert!(html.contains("/files"));
    }

    #[test]
    fn discovery_feed_only_renders_tagged_documents_and_explains_popularity() {
        let html = render_feed(&[FeedDocument {
            name: "public.md".to_owned(),
            hash: "abc".to_owned(),
            updated_at: 42,
            tag_count: 3,
            tags: vec!["rust".to_owned()],
        }]);
        assert!(html.contains("public.md"));
        assert!(html.contains("3 global tags"));
        assert!(html.contains("Popularity is the number of global tags"));
        assert!(!html.contains("private.md"));
    }

    #[tokio::test]
    async fn homepage_and_feed_routes_show_only_globally_tagged_local_documents() {
        let (router, store, tags, _temp, _endpoint, _docs, _world) = test_router().await;
        let home = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(home.status(), StatusCode::OK);
        let body = to_bytes(home.into_body(), 1_000_000).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("World explorer"));
        assert!(html.contains("Global chat"));
        assert!(html.contains("Recent documents"));
        assert!(html.contains("Recent tags"));
        assert!(html.contains("Popular tags"));
        assert!(html.contains("href=\"/files\""));

        let partial = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("X-Partial-Request", "true")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let partial = to_bytes(partial.into_body(), 1_000_000).await.unwrap();
        let partial = String::from_utf8(partial.to_vec()).unwrap();
        assert!(partial.contains("id=\"main-header\""));
        assert!(partial.contains("data-home-app"));
        assert!(partial.contains("<a href=\"/files\" data-nav class=\"crt-glow\">files</a>"));

        let public_blob = store.blobs().add_bytes(b"public".to_vec()).await.unwrap();
        store
            .tags()
            .set("public.md", public_blob.hash)
            .await
            .unwrap();
        let private_blob = store.blobs().add_bytes(b"private".to_vec()).await.unwrap();
        store
            .tags()
            .set("private.md", private_blob.hash)
            .await
            .unwrap();
        let untagged_blob = store.blobs().add_bytes(b"untagged".to_vec()).await.unwrap();
        store
            .tags()
            .set("untagged.md", untagged_blob.hash)
            .await
            .unwrap();
        let deleted_blob = store.blobs().add_bytes(b"deleted".to_vec()).await.unwrap();
        store
            .tags()
            .set("deleted.md", deleted_blob.hash)
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"public.md", b"label", Some(b"public"), b"")
            .await
            .unwrap();
        // A distinct key rendering the same label must not inflate popularity
        // for this subject.
        tags.set_tag(&tags.global, b"public.md", b"label:public", None, b"")
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"public.md", b"category", Some(b"notes"), b"")
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"public.md", b"modified", Some(b"100"), b"")
            .await
            .unwrap();
        let recent_blob = store.blobs().add_bytes(b"recent".to_vec()).await.unwrap();
        store
            .tags()
            .set("recent.md", recent_blob.hash)
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"recent.md", b"label", Some(b"public"), b"")
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"recent.md", b"modified", Some(b"200"), b"")
            .await
            .unwrap();
        tags.set_tag(&tags.node, b"private.md", b"label", Some(b"private"), b"")
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"deleted.md", b"label", Some(b"public"), b"")
            .await
            .unwrap();
        tags.set_tag(&tags.global, b"deleted.md", b"deleted", None, b"")
            .await
            .unwrap();
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/home/feed")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 100_000).await.unwrap();
        let feed: FeedResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(feed.recent.len(), 2);
        assert_eq!(feed.recent[0].name, "recent.md");
        assert_eq!(feed.popular[0].name, "public.md");
        assert_eq!(feed.popular[0].tag_count, 2);
        assert_eq!(feed.recent_tags.len(), 3);
        assert!(feed.recent_tags[0].timestamp >= feed.recent_tags[1].timestamp);
        assert_eq!(feed.popular_tags[0].label, "label:public");
        assert_eq!(feed.popular_tags[0].occurrences, 2);
        assert_eq!(
            feed.scope,
            "this server's local store and global tag namespace"
        );

        assert!(!feed.recent.iter().any(|doc| doc.name == "private.md"));
        assert!(!feed.recent.iter().any(|doc| doc.name == "untagged.md"));
        assert!(!feed.recent.iter().any(|doc| doc.name == "deleted.md"));
        assert!(
            !feed
                .recent_tags
                .iter()
                .any(|tag| tag.subject.as_deref() == Some("deleted.md"))
        );

        let files = router
            .oneshot(
                Request::builder()
                    .uri("/files")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(files.status(), StatusCode::OK);
    }

    #[test]
    fn global_chat_embeds_existing_authenticated_world_chat() {
        let html = render_home(&[], "lobby", true, "/world");
        assert!(html.contains("/world?world=lobby"));
        assert!(html.contains("authenticated default world"));
        assert!(html.contains("guest capability"));
    }

    #[tokio::test]
    async fn home_chat_sends_and_receives_live_messages_for_authenticated_world_members() {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        let (router, _store, _tags, _temp, _endpoint, _docs, world) = test_router().await;
        let (_, alice) = world
            .issue("alice".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, bob) = world
            .issue("bob".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let connect = |capability: crate::world::JoinCapability| async move {
            let (mut client, _) =
                tokio_tungstenite::connect_async(format!("ws://{address}/ws/world"))
                    .await
                    .unwrap();
            client
                .send(ClientMessage::Text(
                    serde_json::json!({
                        "type": "join",
                        "capability": capability.expose(),
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            client
        };
        let mut alice = connect(alice).await;
        let mut bob = connect(bob).await;
        for client in [&mut alice, &mut bob] {
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let ClientMessage::Text(frame) = message else {
                panic!("expected snapshot text frame");
            };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&frame).unwrap()["type"],
                "snapshot"
            );
        }
        alice
            .send(ClientMessage::Text(
                r#"{"type":"chat","text":"hello from home"}"#.into(),
            ))
            .await
            .unwrap();
        for client in [&mut alice, &mut bob] {
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let ClientMessage::Text(frame) = message else {
                panic!("expected chat text frame");
            };
            let frame: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(frame["type"], "event");
            assert!(frame.to_string().contains("hello from home"));
        }
    }
}
