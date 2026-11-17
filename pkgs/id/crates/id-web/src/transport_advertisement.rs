//! HTTP publication for signed transport advertisements (#8214).
//!
//! The caller owns transport discovery and supplies the usable paths. This
//! module owns the signed object and exposes a small router that can be merged
//! into any Axum server without coupling publication to `AppState`.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use ed25519_dalek::SigningKey;
use id_core::transport_advertisement::{
    MAX_TTL_SECONDS, SequenceFile, SignedAdvertisement, TransportAdvertisement, TransportEntry,
};
use serde::Deserialize;

/// The decision record recommends a one-hour advertisement lifetime.
pub const DEFAULT_TTL_SECONDS: u64 = 60 * 60;
/// Endpoint path from decision record #5793.
pub const WELL_KNOWN_PATH: &str = ".well-known/id-transport-advertisement";

#[derive(Debug)]
struct Published {
    transports: Vec<TransportEntry>,
    current: Option<(SignedAdvertisement, Vec<u8>)>,
}

/// Shared publisher for one server identity.
///
/// `set_transports` is the transport-change notification seam: a changed set
/// invalidates the current serialized object, and the next request mints a
/// newly signed advertisement with a durable higher sequence. Requests also
/// refresh the advertisement after its TTL.
#[derive(Clone)]
pub struct AdvertisementPublisher {
    signing_key: Arc<SigningKey>,
    sequence: SequenceFile,
    subject: String,
    ttl_seconds: u64,
    published: Arc<Mutex<Published>>,
}

impl std::fmt::Debug for AdvertisementPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvertisementPublisher")
            .field("subject", &self.subject)
            .field("ttl_seconds", &self.ttl_seconds)
            .finish_non_exhaustive()
    }
}

impl AdvertisementPublisher {
    /// Create a publisher using a persistent per-identity sequence file.
    ///
    /// The sequence file's parent directory must already exist. TTL must be
    /// positive and no greater than the protocol's 24-hour maximum.
    pub fn new(
        signing_key: SigningKey,
        sequence_path: impl Into<std::path::PathBuf>,
        transports: Vec<TransportEntry>,
        ttl_seconds: u64,
    ) -> std::io::Result<Self> {
        if ttl_seconds == 0 || ttl_seconds > MAX_TTL_SECONDS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "advertisement TTL must be between 1 second and 24 hours",
            ));
        }
        let subject = TransportAdvertisement::subject_for(&signing_key.verifying_key());
        Ok(Self {
            signing_key: Arc::new(signing_key),
            sequence: SequenceFile::new(sequence_path),
            subject,
            ttl_seconds,
            published: Arc::new(Mutex::new(Published {
                transports,
                current: None,
            })),
        })
    }

    /// The identity fingerprint this publisher signs for.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Replace the published paths after a transport change.
    pub fn set_transports(&self, transports: Vec<TransportEntry>) {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if published.transports != transports {
            published.transports = transports;
            published.current = None;
        }
    }

    fn current_bytes(&self, now: u64) -> std::io::Result<Vec<u8>> {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if published
            .current
            .as_ref()
            .is_none_or(|(signed, _)| signed.advertisement.expires_at <= now)
        {
            let expires_at = now.checked_add(self.ttl_seconds).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "timestamp overflow")
            })?;
            let advertisement = TransportAdvertisement {
                subject: self.subject.clone(),
                transports: published.transports.clone(),
                issued_at: now,
                expires_at,
                sequence: self.sequence.next_sequence()?,
            };
            let signed = advertisement.sign(&self.signing_key);
            let bytes = serde_json::to_vec(&signed).map_err(std::io::Error::other)?;
            published.current = Some((signed, bytes));
        }
        published
            .current
            .as_ref()
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| std::io::Error::other("advertisement was not published"))
    }
}

/// A composable router serving the signed advertisement at its well-known URL.
#[must_use]
pub fn router(publisher: AdvertisementPublisher) -> Router {
    Router::new()
        .route(&format!("/{WELL_KNOWN_PATH}"), get(get_advertisement))
        .with_state(publisher)
}

#[derive(Debug, Deserialize)]
struct AdvertisementQuery {
    subject: String,
}

async fn get_advertisement(
    State(publisher): State<AdvertisementPublisher>,
    Query(query): Query<AdvertisementQuery>,
) -> Response {
    if query.subject != publisher.subject() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    match publisher.current_bytes(now) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(error) => {
            tracing::error!("could not publish transport advertisement: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt as _;

    fn transport(endpoint: &str) -> Vec<TransportEntry> {
        vec![TransportEntry {
            kind: id_core::transport_advertisement::TransportKind::Tcp,
            endpoint: endpoint.to_owned(),
            priority: 30,
            broker: None,
        }]
    }

    fn publisher(dir: &tempfile::TempDir) -> AdvertisementPublisher {
        AdvertisementPublisher::new(
            SigningKey::from_bytes(&[7; 32]),
            dir.path().join("sequence"),
            transport("server.example:443"),
            60,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn publishes_a_signed_ad_for_the_requested_subject_and_reuses_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let publisher = publisher(&dir);
        let subject = publisher.subject().to_owned();
        let app = router(publisher);
        let uri = format!("/{WELL_KNOWN_PATH}?subject={subject}");
        let first = app
            .clone()
            .oneshot(axum::http::Request::get(&uri).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], "application/json");
        let first_bytes = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        let signed: SignedAdvertisement = serde_json::from_slice(&first_bytes).unwrap();
        assert_eq!(signed.advertisement.subject, subject);
        assert_eq!(signed.advertisement.sequence, 1);
        signed.verify(&subject).unwrap();

        let second = app
            .oneshot(axum::http::Request::get(uri).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let second_bytes = to_bytes(second.into_body(), usize::MAX).await.unwrap();
        assert_eq!(first_bytes, second_bytes, "the same ad is served byte-for-byte");
    }

    #[tokio::test]
    async fn refuses_a_different_subject() {
        let dir = tempfile::tempdir().unwrap();
        let app = router(publisher(&dir));
        let response = app
            .oneshot(
                axum::http::Request::get(format!("/{WELL_KNOWN_PATH}?subject=other"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn transport_changes_and_ttl_expiry_mint_higher_sequences() {
        let dir = tempfile::tempdir().unwrap();
        let publisher = publisher(&dir);
        let first: SignedAdvertisement =
            serde_json::from_slice(&publisher.current_bytes(100).unwrap()).unwrap();
        publisher.set_transports(transport("new.example:443"));
        let changed: SignedAdvertisement =
            serde_json::from_slice(&publisher.current_bytes(101).unwrap()).unwrap();
        assert_eq!(changed.advertisement.sequence, first.advertisement.sequence + 1);
        assert_eq!(changed.advertisement.transports[0].endpoint, "new.example:443");
        let expired: SignedAdvertisement =
            serde_json::from_slice(&publisher.current_bytes(161).unwrap()).unwrap();
        assert_eq!(expired.advertisement.sequence, changed.advertisement.sequence + 1);
    }

    #[test]
    fn rejects_invalid_ttl() {
        let dir = tempfile::tempdir().unwrap();
        assert!(AdvertisementPublisher::new(
            SigningKey::from_bytes(&[7; 32]),
            dir.path().join("sequence"),
            Vec::new(),
            MAX_TTL_SECONDS + 1,
        )
        .is_err());
    }
}
