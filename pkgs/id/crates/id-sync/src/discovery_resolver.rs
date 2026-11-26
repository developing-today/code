//! The discovery resolver (#10708): find a world's freshest valid announcement.
//!
//! It asks the three discovery layers in the order design #6287 §6
//! decides — the invite link, then pkarr by `world_id`, then the
//! registries the reader subscribes to — and keeps the freshest valid
//! announcement across all of them.
//!
//! The order is a fallback order, not an authority order: no layer
//! can override a fresher `seq` from another. A layer only wins a
//! tie, which keeps the most direct source when two layers carry the
//! same announcement. Every candidate is checked on its own: its
//! signature against its own world key, the queried `world_id`, and
//! expiry at `now` (an expired announcement is void wherever it is
//! held). A registry's tombstone is that registry's opinion only: it
//! drops the registry's own entry and says why, and never voids an
//! announcement another layer serves.
//!
//! [`arbitrate`] is the pure rule over what the layers returned;
//! [`Resolver`] asks the layers and then arbitrates.

use std::fmt;

use anyhow::{Result, bail};

use crate::announcement::{SignedAnnouncement, Visibility};
use crate::announcement_relay::RelaySinks;
use crate::bridge_http::BridgeHttp;
use crate::registry::{RegistryEntry, SignedRegistryList};

/// A discovery layer, in the §6 order: [`Layer::Invite`] first,
/// then [`Layer::Pkarr`], then each subscribed registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// The announcement carried by an invite link, taken exactly.
    Invite,
    /// The announcement pkarr relays serve for the world key.
    Pkarr,
    /// The announcement a subscribed registry lists, named by the
    /// registry's key.
    Registry([u8; 32]),
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invite => f.write_str("invite link"),
            Self::Pkarr => f.write_str("pkarr"),
            Self::Registry(key) => {
                f.write_str("registry ")?;
                for byte in &key[..4] {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    }
}

/// What one layer returned for the queried world.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Offer {
    /// The layer served this announcement, still to be checked.
    Announcement(SignedAnnouncement),
    /// The layer has nothing for this world.
    Absent,
    /// A registry delisted the world, with its stated reason.
    Delisted(String),
    /// The layer could not be asked (a relay was down, say).
    Failed(String),
}

/// How [`arbitrate`] judged one layer's offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This layer's announcement was chosen.
    Chosen,
    /// Valid, but another layer served a higher seq, or the same seq
    /// from a layer earlier in the order.
    Superseded {
        /// The seq this layer served.
        seq: u64,
    },
    /// The layer served an announcement that is not valid for the
    /// queried world at `now`.
    Refused(String),
    /// The layer has nothing for this world.
    Absent,
    /// A registry delisted the world.
    Delisted(String),
    /// The layer could not be asked.
    Failed(String),
}

/// The chosen announcement, the layer it came from, and how every
/// layer's offer was judged, in layer order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The freshest valid announcement across the layers.
    pub announcement: SignedAnnouncement,
    /// The layer that served it.
    pub layer: Layer,
    /// Every layer asked, in §6 order, with its outcome.
    pub considered: Vec<(Layer, Outcome)>,
}

