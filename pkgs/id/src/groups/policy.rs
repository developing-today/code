//! Group policy records and templates (design §5.1 and §5.3).
//!
//! A policy record is written in full into the group's create event, and it changes only through
//! a `group.policy` event by an Admin. A template is a named preset that a create event names, and
//! the record must be one that template admits.

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use super::Role;

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

/// What happens to a removed member's grants (design §7.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum Delegation {
    /// Grants made before the removal stand. Grants made concurrently with it are void.
    #[value(name = "keep_prior")]
    KeepPrior,
    /// A grant stands only while its grantor still holds the scope. The default.
    Cascade,
    /// The removal names a successor, who takes over the removed member's grants from the removal
    /// on. Without a successor the grants end.
    Transfer,
}

/// What a joiner may see of earlier history (design §8.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum History {
    /// The whole history.
    #[default]
    Full,
    /// The last N epochs (P4.5). A joiner reads older messages only by asking a member (P4.6).
    /// N is at least 1.
    Recent(u32),
    /// Only what came after the join.
    FromJoin,
    /// Causal history (deferred).
    Causal,
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
    /// The fate of a removed member's grants. `None` means cascade (design §7.3). Read it through
    /// [`Policy::delegation_mode`].
    pub delegation: Option<Delegation>,
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
    pub const fn friendship() -> Self {
        Self {
            member_cap: Some(2),
            join: Join::MutualRequest,
            on_member_removed: OnMemberRemoved::Dissolve,
            removal: Removal::Strong,
            delegation: Some(Delegation::Cascade),
            history: History::Full,
            scheme: None,
            roles: RoleRules {
                change: Role::Admin,
            },
        }
    }

    /// The delegation mode in effect: the record's, or cascade when it names none.
    pub fn delegation_mode(&self) -> Delegation {
        self.delegation.unwrap_or(Delegation::Cascade)
    }

    /// Whether the record is usable. Only Admin may change roles and policy, a cap must allow
    /// at least one member, and a recent history window must hold at least one epoch.
    pub fn is_valid(&self) -> bool {
        self.roles.change == Role::Admin
            && self.member_cap.is_none_or(|cap| cap >= 1)
            && !matches!(self.history, History::Recent(0))
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
