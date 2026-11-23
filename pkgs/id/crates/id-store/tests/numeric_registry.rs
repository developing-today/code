//! Integration tests for the A9122 numeric registry write boundary.
//!
//! Oracle: a small independent policy procedure checks the allowed kind mask,
//! while the existing numeric comparator is the operational-budget oracle.
//! Registry generation and authorization remain exercised through the
//! `RegistryState` seam rather than being reimplemented in this test.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use id_core::namespace_definition::{
    CoreKind, NamespaceDefinition, NamespaceId, RegistryId, Token,
};
use id_core::numeric_value::{BigInt, NumericLimits, NumericValue};
use id_store::namespace_registry::{AdminScopeAuthorizer, MutationContext, RegistryState};
use id_store::numeric_registry::{NumericNamespacePolicy, NumericRegistry, NumericRegistryError};

fn definition(name: &str) -> NamespaceDefinition {
    NamespaceDefinition {
        namespace: NamespaceId::new(name).unwrap(),
        core: CoreKind::Array,
        handler: Token::new("handler", "numeric").unwrap(),
        encoding_profile: Token::new("encoding", "numeric-v1").unwrap(),
        read_policy: Token::new("read", "anyone").unwrap(),
        write_policy: Token::new("write", "owner").unwrap(),
        shorthand_policy: Token::new("shorthand", "none").unwrap(),
        schema_version: 1,
    }
}

fn admin() -> MutationContext {
    MutationContext::new("operator", "tenant-a", "registry-admin").unwrap()
}

#[test]
fn rejects_a_kind_not_allowed_by_the_namespace_policy_at_admission() {
    let policy = NumericNamespacePolicy::with_limits(
        1 << id_core::numeric_value::KIND_INTEGER,
        NumericLimits::default(),
    );
    let registry = NumericRegistry::new(policy);
    let error = registry
        .admit(NumericValue::decimal(&BigInt::from_i64(3), &BigInt::zero()))
        .unwrap_err();
    assert_eq!(error, NumericRegistryError::KindNotAllowed { kind: 1 });
    assert_eq!(error.code(), "kind_not_allowed");
}

#[test]
fn configured_budget_is_resource_limit_not_numeric_semantics() {
    let registry = NumericRegistry::new(NumericNamespacePolicy::with_limits(
        id_store::numeric_registry::ALL_KINDS,
        NumericLimits {
            max_magnitude_bytes: 8,
        },
    ));
    let value = NumericValue::integer(BigInt::from_be_magnitude(false, &[0xff; 9]));
    let error = registry.admit(value).unwrap_err();
    assert_eq!(error.code(), "resource_limit");
    assert!(matches!(error, NumericRegistryError::Numeric(_)));
}

#[test]
fn large_valid_value_is_admitted_within_operational_budget() {
    let registry = NumericRegistry::new(NumericNamespacePolicy::with_limits(
        id_store::numeric_registry::ALL_KINDS,
        NumericLimits {
            max_magnitude_bytes: 8 * 1024,
        },
    ));
    let digits = "9".repeat(5_000);
    let value = NumericValue::decimal(
        &BigInt::parse(&digits).unwrap(),
        &BigInt::parse("-400").unwrap(),
    );
    let key = registry.admit(value.clone()).unwrap();
    assert_eq!(key.value(), &value);
}

#[test]
fn admitted_key_does_not_bypass_versioned_registry_authorization() {
    let validator = NumericRegistry::new(NumericNamespacePolicy::ordered());
    let key = validator
        .admit(NumericValue::integer(BigInt::from_i64(7)))
        .unwrap();
    assert_eq!(key.value().kind_tag(), id_core::numeric_value::KIND_INTEGER);

    let mut state = RegistryState::new(RegistryId::server("numeric").unwrap());
    let authorizer = AdminScopeAuthorizer::new("registry-admin");
    assert_eq!(
        state.create(&authorizer, &admin(), 0, definition("numbers")),
        Ok(1)
    );
    assert_eq!(state.generation(), 1);
}
