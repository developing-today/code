//! Group policy records and templates (design §5.1 and §5.3).
//!
//! A policy record is written in full into the group's create event, and it changes only through
//! a `group.policy` event by an Admin. A template is a named preset that a create event names, and
//! the record must be one that template admits.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use super::{Key, Role};

pub use id_store::policy::History;

/// A concrete group-removal behavior, without its per-call options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum RemoveModeKind {
    /// End the removed member's grants.
    Revoke,
    /// Reissue the removed member's grants to a successor.
    Transfer,
    /// Retain grants under their original grantor.
    KeepPrior,
    /// Re-anchor grants at the group.
    ReassignToGroup,
}

/// A successor category that group policy can allow or reject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum SuccessorKind {
    /// A specifically named member.
    Member,
    /// A group administrator.
    Admin,
    /// A specifically designated account.
    Designated,
    /// The account performing the removal.
    Actor,
}

/// Who takes over grants in a transfer removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoveSuccessor {
    /// A specifically named member.
    Member(#[serde(with = "super::wire::key")] Key),
    /// A group administrator, resolved at the removal's causal position.
    Admin,
    /// A specifically designated account.
    Designated(#[serde(with = "super::wire::key")] Key),
    /// The account performing the removal.
    Actor,
}

impl RemoveSuccessor {
    /// The policy category for this successor.
    pub const fn kind(self) -> SuccessorKind {
        match self {
            Self::Member(_) => SuccessorKind::Member,
            Self::Admin => SuccessorKind::Admin,
            Self::Designated(_) => SuccessorKind::Designated,
            Self::Actor => SuccessorKind::Actor,
        }
    }
}

/// Options specific to a transfer removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferOpts {
    /// The successor that receives the removed member's grants.
    pub successor: RemoveSuccessor,
}

/// The requested behavior for one member removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoveMode {
    /// End the removed member's grants.
    Revoke,
    /// Reissue the grants to a successor.
    Transfer(TransferOpts),
    /// Retain prior grants under their original grantor. A re-add starts clean:
    /// pre-removal grants the account made or received do not revive when it is
    /// re-added (issue #169); they must be re-issued.
    KeepPrior,
    /// Re-anchor grants at the group.
    ReassignToGroup,
    /// Resolve to the group's configured default before emitting an event.
    ServerDefault,
}

impl RemoveMode {
    /// The concrete policy category, or `None` for the pre-emission resolver request.
    pub const fn kind(self) -> Option<RemoveModeKind> {
        match self {
            Self::Revoke => Some(RemoveModeKind::Revoke),
            Self::Transfer(_) => Some(RemoveModeKind::Transfer),
            Self::KeepPrior => Some(RemoveModeKind::KeepPrior),
            Self::ReassignToGroup => Some(RemoveModeKind::ReassignToGroup),
            Self::ServerDefault => None,
        }
    }
}

/// Group policy for explicit member-removal modes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePolicy {
    /// Concrete modes callers may request. `ServerDefault` is not a concrete mode.
    pub allowed_modes: Vec<RemoveModeKind>,
    /// Successor categories callers may use with `Transfer`.
    pub allowed_successors: Vec<SuccessorKind>,
    /// The concrete mode selected for a `RemoveMode::ServerDefault` request.
    pub default_mode: RemoveMode,
}

impl Default for RemovePolicy {
    /// Allow the settled modes and successor categories; default to the least-authority behavior.
    fn default() -> Self {
        Self {
            allowed_modes: vec![
                RemoveModeKind::Revoke,
                RemoveModeKind::Transfer,
                RemoveModeKind::KeepPrior,
                RemoveModeKind::ReassignToGroup,
            ],
            allowed_successors: vec![
                SuccessorKind::Member,
                SuccessorKind::Admin,
                SuccessorKind::Designated,
                SuccessorKind::Actor,
            ],
            default_mode: RemoveMode::Revoke,
        }
    }
}

impl RemovePolicy {
    /// Whether this policy's lists and default form a valid configuration.
    pub fn is_valid(&self) -> bool {
        let unique_modes = self
            .allowed_modes
            .iter()
            .enumerate()
            .all(|(index, mode)| !self.allowed_modes[..index].contains(mode));
        let unique_successors = self
            .allowed_successors
            .iter()
            .enumerate()
            .all(|(index, successor)| !self.allowed_successors[..index].contains(successor));
        let Some(default_kind) = self.default_mode.kind() else {
            return false;
        };

        !self.allowed_modes.is_empty()
            && unique_modes
            && unique_successors
            && self.allowed_modes.contains(&default_kind)
            && match self.default_mode {
                RemoveMode::Transfer(opts) => {
                    self.allowed_successors.contains(&opts.successor.kind())
                }
                RemoveMode::ServerDefault => false,
                _ => true,
            }
            && (!self.allowed_modes.contains(&RemoveModeKind::Transfer)
                || !self.allowed_successors.is_empty())
    }

