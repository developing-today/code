//! The freshness token and its frontier check (#13094; design #9128
//! §6, `doc/2026-11-23_design_id9128_social_app_reference_model.md`).
//!
//! Forum and feed views are polled far more often than they change, so
//! a view carries a **freshness token**: a small, server-signed
//! statement of the view's input frontier at issue time. On the next
//! poll the client presents the token instead of re-requesting the
//! view, and [`check_freshness`] — a pure function of the presented
//! token and the server's current state — answers one of:
//!
//! - [`FreshnessVerdict::NothingNew`]: the frontier is unchanged;
//!   the server re-issues the token with a fresh `issued_at` (the
//!   **latest-check** proof of §6.3).
//! - [`FreshnessVerdict::Delta`]: the frontier advanced; the verdict
//!   names exactly the advanced inputs with the token's per-input
//!   positions, so the server reads entries after those positions
//!   (append-read with `after-seq`, #9127 §3) and the client never
//!   refetches the unchanged prefix. A token minted at a past
//!   frontier — the **since-date** form of §6.4 — is just an old
//!   token checked the same way; server order, not wall clock, is
//!   the authority.
//! - [`FreshnessVerdict::Resync`]: the token's frontier is no longer
//!   reachable (compaction aged it out, a tombstone-set rewrite or a
//!   history rewrite moved a component backwards, the view's input
//!   set changed). The client refetches the view once and receives
//!   a fresh token. Resync is a correctness fallback, not an error.
//! - [`FreshnessVerdict::NoToken`]: no token was presented, or the
//!   token failed signature or view verification. Per §6.5 a forged
//!   or foreign token is treated as no token (full view fetch),
//!   never as an error to the client.
//!
//! The third form of §6.4, **before-date**, pins a historical
//! frontier to render against; rendering is the view builder's job
//! (#13096), which consumes the [`Frontier`] and [`FreshnessToken`]
//! types defined here. Per-subtree tokens (§7) are tokens over a
//! subtree's inputs, minted and checked by exactly this code.
//!
//! This module is pure logic: no I/O, no store access, no clock. The
//! server supplies the current frontier and the oldest frontier it
//! can still serve deltas from (`oldest_reachable`, its retention
//! floor); everything else is comparison. Signing follows the Part A
//! conventions: ed25519, a fixed domain prefix on the signature
//! preimage, and the deterministic CBOR profile of
//! [`crate::canonical_cbor`], in which every field is critical — a
//! reader rejects any field it does not know.
//!
//! Tokens do not expire by time (§8.4/§8.5 of the record): a token
//! becomes unusable only when its frontier is unreachable.
//! Federated views compose per-server tokens client-side; a token
//! proves something only about its own server's view, which is why
//! the check takes the expected server identity.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};

/// Fixed prefix of the token signature preimage, ending in a NUL so
/// no domain text can extend it (the Part A preimage convention).
pub const TOKEN_SIGNING_DOMAIN: &[u8] = b"id/freshness-token/v1\0";

/// The identity of a view: the hash of its canonical view definition
/// (board/feed id + projection + policy revision, §6.1). A policy
/// revision change therefore changes the view id, and a token minted
/// under the old revision fails view verification — treated as no
/// token, per §6.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewId(pub [u8; 32]);

