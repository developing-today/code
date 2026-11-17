//! F13.7 step 4 — client grant and local audit adapter for the native
//! shell (design `doc/2026-11-01_design_id789.md` §4 and §7 step 4).
//!
//! The host broker (step 3, `id-worlds` `provider_broker`) is the
//! policy point for what a guest may *request*; this module is the
//! local authority for what this shell will *do*. The two checks are
//! independent by design: a host "allow" cannot bypass the local
//! check here, and a local grant alone never causes the host to route
//! a request. Concretely, this module provides:
//!
//! - **Per-world local opt-in.** A provider is usable only where the
//!   local user has granted it for that world. The default is no
//!   grants anywhere: nothing is offered, and every request is
//!   refused. Grant-on-use is never performed — a request for an
//!   ungranted provider is a refusal, not a prompt or an implicit
//!   grant.
//! - **Local bounds.** Each grant carries bounds (`max_bytes`,
//!   `per_minute`, `in_flight`) that compose by minimum with the
//!   manifest and world bounds on the host side; locally they are
//!   enforced on every request, and an over-bound result is refused
//!   whole (`result_too_large`), never silently truncated.
//! - **Immediate revoke.** Revoking a grant removes it from the
//!   offer set at once and makes the next request for that
//!   world/provider refuse with `provider_revoked`; no new execution
//!   starts under a revoked grant.
//! - **A local audit entry per decision.** Every request handled
//!   here — executed or refused — appends one user-readable entry to
//!   the local audit log (world, provider, nonce, outcome, refusal
//!   code). This is the client-local log the design separates from
//!   the host F12.6 record: it proves the local check happened, and
//!   it never carries argument values or result bytes.
//!
//! The adapter seam is [`ClientProviderExecutor`]: step 5 registers
//! the first real executor (`client.file.read`); step 4 registers
//! none in the shipped shell, so a granted-but-unimplemented provider
//! refuses `provider_unavailable` and an ungranted one refuses
//! `provider_not_granted` — no provider is enabled by default, and a
//! malicious or mistaken host request still meets the local check.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

/// Refusal: the local user has not opted this provider in for this
/// world (and grant-on-use never applies).
pub const REFUSAL_NOT_GRANTED: &str = "provider_not_granted";
/// Refusal: the provider was opted in for this world and the local
/// user has since revoked the grant.
pub const REFUSAL_REVOKED: &str = "provider_revoked";
/// Refusal: the provider is granted but this shell has no executor
/// registered for it (no provider is enabled by default).
pub const REFUSAL_UNAVAILABLE: &str = "provider_unavailable";
/// Refusal: the grant's per-minute local bound is exhausted.
pub const REFUSAL_RATE_LIMITED: &str = "provider_rate_limited";
/// Refusal: the grant's in-flight local bound is exhausted.
pub const REFUSAL_BUSY: &str = "provider_busy";
/// Refusal: the request arrived with no time left to answer it.
pub const REFUSAL_EXPIRED: &str = "provider_timeout";
/// Refusal: the executor produced more than the local byte bound;
/// the result is refused whole, never truncated.
pub const REFUSAL_TOO_LARGE: &str = "result_too_large";
/// Refusal: the request frame lacks a usable nonce or provider name.
pub const REFUSAL_MALFORMED: &str = "provider_malformed_request";

/// Rate-limit window for `per_minute` bounds, in milliseconds.
const RATE_WINDOW_MS: u64 = 60_000;
/// Maximum local audit entries retained; oldest dropped first. The
/// local log is a session record, not a durable store.
pub const MAX_AUDIT_ENTRIES: usize = 512;

/// Local bounds on one granted provider, set by the local user at
/// grant time. They compose by minimum with the manifest and world
/// bounds the host enforces; the local values here are the ones this
/// shell will never exceed regardless of what a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalBounds {
    /// Maximum result bytes one execution may return.
    pub max_bytes: u32,
    /// Maximum executions started per minute.
    pub per_minute: u32,
    /// Maximum executions in flight at once.
    pub in_flight: u32,
}

impl LocalBounds {
    /// The design's starting bounds for the first slice
    /// (`client.file.read`: 32 KiB, ten reads/minute, two in flight).
    pub const DESIGN_DEFAULT: Self = Self {
        max_bytes: 32_768,
        per_minute: 10,
        in_flight: 2,
    };