    /// Whether `mode` is concrete and permitted, including its transfer successor category.
    pub fn allows(&self, mode: RemoveMode) -> bool {
        let Some(kind) = mode.kind() else {
            return false;
        };
        self.allowed_modes.contains(&kind)
            && match mode {
                RemoveMode::Transfer(opts) => {
                    self.allowed_successors.contains(&opts.successor.kind())
                }
                RemoveMode::ServerDefault => false,
                _ => true,
            }
    }
}

/// Who may join a group, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Join {
    /// A manager adds members.
    ManagerAdd,
    /// An invite event admits the joiner (design §10).
    Invite,
    /// One side asks and the other accepts. Friendship uses this.
    MutualRequest,
    /// Anyone may join.
    Open,
}

/// What a removal does to the group as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnMemberRemoved {
    /// The group goes on without the removed member.
    Continue,
    /// Removing either side ends the group. Friendship uses this.
    Dissolve,
}

/// How a removal takes effect (design §6). Strong is the default. The voiding track reads this
/// setting: strong remove voids the events concurrent with a removal, weak remove stands them and
/// applies only the authority check (design §6.2 and §6.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Removal {
    /// Strong remove, the default.
    #[default]
    Strong,
    /// Weak remove.
    Weak,
}

/// Legacy group-wide delegation preference. New removals carry their own [`RemoveMode`], which is
/// validated by `remove_policy` and is not overridden by this setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Delegation {
    /// Legacy preference to keep grants made before a removal.
    /// A re-add starts clean: pre-removal grants the account made or received do not revive
    /// when it is re-added (issue #169); they must be re-issued.
    #[value(name = "keep_prior")]
    KeepPrior,
    /// Legacy preference to cascade a grant when its grantor loses the scope. The default.
    Cascade,
    /// Legacy preference for transfer removals. New events record a concrete successor in
    /// `RemoveMode::Transfer`.
    Transfer,
}

/// A key-agreement scheme and its version (design §8.7). A policy with no scheme has `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheme {
    /// The scheme's name.
    pub name: String,
    /// The scheme's version.
    pub version: u32,
}

/// Which role may change roles and the policy record. Only Admin may, so the field is a rule and
/// not a choice: [`Policy::is_valid`] refuses any other value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRules {
    /// The least role that may change roles and policy.
    pub change: Role,
}

/// A group's policy record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// The most members the group may have. `None` means no cap. Only an Admin may change it.
    pub member_cap: Option<usize>,
    /// How a joiner gets in.
    pub join: Join,
    /// What removing a member does to the group.
    pub on_member_removed: OnMemberRemoved,
    /// How removals take effect.
    pub removal: Removal,
    /// Legacy group-wide delegation preference. A removal's concrete [`RemoveMode`] takes precedence.
    pub delegation: Option<Delegation>,
    /// Allowed explicit removal modes and the mode used for `ServerDefault`.
    #[serde(default)]
    pub remove_policy: RemovePolicy,
    /// What a joiner may see of earlier history.
    pub history: History,
    /// The key-agreement scheme. `None` means no scheme.
    pub scheme: Option<Scheme>,
    /// Which role may change roles and policy.
    pub roles: RoleRules,
}

impl Default for Policy {
    /// A normal group: no cap, managers add members, removal continues the group, strong remove,
    /// full history, no scheme, and Admin changes roles and policy.
    fn default() -> Self {
        Self {
            member_cap: None,
            join: Join::ManagerAdd,
            on_member_removed: OnMemberRemoved::Continue,
            removal: Removal::Strong,
            delegation: None,
            remove_policy: RemovePolicy::default(),
            history: History::Full,
            scheme: None,
            roles: RoleRules {
                change: Role::Admin,
            },
        }
    }
}

impl Policy {
    /// The friendship template (design §5.3): two members, mutual request, dissolve on removal,
    /// strong remove, cascade delegation, and full history.
    pub fn friendship() -> Self {
        Self {
            member_cap: Some(2),
            join: Join::MutualRequest,
            on_member_removed: OnMemberRemoved::Dissolve,
            removal: Removal::Strong,
            delegation: Some(Delegation::Cascade),
            remove_policy: RemovePolicy::default(),
            history: History::Full,
            scheme: None,
            roles: RoleRules {
                change: Role::Admin,
            },
        }
    }

    /// The legacy delegation preference: the record's, or cascade when it names none.
    pub fn delegation_mode(&self) -> Delegation {
        self.delegation.unwrap_or(Delegation::Cascade)
    }

    /// Whether the record is usable. Only Admin may change roles and policy, a cap must allow
    /// at least one member, and a recent history window must hold at least one epoch.
    pub fn is_valid(&self) -> bool {
        self.roles.change == Role::Admin
            && self.member_cap.is_none_or(|cap| cap >= 1)
            && !matches!(self.history, History::Recent(0))
            && self.remove_policy.is_valid()
    }
}

/// A named preset that a create event names. The create's policy record is written in full, and
/// [`Template::admits`] checks it against the preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Template {
    /// Two members who ask and accept each other (design §5.4).
    Friendship,
    /// Many readers, some writers, few managers. No cap. Joins by manager add, invite, or open.
    Channel,
    /// Joins by invite only.
    Private,
    /// Any combination of fields.
    Custom,
}

