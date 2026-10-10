//! A durable queue of friend envelopes waiting to be pushed to the recipient's
//! server.
//!
//! Each change is one JSON line, synced before it counts. The file is replayed
//! on open, so an envelope that was not finished survives a restart. A line cut
//! short by a crash was never acknowledged, so it is dropped on open.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode, Url, header::CONTENT_TYPE, redirect};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::directory::Envelope;

/// Deliveries attempted before an envelope is given up.
pub const MAX_ATTEMPTS: u32 = 12;
const BASE_DELAY_MS: u64 = 60_000;
const MAX_DELAY_MS: u64 = 6 * 60 * 60 * 1000;
const TIMEOUT: Duration = Duration::from_secs(30);

/// How a queued envelope ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// The recipient's server accepted it.
    Delivered,
    /// The recipient's server refused it, so it is not retried.
    Refused,
    /// Every attempt failed.
    GaveUp,
}

/// The result of one delivery attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The envelope was delivered or was already applied there.
    Delivered,
    /// The envelope will not be accepted, and retrying cannot change that.
    Refused(String),
    /// The attempt failed in a way a later attempt may fix.
    Retry(String),
}

/// One queued envelope and what has happened to it so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The `/envelope` endpoint on the recipient's server.
    pub url: String,
    /// The envelope to deliver.
    pub envelope: Envelope,
    /// Attempts made so far.
    pub attempts: u32,
    /// The earliest time the next attempt may run, in Unix milliseconds.
    pub next_at: u64,
    /// The reason for the last failed attempt or the final outcome.
    pub last_error: Option<String>,
    /// Set once the envelope is no longer queued.
    pub finish: Option<Finish>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Record {
    Queued {
        url: String,
        envelope: Envelope,
    },
    Attempted {
        id: String,
        attempts: u32,
        next_at: u64,
        error: String,
    },
    Finished {
        id: String,
        attempts: u32,
        outcome: Finish,
        detail: Option<String>,
    },
    Withdrawn {
        id: String,
    },
}

/// The queue, keyed by envelope ID.
#[derive(Debug)]
pub struct Outbox {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl Outbox {
    /// Opens the queue at `path`, replaying every record already in it.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, or a complete line is corrupt.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let mut outbox = Self {
            path: path.into(),
            entries: BTreeMap::new(),
        };
        let text = match fs::read_to_string(&outbox.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(outbox),
            Err(error) => return Err(error).context("reading the envelope outbox"),
        };
        let mut kept = 0;
        for (number, line) in text.split_inclusive('\n').enumerate() {
            let complete = line.ends_with('\n');
            match serde_json::from_str::<Record>(line.trim_end()) {
                Ok(record) => {
                    outbox.apply(record)?;
                    kept += line.len();
                }
                Err(_) if !complete => break,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("envelope outbox line {} is corrupt", number + 1)
                    });
                }
            }
        }
        if kept < text.len() {
            OpenOptions::new()
                .write(true)
                .open(&outbox.path)?
                .set_len(kept as u64)?;
        }
        Ok(outbox)
    }

    /// Queues `envelope` for delivery to `url`, due immediately.
    ///
    /// # Errors
    ///
    /// Fails if the URL is not allowed, the envelope is already queued, or the
    /// record cannot be written.
    pub fn enqueue(&mut self, url: &str, envelope: Envelope) -> Result<()> {
        check_url(url)?;
        ensure!(
            !self.entries.contains_key(&envelope.id),
            "envelope {} is already queued",
            envelope.id
        );
        self.commit(Record::Queued {
            url: url.to_owned(),
            envelope,
        })
    }

    /// The IDs of envelopes that are queued and due at `now`.
    #[must_use]
    pub fn due(&self, now: u64) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.finish.is_none() && entry.next_at <= now)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The queued envelope with `id`, finished or not.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Removes an envelope that was queued but never attempted, as if it had
    /// never been queued.
    ///
    /// # Errors
    ///
    /// Fails if `id` is not queued, has been attempted, or the record cannot be
    /// written.
    pub fn withdraw(&mut self, id: &str) -> Result<()> {
        let entry = self.entries.get(id).context("envelope is not queued")?;
        ensure!(
            entry.attempts == 0 && entry.finish.is_none(),
            "envelope {id} was already attempted"
        );
        self.commit(Record::Withdrawn { id: id.to_owned() })
    }

    /// Records the outcome of an attempt made at `now`.
    ///
    /// # Errors
    ///
    /// Fails if `id` is not queued, or the record cannot be written.
    pub fn record(&mut self, id: &str, now: u64, outcome: Outcome) -> Result<()> {
        let entry = self.entries.get(id).context("envelope is not queued")?;
        ensure!(entry.finish.is_none(), "envelope {id} already finished");
        let attempts = entry.attempts + 1;
        let record = match outcome {
            Outcome::Delivered => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::Delivered,
                detail: None,
            },
            Outcome::Refused(error) => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::Refused,
                detail: Some(error),
            },
            Outcome::Retry(error) if attempts >= MAX_ATTEMPTS => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::GaveUp,
                detail: Some(error),
            },
            Outcome::Retry(error) => Record::Attempted {
                id: id.to_owned(),
                attempts,
                next_at: now.saturating_add(backoff_ms(attempts)),
                error,
            },
        };
        self.commit(record)
    }

    /// Every queued envelope, finished or not, in ID order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    fn commit(&mut self, record: Record) -> Result<()> {
        let mut line = serde_json::to_string(&record)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.sync_data()?;
        self.apply(record)
    }

    fn apply(&mut self, record: Record) -> Result<()> {
        match record {
            Record::Queued { url, envelope } => {
                ensure!(
                    !self.entries.contains_key(&envelope.id),
                    "envelope {} is queued twice",
                    envelope.id
                );
                let id = envelope.id.clone();
                self.entries.insert(
                    id,
                    Entry {
                        url,
                        envelope,
                        attempts: 0,
                        next_at: 0,
                        last_error: None,
                        finish: None,
                    },
                );
            }
            Record::Attempted {
                id,
                attempts,
                next_at,
                error,
            } => {
                let entry = self.entry_mut(&id)?;
                entry.attempts = attempts;
                entry.next_at = next_at;
                entry.last_error = Some(error);
            }
            Record::Finished {
                id,
                attempts,
                outcome,
                detail,
            } => {
                let entry = self.entry_mut(&id)?;
                entry.attempts = attempts;
                entry.finish = Some(outcome);
                entry.last_error = detail;
            }
            Record::Withdrawn { id } => {
                ensure!(
                    self.entries.remove(&id).is_some(),
                    "outbox record withdraws unknown envelope {id}"
                );
            }
        }
        Ok(())
    }

    fn entry_mut(&mut self, id: &str) -> Result<&mut Entry> {
        self.entries
            .get_mut(id)
            .with_context(|| format!("outbox record names unknown envelope {id}"))
    }
}