    /// The effective bound of two bound sets: the minimum of each
    /// field, per the design's compose-by-minimum rule.
    #[must_use]
    pub fn compose_minimum(self, other: Self) -> Self {
        Self {
            max_bytes: self.max_bytes.min(other.max_bytes),
            per_minute: self.per_minute.min(other.per_minute),
            in_flight: self.in_flight.min(other.in_flight),
        }
    }
}

/// One local provider execution. Implementors hold the actual local
/// resource (step 5: a user-selected file root); the policy in this
/// module decides whether `execute` is reached at all.
///
/// `Err` carries a stable refusal code that is returned to the host
/// verbatim and recorded in the local audit entry — never a message
/// containing argument values, paths, or content.
pub trait ClientProviderExecutor: Send {
    /// Execute one request whose local checks have passed.
    ///
    /// # Errors
    ///
    /// Returns a stable refusal code when the resource refuses.
    fn execute(&mut self, args: &Value, max_bytes: u32) -> Result<Vec<u8>, String>;
}

/// One user-readable local audit entry: the record that the local
/// adapter executed or refused a provider request. Deliberately
/// carries no argument values, local paths, or result bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAuditEntry {
    /// When the decision was made (milliseconds, caller's clock).
    pub at_ms: u64,
    /// The world the request arrived for.
    pub world: String,
    /// The provider named by the request.
    pub provider: String,
    /// The host-minted nonce the request carried.
    pub nonce: String,
    /// `executed` or `refused`.
    pub outcome: &'static str,
    /// The stable refusal code when `outcome` is `refused`.
    pub reason: Option<String>,
}

/// Mutable per-grant enforcement state.
#[derive(Clone, Copy, Debug)]
struct GrantState {
    bounds: LocalBounds,
    window_start_ms: u64,
    window_count: u32,
    in_flight: u32,
}

/// The shell-side provider adapter: local grants, executors, and the
/// local audit log for one shell process. Owned by a single session
/// task; all state transitions are immediate on the owning task.
#[derive(Default)]
pub struct ProviderAdapter {
    grants: HashMap<(String, String), GrantState>,
    revoked: HashSet<(String, String)>,
    executors: HashMap<String, Box<dyn ClientProviderExecutor>>,
    audit: Vec<LocalAuditEntry>,
}

impl std::fmt::Debug for ProviderAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAdapter")
            .field("grants", &self.grants.len())
            .field("executors", &self.executors.keys().collect::<Vec<_>>())
            .field("audit_entries", &self.audit.len())
            .finish_non_exhaustive()
    }
}

impl ProviderAdapter {
    /// A new adapter with no grants and no executors: the shipped
    /// default, in which nothing is offered and every request is
    /// refused by the local check.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the executor that serves one provider name. A
    /// provider with an executor but no grant for a world is still
    /// never offered or executed for that world.
    pub fn register_executor(&mut self, provider: &str, executor: Box<dyn ClientProviderExecutor>) {
        self.executors.insert(provider.to_owned(), executor);
    }

    /// Opt `provider` in for `world` with the given local bounds.
    /// This is the local user's grant; it is per-world, and it is
    /// the only way a provider becomes offered or executable.
    pub fn grant(&mut self, world: &str, provider: &str, bounds: LocalBounds) {
        let key = (world.to_owned(), provider.to_owned());
        self.revoked.remove(&key);
        self.grants.insert(
            key,
            GrantState {
                bounds,
                window_start_ms: 0,
                window_count: 0,
                in_flight: 0,
            },
        );
    }

    /// Revoke a grant, effective immediately: the provider leaves
    /// the world's offer set and the next request for it refuses
    /// with [`REFUSAL_REVOKED`]. Returns whether a grant existed.
    pub fn revoke(&mut self, world: &str, provider: &str) -> bool {
        let key = (world.to_owned(), provider.to_owned());
        let existed = self.grants.remove(&key).is_some();
        if existed {
            self.revoked.insert(key);
        }
        existed
    }

    /// Whether `provider` is currently granted for `world`.
    #[must_use]
    pub fn is_granted(&self, world: &str, provider: &str) -> bool {
        self.grants
            .contains_key(&(world.to_owned(), provider.to_owned()))
    }

    /// The providers this shell may advertise for `world`: granted
    /// for that world *and* backed by a registered executor, sorted
    /// for a stable offer frame.
    #[must_use]
    pub fn offered_providers(&self, world: &str) -> Vec<String> {
        let mut offered: Vec<String> = self
            .grants
            .keys()
            .filter(|(grant_world, provider)| {
                grant_world == world && self.executors.contains_key(provider)
            })
            .map(|(_, provider)| provider.clone())
            .collect();
        offered.sort();
        offered.dedup();
        offered
    }

