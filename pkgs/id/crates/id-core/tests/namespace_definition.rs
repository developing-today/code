//! Invariant tests for the local namespace registry record (slice
//! A9116-registry-record, issue #10243; design A-9116.1, S4.1–S4.7 and
//! S4.19–S4.22).
//!
//! Oracle: **invariant** — over generated inputs: there are exactly two
//! cores, and a definition can never name a third (any other core token,
//! the derived shape included, fails as `unknown_core`); resolution is a
//! finite map keyed by explicit registry identity and namespace name, so
//! the same name resolves to each registry's own definition and a name
//! missing from the named registry fails with no fallback to any other;
//! records round-trip byte for byte; and a handler is only ever handed an
//! address on its namespace's core.
//!
//! The crate-root export of `namespace_definition` and `array_address`
//! belongs to slice 9110-02, so until that lands this test includes them
//! by path; `id_shape` and `canonical_cbor` are already exported and are
//! re-exported under the names the module uses.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod id_shape {
    pub use id_core::id_shape::*;
}

mod canonical_cbor {
    pub use id_core::canonical_cbor::*;
}

#[path = "../src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../src/namespace_definition.rs"]
mod namespace_definition;

use std::collections::BTreeMap;

use array_address::ArrayAddress;
use canonical_cbor::{Value, encode_canonical};
use id_shape::{ContentId, HashProfile, IdShape};
use namespace_definition::{
    CoreAddress, CoreKind, LocalRegistry, NamespaceDefinition, NamespaceId, RegistryId, Resolver,
    Token,
};
use proptest::prelude::*;

fn definition(name: &str, core: CoreKind, handler: &str) -> NamespaceDefinition {
    NamespaceDefinition {
        namespace: NamespaceId::new(name).unwrap(),
        core,
        handler: Token::new("handler", handler).unwrap(),
        encoding_profile: Token::new("encoding", "text").unwrap(),
        read_policy: Token::new("read", "anyone").unwrap(),
        write_policy: Token::new("write", "owner").unwrap(),
        shorthand_policy: Token::new("shorthand", "none").unwrap(),
        schema_version: 1,
    }
}

/// A record map with `core` written as given and the other fields valid.
fn record_with_core(core: &str) -> Vec<u8> {
    let mut entries = vec![
        ("v", Value::UInt(1)),
        ("namespace", Value::Text("files".into())),
        ("core", Value::Text(core.into())),
        ("handler", Value::Text("blob".into())),
        ("encoding", Value::Text("text".into())),
        ("read", Value::Text("anyone".into())),
        ("write", Value::Text("owner".into())),
        ("shorthand", Value::Text("none".into())),
        ("schema", Value::UInt(1)),
    ]
    .into_iter()
    .map(|(key, value)| (Value::Text(key.into()), value))
    .collect::<Vec<_>>();
    entries.sort_by_cached_key(|(key, _)| encode_canonical(key).unwrap());
    encode_canonical(&Value::Map(entries)).unwrap()
}

fn content_address() -> CoreAddress {
    CoreAddress::Content(ContentId::of_bytes(HashProfile::RawBlake3_256, b"abc").unwrap())
}

fn array_address() -> CoreAddress {
    CoreAddress::Array(ArrayAddress::new(vec![Some(b"a".to_vec()), None]))
}

// ---------------------------------------------------------------- fixed

#[test]
fn there_are_exactly_two_cores() {
    assert_eq!(CoreKind::ALL, [CoreKind::Content, CoreKind::Array]);
    for core in CoreKind::ALL {
        assert_eq!(CoreKind::from_token(core.token()).unwrap(), core);
        assert_eq!(CoreKind::from_shape(core.shape()).unwrap(), core);
        // The core token is the ID shape token of its addresses.
        assert_eq!(core.token(), core.shape().token());
    }
    // A derived namespace is a lens over a core, never a core.
    assert_eq!(
        CoreKind::from_shape(IdShape::Derived).unwrap_err().class(),
        "unknown_core"
    );
}

#[test]
fn a_record_cannot_name_a_third_core() {
    for core in [
        "derived", "name", "numeric", "stream", "world", "path", "hash", "Content", "ARRAY", "",
        "content ", "array2",
    ] {
        let error = NamespaceDefinition::decode(&record_with_core(core)).unwrap_err();
        assert_eq!(error.class(), "unknown_core", "{core:?}");
    }
    for core in ["content", "array"] {
        NamespaceDefinition::decode(&record_with_core(core)).unwrap();
    }
}

#[test]
fn record_bytes_are_pinned() {
    let record = definition("files", CoreKind::Content, "blob");
    let bytes = record.encode().unwrap();
    assert_eq!(bytes, record_with_core("content"));
    assert_eq!(NamespaceDefinition::decode(&bytes).unwrap(), record);
}

