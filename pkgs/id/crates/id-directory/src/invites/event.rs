//! The inviter's signed invite event.
//!
//! Two wire forms exist. The **v1 form** is the original fixed layout —
//! `inviter (32) || ticket_digest (32) || group (32) || role (1) || expires_ms (8, big-endian)`,
//! then the signature (64) — and is read-only: events already minted in it keep parsing and
//! verifying, and carry no delegation terms (a group target, no use cap, depth 0, no parent,
//! no redeemer pin). New invites are minted in the **v2 form**: a magic + version header,
//! then the body as postcard (#5674), then the signature. The v2 body carries the delegation
//! terms of the #5751 design record: the target (a group, or a world and its fronting group),
//! a use cap, the remaining delegation depth, the parent invite's digest, and an optional
//! redeemer pin.
//!
//! Budgets (§1 of the record): the v2 body is at most [`MAX_BODY_LEN`] bytes, refused at
//! decode, so a future field cannot silently grow the object; the paste form's budget lives
//! in [`super::invitation`]. A v1 event whose inviter key began with the v2 magic would be
//! misread as v2 — keys are random, and v1 is a legacy read form, so the magic is the
//! discriminator, the same shape as the `IDKEY` envelope in `id-core`.
//!
//! In both forms the signature covers the body's bytes behind a domain tag, so a signature
//! made here cannot be replayed as a signature over some other message. Role codes are part
//! of the v1 format: 0 `Pull`, 1 `Read`, 2 `Write`, 3 `Manage`, 4 `Admin` — the same order
//! postcard assigns the variants in v2.

use anyhow::{Context, Result, anyhow, bail, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use iroh_tickets::{Ticket as _, endpoint::EndpointTicket};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use id_events::groups::Role;

const DOMAIN_V1: &[u8] = b"id-invite/v1/event";
const DOMAIN_V2: &[u8] = b"id-invite/v2/event";
const DIGEST_DOMAIN: &[u8] = b"id-invite/body-digest";
const TICKET_DOMAIN: &[u8] = b"id-invite/v1/ticket";
/// The v1 fields before the signature: 32 + 32 + 32 + 1 + 8 bytes.
const FIELDS_LEN: usize = 105;
const EVENT_LEN: usize = FIELDS_LEN + 64;

const V2_MAGIC: &[u8; 5] = b"IDINV";
const V2_VERSION: u8 = 2;
/// Magic + version byte.
const V2_HEADER_LEN: usize = 6;

/// The most bytes a v2 invite body may occupy (§1). A body over budget is refused at
/// decode: the budget is a hard parser limit, not a hope.
pub const MAX_BODY_LEN: usize = 512;
/// The deepest a source invite may be minted (§2): world/group policy may raise an
/// open-delegation source invite from its default of 1 to at most this.
pub const MAX_SOURCE_DEPTH: u8 = 3;
/// A link invite's default lifetime (§3): 7 days, in milliseconds.
pub const LINK_DEFAULT_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// A link invite's default use cap (§3).
pub const LINK_DEFAULT_USES_CAP: u32 = 25;

/// What an invite joins the redeemer to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InviteTarget {
    /// The group itself.
    Group([u8; 32]),
    /// A world, reached through the group that fronts it (§4: the group's live
    /// fronting set decides what membership opens).
    World {
        /// The world the invite targets.
        world: [u8; 32],
        /// The group fronting the world; the group a link names and a route
        /// check compares against.
        fronting_group: [u8; 32],
    },
}

impl InviteTarget {
    /// The group the invite is anchored to: the target group, or the fronting group.
    #[must_use]
    pub const fn group(&self) -> [u8; 32] {
        match self {
            Self::Group(group)
            | Self::World {
                fronting_group: group,
                ..
            } => *group,
        }
    }
}

