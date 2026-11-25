//! World announcements carried in pkarr signed packets (#10706).
//!
//! The encoding, timing and sink contracts are pinned by the design
//! addendum doc/2026-11-25T03-10-00Z_design_id10706_announcement_pkarr
//! (parent: 2026-11-16_design_id6287_discovery_registry_federation_v1,
//! section 4). Its stdlib reference emits the known-answer vectors the
//! tests below reproduce.
//!
//! - One `TXT` record named `_ida.<z32 world key>` whose rdata is the
//!   [`SignedAnnouncement`] CBOR split into 255-byte character-strings.
//! - The pkarr relay payload is `signature ‖ timestamp_us_be ‖ dns`,
//!   the signature being the world key's BEP44 signature over the
//!   bencoded `3:seqi<ts>e1:v<len>:<dns>`.
//! - The announcement's `seq` is the pkarr timestamp, so one number
//!   orders both the BEP44 item and the per-world sequence.
//!
//! [`Publisher`] is the pure state machine the world runtime drives on
//! a change and on its timer; [`resolve`] is the resolver's pkarr step.
//! Nothing here does I/O.

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::announcement::{Announcement, MAX_LIFETIME_SECONDS, SignedAnnouncement, Subject};

/// The DNS packet ceiling of a pkarr signed packet, in bytes.
pub const MAX_DNS_PACKET_BYTES: usize = 1_000;
/// The record label the announcement lives under.
pub const RECORD_LABEL: &str = "_ida";
/// The record's DNS TTL, in seconds.
pub const RECORD_TTL_SECONDS: u32 = 300;
/// How long the Mainline DHT keeps a BEP44 item without a refresh.
pub const DHT_RETENTION_SECONDS: u64 = 2 * 60 * 60;
/// The default announcement lifetime when the operator sets none.
pub const DEFAULT_LIFETIME_SECONDS: u64 = 3 * 60 * 60;

const TYPE_TXT: u16 = 16;
const CLASS_IN: u16 = 1;
/// QR + AA: a response, as pkarr builds its packets.
const FLAGS: u16 = 0x8400;
const HEADER_BYTES: usize = 12;
const SIGNATURE_BYTES: usize = 64;
const TIMESTAMP_BYTES: usize = 8;

const Z32_ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

/// z-base-32 encoding, the form pkarr names keys in.
#[must_use]
pub fn z32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0_u32;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(Z32_ALPHABET[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(
            Z32_ALPHABET[((buffer << (5 - bits)) & 31) as usize],
        ));
    }
    out
}