/// The arbitration rule: check each offer, then keep the valid
/// announcement with the highest seq; on equal seq the layer earlier
/// in the §6 order wins.
///
/// An announcement is valid when its signature verifies against its
/// own world key, it names `world_id`, and it has not expired at
/// `now` (Unix seconds).
///
/// # Errors
///
/// Fails when no layer served a valid announcement; the message says
/// what each layer returned.
pub fn arbitrate(world_id: &[u8; 32], offers: Vec<(Layer, Offer)>, now: u64) -> Result<Resolution> {
    let mut offers = offers;
    offers.sort_by_key(|(layer, _)| *layer);
    let mut considered: Vec<(Layer, Outcome)> = Vec::with_capacity(offers.len());
    let mut chosen: Option<(usize, SignedAnnouncement)> = None;
    for (index, (layer, offer)) in offers.into_iter().enumerate() {
        let outcome = match offer {
            Offer::Absent => Outcome::Absent,
            Offer::Delisted(reason) => Outcome::Delisted(reason),
            Offer::Failed(reason) => Outcome::Failed(reason),
            Offer::Announcement(signed) => {
                if let Err(error) = check(world_id, layer, &signed, now) {
                    Outcome::Refused(format!("{error:#}"))
                } else {
                    let fresher = chosen.as_ref().is_none_or(|(_, current)| {
                        signed.announcement.seq > current.announcement.seq
                    });
                    if fresher {
                        if let Some((previous, current)) = chosen.take() {
                            considered[previous].1 = Outcome::Superseded {
                                seq: current.announcement.seq,
                            };
                        }
                        chosen = Some((index, signed));
                        Outcome::Chosen
                    } else {
                        Outcome::Superseded {
                            seq: signed.announcement.seq,
                        }
                    }
                }
            }
        };
        considered.push((layer, outcome));
    }
    if let Some((index, announcement)) = chosen {
        return Ok(Resolution {
            announcement,
            layer: considered[index].0,
            considered,
        });
    }
    let reasons: Vec<String> = considered
        .iter()
        .map(|(layer, outcome)| format!("{layer}: {}", describe(outcome)))
        .collect();
    if reasons.is_empty() {
        bail!("no discovery layer was asked");
    }
    bail!(
        "no discovery layer served a valid announcement: {}",
        reasons.join("; ")
    )
}

/// One candidate's own checks: right world, signature, not expired,
/// and, from a registry, a world that chose to be listed.
///
/// An honest registry refuses an unlisted announcement at inclusion,
/// but the reader checks again: a registry that lists one anyway is
/// publishing a world its owner kept out of registries (design §6),
/// and the resolver will not discover it there (#15646).
fn check(world_id: &[u8; 32], layer: Layer, signed: &SignedAnnouncement, now: u64) -> Result<()> {
    if signed.announcement.world_id != *world_id {
        bail!("the announcement names another world");
    }
    if matches!(layer, Layer::Registry(_)) && signed.announcement.visibility != Visibility::Listed {
        bail!("a registry listed a world that is not listed");
    }
    signed.verify_at(now)
}

/// A short account of a non-chosen outcome, for error messages.
fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Chosen => "chosen".to_owned(),
        Outcome::Superseded { seq } => format!("superseded (seq {seq})"),
        Outcome::Refused(reason) => format!("refused: {reason}"),
        Outcome::Absent => "nothing for this world".to_owned(),
        Outcome::Delisted(reason) => format!("delisted: {reason}"),
        Outcome::Failed(reason) => format!("not reachable: {reason}"),
    }
}

/// What a subscribed registry says about `world_id`: its entry, its
/// tombstone, or nothing.
#[must_use]
pub fn registry_offer(list: &SignedRegistryList, world_id: &[u8; 32]) -> Offer {
    if let Some(entry) = list.list.entry(world_id) {
        return Offer::Announcement(entry.announcement.clone());
    }
    match list.list.tombstone(world_id) {
        Some(tombstone) => Offer::Delisted(tombstone.reason.clone()),
        None => Offer::Absent,
    }
}

/// Browse the subscribed registries by name or tag.
///
/// Returns every live listed world whose name contains `query` or
/// whose tags include it, ignoring case, one result per world (the
/// highest seq any registry holds), ordered by name. An empty query
/// lists every live world.
///
/// Tags are discovery hints only (design §3): a match here says what
/// a registry holds, never that the world is what its tags say. An
/// entry that is not listed is skipped, as [`arbitrate`] refuses it.
#[must_use]
pub fn search<'a>(
    registries: &'a [SignedRegistryList],
    query: &str,
    now: u64,
) -> Vec<&'a RegistryEntry> {
    let query = query.trim().to_lowercase();
    let mut found: Vec<&RegistryEntry> = Vec::new();
    for list in registries {
        for entry in list.list.live_entries(now) {
            let announcement = &entry.announcement.announcement;
            if announcement.visibility != Visibility::Listed {
                continue;
            }
            let matches = query.is_empty()
                || announcement.name.to_lowercase().contains(&query)
                || announcement
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase() == query);
            if !matches {
                continue;
            }
            match found
                .iter_mut()
                .find(|held| held.announcement.announcement.world_id == announcement.world_id)
            {
                Some(held) => {
                    if announcement.seq > held.announcement.announcement.seq {
                        *held = entry;
                    }
                }
                None => found.push(entry),
            }
        }
    }
    found.sort_by(|a, b| {
        a.announcement
            .announcement
            .name
            .cmp(&b.announcement.announcement.name)
            .then(
                a.announcement
                    .announcement
                    .world_id
                    .cmp(&b.announcement.announcement.world_id),
            )
    });
    found
}