/// Where an invite's authority comes from (#5751). The source sets the default
/// delegation depth (§2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteSource {
    /// Open delegation: participants may invite; source invites default to depth 1.
    OpenDelegation,
    /// A moderator grant: source invites default to depth 0; a grant may extend
    /// depth explicitly, never beyond the grantor's own remaining depth.
    ModeratorGrant,
}

impl InviteSource {
    /// The depth a source invite from this source is minted with by default (§2).
    #[must_use]
    pub const fn default_depth(&self) -> u8 {
        match self {
            Self::OpenDelegation => 1,
            Self::ModeratorGrant => 0,
        }
    }
}

/// The delegation terms an inviter chooses, before they are signed into a body.
/// Defaults follow §2–§3; see [`InviteTerms::source`] and [`InviteTerms::link`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteTerms {
    /// What the invite joins the redeemer to.
    pub target: InviteTarget,
    /// The role the joiner gets.
    pub role: Role,
    /// The time, in milliseconds, from which the invite is refused.
    pub expires_ms: u64,
    /// The most redemptions the invite admits, or `None` for uncapped (a deliberate
    /// act for a genuinely public link, never the link default).
    pub uses_cap: Option<u32>,
    /// How much re-inviting power the redeemer gets; strictly decreases along a
    /// chain (§2).
    pub depth_remaining: u8,
    /// The digest of the invite the minter redeemed, or `None` for a source invite.
    pub parent_digest: Option<[u8; 32]>,
    /// The one redeemer the invite is pinned to (§3), or `None` for a bearer invite.
    pub redeemer: Option<[u8; 32]>,
}

impl InviteTerms {
    /// A source invite's terms: depth from the source's default, no use cap, no
    /// parent, no pin. The inviter sets the expiry.
    #[must_use]
    pub const fn source(
        source: InviteSource,
        target: InviteTarget,
        role: Role,
        expires_ms: u64,
    ) -> Self {
        Self {
            target,
            role,
            expires_ms,
            uses_cap: None,
            depth_remaining: source.default_depth(),
            parent_digest: None,
            redeemer: None,
        }
    }

    /// A link invite's terms (§3): a 7-day expiry and a 25-use cap land when the
    /// inviter sets neither; depth comes from the source's default (§2).
    #[must_use]
    pub const fn link(source: InviteSource, target: InviteTarget, role: Role, now_ms: u64) -> Self {
        Self {
            expires_ms: now_ms.saturating_add(LINK_DEFAULT_LIFETIME_MS),
            uses_cap: Some(LINK_DEFAULT_USES_CAP),
            ..Self::source(source, target, role, 0)
        }
    }

    /// Sets the depth, for a policy that raises (or lowers) a source invite's
    /// default. A source depth over [`MAX_SOURCE_DEPTH`] is refused (§2).
    ///
    /// # Errors
    ///
    /// Fails if `depth_remaining` exceeds [`MAX_SOURCE_DEPTH`].
    pub fn with_depth(mut self, depth_remaining: u8) -> Result<Self> {
        ensure!(
            depth_remaining <= MAX_SOURCE_DEPTH,
            "a source invite's depth is {depth_remaining}, over the cap of {MAX_SOURCE_DEPTH}"
        );
        self.depth_remaining = depth_remaining;
        Ok(self)
    }
}

/// An invite the inviter has signed for one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteEvent {
    /// The signed fields.
    pub body: InviteBody,
    /// Ed25519 over the body's message, verified with `verify_strict`.
    pub signature: [u8; 64],
}