    /// The `provider_offer` client frame for `world`, or `None`
    /// when nothing is offered (the default: no frame is sent).
    #[must_use]
    pub fn offer_frame(&self, world: &str) -> Option<Value> {
        let providers = self.offered_providers(world);
        if providers.is_empty() {
            None
        } else {
            Some(json!({"type": "provider_offer", "providers": providers}))
        }
    }

    /// The local audit entries recorded so far, oldest first.
    #[must_use]
    pub fn audit_entries(&self) -> &[LocalAuditEntry] {
        &self.audit
    }

    fn record_audit(&mut self, entry: LocalAuditEntry) {
        self.audit.push(entry);
        if self.audit.len() > MAX_AUDIT_ENTRIES {
            let excess = self.audit.len() - MAX_AUDIT_ENTRIES;
            self.audit.drain(..excess);
        }
    }

    fn refuse(
        &mut self,
        at_ms: u64,
        world: &str,
        provider: &str,
        nonce: &str,
        code: &str,
    ) -> Value {
        self.record_audit(LocalAuditEntry {
            at_ms,
            world: world.to_owned(),
            provider: provider.to_owned(),
            nonce: nonce.to_owned(),
            outcome: "refused",
            reason: Some(code.to_owned()),
        });
        json!({
            "type": "provider_reply",
            "nonce": nonce,
            "ok": false,
            "error": code,
        })
    }

    /// Handle one `provider_request` server frame for `world` and
    /// return the `provider_reply` client frame to send back. Every
    /// path — malformed, ungranted, revoked, over a local bound,
    /// executor refusal, oversize result, success — records exactly
    /// one local audit entry, and no path executes a provider the
    /// local user has not granted for this world.
    pub fn handle_request(&mut self, world: &str, frame: &Value, now_ms: u64) -> Value {
        let nonce = frame["nonce"].as_str().unwrap_or_default().to_owned();
        let provider = frame["provider"].as_str().unwrap_or_default().to_owned();
        if nonce.is_empty() || provider.is_empty() {
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_MALFORMED);
        }
        if frame["deadline_ms"].as_u64().unwrap_or_default() == 0 {
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_EXPIRED);
        }
        let key = (world.to_owned(), provider.clone());
        let Some(mut state) = self.grants.get(&key).copied() else {
            let code = if self.revoked.contains(&key) {
                REFUSAL_REVOKED
            } else {
                REFUSAL_NOT_GRANTED
            };
            return self.refuse(now_ms, world, &provider, &nonce, code);
        };
        if now_ms.saturating_sub(state.window_start_ms) >= RATE_WINDOW_MS {
            state.window_start_ms = now_ms;
            state.window_count = 0;
        }
        if state.window_count >= state.bounds.per_minute {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_RATE_LIMITED);
        }
        if state.in_flight >= state.bounds.in_flight {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_BUSY);
        }
        if !self.executors.contains_key(&provider) {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_UNAVAILABLE);
        }
        state.window_count += 1;
        state.in_flight += 1;
        self.grants.insert(key.clone(), state);
        let args = frame["args"].clone();
        let result = match self.executors.get_mut(&provider) {
            Some(executor) => executor.execute(&args, state.bounds.max_bytes),
            None => Err(REFUSAL_UNAVAILABLE.to_owned()),
        };
        if let Some(current) = self.grants.get_mut(&key) {
            current.in_flight = current.in_flight.saturating_sub(1);
        }
        match result {
            Ok(bytes) if bytes.len() <= state.bounds.max_bytes as usize => {
                self.record_audit(LocalAuditEntry {
                    at_ms: now_ms,
                    world: world.to_owned(),
                    provider: provider.clone(),
                    nonce: nonce.clone(),
                    outcome: "executed",
                    reason: None,
                });
                json!({
                    "type": "provider_reply",
                    "nonce": nonce,
                    "ok": true,
                    "result_hex": hex_encode(&bytes),
                })
            }
            Ok(_) => self.refuse(now_ms, world, &provider, &nonce, REFUSAL_TOO_LARGE),
            Err(code) => self.refuse(now_ms, world, &provider, &nonce, &code),
        }
    }
}