fn push_name(out: &mut Vec<u8>, labels: &[&str]) {
    for label in labels {
        // Both labels are fixed-shape ASCII under 64 bytes.
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
}

/// The DNS packet bytes a signed announcement of `signed_len` bytes
/// needs: header, the fully qualified name, the fixed record fields
/// and one length byte per 255-byte chunk.
#[must_use]
pub const fn dns_packet_len(signed_len: usize) -> usize {
    let name = 1 + RECORD_LABEL.len() + 1 + 52 + 1;
    HEADER_BYTES + name + 10 + signed_len + signed_len.div_ceil(255)
}

/// The largest signed announcement a pkarr DNS packet can carry.
#[must_use]
pub const fn max_signed_announcement_bytes() -> usize {
    let mut n = 0;
    while dns_packet_len(n + 1) <= MAX_DNS_PACKET_BYTES {
        n += 1;
    }
    n
}

/// Build the DNS packet carrying `signed` (an announcement's CBOR)
/// under `world_key`.
///
/// # Errors
///
/// Fails when `signed` is empty or the packet would exceed
/// [`MAX_DNS_PACKET_BYTES`].
pub fn dns_packet(world_key: &[u8; 32], signed: &[u8]) -> Result<Vec<u8>> {
    ensure!(!signed.is_empty(), "an announcement record cannot be empty");
    let len = dns_packet_len(signed.len());
    ensure!(
        len <= MAX_DNS_PACKET_BYTES,
        "announcement DNS packet is {len} bytes, over the {MAX_DNS_PACKET_BYTES}-byte pkarr limit"
    );
    let mut rdata = Vec::with_capacity(signed.len() + signed.len().div_ceil(255));
    for chunk in signed.chunks(255) {
        rdata.push(chunk.len() as u8);
        rdata.extend_from_slice(chunk);
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&0_u16.to_be_bytes());
    out.extend_from_slice(&FLAGS.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
    push_name(&mut out, &[RECORD_LABEL, &z32_encode(world_key)]);
    out.extend_from_slice(&TYPE_TXT.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());
    out.extend_from_slice(&RECORD_TTL_SECONDS.to_be_bytes());
    out.extend_from_slice(&u16::try_from(rdata.len())?.to_be_bytes());
    out.extend_from_slice(&rdata);
    debug_assert_eq!(out.len(), len);
    Ok(out)
}

/// The BEP44 bytes the pkarr signature covers.
#[must_use]
pub fn bep44_signable(timestamp_us: u64, dns: &[u8]) -> Vec<u8> {
    let mut out = format!("3:seqi{timestamp_us}e1:v{}:", dns.len()).into_bytes();
    out.extend_from_slice(dns);
    out
}

/// A pkarr signed packet: the unit a relay stores and the DHT holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PkarrItem {
    /// The BEP44 seq: microseconds since the Unix epoch.
    pub timestamp_us: u64,
    /// The DNS packet.
    pub dns_packet: Vec<u8>,
    /// The key's signature over [`bep44_signable`].
    pub signature: [u8; 64],
}

impl PkarrItem {
    /// Wrap a signed announcement in its pkarr item, signed by the
    /// world key. The timestamp is the announcement's `seq`.
    ///
    /// # Errors
    ///
    /// Fails when `key` is not the announcement's world key, or the
    /// announcement does not fit the packet.
    pub fn sign(key: &SigningKey, signed: &SignedAnnouncement) -> Result<Self> {
        let world_key = key.verifying_key().to_bytes();
        ensure!(
            signed.announcement.world_id == world_key,
            "a world announcement is published only under its own world key"
        );
        let bytes = signed.to_bytes()?;
        let dns_packet = dns_packet(&world_key, &bytes)?;
        let timestamp_us = signed.announcement.seq;
        let signature = key
            .sign(&bep44_signable(timestamp_us, &dns_packet))
            .to_bytes();
        Ok(Self {
            timestamp_us,
            dns_packet,
            signature,
        })
    }

    /// The pkarr relay body: `signature ‖ timestamp_us_be ‖ dns`.
    #[must_use]
    pub fn to_relay_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SIGNATURE_BYTES + TIMESTAMP_BYTES + self.dns_packet.len());
        out.extend_from_slice(&self.signature);
        out.extend_from_slice(&self.timestamp_us.to_be_bytes());
        out.extend_from_slice(&self.dns_packet);
        out
    }

    /// Split a relay body. Does not verify; [`resolve`] does.
    ///
    /// # Errors
    ///
    /// Fails when the body is shorter than its header or its packet
    /// is over [`MAX_DNS_PACKET_BYTES`].
    pub fn from_relay_payload(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() > SIGNATURE_BYTES + TIMESTAMP_BYTES,
            "pkarr payload is shorter than its header"
        );
        let (signature, rest) = bytes.split_at(SIGNATURE_BYTES);
        let (timestamp, dns) = rest.split_at(TIMESTAMP_BYTES);
        ensure!(
            dns.len() <= MAX_DNS_PACKET_BYTES,
            "pkarr DNS packet is over {MAX_DNS_PACKET_BYTES} bytes"
        );
        Ok(Self {
            timestamp_us: u64::from_be_bytes(timestamp.try_into()?),
            dns_packet: dns.to_vec(),
            signature: signature.try_into()?,
        })
    }

    /// Check the pkarr signature under `world_key`.
    ///
    /// # Errors
    ///
    /// Fails when the key is not a key or the signature does not verify.
    pub fn verify(&self, world_key: &[u8; 32]) -> Result<()> {
        let key = VerifyingKey::from_bytes(world_key).context("world key is not a key")?;
        key.verify_strict(
            &bep44_signable(self.timestamp_us, &self.dns_packet),
            &Signature::from_bytes(&self.signature),
        )
        .context("pkarr signature does not verify under the queried key")
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .context("DNS packet offset overflows")?;
        let slice = self
            .bytes
            .get(self.at..end)
            .context("DNS packet is truncated")?;
        self.at = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }

    fn name(&mut self) -> Result<Vec<String>> {
        let mut labels = Vec::new();
        loop {
            let len = self.take(1)?[0];
            if len & 0xc0 != 0 {
                bail!("compressed names are not used in announcement packets");
            }
            if len == 0 {
                return Ok(labels);
            }
            let label = std::str::from_utf8(self.take(usize::from(len))?)
                .context("DNS label is not text")?;
            labels.push(label.to_ascii_lowercase());
        }
    }
}

