#![allow(clippy::unwrap_used)]
use id_core::discovery::{DEFAULT_TOPIC, DEFAULT_TOPIC_SECRET, resolve_config};

#[test]
fn probe_no_default_topic_with_only_cli_topic() {
    let cfg = resolve_config(&[], Some("my-private-net"), None, false, false, true);
    println!("topic={:?}", cfg.topic);
    println!("secret_is_public_default={}", cfg.topic_secret == DEFAULT_TOPIC_SECRET);
    // The pairing claim: skipping defaults while naming a private topic
    // should not silently pair it with the public network secret.
    assert_eq!(cfg.topic, "my-private-net");
    assert_ne!(cfg.topic_secret, DEFAULT_TOPIC_SECRET, "private topic paired with the PUBLIC default secret");
}

#[test]
fn probe_no_default_topic_with_only_cli_secret() {
    let cfg = resolve_config(&[], None, Some("my-private-secret"), false, false, true);
    println!("topic={:?} secret_is_cli={}", cfg.topic, cfg.topic_secret == b"my-private-secret");
    assert_eq!(cfg.topic_secret, b"my-private-secret".to_vec());
    assert_ne!(cfg.topic, DEFAULT_TOPIC, "private secret paired with the PUBLIC default topic");
}
