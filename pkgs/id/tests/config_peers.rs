//! The peer plane `peers[]`, first part of slice 9111-03 (#9111 record
//! §3.2, §6.3, §6.4, §6.5 step 3; #9141 §2 to §4).
//!
//! Oracle: the record's rules, stated before the code. One entry per
//! peer; unknown keys in a peer entry fail closed; `relay_granted` only
//! with `fully_trusted`; `send_only` has no `sync.accept`;
//! `accept_signed_from` has no relay and no `sync.send`; `unknown` has no
//! operations; a policy window must be non-empty; the per-peer timeout is
//! inside the fan-out limit. Resolution sorts peers and operations, so
//! authoring order never changes the digest, and a document with no peers
//! keeps the digest it had before the plane existed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    ConfigDocument, ConfigError, PER_PEER_TIMEOUT_MS_LIMIT, PeerClass, PeerOperation, explain,
    resolved_digest,
};
use serde_json::{Value, json};

fn doc(peers: Value) -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-peers",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        },
        "peers": peers
    })
}

fn load(value: &Value) -> Result<id_config::ResolvedConfig, ConfigError> {
    ConfigDocument::parse(&value.to_string())?.resolve()
}

fn load_err(value: &Value) -> ConfigError {
    match load(value) {
        Ok(resolved) => panic!("expected rejection, resolved {resolved:?}"),
        Err(err) => err,
    }
}

fn peer(id: &str, class: &str, operations: &[&str]) -> Value {
    json!({
        "peer_id": id,
        "class": class,
        "operations": operations,
        "policy_version": 1
    })
}

#[test]
fn a_minimal_entry_takes_every_default() {
    let resolved = load(&doc(json!([{ "peer_id": "p1", "policy_version": 3 }]))).unwrap();
    let entry = &resolved.peers[0];
    assert_eq!(entry.peer_id, "p1");
    assert_eq!(entry.class, PeerClass::Unknown);
    assert!(entry.operations.is_empty());
    assert!(!entry.relay_granted);
    assert_eq!(entry.scope, None);
    assert_eq!(entry.policy_version, 3);
    assert_eq!(
        (entry.valid_from.as_deref(), entry.expires_at.as_deref()),
        (None, None)
    );
    assert_eq!((entry.timeout_ms, entry.priority), (None, None));
}

#[test]
fn every_class_accepts_the_operations_it_can_use() {
    let peers = json!([
        {
            "peer_id": "trusted",
            "class": "fully_trusted",
            "operations": ["relay", "sync.accept", "sync.send", "query.out", "query.answer"],
            "relay_granted": true,
            "scope": "tenant:home",
            "policy_version": 9,
            "valid_from": "2026-01-01T00:00:00Z",
            "expires_at": "2027-01-01T00:00:00+02:00",
            "timeout_ms": PER_PEER_TIMEOUT_MS_LIMIT,
            "priority": 0
        },
        peer("outbox", "send_only", &["sync.send", "query.out", "query.answer"]),
        peer("inbox", "accept_signed_from", &["sync.accept", "query.out"]),
        peer("stranger", "unknown", &[])
    ]);
    let resolved = load(&doc(peers)).unwrap();
    let ids: Vec<&str> = resolved.peers.iter().map(|p| p.peer_id.as_str()).collect();
    assert_eq!(ids, ["inbox", "outbox", "stranger", "trusted"]);
    let trusted = &resolved.peers[3];
    assert_eq!(
        trusted.operations,
        [
            PeerOperation::SyncAccept,
            PeerOperation::SyncSend,
            PeerOperation::QueryOut,
            PeerOperation::QueryAnswer,
            PeerOperation::Relay,
        ],
        "operations resolve in canonical order"
    );
}