/// The fields the inviter signs.
///
/// A body read from a v1 event carries the legacy terms — a group target, no
/// use cap, depth 0, no parent, no pin — and keeps the v1 wire form when
/// re-serialized, so its signature still verifies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteBody {
    /// The signing key: the inviter's account key.
    pub inviter: [u8; 32],
    /// The digest of the ticket's canonical bytes. The string form is not used, so two strings
    /// that parse to the same ticket give the same digest.
    pub ticket_digest: [u8; 32],
    /// What the invite joins the redeemer to.
    pub target: InviteTarget,
    /// The role the joiner gets.
    pub role: Role,
    /// The time, in milliseconds, from which the invite is refused.
    pub expires_ms: u64,
    /// The most redemptions the invite admits, or `None` for uncapped.
    pub uses_cap: Option<u32>,
    /// How much re-inviting power the redeemer gets (§2).
    pub depth_remaining: u8,
    /// The digest of the invite the minter redeemed, or `None` for a source invite.
    pub parent_digest: Option<[u8; 32]>,
    /// The one redeemer the invite is pinned to (§3), or `None` for a bearer invite.
    pub redeemer: Option<[u8; 32]>,
    /// Read from (or minted as) the v1 fixed layout. Not part of the v2 encoding:
    /// it selects the wire form and the signing domain.
    #[serde(skip)]
    legacy_v1: bool,
}

impl InviteEvent {
    /// Signs an invite for `ticket`, to `group` with `role`, expiring at `expires_ms`.
    /// The event is minted in the v2 form with no terms beyond the target, the role
    /// and the expiry; [`InviteEvent::sign_terms`] mints with the #5751 terms.
    #[must_use]
    pub fn sign(
        key: &SigningKey,
        group: &[u8; 32],
        role: Role,
        expires_ms: u64,
        ticket: &EndpointTicket,
    ) -> Self {
        let body = InviteBody {
            inviter: key.verifying_key().to_bytes(),
            ticket_digest: ticket_digest(ticket),
            target: InviteTarget::Group(*group),
            role,
            expires_ms,
            uses_cap: None,
            depth_remaining: 0,
            parent_digest: None,
            redeemer: None,
            legacy_v1: false,
        };
        let signature = key.sign(&body.message()).to_bytes();
        Self { body, signature }
    }

    /// Signs an invite for `ticket` with the inviter's chosen terms.
    ///
    /// # Errors
    ///
    /// Fails if a source invite's depth exceeds [`MAX_SOURCE_DEPTH`] (§2), or the
    /// encoded body exceeds [`MAX_BODY_LEN`] (§1).
    pub fn sign_terms(
        key: &SigningKey,
        ticket: &EndpointTicket,
        terms: InviteTerms,
    ) -> Result<Self> {
        if terms.parent_digest.is_none() {
            ensure!(
                terms.depth_remaining <= MAX_SOURCE_DEPTH,
                "a source invite's depth is {}, over the cap of {MAX_SOURCE_DEPTH}",
                terms.depth_remaining
            );
        }
        let body = InviteBody {
            inviter: key.verifying_key().to_bytes(),
            ticket_digest: ticket_digest(ticket),
            target: terms.target,
            role: terms.role,
            expires_ms: terms.expires_ms,
            uses_cap: terms.uses_cap,
            depth_remaining: terms.depth_remaining,
            parent_digest: terms.parent_digest,
            redeemer: terms.redeemer,
            legacy_v1: false,
        };
        ensure!(
            body.encoded().len() <= MAX_BODY_LEN,
            "an invite body is over the {MAX_BODY_LEN}-byte budget"
        );
        let signature = key.sign(&body.message()).to_bytes();
        Ok(Self { body, signature })
    }

