//! Parsing and checking a server config (P2.9). Every refusal names the key it is about.
//!
//! A file is read in three passes. The users come first, so a group can name its creator. Each
//! group is then claimed with its ID. Last, members and the create rule are resolved against every
//! key. A key is claimed once across users and groups, so it names one object.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use p2panda_core::Hash;
use serde::Deserialize;
use thiserror::Error;

use super::{CreateRule, GroupSpec, Object, PolicySpec, ServerConfig, UserSpec};
use crate::directory::{hex_to_array, normalize_key};
use crate::groups::{Key, Role};
use crate::pull_policy::PullPolicy;

/// Why a config was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read the server config: {0}")]
    Read(String),
    /// The text is not valid TOML, or it has a field the format does not have.
    #[error("the server config is not valid: {0}")]
    Parse(String),
    /// A key names two users, two groups, or a user and a group.
    #[error("key `{0}` is declared more than once; keys are unique within the config")]
    DuplicateKey(String),
    /// A user or group declaration key is blank or carries surrounding whitespace.
    #[error(
        "{kind} key `{key}` is not a usable declaration key; keys are non-blank and carry no surrounding whitespace"
    )]
    InvalidKey {
        /// The kind of declaration the key belongs to.
        kind: &'static str,
        /// The key exactly as declared.
        key: String,
    },
    /// Two user keys name the same public key, so the two declared users are one account.
    #[error(
        "users `{first}` and `{second}` name the same id; each user key must name a different id"
    )]
    DuplicateId {
        /// The first key that named the ID.
        first: String,
        /// The later key that named the same ID.
        second: String,
    },
    /// A reference names no key of the kind it needs.
    #[error("unknown {kind} key `{key}` in {place}")]
    UnknownKey {
        /// The kind the reference needs.
        kind: &'static str,
        /// The key named.
        key: String,
        /// Where the reference is.
        place: String,
    },
    /// A reference names a key of the other kind.
    #[error("`{key}` names a {found}, not a {expected}, in {place}")]
    WrongKind {
        /// The key named.
        key: String,
        /// The kind the reference needs.
        expected: &'static str,
        /// The kind the key names.
        found: &'static str,
        /// Where the reference is.
        place: String,
    },
    /// A user's ID is not an Ed25519 public key written as 64 hex digits.
    #[error("user `{key}` has a malformed id: {reason}")]
    MalformedId {
        /// The user's key.
        key: String,
        /// What is wrong with the ID.
        reason: String,
    },
    /// A role name is none of the five roles.
    #[error("`{role}` in {place} is not a role; use pull, read, write, manage, or admin")]
    UnknownRole {
        /// The name given.
        role: String,
        /// Where it is.
        place: String,
    },
    /// A name or alias is blank.
    #[error("`{key}` has an empty {field}")]
    Empty {
        /// The key of the object.
        key: String,
        /// Which field is blank.
        field: &'static str,
    },
    /// `[create]` lists no user and no group, so no one could create a group.
    #[error("[create] lists no user and no group, so no one could create a group")]
    EmptyCreate,
    /// A group lists its creator as a member. The creator is its Admin already.
    #[error(
        "group `{group}` lists its creator `{creator}` as a member; the creator is its Admin already"
    )]
    CreatorListed {
        /// The group's key.
        group: String,
        /// The creator's key.
        creator: String,
    },
    /// A pull policy name is neither of the two policies.
    #[error("`{policy}` in {place} is not a pull policy; use open or members")]
    UnknownPullPolicy {
        /// The name given.
        policy: String,
        /// Where it is.
        place: String,
    },
    /// The freshness window is zero seconds, so no accept could ever fall inside it.
    #[error("the freshness window in [policy] is 0 seconds; omit it or give at least 1")]
    ZeroWindow,
    /// A `[budgets]` value is zero, so the budget could never admit anything.
    #[error("`{field}` in [budgets] is 0; omit it or give at least 1")]
    ZeroBudget {
        /// The field that was set to zero.
        field: &'static str,
    },
    /// A `[[budgets.counterparty]]` window is longer than the millisecond
    /// window it converts to can represent, so accepting it would silently
    /// run a shorter window than the one configured.
    #[error(
        "`window_seconds` in [budgets] is {seconds} seconds, which overflows milliseconds; give at most {max_seconds}"
    )]
    WindowTooLarge {
        /// The window length given, in seconds.
        seconds: u64,
        /// The largest window length that converts to milliseconds exactly.
        max_seconds: u64,
    },
    /// Two `[[budgets.counterparty]]` rows name the same peer.
    #[error("peer `{peer}` has more than one [[budgets.counterparty]] row; give each peer one row")]
    DuplicateCounterparty {
        /// The peer named twice, as written in the file.
        peer: String,
    },
    /// A group lists one member twice.
    #[error("group `{group}` lists member `{member}` more than once")]
    DuplicateMember {
        /// The group's key.
        group: String,
        /// The member's key.
        member: String,
    },
    /// `[create]` lists the same user or group twice, so the repetition is a typo.
    #[error("`{key}` is listed more than once in {list}")]
    DuplicateCreate {
        /// The key exactly as declared in the repeated entry.
        key: String,
        /// Which `[create]` list held the repetition.
        list: &'static str,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    create: Option<RawCreate>,
    policy: Option<RawPolicy>,
    budgets: Option<RawBudgets>,
    #[serde(default)]
    users: Vec<RawUser>,
    #[serde(default)]
    groups: Vec<RawGroup>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCreate {
    #[serde(default)]
    users: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
    min_role: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudgets {
    app_inbound_bytes_per_second: Option<u64>,
    app_inbound_bytes_per_hour: Option<u64>,
    app_outbound_bytes_per_second: Option<u64>,
    app_outbound_bytes_per_hour: Option<u64>,
    per_peer_bytes_per_hour: Option<u64>,
    per_peer_burst_bytes_per_minute: Option<u64>,
    per_ip_bytes_per_hour: Option<u64>,
    #[serde(default)]
    counterparty: Vec<RawCounterparty>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCounterparty {
    peer: String,
    bytes_per_window: u64,
    window_seconds: u64,
    messages_per_window: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    freshness_window_seconds: Option<u64>,
    pull_policy: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUser {
    key: String,
    id: String,
    alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGroup {
    key: String,
    name: String,
    creator: String,
    alias: Option<String>,
    #[serde(default)]
    members: Vec<RawMember>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMember {
    user: String,
    role: String,
}

/// Parses and checks `text`. See the module doc for the passes.
pub(super) fn parse(text: &str) -> Result<ServerConfig, ConfigError> {
    let raw: Raw = toml::from_str(text).map_err(|error| ConfigError::Parse(error.to_string()))?;
    let mut names = BTreeMap::new();
    let mut aliases = BTreeMap::new();

    let mut users = Vec::with_capacity(raw.users.len());
    let mut user_ids: BTreeMap<Key, String> = BTreeMap::new();
    for user in &raw.users {
        check_declaration_key("user", &user.key)?;
        let id = parse_id(&user.key, &user.id)?;
        claim(&mut names, &user.key, Object::User(id))?;
        if let Some(first) = user_ids.get(&id) {
            return Err(ConfigError::DuplicateId {
                first: first.clone(),
                second: user.key.clone(),
            });
        }
        user_ids.insert(id, user.key.clone());
        record_alias(
            &mut aliases,
            &user.key,
            Object::User(id),
            user.alias.as_deref(),
        )?;
        users.push(UserSpec {
            key: user.key.clone(),
            id,
        });
    }

    let mut pending = Vec::with_capacity(raw.groups.len());
    for group in &raw.groups {
        check_declaration_key("group", &group.key)?;
        let creator = user_of(&names, &group.creator, &format!("group `{}`", group.key))?;
        let id = group_id(&creator, &group.key);
        claim(&mut names, &group.key, Object::Group(id))?;
        if group.name.trim().is_empty() {
            return Err(ConfigError::Empty {
                key: group.key.clone(),
                field: "name",
            });
        }
        record_alias(
            &mut aliases,
            &group.key,
            Object::Group(id),
            group.alias.as_deref(),
        )?;
        pending.push((group, creator, id));
    }

    let mut groups = Vec::with_capacity(pending.len());
    for (group, creator, id) in pending {
        let place = format!("the members of group `{}`", group.key);
        let mut members: Vec<(Key, Role)> = Vec::with_capacity(group.members.len());
        for member in &group.members {
            let key = user_of(&names, &member.user, &place)?;
            let role = parse_role(&member.role, &place)?;
            if key == creator {
                return Err(ConfigError::CreatorListed {
                    group: group.key.clone(),
                    creator: group.creator.clone(),
                });
            }
            if members.iter().any(|(listed, _)| *listed == key) {
                return Err(ConfigError::DuplicateMember {
                    group: group.key.clone(),
                    member: member.user.clone(),
                });
            }
            members.push((key, role));
        }
        groups.push(GroupSpec {
            key: group.key.clone(),
            name: group.name.trim().to_owned(),
            id,
            creator,
            members,
        });
    }

    let create = match &raw.create {
        None => CreateRule::Anyone,
        Some(rule) => create_rule(rule, &names)?,
    };
    let policy = match &raw.policy {
        None => PolicySpec::default(),
        Some(raw_policy) => policy_spec(raw_policy)?,
    };
    let budgets = match &raw.budgets {
        None => crate::abuse::BudgetPolicy::default(),
        Some(raw_budgets) => budget_policy(raw_budgets, &names)?,
    };
    Ok(ServerConfig {
        users,
        groups,
        create,
        policy,
        budgets,
        names,
        aliases,
    })
}

/// The #2247 budgets from their `[budgets]` table. Every field the
/// table leaves unset keeps its code default, so a table that sets
/// one knob changes only that knob. Counterparty `peer` names a
/// declared user key or a raw 64-hex public key; each peer may have
/// one row, and the row replaces the per-peer defaults for it.
fn budget_policy(
    raw: &RawBudgets,
    names: &BTreeMap<String, Object>,
) -> Result<crate::abuse::BudgetPolicy, ConfigError> {
    use crate::abuse::BudgetLimit;
    let mut policy = crate::abuse::BudgetPolicy::default();
    let checked = |field: &'static str, value: Option<u64>| -> Result<Option<u64>, ConfigError> {
        match value {
            Some(0) => Err(ConfigError::ZeroBudget { field }),
            other => Ok(other),
        }
    };
    if let Some(v) = checked(
        "app_inbound_bytes_per_second",
        raw.app_inbound_bytes_per_second,
    )? {
        policy.app_inbound[0] = BudgetLimit::bytes(1_000, v);
    }
    if let Some(v) = checked("app_inbound_bytes_per_hour", raw.app_inbound_bytes_per_hour)? {
        policy.app_inbound[1] = BudgetLimit::bytes(3_600_000, v);
    }
    if let Some(v) = checked(
        "app_outbound_bytes_per_second",
        raw.app_outbound_bytes_per_second,
    )? {
        policy.app_outbound[0] = BudgetLimit::bytes(1_000, v);
    }
    if let Some(v) = checked(
        "app_outbound_bytes_per_hour",
        raw.app_outbound_bytes_per_hour,
    )? {
        policy.app_outbound[1] = BudgetLimit::bytes(3_600_000, v);
    }
    if let Some(v) = checked(
        "per_peer_burst_bytes_per_minute",
        raw.per_peer_burst_bytes_per_minute,
    )? {
        policy.per_peer[0] = BudgetLimit::bytes(60_000, v);
    }
    if let Some(v) = checked("per_peer_bytes_per_hour", raw.per_peer_bytes_per_hour)? {
        policy.per_peer[1] = BudgetLimit::bytes(3_600_000, v);
    }
    if let Some(v) = checked("per_ip_bytes_per_hour", raw.per_ip_bytes_per_hour)? {
        policy.per_ip[0] = BudgetLimit::bytes(3_600_000, v);
    }
    let mut seen: BTreeSet<crate::abuse::EndpointId> = BTreeSet::new();
    for row in &raw.counterparty {
        if row.bytes_per_window == 0 {
            return Err(ConfigError::ZeroBudget {
                field: "bytes_per_window",
            });
        }
        if row.window_seconds == 0 {
            return Err(ConfigError::ZeroBudget {
                field: "window_seconds",
            });
        }
        if row.window_seconds > u64::MAX / 1_000 {
            return Err(ConfigError::WindowTooLarge {
                seconds: row.window_seconds,
                max_seconds: u64::MAX / 1_000,
            });
        }
        if row.messages_per_window == Some(0) {
            return Err(ConfigError::ZeroBudget {
                field: "messages_per_window",
            });
        }
        let peer = counterparty_peer(&row.peer, names)?;
        if !seen.insert(peer) {
            return Err(ConfigError::DuplicateCounterparty {
                peer: row.peer.clone(),
            });
        }
        policy.counterparty_overrides.push((
            peer,
            vec![BudgetLimit {
                // Exact: the check above refuses any window whose
                // millisecond conversion would overflow.
                window_ms: row.window_seconds * 1_000,
                bytes_per_window: row.bytes_per_window,
                messages_per_window: row.messages_per_window.unwrap_or(0),
            }],
        ));
    }
    Ok(policy)
}

/// Resolves a counterparty `peer` to an endpoint ID: a declared
/// user key names that user; anything else must be the peer's
/// 64-hex public key.
fn counterparty_peer(
    peer: &str,
    names: &BTreeMap<String, Object>,
) -> Result<crate::abuse::EndpointId, ConfigError> {
    if let Some(object) = names.get(peer) {
        return match object {
            Object::User(id) => Ok(*id),
            Object::Group(_) => Err(ConfigError::WrongKind {
                key: peer.to_owned(),
                expected: "user",
                found: "group",
                place: "a [[budgets.counterparty]] row".to_owned(),
            }),
        };
    }
    let unknown = || ConfigError::UnknownKey {
        kind: "user",
        key: peer.to_owned(),
        place: "a [[budgets.counterparty]] row".to_owned(),
    };
    let normalized = normalize_key(peer).map_err(|_| unknown())?;
    hex_to_array::<32>(&normalized).ok_or_else(unknown)
}

/// The serving policy from its `[policy]` table. An absent table, and a table that sets
/// neither knob, both give the code defaults.
fn policy_spec(raw: &RawPolicy) -> Result<PolicySpec, ConfigError> {
    let freshness_window = match raw.freshness_window_seconds {
        None => None,
        Some(0) => return Err(ConfigError::ZeroWindow),
        Some(seconds) => Some(Duration::from_secs(seconds)),
    };
    let pull_policy = match &raw.pull_policy {
        None => PullPolicy::Open,
        Some(name) => parse_pull_policy(name, "[policy].pull_policy")?,
    };
    Ok(PolicySpec {
        freshness_window,
        pull_policy,
    })
}

fn parse_pull_policy(name: &str, place: &str) -> Result<PullPolicy, ConfigError> {
    match name.trim().to_ascii_lowercase().as_str() {
        "open" => Ok(PullPolicy::Open),
        "members" => Ok(PullPolicy::Members),
        _ => Err(ConfigError::UnknownPullPolicy {
            policy: name.to_owned(),
            place: place.to_owned(),
        }),
    }
}

/// The create rule from its `[create]` table, with every name resolved.
fn create_rule(
    rule: &RawCreate,
    names: &BTreeMap<String, Object>,
) -> Result<CreateRule, ConfigError> {
    if rule.users.is_empty() && rule.groups.is_empty() {
        return Err(ConfigError::EmptyCreate);
    }
    let mut users = BTreeSet::new();
    for key in &rule.users {
        let id = user_of(names, key, "[create].users")?;
        if !users.insert(id) {
            return Err(ConfigError::DuplicateCreate {
                key: key.clone(),
                list: "[create].users",
            });
        }
    }
    let mut groups = BTreeSet::new();
    for key in &rule.groups {
        let id = group_of(names, key, "[create].groups")?;
        if !groups.insert(id) {
            return Err(ConfigError::DuplicateCreate {
                key: key.clone(),
                list: "[create].groups",
            });
        }
    }
    let floor = match &rule.min_role {
        None => Role::Pull,
        Some(name) => parse_role(name, "[create].min_role")?,
    };
    Ok(CreateRule::Listed {
        users,
        groups,
        floor,
    })
}

/// A declaration key must be non-blank and exactly as referenced: surrounding whitespace is
/// refused, never trimmed, so a key cannot diverge from the trimmed account name that
/// provisioning derives from it, and references keep exact-key semantics.
fn check_declaration_key(kind: &'static str, key: &str) -> Result<(), ConfigError> {
    if key.trim().is_empty() || key.trim() != key {
        return Err(ConfigError::InvalidKey {
            kind,
            key: key.to_owned(),
        });
    }
    Ok(())
}

/// Records that `key` names `object`. Keys are unique across users and groups.
fn claim(
    names: &mut BTreeMap<String, Object>,
    key: &str,
    object: Object,
) -> Result<(), ConfigError> {
    if names.contains_key(key) {
        return Err(ConfigError::DuplicateKey(key.to_owned()));
    }
    names.insert(key.to_owned(), object);
    Ok(())
}

/// Stores the object's primary alias, if the file sets one. A blank alias is refused.
fn record_alias(
    aliases: &mut BTreeMap<Object, String>,
    key: &str,
    object: Object,
    alias: Option<&str>,
) -> Result<(), ConfigError> {
    let Some(alias) = alias else {
        return Ok(());
    };
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(ConfigError::Empty {
            key: key.to_owned(),
            field: "alias",
        });
    }
    aliases.insert(object, alias.to_owned());
    Ok(())
}

/// The public key a user key names.
fn user_of(names: &BTreeMap<String, Object>, key: &str, place: &str) -> Result<Key, ConfigError> {
    match names.get(key) {
        Some(Object::User(id)) => Ok(*id),
        Some(Object::Group(_)) => Err(ConfigError::WrongKind {
            key: key.to_owned(),
            expected: "user",
            found: "group",
            place: place.to_owned(),
        }),
        None => Err(ConfigError::UnknownKey {
            kind: "user",
            key: key.to_owned(),
            place: place.to_owned(),
        }),
    }
}

/// The ID a group key names.
fn group_of(names: &BTreeMap<String, Object>, key: &str, place: &str) -> Result<Hash, ConfigError> {
    match names.get(key) {
        Some(Object::Group(id)) => Ok(*id),
        Some(Object::User(_)) => Err(ConfigError::WrongKind {
            key: key.to_owned(),
            expected: "group",
            found: "user",
            place: place.to_owned(),
        }),
        None => Err(ConfigError::UnknownKey {
            kind: "group",
            key: key.to_owned(),
            place: place.to_owned(),
        }),
    }
}

/// The public key a user's ID names. It is written as 64 hex digits, in either case.
fn parse_id(key: &str, id: &str) -> Result<Key, ConfigError> {
    let malformed = |reason: String| ConfigError::MalformedId {
        key: key.to_owned(),
        reason,
    };
    let normalized = normalize_key(id).map_err(|error| malformed(format!("{error:#}")))?;
    hex_to_array::<32>(&normalized).ok_or_else(|| malformed("not 64 hex digits".to_owned()))
}

fn parse_role(role: &str, place: &str) -> Result<Role, ConfigError> {
    match role.trim().to_ascii_lowercase().as_str() {
        "pull" => Ok(Role::Pull),
        "read" => Ok(Role::Read),
        "write" => Ok(Role::Write),
        "manage" => Ok(Role::Manage),
        "admin" => Ok(Role::Admin),
        _ => Err(ConfigError::UnknownRole {
            role: role.to_owned(),
            place: place.to_owned(),
        }),
    }
}

/// The ID of the group a creator makes under `key`. It is the same on every start, so a restart
/// finds the group it made before.
fn group_id(creator: &Key, key: &str) -> Hash {
    let mut bytes = b"id-config-group-v1".to_vec();
    bytes.extend_from_slice(creator);
    bytes.extend_from_slice(key.as_bytes());
    Hash::digest(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::Path;

    use ed25519_dalek::SigningKey;

    use crate::groups::{Key, Role};
    use crate::pull_policy::PullPolicy;
    use crate::server_config::{ConfigError, CreateRule, Object, PolicySpec, ServerConfig};
    use crate::world::hex_encode;

    /// The public key of the account that the test calls `n`.
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

    /// The four users, then `extra`.
    fn config(extra: &str) -> Result<ServerConfig, ConfigError> {
        ServerConfig::parse(&format!("{}{extra}", users()))
    }

    /// A group named `team`, created by ann, with bo as a writer.
    const TEAM: &str = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                        [[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n";

    #[test]
    fn test_a_config_translates_every_key_to_its_id() {
        let config = config(TEAM).unwrap();
        assert_eq!(config.user_id("ann"), Some(key(1)));
        assert_eq!(config.user_id("bo"), Some(key(2)));
        let team = config.group_id("team").unwrap();
        let group = &config.groups()[0];
        assert_eq!(group.id, team);
        assert_eq!(group.name, "Team");
        assert_eq!(group.creator, key(1));
        assert_eq!(group.members, vec![(key(2), Role::Write)]);
        assert_eq!(config.user_id("team"), None, "a group key is not a user");
        assert_eq!(config.group_id("ann"), None, "a user key is not a group");
    }

    #[test]
    fn test_no_policy_table_keeps_the_code_defaults() {
        let config = config("").unwrap();
        assert_eq!(config.pull_policy(), PullPolicy::Open);
        assert_eq!(config.freshness().window(), crate::seitan::DEFAULT_WINDOW);
        assert_eq!(config.policy(), PolicySpec::default());
    }

    #[test]
    fn test_a_policy_table_sets_both_knobs() {
        let config =
            config("\n[policy]\nfreshness_window_seconds = 60\npull_policy = \"members\"\n")
                .unwrap();
        assert_eq!(config.pull_policy(), PullPolicy::Members);
        assert_eq!(
            config.freshness().server,
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(
            config.freshness().window(),
            std::time::Duration::from_secs(60)
        );
    }

    #[test]
    fn test_an_unknown_pull_policy_fails_startup() {
        let refused = config("\n[policy]\npull_policy = \"friends\"\n").unwrap_err();
        assert!(matches!(refused, ConfigError::UnknownPullPolicy { .. }));
    }

    #[test]
    fn test_an_unknown_policy_key_fails_startup() {
        let refused = config("\n[policy]\npull_policy = \"open\"\nwindow = 60\n").unwrap_err();
        assert!(matches!(refused, ConfigError::Parse(_)));
    }

    #[test]
    fn test_a_zero_freshness_window_fails_startup() {
        let refused = config("\n[policy]\nfreshness_window_seconds = 0\n").unwrap_err();
        assert_eq!(refused, ConfigError::ZeroWindow);
    }

    #[test]
    fn test_the_same_text_parses_to_the_same_config() {
        assert_eq!(config(TEAM).unwrap(), config(TEAM).unwrap());
    }

    #[test]
    fn test_a_user_key_declared_twice_is_refused_naming_it() {
        let again = format!("\n[[users]]\nkey = \"ann\"\nid = \"{}\"\n", hex(9));
        assert_eq!(
            config(&again),
            Err(ConfigError::DuplicateKey("ann".to_owned()))
        );
    }

    #[test]
    fn test_a_blank_user_key_is_refused_naming_it() {
        let text = format!("\n[[users]]\nkey = \"\"\nid = \"{}\"\n", hex(9));
        assert_eq!(
            config(&text),
            Err(ConfigError::InvalidKey {
                kind: "user",
                key: String::new(),
            })
        );
    }

    #[test]
    fn test_a_whitespace_only_user_key_is_refused() {
        let text = format!("\n[[users]]\nkey = \"  \"\nid = \"{}\"\n", hex(9));
        assert_eq!(
            config(&text),
            Err(ConfigError::InvalidKey {
                kind: "user",
                key: "  ".to_owned(),
            })
        );
    }

    #[test]
    fn test_a_padded_user_key_is_refused_not_trimmed() {
        let text = format!("\n[[users]]\nkey = \" eve\"\nid = \"{}\"\n", hex(9));
        assert_eq!(
            config(&text),
            Err(ConfigError::InvalidKey {
                kind: "user",
                key: " eve".to_owned(),
            })
        );
    }

    #[test]
    fn test_a_blank_group_key_is_refused_naming_it() {
        let text = "\n[[groups]]\nkey = \"\"\nname = \"Team\"\ncreator = \"ann\"\n";
        assert_eq!(
            config(text),
            Err(ConfigError::InvalidKey {
                kind: "group",
                key: String::new(),
            })
        );
    }

    #[test]
    fn test_a_padded_group_key_is_refused_not_trimmed() {
        let text = "\n[[groups]]\nkey = \"team \"\nname = \"Team\"\ncreator = \"ann\"\n";
        assert_eq!(
            config(text),
            Err(ConfigError::InvalidKey {
                kind: "group",
                key: "team ".to_owned(),
            })
        );
    }

    #[test]
    fn test_a_padded_reference_does_not_resolve_to_a_declared_key() {
        let text = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                    [[groups.members]]\nuser = \" bo\"\nrole = \"write\"\n";
        let refused = config(text).unwrap_err();
        assert!(matches!(refused, ConfigError::UnknownKey { .. }));
    }

    #[test]
    fn test_two_user_keys_naming_the_same_id_are_refused_naming_both() {
        let text = format!(
            "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n",
            hex(1),
            hex(1)
        );
        assert_eq!(
            ServerConfig::parse(&text),
            Err(ConfigError::DuplicateId {
                first: "ann".to_owned(),
                second: "bo".to_owned(),
            })
        );
    }

    #[test]
    fn test_the_same_id_in_a_different_case_is_still_a_duplicate() {
        let text = format!(
            "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n",
            hex(1),
            hex(1).to_ascii_uppercase()
        );
        assert_eq!(
            ServerConfig::parse(&text),
            Err(ConfigError::DuplicateId {
                first: "ann".to_owned(),
                second: "bo".to_owned(),
            })
        );
    }

    #[test]
    fn test_two_users_with_distinct_ids_may_share_an_alias_text() {
        let text = format!(
            "[[users]]\nkey = \"ann\"\nid = \"{}\"\nalias = \"Pal\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\nalias = \"Pal\"\n",
            hex(1),
            hex(2)
        );
        let config = ServerConfig::parse(&text).unwrap();
        assert_eq!(config.user_id("ann"), Some(key(1)));
        assert_eq!(config.user_id("bo"), Some(key(2)));
    }

    #[test]
    fn test_a_group_key_that_reuses_a_user_key_is_refused_naming_it() {
        let extra = "\n[[groups]]\nkey = \"bo\"\nname = \"Bo\"\ncreator = \"ann\"\n";
        assert_eq!(
            config(extra),
            Err(ConfigError::DuplicateKey("bo".to_owned()))
        );
    }

    #[test]
    fn test_two_groups_with_one_key_are_refused_naming_it() {
        let extra = format!("{TEAM}{TEAM}");
        assert_eq!(
            config(&extra),
            Err(ConfigError::DuplicateKey("team".to_owned()))
        );
    }

    #[test]
    fn test_an_unknown_creator_is_refused_naming_the_key() {
        let extra = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"zed\"\n";
        assert!(matches!(
            config(extra),
            Err(ConfigError::UnknownKey { ref key, kind: "user", .. }) if key == "zed"
        ));
    }

    #[test]
    fn test_an_unknown_member_is_refused_naming_the_key() {
        let extra = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                     [[groups.members]]\nuser = \"zed\"\nrole = \"read\"\n";
        assert!(matches!(
            config(extra),
            Err(ConfigError::UnknownKey { ref key, kind: "user", .. }) if key == "zed"
        ));
    }

    #[test]
    fn test_an_unknown_name_in_the_create_rule_is_refused_naming_it() {
        assert!(matches!(
            config("\n[create]\nusers = [\"zed\"]\n"),
            Err(ConfigError::UnknownKey { ref key, kind: "user", .. }) if key == "zed"
        ));
        assert!(matches!(
            config(&format!("{TEAM}\n[create]\ngroups = [\"nope\"]\n")),
            Err(ConfigError::UnknownKey { ref key, kind: "group", .. }) if key == "nope"
        ));
    }

    #[test]
    fn test_a_key_of_the_wrong_kind_is_refused_naming_it() {
        // A group named where a user belongs, and a user named where a group belongs.
        assert!(matches!(
            config(&format!("{TEAM}\n[create]\ngroups = [\"ann\"]\n")),
            Err(ConfigError::WrongKind { ref key, expected: "group", .. }) if key == "ann"
        ));
        let extra = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n\
                     [[groups.members]]\nuser = \"team\"\nrole = \"read\"\n";
        assert!(matches!(
            config(extra),
            Err(ConfigError::WrongKind { ref key, expected: "user", .. }) if key == "team"
        ));
    }

    #[test]
    fn test_an_unknown_role_is_refused_naming_it() {
        let member = TEAM.replace("\"write\"", "\"owner\"");
        assert!(matches!(
            config(&member),
            Err(ConfigError::UnknownRole { ref role, .. }) if role == "owner"
        ));
        assert!(matches!(
            config(&format!("{TEAM}\n[create]\ngroups = [\"team\"]\nmin_role = \"owner\"\n")),
            Err(ConfigError::UnknownRole { ref role, .. }) if role == "owner"
        ));
    }

    #[test]
    fn test_every_role_name_is_accepted_and_case_is_ignored() {
        for (name, role) in [
            ("pull", Role::Pull),
            ("read", Role::Read),
            ("write", Role::Write),
            ("manage", Role::Manage),
            ("Admin", Role::Admin),
        ] {
            let member = TEAM.replace("\"write\"", &format!("\"{name}\""));
            let config = config(&member).unwrap();
            assert_eq!(config.groups()[0].members, vec![(key(2), role)], "{name}");
        }
    }

    #[test]
    fn test_a_malformed_user_id_is_refused_naming_the_user() {
        let short = users().replace(&hex(1), "abc");
        assert!(matches!(
            ServerConfig::parse(&short),
            Err(ConfigError::MalformedId { ref key, .. }) if key == "ann"
        ));
        let not_hex = users().replace(&hex(1), &"zz".repeat(32));
        assert!(matches!(
            ServerConfig::parse(&not_hex),
            Err(ConfigError::MalformedId { ref key, .. }) if key == "ann"
        ));
    }

    #[test]
    fn test_an_uppercase_user_id_is_the_same_user() {
        let upper = users().replace(&hex(1), &hex(1).to_ascii_uppercase());
        let config = ServerConfig::parse(&upper).unwrap();
        assert_eq!(config.user_id("ann"), Some(key(1)));
    }

    #[test]
    fn test_a_creator_listed_as_a_member_is_refused() {
        let member = TEAM.replace("user = \"bo\"", "user = \"ann\"");
        assert_eq!(
            config(&member),
            Err(ConfigError::CreatorListed {
                group: "team".to_owned(),
                creator: "ann".to_owned(),
            })
        );
    }

    #[test]
    fn test_a_member_listed_twice_is_refused_naming_it() {
        let twice = format!("{TEAM}\n[[groups.members]]\nuser = \"bo\"\nrole = \"read\"\n");
        assert_eq!(
            config(&twice),
            Err(ConfigError::DuplicateMember {
                group: "team".to_owned(),
                member: "bo".to_owned(),
            })
        );
    }

    #[test]
    fn test_an_empty_create_table_is_refused() {
        assert_eq!(config("\n[create]\n"), Err(ConfigError::EmptyCreate));
    }

    #[test]
    fn test_an_empty_name_or_alias_is_refused_naming_its_key() {
        let blank_name = TEAM.replace("name = \"Team\"", "name = \"  \"");
        assert_eq!(
            config(&blank_name),
            Err(ConfigError::Empty {
                key: "team".to_owned(),
                field: "name",
            })
        );
        let blank_alias = users().replace(
            &format!("id = \"{}\"", hex(2)),
            &format!("id = \"{}\"\nalias = \"\"", hex(2)),
        );
        assert_eq!(
            ServerConfig::parse(&blank_alias),
            Err(ConfigError::Empty {
                key: "bo".to_owned(),
                field: "alias",
            })
        );
    }

    #[test]
    fn test_an_unknown_field_is_refused() {
        let extra = format!("{}\nhue = \"red\"\n", users());
        assert!(matches!(
            ServerConfig::parse(&extra),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn test_a_primary_alias_is_stored_and_read_back() {
        let text = format!(
            "{}\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\nalias = \"team\"\n",
            users().replacen("key = \"ann\"", "key = \"ann\"\nalias = \"Ann\"", 1)
        );
        let config = ServerConfig::parse(&text).unwrap();
        let team = config.group_id("team").unwrap();
        assert_eq!(
            config.primary_alias(&Object::Group(team)),
            Some("team"),
            "the group's alias is read back"
        );
        assert_eq!(
            config.primary_alias(&Object::User(key(1))),
            Some("Ann"),
            "the user's alias is read back"
        );
        assert_eq!(
            config.primary_alias(&Object::User(key(2))),
            None,
            "an object with no alias has none"
        );
    }

    #[test]
    fn test_without_a_create_table_anyone_may_create() {
        assert_eq!(config(TEAM).unwrap().create_rule(), &CreateRule::Anyone);
    }

    #[test]
    fn test_the_create_rule_keeps_its_listed_users_groups_and_floor() {
        let parsed = config(&format!(
            "{TEAM}\n[create]\nusers = [\"ann\"]\ngroups = [\"team\"]\nmin_role = \"write\"\n"
        ))
        .unwrap();
        let team = parsed.group_id("team").unwrap();
        assert_eq!(
            parsed.create_rule(),
            &CreateRule::Listed {
                users: std::iter::once(key(1)).collect(),
                groups: std::iter::once(team).collect(),
                floor: Role::Write,
            }
        );
    }

    #[test]
    fn test_budgets_default_when_the_table_is_absent() {
        assert_eq!(
            config("").unwrap().budgets(),
            &crate::abuse::BudgetPolicy::default()
        );
    }

    #[test]
    fn test_budgets_table_overrides_only_the_knobs_it_sets() {
        let parsed = config("[budgets]\nper_peer_bytes_per_hour = 12345\n").unwrap();
        let budgets = parsed.budgets();
        assert_eq!(
            budgets.per_peer[1],
            crate::abuse::BudgetLimit::bytes(3_600_000, 12345)
        );
        // Everything else keeps the code default.
        let defaults = crate::abuse::BudgetPolicy::default();
        assert_eq!(budgets.app_inbound, defaults.app_inbound);
        assert_eq!(budgets.per_peer[0], defaults.per_peer[0]);
        assert_eq!(budgets.per_ip, defaults.per_ip);
    }

    #[test]
    fn test_counterparty_row_resolves_a_user_key_and_a_raw_hex_key() {
        let parsed = config(&format!(
            "[budgets]\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 1048576\nwindow_seconds = 86400\nmessages_per_window = 1000\n\n[[budgets.counterparty]]\npeer = \"{}\"\nbytes_per_window = 5\nwindow_seconds = 60\n",
            hex(3)
        ))
        .unwrap();
        let overrides = &parsed.budgets().counterparty_overrides;
        assert_eq!(overrides.len(), 2);
        assert_eq!(overrides[0].0, key(1));
        assert_eq!(
            overrides[0].1,
            vec![crate::abuse::BudgetLimit {
                window_ms: 86_400_000,
                bytes_per_window: 1_048_576,
                messages_per_window: 1000,
            }]
        );
        assert_eq!(overrides[1].0, key(3));
        // The resolved override is the one the evaluator consults.
        assert_eq!(
            parsed.budgets().limits_for_peer(&key(1)),
            overrides[0].1.as_slice()
        );
    }

    #[test]
    fn test_counterparty_rows_refuse_duplicates_zeroes_and_unknown_peers() {
        let duplicate = config(
            "[budgets]\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 5\nwindow_seconds = 60\n\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 6\nwindow_seconds = 60\n",
        );
        assert!(matches!(
            duplicate,
            Err(ConfigError::DuplicateCounterparty { .. })
        ));
        let zero = config("[budgets]\nper_ip_bytes_per_hour = 0\n");
        assert!(matches!(zero, Err(ConfigError::ZeroBudget { .. })));
        let unknown = config(
            "[budgets]\n[[budgets.counterparty]]\npeer = \"nobody\"\nbytes_per_window = 5\nwindow_seconds = 60\n",
        );
        assert!(matches!(unknown, Err(ConfigError::UnknownKey { .. })));
    }

    #[test]
    fn test_counterparty_window_at_the_millisecond_boundary_converts_exactly() {
        // u64::MAX / 1000 is the largest window_seconds whose millisecond
        // conversion is representable; it must convert exactly.
        let max_seconds = u64::MAX / 1_000;
        let parsed = config(&format!(
            "[budgets]\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 5\nwindow_seconds = {max_seconds}\n"
        ))
        .unwrap();
        let overrides = &parsed.budgets().counterparty_overrides;
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].1[0].window_ms, max_seconds * 1_000);
    }

    #[test]
    fn test_counterparty_window_past_the_millisecond_boundary_is_refused() {
        // The first overflowing value (and anything above it) is a config
        // error, never a silently saturated window.
        for seconds in [u64::MAX / 1_000 + 1, u64::MAX] {
            let refused = config(&format!(
                "[budgets]\n[[budgets.counterparty]]\npeer = \"ann\"\nbytes_per_window = 5\nwindow_seconds = {seconds}\n"
            ));
            assert!(
                matches!(
                    refused,
                    Err(ConfigError::WindowTooLarge {
                        seconds: given,
                        max_seconds
                    }) if given == seconds && max_seconds == u64::MAX / 1_000
                ),
                "window_seconds = {seconds} must be refused, got {refused:?}"
            );
        }
    }

    #[test]
    fn test_a_repeated_user_in_the_create_rule_is_refused_naming_it() {
        assert!(matches!(
            config(&format!("{TEAM}\n[create]\nusers = [\"ann\", \"bo\", \"ann\"]\n")),
            Err(ConfigError::DuplicateCreate { ref key, list: "[create].users" }) if key == "ann"
        ));
    }

    #[test]
    fn test_a_repeated_group_in_the_create_rule_is_refused_naming_it() {
        assert!(matches!(
            config(&format!("{TEAM}\n[create]\nusers = [\"ann\"]\ngroups = [\"team\", \"team\"]\n")),
            Err(ConfigError::DuplicateCreate { ref key, list: "[create].groups" }) if key == "team"
        ));
    }

    #[test]
    fn test_a_create_rule_without_repetition_is_unchanged() {
        let parsed = config(&format!(
            "{TEAM}\n[create]\nusers = [\"ann\", \"bo\"]\ngroups = [\"team\"]\n"
        ))
        .unwrap();
        let team = parsed.group_id("team").unwrap();
        assert_eq!(
            parsed.create_rule(),
            &CreateRule::Listed {
                users: [key(1), key(2)].into_iter().collect(),
                groups: std::iter::once(team).collect(),
                floor: Role::Pull,
            }
        );
    }

    #[test]
    fn test_a_missing_config_file_is_refused() {
        let missing = Path::new("/nonexistent/id-server-config.toml");
        assert!(matches!(
            ServerConfig::load(missing),
            Err(ConfigError::Read(_))
        ));
    }
}
