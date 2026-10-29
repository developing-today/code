//! Server config (Phase 2, P2.9): who may create groups, what the server provisions at startup, and
//! each object's primary alias.
//!
//! The config is a TOML file. Every table is optional. An empty file is valid: anyone may create a
//! group, and nothing is provisioned.
//!
//! ```toml
//! [create]                       # absent: anyone may create a group
//! users = ["ann"]                # these users may create
//! groups = ["staff"]             # members of these groups may create
//! min_role = "write"             # the least role a member of a listed group may have
//!
//! [[users]]
//! key = "ann"                    # a local name, unique within the config
//! id = "<64 hex digits>"         # the user's Ed25519 public key
//! alias = "Ann"                  # optional primary alias
//!
//! [[groups]]
//! key = "staff"
//! name = "Staff"
//! creator = "ann"                # a user, who becomes the group's Admin
//! alias = "staff"                # optional primary alias
//!
//! [[groups.members]]
//! user = "bo"
//! role = "write"                 # pull, read, write, manage, or admin
//!
//! [policy]                       # absent: the code defaults apply
//! freshness_window_seconds = 300 # the server-level freshness window (N23)
//! pull_policy = "open"           # who may pull a world's events: "open" or "members" (P6.5)
//! ```
//!
//! [`ServerConfig::parse`] checks the whole file and translates every key to its ID, so a bad file
//! fails at startup. [`Provisioned::apply`] creates what the config names through the same paths a
//! user action takes, and a second run creates nothing.

mod parse;
pub mod provision;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use p2panda_core::Hash;

use crate::groups::{Key, Projection, Role};
use crate::pull_policy::PullPolicy;
use crate::seitan::Freshness;

/// Re-exported so `crate::server_config::Object` keeps working: the type now lives in
/// [`crate::aliases`] so alias display builds without the `world` feature.
pub use crate::aliases::Object;
pub use parse::ConfigError;
pub use provision::{ProvisionError, Provisioned, Report};

/// The server config, checked, with every key translated to its ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    users: Vec<UserSpec>,
    groups: Vec<GroupSpec>,
    create: CreateRule,
    policy: PolicySpec,
    /// Every local key, for users and groups alike. Keys are unique within the config.
    names: BTreeMap<String, Object>,
    /// The primary alias of each object that has one.
    aliases: BTreeMap<Object, String>,
}

/// A user the config creates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSpec {
    /// The local key.
    pub key: String,
    /// The user's public key.
    pub id: Key,
}

/// A group the config creates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSpec {
    /// The local key.
    pub key: String,
    /// The official name the group starts with. An Admin may change it later.
    pub name: String,
    /// The group's ID, which is the ID of its create event.
    pub id: Hash,
    /// The user who creates the group and is its Admin.
    pub creator: Key,
    /// The other members, in the order the config lists them, with their roles.
    pub members: Vec<(Key, Role)>,
}

/// The server's serving policy: the operator-set knobs of the freshness window (N23) and the
/// pull policy (P6.5). Both have code defaults, so a config that sets neither behaves exactly
/// as before: a five-minute window and open pull.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicySpec {
    /// The server-level freshness window. `None` keeps [`crate::seitan::DEFAULT_WINDOW`].
    pub freshness_window: Option<Duration>,
    /// Who may pull a world's events from this server.
    pub pull_policy: PullPolicy,
}

/// Who may create a group. Anyone may by default. A config may list who may.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateRule {
    /// Anyone may create a group.
    Anyone,
    /// Only the listed users, and members of the listed groups at or above `floor`.
    Listed {
        /// Users who may create.
        users: BTreeSet<Key>,
        /// Groups whose members may create.
        groups: BTreeSet<Hash>,
        /// The least role a member of a listed group may have. Unset in the file means any member.
        floor: Role,
    },
}

impl CreateRule {
    /// Whether `creator` may create a group, given the groups as they stand now. An inactive group
    /// grants nothing.
    #[must_use]
    pub fn allows(&self, creator: &Key, groups: &Projection) -> bool {
        match self {
            Self::Anyone => true,
            Self::Listed {
                users,
                groups: listed,
                floor,
            } => {
                users.contains(creator)
                    || listed.iter().any(|id| {
                        groups.group(id).is_some_and(|view| {
                            view.is_active()
                                && view.members.get(creator).is_some_and(|role| role >= floor)
                        })
                    })
            }
        }
    }
}

impl ServerConfig {
    /// Parses and checks a config. Every refusal names the key it is about.
    ///
    /// # Errors
    ///
    /// Fails on invalid TOML or an unknown field, a key declared twice, a reference to an unknown
    /// key, a malformed user ID, an unknown role, an empty name, alias, or `[create]`, and a group
    /// that lists its creator or a member twice.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        parse::parse(text)
    }

    /// Reads and parses the config at `path`.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, or on any error from [`ServerConfig::parse`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| ConfigError::Read(format!("{}: {error}", path.display())))?;
        Self::parse(&text)
    }

    /// The users the config creates, in file order.
    #[must_use]
    pub fn users(&self) -> &[UserSpec] {
        &self.users
    }

    /// The groups the config creates, in file order. A group may only admit a creator from groups
    /// listed before it.
    #[must_use]
    pub fn groups(&self) -> &[GroupSpec] {
        &self.groups
    }

    /// Who may create a group.
    #[must_use]
    pub const fn create_rule(&self) -> &CreateRule {
        &self.create
    }

    /// The server's serving policy, as the `[policy]` table set it (or the code defaults).
    #[must_use]
    pub const fn policy(&self) -> PolicySpec {
        self.policy
    }

    /// The freshness settings this server applies: the configured window at the server
    /// level, with the world and inviter levels unset, so a more specific setting made
    /// elsewhere still wins and the code default still applies when nothing is set.
    #[must_use]
    pub fn freshness(&self) -> Freshness {
        Freshness {
            server: self.policy.freshness_window,
            world: None,
            inviter: None,
        }
    }

    /// Who may pull a world's events from this server, as configured.
    #[must_use]
    pub const fn pull_policy(&self) -> PullPolicy {
        self.policy.pull_policy
    }

    /// The public key a user key names.
    #[must_use]
    pub fn user_id(&self, key: &str) -> Option<Key> {
        match self.names.get(key)? {
            Object::User(id) => Some(*id),
            Object::Group(_) => None,
        }
    }

    /// The ID a group key names.
    #[must_use]
    pub fn group_id(&self, key: &str) -> Option<Hash> {
        match self.names.get(key)? {
            Object::Group(id) => Some(*id),
            Object::User(_) => None,
        }
    }

    /// The primary alias the config sets for an object, if any. The alias display is P5.7.
    #[must_use]
    pub fn primary_alias(&self, object: &Object) -> Option<&str> {
        self.aliases.get(object).map(String::as_str)
    }
}
