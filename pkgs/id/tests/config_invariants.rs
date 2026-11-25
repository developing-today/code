//! Invariant tests for slice 9111-01 (#10178): the versioned configuration
//! schema and its hard invariants.
//!
//! Oracle (from the slice manifest): **invariant** — "Canonical V2 config
//! rejects any attempt to disable fixed gates and resolves
//! deterministically." Verifier: an independent schema/reference validator
//! with a generated invalid-config matrix.
//!
//! How the independence works here: this file does not trust the
//! `id-config` crate's own constants to define the contract. It loads the
//! JSON Schema at `pkgs/id/config/id-config-v1.schema.json` directly and
//! *generates* cases from it — one gate-disable attempt per gate the schema
//! declares, one missing-field case per field the schema requires — then
//! checks the crate rejects every one. The hand-written matrix below adds
//! the semantic invariants a JSON Schema cannot state (ceiling widening,
//! behavior beyond a ceiling, duplicate scopes, determinism of the
//! resolver).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use id_config::{
    CONFIG_SCHEMA_VERSION, ConfigDocument, ConfigError, DEADLINE_MS_LIMIT, FIXED_GATE_IDS,
    Fanout, FieldClass, MAX_HOPS_LIMIT, Mode, PEER_BREADTH_LIMIT, PER_PEER_TIMEOUT_MS_LIMIT,
    ResolvedConfig, SyncScope, Tier, field_class,
};
use serde_json::{Value, json};

fn schema() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("config")
        .join("id-config-v1.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("schema file {} must exist: {err}", path.display()));
    serde_json::from_str(&text).expect("schema file must be valid JSON")
}

/// A known-valid document, in the canonical authoring shape.
fn valid_doc() -> Value {
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
                "behavior": { "sync_scope": "metadata" }
            }
        ],
        "extensions": { "example.com/experiment": { "enabled": true } }
    })
}

fn load(doc: &Value) -> Result<ResolvedConfig, ConfigError> {
    ConfigDocument::parse(&doc.to_string())?.resolve()
}

fn load_err(doc: &Value) -> ConfigError {
    match load(doc) {
        Ok(resolved) => panic!("document must be rejected, but resolved to {resolved:?}"),
        Err(err) => err,
    }
}

#[test]
fn schema_and_rust_contract_agree() {
    let schema = schema();
    // The schema is fail-closed at the top level and pins version 1, which
    // must be the version the crate accepts.
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["properties"]["schema_version"]["const"], json!(1));
    assert_eq!(u64::from(CONFIG_SCHEMA_VERSION), 1);
    // Every fixed gate the crate names must be a const-true property in the
    // schema's gate object, and vice versa.
    let schema_gates = schema["$defs"]["fixed_gates"]["properties"]
        .as_object()
        .expect("schema must declare fixed gates");
    assert_eq!(schema_gates.len(), FIXED_GATE_IDS.len());
    for gate in FIXED_GATE_IDS {
        let field = gate.replace('-', "_");
        assert_eq!(
            schema_gates[&field]["const"],
            json!(true),
            "schema must pin gate {gate} to const true"
        );
    }
}

#[test]
fn oracle_rejects_any_attempt_to_disable_fixed_gates() {
    // Generated matrix: one disable attempt per gate the *schema* declares.
    let schema = schema();
    let gate_fields: Vec<String> = schema["$defs"]["fixed_gates"]["properties"]
        .as_object()
        .expect("schema gates")
        .keys()
        .cloned()
        .collect();
    assert!(!gate_fields.is_empty());
    for field in gate_fields {
        let mut doc = valid_doc();
        doc["gates"] = json!({ field.clone(): false });
        let err = load_err(&doc);
        assert!(
            matches!(err, ConfigError::FixedGateDisabled(_)),
            "disabling {field} must be a FixedGateDisabled rejection, got {err:?}"
        );
    }
    // And the untouched document — gates omitted entirely — resolves with
    // every gate enabled: omission is not a disable path.
    let resolved = load(&valid_doc()).expect("valid document resolves");
    assert!(resolved.gates.signature_verification);
    assert!(resolved.gates.validation);
    assert!(resolved.gates.replay_protection);
    assert!(resolved.gates.authorization);
    assert!(resolved.gates.tenant_scope);
}