#[test]
fn every_field_is_critical_and_required() {
    let record = definition("files", CoreKind::Array, "rows");
    let Value::Map(mut entries) = record.to_value() else {
        panic!("not a map")
    };
    // An unknown field rejects.
    let mut extra = entries.clone();
    extra.push((Value::Text("zz".into()), Value::Null));
    assert_eq!(
        NamespaceDefinition::from_value(&Value::Map(extra))
            .unwrap_err()
            .class(),
        "unknown_field"
    );
    // A missing field rejects.
    for index in 0..entries.len() {
        let mut fewer = entries.clone();
        fewer.remove(index);
        assert_eq!(
            NamespaceDefinition::from_value(&Value::Map(fewer))
                .unwrap_err()
                .class(),
            "malformed"
        );
    }
    // Another version rejects.
    for (key, value) in &mut entries {
        if *key == Value::Text("v".into()) {
            *value = Value::UInt(2);
        }
    }
    assert_eq!(
        NamespaceDefinition::from_value(&Value::Map(entries))
            .unwrap_err()
            .class(),
        "unsupported_version"
    );
}

#[test]
fn handlers_get_only_addresses_on_their_core() {
    let files = definition("files", CoreKind::Content, "blob");
    let rows = definition("rows", CoreKind::Array, "table");
    assert_eq!(files.admit(content_address()).unwrap(), content_address());
    assert_eq!(rows.admit(array_address()).unwrap(), array_address());
    assert_eq!(
        files.admit(array_address()).unwrap_err().class(),
        "core_mismatch"
    );
    assert_eq!(
        rows.admit(content_address()).unwrap_err().class(),
        "core_mismatch"
    );
}

#[test]
fn same_name_resolves_differently_per_registry_with_no_fallback() {
    let alpha = RegistryId::server("alpha.example").unwrap();
    let beta = RegistryId::server("beta.example").unwrap();
    let handle = RegistryId::handle("imported-2026").unwrap();
    let mut a = LocalRegistry::new(alpha.clone());
    a.define(definition("files", CoreKind::Content, "blob"))
        .unwrap();
    a.define(definition("only-here", CoreKind::Array, "rows"))
        .unwrap();
    let mut b = LocalRegistry::new(beta.clone());
    b.define(definition("files", CoreKind::Array, "table"))
        .unwrap();
    let mut resolver = Resolver::new();
    resolver.add(a).unwrap();
    resolver.add(b).unwrap();

    let files = NamespaceId::new("files").unwrap();
    assert_eq!(
        resolver.resolve(&alpha, &files).unwrap().core,
        CoreKind::Content
    );
    assert_eq!(
        resolver.resolve(&beta, &files).unwrap().core,
        CoreKind::Array
    );

    // Defined in alpha only: beta does not fall back to alpha.
    let only = NamespaceId::new("only-here").unwrap();
    assert_eq!(
        resolver.resolve(&beta, &only).unwrap_err().class(),
        "unknown_namespace"
    );
    // A registry the resolver does not have is not replaced by another.
    assert_eq!(
        resolver.resolve(&handle, &files).unwrap_err().class(),
        "unknown_registry"
    );
    // A server ID and a handle with the same text are different registries.
    let same_text = RegistryId::handle("alpha.example").unwrap();
    assert_eq!(
        resolver.resolve(&same_text, &files).unwrap_err().class(),
        "unknown_registry"
    );
}

#[test]
fn duplicates_are_refused() {
    let id = RegistryId::server("alpha").unwrap();
    let mut registry = LocalRegistry::new(id);
    registry
        .define(definition("files", CoreKind::Content, "blob"))
        .unwrap();
    assert_eq!(
        registry
            .define(definition("files", CoreKind::Array, "other"))
            .unwrap_err()
            .class(),
        "duplicate"
    );
    assert_eq!(registry.definitions().count(), 1);
    let mut resolver = Resolver::new();
    resolver.add(registry.clone()).unwrap();
    assert_eq!(resolver.add(registry).unwrap_err().class(), "duplicate");
}

#[test]
fn malformed_names_and_tokens_are_refused() {
    for name in [
        "",
        "1files",
        "-files",
        "fi les",
        "files/x",
        "fïles",
        &"a".repeat(64),
    ] {
        assert_eq!(
            NamespaceId::new(name).unwrap_err().class(),
            "malformed",
            "{name:?}"
        );
    }
    for token in ["", "Blob", "-blob", "bl ob", &"b".repeat(64)] {
        assert_eq!(
            Token::new("handler", token).unwrap_err().class(),
            "malformed",
            "{token:?}"
        );
    }
    for id in ["", "has space", "tab\t", &"r".repeat(256)] {
        assert_eq!(RegistryId::server(id).unwrap_err().class(), "malformed");
        assert_eq!(RegistryId::handle(id).unwrap_err().class(), "malformed");
    }
}

