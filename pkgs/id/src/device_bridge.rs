//! MQTT device bridge PoC for `id.device.v1` (#2343).
//!
//! **This module is a labelled proof of concept, not the permanent
//! feature.** It proves the topic contract and the bridge semantics against
//! an in-memory fake transport; it speaks no real MQTT and must not be
//! mistaken for production firmware or a production bridge. The written
//! schema lives in
//! `docs/2026-11-09T05-00-00Z_feature_mqtt_device_bridge_id_device_v1/`.
//!
//! The contract, settled by #5901 and specified by #2343:
//!
//! * One connection per device, one topic prefix per world:
//!   `<prefix>/{manifest,wants,cap/result,surface,events}`.
//! * The manifest is the only retained document. A retained `wants` replays
//!   a stale request, so the bridge rejects it.
//! * Wire bounds match the host protocol: 64 KiB `wants` documents and
//!   events, 8 KiB request args ([`id_core::world_caps`] constants).
//! * Authentication is fail-closed: unknown device or wrong credential
//!   refuses the connection, and nothing is published or delivered.
//! * Firmware stays in the closed subset: `device.info`, small
//!   `screen`/`text`, a closed `form.*` subset, `loc.position` when GNSS is
//!   present, and key/encoder events. A rotary encoder reports as a `Key`
//!   event with a signed `delta` in `args`.
//!
//! [`id_core`]: ../../id_core/index.html

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use id_core::world_caps::{self, DeviceManifest, MAX_ARGS_BYTES, MAX_EVENT_BYTES, MAX_WANTS_BYTES};

/// Protocol marker published alongside every PoC document.
pub const PROTOCOL_VERSION: &str = "id.device.v1";
/// Recommended minimum `time.tick` interval for the device path, in
/// milliseconds: 1 Hz keeps the radio duty cycle low enough to avoid wake
/// storms on ESP32-class hardware (#2343).
pub const MIN_TIME_TICK_MS: u64 = 1_000;
/// First reconnect delay, in milliseconds.
pub const RECONNECT_BASE_MS: u64 = 500;
/// Reconnect delay ceiling, in milliseconds.
pub const RECONNECT_CAP_MS: u64 = 30_000;

/// Reconnect delay for a 0-based attempt: exponential backoff from
/// [`RECONNECT_BASE_MS`] capped at [`RECONNECT_CAP_MS`].
#[must_use]
pub fn backoff_delay_ms(attempt: u32) -> u64 {
    let shift = attempt.min(16);
    RECONNECT_BASE_MS
        .saturating_mul(1_u64 << shift)
        .min(RECONNECT_CAP_MS)
}

/// The five topics of one device under one world prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceTopics {
    /// Retained device manifest, published on connect.
    pub manifest: String,
    /// Host-bound wants document. Never retained.
    pub wants: String,
    /// Bridge-bound capability result or refusal.
    pub result: String,
    /// Host-bound surface document.
    pub surface: String,
    /// Device-bound event stream.
    pub events: String,
}

/// Build the topic set for `prefix` (the per-world prefix).
#[must_use]
pub fn topics(prefix: &str) -> DeviceTopics {
    DeviceTopics {
        manifest: format!("{prefix}/manifest"),
        wants: format!("{prefix}/wants"),
        result: format!("{prefix}/cap/result"),
        surface: format!("{prefix}/surface"),
        events: format!("{prefix}/events"),
    }
}

/// Device credential presented at connect time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    /// Device id; also the last segment of the world prefix.
    pub device_id: String,
    /// Per-device secret, provisioned out of band (#5901: TLS plus
    /// per-device credentials).
    pub token: String,
}

/// Fail-closed authenticator: only provisioned `(device_id, token)` pairs
/// connect. Anything else is refused before any publish or delivery.
#[derive(Clone, Debug, Default)]
pub struct Authenticator {
    provisioned: BTreeMap<String, String>,
}

impl Authenticator {
    /// Empty authenticator; provision devices before connecting.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Provision one device id with its secret.
    pub fn provision(&mut self, device_id: &str, token: &str) {
        self.provisioned
            .insert(device_id.to_owned(), token.to_owned());
    }

    /// Check a credential. The comparison is exact; failure carries no
    /// oracle about which half was wrong.
    ///
    /// # Errors
    ///
    /// Fails closed on unknown devices and wrong tokens.
    pub fn authenticate(&self, credential: &Credential) -> Result<()> {
        match self.provisioned.get(&credential.device_id) {
            Some(expected) if expected == &credential.token => Ok(()),
            _ => bail!("authentication failed for device"),
        }
    }
}

