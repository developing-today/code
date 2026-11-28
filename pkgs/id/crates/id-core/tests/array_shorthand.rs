//! Model tests for configured bare-string shorthand (slice
//! A9119-shorthand-policy, issue #10261; design A-9119.1, S4.29–S4.33).
//!
//! Oracle: **model** — the expansion function is compared with a
//! configured-index reference over generated configuration operations.
//! Every expansion and every error matches the model; an unset policy is
//! `missing_shorthand_policy` with no fallback index; a server override
//! beats the namespace default only on the server where it is configured;
//! and only array-core namespaces accept shorthand.
//!
//! The crate-root export of `array_shorthand`, `namespace_definition` and
//! `array_address` belongs to slice 9110-02, so until that lands this
//! test includes them by path; `id_shape` and `canonical_cbor` are
//! already exported and are re-exported under the names the modules use.

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
#[allow(dead_code)]
mod namespace_definition;

#[path = "../src/array_shorthand.rs"]
mod array_shorthand;

use array_address::ArrayAddress;
use array_shorthand::{PrefixFill, ShorthandConfig, ShorthandError, ShorthandPolicy, TextEncoding};
use namespace_definition::{CoreKind, NamespaceDefinition, NamespaceId, RegistryId, Token};
use proptest::prelude::*;

const NAMESPACES: [&str; 3] = ["wiki", "people", "blobs"];
const SERVERS: [&str; 2] = ["alpha", "beta"];

fn ns(name: &str) -> NamespaceId {
    NamespaceId::new(name).unwrap()
}

fn server(name: &str) -> RegistryId {
    RegistryId::server(name).unwrap()
}

fn definition(name: &str, core: CoreKind) -> NamespaceDefinition {
    NamespaceDefinition {
        namespace: ns(name),
        core,
        handler: Token::new("handler", "store").unwrap(),
        encoding_profile: Token::new("encoding", "text").unwrap(),
        read_policy: Token::new("read", "anyone").unwrap(),
        write_policy: Token::new("write", "owner").unwrap(),
        shorthand_policy: Token::new("shorthand", "configured").unwrap(),
        schema_version: 1,
    }
}

fn slots(address: &ArrayAddress) -> Vec<Option<Vec<u8>>> {
    address.slots().to_vec()
}

#[test]
fn design_example_slot_two_expands_with_null_prefix() {
    // Slot 2 is the design's example configuration, not a default.
    let mut config = ShorthandConfig::new();
    config.set_namespace_default(ns("wiki"), ShorthandPolicy::new(2));
    let got = config
        .expand(
            &server("alpha"),
            &definition("wiki", CoreKind::Array),
            "name",
        )
        .unwrap();
    assert_eq!(slots(&got), vec![None, None, Some(b"name".to_vec())]);
}

#[test]
fn unset_policy_is_an_error_not_a_fallback() {
    let config = ShorthandConfig::new();
    let error = config
        .expand(
            &server("alpha"),
            &definition("wiki", CoreKind::Array),
            "name",
        )
        .unwrap_err();
    assert_eq!(error.class(), "missing_shorthand_policy");
    assert_eq!(
        error,
        ShorthandError::MissingShorthandPolicy {
            namespace: ns("wiki")
        }
    );
    assert_eq!(error.to_string(), "missing_shorthand_policy: wiki");
    // A policy on one namespace does not leak to another.
    let mut config = ShorthandConfig::new();
    config.set_namespace_default(ns("people"), ShorthandPolicy::new(0));
    assert_eq!(
        config
            .policy_for(&server("alpha"), &ns("wiki"))
            .unwrap_err()
            .class(),
        "missing_shorthand_policy"
    );
}

#[test]
fn server_override_wins_only_where_configured() {
    let mut config = ShorthandConfig::new();
    config.set_namespace_default(ns("wiki"), ShorthandPolicy::new(1));
    config.set_server_override(server("alpha"), ns("wiki"), ShorthandPolicy::new(3));
    assert_eq!(
        config.policy_for(&server("alpha"), &ns("wiki")).unwrap(),
        ShorthandPolicy::new(3)
    );
    assert_eq!(
        config.policy_for(&server("beta"), &ns("wiki")).unwrap(),
        ShorthandPolicy::new(1)
    );
    // An override alone configures only its own server.
    config.clear_namespace_default(&ns("wiki"));
    assert_eq!(
        config.policy_for(&server("alpha"), &ns("wiki")).unwrap(),
        ShorthandPolicy::new(3)
    );
    assert_eq!(
        config
            .policy_for(&server("beta"), &ns("wiki"))
            .unwrap_err()
            .class(),
        "missing_shorthand_policy"
    );
    // A handle registry is a different context from a server with the
    // same text.
    assert_eq!(
        config
            .policy_for(&RegistryId::handle("alpha").unwrap(), &ns("wiki"))
            .unwrap_err()
            .class(),
        "missing_shorthand_policy"
    );
}

#[test]
fn content_core_namespaces_refuse_shorthand() {
    let mut config = ShorthandConfig::new();
    config.set_namespace_default(ns("blobs"), ShorthandPolicy::new(0));
    let error = config
        .expand(
            &server("alpha"),
            &definition("blobs", CoreKind::Content),
            "x",
        )
        .unwrap_err();
    assert_eq!(error.class(), "shorthand_requires_array_core");
}

#[test]
fn policy_shape_is_null_fill_and_utf8() {
    let policy = ShorthandPolicy::new(5);
    assert_eq!(policy.default_slot(), 5);
    assert_eq!(policy.prefix_fill(), PrefixFill::Null);
    assert_eq!(policy.text_encoding(), TextEncoding::Utf8);
    // Slot 0 is the string alone; the empty string is empty bytes, not null.
    assert_eq!(
        slots(&ShorthandPolicy::new(0).expand("").unwrap()),
        vec![Some(Vec::new())]
    );
    // Text bytes are the exact UTF-8, with no normalization.
    let text = "e\u{301}雪";
    assert_eq!(
        slots(&ShorthandPolicy::new(1).expand(text).unwrap()),
        vec![None, Some(text.as_bytes().to_vec())]
    );
}

