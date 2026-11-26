//! The fan-out remote list `fanout.remotes[]`, fourth part of slice
//! 9111-03 (#9111 record §7.1, §7.2, §7.3).
//!
//! Oracle: the record's rules, stated before the code. Absent means `[]`,
//! local-only, and the digest is unchanged. Each remote's `peer_id` must
//! have a `peers[]` entry and appear once. A peer that can never be asked
//! (`send_only`, `unknown`, or no `query.out` grant) is a load error.
//! `relay: true` needs a `fully_trusted` peer with `relay_granted`; any
//! other relay request is a load error, never a silent drop. `priority`
//! orders asks only, ties in list order. The effective timeout is the
//! remote's, else the peer's, else `per_peer_timeout_ms`, capped by
//! `deadline_ms`; the effective scope is the remote's, else the peer's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{ConfigDocument, ConfigError, ResolvedConfig, explain, resolved_digest};
use serde_json::{Value, json};

fn peers() -> Value {
    json!([
        {
            "peer_id": "trusted",
            "class": "fully_trusted",
            "operations": ["query.out", "relay"],
            "relay_granted": true,
            "scope": "team",
            "policy_version": 1,
            "timeout_ms": 1_200
        },
        {
            "peer_id": "signed",
            "class": "accept_signed_from",
            "operations": ["query.out"],
            "policy_version": 1
        },
        {
            "peer_id": "sender",
            "class": "send_only",
            "operations": ["sync.send"],
            "policy_version": 1
        },
        {
            "peer_id": "stranger",
            "policy_version": 1
        },
        {
            "peer_id": "quiet",
            "class": "fully_trusted",
            "operations": ["sync.send"],
            "policy_version": 1
        }
    ])
}

fn base() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-remotes",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        },
        "peers": peers()
    })
}

fn with_remotes(remotes: Value) -> Value {
    let mut doc = base();
    doc["fanout"] = json!({ "remotes": remotes });
    doc
}

fn load(value: &Value) -> Result<ResolvedConfig, ConfigError> {
    ConfigDocument::parse(&value.to_string())?.resolve()
}

fn load_err(value: &Value) -> ConfigError {
    match load(value) {
        Ok(resolved) => panic!("expected rejection, resolved {resolved:?}"),
        Err(err) => err,
    }
}

#[test]
fn absent_remotes_mean_local_only_and_keep_the_digest() {
    let plain = load(&base()).unwrap();
    assert!(plain.fanout.remotes.is_empty());
    let empty = load(&with_remotes(json!([]))).unwrap();
    assert_eq!(
        resolved_digest(&plain).unwrap(),
        resolved_digest(&empty).unwrap()
    );
    let canonical: Value = serde_json::from_str(&plain.canonical_json().unwrap()).unwrap();
    assert!(canonical["fanout"].get("remotes").is_none());
}

#[test]
fn remotes_resolve_in_priority_order_with_ties_in_list_order() {
    let resolved = load(&with_remotes(json!([
        { "peer_id": "signed", "priority": 2 },
        { "peer_id": "trusted", "priority": 1 }
    ])))
    .unwrap();
    let order: Vec<&str> = resolved
        .fanout
        .remotes
        .iter()
        .map(|r| r.peer_id.as_str())
        .collect();
    assert_eq!(order, ["trusted", "signed"]);

    let tied = load(&with_remotes(json!([
        { "peer_id": "signed", "priority": 3 },
        { "peer_id": "trusted", "priority": 3 }
    ])))
    .unwrap();
    let order: Vec<&str> = tied
        .fanout
        .remotes
        .iter()
        .map(|r| r.peer_id.as_str())
        .collect();
    assert_eq!(order, ["signed", "trusted"]);
}

#[test]
fn the_effective_timeout_follows_remote_then_peer_then_fanout_then_deadline() {
    let mut doc = with_remotes(json!([
        { "peer_id": "trusted", "priority": 1 },
        { "peer_id": "signed", "priority": 2 }
    ]));
    let resolved = load(&doc).unwrap();
    let remotes = &resolved.fanout.remotes;
    // The peer's own timeout, then the fan-out default of 2000.
    assert_eq!(remotes[0].timeout_ms, Some(1_200));
    assert_eq!(remotes[1].timeout_ms, Some(2_000));

    doc["fanout"]["remotes"][0]["timeout_ms"] = json!(800);
    doc["fanout"]["per_peer_timeout_ms"] = json!(1_500);
    let resolved = load(&doc).unwrap();
    assert_eq!(resolved.fanout.remotes[0].timeout_ms, Some(800));
    assert_eq!(resolved.fanout.remotes[1].timeout_ms, Some(1_500));

    doc["fanout"]["deadline_ms"] = json!(500);
    let resolved = load(&doc).unwrap();
    assert_eq!(resolved.fanout.remotes[0].timeout_ms, Some(500));
    assert_eq!(resolved.fanout.remotes[1].timeout_ms, Some(500));
}

#[test]
fn the_effective_scope_is_the_remotes_else_the_peers() {
    let resolved = load(&with_remotes(json!([
        { "peer_id": "trusted", "priority": 1 },
        { "peer_id": "signed", "priority": 2 }
    ])))
    .unwrap();
    assert_eq!(resolved.fanout.remotes[0].scope.as_deref(), Some("team"));
    assert_eq!(resolved.fanout.remotes[1].scope, None);

    let resolved = load(&with_remotes(json!([
        { "peer_id": "trusted", "priority": 1, "scope": "public" }
    ])))
    .unwrap();
    assert_eq!(resolved.fanout.remotes[0].scope.as_deref(), Some("public"));
}

