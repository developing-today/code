// Probe: does Petnames::open replay records that Petnames::set refuses?
use id_core::petname::Petnames;

#[test]
#[allow(clippy::unwrap_used)]
fn probe_open_replays_names_set_would_refuse() {
    let dir = std::env::temp_dir().join(format!("petname-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("petnames.jsonl");
    // Two contacts with fold-equal names (set() refuses the second as Conflict),
    // plus a name with a format char (U+200B) and an overlong name (>64 chars).
    let long = "x".repeat(80);
    let journal = format!(
        "{{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-a\",\"name\":\"PayPal\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-b\",\"name\":\"paypal\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-c\",\"name\":\"Evil\u{200b}Name\"}}\n\
         {{\"op\":\"set\",\"owner\":\"me\",\"account\":\"acct-d\",\"name\":\"{long}\"}}\n"
    );
    std::fs::write(&path, journal).unwrap();
    let store = Petnames::open(&path).unwrap();
    let loaded: Vec<(String, String)> = store
        .of_owner("me")
        .map(|(a, n)| (a.to_string(), n.to_string()))
        .collect();
    println!("LOADED: {loaded:?}");
    assert_eq!(
        loaded.len(),
        4,
        "all four refused-at-write records loaded verbatim"
    );
    assert!(store.get("me", "acct-b") == Some("paypal"));
    assert!(store.get("me", "acct-c").unwrap().contains('\u{200b}'));
}
