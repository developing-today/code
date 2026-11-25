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
//! [`NoteReader`] is the one predicate every projection in
//! [`crate::note_projections`] takes for its `can_read` seam: it looks
//! up each note's owner default and applies the same order, so no
//! projection re-derives it.

use std::collections::BTreeMap;

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;

use crate::note_checkpoints::{GroupRoles, NoteAccess, NoteCheckpoints, NotePrincipal, NoteRole};

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

/// One reader's read check over a note log, ready to hand every
/// projection (journal, capture, database, transclusion) as its
/// `can_read` predicate.
///
/// The user-default level of the #5678 order is the note owner's
/// default, so it is looked up per note from `owner_defaults` by the
/// note's owner; `server_default` applies to every note. A note the
/// log does not hold is denied, so a projection over it never
/// confirms that it exists (design invariant 6).
#[derive(Clone, Debug)]
pub struct NoteReader<'a> {
    notes: &'a NoteCheckpoints,
    reader: AuthorKey,
    groups: &'a GroupRoles,
    owner_defaults: &'a BTreeMap<AuthorKey, NoteAccess>,
    server_default: Option<NoteAccess>,
}

impl<'a> NoteReader<'a> {
    /// A reader's check over `notes`, with each owner's default
    /// access and the host's configured default.
    #[must_use]
    pub const fn new(
        notes: &'a NoteCheckpoints,
        reader: AuthorKey,
        groups: &'a GroupRoles,
        owner_defaults: &'a BTreeMap<AuthorKey, NoteAccess>,
        server_default: Option<NoteAccess>,
    ) -> Self {
        Self {
            notes,
            reader,
            groups,
            owner_defaults,
            server_default,
        }
    }

    /// The two fall-through levels for one note: its owner's default
    /// and the server's. `None` for a note the log does not hold.
    #[must_use]
    pub fn defaults_for(&self, note: Hash) -> Option<ReadDefaults> {
        let (owner, _) = self.notes.access(note)?;
        Some(ReadDefaults {
            user_default: self.owner_defaults.get(&owner).cloned(),
            server_default: self.server_default.clone(),
        })
    }

    /// Which level of the #5678 order decides `note` for this
    /// reader's host. `None` for a note the log does not hold.
    #[must_use]
    pub fn resolve(&self, note: Hash) -> Option<ResolvedRead> {
        let defaults = self.defaults_for(note)?;
        resolve_for(self.notes, note, &defaults)
    }