/// Delivers every envelope due at `now`, recording each outcome as it happens.
///
/// The lock is released while a request is in flight, so enqueueing is not
/// held up by a slow recipient. Returns how many attempts were made.
///
/// # Errors
///
/// Fails if an outcome cannot be written. Envelopes already recorded stay
/// recorded.
pub async fn flush(outbox: &Mutex<Outbox>, client: &Client, now: u64) -> Result<usize> {
    let due: Vec<(String, String, Envelope)> = {
        let queue = outbox.lock().await;
        queue
            .due(now)
            .into_iter()
            .filter_map(|id| {
                let entry = queue.entry(&id)?;
                Some((id.clone(), entry.url.clone(), entry.envelope.clone()))
            })
            .collect()
    };
    for (id, url, envelope) in &due {
        let outcome = deliver(client, url, envelope).await;
        outbox.lock().await.record(id, now, outcome)?;
    }
    Ok(due.len())
}

/// Accepts `https` URLs, and `http` only for a loopback host.
///
/// # Errors
///
/// Fails for anything else, so an envelope is never sent in the clear to
/// another machine.
pub fn check_url(url: &str) -> Result<()> {
    let parsed = Url::parse(url).context("outbox URL is not a URL")?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed
                .host_str()
                .unwrap_or_default()
                .trim_start_matches('[')
                .trim_end_matches(']');
            ensure!(
                host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
                "outbox URL must use https unless it is a loopback address"
            );
            Ok(())
        }
        other => bail!("outbox URL scheme {other:?} is not supported"),
    }
}

/// An HTTP client for delivery: bounded in time, and no redirects followed.
///
/// # Errors
///
/// Fails if the TLS backend cannot be initialised.
pub fn http_client() -> Result<Client> {
    // reqwest's rustls backend needs a process-wide provider. If one is already
    // installed this fails harmlessly, and the installed one is used.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    Client::builder()
        .timeout(TIMEOUT)
        .redirect(redirect::Policy::none())
        .build()
        .context("building the envelope delivery client")
}

