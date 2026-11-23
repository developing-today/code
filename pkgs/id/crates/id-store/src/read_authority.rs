//! Note-granularity read authority (#12751, #5678 ruling, #5839 §6):
//! the resolution order every projection (journal, database, outline,
//! graph, search) and transclusion rendering must apply before a note —
//! or its existence — reaches a reader.
//!
//! The order, first set level wins:
//!
//! 1. object — the note's own recorded [`NoteAccess`];
//! 2. user default — the note owner's default access for their notes;
//! 3. server config — the host's configured default;
//! 4. public — the #5678 default when nothing above says otherwise.
//!
//! "User default" is the note owner's default, not the reader's: a
//! reader's preference cannot make another owner's note readable to
//! them. That reading is recorded on #12751 (decide-and-proceed,
//! FDR-0013); the ruling's words alone do not say whose default it is.
//!
//! The resolver is pure: it takes the three levels as inputs and
//! returns which level won and the access it carries, so a test can
//! observe the order directly instead of trusting a boolean.
//! [`may_read`] composes it with a [`NoteCheckpoints`] log. Unknown
//! notes are denied — a read check must not leak existence (design
//! invariant 6) — and a known note with no recorded policy falls
//! through to the user default, then the server default, then public.
//!
//! This module decides reads only. It wires into no projection yet:
//! the journal view is the first caller (next slice on #12751), and
//! the other projections follow it instead of re-deriving the order.

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;

use crate::note_checkpoints::{GroupRoles, NoteAccess, NoteCheckpoints, NoteRole};

/// Which level of the #5678 resolution order won a read decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadLevel {
    /// The note's own recorded access policy decided.
    Object,
    /// The note owner's default decided (no object policy recorded).
    UserDefault,
    /// The host's configured default decided (neither of the above set).
    ServerConfig,
    /// Nothing above was set: the #5678 public default decided.
    Public,
}

/// A resolved read: the winning level and the access it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRead {
    /// Which level of the order won.
    pub level: ReadLevel,
    /// The access that level carries.
    pub access: NoteAccess,
}

/// The two fall-through levels of the #5678 order, as the host
/// supplies them: the note owner's default and the server's
/// configured default. Either may be unset, in which case that level
/// is skipped. There is no server-config type in this crate yet; the
/// host fills these in from its own configuration when it calls
/// [`resolve_read`] or [`may_read`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadDefaults {
    /// The note owner's default access for their notes, when set.
    pub user_default: Option<NoteAccess>,
    /// The host's configured default access, when set.
    pub server_default: Option<NoteAccess>,
}

impl ReadDefaults {
    /// No defaults at any level: everything falls through to public.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Only the owner's default is set.
    #[must_use]
    pub fn user(access: NoteAccess) -> Self {
        Self {
            user_default: Some(access),
            server_default: None,
        }
    }

    /// Only the server's default is set.
    #[must_use]
    pub fn server(access: NoteAccess) -> Self {
        Self {
            user_default: None,
            server_default: Some(access),
        }
    }

    /// Both defaults are set; the owner's wins over the server's.
    #[must_use]
    pub fn both(user: NoteAccess, server: NoteAccess) -> Self {
        Self {
            user_default: Some(user),
            server_default: Some(server),
        }
    }
}

/// Resolves the #5678 read order over the three levels: the object's
/// own policy wins when recorded; otherwise the owner's default, then
/// the server's default, then public. The first set level wins —
/// a later level never overrides an earlier one.
#[must_use]
pub fn resolve_read(object: Option<&NoteAccess>, defaults: &ReadDefaults) -> ResolvedRead {
    if let Some(access) = object {
        return ResolvedRead {
            level: ReadLevel::Object,
            access: access.clone(),
        };
    }
    if let Some(access) = &defaults.user_default {
        return ResolvedRead {
            level: ReadLevel::UserDefault,
            access: access.clone(),
        };
    }
    if let Some(access) = &defaults.server_default {
        return ResolvedRead {
            level: ReadLevel::ServerConfig,
            access: access.clone(),
        };
    }
    ResolvedRead {
        level: ReadLevel::Public,
        access: NoteAccess::Public,
    }
}