impl Template {
    /// Whether this template admits `policy`.
    pub fn admits(self, policy: &Policy) -> bool {
        match self {
            Self::Friendship => *policy == Policy::friendship(),
            Self::Channel => {
                policy.member_cap.is_none()
                    && policy.scheme.is_none()
                    && matches!(policy.join, Join::ManagerAdd | Join::Invite | Join::Open)
            }
            Self::Private => policy.join == Join::Invite,
            Self::Custom => true,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_the_friendship_template_admits_only_the_friendship_policy() {
        assert!(Template::Friendship.admits(&Policy::friendship()));
        let wider = Policy {
            member_cap: Some(3),
            ..Policy::friendship()
        };
        assert!(!Template::Friendship.admits(&wider));
    }

    #[test]
    fn test_a_channel_has_no_cap_and_a_private_group_joins_by_invite() {
        let capped = Policy {
            member_cap: Some(2),
            ..Policy::default()
        };
        assert!(!Template::Channel.admits(&capped));
        let private = Policy {
            join: Join::Invite,
            ..Policy::default()
        };
        assert!(Template::Private.admits(&private));
        assert!(!Template::Private.admits(&Policy::default()));
    }

    #[test]
    fn test_only_an_admin_rule_and_a_positive_cap_are_valid() {
        assert!(Policy::default().is_valid());
        let manager_rule = Policy {
            roles: RoleRules {
                change: Role::Manage,
            },
            ..Policy::default()
        };
        assert!(!manager_rule.is_valid());
        let no_members = Policy {
            member_cap: Some(0),
            ..Policy::default()
        };
        assert!(!no_members.is_valid());
    }

    #[test]
    fn test_remove_policy_defaults_to_revoke_and_allows_configured_modes() {
        let policy = RemovePolicy::default();
        assert!(policy.is_valid());
        assert_eq!(policy.default_mode, RemoveMode::Revoke);
        assert!(policy.allows(RemoveMode::Revoke));
        assert!(!policy.allows(RemoveMode::ServerDefault));
        assert!(policy.allows(RemoveMode::Transfer(TransferOpts {
            successor: RemoveSuccessor::Member([7; 32]),
        })));
    }

    #[test]
    fn test_remove_policy_restricts_modes_and_successor_kinds() {
        let policy = RemovePolicy {
            allowed_modes: vec![RemoveModeKind::Revoke, RemoveModeKind::Transfer],
            allowed_successors: vec![SuccessorKind::Member],
            default_mode: RemoveMode::Revoke,
        };
        assert!(policy.is_valid());
        assert!(policy.allows(RemoveMode::Transfer(TransferOpts {
            successor: RemoveSuccessor::Member([3; 32]),
        })));
        assert!(!policy.allows(RemoveMode::Transfer(TransferOpts {
            successor: RemoveSuccessor::Admin,
        })));
        assert!(!policy.allows(RemoveMode::KeepPrior));
    }

    #[test]
    fn test_remove_mode_serializes_successor_keys_as_hex() {
        let mode = RemoveMode::Transfer(TransferOpts {
            successor: RemoveSuccessor::Designated([0xab; 32]),
        });
        let encoded = serde_json::to_string(&mode).unwrap();
        assert!(encoded.contains(&"ab".repeat(32)));
        let decoded: RemoveMode = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, mode);
    }

    #[test]
    fn test_remove_policy_rejects_invalid_defaults_and_duplicate_entries() {
        let resolver_default = RemovePolicy {
            default_mode: RemoveMode::ServerDefault,
            ..RemovePolicy::default()
        };
        assert!(!resolver_default.is_valid());

        let duplicate_mode = RemovePolicy {
            allowed_modes: vec![RemoveModeKind::Revoke, RemoveModeKind::Revoke],
            ..RemovePolicy::default()
        };
        assert!(!duplicate_mode.is_valid());

        let disallowed_default_successor = RemovePolicy {
            allowed_modes: vec![RemoveModeKind::Transfer],
            allowed_successors: vec![SuccessorKind::Member],
            default_mode: RemoveMode::Transfer(TransferOpts {
                successor: RemoveSuccessor::Admin,
            }),
        };
        assert!(!disallowed_default_successor.is_valid());

        let invalid_group_policy = Policy {
            remove_policy: resolver_default,
            ..Policy::default()
        };
        assert!(!invalid_group_policy.is_valid());
    }

    #[test]
    fn test_old_policy_records_default_the_new_remove_policy() {
        let mut old_record = serde_json::to_value(Policy::default()).unwrap();
        old_record.as_object_mut().unwrap().remove("remove_policy");
        let decoded: Policy = serde_json::from_value(old_record).unwrap();
        assert_eq!(decoded.remove_policy, RemovePolicy::default());
    }

    #[test]
    fn test_a_recent_window_must_hold_at_least_one_epoch() {
        let empty_window = Policy {
            history: History::Recent(0),
            ..Policy::default()
        };
        assert!(!empty_window.is_valid());
        let one_epoch = Policy {
            history: History::Recent(1),
            ..Policy::default()
        };
        assert!(one_epoch.is_valid());
    }
}
