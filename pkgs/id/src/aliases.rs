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

use crate::server_config::{Object, ServerConfig};
use crate::tag_claims::Claim;
use crate::waiting::AuthorKey;

/// The tag name an alias claim carries. Its value is the alias text.
pub const ALIAS_NAME: &str = "alias";

/// The fewest hex digits a row's prefix has, as git shows short IDs.
const MIN_PREFIX: usize = 7;

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

/// The alias claims on `subject`, in claim order. A bare tag, or an alias with empty text, is not
/// an alias.
#[must_use]
pub fn aliases_of(claims: &[Claim], subject: &Hash) -> Vec<Alias> {
    claims
        .iter()
        .filter(|claim| claim.subject == *subject && claim.name == ALIAS_NAME)
        .filter_map(|claim| {
            let text = claim.value.as_deref().filter(|text| !text.is_empty())?;
            Some(Alias {
                claim: claim.id,
                text: text.to_owned(),
                author: claim.author,
            })
        })
        .collect()
}

/// The rows of a view over `named`, with the rules above. `server` is the server config, if this
/// server has one. `pet_names` maps a subject to the owner's pet name for it.
#[must_use]
pub fn rows(
    named: &[Named],
    claims: &[Claim],
    server: Option<&ServerConfig>,
    pet_names: &BTreeMap<Hash, String>,
) -> Vec<Row> {
    let hexes: Vec<String> = named.iter().map(|object| object.subject.to_hex()).collect();
    named
        .iter()
        .enumerate()
        .map(|(index, object)| {
            let aliases = aliases_of(claims, &object.subject);
            // A group's subject is its create event ID, which is the config's group ID.
            let primary =
                server.and_then(|config| config.primary_alias(&Object::Group(object.subject)));
            let pet = pet_names.get(&object.subject).map(String::as_str);
            Row {
                subject: object.subject,
                name: object.name.clone(),
                prefix: shortest_prefix(index, &hexes),
                shown: shown_for(pet, primary, &aliases),
                aliases,
            }
        })
        .collect()
}

/// What a view shows as one object's alias: the pet name, then the primary, then the claims.
fn shown_for(pet: Option<&str>, primary: Option<&str>, aliases: &[Alias]) -> Shown {
    if let Some(name) = pet {
        return Shown::PetName(name.to_owned());
    }
    if let Some(text) = primary {
        return Shown::Primary(text.to_owned());
    }
    match aliases {
        [] => Shown::Unnamed,
        [only] => Shown::Only(only.clone()),
        _ => Shown::Contested,
    }
}

/// The shortest prefix of `hexes[index]`, at least [`MIN_PREFIX`] digits, that no other entry
/// starts with. The full ID when no prefix is unique.
fn shortest_prefix(index: usize, hexes: &[String]) -> String {
    let own = hexes[index].as_str();
    (MIN_PREFIX..=own.len())
        .map(|length| &own[..length])
        .find(|prefix| {
            hexes
                .iter()
                .enumerate()
                .all(|(other, hex)| other == index || !hex.starts_with(prefix))
        })
        .unwrap_or(own)
        .to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn claim(n: u8, subject: Hash, name: &str, value: Option<&str>) -> Claim {
        Claim {
            id: Hash::digest([n]),
            author: [n; 32],
            subject,
            name: name.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn test_a_bare_or_empty_alias_is_not_an_alias() {
        let subject = Hash::digest(b"ann");
        let claims = [
            claim(1, subject, ALIAS_NAME, None),
            claim(2, subject, ALIAS_NAME, Some("")),
            claim(3, subject, "seen", Some("Ann")),
            claim(4, Hash::digest(b"bo"), ALIAS_NAME, Some("Ann")),
        ];
        assert!(aliases_of(&claims, &subject).is_empty());
    }

    #[test]
    fn test_the_pet_name_wins_over_the_primary_and_the_primary_over_the_claims() {
        let alias = Alias {
            claim: Hash::digest(b"claim"),
            text: "Ann".to_owned(),
            author: [1; 32],
        };
        let two = [alias.clone(), alias.clone()];
        assert_eq!(
            shown_for(Some("Mine"), Some("team"), &two),
            Shown::PetName("Mine".to_owned())
        );
        assert_eq!(
            shown_for(None, Some("team"), &two),
            Shown::Primary("team".to_owned())
        );
        assert_eq!(shown_for(None, None, &two), Shown::Contested);
        assert_eq!(
            shown_for(None, None, std::slice::from_ref(&alias)),
            Shown::Only(alias)
        );
        assert_eq!(shown_for(None, None, &[]), Shown::Unnamed);
    }

    #[test]
    fn test_a_lone_row_shows_the_minimum_prefix() {
        let subject = Hash::digest(b"ann");
        let view = rows(
            &[Named {
                subject,
                name: "ann".to_owned(),
            }],
            &[],
            None,
            &BTreeMap::new(),
        );
        assert_eq!(view[0].prefix.len(), MIN_PREFIX);
        assert!(subject.to_hex().starts_with(&view[0].prefix));
    }
}
