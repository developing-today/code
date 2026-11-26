//! Client-side discovery settings: registry subscriptions, visibility
//! scopes, and the per-client relay-required flag.
//!
//! From the discovery/registry/federation design (doc
//! `2026-11-16_design_id6287_discovery_registry_federation_v1.md`
//! sections 5-7; implementation breakdown item 5/5, #10709).
//!
//! This is the settings layer only. It owns no network and does not
//! resolve anything: the resolver (breakdown item 4/5, #10708) reads
//! these settings when it lands. What this module guarantees today,
//! with tests:
//!
//! - A fresh self-host follows no registry until its operator chooses
//!   one ([`ClientDiscoveryConfig::default`]); our hosted instance
//!   ships subscribed to our curated registry
//!   ([`ClientDiscoveryConfig::hosted_with_curated`]).
//! - A private world (no announcement) is undiscoverable by every
//!   layer: [`pkarr_publishable`] and [`registry_listable`] both
//!   return false for `None`, and [`Visibility::Unlisted`] never
//!   enters a registry (enforced in [`crate::registry`]).
//! - Relay-required is per-client: [`direct_connections_allowed`]
//!   is false exactly when the flag is set, and
//!   [`RELAY_REQUIRED_DISCLOSURE`] states the honest UI disclosure
//!   (direct connections expose the joiner's IP to the peer).

use crate::announcement::Visibility;

/// Honest UI disclosure for relay-required mode (design section 7):
/// direct (hole-punched) connections expose the joiner's IP to the
/// peer; a client in relay-required mode connects only via relay.
pub const RELAY_REQUIRED_DISCLOSURE: &str =
    "Relay-required: connects only via relay. Direct connections would expose your IP to the peer.";

/// Client-side discovery settings for the world join flow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientDiscoveryConfig {
    /// Registry keys this client follows, in the order the operator
    /// added them. Empty means invite links only.
    pub subscribed_registries: Vec<[u8; 32]>,
    /// When true, the client must never open a direct connection.
    pub relay_required: bool,
}

impl ClientDiscoveryConfig {
    /// A fresh self-host: no registry subscription, relay not required.
    #[must_use]
    pub fn fresh_self_host() -> Self {
        Self::default()
    }

    /// Our hosted instance default: subscribed to the curated registry.
    #[must_use]
    pub fn hosted_with_curated(curated_registry_key: [u8; 32]) -> Self {
        Self {
            subscribed_registries: vec![curated_registry_key],
            relay_required: false,
        }
    }

    /// Follow one more registry. Duplicate subscriptions are ignored.
    pub fn subscribe(&mut self, registry_key: [u8; 32]) {
        if !self.subscribed_registries.contains(&registry_key) {
            self.subscribed_registries.push(registry_key);
        }
    }

    /// Stop following a registry. Unknown keys are a no-op.
    pub fn unsubscribe(&mut self, registry_key: &[u8; 32]) {
        self.subscribed_registries.retain(|key| key != registry_key);
    }

    /// Whether this client follows `registry_key`.
    #[must_use]
    pub fn is_subscribed(&self, registry_key: &[u8; 32]) -> bool {
        self.subscribed_registries
            .iter()
            .any(|key| key == registry_key)
    }
}

/// Whether the world at `visibility` is published to pkarr for direct
/// lookup by key. `None` is a private world: no announcement exists,
/// so nothing is published anywhere. `Unlisted` and `Listed` both
/// publish to pkarr (design section 6).
#[must_use]
pub const fn pkarr_publishable(visibility: Option<Visibility>) -> bool {
    match visibility {
        None => false,
        Some(Visibility::Unlisted | Visibility::Listed) => true,
    }
}

/// Whether the world at `visibility` may be submitted to a registry.
/// Only `Listed` worlds are listable; `Unlisted` worlds are refused
/// at inclusion and `None` (private) has no announcement to submit
/// (design section 6; enforced in [`crate::registry::Registry::include`]).
#[must_use]
pub const fn registry_listable(visibility: Option<Visibility>) -> bool {
    matches!(visibility, Some(Visibility::Listed))
}

/// Whether this client may open a direct (hole-punched) connection.
/// Relay-required mode never opens one: the peer sees only the relay.
#[must_use]
pub const fn direct_connections_allowed(relay_required: bool) -> bool {
    !relay_required
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn fresh_self_host_follows_no_registry() {
        let config = ClientDiscoveryConfig::fresh_self_host();
        assert!(config.subscribed_registries.is_empty());
        assert!(!config.relay_required);
        assert_eq!(config, ClientDiscoveryConfig::default());
    }

    #[test]
    fn hosted_default_subscribes_to_curated_only() {
        let curated = [7_u8; 32];
        let config = ClientDiscoveryConfig::hosted_with_curated(curated);
        assert_eq!(config.subscribed_registries, vec![curated]);
        assert!(config.is_subscribed(&curated));
        assert!(!config.is_subscribed(&[8_u8; 32]));
    }

    #[test]
    fn subscribe_and_unsubscribe_round_trip() {
        let mut config = ClientDiscoveryConfig::fresh_self_host();
        let key = [9_u8; 32];
        config.subscribe(key);
        config.subscribe(key);
        assert_eq!(config.subscribed_registries, vec![key]);
        config.unsubscribe(&key);
        assert!(config.subscribed_registries.is_empty());
        config.unsubscribe(&key);
    }

    #[test]
    fn private_world_is_undiscoverable_by_every_layer() {
        assert!(!pkarr_publishable(None));
        assert!(!registry_listable(None));
        assert!(
            !pkarr_publishable(Some(Visibility::Unlisted))
                || !registry_listable(Some(Visibility::Unlisted))
        );
        assert!(!registry_listable(Some(Visibility::Unlisted)));
        assert!(pkarr_publishable(Some(Visibility::Listed)));
        assert!(registry_listable(Some(Visibility::Listed)));
    }

    #[test]
    fn relay_required_never_opens_direct() {
        assert!(direct_connections_allowed(false));
        assert!(!direct_connections_allowed(true));
        assert!(RELAY_REQUIRED_DISCLOSURE.contains("expose your IP"));
    }
}
