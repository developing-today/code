//! Provisioning: applying a server config to one server's accounts and groups (P2.9).
//!
//! Provisioning goes through the paths a user action takes. A user is created with
//! [`Directory::sign_up_keyed`], the call a user makes to sign up with a key. A group is a create
//! event, and each member is an add event by the creator. Both are folded by [`fold`], so the Admin
//! check, the member cap, and the rest apply. The creator must pass the config's [`CreateRule`],
//! which is the limit a user's create meets.
//!
//! The events are derived from the config and are not signed. The config holds public keys only,
//! so there is no key to sign with. Each group is provisioned once: a second run skips a group that
//! exists, so a restart changes nothing and an Admin's later change is not undone.

use p2panda_core::Hash;
use thiserror::Error;

use super::{GroupSpec, ServerConfig};
use crate::directory::Directory;
use crate::groups::{
    Body, Create, Event, FoldError, Key, MemberOp, Outcome, Policy, Projection, Refusal, Template,
    fold,
};
use crate::world::hex_encode;

/// The accounts and group events of one server, as provisioning leaves them.
#[derive(Debug)]
pub struct Provisioned {
    directory: Directory,
    events: Vec<Event>,
}

/// What one [`Provisioned::apply`] created. All zero means the config was already in effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Users whose accounts were created.
    pub users_created: usize,
    /// Groups created.
    pub groups_created: usize,
    /// Members added to the groups created.
    pub members_added: usize,
}

/// Why a config could not be applied.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProvisionError {
    /// The directory refused a user's account.
    #[error("the directory refused user `{key}`: {message}")]
    User {
        /// The user's key.
        key: String,
        /// The directory's reason.
        message: String,
    },
    /// The config's create rule does not allow the group's creator.
    #[error("group `{group}` is not allowed: its creator {creator} may not create groups")]
    NotAllowed {
        /// The group's key.
        group: String,
        /// The creator's public key, in hex.
        creator: String,
    },
    /// The group fold refused one of the group's events.
    #[error("provisioning group `{group}` was refused: {refusal:?}")]
    Refused {
        /// The group's key.
        group: String,
        /// The fold's reason.
        refusal: Refusal,
    },
    /// The events cannot be folded. This means an invariant broke.
    #[error(transparent)]
    Fold(#[from] FoldError),
}

impl Default for Provisioned {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioned {
    /// Nothing provisioned yet: no accounts and no events.
    #[must_use]
    pub fn new() -> Self {
        Self {
            directory: Directory::new(),
            events: Vec::new(),
        }
    }

    /// The accounts.
    #[must_use]
    pub const fn directory(&self) -> &Directory {
        &self.directory
    }

    /// Every group event, in the order it was recorded.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The groups as the events determine them.
    ///
    /// # Errors
    ///
    /// Fails if the events cannot be folded.
    pub fn projection(&self) -> Result<Projection, FoldError> {
        fold(&self.events)
    }

    /// Records one event, as a user action does. A refused event is kept in the log with no effect,
    /// as the fold decides. Read its outcome from the returned groups.
    ///
    /// # Errors
    ///
    /// Fails if the events cannot be folded.
    pub fn record(&mut self, event: Event) -> Result<Projection, FoldError> {
        self.events.push(event);
        self.projection()
    }