#[test]
fn a_remote_must_name_a_configured_peer_once() {
    let err = load_err(&with_remotes(
        json!([{ "peer_id": "nobody", "priority": 1 }]),
    ));
    assert!(
        matches!(&err, ConfigError::UnknownRemotePeer(p) if p == "nobody"),
        "{err:?}"
    );
    let err = load_err(&with_remotes(json!([
        { "peer_id": "signed", "priority": 1 },
        { "peer_id": "signed", "priority": 2 }
    ])));
    assert!(
        matches!(&err, ConfigError::DuplicateRemote(p) if p == "signed"),
        "{err:?}"
    );
}

#[test]
fn a_peer_that_can_never_be_asked_is_a_load_error() {
    for peer in ["sender", "stranger", "quiet"] {
        let err = load_err(&with_remotes(json!([{ "peer_id": peer, "priority": 1 }])));
        assert!(
            matches!(&err, ConfigError::RemoteNotAskable { peer_id, .. } if peer_id == peer),
            "{peer}: {err:?}"
        );
    }
}

#[test]
fn relay_needs_a_fully_trusted_peer_with_relay_granted() {
    let resolved = load(&with_remotes(json!([
        { "peer_id": "trusted", "priority": 1, "relay": true }
    ])))
    .unwrap();
    assert!(resolved.fanout.remotes[0].relay);

    let err = load_err(&with_remotes(json!([
        { "peer_id": "signed", "priority": 1, "relay": true }
    ])));
    assert!(
        matches!(&err, ConfigError::RemoteRelayNotGranted(p) if p == "signed"),
        "{err:?}"
    );
}

#[test]
fn a_bad_timeout_or_scope_rejects() {
    for bad in [0, 30_001] {
        let err = load_err(&with_remotes(json!([
            { "peer_id": "signed", "priority": 1, "timeout_ms": bad }
        ])));
        assert!(
            matches!(
                &err,
                ConfigError::OutOfRange {
                    field: "fanout.remotes[].timeout_ms",
                    ..
                }
            ),
            "{bad}: {err:?}"
        );
    }
    let err = load_err(&with_remotes(json!([
        { "peer_id": "signed", "priority": 1, "scope": "  " }
    ])));
    assert!(
        matches!(&err, ConfigError::EmptyRemoteScope(p) if p == "signed"),
        "{err:?}"
    );
}

#[test]
fn unknown_keys_and_a_missing_priority_fail_closed() {
    for remote in [
        json!({ "peer_id": "signed", "priority": 1, "class": "fully_trusted" }),
        json!({ "peer_id": "signed" }),
    ] {
        let doc = with_remotes(json!([remote]));
        assert!(ConfigDocument::parse(&doc.to_string()).is_err(), "{remote}");
    }
}

#[test]
fn a_resolved_config_with_remotes_round_trips() {
    let resolved = load(&with_remotes(json!([
        { "peer_id": "trusted", "priority": 1, "relay": true },
        { "peer_id": "signed", "priority": 2, "scope": "public" }
    ])))
    .unwrap();
    let back: ResolvedConfig = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert_eq!(back, resolved);
}

#[test]
fn explain_lists_each_remote_with_its_sources() {
    let mut doc = with_remotes(json!([
        { "peer_id": "trusted", "priority": 1, "relay": true },
        { "peer_id": "signed", "priority": 2, "timeout_ms": 900, "scope": "public" }
    ]));
    doc["fanout"]["deadline_ms"] = json!(1_000);
    let explanation = explain(&doc.to_string()).unwrap();
    let entry = |path: &str| {
        explanation
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no entry {path}"))
    };
    let t = "fanout.remotes[\"trusted\"]";
    let s = "fanout.remotes[\"signed\"]";
    assert_eq!(entry(&format!("{t}.relay")).value, "true");
    assert_eq!(entry(&format!("{t}.relay")).source, "document");
    assert_eq!(entry(&format!("{t}.timeout_ms")).value, "1000");
    assert_eq!(
        entry(&format!("{t}.timeout_ms")).source,
        "clamped:fanout.deadline_ms"
    );
    assert_eq!(entry(&format!("{t}.scope")).value, "\"team\"");
    assert_eq!(entry(&format!("{t}.scope")).source, "peers[\"trusted\"]");
    assert_eq!(entry(&format!("{s}.relay")).source, "default");
    assert_eq!(entry(&format!("{s}.timeout_ms")).value, "900");
    assert_eq!(entry(&format!("{s}.timeout_ms")).source, "document");
    assert_eq!(entry(&format!("{s}.scope")).source, "document");
}

#[test]
fn the_schema_names_the_remote_list() {
    let schema: Value =
        serde_json::from_str(include_str!("../config/id-config-v1.schema.json")).unwrap();
    let remotes = &schema["$defs"]["fanout"]["properties"]["remotes"];
    assert_eq!(remotes["items"]["$ref"], json!("#/$defs/fanout_remote"));
    let remote = &schema["$defs"]["fanout_remote"];
    assert_eq!(remote["additionalProperties"], json!(false));
    assert_eq!(remote["required"], json!(["peer_id", "priority"]));
    assert_eq!(remote["properties"]["timeout_ms"]["maximum"], json!(30_000));
}