/// The role `reader` effectively holds under one resolved access:
/// the owner is always [`NoteRole::Admin`]; on a public note every
/// reader holds [`NoteRole::Write`] (read included); on a scoped note
/// the best matching grant wins — a direct user grant, or a group
/// grant the `groups` resolver answers for, capped at the grant's
/// role. `None` when the reader holds nothing on a scoped note.
/// This is the grant logic [`NoteCheckpoints::effective_role`]
/// applies to a note's recorded policy, factored out so projections
/// and transclusion rendering resolve it over a [`ResolvedRead`]
/// instead of re-deriving it.
#[must_use]
pub fn role_for(
    owner: AuthorKey,
    access: &NoteAccess,
    reader: AuthorKey,
    groups: &GroupRoles,
) -> Option<NoteRole> {
    if reader == owner {
        return Some(NoteRole::Admin);
    }
    match access {
        NoteAccess::Public => Some(NoteRole::Write),
        NoteAccess::Scoped(grants) => {
            let mut best: Option<NoteRole> = None;
            for grant in grants {
                let role = match grant.principal {
                    NotePrincipal::User(user) if user == reader => Some(grant.role),
                    NotePrincipal::Group(group) => groups
                        .role(group, reader)
                        .map(|resolved| resolved.min(grant.role)),
                    _ => None,
                };
                if let Some(role) = role {
                    best = Some(best.map_or(role, |b: NoteRole| b.max(role)));
                }
            }
            best
        }
    }
}

/// The resolved read access for a note in a log: `None` when the log
/// has no such note. A known note with no recorded policy resolves
/// through the defaults, exactly as [`may_read`] checks it.
#[must_use]
pub fn resolve_for(
    notes: &NoteCheckpoints,
    note: Hash,
    defaults: &ReadDefaults,
) -> Option<ResolvedRead> {
    if notes.access(note).is_none() {
        return None;
    }
    Some(match notes.recorded_access(note) {
        Some((_, access)) => ResolvedRead {
            level: ReadLevel::Object,
            access,
        },
        None => resolve_read(None, defaults),
    })
}

