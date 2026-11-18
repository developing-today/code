//! The program gallery's public read surface (#10003; v3 design
//! `doc/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3/`, G1, G3, G7).
//!
//! Three routes over a [`Gallery`], answered as JSON records and bundle
//! bytes exactly as the registry holds them:
//!
//! - `GET /api/gallery/records?author=&name=`: the registry's list.
//! - `GET /api/gallery/bundles/{content_hash}`: the registry's fetch.
//! - `GET /api/gallery/search?q=&category=&limit=`: search over the
//!   rebuildable index, in its documented order (exact name, then
//!   category, then most recent version).
//!
//! Records are signed and bundles are content-addressed, so every answer
//! is cacheable at any tier without trust implications (G7): a cache cannot
//! forge a record that verifies. A fetched bundle never changes, so it is
//! marked immutable. With no gallery wired in, every route answers 404.

use std::sync::{Arc, RwLock};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};

use id_worlds::program_bundle::BundleRecord;
use id_worlds::program_gallery::{Gallery, SearchQuery};
use id_worlds::program_registry::ListQuery;

/// A gallery shared between the web routes and whatever publishes to it.
pub type SharedGallery = Arc<RwLock<Gallery>>;

/// How long list and search answers may be cached, in seconds. They change
/// when someone publishes; the records in them never do.
const LISTING_MAX_AGE: &str = "public, max-age=60";

/// A bundle is content-addressed, so its bytes never change.
const BUNDLE_CACHE: &str = "public, max-age=31536000, immutable";

/// The order search results come in, stated in every search answer.
pub const SEARCH_ORDER: &str = "exact-name, category, most-recent-version";

/// The gallery routes, for merging into a router.
pub fn gallery_routes() -> Router<Option<SharedGallery>> {
    Router::new()
        .route("/api/gallery/records", get(records_handler))
        .route("/api/gallery/bundles/{content_hash}", get(bundle_handler))
        .route("/api/gallery/search", get(search_handler))
}

/// A list or search answer.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordsAnswer {
    /// The signed records, each verifiable without its payload.
    pub records: Vec<BundleRecord>,
    /// For a search, the documented result order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListParams {
    author: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    category: Option<String>,
    limit: Option<usize>,
}

async fn records_handler(
    State(gallery): State<Option<SharedGallery>>,
    Query(params): Query<ListParams>,
) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let query = ListQuery {
        author: params.author,
        name: params.name,
    };
    let records = match gallery.read() {
        Ok(gallery) => gallery.list(&query),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    listing(RecordsAnswer {
        records,
        order: None,
    })
}

async fn bundle_handler(
    State(gallery): State<Option<SharedGallery>>,
    Path(content_hash): Path<String>,
) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let well_formed = content_hash.len() == 64
        && content_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !well_formed {
        return (
            StatusCode::BAD_REQUEST,
            "a content hash is 64 lower-case hex digits",
        )
            .into_response();
    }
    let bytes = match gallery.read() {
        Ok(gallery) => gallery.fetch(&content_hash),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(bytes) = bytes else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(BUNDLE_CACHE),
    );
    if let Ok(etag) = HeaderValue::from_str(&format!("\"{content_hash}\"")) {
        headers.insert(header::ETAG, etag);
    }
    response
}

