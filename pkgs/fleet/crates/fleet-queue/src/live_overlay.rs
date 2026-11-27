//! Broker live overlay for fleet hosts (#14715, transport &
//! pooling design §1.2, work item 1b).
//!
//! The registry file (`doc/fleet-machine-registry.yaml`) is the
//! static source of truth for a host: its `id`, its transport
//! offers, its enrollment. Liveness is runtime state and must
//! never be written back into that file — a registry that carries
//! liveness becomes a lie the moment the runtime dies. The live
//! overlay therefore lives here, on the broker side (the queue
//! broker already tracks seat liveness), keyed by the registry
//! `id` and, within a host, by the offer's registry `name`.
//!
//! Three rules from the design are load-bearing and are pinned
//! by this module's tests:
//!
//! - **Unread load is unknown, never zero** (#13611). A report
//!   that carries no load reading stores `None`, and every read
//!   path surfaces that as unknown — placement on a blind zero
//!   piles work onto a loaded host.
//! - **Stale entries age out.** An offer whose last report is
//!   older than [`LIVE_OVERLAY_TTL_SECONDS`] is absent from the
//!   selection read path, not present-and-healthy: silence is a
//!   state, and it is never the healthy one.
//! - **The read path is shaped for selection (1c, #14716).**
//!   [`LiveOverlay::selection_view`] returns only fresh offers,
//!   in offer-name order, with load still optional — the
//!   selection code consumes it without re-deriving freshness
//!   or load semantics.
//!
//! This module is the store-and-serve core only: pure, in-memory,
//! no I/O. Wiring it to the broker's wire surface is a follow-up
//! slice; the store's keying and semantics are the contract that
//! wiring will expose.

use std::collections::BTreeMap;

/// How long an offer report stays fresh in the overlay. The
/// watchdog's registry health cache uses twice its 30-minute
/// cadence; broker liveness is a much hotter signal (the broker
/// sees seat traffic continuously), so the overlay ages entries
/// out after two minutes of silence.
pub const LIVE_OVERLAY_TTL_SECONDS: u64 = 120;

/// One liveness report for one offer of one host, as recorded
/// by [`LiveOverlay::record`]. `seen_at` is a Unix timestamp in
/// seconds, supplied by the caller so the store stays pure and
/// tests control time.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferReport {
    /// The offer's registry `name` (e.g. `commander-door`).
    pub offer: String,
    /// When the offer was last seen, Unix seconds.
    pub seen_at: u64,
    /// The host's load at report time, when a reading exists.
    /// `None` is unknown — never read it as zero (#13611).
    pub load: Option<f64>,
    /// Per-offer health at report time, when a check ran.
    /// `None` is unmeasured, not healthy.
    pub healthy: Option<bool>,
}

/// One fresh offer as the selection code (1c) consumes it.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferLiveness {
    /// The offer's registry `name`.
    pub offer: String,
    /// When the offer was last seen, Unix seconds.
    pub last_seen: u64,
    /// Load at last report; `None` is unknown, never zero.
    pub load: Option<f64>,
    /// Health at last report; `None` is unmeasured.
    pub healthy: Option<bool>,
}

impl OfferLiveness {
    /// The load as selection may display or log it: an unread
    /// load is the string `"unknown"`, never `"0"`.
    pub fn load_label(&self) -> String {
        match self.load {
            Some(load) => load.to_string(),
            None => "unknown".to_string(),
        }
    }
}

/// The broker's live overlay: per-host, per-offer liveness,
/// keyed by registry host `id`, then by offer `name`. Nothing
/// here is persisted to the registry file.
#[derive(Clone, Debug, Default)]
pub struct LiveOverlay {
    hosts: BTreeMap<String, BTreeMap<String, OfferLiveness>>,
}