/// The layers a reader resolves against: pkarr relays, if any, and
/// the registry lists it subscribes to.
///
/// Registry lists are taken as given: verify each one (with
/// [`SignedRegistryList::verify_from`] for the key the reader chose)
/// when it is fetched. Each listed announcement is still checked
/// again on its own here.
#[derive(Clone, Debug, Default)]
pub struct Resolver {
    pkarr: Option<RelaySinks>,
    registries: Vec<SignedRegistryList>,
}

impl Resolver {
    /// A resolver over `pkarr` relays (none skips the pkarr layer) and
    /// the subscribed `registries`.
    #[must_use]
    pub const fn new(pkarr: Option<RelaySinks>, registries: Vec<SignedRegistryList>) -> Self {
        Self { pkarr, registries }
    }

    /// The subscribed registry lists.
    #[must_use]
    pub fn registries(&self) -> &[SignedRegistryList] {
        &self.registries
    }

    /// Resolve `world_id`: the invite link's announcement, if the
    /// reader holds one, then pkarr, then each subscribed registry,
    /// arbitrated by [`arbitrate`].
    ///
    /// Every layer is asked, even after an earlier one answers,
    /// because a later layer may hold a fresher seq.
    ///
    /// # Errors
    ///
    /// Fails when no layer served a valid announcement.
    pub async fn resolve<H: BridgeHttp>(
        &self,
        http: &mut H,
        world_id: &[u8; 32],
        invite: Option<SignedAnnouncement>,
        now: u64,
    ) -> Result<Resolution> {
        let mut offers = Vec::new();
        if let Some(signed) = invite {
            offers.push((Layer::Invite, Offer::Announcement(signed)));
        }
        if let Some(relays) = &self.pkarr {
            let offer = match relays.fetch(http, world_id, now).await {
                Ok(signed) => Offer::Announcement(signed),
                Err(error) => Offer::Failed(format!("{error:#}")),
            };
            offers.push((Layer::Pkarr, offer));
        }
        for list in &self.registries {
            offers.push((
                Layer::Registry(list.list.registry_key),
                registry_offer(list, world_id),
            ));
        }
        arbitrate(world_id, offers, now)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::announcement::{Announcement, Occupancy, Subject};
    use crate::announcement_pkarr::{PkarrItem, Publisher};
    use crate::bridge_http::{HttpRequest, HttpResponse};
    use crate::registry::Registry;

    const NOW: u64 = 1_763_500_000;

    fn world_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn registry_key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn fields(key: &SigningKey, seq: u64, visibility: Visibility) -> Announcement {
        Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "Harbor".to_owned(),
            description: "a quiet world".to_owned(),
            subject: Subject::Endpoint {
                node_id: [8; 32],
                relay: None,
            },
            occupancy: Occupancy::HasRoom,
            tags: vec!["sailing".to_owned()],
            visibility,
            issued_at: NOW - 60,
            expires_at: NOW + 3_600,
            seq,
        }
    }

    fn signed(seq: u64, visibility: Visibility) -> SignedAnnouncement {
        let key = world_key();
        SignedAnnouncement::sign(&key, fields(&key, seq, visibility)).unwrap()
    }

    fn world_id() -> [u8; 32] {
        world_key().verifying_key().to_bytes()
    }

    fn registry_with(byte: u8, announcement: Option<SignedAnnouncement>) -> SignedRegistryList {
        let mut registry = Registry::new(registry_key(byte), "worlds we like").unwrap();
        if let Some(announcement) = announcement {
            registry.include(announcement, NOW - 30).unwrap();
        }
        registry.publish(NOW - 10).unwrap()
    }

    /// Relays that serve one stored pkarr payload, or nothing.
    struct OneRelay(Option<Vec<u8>>);

