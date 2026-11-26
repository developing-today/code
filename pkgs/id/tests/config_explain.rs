//! Explain output for slice 9111-F2 (#9111 record §6.8): the resolved
//! document with the source of each value and the digest.
//!
//! Oracle: the record's layer order (§6.2) and its example: a value the
//! document leaves out comes from its default or the mode, a value the
//! document sets says `document`, a scoped override names its scope, a
//! clamped value says what clamped it, and the digest is the one
//! activation records.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{ConfigDocument, ConfigError, explain, resolved_digest};
use serde_json::{Value, json};

fn doc() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-home-node",
        "revision": 7,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.1", "id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        },
        "behavior": { "tier": "warm" },
        "overrides": [
            {
                "scope": "peer:alpha",
                "ceilings": { "max_hops": 3 },
                "behavior": { "max_hops": 2 }
            },
            {
                "scope": "tenant:beta",
                "ceilings": { "max_hops": 4, "max_fan_out": 8 },
                "behavior": { "sync_scope": "metadata" }
            }
        ],
        "fanout": { "deadline_ms": 1_500 },
        "extensions": { "example.com/experiment": { "enabled": true } }
    })
}

fn source<'a>(explanation: &'a id_config::Explanation, path: &str) -> &'a str {
    &explanation
        .entry(path)
        .unwrap_or_else(|| panic!("explain has no entry for {path}"))
        .source
}

fn value<'a>(explanation: &'a id_config::Explanation, path: &str) -> &'a str {
    &explanation.entry(path).expect("entry").value
}

#[test]
fn every_value_names_the_layer_that_set_it() {
    let explanation = explain(&doc().to_string()).expect("valid doc explains");

    // Top level and the document's own ceilings.
    assert_eq!(source(&explanation, "revision"), "document");
    assert_eq!(
        value(&explanation, "protocol_versions"),
        r#"["id/2.0","id/2.1"]"#
    );
    assert_eq!(source(&explanation, "ceilings.max_bytes"), "document");
    assert_eq!(source(&explanation, "ceilings.max_time_ms"), "document");

    // A ceiling names the scope that narrowed it last in scope order:
    // peer:alpha takes max_hops to 3; tenant:beta's 4 does not narrow it.
    assert_eq!(value(&explanation, "ceilings.max_hops"), "3");
    assert_eq!(
        source(&explanation, "ceilings.max_hops"),
        "override:peer:alpha"
    );
    assert_eq!(value(&explanation, "ceilings.max_fan_out"), "8");
    assert_eq!(
        source(&explanation, "ceilings.max_fan_out"),
        "override:tenant:beta"
    );

    // Gates are compiled in.
    for gate in [
        "gates.signature_verification",
        "gates.validation",
        "gates.replay_protection",
        "gates.authorization",
        "gates.tenant_scope",
    ] {
        assert_eq!(value(&explanation, gate), "true");
        assert_eq!(source(&explanation, gate), "fixed");
    }

    // Behavior follows the resolver's provenance.
    assert_eq!(value(&explanation, "behavior.max_hops"), "2");
    assert_eq!(
        source(&explanation, "behavior.max_hops"),
        "override:peer:alpha"
    );
    assert_eq!(value(&explanation, "behavior.sync_scope"), r#""metadata""#);
    assert_eq!(
        source(&explanation, "behavior.sync_scope"),
        "override:tenant:beta"
    );
    assert_eq!(value(&explanation, "behavior.tier"), r#""warm""#);
    assert_eq!(source(&explanation, "behavior.tier"), "document");

    // Fan-out: the deadline was set, the breadth is the default, and the
    // default per-peer timeout of 2000 was clamped to the 1500 deadline.
    assert_eq!(source(&explanation, "fanout.deadline_ms"), "document");
    assert_eq!(value(&explanation, "fanout.peer_breadth"), "4");
    assert_eq!(source(&explanation, "fanout.peer_breadth"), "default");
    assert_eq!(value(&explanation, "fanout.per_peer_timeout_ms"), "1500");
    assert_eq!(
        source(&explanation, "fanout.per_peer_timeout_ms"),
        "clamped:fanout.deadline_ms"
    );

    // Extensions are retained and say they are not executed.
    assert_eq!(
        source(&explanation, "extensions.example.com/experiment"),
        "document (retained, not executed)"
    );
}

#[test]
fn mode_defaults_and_untouched_fanout_keys_say_so() {
    let mut doc = doc();
    let object = doc.as_object_mut().unwrap();
    object.remove("overrides");
    object.remove("behavior");
    object.insert("fanout".to_owned(), json!({ "per_peer_timeout_ms": 900 }));
    let explanation = explain(&doc.to_string()).unwrap();
    for field in ["behavior.max_hops", "behavior.sync_scope", "behavior.tier"] {
        assert_eq!(
            source(&explanation, field),
            "mode:federated-relay",
            "{field}"
        );
    }
    assert_eq!(source(&explanation, "ceilings.max_hops"), "document");
    assert_eq!(source(&explanation, "fanout.deadline_ms"), "default");
    assert_eq!(value(&explanation, "fanout.per_peer_timeout_ms"), "900");
    assert_eq!(
        source(&explanation, "fanout.per_peer_timeout_ms"),
        "document"
    );
}

#[test]
fn digest_is_the_activation_digest_and_text_is_deterministic() {
    let text = doc().to_string();
    let explanation = explain(&text).unwrap();
    let resolved = ConfigDocument::parse(&text).unwrap().resolve().unwrap();
    assert_eq!(explanation.digest, resolved_digest(&resolved).unwrap());
    assert_eq!(explanation.digest.len(), 64);

    let rendered = explanation.to_string();
    assert_eq!(rendered, explain(&text).unwrap().to_string());
    assert!(
        rendered
            .lines()
            .any(|line| line == "ceilings.max_hops = 3  # override:peer:alpha")
    );
    assert_eq!(
        rendered.lines().last().unwrap(),
        format!("digest = sha256:{}", explanation.digest)
    );
    // One line per entry, then the digest.
    assert_eq!(rendered.lines().count(), explanation.entries.len() + 1);
    let paths: Vec<&str> = explanation
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    let mut unique = paths.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), paths.len(), "each path is explained once");
}

#[test]
fn a_document_that_would_not_load_is_not_explained() {
    let mut doc = doc();
    doc["fanout"] = json!({ "peer_breadth": 17 });
    assert!(matches!(
        explain(&doc.to_string()),
        Err(ConfigError::OutOfRange {
            field: "fanout.peer_breadth",
            ..
        })
    ));
    assert!(matches!(explain("{"), Err(ConfigError::Parse(_))));
}