impl LiveOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store one report. A later report for the same
    /// (host, offer) replaces the earlier one wholesale — the
    /// overlay is last-write-wins runtime state, not a log.
    pub fn record(&mut self, host_id: &str, report: OfferReport) {
        let entry = OfferLiveness {
            offer: report.offer.clone(),
            last_seen: report.seen_at,
            load: report.load,
            healthy: report.healthy,
        };
        self.hosts
            .entry(host_id.to_string())
            .or_default()
            .insert(report.offer, entry);
    }

    /// The read path for selection (1c, #14716): the host's
    /// fresh offers at `now`, in offer-name order. Stale offers
    /// are absent — they have aged out, and an absent offer can
    /// never be mistaken for a healthy one. An unknown host, or
    /// a host whose offers have all aged out, yields an empty
    /// vec.
    pub fn selection_view(&self, host_id: &str, now: u64) -> Vec<OfferLiveness> {
        self.selection_view_within(host_id, now, LIVE_OVERLAY_TTL_SECONDS)
    }

    /// [`LiveOverlay::selection_view`] with an explicit TTL,
    /// for callers (and tests) that age entries on a different
    /// horizon.
    pub fn selection_view_within(
        &self,
        host_id: &str,
        now: u64,
        ttl_seconds: u64,
    ) -> Vec<OfferLiveness> {
        let Some(offers) = self.hosts.get(host_id) else {
            return Vec::new();
        };
        offers
            .values()
            .filter(|offer| now.saturating_sub(offer.last_seen) <= ttl_seconds)
            .cloned()
            .collect()
    }

    /// Drop every entry older than the TTL at `now`, across all
    /// hosts, so aged-out state does not accumulate in the
    /// broker. Hosts left with no offers are dropped too.
    /// Returns how many offer entries were pruned.
    pub fn prune(&mut self, now: u64) -> usize {
        self.prune_within(now, LIVE_OVERLAY_TTL_SECONDS)
    }

    /// [`LiveOverlay::prune`] with an explicit TTL.
    pub fn prune_within(&mut self, now: u64, ttl_seconds: u64) -> usize {
        let mut pruned = 0;
        self.hosts.retain(|_, offers| {
            let before = offers.len();
            offers.retain(|_, offer| now.saturating_sub(offer.last_seen) <= ttl_seconds);
            pruned += before - offers.len();
            !offers.is_empty()
        });
        pruned
    }

    /// How many hosts currently hold any entry (fresh or not).
    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }
}

#[cfg(test)]
mod tests {
    // Oracle: the transport & pooling design §1.2 (broker live
    // overlay keyed by registry `id`, never in the registry file)
    // and #13611 (unread load is unknown, never zero).
    use super::*;

    fn report(offer: &str, seen_at: u64, load: Option<f64>, healthy: Option<bool>) -> OfferReport {
        OfferReport {
            offer: offer.to_string(),
            seen_at,
            load,
            healthy,
        }
    }

    #[test]
    fn overlay_is_keyed_by_registry_id_and_offer_name() {
        let mut overlay = LiveOverlay::new();
        overlay.record("amd-6", report("ssh", 1_000, Some(0.4), Some(true)));
        overlay.record("amd-6", report("commander-door", 1_000, None, Some(true)));
        overlay.record("other-host", report("ssh", 1_000, Some(0.1), Some(false)));

        let view = overlay.selection_view("amd-6", 1_000);
        assert_eq!(view.len(), 2);
        // Offer-name order, so selection sees a stable shape.
        assert_eq!(view[0].offer, "commander-door");
        assert_eq!(view[1].offer, "ssh");
        // Another host's offers never leak into this host's view.
        let other = overlay.selection_view("other-host", 1_000);
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].healthy, Some(false));
        assert!(overlay.selection_view("never-reported", 1_000).is_empty());
    }

    #[test]
    fn unread_load_is_unknown_never_zero() {
        let mut overlay = LiveOverlay::new();
        overlay.record("amd-6", report("ssh", 1_000, None, None));

        let view = overlay.selection_view("amd-6", 1_000);
        assert_eq!(view[0].load, None);
        assert_eq!(view[0].load_label(), "unknown");
        // Unmeasured health is likewise not promoted to healthy.
        assert_eq!(view[0].healthy, None);
    }

    #[test]
    fn stale_offers_age_out_of_the_selection_view() {
        let mut overlay = LiveOverlay::new();
        overlay.record("amd-6", report("ssh", 1_000, Some(0.4), Some(true)));
        overlay.record(
            "amd-6",
            report(
                "commander-door",
                1_000 + LIVE_OVERLAY_TTL_SECONDS,
                None,
                Some(true),
            ),
        );

        // One TTL past ssh's report: ssh is absent, not healthy;
        // the freshly reported offer remains.
        let now = 1_000 + LIVE_OVERLAY_TTL_SECONDS + 1;
        let view = overlay.selection_view("amd-6", now);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].offer, "commander-door");
    }

    #[test]
    fn a_later_report_replaces_the_earlier_one() {
        let mut overlay = LiveOverlay::new();
        overlay.record("amd-6", report("ssh", 1_000, Some(0.9), Some(false)));
        overlay.record("amd-6", report("ssh", 1_050, Some(0.2), Some(true)));

        let view = overlay.selection_view("amd-6", 1_050);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].last_seen, 1_050);
        assert_eq!(view[0].load, Some(0.2));
        assert_eq!(view[0].healthy, Some(true));
    }

    #[test]
    fn prune_drops_stale_entries_and_empty_hosts() {
        let mut overlay = LiveOverlay::new();
        overlay.record("amd-6", report("ssh", 1_000, None, None));
        overlay.record("gone-host", report("ssh", 1_000, None, None));

        let now = 1_000 + LIVE_OVERLAY_TTL_SECONDS + 1;
        assert_eq!(overlay.prune(now), 2);
        assert_eq!(overlay.host_count(), 0);
        assert!(overlay.selection_view("amd-6", now).is_empty());
    }
}