/// Every announcement record in `dns`, as its joined character-strings.
/// A record counts when it is `TXT`/`IN` and named `_ida` or
/// `_ida.<z32 world key>`; other records are ignored.
///
/// # Errors
///
/// Fails when the packet is malformed.
pub fn announcement_records(dns: &[u8], world_key: &[u8; 32]) -> Result<Vec<Vec<u8>>> {
    ensure!(
        dns.len() >= HEADER_BYTES && dns.len() <= MAX_DNS_PACKET_BYTES,
        "DNS packet size is out of range"
    );
    let mut reader = Reader { bytes: dns, at: 4 };
    let questions = reader.u16()?;
    let mut records = 0_usize;
    for _ in 0..3 {
        records += usize::from(reader.u16()?);
    }
    for _ in 0..questions {
        reader.name()?;
        reader.take(4)?;
    }
    let origin = z32_encode(world_key);
    let mut found = Vec::new();
    for _ in 0..records {
        let labels = reader.name()?;
        let rtype = reader.u16()?;
        let rclass = reader.u16()?;
        reader.take(4)?;
        let rdlen = usize::from(reader.u16()?);
        let rdata = reader.take(rdlen)?;
        let named = match labels.as_slice() {
            [label] => label == RECORD_LABEL,
            [label, key] => label == RECORD_LABEL && *key == origin,
            _ => false,
        };
        if !(named && rtype == TYPE_TXT && rclass == CLASS_IN) {
            continue;
        }
        let mut joined = Vec::with_capacity(rdata.len());
        let mut strings = Reader {
            bytes: rdata,
            at: 0,
        };
        while strings.at < rdata.len() {
            let len = strings.take(1)?[0];
            joined.extend_from_slice(strings.take(usize::from(len))?);
        }
        found.push(joined);
    }
    ensure!(
        reader.at == dns.len(),
        "trailing bytes after the DNS records"
    );
    Ok(found)
}

/// The resolver's pkarr step: verify a relay payload fetched for
/// `world_key` and return its announcement, valid at `now` (Unix
/// seconds). The checks run in the addendum's order and every
/// failure is a refusal.
///
/// # Errors
///
/// Fails on a bad pkarr signature, anything but exactly one
/// announcement record, a record that does not decode, an
/// announcement for another world, a bad announcement signature, a
/// `seq` that is not the packet timestamp, or an expired announcement.
pub fn resolve(world_key: &[u8; 32], payload: &[u8], now: u64) -> Result<SignedAnnouncement> {
    let item = PkarrItem::from_relay_payload(payload)?;
    item.verify(world_key)?;
    let mut records = announcement_records(&item.dns_packet, world_key)?;
    ensure!(
        records.len() == 1,
        "packet carries {} announcement records, not exactly one",
        records.len()
    );
    let signed = SignedAnnouncement::from_bytes(&records.remove(0))?;
    ensure!(
        signed.announcement.world_id == *world_key,
        "announcement world_id is not the queried key"
    );
    signed.verify()?;
    ensure!(
        signed.announcement.seq == item.timestamp_us,
        "announcement seq is not the packet timestamp"
    );
    signed.verify_at(now)?;
    Ok(signed)
}

/// The publishing state machine for one world (addendum section 2).
///
/// The runtime calls [`Publisher::publish`] when a published field
/// changes and whenever [`Publisher::due`] says a heartbeat is owed,
/// then hands the returned item to its sinks.
#[derive(Debug)]
pub struct Publisher {
    key: SigningKey,
    lifetime_seconds: u64,
    last_timestamp_us: Option<u64>,
    next_heartbeat_at: Option<u64>,
}