#[test]
fn an_unallocatable_slot_is_an_error_not_an_abort() {
    assert_eq!(
        ShorthandPolicy::new(usize::MAX)
            .expand("x")
            .unwrap_err()
            .class(),
        "expansion_too_large"
    );
    assert_eq!(
        ShorthandPolicy::new(usize::MAX / 2)
            .expand("x")
            .unwrap_err()
            .class(),
        "expansion_too_large"
    );
}

/// One configuration step or query.
#[derive(Debug, Clone)]
enum Op {
    SetDefault(usize, usize),
    ClearDefault(usize),
    SetOverride(usize, usize, usize),
    ClearOverride(usize, usize),
    Expand(usize, usize, String, bool),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let ns_index = 0..NAMESPACES.len();
    let server_index = 0..SERVERS.len();
    let slot = 0_usize..12;
    prop_oneof![
        (ns_index.clone(), slot.clone()).prop_map(|(n, s)| Op::SetDefault(n, s)),
        ns_index.clone().prop_map(Op::ClearDefault),
        (server_index.clone(), ns_index.clone(), slot)
            .prop_map(|(v, n, s)| Op::SetOverride(v, n, s)),
        (server_index.clone(), ns_index.clone()).prop_map(|(v, n)| Op::ClearOverride(v, n)),
        (server_index, ns_index, "\\PC{0,8}", any::<bool>())
            .prop_map(|(v, n, t, array)| Op::Expand(v, n, t, array)),
    ]
}

/// The reference: plain association lists and a hand-built expansion.
#[derive(Default)]
struct Model {
    defaults: Vec<(usize, usize)>,
    overrides: Vec<((usize, usize), usize)>,
}

impl Model {
    fn set<K: PartialEq>(list: &mut Vec<(K, usize)>, key: K, slot: usize) -> Option<usize> {
        let old = Self::clear(list, &key);
        list.push((key, slot));
        old
    }

    fn clear<K: PartialEq>(list: &mut Vec<(K, usize)>, key: &K) -> Option<usize> {
        let index = list.iter().position(|(k, _)| k == key)?;
        Some(list.remove(index).1)
    }

    fn expand(
        &self,
        server: usize,
        namespace: usize,
        text: &str,
        array: bool,
    ) -> Result<Vec<Option<Vec<u8>>>, &'static str> {
        if !array {
            return Err("shorthand_requires_array_core");
        }
        let slot = self
            .overrides
            .iter()
            .find(|(key, _)| *key == (server, namespace))
            .map(|(_, slot)| *slot)
            .or_else(|| {
                self.defaults
                    .iter()
                    .find(|(key, _)| *key == namespace)
                    .map(|(_, slot)| *slot)
            })
            .ok_or("missing_shorthand_policy")?;
        let mut out = vec![None; slot];
        out.push(Some(text.as_bytes().to_vec()));
        Ok(out)
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn configuration_operations_match_the_reference_model(
        ops in proptest::collection::vec(op_strategy(), 0..40),
    ) {
        let mut config = ShorthandConfig::new();
        let mut model = Model::default();
        for op in ops {
            match op {
                Op::SetDefault(n, s) => {
                    let got = config.set_namespace_default(ns(NAMESPACES[n]), ShorthandPolicy::new(s));
                    let want = Model::set(&mut model.defaults, n, s);
                    prop_assert_eq!(got.map(ShorthandPolicy::default_slot), want);
                }
                Op::ClearDefault(n) => {
                    let got = config.clear_namespace_default(&ns(NAMESPACES[n]));
                    let want = Model::clear(&mut model.defaults, &n);
                    prop_assert_eq!(got.map(ShorthandPolicy::default_slot), want);
                }
                Op::SetOverride(v, n, s) => {
                    let got = config.set_server_override(
                        server(SERVERS[v]),
                        ns(NAMESPACES[n]),
                        ShorthandPolicy::new(s),
                    );
                    let want = Model::set(&mut model.overrides, (v, n), s);
                    prop_assert_eq!(got.map(ShorthandPolicy::default_slot), want);
                }
                Op::ClearOverride(v, n) => {
                    let got = config.clear_server_override(&server(SERVERS[v]), &ns(NAMESPACES[n]));
                    let want = Model::clear(&mut model.overrides, &(v, n));
                    prop_assert_eq!(got.map(ShorthandPolicy::default_slot), want);
                }
                Op::Expand(v, n, text, array) => {
                    let core = if array { CoreKind::Array } else { CoreKind::Content };
                    let got = config
                        .expand(&server(SERVERS[v]), &definition(NAMESPACES[n], core), &text)
                        .map(|address| slots(&address))
                        .map_err(|error| error.class());
                    prop_assert_eq!(got, model.expand(v, n, &text, array));
                }
            }
        }
    }

    /// Expansion is pure: the string is always the last slot, at exactly
    /// the configured index, and every earlier slot is null.
    #[test]
    fn expansion_places_the_string_at_the_configured_index(
        slot in 0_usize..64,
        text in "\\PC{0,16}",
    ) {
        let got = ShorthandPolicy::new(slot).expand(&text).unwrap();
        prop_assert_eq!(got.len(), slot + 1);
        prop_assert!(got.slots()[..slot].iter().all(Option::is_none));
        prop_assert_eq!(got.slots()[slot].as_deref(), Some(text.as_bytes()));
    }
}
