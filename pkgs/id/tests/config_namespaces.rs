//! The namespace plane `namespaces`, sixth part of slice 9111-03 (#9111
//! record §3.1, §6.3, §6.4, §7.4).
//!
//! Oracle: the record's rules, stated before the code. `namespaces` is a
//! reference to the server-local namespace registry (#9116), and its
//! default is the built-in registry only. The reference is `builtin`,
//! `server:<id>` or `handle:<handle>`, where the ID or handle is 1 to 255
//! visible ASCII characters. An absent key and `null` both take the
//! default. Anything else is a load error, unknown keys fail closed, and a
//! document without `namespaces`, or one that restates the default, keeps
//! its digest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    BUILTIN_NAMESPACE_REGISTRY, ConfigDocument, ConfigError, FieldClass,
    MAX_NAMESPACE_REGISTRY_ID_LEN, RegistryRef, ResolvedConfig, explain, field_class,
    resolved_digest,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-namespaces",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        }
    })
}

fn with_namespaces(namespaces: Value) -> Value {
    let mut doc = base();
    doc["namespaces"] = namespaces;
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
fn an_absent_section_uses_the_builtin_registry_only() {
    let resolved = load(&base()).unwrap();
    assert_eq!(resolved.namespaces.registry, "builtin");
    assert_eq!(BUILTIN_NAMESPACE_REGISTRY, "builtin");
    assert!(resolved.namespaces.is_default());
    assert_eq!(
        resolved.namespaces.registry_ref().unwrap(),
        RegistryRef::Builtin
    );

    let empty = load(&with_namespaces(json!({}))).unwrap();
    assert!(empty.namespaces.is_default());
    let null = load(&with_namespaces(json!({ "registry": null }))).unwrap();
    assert!(null.namespaces.is_default());
}

#[test]
fn server_and_handle_references_resolve() {
    let server = load(&with_namespaces(json!({ "registry": "server:node-7" }))).unwrap();
    assert_eq!(server.namespaces.registry, "server:node-7");
    assert!(!server.namespaces.is_default());
    assert_eq!(
        server.namespaces.registry_ref().unwrap(),
        RegistryRef::Server("node-7".to_owned())
    );

    let handle = load(&with_namespaces(json!({ "registry": "handle:team/main" }))).unwrap();
    assert_eq!(
        handle.namespaces.registry_ref().unwrap(),
        RegistryRef::Handle("team/main".to_owned())
    );

    let longest = format!("server:{}", "a".repeat(MAX_NAMESPACE_REGISTRY_ID_LEN));
    assert!(load(&with_namespaces(json!({ "registry": longest }))).is_ok());
}

#[test]
fn the_display_form_is_the_written_form() {
    for text in ["builtin", "server:x", "handle:h:1"] {
        assert_eq!(RegistryRef::parse(text).unwrap().to_string(), text);
    }
}

#[test]
fn a_malformed_reference_rejects_the_document() {
    let too_long = format!("handle:{}", "a".repeat(MAX_NAMESPACE_REGISTRY_ID_LEN + 1));
    for registry in [
        "",
        "Builtin",
        "builtin:",
        "server",
        "server:",
        "handle:",
        "cluster:x",
        "server:a b",
        "server:caf\u{e9}",
        "server:\tx",
        too_long.as_str(),
    ] {
        let err = load_err(&with_namespaces(json!({ "registry": registry })));
        assert!(
            matches!(&err, ConfigError::InvalidNamespaceRegistry(text) if text == registry),
            "{registry:?}: {err:?}"
        );
    }
}

#[test]
fn unknown_keys_and_wrong_types_fail_closed() {
    for namespaces in [
        json!({ "registries": "builtin" }),
        json!({ "registry": "builtin", "fallback": "global" }),
        json!({ "namespaces": {} }),
        json!({ "registry": 7 }),
        json!({ "registry": ["builtin"] }),
        json!("builtin"),
    ] {
        let doc = with_namespaces(namespaces.clone());
        assert!(
            ConfigDocument::parse(&doc.to_string()).is_err(),
            "{namespaces}"
        );
    }
}

#[test]
fn the_digest_is_unchanged_without_namespaces_or_with_only_the_default() {
    let plain = resolved_digest(&load(&base()).unwrap()).unwrap();
    for restated in [
        json!({}),
        json!({ "registry": null }),
        json!({ "registry": "builtin" }),
    ] {
        let digest = resolved_digest(&load(&with_namespaces(restated.clone())).unwrap()).unwrap();
        assert_eq!(plain, digest, "{restated}");
    }
    let resolved = load(&base()).unwrap();
    let canonical: Value = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert!(canonical.get("namespaces").is_none());

    let other =
        resolved_digest(&load(&with_namespaces(json!({ "registry": "server:node-7" }))).unwrap())
            .unwrap();
    assert_ne!(plain, other);
}

#[test]
fn a_resolved_config_with_a_registry_round_trips() {
    let resolved = load(&with_namespaces(json!({ "registry": "handle:ops" }))).unwrap();
    let canonical = resolved.canonical_json().unwrap();
    let value: Value = serde_json::from_str(&canonical).unwrap();
    assert_eq!(value["namespaces"], json!({ "registry": "handle:ops" }));
    let back: ResolvedConfig = serde_json::from_str(&canonical).unwrap();
    assert_eq!(back, resolved);
}

#[test]
fn explain_names_the_registry_and_its_source() {
    let entry = |doc: &Value| {
        explain(&doc.to_string())
            .unwrap()
            .entries
            .into_iter()
            .find(|e| e.path == "namespaces.registry")
            .expect("no namespaces.registry entry")
    };
    let default = entry(&base());
    assert_eq!(default.value, "\"builtin\"");
    assert_eq!(default.source, "default");

    let null = entry(&with_namespaces(json!({ "registry": null })));
    assert_eq!(null.source, "default");

    let written = entry(&with_namespaces(json!({ "registry": "server:node-7" })));
    assert_eq!(written.value, "\"server:node-7\"");
    assert_eq!(written.source, "document");
}

#[test]
fn namespaces_is_behavior_and_the_schema_names_it() {
    assert_eq!(field_class("namespaces"), Some(FieldClass::Behavior));
    assert_eq!(
        field_class("namespaces.registry"),
        Some(FieldClass::Behavior)
    );
    let schema: Value =
        serde_json::from_str(include_str!("../config/id-config-v1.schema.json")).unwrap();
    assert_eq!(
        schema["properties"]["namespaces"]["$ref"],
        json!("#/$defs/namespaces")
    );
    let namespaces = &schema["$defs"]["namespaces"];
    assert_eq!(namespaces["additionalProperties"], json!(false));
    let choices = namespaces["properties"]["registry"]["oneOf"]
        .as_array()
        .unwrap();
    assert!(choices.contains(&json!({ "const": "builtin" })));
    assert!(choices.contains(&json!({ "type": "null" })));
    assert!(
        choices
            .iter()
            .any(|c| c["pattern"] == json!("^(server|handle):[!-~]{1,255}$"))
    );
}
