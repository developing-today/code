#![allow(clippy::unwrap_used, clippy::expect_used)]
use ed25519_dalek::SigningKey;
use id::server_config::{ConfigError, ServerConfig};

fn hex(n: u8) -> String {
    let key = SigningKey::from_bytes(&[n; 32]).verifying_key().to_bytes();
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn users() -> String {
    format!(
        "[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n[[users]]\nkey = \"bo\"\nid = \"{}\"\n",
        hex(1),
        hex(2)
    )
}

const TEAM: &str = "\n[[groups]]\nkey = \"team\"\nname = \"Team\"\ncreator = \"ann\"\n\n[[groups.members]]\nuser = \"bo\"\nrole = \"write\"\n";

#[test]
fn probe_create_users_duplicate() {
    let text = format!("{}{}\n[create]\nusers = [\"ann\", \"ann\"]\n", users(), TEAM);
    let result = ServerConfig::parse(&text);
    println!("CREATE-USERS-DUP: {result:?}");
    // Current behavior under probe: parses Ok, duplicate silently deduped by BTreeSet.
    assert!(result.is_ok(), "probe expects current silent-dedup behavior");
}

#[test]
fn probe_create_groups_duplicate() {
    let text = format!("{}{}\n[create]\nusers = [\"ann\"]\ngroups = [\"team\", \"team\"]\n", users(), TEAM);
    let result = ServerConfig::parse(&text);
    println!("CREATE-GROUPS-DUP: {result:?}");
    assert!(result.is_ok(), "probe expects current silent-dedup behavior");
}

#[test]
fn probe_member_duplicate_control() {
    let text = format!("{}{}\n[[groups.members]]\nuser = \"bo\"\nrole = \"read\"\n", users(), TEAM);
    let result = ServerConfig::parse(&text);
    println!("MEMBER-DUP-CONTROL: {result:?}");
    assert!(matches!(result, Err(ConfigError::DuplicateMember { .. })));
}

#[test]
fn probe_create_same_key_twice_via_both_lists_is_kind_checked() {
    // sanity: a group key in users list is WrongKind (parser is strict elsewhere)
    let text = format!("{}{}\n[create]\nusers = [\"team\"]\n", users(), TEAM);
    let result = ServerConfig::parse(&text);
    println!("CREATE-WRONGKIND: {result:?}");
    assert!(matches!(result, Err(ConfigError::WrongKind { .. })));
}