impl ViewId {
    /// The view id of a canonical view definition's bytes.
    #[must_use]
    pub fn of_definition(canonical_definition: &[u8]) -> Self {
        Self(*blake3::hash(canonical_definition).as_bytes())
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The name of one input of a view: an append-list attribute path
/// for a list input, a projection name for a relation input. Names
/// are the view definition's own; the frontier orders them by name
/// so the canonical encoding is stable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputId(String);

impl InputId {
    /// A named input.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The input's name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for InputId {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl From<String> for InputId {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl fmt::Display for InputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The high-water mark of one view input (§6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputFrontier {
    /// An append-list input: the accepted head sequence (#9127 §4)
    /// and the revision of the tombstone set (#9130) that filters
    /// the list.
    List {
        /// The accepted head sequence: entries at or below it are
        /// the list as the token saw it.
        head_seq: u64,
        /// The tombstone-set revision filtering the list.
        tombstone_revision: u64,
    },
    /// A relation input: the projection checkpoint of the triples
    /// adapter (#9125's `checkpoint`).
    Relation {
        /// The projection checkpoint.
        checkpoint: u64,
    },
}

impl InputFrontier {
    /// A list input's mark.
    #[must_use]
    pub const fn list(head_seq: u64, tombstone_revision: u64) -> Self {
        Self::List {
            head_seq,
            tombstone_revision,
        }
    }

    /// A relation input's mark.
    #[must_use]
    pub const fn relation(checkpoint: u64) -> Self {
        Self::Relation { checkpoint }
    }

    /// Component-wise comparison with another mark for the same
    /// input: `Some(Ordering)` when the two are comparable (both
    /// components move the same way, or one side equals), `None`
    /// when they diverged (one component advanced while the other
    /// regressed) or the input changed kind.
    #[must_use]
    pub fn compare(&self, other: &Self) -> Option<Ordering> {
        match (*self, *other) {
            (
                Self::List {
                    head_seq: h1,
                    tombstone_revision: t1,
                },
                Self::List {
                    head_seq: h2,
                    tombstone_revision: t2,
                },
            ) => product_ordering((h1, t1), (h2, t2)),
            (Self::Relation { checkpoint: c1 }, Self::Relation { checkpoint: c2 }) => {
                Some(c1.cmp(&c2))
            }
            _ => None,
        }
    }
}

/// The product order on two counters: less-or-equal iff both
/// components are, greater-or-equal iff both are, incomparable when
/// they move in opposite directions.
fn product_ordering(a: (u64, u64), b: (u64, u64)) -> Option<Ordering> {
    let le = a.0 <= b.0 && a.1 <= b.1;
    let ge = a.0 >= b.0 && a.1 >= b.1;
    match (le, ge) {
        (true, true) => Some(Ordering::Equal),
        (true, false) => Some(Ordering::Less),
        (false, true) => Some(Ordering::Greater),
        (false, false) => None,
    }
}

/// A view's input frontier: the vector of high-water marks of the
/// view's inputs (§6.2). Two frontiers for the same view are
/// comparable component-wise; a frontier is **unchanged** iff every
/// component equals the current one. A frontier is digest-sized per
/// input — comparing it is a lookup of current heads, never a scan
/// of entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontier {
    inputs: BTreeMap<InputId, InputFrontier>,
}

impl Frontier {
    /// An empty frontier (a view with no inputs).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one input's mark, replacing any previous mark for
    /// that input; returns the replaced mark, if any.
    pub fn insert(&mut self, input: InputId, mark: InputFrontier) -> Option<InputFrontier> {
        self.inputs.insert(input, mark)
    }

    /// Build a frontier from `(input, mark)` pairs.
    #[must_use]
    pub fn from_inputs(pairs: impl IntoIterator<Item = (InputId, InputFrontier)>) -> Self {
        Self {
            inputs: pairs.into_iter().collect(),
        }
    }

    /// One input's mark, if the frontier covers that input.
    #[must_use]
    pub fn get(&self, input: &InputId) -> Option<InputFrontier> {
        self.inputs.get(input).copied()
    }

    /// Whether the frontier covers no inputs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }

    /// How many inputs the frontier covers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inputs.len()
    }

    /// The covered inputs and their marks, in input-name order.
    pub fn iter(&self) -> impl Iterator<Item = (&InputId, &InputFrontier)> {
        self.inputs.iter()
    }

    /// Component-wise comparison (§6.2): `Some(Ordering::Equal)`
    /// iff every component equals; `Some(Ordering::Less)` iff every
    /// component is at or below the other's and at least one is
    /// strictly below; `Some(Ordering::Greater)` symmetrically;
    /// `None` when the frontiers are incomparable — different input
    /// sets, an input that changed kind, or components that moved
    /// in opposite directions (diverged histories).
    #[must_use]
    pub fn compare(&self, other: &Self) -> Option<Ordering> {
        if self.inputs.len() != other.inputs.len() {
            return None;
        }
        let mut combined = Ordering::Equal;
        for (input, mark) in &self.inputs {
            let other_mark = other.inputs.get(input)?;
            let component = mark.compare(other_mark)?;
            combined = match (combined, component) {
                (Ordering::Equal, c) => c,
                (c, Ordering::Equal) => c,
                (a, b) if a == b => a,
                _ => return None,
            };
        }
        Some(combined)
    }
}

impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.compare(other)
    }
}

/// A freshness token (§6.1): a server-signed statement that a view
/// stood at a frontier at issue time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshnessToken {
    /// The view this token speaks for.
    pub view_id: ViewId,
    /// The view's input frontier at issue time.
    pub frontier: Frontier,
    /// Server time of the check that issued the token, in
    /// milliseconds since the Unix epoch. Informational only:
    /// tokens do not expire by time (§8.5), and server order, not
    /// wall clock, is the authority (§6.4).
    pub issued_at: u64,
    /// The issuing server's identity.
    pub server: VerifyingKey,
    /// The server's signature over the fields above.
    pub signature: Signature,
}