impl Publisher {
    /// A publisher for the world `key`, with announcements living
    /// `lifetime_seconds`.
    ///
    /// # Errors
    ///
    /// Fails when the lifetime is outside `1..=7 days`.
    pub fn new(key: SigningKey, lifetime_seconds: u64) -> Result<Self> {
        ensure!(
            (1..=MAX_LIFETIME_SECONDS).contains(&lifetime_seconds),
            "announcement lifetime must be within 1..={MAX_LIFETIME_SECONDS} seconds"
        );
        Ok(Self {
            key,
            lifetime_seconds,
            last_timestamp_us: None,
            next_heartbeat_at: None,
        })
    }

    /// Seconds between heartbeats: half of the lifetime or of the DHT
    /// retention, whichever is shorter, and at least one.
    #[must_use]
    pub fn heartbeat_seconds(&self) -> u64 {
        (self.lifetime_seconds.min(DHT_RETENTION_SECONDS) / 2).max(1)
    }

    /// Whether a publish is owed at `now` (Unix seconds): never yet
    /// published, or the heartbeat is due.
    #[must_use]
    pub fn due(&self, now: u64) -> bool {
        self.next_heartbeat_at.is_none_or(|at| now >= at)
    }

    /// When the next heartbeat is due, once anything was published.
    #[must_use]
    pub fn next_heartbeat_at(&self) -> Option<u64> {
        self.next_heartbeat_at
    }