#[test]
fn class_conflicts_fail_closed() {
    let cases = [
        (
            json!({ "peer_id": "x", "class": "send_only", "relay_granted": true, "policy_version": 1 }),
            "relay_granted is only allowed with class fully_trusted",
        ),
        (
            json!({ "peer_id": "x", "class": "accept_signed_from", "relay_granted": true, "policy_version": 1 }),
            "relay_granted is only allowed with class fully_trusted",
        ),
        (
            peer("x", "fully_trusted", &["relay"]),
            "the relay operation needs relay_granted",
        ),
        (
            peer("x", "accept_signed_from", &["relay"]),
            "the relay operation needs relay_granted",
        ),
        (
            peer("x", "send_only", &["sync.accept"]),
            "class send_only never accepts inbound data (sync.accept)",
        ),
        (
            peer("x", "accept_signed_from", &["sync.send"]),
            "class accept_signed_from grants no outbound sends (sync.send)",
        ),
        (
            peer("x", "unknown", &["query.out"]),
            "class unknown grants no operations",
        ),
    ];
    for (entry, expected) in cases {
        match load_err(&doc(json!([entry]))) {
            ConfigError::PeerClassConflict { peer_id, rule, .. } => {
                assert_eq!(peer_id, "x");
                assert_eq!(rule, expected);
            }
            other => panic!("expected a class conflict ({expected}), got {other:?}"),
        }
    }
}

#[test]
fn identity_and_shape_errors_fail_closed() {
    assert!(matches!(
        load_err(&doc(json!([peer("", "unknown", &[])]))),
        ConfigError::EmptyPeerId
    ));
    assert!(matches!(
        load_err(&doc(json!([peer("a", "unknown", &[]), peer("a", "fully_trusted", &[])]))),
        ConfigError::DuplicatePeer(id) if id == "a"
    ));
    assert!(matches!(
        load_err(&doc(json!([peer(
            "a",
            "fully_trusted",
            &["query.out", "query.out"]
        )]))),
        ConfigError::DuplicatePeerOperation {
            operation: "query.out",
            ..
        }
    ));
    assert!(matches!(
        load_err(&doc(json!([{ "peer_id": "a", "scope": " ", "policy_version": 1 }]))),
        ConfigError::EmptyPeerScope(id) if id == "a"
    ));
    // Required, typed and closed: each of these is a parse error.
    for entry in [
        json!({ "peer_id": "a" }),
        json!({ "policy_version": 1 }),
        json!({ "peer_id": "a", "policy_version": 1, "class": "trusted" }),
        json!({ "peer_id": "a", "policy_version": 1, "operations": ["sync.write"] }),
        json!({ "peer_id": "a", "policy_version": -1 }),
        json!({ "peer_id": "a", "policy_version": 1, "overrides": {} }),
        json!({ "peer_id": "a", "policy_version": 1, "trust_everything": true }),
    ] {
        assert!(
            matches!(
                load_err(&doc(json!([entry.clone()]))),
                ConfigError::Parse(_)
            ),
            "{entry} must not parse"
        );
    }
}

