//! The pkarr relay sinks for world announcements (#10706, addendum
//! doc/2026-11-25T03-10-00Z_design_id10706_announcement_pkarr
//! section 3).
//!
//! An operator configures a list of pkarr relay base URLs. Publishing
//! is `PUT <base>/<z32 world key>` with the relay payload, and a
//! publish succeeds when at least one relay accepts it. Fetching is
//! `GET <base>/<z32>`; every answer goes through
//! [`announcement_pkarr::resolve`], and the newest valid announcement
//! wins. The transport is the [`BridgeHttp`] seam, so tests run
//! against an in-memory relay.

use anyhow::{Result, anyhow, bail, ensure};

use crate::announcement::SignedAnnouncement;
use crate::announcement_pkarr::{self, PkarrItem, z32_encode};
use crate::bridge_http::{BridgeHttp, HttpRequest};

/// Our relay's base URL: the default on our hosted instance and the
/// suggested default for a self-host.
pub const DEVELOPING_TODAY_RELAY: &str = "https://pkarr.developing-today.workers.dev/pkarr";

/// What one relay said to one publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SinkOutcome {
    /// The relay stored the item (any 2xx).
    Accepted,
    /// The relay holds a newer item for this key (409): another
    /// publisher holds the key. Reported, not retried.
    Stale,
    /// The relay refused with another status. Retried at the next
    /// heartbeat.
    Refused {
        /// The HTTP status code.
        status: u16,
        /// The relay's body, as text.
        message: String,
    },
    /// The relay could not be reached. Retried at the next heartbeat.
    Unreachable {
        /// The transport error.
        error: String,
    },
}

/// Every relay's answer to one publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishReport {
    /// One entry per configured relay, in configuration order.
    pub outcomes: Vec<(String, SinkOutcome)>,
}

impl PublishReport {
    /// A publish succeeds when at least one relay accepted it.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.outcomes
            .iter()
            .any(|(_, outcome)| *outcome == SinkOutcome::Accepted)
    }

    /// Whether any relay said another publisher holds the key.
    #[must_use]
    pub fn contested(&self) -> bool {
        self.outcomes
            .iter()
            .any(|(_, outcome)| *outcome == SinkOutcome::Stale)
    }
}

/// The operator's relay list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelaySinks {
    bases: Vec<String>,
}

impl RelaySinks {
    /// A sink list. Each base must be an `http` or `https` URL with
    /// no query or fragment; a trailing `/` is dropped. An empty list
    /// is valid and publishes nowhere.
    ///
    /// # Errors
    ///
    /// Fails on a base that is not such a URL, or a duplicate.
    pub fn new<I, S>(bases: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut out: Vec<String> = Vec::new();
        for base in bases {
            let base: String = base.into();
            let trimmed = base.trim_end_matches('/');
            let rest = trimmed
                .strip_prefix("https://")
                .or_else(|| trimmed.strip_prefix("http://"));
            let Some(rest) = rest else {
                bail!("pkarr relay {base} is not an http or https URL");
            };
            ensure!(
                !rest.is_empty() && !rest.starts_with('/'),
                "pkarr relay {base} names no host"
            );
            ensure!(
                !trimmed.contains(['?', '#']) && !trimmed.chars().any(char::is_whitespace),
                "pkarr relay {base} carries a query, fragment or space"
            );
            ensure!(
                !out.iter().any(|existing| existing == trimmed),
                "pkarr relay {base} is listed twice"
            );
            out.push(trimmed.to_owned());
        }
        Ok(Self { bases: out })
    }

    /// The configured bases, in order.
    #[must_use]
    pub fn bases(&self) -> &[String] {
        &self.bases
    }

    fn url(base: &str, world_key: &[u8; 32]) -> String {
        format!("{base}/{}", z32_encode(world_key))
    }

    /// Send `item` to every relay.
    pub async fn publish<H: BridgeHttp>(
        &self,
        http: &mut H,
        world_key: &[u8; 32],
        item: &PkarrItem,
    ) -> PublishReport {
        let body = item.to_relay_payload();
        let mut outcomes = Vec::with_capacity(self.bases.len());
        for base in &self.bases {
            let request = HttpRequest {
                method: "PUT".to_owned(),
                url: Self::url(base, world_key),
                headers: vec![(
                    "content-type".to_owned(),
                    "application/pkarr.org/relays#payload".to_owned(),
                )],
                body: body.clone(),
            };
            let outcome = match http.request(request).await {
                Ok(response) if (200..300).contains(&response.status) => SinkOutcome::Accepted,
                Ok(response) if response.status == 409 => SinkOutcome::Stale,
                Ok(response) => SinkOutcome::Refused {
                    status: response.status,
                    message: String::from_utf8_lossy(&response.body).into_owned(),
                },
                Err(error) => SinkOutcome::Unreachable {
                    error: format!("{error:#}"),
                },
            };
            outcomes.push((base.clone(), outcome));
        }
        PublishReport { outcomes }
    }