/// Whether `reader` may read `note` under the #5678 order: the
/// note's recorded policy, else the owner's default, else the
/// server's default, else public. Unknown notes are denied — a
/// read check never confirms a note exists (design invariant 6).
#[must_use]
pub fn may_read(
    notes: &NoteCheckpoints,
    note: Hash,
    reader: AuthorKey,
    groups: &GroupRoles,
    defaults: &ReadDefaults,
) -> bool {
    let Some((owner, _)) = notes.access(note) else {
        return false;
    };
    let Some(resolved) = resolve_for(notes, note, defaults) else {
        return false;
    };
    role_for(owner, &resolved.access, reader, groups).is_some()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::documents::DocumentRef;
    use crate::note_checkpoints::{NoteGrant, NotePrincipal};

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn author(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    fn content(bytes: &[u8]) -> DocumentRef {
        DocumentRef::for_bytes("text/markdown", bytes).unwrap()
    }

    fn grant(user: AuthorKey, role: NoteRole) -> NoteGrant {
        NoteGrant {
            principal: NotePrincipal::User(user),
            role,
        }
    }

    fn group_grant(group: Hash, role: NoteRole) -> NoteGrant {
        NoteGrant {
            principal: NotePrincipal::Group(group),
            role,
        }
    }

    /// Three access values, one per order level, distinct so the
    /// winner is observable: object denies everyone, the user
    /// default is public, the server default grants one reader.
    fn level_policies() -> (NoteAccess, NoteAccess, NoteAccess) {
        (
            NoteAccess::Scoped(vec![]),
            NoteAccess::Public,
            NoteAccess::Scoped(vec![grant(author(9), NoteRole::Read)]),
        )
    }

    #[test]
    fn test_resolution_order_first_set_level_wins() {
        let (object, user, server) = level_policies();
        // All eight presence combinations, exhaustively: the order
        // property's whole input space is three presence bits.
        let cases: [(
            Option<NoteAccess>,
            Option<NoteAccess>,
            Option<NoteAccess>,
            ReadLevel,
            NoteAccess,
        ); 8] = [
            (
                Some(object.clone()),
                Some(user.clone()),
                Some(server.clone()),
                ReadLevel::Object,
                object.clone(),
            ),
            (
                Some(object.clone()),
                Some(user.clone()),
                None,
                ReadLevel::Object,
                object.clone(),
            ),
            (
                Some(object.clone()),
                None,
                Some(server.clone()),
                ReadLevel::Object,
                object.clone(),
            ),
            (
                Some(object.clone()),
                None,
                None,
                ReadLevel::Object,
                object.clone(),
            ),
            (
                None,
                Some(user.clone()),
                Some(server.clone()),
                ReadLevel::UserDefault,
                user.clone(),
            ),
            (
                None,
                Some(user.clone()),
                None,
                ReadLevel::UserDefault,
                user.clone(),
            ),
            (
                None,
                None,
                Some(server.clone()),
                ReadLevel::ServerConfig,
                server.clone(),
            ),
            (None, None, None, ReadLevel::Public, NoteAccess::Public),
        ];
        for (object, user, server, want_level, want_access) in cases {
            let defaults = ReadDefaults {
                user_default: user,
                server_default: server,
            };
            let resolved = resolve_read(object.as_ref(), &defaults);
            assert_eq!(resolved.level, want_level);
            assert_eq!(resolved.access, want_access);
        }
    }

    #[test]
    fn test_a_later_level_never_overrides_an_earlier_one() {
        // Object says scoped-deny-all; both defaults say public.
        // The stranger stays denied: the defaults do not reopen it.
        let mut log = NoteCheckpoints::new();
        log.create_with_access(
            hash(10),
            author(1),
            content(b"scoped"),
            NoteAccess::Scoped(vec![]),
        )
        .unwrap();
        let defaults = ReadDefaults::both(NoteAccess::Public, NoteAccess::Public);
        assert!(!may_read(
            &log,
            hash(10),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            resolve_for(&log, hash(10), &defaults).unwrap().level,
            ReadLevel::Object
        );
    }

    #[test]
    fn test_may_read_denies_an_unknown_note_without_leaking_existence() {
        let log = NoteCheckpoints::new();
        // Even with public defaults, a note the log does not have
        // reads as denied — and resolves to nothing, not to public.
        let defaults = ReadDefaults::both(NoteAccess::Public, NoteAccess::Public);
        assert!(!may_read(
            &log,
            hash(99),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(resolve_for(&log, hash(99), &defaults), None);
    }

    #[test]
    fn test_a_note_with_no_policy_falls_through_to_the_user_default() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        // The owner's default scopes the note: a stranger is denied,
        // the owner still reads, and the level says so.
        let defaults = ReadDefaults::user(NoteAccess::Scoped(vec![]));
        assert!(!may_read(
            &log,
            hash(10),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert!(may_read(
            &log,
            hash(10),
            author(1),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            resolve_for(&log, hash(10), &defaults).unwrap(),
            ResolvedRead {
                level: ReadLevel::UserDefault,
                access: NoteAccess::Scoped(vec![]),
            }
        );
    }

    #[test]
    fn test_a_note_with_no_policy_and_no_user_default_falls_to_the_server_default() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        // The server default grants only reader 9: reader 9 reads,
        // any other stranger does not, and the owner still reads.
        let defaults =
            ReadDefaults::server(NoteAccess::Scoped(vec![grant(author(9), NoteRole::Read)]));
        assert!(may_read(
            &log,
            hash(10),
            author(9),
            &GroupRoles::none(),
            &defaults
        ));
        assert!(!may_read(
            &log,
            hash(10),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert!(may_read(
            &log,
            hash(10),
            author(1),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            resolve_for(&log, hash(10), &defaults).unwrap().level,
            ReadLevel::ServerConfig
        );
    }

    #[test]
    fn test_a_note_with_nothing_set_is_public() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        let defaults = ReadDefaults::none();
        assert!(may_read(
            &log,
            hash(10),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            resolve_for(&log, hash(10), &defaults).unwrap(),
            ResolvedRead {
                level: ReadLevel::Public,
                access: NoteAccess::Public,
            }
        );
    }

    #[test]
    fn test_a_group_grant_in_the_resolved_default_still_reads() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        // No object policy; the user default grants group 7 Read.
        // A member of group 7 reads through the resolver; a
        // non-member does not — the group path survives resolution.
        let defaults = ReadDefaults::user(NoteAccess::Scoped(vec![group_grant(
            hash(7),
            NoteRole::Read,
        )]));
        let groups = GroupRoles::new(|group, member| {
            if group == hash(7) && member == author(3) {
                Some(NoteRole::Write)
            } else {
                None
            }
        });
        assert!(may_read(&log, hash(10), author(3), &groups, &defaults));
        assert!(!may_read(&log, hash(10), author(4), &groups, &defaults));
    }

    #[test]
    fn test_the_owner_reads_under_every_level() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        // Scoped at every level, granting nobody: the owner still
        // reads — ownership is Admin at the role layer, not a grant.
        let defaults = ReadDefaults::both(NoteAccess::Scoped(vec![]), NoteAccess::Scoped(vec![]));
        assert!(may_read(
            &log,
            hash(10),
            author(1),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            role_for(
                author(1),
                &NoteAccess::Scoped(vec![]),
                author(1),
                &GroupRoles::none()
            ),
            Some(NoteRole::Admin)
        );
    }
}