    /// The event's bytes: the body in its wire form, then the signature.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.body.body_bytes().as_slice(), self.signature.as_slice()].concat()
    }

    /// Reads an event written by [`InviteEvent::to_bytes`], in either wire form:
    /// bytes starting with the v2 magic are v2; anything else is read as the v1
    /// fixed layout, whose length must be exact.
    ///
    /// # Errors
    ///
    /// Fails if the form is malformed: a v2 body over the [`MAX_BODY_LEN`] budget,
    /// an unsupported version, a body that does not decode exactly, a v1 length
    /// that is wrong, or a v1 role code this format does not define.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(V2_MAGIC) {
            Self::from_v2_bytes(bytes)
        } else {
            Self::from_v1_bytes(bytes)
        }
    }

    fn from_v2_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= V2_HEADER_LEN + 1 + 64,
            "an invite event is at least {} bytes, not {}",
            V2_HEADER_LEN + 1 + 64,
            bytes.len()
        );
        let version = bytes[V2_MAGIC.len()];
        ensure!(
            version == V2_VERSION,
            "unsupported invite event version {version}"
        );
        let body_bytes = &bytes[V2_HEADER_LEN..bytes.len() - 64];
        ensure!(
            body_bytes.len() <= MAX_BODY_LEN,
            "an invite event body is at most {MAX_BODY_LEN} bytes, not {}",
            body_bytes.len()
        );
        let (body, rest) = postcard::take_from_bytes::<InviteBody>(body_bytes)
            .map_err(|error| anyhow!("the invite event body does not decode: {error}"))?;
        ensure!(
            rest.is_empty(),
            "the invite event body has {} trailing bytes",
            rest.len()
        );
        let signature = bytes[bytes.len() - 64..]
            .first_chunk::<64>()
            .context("the invite event has no signature")?;
        Ok(Self {
            body,
            signature: *signature,
        })
    }

    fn from_v1_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == EVENT_LEN,
            "an invite event is {EVENT_LEN} bytes, not {}",
            bytes.len()
        );
        let (inviter, rest) = bytes
            .split_first_chunk::<32>()
            .context("the invite event has no inviter")?;
        let (ticket_digest, rest) = rest
            .split_first_chunk::<32>()
            .context("the invite event has no ticket digest")?;
        let (group, rest) = rest
            .split_first_chunk::<32>()
            .context("the invite event has no group")?;
        let (&code, rest) = rest.split_first().context("the invite event has no role")?;
        let (expires, rest) = rest
            .split_first_chunk::<8>()
            .context("the invite event has no expiry")?;
        let signature = rest
            .first_chunk::<64>()
            .context("the invite event has no signature")?;
        Ok(Self {
            body: InviteBody {
                inviter: *inviter,
                ticket_digest: *ticket_digest,
                target: InviteTarget::Group(*group),
                role: role_from_code(code)?,
                expires_ms: u64::from_be_bytes(*expires),
                uses_cap: None,
                depth_remaining: 0,
                parent_digest: None,
                redeemer: None,
                legacy_v1: true,
            },
            signature: *signature,
        })
    }

    /// Checks the signature against the signing key the event names.
    ///
    /// This proves only that the named key signed the body. It does not prove that the key is
    /// the one a link names. The caller checks that.
    ///
    /// # Errors
    ///
    /// Fails if the named key is not a valid Ed25519 key, or the signature does not verify.
    pub fn verify_signature(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(&self.body.inviter).map_err(|error| {
            anyhow!("the invite event is signed by a key that is not valid: {error}")
        })?;
        key.verify_strict(
            &self.body.message(),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|error| anyhow!("the invite event has a bad signature: {error}"))
    }
}

impl InviteBody {
    /// The group the invite is anchored to: the target group, or the fronting
    /// group of a world target. This is the group a link names.
    #[must_use]
    pub const fn group(&self) -> [u8; 32] {
        self.target.group()
    }