impl FreshnessToken {
    /// Issue a token: sign the statement that `view_id` stood at
    /// `frontier` at `issued_at`, under the server's key.
    ///
    /// # Errors
    ///
    /// Returns the canonical-encoding error if the statement cannot
    /// be encoded in the canonical profile. The values a caller can
    /// construct are always in the profile's closed model, so this
    /// is unreachable in practice; it is propagated, never panicked.
    pub fn issue(
        view_id: ViewId,
        frontier: Frontier,
        issued_at: u64,
        signing_key: &SigningKey,
    ) -> Result<Self, CborError> {
        let unsigned = Self {
            view_id,
            frontier,
            issued_at,
            server: signing_key.verifying_key(),
            signature: Signature::from_bytes(&[0u8; 64]),
        };
        let signature = signing_key.sign(&unsigned.signing_bytes()?);
        Ok(Self {
            signature,
            ..unsigned
        })
    }

    /// The bytes the signature covers: the fixed domain prefix, then
    /// the canonical encoding of the signed fields.
    ///
    /// # Errors
    ///
    /// As [`FreshnessToken::issue`].
    pub fn signing_bytes(&self) -> Result<Vec<u8>, CborError> {
        let mut bytes = Vec::with_capacity(TOKEN_SIGNING_DOMAIN.len() + 128);
        bytes.extend_from_slice(TOKEN_SIGNING_DOMAIN);
        bytes.extend_from_slice(&encode_canonical(&self.signed_value())?);
        Ok(bytes)
    }

    /// Whether the signature verifies against the token's server
    /// key over the signed fields. A token whose fields cannot be
    /// encoded cannot verify.
    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let Ok(bytes) = self.signing_bytes() else {
            return false;
        };
        self.server.verify_strict(&bytes, &self.signature).is_ok()
    }

    /// The token's full canonical encoding, signature included —
    /// the form a client stores and presents on the next poll.
    ///
    /// # Errors
    ///
    /// As [`FreshnessToken::issue`].
    pub fn encode(&self) -> Result<Vec<u8>, CborError> {
        let mut entries = self.signed_entries();
        entries.push((
            Value::Text("signature".to_owned()),
            Value::Bytes(self.signature.to_bytes().to_vec()),
        ));
        encode_canonical(&Value::Map(entries))
    }

    /// Decode a token from its canonical encoding. Every field is
    /// critical: an unknown field, a missing field, or a value of
    /// the wrong shape is a rejection, never a silent drop.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::NotCanonical`] for bytes outside the
    /// canonical profile and [`DecodeError::Malformed`] for a
    /// canonical value that is not a freshness token.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = decode_canonical(bytes).map_err(DecodeError::NotCanonical)?;
        let Value::Map(entries) = value else {
            return Err(DecodeError::Malformed("token is not a map"));
        };
        let mut view_id = None;
        let mut frontier = None;
        let mut issued_at = None;
        let mut server = None;
        let mut signature = None;
        for (key, field) in &entries {
            let Value::Text(name) = key else {
                return Err(DecodeError::Malformed("token key is not text"));
            };
            match name.as_str() {
                "view_id" => view_id = Some(decode_view_id(field)?),
                "frontier" => frontier = Some(decode_frontier(field)?),
                "issued_at" => issued_at = Some(decode_u64(field, "issued_at")?),
                "server" => server = Some(decode_verifying_key(field)?),
                "signature" => signature = Some(decode_signature(field)?),
                _ => return Err(DecodeError::Malformed("unknown token field")),
            }
        }
        Ok(Self {
            view_id: view_id.ok_or(DecodeError::Malformed("missing view_id"))?,
            frontier: frontier.ok_or(DecodeError::Malformed("missing frontier"))?,
            issued_at: issued_at.ok_or(DecodeError::Malformed("missing issued_at"))?,
            server: server.ok_or(DecodeError::Malformed("missing server"))?,
            signature: signature.ok_or(DecodeError::Malformed("missing signature"))?,
        })
    }

    /// The signed fields as a canonical value (the signature field
    /// itself is never signed).
    fn signed_value(&self) -> Value {
        Value::Map(self.signed_entries())
    }

    /// The signed fields as map entries.
    fn signed_entries(&self) -> Vec<(Value, Value)> {
        vec![
            (
                Value::Text("view_id".to_owned()),
                Value::Bytes(self.view_id.0.to_vec()),
            ),
            (
                Value::Text("frontier".to_owned()),
                frontier_value(&self.frontier),
            ),
            (
                Value::Text("issued_at".to_owned()),
                Value::UInt(self.issued_at),
            ),
            (
                Value::Text("server".to_owned()),
                Value::Bytes(self.server.to_bytes().to_vec()),
            ),
        ]
    }
}