/// Lowercase hex encoding for reply result bytes (the contract's
/// `result_hex` field).
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct FakeExecutor(Vec<u8>);

    impl ClientProviderExecutor for FakeExecutor {
        fn execute(&mut self, _args: &Value, _max_bytes: u32) -> Result<Vec<u8>, String> {
            Ok(self.0.clone())
        }
    }

    struct RefusingExecutor;

    impl ClientProviderExecutor for RefusingExecutor {
        fn execute(&mut self, _args: &Value, _max_bytes: u32) -> Result<Vec<u8>, String> {
            Err("provider_local_refusal".to_owned())
        }
    }

    fn request(nonce: &str, provider: &str) -> Value {
        json!({
            "type": "provider_request",
            "nonce": nonce,
            "provider": provider,
            "args": {"path": "notes.txt"},
            "deadline_ms": 5000,
        })
    }

    fn granted_adapter() -> ProviderAdapter {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![0x00, 0xff])));
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        adapter
    }

    #[test]
    fn the_default_adapter_offers_nothing_and_refuses_everything() {
        let mut adapter = ProviderAdapter::new();
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], REFUSAL_NOT_GRANTED);
        assert_eq!(adapter.audit_entries().len(), 1);
        assert_eq!(adapter.audit_entries()[0].outcome, "refused");
    }

    #[test]
    fn a_granted_provider_executes_and_the_reply_is_hex_bounded_bytes() {
        let mut adapter = granted_adapter();
        assert_eq!(
            adapter.offer_frame("world-a"),
            Some(json!({"type": "provider_offer", "providers": ["client.file.read"]}))
        );
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result_hex"], "00ff");
        assert_eq!(adapter.audit_entries()[0].outcome, "executed");
        assert_eq!(adapter.audit_entries()[0].reason, None);
    }

    #[test]
    fn grants_are_per_world() {
        let mut adapter = granted_adapter();
        let reply = adapter.handle_request("world-b", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_NOT_GRANTED);
        assert_eq!(adapter.offer_frame("world-b"), None);
    }

    #[test]
    fn revoke_is_immediate_for_offers_and_requests() {
        let mut adapter = granted_adapter();
        assert!(adapter.revoke("world-a", "client.file.read"));
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_REVOKED);
        // Re-granting clears the revoked marker.
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 2_000);
        assert_eq!(reply["ok"], true);
    }

    #[test]
    fn a_grant_without_an_executor_is_unavailable_and_never_offered() {
        let mut adapter = ProviderAdapter::new();
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_UNAVAILABLE);
    }

    #[test]
    fn the_per_minute_bound_refuses_and_then_the_window_resets() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![1])));
        adapter.grant(
            "world-a",
            "client.file.read",
            LocalBounds {
                per_minute: 1,
                ..LocalBounds::DESIGN_DEFAULT
            },
        );
        let first = adapter.handle_request("world-a", &request("01", "client.file.read"), 1_000);
        assert_eq!(first["ok"], true);
        let second = adapter.handle_request("world-a", &request("02", "client.file.read"), 2_000);
        assert_eq!(second["error"], REFUSAL_RATE_LIMITED);
        let third = adapter.handle_request("world-a", &request("03", "client.file.read"), 62_000);
        assert_eq!(third["ok"], true);
    }

    #[test]
    fn an_oversize_result_is_refused_whole_never_truncated() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![7; 8])));
        adapter.grant(
            "world-a",
            "client.file.read",
            LocalBounds {
                max_bytes: 4,
                ..LocalBounds::DESIGN_DEFAULT
            },
        );
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], REFUSAL_TOO_LARGE);
        assert!(reply.get("result_hex").is_none());
    }

    #[test]
    fn executor_refusals_and_expired_requests_are_audited_with_their_codes() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(RefusingExecutor));
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], "provider_local_refusal");
        let mut expired = request("cd34", "client.file.read");
        expired["deadline_ms"] = json!(0);
        let reply = adapter.handle_request("world-a", &expired, 2_000);
        assert_eq!(reply["error"], REFUSAL_EXPIRED);
        let reasons: Vec<Option<String>> = adapter
            .audit_entries()
            .iter()
            .map(|entry| entry.reason.clone())
            .collect();
        assert_eq!(
            reasons,
            vec![
                Some("provider_local_refusal".to_owned()),
                Some(REFUSAL_EXPIRED.to_owned())
            ]
        );
    }

    #[test]
    fn bounds_compose_by_minimum() {
        let composed = LocalBounds::DESIGN_DEFAULT.compose_minimum(LocalBounds {
            max_bytes: 1024,
            per_minute: 60,
            in_flight: 1,
        });
        assert_eq!(
            composed,
            LocalBounds {
                max_bytes: 1024,
                per_minute: 10,
                in_flight: 1,
            }
        );
    }
}
