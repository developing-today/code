//! `id group` subcommand definitions: accounts, membership, policy,
//! and delegation.

use clap::{Args, Subcommand, ValueEnum};

use id_events::groups::Role;
use id_events::groups::policy::Delegation;
use id_events::groups::policy::SuccessorKind;

/// Subcommands for `id group`. Accounts and groups are 64 hex characters.
#[derive(Subcommand, Debug)]
pub enum GroupCommand {
    /// Create a group. A normal group makes its creator Admin. A friendship names its other side
    /// with `--with`, has no Admin unless `--admins-on-create`, and is active once the other side
    /// accepts.
    Create {
        /// The kind of group to create.
        #[arg(long, value_enum, default_value = "normal")]
        template: GroupTemplate,
        /// The other side of a friendship (required for `--template friendship`).
        #[arg(long, value_name = "ACCOUNT")]
        with: Option<String>,
        /// The most members the group may have, counting the creator (normal groups).
        #[arg(long, value_name = "N")]
        max: Option<usize>,
        /// Make both friends Admin (friendships).
        #[arg(long)]
        admins_on_create: bool,
    },
    /// Show a group's policy record.
    Policy {
        /// The group.
        group: String,
    },
    /// Add a member with a role. The author must be a manager of the group.
    Add {
        /// The group.
        group: String,
        /// The account to add.
        member: String,
        /// The member's role.
        #[arg(long, value_enum, default_value = "read")]
        role: Role,
    },
    /// Remove a member. Removing yourself is leaving. The group's default mode is used when
    /// `--mode` is omitted.
    Remove {
        /// The group.
        group: String,
        /// The account to remove.
        member: String,
        /// Grant-disposition mode. Defaults to the group's configured remove default.
        #[arg(long, value_enum)]
        mode: Option<GroupRemoveMode>,
        /// The successor category for `--mode transfer` (defaults to `member`).
        #[arg(long, value_enum)]
        successor_kind: Option<SuccessorKind>,
        /// The account for a `member` or `designated` transfer successor.
        #[arg(long, value_name = "ACCOUNT")]
        successor: Option<String>,
    },
    /// Set a member's role. Only an Admin may.
    #[command(name = "role")]
    SetRole {
        /// The group.
        group: String,
        /// The member whose role changes.
        member: String,
        /// Its new role.
        #[arg(value_enum)]
        role: Role,
    },
    /// List the groups in the log, one line each.
    List,
    /// Unavailable: group admission and network joining are not implemented yet.
    Join {
        /// The pasted ticket or link.
        invite: String,
    },
}

/// The kinds of group `id group create` makes.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupTemplate {
    /// A group whose creator is Admin. Members are added by a manager, and a cap may be set.
    Normal,
    /// Two members who ask and accept each other.
    Friendship,
}

/// A requested mode for `id group remove`; `server-default` is resolved before recording.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupRemoveMode {
    /// Resolve to the group's configured removal default.
    ServerDefault,
    /// End the removed member's grants.
    Revoke,
    /// Transfer the removed member's grants to a successor.
    Transfer,
    /// Keep grants under their original grantor.
    KeepPrior,
    /// Re-anchor grants at the group.
    ReassignToGroup,
}

/// The arguments of `id delegate`.
#[derive(Args, Debug)]
pub struct DelegateArgs {
    /// The group.
    pub group: String,
    /// The account that is granted the scope.
    pub grantee: String,
    /// The scope to grant.
    #[arg(long, value_enum, default_value = "read")]
    pub scope: Role,
    /// Let the grantee pass the scope on to others.
    #[arg(long)]
    pub redelegate: bool,
    /// The group's delegation mode, set first if it differs: `keep_prior`, `cascade` or `transfer`.
    #[arg(long, value_enum)]
    pub mode: Option<Delegation>,
    /// Print JSON instead of a line.
    #[arg(long)]
    pub json: bool,
}

/// The arguments of `id revoke`.
#[derive(Args, Debug)]
pub struct RevokeArgs {
    /// The group.
    pub group: String,
    /// The grant's event ID, as `id delegate` printed it.
    pub grant: String,
    /// Print JSON instead of a line.
    #[arg(long)]
    pub json: bool,
}