/// Why a byte string is not a freshness token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes are outside the canonical CBOR profile.
    NotCanonical(CborError),
    /// The bytes are canonical but not shaped like a token. The
    /// text names the problem and carries no decoded value.
    Malformed(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCanonical(error) => write!(f, "not canonical CBOR: {error}"),
            Self::Malformed(problem) => write!(f, "malformed freshness token: {problem}"),
        }
    }
}

impl std::error::Error for DecodeError {}

fn frontier_value(frontier: &Frontier) -> Value {
    Value::Map(
        frontier
            .iter()
            .map(|(input, mark)| {
                let component = match *mark {
                    InputFrontier::List {
                        head_seq,
                        tombstone_revision,
                    } => Value::Map(vec![
                        (
                            Value::Text("kind".to_owned()),
                            Value::Text("list".to_owned()),
                        ),
                        (Value::Text("head_seq".to_owned()), Value::UInt(head_seq)),
                        (
                            Value::Text("tombstone_revision".to_owned()),
                            Value::UInt(tombstone_revision),
                        ),
                    ]),
                    InputFrontier::Relation { checkpoint } => Value::Map(vec![
                        (
                            Value::Text("kind".to_owned()),
                            Value::Text("relation".to_owned()),
                        ),
                        (
                            Value::Text("checkpoint".to_owned()),
                            Value::UInt(checkpoint),
                        ),
                    ]),
                };
                (Value::Text(input.as_str().to_owned()), component)
            })
            .collect(),
    )
}

fn decode_u64(value: &Value, field: &'static str) -> Result<u64, DecodeError> {
    match value {
        Value::UInt(n) => Ok(*n),
        _ => Err(DecodeError::Malformed(field)),
    }
}

fn decode_view_id(value: &Value) -> Result<ViewId, DecodeError> {
    let Value::Bytes(bytes) = value else {
        return Err(DecodeError::Malformed("view_id is not bytes"));
    };
    let digest: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_error| DecodeError::Malformed("view_id is not 32 bytes"))?;
    Ok(ViewId(digest))
}

fn decode_verifying_key(value: &Value) -> Result<VerifyingKey, DecodeError> {
    let Value::Bytes(bytes) = value else {
        return Err(DecodeError::Malformed("server is not bytes"));
    };
    let key_bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_error| DecodeError::Malformed("server is not 32 bytes"))?;
    VerifyingKey::from_bytes(&key_bytes)
        .map_err(|_error| DecodeError::Malformed("server is not a valid key"))
}

fn decode_signature(value: &Value) -> Result<Signature, DecodeError> {
    let Value::Bytes(bytes) = value else {
        return Err(DecodeError::Malformed("signature is not bytes"));
    };
    let sig_bytes: [u8; 64] = bytes
        .as_slice()
        .try_into()
        .map_err(|_error| DecodeError::Malformed("signature is not 64 bytes"))?;
    Ok(Signature::from_bytes(&sig_bytes))
}

fn decode_frontier(value: &Value) -> Result<Frontier, DecodeError> {
    let Value::Map(entries) = value else {
        return Err(DecodeError::Malformed("frontier is not a map"));
    };
    let mut frontier = Frontier::new();
    for (key, component) in entries {
        let Value::Text(name) = key else {
            return Err(DecodeError::Malformed("frontier key is not text"));
        };
        let mark = decode_input_frontier(component)?;
        if frontier.insert(InputId::new(name.clone()), mark).is_some() {
            return Err(DecodeError::Malformed("duplicate frontier input"));
        }
    }
    Ok(frontier)
}