#[test]
fn policy_window_and_timeout_are_checked() {
    let windowed = |from: &str, until: &str| json!([{ "peer_id": "w", "policy_version": 1, "valid_from": from, "expires_at": until }]);
    assert!(
        load(&doc(windowed(
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:01Z"
        )))
        .is_ok()
    );
    for (from, until) in [
        ("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
        // 01:00+01:00 is the same instant as 00:00Z.
        ("2026-01-01T00:00:00Z", "2026-01-01T01:00:00+01:00"),
        ("2026-06-01T00:00:00Z", "2026-01-01T00:00:00Z"),
    ] {
        assert!(
            matches!(load_err(&doc(windowed(from, until))), ConfigError::EmptyPeerWindow(id) if id == "w"),
            "{from} .. {until}"
        );
    }
    for bad in ["2026-02-30T00:00:00Z", "tomorrow", "2026-01-01T00:00:00"] {
        let peers = json!([{ "peer_id": "w", "policy_version": 1, "expires_at": bad }]);
        assert!(
            matches!(
                load_err(&doc(peers)),
                ConfigError::InvalidPeerInstant {
                    field: "expires_at",
                    ..
                }
            ),
            "{bad}"
        );
    }
    for timeout in [0, PER_PEER_TIMEOUT_MS_LIMIT + 1] {
        let peers = json!([{ "peer_id": "t", "policy_version": 1, "timeout_ms": timeout }]);
        assert!(matches!(
            load_err(&doc(peers)),
            ConfigError::OutOfRange { field: "peers[].timeout_ms", value, .. } if value == timeout
        ));
    }
}

#[test]
fn authoring_order_never_changes_the_digest() {
    let a = doc(json!([
        peer("b", "fully_trusted", &["sync.send", "sync.accept"]),
        peer("a", "send_only", &["query.out", "sync.send"])
    ]));
    let b = doc(json!([
        peer("a", "send_only", &["sync.send", "query.out"]),
        peer("b", "fully_trusted", &["sync.accept", "sync.send"])
    ]));
    let (ra, rb) = (load(&a).unwrap(), load(&b).unwrap());
    assert_eq!(ra, rb);
    assert_eq!(resolved_digest(&ra).unwrap(), resolved_digest(&rb).unwrap());
    let changed = doc(json!([
        peer("a", "send_only", &["sync.send"]),
        peer("b", "fully_trusted", &["sync.accept", "sync.send"])
    ]));
    assert_ne!(
        resolved_digest(&ra).unwrap(),
        resolved_digest(&load(&changed).unwrap()).unwrap(),
        "a changed grant changes the digest"
    );
}

#[test]
fn no_peers_keeps_the_canonical_encoding() {
    let mut without = doc(json!([]));
    without.as_object_mut().unwrap().remove("peers");
    let empty = load(&doc(json!([]))).unwrap();
    let absent = load(&without).unwrap();
    assert_eq!(empty, absent);
    let canonical = absent.canonical_json().unwrap();
    assert!(!canonical.contains("\"peers\""), "{canonical}");
}

#[test]
fn explain_lists_each_peer_field_with_its_source() {
    let peers = json!([
        { "peer_id": "alpha", "class": "accept_signed_from", "operations": ["sync.accept"], "policy_version": 2 }
    ]);
    let explanation = explain(&doc(peers).to_string()).unwrap();
    let line = |path: &str| {
        let entry = explanation
            .entry(path)
            .unwrap_or_else(|| panic!("explain has no entry for {path}"));
        (entry.value.as_str(), entry.source.as_str())
    };
    assert_eq!(
        line("peers[\"alpha\"].class"),
        ("\"accept_signed_from\"", "document")
    );
    assert_eq!(
        line("peers[\"alpha\"].operations"),
        ("[\"sync.accept\"]", "document")
    );
    assert_eq!(line("peers[\"alpha\"].relay_granted"), ("false", "default"));
    assert_eq!(line("peers[\"alpha\"].scope"), ("null", "default"));
    assert_eq!(line("peers[\"alpha\"].policy_version"), ("2", "document"));
}

#[test]
fn schema_declares_the_peer_plane() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("config")
        .join("id-config-v1.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let peer = &schema["$defs"]["peer"];
    assert_eq!(peer["additionalProperties"], json!(false));
    assert_eq!(peer["required"], json!(["peer_id", "policy_version"]));
    let classes: Vec<PeerClass> =
        serde_json::from_value(peer["properties"]["class"]["enum"].clone()).unwrap();
    assert_eq!(
        classes,
        [
            PeerClass::FullyTrusted,
            PeerClass::SendOnly,
            PeerClass::AcceptSignedFrom,
            PeerClass::Unknown
        ]
    );
    let operations: Vec<PeerOperation> =
        serde_json::from_value(peer["properties"]["operations"]["items"]["enum"].clone()).unwrap();
    assert_eq!(operations.len(), 5);
    assert_eq!(
        peer["properties"]["timeout_ms"]["maximum"],
        json!(PER_PEER_TIMEOUT_MS_LIMIT)
    );
    assert_eq!(
        schema["properties"]["peers"]["items"]["$ref"],
        json!("#/$defs/peer")
    );
}