#[test]
fn oracle_generated_missing_and_unknown_field_matrix() {
    // Generated matrix: removing any field the schema requires rejects.
    let schema = schema();
    let required: Vec<String> = schema["required"]
        .as_array()
        .expect("schema required list")
        .iter()
        .map(|field| field.as_str().expect("field name").to_owned())
        .collect();
    assert!(required.len() >= 6);
    for field in required {
        let mut doc = valid_doc();
        doc.as_object_mut().expect("doc object").remove(&field);
        load_err(&doc);
    }
    // Unknown fields reject at every level of the document (fail-closed
    // unknown-field policy), not just the top.
    for (pointer, key) in [
        ("", "surprise"),
        ("ceilings", "max_worlds"),
        ("behavior", "gate_bypass"),
        ("gates", "extra_gate"),
    ] {
        let mut doc = valid_doc();
        if !pointer.is_empty() && !doc[pointer].is_object() {
            doc[pointer] = json!({});
        }
        let target = if pointer.is_empty() {
            doc.as_object_mut().expect("doc object")
        } else {
            doc[pointer].as_object_mut().expect("nested object")
        };
        target.insert(key.to_owned(), json!(true));
        load_err(&doc);
    }
}

#[test]
fn oracle_invalid_config_matrix() {
    let cases: Vec<(&str, Value)> = vec![
        ("unsupported schema version", {
            let mut doc = valid_doc();
            doc["schema_version"] = json!(2);
            doc
        }),
        ("empty config id", {
            let mut doc = valid_doc();
            doc["config_id"] = json!("  ");
            doc
        }),
        ("empty protocol allowlist", {
            let mut doc = valid_doc();
            doc["protocol_versions"] = json!([]);
            doc
        }),
        ("duplicate protocol version", {
            let mut doc = valid_doc();
            doc["protocol_versions"] = json!(["id/2.0", "id/2.0"]);
            doc
        }),
        ("zero ceiling", {
            let mut doc = valid_doc();
            doc["ceilings"]["max_hops"] = json!(0);
            doc
        }),
        ("document behavior beyond its own ceiling", {
            let mut doc = valid_doc();
            doc["behavior"] = json!({ "max_hops": 6 });
            doc
        }),
        ("scope widens a hard ceiling", {
            let mut doc = valid_doc();
            doc["overrides"] = json!([
                { "scope": "peer:alpha", "ceilings": { "max_hops": 9 } }
            ]);
            doc
        }),
        ("scope behavior beyond its narrowed ceiling", {
            let mut doc = valid_doc();
            doc["overrides"] = json!([
                {
                    "scope": "peer:alpha",
                    "ceilings": { "max_hops": 2 },
                    "behavior": { "max_hops": 3 }
                }
            ]);
            doc
        }),
        ("duplicate scope", {
            let mut doc = valid_doc();
            doc["overrides"] = json!([
                { "scope": "peer:alpha" },
                { "scope": "peer:alpha" }
            ]);
            doc
        }),
        ("empty scope", {
            let mut doc = valid_doc();
            doc["overrides"] = json!([{ "scope": "" }]);
            doc
        }),
        ("unnamespaced extension key", {
            let mut doc = valid_doc();
            doc["extensions"] = json!({ "experiment": { "enabled": true } });
            doc
        }),
        ("unknown mode", {
            let mut doc = valid_doc();
            doc["mode"] = json!("mesh-everything");
            doc
        }),
    ];
    for (name, doc) in cases {
        match load(&doc) {
            Ok(resolved) => panic!("case {name:?} must be rejected, resolved to {resolved:?}"),
            Err(_) => {}
        }
    }
}