    /// The digest naming this invite: over the body's wire bytes, behind a domain
    /// tag. A re-invite records it as its `parent_digest`, so the chain back to
    /// the authority source is auditable from the journals (§2).
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        *Hash::digest([DIGEST_DOMAIN, &self.body_bytes()].concat()).as_bytes()
    }

    /// Checks a redeemer against the pin (§3): a pinned invite refuses anyone
    /// else; an unpinned (bearer) invite admits any redeemer.
    ///
    /// # Errors
    ///
    /// Fails if the invite is pinned to a different redeemer.
    pub fn check_redeemer(&self, redeemer: &[u8; 32]) -> Result<()> {
        if let Some(pin) = self.redeemer {
            ensure!(pin == *redeemer, "the invite is pinned to another redeemer");
        }
        Ok(())
    }

    /// Whether the invite admits one more redemption, given how many have already
    /// been recorded: an uncapped invite always does; a capped invite stops at
    /// its cap (§3). The counting itself is the redemption journal's (§5 step 2).
    #[must_use]
    pub fn admits_redemption(&self, redeemed: u32) -> bool {
        self.uses_cap.is_none_or(|cap| redeemed < cap)
    }

    /// The terms for a re-invite minted by this invite's redeemer (§2): the depth
    /// must strictly decrease, and the terms name this invite as their parent.
    ///
    /// # Errors
    ///
    /// Fails if `depth_remaining` is not less than this invite's own depth — an
    /// inviter can never grant more depth than they hold.
    pub fn child_terms(
        &self,
        target: InviteTarget,
        role: Role,
        expires_ms: u64,
        depth_remaining: u8,
        uses_cap: Option<u32>,
        redeemer: Option<[u8; 32]>,
    ) -> Result<InviteTerms> {
        ensure!(
            depth_remaining < self.depth_remaining,
            "a re-invite's depth ({depth_remaining}) must be less than the depth it was minted from ({})",
            self.depth_remaining
        );
        Ok(InviteTerms {
            target,
            role,
            expires_ms,
            uses_cap,
            depth_remaining,
            parent_digest: Some(self.digest()),
            redeemer,
        })
    }

    /// The body's bytes in its wire form, without the signature: the v1 fixed
    /// fields, or the v2 header + postcard encoding.
    fn body_bytes(&self) -> Vec<u8> {
        if self.legacy_v1 {
            return self.fields();
        }
        let mut bytes = Vec::with_capacity(V2_HEADER_LEN + 128);
        bytes.extend_from_slice(V2_MAGIC);
        bytes.push(V2_VERSION);
        bytes.extend_from_slice(&self.encoded());
        bytes
    }

    /// The postcard encoding of the body. Encoding a body of fixed-size fields
    /// and varints does not fail; the empty fallback fails closed, since a
    /// signature over it will not verify against any honestly encoded body.
    fn encoded(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    /// The fixed-layout fields, without the domain tag (the v1 wire form).
    fn fields(&self) -> Vec<u8> {
        [
            self.inviter.as_slice(),
            self.ticket_digest.as_slice(),
            self.group().as_slice(),
            [role_code(self.role)].as_slice(),
            self.expires_ms.to_be_bytes().as_slice(),
        ]
        .concat()
    }

    fn message(&self) -> Vec<u8> {
        if self.legacy_v1 {
            [DOMAIN_V1, &self.fields()].concat()
        } else {
            [DOMAIN_V2, &self.encoded()].concat()
        }
    }
}

const fn role_code(role: Role) -> u8 {
    match role {
        Role::Pull => 0,
        Role::Read => 1,
        Role::Write => 2,
        Role::Manage => 3,
        Role::Admin => 4,
    }
}

fn role_from_code(code: u8) -> Result<Role> {
    Ok(match code {
        0 => Role::Pull,
        1 => Role::Read,
        2 => Role::Write,
        3 => Role::Manage,
        4 => Role::Admin,
        other => bail!("unknown role code {other} in an invite event"),
    })
}

