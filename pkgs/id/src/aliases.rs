//! Alias display over tag claims (Phase 5, P5.7; owner decisions 2026-10-12).
//!
//! An alias is a tag claim named [`ALIAS_NAME`], whose value is the alias text. Its subject is the
//! object's ID: a group's create event, or a user's first identity event (#160, not yet built).
//! This module is the one place that decides what a view shows for an object's aliases.
//!
//! The rules, in order:
//!
//! - A pet name, which this client's owner set, is shown first. It is local and adds no claim.
//! - Otherwise the server's stored primary alias is shown, as one value. Only groups have one for
//!   now, because a user's primary is keyed by account key and no user subject exists yet (#160).
//! - Otherwise, one alias claim is shown alone, and two or more are shown as contested. Nothing is
//!   picked silently. The full set is always in [`Row::aliases`].
//!
//! Nothing is unique. Two objects may claim the same alias, and both are returned. Each row shows a
//! prefix of its ID, the shortest one that no other row in the view shares, so the rows can be told
//! apart. The creation date is not shown, because the signed header has no time field (open).

use std::collections::BTreeMap;

use p2panda_core::Hash;

use crate::server_config::ServerConfig;
use crate::tag_claims::Claim;
use crate::waiting::AuthorKey;

/// The tag name an alias claim carries. Its value is the alias text.
pub const ALIAS_NAME: &str = "alias";

/// One alias claim on an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The claim's ID: the `tag.set` event that made it.
    pub claim: Hash,
    /// The alias text.
    pub text: String,
    /// The key that made the claim.
    pub author: AuthorKey,
}

/// What a view shows as an object's alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// The owner's pet name for the object: a local name that only this client sees.
    PetName(String),
    /// The server's stored primary alias.
    Primary(String),
    /// The only alias claim on the object.
    Only(Alias),
    /// Two or more alias claims, none picked. The full set is in [`Row::aliases`].
    Contested,
    /// No alias claim and no primary alias, so the name stands alone.
    Unnamed,
}

/// An object in a view: its ID and its official name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// The object's subject: the ID its alias claims name.
    pub subject: Hash,
    /// The official name.
    pub name: String,
}

/// One object as a view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The object's subject.
    pub subject: Hash,
    /// The official name.
    pub name: String,
    /// The shortest prefix of the subject's hex ID that no other row in the view shares. When no
    /// prefix is unique, which needs two rows with the same subject, it is the full ID.
    pub prefix: String,
    /// What the view shows as the object's alias.
    pub shown: Shown,
    /// Every alias claim on the object, in claim order.
    pub aliases: Vec<Alias>,
}

/// The alias claims on `subject`, in claim order.
#[must_use]
pub fn aliases_of(_claims: &[Claim], _subject: &Hash) -> Vec<Alias> {
    Vec::new()
}

/// The rows of a view over `named`, with the rules above. `server` is the server config, if this
/// server has one. `pet_names` maps a subject to the owner's pet name for it.
#[must_use]
pub fn rows(
    _named: &[Named],
    _claims: &[Claim],
    _server: Option<&ServerConfig>,
    _pet_names: &BTreeMap<Hash, String>,
) -> Vec<Row> {
    Vec::new()
}