/// Posts `envelope` as JSON to the `/envelope` endpoint at `url`.
pub async fn deliver(client: &Client, url: &str, envelope: &Envelope) -> Outcome {
    let json = match serde_json::to_string(envelope) {
        Ok(json) => json,
        Err(error) => return Outcome::Refused(format!("envelope does not encode: {error}")),
    };
    match client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .body(json)
        .send()
        .await
    {
        Ok(response) => classify(response.status()),
        Err(error) => Outcome::Retry(format!("send failed: {error}")),
    }
}

fn classify(status: StatusCode) -> Outcome {
    if status.is_success() {
        Outcome::Delivered
    } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        Outcome::Retry(format!("server answered {status}"))
    } else {
        Outcome::Refused(format!("server refused with {status}"))
    }
}

fn backoff_ms(attempts: u32) -> u64 {
    let shift = attempts.saturating_sub(1).min(20);
    (BASE_DELAY_MS << shift).min(MAX_DELAY_MS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::directory::EnvelopeKind;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const NOW: u64 = 1_000_000_000;
    const MINUTE: u64 = 60_000;

    fn envelope(id: &str) -> Envelope {
        Envelope {
            kind: EnvelopeKind::FriendRequest,
            from: "ann".to_owned(),
            to: "bo".to_owned(),
            audience: "club".to_owned(),
            id: id.to_owned(),
            request: None,
            at: NOW,
            signature: "00".to_owned(),
        }
    }

    fn path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("outbox.jsonl")
    }

    /// Accepts one connection, answers `status`, and returns what it read.
    async fn stub(status: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/envelope", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buf).into_owned();
                if let Some(head) = text.find("\r\n\r\n") {
                    let length = text[..head]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .map(str::to_owned)
                        })
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= head + 4 + length {
                        break;
                    }
                }
            }
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn queued_envelopes_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox
            .enqueue("https://bo.example/envelope", envelope("e1"))
            .unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(reopened.due(NOW), vec!["e1".to_owned()]);
        assert_eq!(reopened.entry("e1").unwrap().envelope, envelope("e1"));
    }

    #[test]
    fn a_retry_waits_for_its_backoff_and_then_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox
            .enqueue("https://bo.example/envelope", envelope("e1"))
            .unwrap();

        outbox
            .record("e1", NOW, Outcome::Retry("down".to_owned()))
            .unwrap();
        let entry = outbox.entry("e1").unwrap();
        assert_eq!((entry.attempts, entry.next_at), (1, NOW + MINUTE));
        assert!(outbox.due(NOW + MINUTE - 1).is_empty());
        assert_eq!(outbox.due(NOW + MINUTE), vec!["e1".to_owned()]);

        for attempt in 2..=MAX_ATTEMPTS {
            outbox
                .record("e1", NOW, Outcome::Retry("down".to_owned()))
                .unwrap();
            assert_eq!(outbox.entry("e1").unwrap().attempts, attempt);
        }
        let entry = outbox.entry("e1").unwrap();
        assert_eq!(entry.finish, Some(Finish::GaveUp));
        assert!(outbox.due(u64::MAX).is_empty());

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(reopened.entry("e1").unwrap().finish, Some(Finish::GaveUp));
    }

    #[test]
    fn a_refusal_is_final() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox
            .enqueue("https://bo.example/envelope", envelope("e1"))
            .unwrap();
        outbox
            .record("e1", NOW, Outcome::Refused("bad signature".to_owned()))
            .unwrap();
        let entry = outbox.entry("e1").unwrap();
        assert_eq!(entry.finish, Some(Finish::Refused));
        assert_eq!(entry.last_error.as_deref(), Some("bad signature"));
        assert!(outbox.due(u64::MAX).is_empty());
        assert!(outbox.record("e1", NOW, Outcome::Delivered).is_err());
    }

    #[test]
    fn a_withdrawn_envelope_is_gone_and_stays_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox
            .enqueue("https://bo.example/envelope", envelope("e1"))
            .unwrap();
        outbox.withdraw("e1").unwrap();
        assert!(outbox.entry("e1").is_none());
        assert!(outbox.due(u64::MAX).is_empty());
        assert!(outbox.withdraw("e1").is_err());
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_none());
    }

    #[test]
    fn a_torn_last_line_is_dropped_and_not_mixed_into_the_next_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox
            .enqueue("https://bo.example/envelope", envelope("e1"))
            .unwrap();
        drop(outbox);
        let mut file = OpenOptions::new().append(true).open(path(&dir)).unwrap();
        file.write_all(b"{\"op\":\"queued\",\"url\"").unwrap();
        drop(file);

        let mut outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        outbox
            .enqueue("https://bo.example/envelope", envelope("e2"))
            .unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_some() && reopened.entry("e2").is_some());
    }

    #[test]
    fn a_corrupt_complete_line_stops_the_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(path(&dir), "not json\n").unwrap();
        assert!(Outbox::open(path(&dir)).is_err());
    }

    #[test]
    fn only_https_or_loopback_http_urls_are_accepted() {
        assert!(check_url("https://bo.example/envelope").is_ok());
        assert!(check_url("http://127.0.0.1:9/envelope").is_ok());
        assert!(check_url("http://localhost:9/envelope").is_ok());
        assert!(check_url("http://[::1]:9/envelope").is_ok());
        assert!(check_url("http://bo.example/envelope").is_err());
        assert!(check_url("ftp://bo.example/envelope").is_err());
        assert!(check_url("not a url").is_err());
    }

    #[tokio::test]
    async fn delivery_posts_the_envelope_as_json_to_the_envelope_route() {
        let client = http_client().unwrap();
        let (url, request) = stub("200 OK").await;
        let outcome = deliver(&client, &url, &envelope("e1")).await;
        assert_eq!(outcome, Outcome::Delivered);
        let received = request.await.unwrap();
        assert!(received.starts_with("POST /envelope "));
        assert!(received.contains("content-type: application/json"));
        let body = &received[received.find("\r\n\r\n").unwrap() + 4..];
        assert_eq!(
            serde_json::from_str::<Envelope>(body).unwrap(),
            envelope("e1")
        );
    }

    #[tokio::test]
    async fn delivery_classifies_responses() {
        let client = http_client().unwrap();

        let (url, request) = stub("503 Service Unavailable").await;
        assert!(matches!(
            deliver(&client, &url, &envelope("e1")).await,
            Outcome::Retry(_)
        ));
        request.await.unwrap();

        let (url, request) = stub("429 Too Many Requests").await;
        assert!(matches!(
            deliver(&client, &url, &envelope("e1")).await,
            Outcome::Retry(_)
        ));
        request.await.unwrap();

        let (url, request) = stub("401 Unauthorized").await;
        assert!(matches!(
            deliver(&client, &url, &envelope("e1")).await,
            Outcome::Refused(_)
        ));
        request.await.unwrap();

        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/envelope", closed.local_addr().unwrap());
        drop(closed);
        assert!(matches!(
            deliver(&client, &url, &envelope("e1")).await,
            Outcome::Retry(_)
        ));
    }

    #[tokio::test]
    async fn flush_delivers_due_envelopes_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let client = http_client().unwrap();
        let (ok_url, ok_request) = stub("200 OK").await;
        let (busy_url, busy_request) = stub("503 Service Unavailable").await;

        let outbox = Mutex::new(Outbox::open(path(&dir)).unwrap());
        outbox
            .lock()
            .await
            .enqueue(&ok_url, envelope("e1"))
            .unwrap();
        outbox
            .lock()
            .await
            .enqueue(&busy_url, envelope("e2"))
            .unwrap();

        assert_eq!(flush(&outbox, &client, NOW).await.unwrap(), 2);
        ok_request.await.unwrap();
        busy_request.await.unwrap();
        let queue = outbox.lock().await;
        assert_eq!(queue.entry("e1").unwrap().finish, Some(Finish::Delivered));
        assert_eq!(queue.entry("e2").unwrap().next_at, NOW + MINUTE);
        assert!(queue.due(NOW).is_empty());
        drop(queue);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(
            reopened.entry("e1").unwrap().finish,
            Some(Finish::Delivered)
        );
        assert_eq!(reopened.due(NOW + MINUTE), vec!["e2".to_owned()]);
    }

    #[tokio::test]
    async fn an_envelope_queued_for_a_closed_world_is_delivered_when_it_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let (url, request) = stub("200 OK").await;
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&url, envelope("e1")).unwrap();
        drop(outbox);

        let (_service, _) = crate::world_store::open_world(
            dir.path(),
            "club",
            crate::world::WorldLimits::default(),
            crate::world_limits::RuntimeLimits::default(),
            |handle| crate::world_session::WorldService::new(handle, None),
        )
        .await
        .unwrap();
        assert!(request.await.unwrap().starts_with("POST /envelope "));

        let mut recorded = false;
        for _ in 0..200 {
            recorded = fs::read_to_string(path(&dir))
                .is_ok_and(|text| text.contains("\"outcome\":\"delivered\""));
            if recorded {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(recorded, "the delivery was not recorded");
    }
}