/// Why the bridge refused a request.
///
/// Oversize args never surface here: `parse_wants` rejects an over-bound
/// document before evaluation, the same boundary as the host path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Machine-readable code (`unknown_cap`, `cap_denied`, `failed`).
    pub code: String,
    /// Human-readable reason.
    pub reason: String,
}

/// The bridge verdict for one wants request: a result value or a refusal
/// (`unknown_cap`, `cap_denied`, `failed`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The capability ran; the value is the host result document.
    Result(serde_json::Value),
    /// The capability was refused with web-equivalent semantics.
    Refused(Refusal),
}

/// Host-side bridge: evaluates wants documents against the granted set with
/// the same refusal vocabulary as the web path.
#[derive(Clone, Debug)]
pub struct Bridge {
    allowed: BTreeSet<String>,
}

impl Bridge {
    /// Build a bridge over the granted capability names.
    #[must_use]
    pub fn new(allowed: BTreeSet<String>) -> Self {
        Self { allowed }
    }

    /// Evaluate a `wants` document body, returning one verdict per request
    /// in document order. Subscriptions are acknowledged without verdicts;
    /// unknown capabilities and ungranted names are refused, never run.
    ///
    /// # Errors
    ///
    /// Fails on malformed or over-bound wants documents.
    pub fn evaluate_wants(&self, body: &str) -> Result<Vec<(String, Verdict)>> {
        let wants = world_caps::parse_wants(body).context("bridge rejects malformed wants")?;
        let mut verdicts = Vec::with_capacity(wants.requests.len());
        for request in &wants.requests {
            // Note: `parse_wants` already enforces `MAX_ARGS_BYTES` per
            // request, so oversize args reject the whole document before
            // evaluation — the same boundary as the host path.
            let verdict = if !world_caps::is_known(&request.cap) {
                Verdict::Refused(Refusal {
                    code: "unknown_cap".to_owned(),
                    reason: format!("capability {} is not in the catalog", request.cap),
                })
            } else if !self.allowed.contains(&request.cap) {
                Verdict::Refused(Refusal {
                    code: "cap_denied".to_owned(),
                    reason: format!("no grant for {}", request.cap),
                })
            } else {
                Verdict::Result(serde_json::json!({
                    "id": request.id,
                    "cap": request.cap,
                    "ok": true,
                }))
            };
            verdicts.push((request.id.clone(), verdict));
        }
        Ok(verdicts)
    }
}

/// Error kinds of the fake in-memory transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerError {
    /// Authentication refused the connection.
    AuthDenied,
    /// A `wants` publish set the retained flag: a retained wants would
    /// replay a stale request to the next subscriber.
    RetainedWants,
    /// A document exceeded its wire bound.
    TooLarge(&'static str),
    /// A publish arrived from an unconnected device.
    NotConnected,
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthDenied => write!(f, "authentication denied"),
            Self::RetainedWants => write!(f, "wants must never be retained"),
            Self::TooLarge(which) => write!(f, "{which} exceeds its wire bound"),
            Self::NotConnected => write!(f, "device is not connected"),
        }
    }
}

impl std::error::Error for BrokerError {}

/// In-memory stand-in for the MQTT broker. It models exactly the semantics
/// the PoC needs to prove: retained manifest delivery, the retained-wants
/// ban, wire bounds, and fail-closed auth. A real transport replaces this
/// struct without touching [`Bridge`].
#[derive(Debug, Default)]
pub struct FakeBroker {
    auth: Authenticator,
    connected: BTreeSet<String>,
    /// The retained manifest per world prefix.
    retained_manifests: BTreeMap<String, String>,
    /// Wants bodies the bridge has received, in arrival order.
    pub received_wants: Vec<String>,
}

impl FakeBroker {
    /// Build a broker over an authenticator.
    #[must_use]
    pub fn new(auth: Authenticator) -> Self {
        Self {
            auth,
            connected: BTreeSet::new(),
            retained_manifests: BTreeMap::new(),
            received_wants: Vec::new(),
        }
    }

    /// Connect one device. Fails closed on bad credentials.
    ///
    /// # Errors
    ///
    /// [`BrokerError::AuthDenied`] on unknown device or wrong token.
    pub fn connect(&mut self, credential: &Credential) -> std::result::Result<(), BrokerError> {
        self.auth
            .authenticate(credential)
            .map_err(|_| BrokerError::AuthDenied)?;
        self.connected.insert(credential.device_id.clone());
        Ok(())
    }