// ---------------------------------------------------------------- generated

fn core() -> impl Strategy<Value = CoreKind> {
    prop_oneof![Just(CoreKind::Content), Just(CoreKind::Array)]
}

fn name() -> impl Strategy<Value = String> {
    "[a-c][a-c0-9-]{0,3}"
}

fn token() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._-]{0,8}"
}

fn any_definition() -> impl Strategy<Value = NamespaceDefinition> {
    (
        name(),
        core(),
        token(),
        token(),
        token(),
        token(),
        token(),
        any::<u64>(),
    )
        .prop_map(
            |(name, core, handler, encoding, read, write, shorthand, schema)| NamespaceDefinition {
                namespace: NamespaceId::new(name).unwrap(),
                core,
                handler: Token::new("handler", handler).unwrap(),
                encoding_profile: Token::new("encoding", encoding).unwrap(),
                read_policy: Token::new("read", read).unwrap(),
                write_policy: Token::new("write", write).unwrap(),
                shorthand_policy: Token::new("shorthand", shorthand).unwrap(),
                schema_version: schema,
            },
        )
}

fn registry_id() -> impl Strategy<Value = RegistryId> {
    prop_oneof![
        "s[0-2]".prop_map(|id| RegistryId::server(id).unwrap()),
        "s[0-2]".prop_map(|id| RegistryId::handle(id).unwrap()),
    ]
}

proptest! {
    #[test]
    fn records_round_trip_byte_for_byte(record in any_definition()) {
        let bytes = record.encode().unwrap();
        let read = NamespaceDefinition::decode(&bytes).unwrap();
        prop_assert_eq!(&read, &record);
        prop_assert_eq!(read.encode().unwrap(), bytes);
    }

    #[test]
    fn only_content_and_array_are_cores(core in "\\PC{0,12}") {
        let expected = matches!(core.as_str(), "content" | "array");
        prop_assert_eq!(CoreKind::from_token(&core).is_ok(), expected);
        match NamespaceDefinition::decode(&record_with_core(&core)) {
            Ok(record) => {
                prop_assert!(expected);
                prop_assert_eq!(record.core.token(), core.as_str());
            }
            Err(error) => {
                prop_assert!(!expected);
                prop_assert_eq!(error.class(), "unknown_core");
            }
        }
    }

    #[test]
    fn resolution_is_the_explicit_finite_map(
        known in proptest::collection::btree_set(registry_id(), 0..4),
        inserts in proptest::collection::vec((registry_id(), any_definition()), 0..24),
        queries in proptest::collection::vec((registry_id(), name()), 1..24),
    ) {
        // Model: registry identity -> namespace name -> definition.
        let mut model: BTreeMap<RegistryId, BTreeMap<NamespaceId, NamespaceDefinition>> =
            known.iter().map(|id| (id.clone(), BTreeMap::new())).collect();
        let mut registries: BTreeMap<RegistryId, LocalRegistry> =
            known.iter().map(|id| (id.clone(), LocalRegistry::new(id.clone()))).collect();
        for (id, record) in inserts {
            let Some(registry) = registries.get_mut(&id) else { continue };
            let names = model.get_mut(&id).unwrap();
            let result = registry.define(record.clone());
            if names.contains_key(&record.namespace) {
                prop_assert_eq!(result.unwrap_err().class(), "duplicate");
            } else {
                prop_assert!(result.is_ok());
                names.insert(record.namespace.clone(), record);
            }
        }
        let mut resolver = Resolver::new();
        for registry in registries.into_values() {
            resolver.add(registry).unwrap();
        }
        for (context, name) in queries {
            let namespace = NamespaceId::new(name).unwrap();
            let actual = resolver.resolve(&context, &namespace);
            match model.get(&context) {
                None => prop_assert_eq!(actual.unwrap_err().class(), "unknown_registry"),
                Some(names) => {
                    if let Some(expected) = names.get(&namespace) {
                        prop_assert_eq!(actual.unwrap(), expected);
                    } else {
                        prop_assert_eq!(actual.unwrap_err().class(), "unknown_namespace");
                    }
                }
            }
        }
    }

    #[test]
    fn admit_accepts_exactly_the_definitions_core(record in any_definition(), on_content in any::<bool>()) {
        let address = if on_content { content_address() } else { array_address() };
        let result = record.admit(address.clone());
        if address.core() == record.core {
            prop_assert_eq!(result.unwrap(), address);
        } else {
            prop_assert_eq!(result.unwrap_err().class(), "core_mismatch");
        }
    }
}