/// The digest of a ticket's canonical bytes.
pub(super) fn ticket_digest(ticket: &EndpointTicket) -> [u8; 32] {
    *Hash::digest([TICKET_DOMAIN, &ticket.encode_bytes()].concat()).as_bytes()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use id_core::test_forgeries::{IDENTITY_KEY, identity_forgery, with_non_canonical_s};

    fn body_for(key: &SigningKey) -> InviteBody {
        InviteBody {
            inviter: key.verifying_key().to_bytes(),
            ticket_digest: [3; 32],
            target: InviteTarget::Group([4; 32]),
            role: Role::Read,
            expires_ms: 1_800_000_000_000,
            uses_cap: None,
            depth_remaining: 0,
            parent_digest: None,
            redeemer: None,
            legacy_v1: false,
        }
    }

    fn signed(body: InviteBody, key: &SigningKey) -> InviteEvent {
        let signature = key.sign(&body.message()).to_bytes();
        InviteEvent { body, signature }
    }

    /// A v1 event for `key`, built byte by byte in the fixed layout — the writer
    /// for this form no longer exists, so the test pins the form itself.
    fn v1_bytes(key: &SigningKey) -> Vec<u8> {
        let fields = [
            key.verifying_key().to_bytes().as_slice(),
            [3; 32].as_slice(),
            [4; 32].as_slice(),
            [2].as_slice(),
            1_800_000_000_000_u64.to_be_bytes().as_slice(),
        ]
        .concat();
        let signature = key.sign(&[DOMAIN_V1, &fields].concat()).to_bytes();
        [fields, signature.to_vec()].concat()
    }

    #[test]
    fn verify_signature_rejects_the_small_order_identity_forgery() {
        // #1386: the identity point is a small-order key, and the shared
        // forgery (R = identity, S = 0) passes plain verify for any
        // message under it — only verify_strict keeps it out.
        let event = InviteEvent {
            body: InviteBody {
                inviter: IDENTITY_KEY,
                ticket_digest: [3; 32],
                target: InviteTarget::Group([4; 32]),
                role: Role::Read,
                expires_ms: 1_800_000_000_000,
                uses_cap: None,
                depth_remaining: 0,
                parent_digest: None,
                redeemer: None,
                legacy_v1: false,
            },
            signature: identity_forgery(),
        };
        assert!(event.verify_signature().is_err());
    }

    #[test]
    fn verify_signature_rejects_a_non_canonical_s() {
        // #1386: a genuine signature mutated to S + L must be refused,
        // while the original verifies.
        let key = SigningKey::from_bytes(&[7; 32]);
        let body = body_for(&key);
        let signature = key.sign(&body.message()).to_bytes();
        let good = InviteEvent {
            body: body.clone(),
            signature,
        };
        assert!(good.verify_signature().is_ok());
        let malleated = InviteEvent {
            body,
            signature: with_non_canonical_s(signature),
        };
        assert!(malleated.verify_signature().is_err());
    }

    #[test]
    fn a_body_carrying_every_term_round_trips_byte_identically() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let body = InviteBody {
            target: InviteTarget::World {
                world: [9; 32],
                fronting_group: [4; 32],
            },
            uses_cap: Some(3),
            depth_remaining: 2,
            parent_digest: Some([5; 32]),
            redeemer: Some([6; 32]),
            ..body_for(&key)
        };
        let event = signed(body, &key);
        let bytes = event.to_bytes();
        assert!(event.body.encoded().len() <= MAX_BODY_LEN);
        assert_eq!(InviteEvent::from_bytes(&bytes).unwrap(), event);
        assert!(event.verify_signature().is_ok());
        assert_eq!(
            event.body.group(),
            [4; 32],
            "a world target anchors to its fronting group"
        );
    }

    #[test]
    fn a_v1_event_still_parses_with_legacy_terms() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let bytes = v1_bytes(&key);
        let event = InviteEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.body.target, InviteTarget::Group([4; 32]));
        assert_eq!(event.body.role, Role::Write);
        assert_eq!(event.body.expires_ms, 1_800_000_000_000);
        assert_eq!(event.body.uses_cap, None);
        assert_eq!(event.body.depth_remaining, 0);
        assert_eq!(event.body.parent_digest, None);
        assert_eq!(event.body.redeemer, None);
        assert!(event.verify_signature().is_ok());
        assert_eq!(
            event.to_bytes(),
            bytes,
            "a v1 event re-serializes in its own form"
        );
    }

    #[test]
    fn a_body_over_budget_is_refused_at_decode() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(V2_MAGIC);
        bytes.push(V2_VERSION);
        bytes.extend_from_slice(&vec![0_u8; MAX_BODY_LEN + 1]);
        bytes.extend_from_slice(&[0_u8; 64]);
        let error = InviteEvent::from_bytes(&bytes).unwrap_err();
        assert!(format!("{error:#}").contains("512"), "{error:#}");
    }

    #[test]
    fn a_pinned_invite_refuses_another_redeemer() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let pinned = InviteBody {
            redeemer: Some([6; 32]),
            ..body_for(&key)
        };
        assert!(pinned.check_redeemer(&[6; 32]).is_ok());
        assert!(pinned.check_redeemer(&[8; 32]).is_err());
        let bearer = body_for(&key);
        assert!(bearer.check_redeemer(&[8; 32]).is_ok());
    }

    #[test]
    fn depth_never_increases_along_a_re_mint() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let parent = InviteBody {
            depth_remaining: 1,
            ..body_for(&key)
        };
        let child = parent
            .child_terms(
                InviteTarget::Group([4; 32]),
                Role::Read,
                1_800_000_000_000,
                0,
                None,
                None,
            )
            .unwrap();
        assert_eq!(child.depth_remaining, 0);
        assert_eq!(child.parent_digest, Some(parent.digest()));
        assert!(
            parent
                .child_terms(
                    InviteTarget::Group([4; 32]),
                    Role::Read,
                    1_800_000_000_000,
                    1,
                    None,
                    None
                )
                .is_err(),
            "the same depth is already too much"
        );
        let leaf = InviteBody {
            depth_remaining: 0,
            ..body_for(&key)
        };
        assert!(
            leaf.child_terms(
                InviteTarget::Group([4; 32]),
                Role::Read,
                1_800_000_000_000,
                0,
                None,
                None
            )
            .is_err(),
            "a leaf has no depth to give"
        );
    }

    #[test]
    fn link_and_source_defaults_land() {
        let now = 1_800_000_000_000_u64;
        let link = InviteTerms::link(
            InviteSource::OpenDelegation,
            InviteTarget::Group([4; 32]),
            Role::Read,
            now,
        );
        assert_eq!(link.expires_ms, now + LINK_DEFAULT_LIFETIME_MS);
        assert_eq!(link.uses_cap, Some(LINK_DEFAULT_USES_CAP));
        assert_eq!(
            link.depth_remaining, 1,
            "open delegation defaults to depth 1"
        );
        let granted = InviteTerms::link(
            InviteSource::ModeratorGrant,
            InviteTarget::Group([4; 32]),
            Role::Read,
            now,
        );
        assert_eq!(
            granted.depth_remaining, 0,
            "a moderator grant defaults to depth 0"
        );
        let raised = InviteTerms::source(
            InviteSource::OpenDelegation,
            InviteTarget::Group([4; 32]),
            Role::Read,
            now,
        )
        .with_depth(MAX_SOURCE_DEPTH)
        .unwrap();
        assert_eq!(raised.depth_remaining, 3);
        assert!(
            InviteTerms::source(
                InviteSource::OpenDelegation,
                InviteTarget::Group([4; 32]),
                Role::Read,
                now,
            )
            .with_depth(MAX_SOURCE_DEPTH + 1)
            .is_err()
        );
    }

    #[test]
    fn a_use_cap_stops_redemption_at_the_cap() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let capped = InviteBody {
            uses_cap: Some(2),
            ..body_for(&key)
        };
        assert!(capped.admits_redemption(0));
        assert!(capped.admits_redemption(1));
        assert!(!capped.admits_redemption(2));
        assert!(body_for(&key).admits_redemption(u32::MAX - 1));
    }
}