    /// Creates the accounts and groups the config names, and adds their members. What already
    /// exists is skipped, so a second run of the same config changes nothing.
    ///
    /// # Errors
    ///
    /// Fails on the first user the directory refuses, the first group whose creator the create rule
    /// does not allow, or the first group event the fold refuses. A group that fails commits no
    /// events. Users created before the failure stay, but the config is refused at startup, so no
    /// server runs on that partial state.
    pub fn apply(&mut self, config: &ServerConfig) -> Result<Report, ProvisionError> {
        let mut report = Report::default();
        for user in config.users() {
            let id = hex_encode(&user.id);
            if self.directory.account(&id).is_some() {
                continue;
            }
            self.directory
                .sign_up_keyed(&id, &user.key)
                .map_err(|error| ProvisionError::User {
                    key: user.key.clone(),
                    message: format!("{error:#}"),
                })?;
            report.users_created += 1;
        }
        for group in config.groups() {
            if self.events.iter().any(|event| event.id == group.id) {
                continue;
            }
            if !config
                .create_rule()
                .allows(&group.creator, &self.projection()?)
            {
                return Err(ProvisionError::NotAllowed {
                    group: group.key.clone(),
                    creator: hex_encode(&group.creator),
                });
            }
            let batch = group_events(group);
            let mut next = self.events.clone();
            next.extend(batch.iter().cloned());
            let projection = fold(&next)?;
            for event in &batch {
                if let Some(Outcome::Refused(refusal)) = projection.outcome(&event.id) {
                    return Err(ProvisionError::Refused {
                        group: group.key.clone(),
                        refusal: refusal.clone(),
                    });
                }
            }
            self.events = next;
            report.groups_created += 1;
            report.members_added += group.members.len();
        }
        Ok(report)
    }
}

/// The events that create `group` and add its members. The create comes first, and each add follows
/// the one before it, so the group's events form one chain.
fn group_events(group: &GroupSpec) -> Vec<Event> {
    let mut events = vec![Event {
        id: group.id,
        author: group.creator,
        after: Vec::new(),
        group: None,
        body: Body::Create(Create {
            template: Template::Custom,
            policy: Policy::default(),
            members: Vec::new(),
            admins_on_create: false,
        }),
    }];
    for (member, role) in &group.members {
        let after = events.last().map_or(group.id, |event| event.id);
        events.push(Event {
            id: add_id(&group.id, member),
            author: group.creator,
            after: vec![after],
            group: Some(group.id),
            body: Body::Member(MemberOp::Add {
                member: *member,
                role: *role,
            }),
        });
    }
    events
}

/// The ID of the event that adds `member` to `group`. Each member is added once, so the ID is the
/// same on every start.
fn add_id(group: &Hash, member: &Key) -> Hash {
    let mut bytes = b"id-config-add-v1".to_vec();
    bytes.extend_from_slice(group.as_bytes());
    bytes.extend_from_slice(member);
    Hash::digest(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::fmt::Write as _;

    use ed25519_dalek::SigningKey;
    use p2panda_core::Hash;
    use proptest::prelude::*;

    use super::{ProvisionError, Provisioned, Report};
    use crate::groups::{Body, Event, Key, MemberOp, Outcome, Role};
    use crate::server_config::ServerConfig;
    use crate::world::hex_encode;

    fn key(n: u8) -> Key {
        SigningKey::from_bytes(&[n; 32]).verifying_key().to_bytes()
    }

    fn hex(n: u8) -> String {
        hex_encode(&key(n))
    }

    /// Four users: ann (1), bo (2), cy (3), and dee (4).
    fn users() -> String {
        format!(
            "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n\n\
             [[users]]\nkey = \"cy\"\nid = \"{}\"\n\n[[users]]\nkey = \"dee\"\nid = \"{}\"\n",
            hex(1),
            hex(2),
            hex(3),
            hex(4)
        )
    }

    fn config(extra: &str) -> String {
        format!("{}{extra}", users())
    }

    /// A group `team`, created by ann, with bo as a writer and cy as a reader.
    const TEAM: &str = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                        [[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n\n\
                        [[groups.members]]\nuser = \"cy\"\nrole = \"read\"\n";

    /// The group `staff`, created by ann, with bo as a writer, cy as a reader, and dee as a manager.
    const STAFF: &str = "\n[[groups]]\nkey = \"staff\"\nname = \"Staff\"\ncreator = \"ann\"\n\n\
                         [[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n\n\
                         [[groups.members]]\nuser = \"cy\"\nrole = \"read\"\n\n\
                         [[groups.members]]\nuser = \"dee\"\nrole = \"manage\"\n";

    /// Ann may create, and so may members of `staff` at or above write.
    fn floor_config(creator: &str) -> String {
        config(&format!(
            "\n[create]\nusers = [\"ann\"]\ngroups = [\"staff\"]\nmin_role = \"write\"\n{STAFF}\n\
             [[groups]]\nkey = \"new\"\nname = \"New\"\ncreator = \"{creator}\"\n"
        ))
    }

    fn apply(text: &str) -> (Provisioned, Result<Report, ProvisionError>) {
        let config = ServerConfig::parse(text).unwrap();
        let mut state = Provisioned::new();
        let result = state.apply(&config);
        (state, result)
    }

    fn view_of(
        state: &Provisioned,
        config: &ServerConfig,
        group: &str,
    ) -> crate::groups::GroupView {
        let id = config.group_id(group).unwrap();
        state.projection().unwrap().group(&id).cloned().unwrap()
    }

    #[test]
    fn test_a_config_creates_its_users_groups_and_members() {
        let (state, result) = apply(&config(TEAM));
        assert_eq!(
            result.unwrap(),
            Report {
                users_created: 4,
                groups_created: 1,
                members_added: 2,
            }
        );
        assert!(state.directory().account(&hex(1)).is_some());
        assert!(state.directory().account(&hex(4)).is_some());

        let config = ServerConfig::parse(&config(TEAM)).unwrap();
        let view = view_of(&state, &config, "team");
        assert_eq!(
            view.members.get(&key(1)),
            Some(&Role::Admin),
            "the creator is Admin"
        );
        assert_eq!(view.members.get(&key(2)), Some(&Role::Write));
        assert_eq!(view.members.get(&key(3)), Some(&Role::Read));
        assert_eq!(view.members.get(&key(4)), None, "dee is not listed in team");
    }

    #[test]
    fn test_a_restart_with_the_same_config_creates_nothing() {
        let text = config(TEAM);
        let (mut state, first) = apply(&text);
        first.unwrap();
        let events = state.events().to_vec();
        let accounts = state.directory().accounts().count();

        // A restart reads the same file again, into a new config.
        let restarted = ServerConfig::parse(&text).unwrap();
        assert_eq!(state.apply(&restarted), Ok(Report::default()));
        assert_eq!(state.events(), events.as_slice());
        assert_eq!(state.directory().accounts().count(), accounts);
    }

    #[test]
    fn test_a_fresh_start_with_the_same_config_gives_the_same_state() {
        let text = config(TEAM);
        let (first, _) = apply(&text);
        let (second, _) = apply(&text);
        assert_eq!(first.events(), second.events());
        let ids = |state: &Provisioned| -> Vec<String> {
            state
                .directory()
                .accounts()
                .map(|account| account.id.clone())
                .collect()
        };
        assert_eq!(ids(&first), ids(&second));
    }

    #[test]
    fn test_a_member_an_admin_removed_is_not_added_again_on_restart() {
        // Provisioning runs once per group. An Admin's later change is a user action, and a
        // restart must not undo it.
        let text = config(TEAM);
        let config = ServerConfig::parse(&text).unwrap();
        let (mut state, first) = apply(&text);
        first.unwrap();
        let team = config.group_id("team").unwrap();
        let last = state.events().last().unwrap().id;
        let remove_id = Hash::digest(b"ann removes bo");
        let remove_bo = Event {
            id: remove_id,
            author: key(1),
            after: vec![last],
            group: Some(team),
            body: Body::Member(MemberOp::Remove {
                member: key(2),
                successor: None,
            }),
        };
        let projection = state.record(remove_bo).unwrap();
        assert_eq!(projection.outcome(&remove_id), Some(&Outcome::Applied));

        assert_eq!(state.apply(&config), Ok(Report::default()));
        let view = view_of(&state, &config, "team");
        assert_eq!(view.members.get(&key(2)), None, "bo stays removed");
        assert_eq!(view.members.get(&key(3)), Some(&Role::Read));
    }

    #[test]
    fn test_a_listed_user_may_create_a_group() {
        let text = config(
            "\n[create]\nusers = [\"ann\"]\n\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n",
        );
        let (state, result) = apply(&text);
        assert_eq!(result.unwrap().groups_created, 1);
        assert!(!state.events().is_empty());
    }

    #[test]
    fn test_an_unlisted_user_may_not_create_a_group_and_nothing_is_committed() {
        let text = config(
            "\n[create]\nusers = [\"ann\"]\n\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"bo\"\n",
        );
        let (state, result) = apply(&text);
        assert!(matches!(result, Err(ProvisionError::NotAllowed { .. })));
        assert!(state.events().is_empty(), "a refused group leaves no event");
    }

    #[test]
    fn test_a_member_of_a_listed_group_at_the_floor_may_create() {
        let (_, result) = apply(&floor_config("bo"));
        assert!(result.is_ok(), "bo is a writer, the floor: {result:?}");
    }

    #[test]
    fn test_a_member_of_a_listed_group_above_the_floor_may_create() {
        let (_, result) = apply(&floor_config("dee"));
        assert!(
            result.is_ok(),
            "dee is a manager, above the floor: {result:?}"
        );
    }

    #[test]
    fn test_a_member_of_a_listed_group_below_the_floor_may_not_create() {
        let (_, result) = apply(&floor_config("cy"));
        assert!(
            matches!(result, Err(ProvisionError::NotAllowed { .. })),
            "cy is a reader, below the floor: {result:?}"
        );
    }

    #[test]
    fn test_with_no_floor_any_member_of_a_listed_group_may_create() {
        let text = config(&format!(
            "\n[create]\nusers = [\"ann\"]\ngroups = [\"staff\"]\n{STAFF}\n\
             [[groups]]\nkey = \"new\"\nname = \"New\"\ncreator = \"cy\"\n"
        ));
        let (_, result) = apply(&text);
        assert!(
            result.is_ok(),
            "any member may create with no floor: {result:?}"
        );
    }

    #[test]
    fn test_a_group_admitted_by_a_group_declared_later_is_refused() {
        // The config's order decides: `new` is checked before `staff` exists.
        let text = config(&format!(
            "\n[create]\nusers = [\"ann\"]\ngroups = [\"staff\"]\nmin_role = \"write\"\n\n\
             [[groups]]\nkey = \"new\"\nname = \"New\"\ncreator = \"bo\"\n{STAFF}"
        ));
        let (_, result) = apply(&text);
        assert!(matches!(result, Err(ProvisionError::NotAllowed { .. })));
    }

    /// A random config: `users` users, groups with random creators and members, and one of three
    /// create rules. Each rule may refuse some creators, and the property holds either way.
    #[derive(Debug, Clone)]
    struct Spec {
        users: usize,
        /// 0: anyone. 1: every user is listed. 2: only u0 is listed.
        rule: u8,
        /// Each group: a creator index, and members as (index, role) pairs.
        groups: Vec<(usize, Vec<(usize, u8)>)>,
    }

    impl Spec {
        fn render(&self) -> String {
            let mut text = String::new();
            for user in 0..self.users {
                let id = hex(u8::try_from(user + 1).unwrap());
                write!(text, "[[users]]\nkey = \"u{user}\"\nid = \"{id}\"\n\n").unwrap();
            }
            match self.rule {
                0 => {}
                1 => {
                    let all: Vec<String> =
                        (0..self.users).map(|user| format!("\"u{user}\"")).collect();
                    write!(text, "[create]\nusers = [{}]\n\n", all.join(", ")).unwrap();
                }
                _ => text.push_str("[create]\nusers = [\"u0\"]\n\n"),
            }
            let roles = ["pull", "read", "write", "manage", "admin"];
            for (group, (creator, members)) in self.groups.iter().enumerate() {
                let creator = creator % self.users;
                write!(
                    text,
                    "[[groups]]\nkey = \"g{group}\"\nname = \"G{group}\"\ncreator = \"u{creator}\"\n\n"
                )
                .unwrap();
                let mut seen = std::collections::BTreeSet::from([creator]);
                for (member, role) in members {
                    let member = member % self.users;
                    if !seen.insert(member) {
                        continue;
                    }
                    let role = roles[usize::from(*role)];
                    write!(
                        text,
                        "[[groups.members]]\nuser = \"u{member}\"\nrole = \"{role}\"\n\n"
                    )
                    .unwrap();
                }
            }
            text
        }
    }

    fn spec() -> impl Strategy<Value = Spec> {
        (
            1usize..=4,
            0u8..3,
            prop::collection::vec(
                (0usize..4, prop::collection::vec((0usize..4, 0u8..5), 0..4)),
                0..4,
            ),
        )
            .prop_map(|(users, rule, groups)| Spec {
                users,
                rule,
                groups,
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prop_the_same_config_applied_twice_gives_the_same_state(spec in spec()) {
            let text = spec.render();
            let config = ServerConfig::parse(&text).unwrap();

            // A fresh start gives the same answer as the first run, success or refusal.
            let mut once = Provisioned::new();
            let first = once.apply(&config);
            let fresh = Provisioned::new().apply(&config);
            prop_assert_eq!(&first, &fresh);

            if first.is_ok() {
                let events = once.events().to_vec();
                let accounts = once.directory().accounts().count();
                prop_assert_eq!(once.apply(&config), Ok(Report::default()));
                prop_assert_eq!(once.events(), events.as_slice());
                prop_assert_eq!(once.directory().accounts().count(), accounts);
            }
        }
    }

    /// A create by `who`, named `name`, as a user action sends it.
    fn create_by(who: Key, name: &str) -> crate::groups::Event {
        crate::groups::Event {
            id: Hash::digest(name.as_bytes()),
            author: who,
            after: Vec::new(),
            group: None,
            body: Body::Create(crate::groups::Create {
                template: crate::groups::Template::Custom,
                policy: crate::groups::Policy::default(),
                members: Vec::new(),
                admins_on_create: false,
            }),
        }
    }

    #[test]
    fn test_a_create_recorded_after_the_start_meets_the_create_rule() {
        // Only ann may create. A create by anyone else that arrives after the start is refused, as
        // the rule refuses it at startup, and it leaves no group behind.
        let config = ServerConfig::parse(&config("\n[create]\nusers = [\"ann\"]\n")).unwrap();
        let mut state = Provisioned::new();
        state.apply(&config).unwrap();
        assert!(state.record(create_by(key(1), "by-ann")).is_ok());
        for who in [2, 3, 4] {
            assert!(
                state
                    .record(create_by(key(who), &format!("by-{who}")))
                    .is_err(),
                "user {who} is not listed and may not create"
            );
            let id = Hash::digest(format!("by-{who}").as_bytes());
            assert!(state.projection().unwrap().group(&id).is_none());
        }
        assert!(
            state
                .projection()
                .unwrap()
                .group(&Hash::digest(b"by-ann"))
                .is_some()
        );
    }
}