    /// Disconnect one device. Journals nothing; the next connect republishes
    /// the manifest, which is the reconnect path the PoC exercises.
    pub fn disconnect(&mut self, device_id: &str) {
        self.connected.remove(device_id);
    }

    fn require_connected(&self, device_id: &str) -> std::result::Result<(), BrokerError> {
        if self.connected.contains(device_id) {
            Ok(())
        } else {
            Err(BrokerError::NotConnected)
        }
    }

    /// Publish a manifest (retained on connect).
    ///
    /// # Errors
    ///
    /// [`BrokerError::NotConnected`] or [`BrokerError::TooLarge`].
    pub fn publish_manifest(
        &mut self,
        device_id: &str,
        prefix: &str,
        manifest: &DeviceManifest,
    ) -> std::result::Result<(), BrokerError> {
        self.require_connected(device_id)?;
        let body = manifest.to_json().to_string();
        ensure_bound(&body, MAX_EVENT_BYTES, "manifest")?;
        self.retained_manifests.insert(prefix.to_owned(), body);
        Ok(())
    }

    /// Read back the retained manifest for a prefix, as a late subscriber
    /// would receive it.
    #[must_use]
    pub fn retained_manifest(&self, prefix: &str) -> Option<&str> {
        self.retained_manifests.get(prefix).map(String::as_str)
    }

    /// Publish a wants document. `retained = true` is refused: retaining a
    /// wants replays a stale request.
    ///
    /// # Errors
    ///
    /// [`BrokerError::RetainedWants`], [`BrokerError::NotConnected`], or
    /// [`BrokerError::TooLarge`].
    pub fn publish_wants(
        &mut self,
        device_id: &str,
        body: &str,
        retained: bool,
    ) -> std::result::Result<(), BrokerError> {
        self.require_connected(device_id)?;
        if retained {
            return Err(BrokerError::RetainedWants);
        }
        ensure_bound(body, MAX_WANTS_BYTES, "wants")?;
        // Bound-check the shape now so the bridge never sees a malformed
        // document; the bridge re-parses authoritatively.
        world_caps::parse_wants(body).map_err(|_| BrokerError::TooLarge("wants"))?;
        self.received_wants.push(body.to_owned());
        Ok(())
    }
}

fn ensure_bound(
    body: &str,
    bound: usize,
    which: &'static str,
) -> std::result::Result<(), BrokerError> {
    if body.len() > bound {
        return Err(BrokerError::TooLarge(which));
    }
    Ok(())
}

/// Fake constrained device: journals locally, enforces the minimum tick,
/// and reconnects with capped exponential backoff.
#[derive(Debug)]
pub struct FakeDevice {
    /// Device id; also used as the credential identity.
    pub device_id: String,
    /// World prefix this device publishes under.
    pub prefix: String,
    manifest: DeviceManifest,
    /// Device-side journal: every locally recorded event, newest last.
    /// Survives reconnect; this is the journaling recommendation in
    /// miniature (#2343).
    pub journal: Vec<String>,
    /// Last tick timestamp, in milliseconds.
    last_tick_ms: Option<u64>,
    /// Consecutive failed connect attempts (drives backoff).
    failed_attempts: u32,
    connected: bool,
}

impl FakeDevice {
    /// Build a fake device with its manifest.
    #[must_use]
    pub fn new(device_id: &str, prefix: &str, manifest: DeviceManifest) -> Self {
        Self {
            device_id: device_id.to_owned(),
            prefix: prefix.to_owned(),
            manifest,
            journal: Vec::new(),
            last_tick_ms: None,
            failed_attempts: 0,
            connected: false,
        }
    }

    /// Connect and publish the retained manifest.
    ///
    /// # Errors
    ///
    /// Propagates broker connect/publish failures; records failures for
    /// backoff.
    pub fn connect(
        &mut self,
        broker: &mut FakeBroker,
        credential: &Credential,
    ) -> std::result::Result<(), BrokerError> {
        match broker.connect(credential) {
            Ok(()) => {
                self.connected = true;
                self.failed_attempts = 0;
                broker.publish_manifest(&self.device_id, &self.prefix, &self.manifest)?;
                self.journal.push("connected".to_owned());
                Ok(())
            }
            Err(error) => {
                self.failed_attempts = self.failed_attempts.saturating_add(1);
                Err(error)
            }
        }
    }