fn decode_input_frontier(value: &Value) -> Result<InputFrontier, DecodeError> {
    let Value::Map(entries) = value else {
        return Err(DecodeError::Malformed("input frontier is not a map"));
    };
    let mut kind = None;
    let mut head_seq = None;
    let mut tombstone_revision = None;
    let mut checkpoint = None;
    for (key, field) in entries {
        let Value::Text(name) = key else {
            return Err(DecodeError::Malformed("input frontier key is not text"));
        };
        match name.as_str() {
            "kind" => {
                let Value::Text(text) = field else {
                    return Err(DecodeError::Malformed("kind is not text"));
                };
                kind = Some(text.as_str());
            }
            "head_seq" => head_seq = Some(decode_u64(field, "head_seq")?),
            "tombstone_revision" => {
                tombstone_revision = Some(decode_u64(field, "tombstone_revision")?);
            }
            "checkpoint" => checkpoint = Some(decode_u64(field, "checkpoint")?),
            _ => return Err(DecodeError::Malformed("unknown input frontier field")),
        }
    }
    match kind {
        Some("list") => {
            if checkpoint.is_some() {
                return Err(DecodeError::Malformed("list input carries a checkpoint"));
            }
            Ok(InputFrontier::list(
                head_seq.ok_or(DecodeError::Malformed("list input missing head_seq"))?,
                tombstone_revision.ok_or(DecodeError::Malformed(
                    "list input missing tombstone_revision",
                ))?,
            ))
        }
        Some("relation") => {
            if head_seq.is_some() || tombstone_revision.is_some() {
                return Err(DecodeError::Malformed("relation input carries list fields"));
            }
            Ok(InputFrontier::relation(checkpoint.ok_or(
                DecodeError::Malformed("relation input missing checkpoint"),
            )?))
        }
        _ => Err(DecodeError::Malformed("unknown input frontier kind")),
    }
}

/// One advanced input of a [`FreshnessVerdict::Delta`]: read this
/// input's entries after the token's position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputAdvance {
    /// The input that advanced.
    pub input: InputId,
    /// The token's mark for the input: the delta starts after it
    /// (for a list input, the append-read `after-seq` is its
    /// `head_seq`, #9127 §3).
    pub after: InputFrontier,
}

/// The answer of the freshness check (§6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshnessVerdict {
    /// No usable token: none was presented, or the presented token
    /// failed signature or view verification (forged, foreign, or
    /// for another view). The caller fetches the full view. Never
    /// an error (§6.5).
    NoToken,
    /// The frontier is unchanged; what the client holds is exactly
    /// current. The server re-issues the token with a fresh
    /// `issued_at`.
    NothingNew,
    /// The frontier advanced. `advanced` names exactly the inputs
    /// whose marks differ from the token's, each with the token's
    /// position to read after; every other input is unchanged.
    Delta {
        /// The advanced inputs, in input-name order.
        advanced: Vec<InputAdvance>,
    },
    /// The token's frontier is no longer reachable: it aged out
    /// past the retention floor, claims state the current frontier
    /// does not dominate (a rewritten history), diverged from it,
    /// or covers a different input set. The client refetches the
    /// view once and receives a fresh token.
    Resync,
}