    /// Fetch the raw relay payload for `key` from every relay, in
    /// configuration order. A relay that is unreachable or answers
    /// anything but 200 gives an error in its slot. Nothing is verified
    /// here: each record family's resolver checks its own payloads.
    pub async fn fetch_payloads<H: BridgeHttp>(
        &self,
        http: &mut H,
        key: &[u8; 32],
    ) -> Vec<(String, Result<Vec<u8>>)> {
        let mut out = Vec::with_capacity(self.bases.len());
        for base in &self.bases {
            let request = HttpRequest {
                method: "GET".to_owned(),
                url: Self::url(base, key),
                headers: Vec::new(),
                body: Vec::new(),
            };
            let payload = match http.request(request).await {
                Ok(response) if response.status == 200 => Ok(response.body),
                Ok(response) => Err(anyhow!("HTTP {}", response.status)),
                Err(error) => Err(error),
            };
            out.push((base.clone(), payload));
        }
        out
    }

    /// Fetch `world_key` from every relay and return the valid
    /// announcement with the highest `seq` at `now` (Unix seconds).
    /// A relay that is unreachable, has nothing, or serves something
    /// that does not resolve is skipped.
    ///
    /// # Errors
    ///
    /// Fails when no relay served a valid announcement.
    pub async fn fetch<H: BridgeHttp>(
        &self,
        http: &mut H,
        world_key: &[u8; 32],
        now: u64,
    ) -> Result<SignedAnnouncement> {
        let mut best: Option<SignedAnnouncement> = None;
        let mut reasons = Vec::new();
        for (base, payload) in self.fetch_payloads(http, world_key).await {
            let resolved =
                payload.and_then(|body| announcement_pkarr::resolve(world_key, &body, now));
            match resolved {
                Ok(signed) => {
                    if best
                        .as_ref()
                        .is_none_or(|current| signed.announcement.seq > current.announcement.seq)
                    {
                        best = Some(signed);
                    }
                }
                Err(error) => reasons.push(format!("{base}: {error:#}")),
            }
        }
        match best {
            Some(signed) => Ok(signed),
            None if reasons.is_empty() => bail!("no pkarr relay is configured"),
            None => bail!(
                "no pkarr relay served a valid announcement: {}",
                reasons.join("; ")
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::HashMap;

    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::announcement::{Announcement, Occupancy, Subject, Visibility};
    use crate::announcement_pkarr::Publisher;
    use crate::bridge_http::HttpResponse;

    const NOW_US: u64 = 1_763_500_000_000_000;
    const NOW: u64 = 1_763_500_000;

    /// An in-memory pkarr relay with the pkarr-worker's rules:
    /// verify, then newest-wins (here reporting stale as 409, as the
    /// reference relay does), plus scripted failures.
    #[derive(Default)]
    struct FakeRelays {
        stored: HashMap<String, (u64, Vec<u8>)>,
        down: Vec<String>,
        refusing: Vec<String>,
        requests: Vec<HttpRequest>,
    }

    impl BridgeHttp for FakeRelays {
        async fn request(&mut self, request: HttpRequest) -> Result<HttpResponse> {
            self.requests.push(request.clone());
            let base = request.url.rsplit_once('/').unwrap().0.to_owned();
            if self.down.contains(&base) {
                bail!("connection refused");
            }
            let respond = |status: u16, body: Vec<u8>| HttpResponse {
                status,
                headers: Vec::new(),
                body,
            };
            if self.refusing.contains(&base) {
                return Ok(respond(503, b"storage failure".to_vec()));
            }
            match request.method.as_str() {
                "PUT" => {
                    let item = PkarrItem::from_relay_payload(&request.body)?;
                    if let Some((stored, _)) = self.stored.get(&request.url) {
                        if *stored >= item.timestamp_us {
                            return Ok(respond(409, b"stale".to_vec()));
                        }
                    }
                    self.stored
                        .insert(request.url.clone(), (item.timestamp_us, request.body));
                    Ok(respond(204, Vec::new()))
                }
                "GET" => Ok(match self.stored.get(&request.url) {
                    Some((_, body)) => respond(200, body.clone()),
                    None => respond(404, b"not found".to_vec()),
                }),
                other => bail!("unexpected method {other}"),
            }
        }
    }

    fn world() -> (SigningKey, Announcement) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let fields = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "harbor".to_owned(),
            description: String::new(),
            subject: Subject::Endpoint {
                node_id: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
                relay: None,
            },
            occupancy: Occupancy::HasRoom,
            tags: Vec::new(),
            visibility: Visibility::Unlisted,
            issued_at: 0,
            expires_at: 1,
            seq: 0,
        };
        (key, fields)
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn bases_are_checked_and_normalised() {
        let sinks = RelaySinks::new([
            DEVELOPING_TODAY_RELAY.to_owned() + "/",
            "http://127.0.0.1:8080".to_owned(),
        ])
        .unwrap();
        assert_eq!(
            sinks.bases(),
            [DEVELOPING_TODAY_RELAY, "http://127.0.0.1:8080"]
        );
        assert!(
            RelaySinks::new(Vec::<String>::new())
                .unwrap()
                .bases()
                .is_empty()
        );
        assert!(RelaySinks::new(["relay.example.net"]).is_err());
        assert!(RelaySinks::new(["https://"]).is_err());
        assert!(RelaySinks::new(["https://relay.example.net/?k=1"]).is_err());
        assert!(RelaySinks::new(["https://a.example", "https://a.example/"]).is_err());
    }

    #[test]
    fn a_publish_round_trips_through_a_relay() {
        let (key, fields) = world();
        let world_key = key.verifying_key().to_bytes();
        let mut publisher = Publisher::new(key, 3600).unwrap();
        let item = publisher.publish(&fields, NOW_US).unwrap();
        let sinks = RelaySinks::new(["https://relay.example.net/pkarr"]).unwrap();
        let mut relays = FakeRelays::default();
        let report = block_on(sinks.publish(&mut relays, &world_key, &item));
        assert!(report.succeeded());
        assert_eq!(
            relays.requests[0].url,
            format!("https://relay.example.net/pkarr/{}", z32_encode(&world_key))
        );
        let fetched = block_on(sinks.fetch(&mut relays, &world_key, NOW + 60)).unwrap();
        assert_eq!(fetched.announcement.seq, item.timestamp_us);
    }

    #[test]
    fn one_accepting_relay_is_enough_and_failures_are_reported() {
        let (key, fields) = world();
        let world_key = key.verifying_key().to_bytes();
        let mut publisher = Publisher::new(key, 3600).unwrap();
        let item = publisher.publish(&fields, NOW_US).unwrap();
        let sinks = RelaySinks::new([
            "https://a.example",
            "https://b.example",
            "https://c.example",
        ])
        .unwrap();
        let mut relays = FakeRelays {
            down: vec!["https://a.example".to_owned()],
            refusing: vec!["https://b.example".to_owned()],
            ..FakeRelays::default()
        };
        let report = block_on(sinks.publish(&mut relays, &world_key, &item));
        assert!(report.succeeded());
        assert!(matches!(
            report.outcomes[0].1,
            SinkOutcome::Unreachable { .. }
        ));
        assert!(matches!(
            report.outcomes[1].1,
            SinkOutcome::Refused { status: 503, .. }
        ));
        assert_eq!(report.outcomes[2].1, SinkOutcome::Accepted);

        relays.refusing.clear();
        relays.down.clear();
        relays.stored.clear();
        let none = RelaySinks::new(Vec::<String>::new()).unwrap();
        assert!(!block_on(none.publish(&mut relays, &world_key, &item)).succeeded());
        assert!(block_on(none.fetch(&mut relays, &world_key, NOW)).is_err());
    }

    #[test]
    fn a_newer_item_elsewhere_reports_the_key_contested() {
        let (key, fields) = world();
        let world_key = key.verifying_key().to_bytes();
        let mut publisher = Publisher::new(key.clone(), 3600).unwrap();
        let sinks = RelaySinks::new(["https://a.example"]).unwrap();
        let mut relays = FakeRelays::default();
        let newer = Publisher::new(key, 3600)
            .unwrap()
            .publish(&fields, NOW_US + 10)
            .unwrap();
        assert!(block_on(sinks.publish(&mut relays, &world_key, &newer)).succeeded());
        let older = publisher.publish(&fields, NOW_US).unwrap();
        let report = block_on(sinks.publish(&mut relays, &world_key, &older));
        assert!(!report.succeeded());
        assert!(report.contested());
    }

    #[test]
    fn fetch_takes_the_newest_valid_answer_and_skips_bad_ones() {
        let (key, fields) = world();
        let world_key = key.verifying_key().to_bytes();
        let mut publisher = Publisher::new(key, 3600).unwrap();
        let first = publisher.publish(&fields, NOW_US).unwrap();
        let second = publisher.publish(&fields, NOW_US + 1_000_000).unwrap();
        let sinks = RelaySinks::new([
            "https://a.example",
            "https://b.example",
            "https://c.example",
        ])
        .unwrap();
        let mut relays = FakeRelays::default();
        let url = |base: &str| format!("{base}/{}", z32_encode(&world_key));
        relays.stored.insert(
            url("https://a.example"),
            (first.timestamp_us, first.to_relay_payload()),
        );
        relays.stored.insert(
            url("https://b.example"),
            (second.timestamp_us, second.to_relay_payload()),
        );
        let mut corrupt = second.to_relay_payload();
        *corrupt.last_mut().unwrap() ^= 1;
        relays
            .stored
            .insert(url("https://c.example"), (second.timestamp_us + 1, corrupt));
        let fetched = block_on(sinks.fetch(&mut relays, &world_key, NOW + 60)).unwrap();
        assert_eq!(fetched.announcement.seq, second.timestamp_us);
    }
}