async fn search_handler(
    State(gallery): State<Option<SharedGallery>>,
    Query(params): Query<SearchParams>,
) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let query = SearchQuery {
        text: params.q,
        category: params.category,
        limit: params.limit,
    };
    let found = match gallery.read() {
        Ok(gallery) => gallery.search(&query),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match found {
        Ok(records) => listing(RecordsAnswer {
            records,
            order: Some(SEARCH_ORDER.to_owned()),
        }),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

fn listing(answer: RecordsAnswer) -> Response {
    let mut response = Json(answer).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(LISTING_MAX_AGE),
    );
    response
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use ed25519_dalek::SigningKey;
    use tower::ServiceExt as _;

    use id_worlds::program_bundle::{ProgramBundle, ProgramSpec, export};
    use id_worlds::program_registry::{AdmitAll, Registry};

    use super::*;

    fn bundle(name: &str, version: u64, description: &str, categories: &[&str]) -> ProgramBundle {
        export(
            format!("{name} {version}").into_bytes(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: Some(description),
                categories,
            },
            &SigningKey::from_bytes(&[7; 32]),
        )
        .unwrap()
    }

    fn app(gallery: Option<SharedGallery>) -> Router {
        gallery_routes().with_state(gallery)
    }

    fn populated() -> (Router, ProgramBundle) {
        let registry = Registry::open_dir(tempfile::tempdir().unwrap().keep(), AdmitAll).unwrap();
        let mut gallery = Gallery::new(registry);
        let chess = bundle("chess", 2, "The classic board game.", &["board"]);
        for published in [
            bundle("chess", 1, "The classic board game.", &["board"]),
            chess.clone(),
            bundle("go", 1, "Stones on a board.", &["board"]),
            bundle("racer", 4, "Arcade driving.", &["arcade"]),
        ] {
            gallery.publish(&published.to_bytes()).unwrap();
        }
        (app(Some(Arc::new(RwLock::new(gallery)))), chess)
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), 64 << 20)
            .await
            .unwrap();
        (status, headers, body.to_vec())
    }

    fn names(body: &[u8]) -> Vec<(String, u64)> {
        let answer: RecordsAnswer = serde_json::from_slice(body).unwrap();
        answer
            .records
            .iter()
            .map(|record| {
                let manifest = record.verify().expect("served records verify");
                (manifest.name, manifest.version)
            })
            .collect()
    }

    #[tokio::test]
    async fn list_serves_verifiable_records_and_filters() {
        let (app, _) = populated();
        let (status, headers, body) = get(&app, "/api/gallery/records").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], LISTING_MAX_AGE);
        assert_eq!(names(&body).len(), 4);
        let (_, _, body) = get(&app, "/api/gallery/records?name=chess").await;
        assert_eq!(
            names(&body),
            vec![("chess".to_owned(), 1), ("chess".to_owned(), 2)]
        );
    }

    #[tokio::test]
    async fn fetch_serves_the_exact_bundle_as_immutable() {
        let (app, chess) = populated();
        let hash = &chess.manifest().content_hash;
        let (status, headers, body) = get(&app, &format!("/api/gallery/bundles/{hash}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, chess.to_bytes());
        assert!(
            ProgramBundle::decode(&body).is_ok(),
            "a fetched bundle verifies"
        );
        assert_eq!(headers[header::CACHE_CONTROL], BUNDLE_CACHE);
        assert_eq!(headers[header::ETAG], format!("\"{hash}\"").as_str());

        let missing = "0".repeat(64);
        let (status, _, _) = get(&app, &format!("/api/gallery/bundles/{missing}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = get(&app, "/api/gallery/bundles/not-a-hash").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_answers_in_the_documented_order() {
        let (app, _) = populated();
        let (status, _, body) = get(&app, "/api/gallery/search?q=chess").await;
        assert_eq!(status, StatusCode::OK);
        let answer: RecordsAnswer = serde_json::from_slice(&body).unwrap();
        assert_eq!(answer.order.as_deref(), Some(SEARCH_ORDER));
        assert_eq!(
            names(&body),
            vec![("chess".to_owned(), 2), ("chess".to_owned(), 1)]
        );
        let (_, _, body) = get(&app, "/api/gallery/search?q=board").await;
        assert_eq!(
            names(&body),
            vec![
                ("chess".to_owned(), 2),
                ("chess".to_owned(), 1),
                ("go".to_owned(), 1)
            ],
            "category matches, most recent version first"
        );
        let (_, _, body) = get(&app, "/api/gallery/search?category=arcade").await;
        assert_eq!(names(&body), vec![("racer".to_owned(), 4)]);
        let (_, _, body) = get(&app, "/api/gallery/search?limit=1").await;
        assert_eq!(names(&body), vec![("racer".to_owned(), 4)]);
        let long = "a".repeat(id_worlds::program_gallery::MAX_QUERY_BYTES + 1);
        let (status, _, _) = get(&app, &format!("/api/gallery/search?q={long}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn without_a_gallery_every_route_is_not_found() {
        let app = app(None);
        for uri in [
            "/api/gallery/records",
            "/api/gallery/search?q=chess",
            &format!("/api/gallery/bundles/{}", "0".repeat(64)),
        ] {
            assert_eq!(get(&app, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
        }
    }
}