/// The freshness check (§6.3) as a pure function.
///
/// `token` is the presented token, if any; `view_id` and `server`
/// are the view being polled and the identity the caller expects
/// the token to speak for; `current` is the view's frontier now;
/// `oldest_reachable` is the oldest frontier the server can still
/// serve deltas from (its retention floor: a token at or above it
/// on every component can be answered with a delta).
///
/// A token failing signature or view verification is treated as no
/// token, never an error (§6.5). Verification order is view, then
/// server, then signature; every failure is the same verdict.
#[must_use]
pub fn check_freshness(
    token: Option<&FreshnessToken>,
    view_id: &ViewId,
    server: &VerifyingKey,
    current: &Frontier,
    oldest_reachable: &Frontier,
) -> FreshnessVerdict {
    let Some(token) = token else {
        return FreshnessVerdict::NoToken;
    };
    if token.view_id != *view_id || token.server != *server || !token.verify_signature() {
        return FreshnessVerdict::NoToken;
    }
    let held = &token.frontier;
    if held == current {
        return FreshnessVerdict::NothingNew;
    }
    let above_floor = matches!(
        oldest_reachable.compare(held),
        Some(Ordering::Less | Ordering::Equal)
    );
    let below_current = matches!(
        held.compare(current),
        Some(Ordering::Less | Ordering::Equal)
    );
    if !above_floor || !below_current {
        return FreshnessVerdict::Resync;
    }
    let advanced = held
        .iter()
        .filter(|(input, mark)| current.get(input) != Some(**mark))
        .map(|(input, mark)| InputAdvance {
            input: input.clone(),
            after: *mark,
        })
        .collect();
    FreshnessVerdict::Delta { advanced }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;

    const ISSUED_AT: u64 = 1_763_000_000_000;

    fn server_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn view() -> ViewId {
        ViewId::of_definition(b"board:general + projection:thread-tree + policy:3")
    }

    fn other_view() -> ViewId {
        ViewId::of_definition(b"board:general + projection:thread-tree + policy:4")
    }

    /// The standard three-input view: two append-lists and one
    /// relation input.
    fn frontier(topics: (u64, u64), comments: (u64, u64), members: u64) -> Frontier {
        Frontier::from_inputs([
            (
                InputId::new("topics"),
                InputFrontier::list(topics.0, topics.1),
            ),
            (
                InputId::new("comments"),
                InputFrontier::list(comments.0, comments.1),
            ),
            (InputId::new("members"), InputFrontier::relation(members)),
        ])
    }

    /// The retention floor at the origin: everything is reachable.
    fn floor_zero() -> Frontier {
        frontier((0, 0), (0, 0), 0)
    }

    fn token_at(frontier: Frontier) -> FreshnessToken {
        FreshnessToken::issue(view(), frontier, ISSUED_AT, &server_key()).unwrap()
    }

    fn check(
        token: Option<&FreshnessToken>,
        current: &Frontier,
        floor: &Frontier,
    ) -> FreshnessVerdict {
        check_freshness(
            token,
            &view(),
            &server_key().verifying_key(),
            current,
            floor,
        )
    }

    // §6.3 check table, row by row. Oracle: the design record §6.3.

    #[test]
    fn test_no_token_when_absent() {
        let current = frontier((5, 1), (9, 0), 3);
        assert_eq!(
            check(None, &current, &floor_zero()),
            FreshnessVerdict::NoToken
        );
    }

    #[test]
    fn test_nothing_new_at_current_frontier() {
        let current = frontier((5, 1), (9, 0), 3);
        let token = token_at(current.clone());
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::NothingNew
        );
    }

    #[test]
    fn test_delta_names_exactly_the_advanced_list_input() {
        let held = frontier((5, 1), (9, 0), 3);
        let current = frontier((8, 1), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Delta {
                advanced: vec![InputAdvance {
                    input: InputId::new("topics"),
                    after: InputFrontier::list(5, 1),
                }],
            }
        );
    }

    #[test]
    fn test_delta_names_exactly_the_advanced_relation_input() {
        let held = frontier((5, 1), (9, 0), 3);
        let current = frontier((5, 1), (9, 0), 4);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Delta {
                advanced: vec![InputAdvance {
                    input: InputId::new("members"),
                    after: InputFrontier::relation(3),
                }],
            }
        );
    }

    #[test]
    fn test_delta_on_tombstone_revision_alone() {
        // No new entries, but the tombstone set moved: the held view
        // is stale even though every head sequence matches.
        let held = frontier((5, 1), (9, 0), 3);
        let current = frontier((5, 2), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Delta {
                advanced: vec![InputAdvance {
                    input: InputId::new("topics"),
                    after: InputFrontier::list(5, 1),
                }],
            }
        );
    }

    #[test]
    fn test_delta_names_all_advanced_inputs_in_name_order() {
        let held = frontier((5, 1), (9, 0), 3);
        let current = frontier((6, 1), (11, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Delta {
                advanced: vec![
                    InputAdvance {
                        input: InputId::new("comments"),
                        after: InputFrontier::list(9, 0),
                    },
                    InputAdvance {
                        input: InputId::new("topics"),
                        after: InputFrontier::list(5, 1),
                    },
                ],
            }
        );
    }

    #[test]
    fn test_resync_when_token_below_retention_floor() {
        // Compaction aged the token's frontier out: the floor has
        // moved past what the token holds.
        let held = frontier((5, 1), (9, 0), 3);
        let floor = frontier((6, 1), (9, 0), 3);
        let current = frontier((8, 1), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor),
            FreshnessVerdict::Resync
        );
    }

    #[test]
    fn test_delta_when_token_exactly_at_retention_floor() {
        let held = frontier((5, 1), (9, 0), 3);
        let current = frontier((8, 1), (9, 0), 3);
        let token = token_at(held.clone());
        assert_eq!(
            check(Some(&token), &current, &held),
            FreshnessVerdict::Delta {
                advanced: vec![InputAdvance {
                    input: InputId::new("topics"),
                    after: InputFrontier::list(5, 1),
                }],
            }
        );
    }

    #[test]
    fn test_resync_when_token_beyond_current() {
        // The token claims a head the server no longer has: the
        // history it was minted on was rewritten.
        let held = frontier((12, 1), (9, 0), 3);
        let current = frontier((8, 1), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Resync
        );
    }

    #[test]
    fn test_resync_when_frontiers_diverge() {
        // Topics advanced while its tombstone revision regressed (a
        // tombstone-set rewrite): the two frontiers are
        // incomparable, so no delta can be served.
        let held = frontier((5, 3), (9, 0), 3);
        let current = frontier((6, 2), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Resync
        );
    }

    #[test]
    fn test_resync_when_input_set_changed() {
        let held = Frontier::from_inputs([(InputId::new("topics"), InputFrontier::list(5, 1))]);
        let current = frontier((5, 1), (9, 0), 3);
        let token = token_at(held);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::Resync
        );
    }

    #[test]
    fn test_forged_signature_is_no_token() {
        // A token whose frontier was edited after signing fails
        // verification and is treated as no token (§6.5), even
        // though the edited frontier would otherwise be a delta.
        let mut token = token_at(frontier((5, 1), (9, 0), 3));
        token.frontier = frontier((4, 1), (9, 0), 3);
        let current = frontier((8, 1), (9, 0), 3);
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::NoToken
        );
    }

    #[test]
    fn test_wrong_view_is_no_token() {
        // A token for the previous policy revision (a different
        // view_id) fails view verification: no token, never an
        // error (§6.5).
        let current = frontier((5, 1), (9, 0), 3);
        let token =
            FreshnessToken::issue(other_view(), current.clone(), ISSUED_AT, &server_key()).unwrap();
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::NoToken
        );
    }

    #[test]
    fn test_foreign_server_is_no_token() {
        // Another server's token verifies against its own key but
        // does not speak for this server (§6.5).
        let current = frontier((5, 1), (9, 0), 3);
        let token =
            FreshnessToken::issue(view(), current.clone(), ISSUED_AT, &other_key()).unwrap();
        assert!(token.verify_signature());
        assert_eq!(
            check(Some(&token), &current, &floor_zero()),
            FreshnessVerdict::NoToken
        );
    }

    #[test]
    fn test_encode_decode_round_trip() {
        let token = token_at(frontier((5, 1), (9, 0), 3));
        let bytes = token.encode().unwrap();
        let decoded = FreshnessToken::decode(&bytes).unwrap();
        assert_eq!(decoded, token);
        assert!(decoded.verify_signature());
        // The decoded token checks exactly like the original.
        let current = frontier((8, 1), (9, 0), 3);
        assert_eq!(
            check(Some(&decoded), &current, &floor_zero()),
            check(Some(&token), &current, &floor_zero())
        );
    }

    #[test]
    fn test_decode_rejects_garbage_and_unknown_fields() {
        assert!(FreshnessToken::decode(b"not a token").is_err());
        assert!(FreshnessToken::decode(&[]).is_err());

        // An extra field is critical-field rejection, not a drop.
        let token = token_at(frontier((5, 1), (9, 0), 3));
        let Value::Map(mut entries) = token.signed_value() else {
            panic!("signed_value is a map");
        };
        entries.push((
            Value::Text("surprise".to_owned()),
            Value::Bytes(token.signature.to_bytes().to_vec()),
        ));
        entries.push((
            Value::Text("signature".to_owned()),
            Value::Bytes(token.signature.to_bytes().to_vec()),
        ));
        let bytes = encode_canonical(&Value::Map(entries)).unwrap();
        assert!(matches!(
            FreshnessToken::decode(&bytes),
            Err(DecodeError::Malformed(_))
        ));
    }

    #[test]
    fn test_view_id_of_definition_is_stable_and_distinct() {
        assert_eq!(view(), view());
        assert_ne!(view(), other_view());
    }

    #[test]
    fn test_frontier_compare_basics() {
        let a = frontier((5, 1), (9, 0), 3);
        let b = frontier((8, 1), (9, 0), 3);
        assert_eq!(a.compare(&a), Some(Ordering::Equal));
        assert_eq!(a.compare(&b), Some(Ordering::Less));
        assert_eq!(b.compare(&a), Some(Ordering::Greater));
        // Diverged components are incomparable, in both directions.
        let diverged = frontier((8, 0), (9, 0), 3);
        assert_eq!(a.compare(&diverged), None);
        assert_eq!(diverged.compare(&a), None);
        // A kind change is incomparable.
        let kind_changed = Frontier::from_inputs([
            (InputId::new("topics"), InputFrontier::relation(5)),
            (InputId::new("comments"), InputFrontier::list(9, 0)),
            (InputId::new("members"), InputFrontier::relation(3)),
        ]);
        assert_eq!(a.compare(&kind_changed), None);
    }

    // Property tests. Oracle: the §6.2 component-wise order laws and
    // the §6.3 check table.

    fn frontier_strategy() -> impl proptest::strategy::Strategy<Value = Frontier> {
        use proptest::prelude::*;
        (0..=6u64, 0..=6u64, 0..=6u64, 0..=6u64, 0..=6u64)
            .prop_map(|(t_h, t_t, c_h, c_t, m)| frontier((t_h, t_t), (c_h, c_t), m))
    }

    fn componentwise_min(a: &Frontier, b: &Frontier) -> Frontier {
        Frontier::from_inputs(a.iter().map(|(input, mark)| {
            let other = b.get(input).unwrap();
            let min = match (*mark, other) {
                (
                    InputFrontier::List {
                        head_seq: h1,
                        tombstone_revision: t1,
                    },
                    InputFrontier::List {
                        head_seq: h2,
                        tombstone_revision: t2,
                    },
                ) => InputFrontier::list(h1.min(h2), t1.min(t2)),
                (
                    InputFrontier::Relation { checkpoint: c1 },
                    InputFrontier::Relation { checkpoint: c2 },
                ) => InputFrontier::relation(c1.min(c2)),
                _ => panic!("same input set and kinds"),
            };
            (input.clone(), min)
        }))
    }

    proptest::proptest! {
        #[test]
        fn prop_frontier_compare_is_reflexive(a in frontier_strategy()) {
            proptest::prop_assert_eq!(a.compare(&a), Some(Ordering::Equal));
            proptest::prop_assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
        }

        #[test]
        fn prop_frontier_compare_is_transitive_consistent(
            a in frontier_strategy(),
            b in frontier_strategy(),
            c in frontier_strategy(),
        ) {
            use proptest::prelude::*;
            let le = |x: &Frontier, y: &Frontier| {
                matches!(x.compare(y), Some(Ordering::Less | Ordering::Equal))
            };
            if le(&a, &b) && le(&b, &c) {
                prop_assert!(le(&a, &c), "transitivity of <=");
            }
            if le(&a, &b) && le(&b, &a) {
                prop_assert_eq!(&a, &b, "antisymmetry of <=");
            }
            // The trait impl and the named method never disagree.
            prop_assert_eq!(a.partial_cmp(&b), a.compare(&b));
        }

        #[test]
        fn prop_check_matches_section_6_3_table(
            held in frontier_strategy(),
            current in frontier_strategy(),
        ) {
            use proptest::prelude::*;
            // A floor at the component-wise minimum keeps every
            // reachable frontier reachable, isolating the
            // held-vs-current half of the table.
            let floor = componentwise_min(&held, &current);
            let token = token_at(held.clone());
            let verdict = check(Some(&token), &current, &floor);
            if held == current {
                prop_assert_eq!(verdict, FreshnessVerdict::NothingNew);
            } else if matches!(
                held.compare(&current),
                Some(Ordering::Less | Ordering::Equal)
            ) {
                let FreshnessVerdict::Delta { advanced } = verdict else {
                    prop_assert!(false, "expected delta, got {verdict:?}");
                    return Ok(());
                };
                // Exactly the inputs whose marks differ, each with
                // the token's position.
                let expected: Vec<InputAdvance> = held
                    .iter()
                    .filter(|(input, mark)| current.get(input) != Some(**mark))
                    .map(|(input, mark)| InputAdvance {
                        input: input.clone(),
                        after: *mark,
                    })
                    .collect();
                prop_assert!(!expected.is_empty());
                prop_assert_eq!(advanced, expected);
            } else {
                prop_assert_eq!(verdict, FreshnessVerdict::Resync);
            }
        }

        #[test]
        fn prop_check_resyncs_when_floor_passes_held(
            held in frontier_strategy(),
            bump in 0..3usize,
        ) {
            use proptest::prelude::*;
            // Advance one input of `held` to make `current`, and put
            // the floor on that same input one step past `held`:
            // the token aged out even though it is below current.
            let names = ["topics", "comments", "members"];
            let name = names[bump];
            let mut current_pairs: Vec<(InputId, InputFrontier)> = held
                .iter()
                .map(|(input, mark)| (input.clone(), *mark))
                .collect();
            let mut floor_pairs = current_pairs.clone();
            for (input, mark) in &mut current_pairs {
                if input.as_str() == name {
                    *mark = match *mark {
                        InputFrontier::List { head_seq, tombstone_revision } =>
                            InputFrontier::list(head_seq + 2, tombstone_revision),
                        InputFrontier::Relation { checkpoint } =>
                            InputFrontier::relation(checkpoint + 2),
                    };
                }
            }
            for (input, mark) in &mut floor_pairs {
                if input.as_str() == name {
                    *mark = match *mark {
                        InputFrontier::List { head_seq, tombstone_revision } =>
                            InputFrontier::list(head_seq + 1, tombstone_revision),
                        InputFrontier::Relation { checkpoint } =>
                            InputFrontier::relation(checkpoint + 1),
                    };
                }
            }
            let current = Frontier::from_inputs(current_pairs);
            let floor = Frontier::from_inputs(floor_pairs);
            let token = token_at(held);
            let verdict = check(Some(&token), &current, &floor);
            prop_assert_eq!(verdict, FreshnessVerdict::Resync);
        }
    }
}