    impl BridgeHttp for OneRelay {
        fn request(
            &mut self,
            _request: HttpRequest,
        ) -> impl Future<Output = Result<HttpResponse>> + Send {
            std::future::ready(Ok(match &self.0 {
                Some(body) => HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: body.clone(),
                },
                None => HttpResponse {
                    status: 404,
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            }))
        }
    }

    fn pkarr_payload(seq_hint: u64) -> (Vec<u8>, u64) {
        let key = world_key();
        let mut publisher = Publisher::new(key.clone(), 10_800).unwrap();
        let base = fields(&key, 0, Visibility::Unlisted);
        let item: PkarrItem = publisher.publish(&base, seq_hint).unwrap();
        (item.to_relay_payload(), item.timestamp_us)
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn relays() -> RelaySinks {
        RelaySinks::new(["https://relay.example/pkarr"]).unwrap()
    }

    #[test]
    fn an_invite_only_world_resolves_from_its_invite() {
        let invite = signed(5, Visibility::Unlisted);
        let resolver = Resolver::new(Some(relays()), vec![registry_with(1, None)]);
        let resolution =
            block_on(resolver.resolve(&mut OneRelay(None), &world_id(), Some(invite.clone()), NOW))
                .unwrap();
        assert_eq!(resolution.layer, Layer::Invite);
        assert_eq!(resolution.announcement, invite);
        assert!(matches!(resolution.considered[1].1, Outcome::Failed(_)));
        assert_eq!(resolution.considered[2].1, Outcome::Absent);
    }

    #[test]
    fn an_unlisted_world_resolves_from_pkarr_alone() {
        let (payload, timestamp) = pkarr_payload(NOW * 1_000_000);
        let resolver = Resolver::new(Some(relays()), vec![registry_with(1, None)]);
        let resolution =
            block_on(resolver.resolve(&mut OneRelay(Some(payload)), &world_id(), None, NOW))
                .unwrap();
        assert_eq!(resolution.layer, Layer::Pkarr);
        assert_eq!(resolution.announcement.announcement.seq, timestamp);
    }

    #[test]
    fn a_listed_world_resolves_from_a_registry() {
        let listed = signed(9, Visibility::Listed);
        let registry = registry_with(1, Some(listed.clone()));
        let registry_id = registry.list.registry_key;
        let resolver = Resolver::new(None, vec![registry]);
        let resolution =
            block_on(resolver.resolve(&mut OneRelay(None), &world_id(), None, NOW)).unwrap();
        assert_eq!(resolution.layer, Layer::Registry(registry_id));
        assert_eq!(resolution.announcement, listed);
    }

    #[test]
    fn a_fresher_pkarr_record_beats_a_stale_registry_entry() {
        let stale = signed(3, Visibility::Listed);
        let (payload, timestamp) = pkarr_payload(NOW * 1_000_000);
        let resolver = Resolver::new(Some(relays()), vec![registry_with(1, Some(stale))]);
        let resolution =
            block_on(resolver.resolve(&mut OneRelay(Some(payload)), &world_id(), None, NOW))
                .unwrap();
        assert_eq!(resolution.layer, Layer::Pkarr);
        assert_eq!(resolution.announcement.announcement.seq, timestamp);
        assert_eq!(resolution.considered[1].1, Outcome::Superseded { seq: 3 });
    }

    #[test]
    fn a_fresher_registry_entry_beats_an_older_invite() {
        let old = signed(3, Visibility::Listed);
        let fresh = signed(4, Visibility::Listed);
        let registry = registry_with(1, Some(fresh.clone()));
        let resolution = arbitrate(
            &world_id(),
            vec![
                (Layer::Invite, Offer::Announcement(old)),
                (
                    Layer::Registry(registry.list.registry_key),
                    registry_offer(&registry, &world_id()),
                ),
            ],
            NOW,
        )
        .unwrap();
        assert_eq!(resolution.announcement, fresh);
        assert_eq!(resolution.considered[0].1, Outcome::Superseded { seq: 3 });
        assert_eq!(resolution.considered[1].1, Outcome::Chosen);
    }

    #[test]
    fn on_equal_seq_the_earlier_layer_wins_whatever_the_offer_order() {
        let same = signed(4, Visibility::Listed);
        let resolution = arbitrate(
            &world_id(),
            vec![
                (Layer::Registry([1; 32]), Offer::Announcement(same.clone())),
                (Layer::Pkarr, Offer::Announcement(same)),
            ],
            NOW,
        )
        .unwrap();
        assert_eq!(resolution.layer, Layer::Pkarr);
        assert_eq!(resolution.considered[0].0, Layer::Pkarr);
    }

    #[test]
    fn forged_and_foreign_announcements_are_refused() {
        let other = SigningKey::from_bytes(&[9; 32]);
        let foreign =
            SignedAnnouncement::sign(&other, fields(&other, 7, Visibility::Listed)).unwrap();
        let mut forged = signed(8, Visibility::Listed);
        forged.announcement.seq = 99;
        let valid = signed(2, Visibility::Listed);
        let resolution = arbitrate(
            &world_id(),
            vec![
                (Layer::Invite, Offer::Announcement(foreign)),
                (Layer::Pkarr, Offer::Announcement(forged)),
                (Layer::Registry([2; 32]), Offer::Announcement(valid.clone())),
            ],
            NOW,
        )
        .unwrap();
        assert_eq!(resolution.announcement, valid);
        assert!(
            matches!(&resolution.considered[0].1, Outcome::Refused(reason) if reason.contains("another world"))
        );
        assert!(
            matches!(&resolution.considered[1].1, Outcome::Refused(reason) if reason.contains("signature"))
        );
    }

    #[test]
    fn an_expired_announcement_is_void_in_every_layer() {
        let held = signed(9, Visibility::Listed);
        let error = arbitrate(
            &world_id(),
            vec![
                (Layer::Invite, Offer::Announcement(held.clone())),
                (Layer::Registry([1; 32]), Offer::Announcement(held)),
            ],
            NOW + 3_600,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("expired"), "{error}");
    }

    #[test]
    fn a_tombstone_drops_only_its_own_registry() {
        let listed = signed(6, Visibility::Listed);
        let mut registry = Registry::new(registry_key(1), "worlds we like").unwrap();
        registry.include(listed.clone(), NOW - 30).unwrap();
        registry.delist(world_id(), "off-topic", NOW - 20).unwrap();
        let delisted = registry.publish(NOW - 10).unwrap();
        let other = registry_with(2, Some(listed.clone()));
        let resolver = Resolver::new(None, vec![delisted, other.clone()]);
        let resolution =
            block_on(resolver.resolve(&mut OneRelay(None), &world_id(), None, NOW)).unwrap();
        assert_eq!(resolution.announcement, listed);
        assert_eq!(resolution.layer, Layer::Registry(other.list.registry_key));
        assert!(
            resolution
                .considered
                .iter()
                .any(|(_, outcome)| *outcome == Outcome::Delisted("off-topic".to_owned()))
        );
    }

    #[test]
    fn no_valid_layer_is_an_error_that_names_each_layer() {
        let error = arbitrate(
            &world_id(),
            vec![
                (Layer::Pkarr, Offer::Failed("relay down".to_owned())),
                (Layer::Registry([1; 32]), Offer::Absent),
            ],
            NOW,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("pkarr: not reachable: relay down"),
            "{error}"
        );
        assert!(error.contains("registry 01010101: nothing"), "{error}");
        assert!(arbitrate(&world_id(), Vec::new(), NOW).is_err());
    }

    #[test]
    fn search_matches_names_and_tags_once_per_world() {
        let older = signed(3, Visibility::Listed);
        let newer = signed(4, Visibility::Listed);
        let registries = vec![registry_with(1, Some(older)), registry_with(2, Some(newer))];
        let by_name = search(&registries, "harb", NOW);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].announcement.announcement.seq, 4);
        assert_eq!(search(&registries, "SAILING", NOW).len(), 1);
        assert_eq!(search(&registries, "sail", NOW), [] as [&RegistryEntry; 0]);
        assert_eq!(search(&registries, "", NOW).len(), 1);
        assert_eq!(
            search(&registries, "", NOW + 3_600),
            [] as [&RegistryEntry; 0]
        );
    }
}