    /// Whether this reader may read `note`. The signature matches the
    /// projections' `can_read` predicate.
    #[must_use]
    pub fn may_read(&self, note: &Hash) -> bool {
        self.defaults_for(*note).is_some_and(|defaults| {
            may_read(self.notes, *note, self.reader, self.groups, &defaults)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::documents::DocumentRef;
    use crate::note_checkpoints::NoteGrant;

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
    fn test_a_rebuild_keeps_a_note_with_no_policy_falling_through() {
        // The #12751 + #3819 composition: a note with no recorded
        // policy falls through to the user default on the live log,
        // and still does after a restart rebuild — the policies
        // snapshot carries no record for it, so seeding it back
        // must not invent a Public entry that would close the
        // fall-through at the object level.
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"plain")).unwrap();
        let defaults = ReadDefaults::user(NoteAccess::Scoped(vec![]));
        let rebuilt = NoteCheckpoints::from_events_with_policies(
            log.events().to_vec(),
            log.policies().unwrap(),
        )
        .unwrap();
        assert!(rebuilt.policies().unwrap().is_empty());
        assert!(!may_read(
            &rebuilt,
            hash(10),
            author(2),
            &GroupRoles::none(),
            &defaults
        ));
        assert!(may_read(
            &rebuilt,
            hash(10),
            author(1),
            &GroupRoles::none(),
            &defaults
        ));
        assert_eq!(
            resolve_for(&rebuilt, hash(10), &defaults).unwrap().level,
            ReadLevel::UserDefault
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

    // -- NoteReader across projections (#12751) -------------------------

    mod across_projections {
        use super::*;
        use crate::note_blocks::{Block, BlockId, BlockKind, BlockTree};
        use crate::note_projections::{
            CalendarDate, CaptureTarget, DatabaseDefinition, NoteProperty, ProjectedNote,
            PropertyFilter, PropertyValue, TransclusionRender, capture_target, database_view,
            journal_view, render_transclusion,
        };

        const A: u8 = 1;
        const B: u8 = 2;
        const C: u8 = 3;

        fn day(d: u8) -> CalendarDate {
            CalendarDate {
                year: 2026,
                month: 11,
                day: d,
            }
        }

        fn tree() -> BlockTree {
            BlockTree {
                blocks: vec![Block {
                    id: BlockId("b".to_owned()),
                    kind: BlockKind::Paragraph,
                    text: "body".to_owned(),
                    children: Vec::new(),
                }],
            }
        }

        fn projected(byte: u8, date: u8) -> ProjectedNote {
            ProjectedNote {
                note: hash(byte),
                revision: hash(byte.wrapping_add(100)),
                tree: tree(),
                dates: vec![day(date)],
                properties: vec![NoteProperty {
                    name: "status".to_owned(),
                    value: PropertyValue::Text("open".to_owned()),
                }],
            }
        }

        /// A's note 10 has no policy (A's default keeps it private);
        /// B's note 11 has no policy and B set no default; A's note
        /// 12 grants C read on the object; note 13 is not in the log.
        fn world() -> (NoteCheckpoints, Vec<ProjectedNote>) {
            let mut log = NoteCheckpoints::new();
            log.create(hash(10), author(A), content(b"a private by default"))
                .unwrap();
            log.create(hash(11), author(B), content(b"b no default"))
                .unwrap();
            log.create_with_access(
                hash(12),
                author(A),
                content(b"a shared with c"),
                NoteAccess::Scoped(vec![grant(author(C), NoteRole::Read)]),
            )
            .unwrap();
            let notes = vec![
                projected(10, 24),
                projected(11, 25),
                projected(12, 23),
                projected(13, 22),
            ];
            (log, notes)
        }

        fn owner_defaults() -> BTreeMap<AuthorKey, NoteAccess> {
            BTreeMap::from([(author(A), NoteAccess::Scoped(vec![]))])
        }

        fn journal_ids(reader: &NoteReader<'_>, notes: &[ProjectedNote]) -> Vec<Hash> {
            journal_view(notes, &|n: &Hash| reader.may_read(n))
                .iter()
                .map(|row| row.note.note)
                .collect()
        }

        #[test]
        fn test_journal_applies_each_owners_default() {
            let (log, notes) = world();
            let groups = GroupRoles::none();
            let defaults = owner_defaults();

            // C reads A's shared note (object level) and B's note
            // (public), never A's note left at A's private default,
            // never a note the log does not hold.
            let c = NoteReader::new(&log, author(C), &groups, &defaults, None);
            assert_eq!(journal_ids(&c, &notes), vec![hash(12), hash(11)]);

            // A reads everything A owns, plus B's public note.
            let a = NoteReader::new(&log, author(A), &groups, &defaults, None);
            assert_eq!(journal_ids(&a, &notes), vec![hash(12), hash(10), hash(11)]);

            assert_eq!(c.resolve(hash(10)).unwrap().level, ReadLevel::UserDefault);
            assert_eq!(c.resolve(hash(11)).unwrap().level, ReadLevel::Public);
            assert_eq!(c.resolve(hash(12)).unwrap().level, ReadLevel::Object);
            assert_eq!(c.resolve(hash(13)), None);
        }

        #[test]
        fn test_server_default_closes_notes_whose_owner_set_none() {
            let (log, notes) = world();
            let groups = GroupRoles::none();
            let defaults = owner_defaults();
            let c = NoteReader::new(
                &log,
                author(C),
                &groups,
                &defaults,
                Some(NoteAccess::Scoped(vec![])),
            );
            // B set no default, so the server's now decides note 11;
            // A's own default still decides note 10, and the object
            // grant on note 12 still wins over both.
            assert_eq!(journal_ids(&c, &notes), vec![hash(12)]);
            assert_eq!(c.resolve(hash(11)).unwrap().level, ReadLevel::ServerConfig);
            assert_eq!(c.resolve(hash(10)).unwrap().level, ReadLevel::UserDefault);
        }

        #[test]
        fn test_capture_never_appends_to_a_note_the_reader_cannot_read() {
            let (log, notes) = world();
            let groups = GroupRoles::none();
            let defaults = owner_defaults();
            let c = NoteReader::new(&log, author(C), &groups, &defaults, None);
            let a = NoteReader::new(&log, author(A), &groups, &defaults, None);
            assert_eq!(
                capture_target(&notes, day(24), &|n: &Hash| c.may_read(n)),
                CaptureTarget::Create { date: day(24) }
            );
            assert!(matches!(
                capture_target(&notes, day(24), &|n: &Hash| a.may_read(n)),
                CaptureTarget::Existing { note } if note.note == hash(10)
            ));
        }

        #[test]
        fn test_database_rows_follow_the_same_order() {
            let (log, notes) = world();
            let groups = GroupRoles::none();
            let defaults = owner_defaults();
            let c = NoteReader::new(&log, author(C), &groups, &defaults, None);
            let def = DatabaseDefinition {
                name: "open items".to_owned(),
                filters: BTreeMap::from([(
                    "status".to_owned(),
                    PropertyFilter::Equals(PropertyValue::Text("open".to_owned())),
                )]),
                columns: vec!["status".to_owned()],
                sort: None,
            };
            let rows: Vec<Hash> = database_view(&def, &notes, &|n: &Hash| c.may_read(n))
                .iter()
                .map(|row| row.note.note)
                .collect();
            assert_eq!(rows, vec![hash(11), hash(12)]);
        }

        #[test]
        fn test_transclusion_degrades_when_the_owner_default_denies() {
            let (log, _) = world();
            let groups = GroupRoles::none();
            let defaults = owner_defaults();
            let source = tree();
            let block = BlockId("b".to_owned());
            let render = |reader: &NoteReader<'_>| {
                render_transclusion(
                    &hash(10),
                    None,
                    Some(&block),
                    None,
                    Some(&source),
                    &hash(10),
                    &|n: &Hash| reader.may_read(n),
                )
            };
            let c = NoteReader::new(&log, author(C), &groups, &defaults, None);
            let a = NoteReader::new(&log, author(A), &groups, &defaults, None);
            assert_eq!(render(&c), TransclusionRender::Unresolved);
            assert!(matches!(render(&a), TransclusionRender::Content { .. }));
        }
    }
}