    /// Sign and wrap `fields` for publishing at `now_us` (microseconds
    /// since the Unix epoch). `issued_at`, `expires_at` and `seq` are
    /// set here; the other fields are taken as given.
    ///
    /// # Errors
    ///
    /// Fails when the announcement is not this world's, when its
    /// endpoint is the world key itself (iroh's own publishing would
    /// overwrite the packet), or when it breaks a construction rule
    /// or the packet budget.
    pub fn publish(&mut self, fields: &Announcement, now_us: u64) -> Result<PkarrItem> {
        let world_key = self.key.verifying_key().to_bytes();
        ensure!(
            fields.world_id == world_key,
            "announcement world_id is not this publisher's world key"
        );
        if let Subject::Endpoint { node_id, .. } = &fields.subject {
            ensure!(
                *node_id != world_key,
                "the world key must not be the endpoint's iroh key: one pkarr packet per key"
            );
        }
        let timestamp_us = self
            .last_timestamp_us
            .map_or(now_us, |previous| now_us.max(previous.saturating_add(1)));
        let now = now_us / 1_000_000;
        let mut announcement = fields.clone();
        announcement.issued_at = now;
        announcement.expires_at = now + self.lifetime_seconds;
        announcement.seq = timestamp_us;
        let signed = SignedAnnouncement::sign(&self.key, announcement)?;
        let item = PkarrItem::sign(&self.key, &signed)?;
        self.last_timestamp_us = Some(timestamp_us);
        self.next_heartbeat_at = Some(now + self.heartbeat_seconds());
        Ok(item)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::announcement::{MAX_PACKET_BYTES, Occupancy, Visibility};
    use crate::hex::hex_encode;

    // From the addendum's fixtures/vectors.json.
    const ISSUED: u64 = 1_763_500_000;
    const WORLD_KEY_Z32: &str = "7jfgaa9nutjyixzikb7tgmsf9gkwq7iqz498zr1nd5ig1fng4esy";
    const BODY_HEX: &str = "aa637365711b000643e4d70fb800646e616d6566686172626f726474616773826463616c6d6474657374677375626a656374a168656e64706f696e74a26572656c6179781a68747470733a2f2f72656c61792e6578616d706c652e6e65742f676e6f64655f696458201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca68776f726c645f69645820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a691d0a106a7669736962696c697479666c69737465646b6465736372697074696f6e7261207175696574207465737420776f726c64";
    const RELAY_PAYLOAD_HEX: &str = "5f35be383f96b8e515473dd2bbb2ba4746198759af4f7c5966872c23d4c0d2679be6309569b0db6c3461d429374d8f0369a4ba40c1229abc72a649ccdcbf7404000643e4d70fb800000084000000000100000000045f69646134376a66676161396e75746a7969787a696b623774676d736639676b77713769717a3439387a72316e6435696731666e673465737900001000010000012c0173ffa2697369676e617475726558400199af1d4f55f43f27641b41ec5c9eb828cf1ef5765189f4d965c092fc383e67b3a0c0e8c6d1cf6595e1da4870a82efd83b02be4d27ac824c0e7ffb041b009016c616e6e6f756e63656d656e74aa637365711b000643e4d70fb800646e616d6566686172626f726474616773826463616c6d6474657374677375626a656374a168656e64706f696e74a26572656c6179781a68747470733a2f2f72656c61792e6578616d706c652e6e65742f676e6f64655f696458201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca68776f726c645f69645820ea4a6c63e29c520abef5507b132ec5f995724776aebebe7b92421eea691446d22c696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a691d0a106a7669736962696c697479666c69737465646b6465736372697074696f6e7261207175696574207465737420776f726c64";

    fn fields() -> (SigningKey, Announcement) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let node = SigningKey::from_bytes(&[8; 32]);
        let announcement = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "harbor".to_owned(),
            description: "a quiet test world".to_owned(),
            subject: Subject::Endpoint {
                node_id: node.verifying_key().to_bytes(),
                relay: Some("https://relay.example.net/".to_owned()),
            },
            occupancy: Occupancy::HasRoom,
            tags: vec!["calm".to_owned(), "test".to_owned()],
            visibility: Visibility::Listed,
            issued_at: 0,
            expires_at: 1,
            seq: 0,
        };
        (key, announcement)
    }

    fn published() -> (SigningKey, PkarrItem) {
        let (key, fields) = fields();
        let mut publisher = Publisher::new(key.clone(), DEFAULT_LIFETIME_SECONDS).unwrap();
        let item = publisher.publish(&fields, ISSUED * 1_000_000).unwrap();
        (key, item)
    }

    fn world_key(key: &SigningKey) -> [u8; 32] {
        key.verifying_key().to_bytes()
    }

    #[test]
    fn kat_reproduces_the_reference_vectors() {
        let (key, item) = published();
        assert_eq!(z32_encode(&world_key(&key)), WORLD_KEY_Z32);
        assert_eq!(hex_encode(&item.to_relay_payload()), RELAY_PAYLOAD_HEX);
        let signed = resolve(&world_key(&key), &item.to_relay_payload(), ISSUED + 60).unwrap();
        assert_eq!(
            hex_encode(&signed.announcement.encode_body().unwrap()),
            BODY_HEX
        );
    }

    #[test]
    fn the_budget_matches_the_reference() {
        assert_eq!(max_signed_announcement_bytes(), 915);
        assert_eq!(max_signed_announcement_bytes(), MAX_PACKET_BYTES);
        let key = [1; 32];
        assert!(dns_packet(&key, &[0; 915]).unwrap().len() <= MAX_DNS_PACKET_BYTES);
        assert!(dns_packet(&key, &[0; 916]).is_err());
        assert!(dns_packet(&key, &[]).is_err());
    }

    #[test]
    fn a_relative_name_resolves_too() {
        let (key, item) = published();
        let origin = z32_encode(&world_key(&key));
        let mut full = vec![4];
        full.extend_from_slice(b"_ida");
        full.push(52);
        full.extend_from_slice(origin.as_bytes());
        full.push(0);
        let at = item
            .dns_packet
            .windows(full.len())
            .position(|w| w == full)
            .unwrap();
        let mut relative = item.dns_packet[..at].to_vec();
        relative.extend_from_slice(b"\x04_ida\x00");
        relative.extend_from_slice(&item.dns_packet[at + full.len()..]);
        let records = announcement_records(&relative, &world_key(&key)).unwrap();
        assert_eq!(
            records,
            announcement_records(&item.dns_packet, &world_key(&key)).unwrap()
        );
    }

    fn resign(key: &SigningKey, timestamp_us: u64, dns_packet: Vec<u8>) -> Vec<u8> {
        let signature = key
            .sign(&bep44_signable(timestamp_us, &dns_packet))
            .to_bytes();
        PkarrItem {
            timestamp_us,
            dns_packet,
            signature,
        }
        .to_relay_payload()
    }

    #[test]
    fn the_resolver_refuses_what_the_addendum_refuses() {
        let (key, item) = published();
        let wk = world_key(&key);
        let now = ISSUED + 60;
        let payload = item.to_relay_payload();

        let mut tampered = payload.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(resolve(&wk, &tampered, now).is_err());

        let other = SigningKey::from_bytes(&[9; 32]);
        assert!(resolve(&world_key(&other), &payload, now).is_err());

        assert!(resolve(&wk, &payload, ISSUED + DEFAULT_LIFETIME_SECONDS).is_err());

        // Another world's valid announcement republished under this key.
        let (_, mut foreign_fields) = fields();
        foreign_fields.world_id = world_key(&other);
        let mut foreign_publisher = Publisher::new(other, 3600).unwrap();
        let foreign = foreign_publisher
            .publish(&foreign_fields, item.timestamp_us)
            .unwrap();
        let record = announcement_records(&foreign.dns_packet, &foreign_fields.world_id)
            .unwrap()
            .remove(0);
        let smuggled = dns_packet(&wk, &record).unwrap();
        assert!(resolve(&wk, &resign(&key, item.timestamp_us, smuggled), now).is_err());

        // The inner seq must be the packet timestamp.
        let skewed = resign(&key, item.timestamp_us + 1, item.dns_packet.clone());
        assert!(resolve(&wk, &skewed, now).is_err());

        // Exactly one record: two and none both refuse.
        let mut doubled = item.dns_packet.clone();
        doubled[7] = 2;
        doubled.extend_from_slice(&item.dns_packet[HEADER_BYTES..]);
        assert!(resolve(&wk, &resign(&key, item.timestamp_us, doubled), now).is_err());
        let mut renamed = item.dns_packet.clone();
        let at = renamed.windows(4).position(|w| w == b"_ida").unwrap();
        renamed[at + 3] = b'b';
        assert!(resolve(&wk, &resign(&key, item.timestamp_us, renamed), now).is_err());

        assert!(resolve(&wk, &payload[..72], now).is_err());
    }

    #[test]
    fn the_publisher_keeps_seq_rising_and_times_heartbeats() {
        let (key, fields) = fields();
        let mut publisher = Publisher::new(key, DEFAULT_LIFETIME_SECONDS).unwrap();
        assert_eq!(publisher.heartbeat_seconds(), 3600);
        assert!(publisher.due(0));
        let first = publisher.publish(&fields, 5_000_000).unwrap();
        assert_eq!(first.timestamp_us, 5_000_000);
        assert_eq!(publisher.next_heartbeat_at(), Some(5 + 3600));
        assert!(!publisher.due(5 + 3599));
        assert!(publisher.due(5 + 3600));
        // A clock that steps back still advances the sequence.
        let second = publisher.publish(&fields, 4_000_000).unwrap();
        assert_eq!(second.timestamp_us, 5_000_001);

        let short = Publisher::new(SigningKey::from_bytes(&[7; 32]), 1200).unwrap();
        assert_eq!(short.heartbeat_seconds(), 600);
        assert!(Publisher::new(SigningKey::from_bytes(&[7; 32]), 0).is_err());
        assert!(
            Publisher::new(SigningKey::from_bytes(&[7; 32]), MAX_LIFETIME_SECONDS + 1).is_err()
        );
    }

    #[test]
    fn the_publisher_refuses_foreign_worlds_and_shared_keys() {
        let (key, mut fields) = fields();
        let mut publisher = Publisher::new(key.clone(), 3600).unwrap();
        let mut foreign = fields.clone();
        foreign.world_id = [9; 32];
        assert!(publisher.publish(&foreign, 1_000_000).is_err());
        fields.subject = Subject::Endpoint {
            node_id: world_key(&key),
            relay: None,
        };
        assert!(publisher.publish(&fields, 1_000_000).is_err());
    }
}