    /// Delay before the next reconnect attempt.
    #[must_use]
    pub fn reconnect_delay_ms(&self) -> u64 {
        backoff_delay_ms(self.failed_attempts)
    }

    /// Record one tick no faster than [`MIN_TIME_TICK_MS`].
    ///
    /// # Errors
    ///
    /// Fails when `now_ms` is sooner than the minimum interval after the
    /// previous tick.
    pub fn tick(&mut self, now_ms: u64) -> Result<()> {
        if let Some(last) = self.last_tick_ms {
            ensure!(
                now_ms.saturating_sub(last) >= MIN_TIME_TICK_MS,
                "tick sooner than minimum {MIN_TIME_TICK_MS}ms interval"
            );
        }
        self.last_tick_ms = Some(now_ms);
        self.journal.push(format!("tick:{now_ms}"));
        Ok(())
    }

    /// Report a rotary-encoder step as a `Key` event with a signed delta.
    #[must_use]
    pub fn encoder_event(&self, key: &str, delta: i32) -> serde_json::Value {
        serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "kind": "Key",
            "key": key,
            "args": {"delta": delta},
        })
    }
}

/// Example manifest for the PoC fake device: pixel screen, keys plus
/// encoder, GNSS, no microphone or camera.
#[must_use]
pub fn example_manifest() -> DeviceManifest {
    DeviceManifest {
        display: serde_json::json!({"kind": "pixel", "width": 128, "height": 64, "colors": 1}),
        input: serde_json::json!({"keys": 8, "encoder": true, "pointer": false, "text_entry": "none"}),
        sensors: serde_json::json!({"gnss": {"accuracy_m": 5}, "microphone": false, "camera": false}),
        audio: serde_json::json!({"out": "pcm", "in": null}),
        network: serde_json::json!({"transport": "mqtt", "rtt_class": "slow"}),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn provisioned() -> (Authenticator, Credential) {
        let mut auth = Authenticator::new();
        auth.provision("device-1", "token-1");
        (
            auth,
            Credential {
                device_id: "device-1".to_owned(),
                token: "token-1".to_owned(),
            },
        )
    }

    #[test]
    fn topics_follow_the_per_world_prefix() {
        let set = topics("worlds/abc/devices/device-1");
        assert_eq!(set.manifest, "worlds/abc/devices/device-1/manifest");
        assert_eq!(set.wants, "worlds/abc/devices/device-1/wants");
        assert_eq!(set.result, "worlds/abc/devices/device-1/cap/result");
        assert_eq!(set.surface, "worlds/abc/devices/device-1/surface");
        assert_eq!(set.events, "worlds/abc/devices/device-1/events");
    }

    #[test]
    fn manifest_is_retained_for_late_subscribers() {
        let (auth, cred) = provisioned();
        let mut broker = FakeBroker::new(auth);
        broker.connect(&cred).unwrap();
        let manifest = example_manifest();
        broker
            .publish_manifest("device-1", "worlds/abc", &manifest)
            .unwrap();
        let retained = broker.retained_manifest("worlds/abc").unwrap();
        let parsed = DeviceManifest::from_json(
            &serde_json::from_str(retained).expect("retained manifest is JSON"),
        )
        .unwrap();
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn full_poc_flow_manifest_wants_result() {
        // Oracle: the #2343 acceptance flow — connect, retained manifest,
        // wants in, result out with web-equivalent refusal vocabulary.
        let (auth, cred) = provisioned();
        let mut broker = FakeBroker::new(auth);
        let mut device = FakeDevice::new("device-1", "worlds/abc", example_manifest());
        device.connect(&mut broker, &cred).unwrap();
        let wants = serde_json::json!({
            "v": 1,
            "subscribe": ["time.tick:1000"],
            "requests": [
                {"id": "r1", "cap": "device.info", "args": {}},
                {"id": "r2", "cap": "bogus.cap", "args": {}},
            ],
        })
        .to_string();
        broker.publish_wants("device-1", &wants, false).unwrap();
        let bridge = Bridge::new(BTreeSet::from(["device.info".to_owned()]));
        let verdicts = bridge.evaluate_wants(&broker.received_wants[0]).unwrap();
        assert_eq!(verdicts.len(), 2);
        assert!(matches!(verdicts[0].1, Verdict::Result(_)));
        match &verdicts[1].1 {
            Verdict::Refused(refusal) => assert_eq!(refusal.code, "unknown_cap"),
            Verdict::Result(_) => panic!("bogus.cap must be refused"),
        }
    }

    #[test]
    fn ungranted_known_cap_is_denied() {
        let bridge = Bridge::new(BTreeSet::new());
        let wants = r#"{"v": 1, "requests": [{"id": "r1", "cap": "device.info", "args": {}}]}"#;
        let verdicts = bridge.evaluate_wants(wants).unwrap();
        match &verdicts[0].1 {
            Verdict::Refused(refusal) => assert_eq!(refusal.code, "cap_denied"),
            Verdict::Result(_) => panic!("ungranted cap must be refused"),
        }
    }

    #[test]
    fn retained_wants_is_rejected() {
        let (auth, cred) = provisioned();
        let mut broker = FakeBroker::new(auth);
        broker.connect(&cred).unwrap();
        let wants = r#"{"v": 1, "requests": []}"#;
        let error = broker.publish_wants("device-1", wants, true).unwrap_err();
        assert_eq!(error, BrokerError::RetainedWants);
        assert!(broker.received_wants.is_empty());
    }

    #[test]
    fn oversize_wants_is_rejected() {
        let (auth, cred) = provisioned();
        let mut broker = FakeBroker::new(auth);
        broker.connect(&cred).unwrap();
        let big = "x".repeat(MAX_WANTS_BYTES + 1);
        let wants = format!(
            r#"{{"v": 1, "requests": [{{"id": "r1", "cap": "device.info", "args": "{big}"}}]}}"#
        );
        // Oversize at publish: the transport refuses before parsing.
        let error = broker.publish_wants("device-1", &wants, false).unwrap_err();
        assert_eq!(error, BrokerError::TooLarge("wants"));
    }

    #[test]
    fn oversize_args_rejects_the_whole_document_like_the_host() {
        // Oracle: the host `parse_wants` boundary — oversize args reject
        // the document at parse time, before any per-request evaluation.
        let bridge = Bridge::new(BTreeSet::from(["device.info".to_owned()]));
        let big = "x".repeat(MAX_ARGS_BYTES + 1);
        let wants = format!(
            r#"{{"v": 1, "requests": [{{"id": "r1", "cap": "device.info", "args": "{big}"}}]}}"#
        );
        assert!(wants.len() <= MAX_WANTS_BYTES);
        let error = bridge.evaluate_wants(&wants).unwrap_err();
        assert!(
            format!("{error:?}").contains("request args exceed"),
            "{error:?}"
        );
    }

    #[test]
    fn auth_fails_closed() {
        let (auth, _) = provisioned();
        let mut broker = FakeBroker::new(auth);
        let wrong = Credential {
            device_id: "device-1".to_owned(),
            token: "wrong".to_owned(),
        };
        assert_eq!(broker.connect(&wrong).unwrap_err(), BrokerError::AuthDenied);
        let unknown = Credential {
            device_id: "nope".to_owned(),
            token: "token-1".to_owned(),
        };
        assert_eq!(
            broker.connect(&unknown).unwrap_err(),
            BrokerError::AuthDenied
        );
        // Nothing was delivered: no retained manifest, no received wants.
        assert!(broker.retained_manifest("worlds/abc").is_none());
        assert!(broker.received_wants.is_empty());
        // And an unauthenticated device cannot publish.
        let manifest = example_manifest();
        assert_eq!(
            broker
                .publish_manifest("device-1", "worlds/abc", &manifest)
                .unwrap_err(),
            BrokerError::NotConnected
        );
    }

    #[test]
    fn tick_enforces_the_minimum_interval() {
        let mut device = FakeDevice::new("device-1", "worlds/abc", example_manifest());
        device.tick(1_000).unwrap();
        assert!(device.tick(1_000 + MIN_TIME_TICK_MS - 1).is_err());
        device.tick(1_000 + MIN_TIME_TICK_MS).unwrap();
        assert_eq!(device.journal.len(), 2);
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_delay_ms(0), RECONNECT_BASE_MS);
        assert_eq!(backoff_delay_ms(1), RECONNECT_BASE_MS * 2);
        assert_eq!(backoff_delay_ms(100), RECONNECT_CAP_MS);
        let mut previous = 0;
        for attempt in 0..10 {
            let delay = backoff_delay_ms(attempt);
            assert!(delay >= previous);
            previous = delay;
        }
    }

    #[test]
    fn encoder_reports_as_key_with_signed_delta() {
        let device = FakeDevice::new("device-1", "worlds/abc", example_manifest());
        let event = device.encoder_event("volume", -3);
        assert_eq!(event["kind"], "Key");
        assert_eq!(event["args"]["delta"], -3);
    }
}
