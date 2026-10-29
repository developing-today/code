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
    /// A group lists one member twice.
    #[error("group `{group}` lists member `{member}` more than once")]
    DuplicateMember {
        /// The group's key.
        group: String,
        /// The member's key.
        member: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    create: Option<RawCreate>,
    policy: Option<RawPolicy>,
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
    for user in &raw.users {
        let id = parse_id(&user.key, &user.id)?;
        claim(&mut names, &user.key, Object::User(id))?;
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
    Ok(ServerConfig {
        users,
        groups,
        create,
        policy,
        names,
        aliases,
    })
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
        users.insert(user_of(names, key, "[create].users")?);
    }
    let mut groups = BTreeSet::new();
    for key in &rule.groups {
        groups.insert(group_of(names, key, "[create].groups")?);
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
    fn test_a_missing_config_file_is_refused() {
        let missing = Path::new("/nonexistent/id-server-config.toml");
        assert!(matches!(
            ServerConfig::load(missing),
            Err(ConfigError::Read(_))
        ));
    }
}