#[test]
fn extension_namespace_keys_match_the_schema_rule() {
    // Regression for #12881: the Rust validator must reject every key the
    // canonical schema pattern ^[^./\s]+([./][^./\s]+)+$ rejects, not only
    // keys with no separator at all.
    let malformed = [
        ".",          // separator only
        "/",          // separator only
        "a..b",       // empty segment
        "a//b",       // empty segment
        "a./b",       // empty segment across mixed separators
        ".a.b",       // leading separator
        "a.b.",       // trailing separator
        "a b.c",      // whitespace inside a segment
        "a/b c",      // whitespace inside a later segment
        "a\tb.c",     // tab whitespace
        "a.b ",       // trailing whitespace
        " example.c", // leading whitespace
    ];
    for key in malformed {
        let mut doc = valid_doc();
        doc["extensions"] = json!({ key: { "enabled": true } });
        match load_err(&doc) {
            ConfigError::UnnamespacedExtension(rejected) => {
                assert_eq!(rejected, key, "error must name the offending key");
            }
            other => panic!("key {key:?} must reject as unnamespaced, got {other:?}"),
        }
    }

    // Valid namespaces still resolve and are retained verbatim.
    for key in ["example.com/experiment", "a.b", "a/b", "a.b/c.d"] {
        let mut doc = valid_doc();
        doc["extensions"] = json!({ key: { "enabled": true } });
        let resolved =
            load(&doc).unwrap_or_else(|err| panic!("key {key:?} must resolve, got {err:?}"));
        assert_eq!(
            resolved.extensions[key],
            json!({ "enabled": true }),
            "valid extension {key:?} must be retained verbatim"
        );
    }
}

#[test]
fn oracle_resolution_is_deterministic() {
    // Same document, authored with overrides and allowlist entries in the
    // opposite order: resolution must not notice.
    let mut shuffled = valid_doc();
    shuffled["protocol_versions"] = json!(["id/2.0", "id/2.1"]);
    let overrides = shuffled["overrides"].as_array().expect("overrides").clone();
    shuffled["overrides"] = Value::Array(overrides.into_iter().rev().collect());

    let a = load(&valid_doc())
        .expect("valid")
        .canonical_json()
        .expect("json");
    let b = load(&shuffled)
        .expect("valid")
        .canonical_json()
        .expect("json");
    assert_eq!(a, b, "resolution must be authoring-order independent");

    // Resolving the same document twice is byte-identical.
    let again = load(&valid_doc())
        .expect("valid")
        .canonical_json()
        .expect("json");
    assert_eq!(a, again);
}

