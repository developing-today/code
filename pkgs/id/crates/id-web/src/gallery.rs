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
//! - `GET /api/gallery/authors/{key}`: an author page addressed by key
//!   alone (G5, #10004).
//! - `GET /api/gallery/collections` and
//!   `GET /api/gallery/collections/{operator}/{name}`: featured collections,
//!   each signed by and attributed to its operator (G4).
//!
//! Operator speech (collections and the operator's tags, G2) is always
//! under a field that names the operator, never mixed into a record, so a
//! surface cannot present it as the author's.
//!
//! Records are signed and bundles are content-addressed, so every answer
//! is cacheable at any tier without trust implications (G7): a cache cannot
//! forge a record that verifies. A fetched bundle never changes, so it is
//! marked immutable. With no gallery wired in, every route answers 404.

use std::collections::BTreeMap;
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
use id_worlds::program_gallery_operator::SignedCollection;
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
        .route("/api/gallery/authors/{author}", get(author_handler))
        .route("/api/gallery/collections", get(collections_handler))
        .route(
            "/api/gallery/collections/{operator}/{name}",
            get(collection_handler),
        )
}

/// A list or search answer.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordsAnswer {
    /// The signed records, each verifiable without its payload.
    pub records: Vec<BundleRecord>,
    /// For a search, the documented result order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// The operator's own tags for these records, by content hash: operator
    /// speech, never part of a signed record (G2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operator_tags: BTreeMap<String, Vec<String>>,
}

/// An author page.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorAnswer {
    /// The author's key: the page's address.
    pub author: String,
    /// Every record that key published here.
    pub records: Vec<BundleRecord>,
    /// The operator's own tags for this author (G2): operator speech.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operator_tags: Vec<String>,
}

/// A featured collection with its attribution.
#[derive(Debug, Serialize, Deserialize)]
pub struct FeaturedAnswer {
    /// The key of the operator who signed and features the collection.
    pub featured_by: String,
    /// The collection exactly as the operator signed it.
    pub collection: SignedCollection,
}

/// Every featured collection.
#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionsAnswer {
    /// The collections, ordered by operator key then name.
    pub collections: Vec<FeaturedAnswer>,
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
    let answer = match gallery.read() {
        Ok(gallery) => {
            let records = gallery.list(&query);
            RecordsAnswer {
                operator_tags: gallery.overlay().tags_for(&records),
                records,
                order: None,
            }
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    listing(&answer)
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
        Ok(gallery) => gallery.search(&query).map(|records| RecordsAnswer {
            operator_tags: gallery.overlay().tags_for(&records),
            records,
            order: Some(SEARCH_ORDER.to_owned()),
        }),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match found {
        Ok(answer) => listing(&answer),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn author_handler(
    State(gallery): State<Option<SharedGallery>>,
    Path(author): Path<String>,
) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let page = match gallery.read() {
        Ok(gallery) => gallery.author_page(&author),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match page {
        Ok(page) => listing(&AuthorAnswer {
            author: page.author,
            records: page.records,
            operator_tags: page.operator_tags,
        }),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

async fn collections_handler(State(gallery): State<Option<SharedGallery>>) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let collections = match gallery.read() {
        Ok(gallery) => gallery.collections().filter_map(featured).collect(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    listing(&CollectionsAnswer { collections })
}

async fn collection_handler(
    State(gallery): State<Option<SharedGallery>>,
    Path((operator, name)): Path<(String, String)>,
) -> Response {
    let Some(gallery) = gallery else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let found = match gallery.read() {
        Ok(gallery) => gallery.collection(&operator, &name).and_then(featured),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    found.map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        |answer| listing(&answer),
    )
}

/// A stored collection with the attribution its own signature proves.
fn featured(signed: &SignedCollection) -> Option<FeaturedAnswer> {
    let collection = signed.verify().ok()?;
    Some(FeaturedAnswer {
        featured_by: collection.operator,
        collection: signed.clone(),
    })
}

fn listing(answer: &impl Serialize) -> Response {
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

    #[tokio::test]
    async fn author_pages_collections_and_operator_tags_are_served_apart() {
        let author = SigningKey::from_bytes(&[7; 32]);
        let author_key = id_worlds::world_session::encode_hex(&author.verifying_key().to_bytes());
        let operator = SigningKey::from_bytes(&[9; 32]);
        let operator_key =
            id_worlds::world_session::encode_hex(&operator.verifying_key().to_bytes());
        let mut gallery = Gallery::new(Registry::in_memory(AdmitAll));
        let chess = bundle("chess", 1, "The classic board game.", &["board"]);
        gallery.publish(&chess.to_bytes()).unwrap();
        let picks = SignedCollection::sign(
            "staff-picks",
            "Staff picks",
            1,
            vec![chess.identity()],
            &operator,
        )
        .unwrap();
        gallery.feature(&picks).unwrap();
        let mut overlay = id_worlds::program_gallery_operator::OperatorOverlay::default();
        overlay.programs.insert(
            chess.manifest().content_hash.clone(),
            vec!["miscategorised".to_owned()],
        );
        overlay
            .authors
            .insert(author_key.clone(), vec!["verified".to_owned()]);
        gallery.set_overlay(overlay).unwrap();
        let app = app(Some(Arc::new(RwLock::new(gallery))));

        let (status, _, body) = get(&app, &format!("/api/gallery/authors/{author_key}")).await;
        assert_eq!(status, StatusCode::OK);
        let page: AuthorAnswer = serde_json::from_slice(&body).unwrap();
        assert_eq!(page.author, author_key);
        assert_eq!(page.records, vec![chess.record()]);
        assert_eq!(page.operator_tags, ["verified"]);
        let (status, _, _) = get(&app, "/api/gallery/authors/not-a-key").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, _, body) = get(&app, "/api/gallery/records").await;
        let answer: RecordsAnswer = serde_json::from_slice(&body).unwrap();
        let hash = &chess.manifest().content_hash;
        assert_eq!(answer.operator_tags[hash], ["miscategorised"]);
        assert_eq!(
            answer.records[0].verify().unwrap().categories,
            ["board"],
            "the signed categories are untouched by the overlay"
        );

        let (_, _, body) = get(&app, "/api/gallery/collections").await;
        let all: CollectionsAnswer = serde_json::from_slice(&body).unwrap();
        assert_eq!(all.collections.len(), 1);
        assert_eq!(all.collections[0].featured_by, operator_key);
        assert_eq!(all.collections[0].collection, picks);
        let (status, _, body) = get(
            &app,
            &format!("/api/gallery/collections/{operator_key}/staff-picks"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let one: FeaturedAnswer = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            one.collection.verify().unwrap().items,
            vec![chess.identity()]
        );
        let (status, _, _) = get(
            &app,
            &format!("/api/gallery/collections/{author_key}/staff-picks"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "attribution is by signer");
    }
}