#[test]
fn resolution_precedence_and_ceilings() {
    let resolved = load(&valid_doc()).expect("valid document resolves");
    // Mode defaults (federated-relay: hops 5, sync full, tier hot) are
    // replaced by the document (tier warm), then by scoped overrides in
    // scope-name order: peer:alpha sets max_hops 2, tenant:beta sets sync
    // metadata.
    assert_eq!(resolved.behavior.max_hops, 2);
    assert_eq!(resolved.behavior.sync_scope, SyncScope::Metadata);
    assert_eq!(resolved.behavior.tier, Tier::Warm);
    // Ceilings only ever narrow: peer:alpha narrowed max_hops 5 -> 3.
    assert_eq!(resolved.ceilings.max_hops, 3);
    assert_eq!(resolved.ceilings.max_fan_out, 16);
    // Provenance is recorded per field, separately from the values.
    let expected: BTreeMap<String, String> = [
        ("max_hops".to_owned(), "override:peer:alpha".to_owned()),
        ("sync_scope".to_owned(), "override:tenant:beta".to_owned()),
        ("tier".to_owned(), "document".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(resolved.provenance, expected);
    // Namespaced extensions are retained verbatim, and the allowlist is in
    // canonical (sorted) order.
    assert_eq!(
        resolved.extensions["example.com/experiment"],
        json!({ "enabled": true })
    );
    assert_eq!(resolved.protocol_versions, ["id/2.0", "id/2.1"]);
    assert_eq!(resolved.mode, Mode::FederatedRelay);
}

#[test]
fn mechanism_behavior_classification_is_stable() {
    // The classification is contract: gates and ceilings are mechanism and
    // must never silently become behavior.
    for path in [
        "schema_version",
        "protocol_versions",
        "ceilings.max_hops",
        "gates.signature_verification",
    ] {
        assert_eq!(field_class(path), Some(FieldClass::Mechanism), "{path}");
    }
    for path in ["mode", "behavior.tier", "overrides", "extensions", "fanout"] {
        assert_eq!(field_class(path), Some(FieldClass::Behavior), "{path}");
    }
    assert_eq!(field_class("no_such_field"), None);
}

/// Slice 9111-F1: the fan-out keys of #9111 record section 7.1.
#[test]
fn fanout_defaults_apply_when_absent() {
    let resolved = load(&valid_doc()).expect("valid doc resolves");
    assert_eq!(resolved.fanout, Fanout::default());
    assert_eq!(resolved.fanout.deadline_ms, 5_000);
    assert_eq!(resolved.fanout.per_peer_timeout_ms, 2_000);
    assert_eq!(resolved.fanout.peer_breadth, 4);
    // A present but empty section also means every default.
    let mut doc = valid_doc();
    doc["fanout"] = json!({});
    assert_eq!(load(&doc).expect("empty fanout").fanout, Fanout::default());
}

#[test]
fn fanout_limits_are_inclusive_and_fail_closed() {
    let cases = [
        ("deadline_ms", DEADLINE_MS_LIMIT),
        ("per_peer_timeout_ms", PER_PEER_TIMEOUT_MS_LIMIT),
        ("peer_breadth", u64::from(PEER_BREADTH_LIMIT)),
    ];
    for (key, limit) in cases {
        let field = format!("fanout.{key}");
        // The limit itself is accepted.
        let mut doc = valid_doc();
        doc["fanout"] = json!({ "deadline_ms": DEADLINE_MS_LIMIT, key: limit });
        load(&doc).unwrap_or_else(|err| panic!("{field} = {limit} must load: {err}"));
        // One past the limit, and zero, are rejected, never clamped.
        for bad in [limit + 1, 0] {
            let mut doc = valid_doc();
            doc["fanout"] = json!({ key: bad });
            match load_err(&doc) {
                ConfigError::OutOfRange {
                    field: got,
                    value,
                    limit: got_limit,
                } => {
                    assert_eq!(got, field);
                    assert_eq!(value, bad);
                    assert_eq!(got_limit, limit);
                }
                other => panic!("{field} = {bad} gave {other:?}"),
            }
        }
    }
    // An unknown fan-out key rejects (fail-closed unknown-field policy).
    let mut doc = valid_doc();
    doc["fanout"] = json!({ "remote_breadth": 2 });
    load_err(&doc);
}

#[test]
fn hop_ceiling_is_bounded() {
    let mut doc = valid_doc();
    doc["ceilings"]["max_hops"] = json!(MAX_HOPS_LIMIT);
    load(&doc).expect("max_hops at the limit loads");
    doc["ceilings"]["max_hops"] = json!(MAX_HOPS_LIMIT + 1);
    match load_err(&doc) {
        ConfigError::OutOfRange { field, value, limit } => {
            assert_eq!(field, "ceilings.max_hops");
            assert_eq!(value, u64::from(MAX_HOPS_LIMIT) + 1);
            assert_eq!(limit, u64::from(MAX_HOPS_LIMIT));
        }
        other => panic!("max_hops 9 gave {other:?}"),
    }
}

#[test]
fn per_peer_timeout_clamps_to_deadline() {
    let mut doc = valid_doc();
    doc["fanout"] = json!({ "deadline_ms": 1_500, "per_peer_timeout_ms": 2_500 });
    let resolved = load(&doc).expect("timeout above deadline is accepted");
    assert_eq!(resolved.fanout.deadline_ms, 1_500);
    assert_eq!(resolved.fanout.per_peer_timeout_ms, 1_500);
    // At or under the deadline it is kept as written.
    doc["fanout"] = json!({ "deadline_ms": 1_500, "per_peer_timeout_ms": 1_500 });
    assert_eq!(load(&doc).expect("equal").fanout.per_peer_timeout_ms, 1_500);
}

#[test]
fn schema_declares_the_fanout_limits() {
    let schema = schema();
    assert_eq!(
        schema["$defs"]["ceilings"]["properties"]["max_hops"]["maximum"],
        json!(MAX_HOPS_LIMIT)
    );
    let fanout = &schema["$defs"]["fanout"];
    assert_eq!(fanout["additionalProperties"], json!(false));
    let defaults = Fanout::default();
    for (key, limit, default) in [
        ("deadline_ms", DEADLINE_MS_LIMIT, defaults.deadline_ms),
        (
            "per_peer_timeout_ms",
            PER_PEER_TIMEOUT_MS_LIMIT,
            defaults.per_peer_timeout_ms,
        ),
        (
            "peer_breadth",
            u64::from(PEER_BREADTH_LIMIT),
            u64::from(defaults.peer_breadth),
        ),
    ] {
        let prop = &fanout["properties"][key];
        assert_eq!(prop["maximum"], json!(limit), "{key} maximum");
        assert_eq!(prop["minimum"], json!(1), "{key} minimum");
        assert_eq!(prop["default"], json!(default), "{key} default");
    }
}
