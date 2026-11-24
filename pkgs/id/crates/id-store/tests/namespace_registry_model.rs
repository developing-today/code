//! Model tests for the versioned namespace registry state (slice
//! A9116-registry-state, issue #10257; design A-9116.1, S4.2–S4.10
//! and S4.18).
//!
//! Oracle: **model** — the registry state machine from the slice's
//! recorded oracle: *mutation requires injected authorized context
//! and increments one generation*. The reference model below is an
//! independent restatement of that machine (a plain map, a counter
//! and a log, with the check ordering the design fixes: authorize,
//! then expected generation, then the record rules). Generated
//! operation sequences run against both the implementation and the
//! model, and every resulting state and error is compared.
//!
//! Targeted tests pin the named cases the manifest calls out:
//! create/update/revoke, the concurrent-generation conflict, an
//! unauthorized mutation changing nothing, an update that tries to
//! change a namespace's core, persistence round-trips of the
//! versioned state, and two registries coexisting with the same
//! namespace name defined differently in each.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use id_core::canonical_cbor::Value;
use id_core::namespace_definition::{
    CoreKind, NamespaceDefinition, NamespaceError, NamespaceId, RegistryId, Token,
};
use id_store::namespace_registry::{
    AdminScopeAuthorizer, AuditEntry, MutationContext, MutationKind, RegistryAuthorizer,
    RegistryState, RegistryStateError,
};

const ADMIN_SCOPE: &str = "registry-admin";

fn definition(name: &str, core: CoreKind, handler: &str, schema: u64) -> NamespaceDefinition {
    NamespaceDefinition {
        namespace: NamespaceId::new(name).unwrap(),
        core,
        handler: Token::new("handler", handler).unwrap(),
        encoding_profile: Token::new("encoding", "text").unwrap(),
        read_policy: Token::new("read", "anyone").unwrap(),
        write_policy: Token::new("write", "owner").unwrap(),
        shorthand_policy: Token::new("shorthand", "none").unwrap(),
        schema_version: schema,
    }
}

fn admin() -> MutationContext {
    MutationContext::new("operator", "tenant-a", ADMIN_SCOPE).unwrap()
}

fn reader() -> MutationContext {
    MutationContext::new("operator", "tenant-a", "registry-read").unwrap()
}

fn authorizer() -> AdminScopeAuthorizer {
    AdminScopeAuthorizer::new(ADMIN_SCOPE)
}

fn server_state() -> RegistryState {
    RegistryState::new(RegistryId::server("amd-6").unwrap())
}

/// The reference model: the oracle restated with a plain map, a
/// counter and a log.
#[derive(Debug, Default)]
struct Model {
    definitions: BTreeMap<String, NamespaceDefinition>,
    generation: u64,
    audit: Vec<AuditEntry>,
}

impl Model {
    fn apply(
        &mut self,
        context: &MutationContext,
        expected: u64,
        mutation: MutationKind,
        definition: Option<&NamespaceDefinition>,
        namespace: &NamespaceId,
    ) -> Result<u64, RegistryStateError> {
        // Authorize first: the shipped authorizer's rule, restated.
        if context.authorization_scope() != ADMIN_SCOPE {
            return Err(RegistryStateError::Unauthorized {
                registry: RegistryId::server("amd-6").unwrap(),
                mutation,
                authorization_scope: context.authorization_scope().to_owned(),
            });
        }
        if expected != self.generation {
            return Err(RegistryStateError::GenerationConflict {
                expected,
                actual: self.generation,
            });
        }
        match mutation {
            MutationKind::Create => {
                let definition = definition.expect("create carries a definition");
                if self.definitions.contains_key(definition.namespace.as_str()) {
                    return Err(RegistryStateError::Record(
                        NamespaceError::DuplicateNamespace {
                            namespace: definition.namespace.clone(),
                        },
                    ));
                }
                self.definitions
                    .insert(definition.namespace.as_str().to_owned(), definition.clone());
            }
            MutationKind::Update => {
                let definition = definition.expect("update carries a definition");
                let Some(existing) = self.definitions.get(definition.namespace.as_str()) else {
                    return Err(RegistryStateError::Record(
                        NamespaceError::UnknownNamespace {
                            registry: RegistryId::server("amd-6").unwrap(),
                            namespace: definition.namespace.clone(),
                        },
                    ));
                };
                if existing.core != definition.core {
                    return Err(RegistryStateError::CoreChange {
                        namespace: definition.namespace.clone(),
                        expected: existing.core,
                        found: definition.core,
                    });
                }
                self.definitions
                    .insert(definition.namespace.as_str().to_owned(), definition.clone());
            }
            MutationKind::Revoke => {
                if self.definitions.remove(namespace.as_str()).is_none() {
                    return Err(RegistryStateError::Record(
                        NamespaceError::UnknownNamespace {
                            registry: RegistryId::server("amd-6").unwrap(),
                            namespace: namespace.clone(),
                        },
                    ));
                }
            }
        }
        self.generation += 1;
        self.audit.push(AuditEntry {
            generation: self.generation,
            mutation,
            namespace: namespace.clone(),
            principal: context.principal().to_owned(),
            tenant_scope: context.tenant_scope().to_owned(),
            authorization_scope: context.authorization_scope().to_owned(),
        });
        Ok(self.generation)
    }
}

/// A small deterministic generator (the workspace's model tests use
/// seeded generation; a splitmix-style LCG keeps this one dep-free).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<'a, T>(&mut self, choices: &'a [T]) -> &'a T {
        &choices[usize::try_from(self.below(u64::try_from(choices.len()).unwrap())).unwrap()]
    }
}

const NAMES: [&str; 4] = ["main", "wiki", "lab", "ops"];
const HANDLERS: [&str; 2] = ["files", "docs"];

#[test]
fn generated_operations_match_the_model() {
    for seed in [1_u64, 42, 0xdead_beef] {
        let mut rng = Rng(seed);
        let mut state = server_state();
        let mut model = Model::default();
        let auth = authorizer();
        for step in 0..300 {
            let mutation = *rng.pick(&[
                MutationKind::Create,
                MutationKind::Update,
                MutationKind::Revoke,
            ]);
            let name = rng.pick(&NAMES);
            let namespace = NamespaceId::new(*name).unwrap();
            let core = *rng.pick(&[CoreKind::Content, CoreKind::Array]);
            let candidate = definition(name, core, rng.pick(&HANDLERS), 1 + rng.below(2));
            let context = if rng.below(4) == 0 { reader() } else { admin() };
            // Mostly the true generation; sometimes a stale or a
            // future one (the concurrent-writer case).
            let expected = match rng.below(5) {
                0 if model.generation > 0 => model.generation - 1,
                1 => model.generation + 1,
                _ => model.generation,
            };
            let definition_arg = match mutation {
                MutationKind::Revoke => None,
                _ => Some(&candidate),
            };
            let expected_result =
                model.apply(&context, expected, mutation, definition_arg, &namespace);
            let actual_result = match mutation {
                MutationKind::Create => state.create(&auth, &context, expected, candidate.clone()),
                MutationKind::Update => state.update(&auth, &context, expected, candidate.clone()),
                MutationKind::Revoke => state.revoke(&auth, &context, expected, &namespace),
            };
            assert_eq!(
                actual_result, expected_result,
                "seed {seed} step {step}: {mutation} {name}"
            );
            assert_eq!(
                state.generation(),
                model.generation,
                "seed {seed} step {step}"
            );
            let actual_definitions: BTreeMap<String, NamespaceDefinition> = state
                .definitions()
                .map(|d| (d.namespace.as_str().to_owned(), d.clone()))
                .collect();
            assert_eq!(
                actual_definitions, model.definitions,
                "seed {seed} step {step}"
            );
            assert_eq!(state.audit_log(), model.audit, "seed {seed} step {step}");
        }
    }
}

#[test]
fn each_successful_mutation_increments_exactly_one_generation() {
    let mut state = server_state();
    let auth = authorizer();
    assert_eq!(state.generation(), 0);
    let created = state
        .create(
            &auth,
            &admin(),
            0,
            definition("main", CoreKind::Content, "files", 1),
        )
        .unwrap();
    assert_eq!(created, 1);
    let updated = state
        .update(
            &auth,
            &admin(),
            1,
            definition("main", CoreKind::Content, "files", 2),
        )
        .unwrap();
    assert_eq!(updated, 2);
    let revoked = state
        .revoke(&auth, &admin(), 2, &NamespaceId::new("main").unwrap())
        .unwrap();
    assert_eq!(revoked, 3);
    assert_eq!(state.audit_log().len(), 3);
    assert_eq!(
        state
            .audit_log()
            .iter()
            .map(|e| e.generation)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn unauthorized_mutation_changes_nothing() {
    let mut state = server_state();
    let auth = authorizer();
    state
        .create(
            &auth,
            &admin(),
            0,
            definition("main", CoreKind::Content, "files", 1),
        )
        .unwrap();
    let before = state.clone();
    let refused = state.create(
        &auth,
        &reader(),
        1,
        definition("wiki", CoreKind::Array, "docs", 1),
    );
    assert_eq!(
        refused,
        Err(RegistryStateError::Unauthorized {
            registry: RegistryId::server("amd-6").unwrap(),
            mutation: MutationKind::Create,
            authorization_scope: "registry-read".to_owned(),
        })
    );
    // Authorization is checked before the generation: an
    // unauthorized caller with a wrong generation still gets the
    // refusal, not the conflict.
    let refused_first = state.revoke(&auth, &reader(), 99, &NamespaceId::new("main").unwrap());
    assert!(matches!(
        refused_first,
        Err(RegistryStateError::Unauthorized { .. })
    ));
    assert_eq!(state, before);
}

#[test]
fn generation_conflict_changes_nothing() {
    let mut state = server_state();
    let auth = authorizer();
    state
        .create(
            &auth,
            &admin(),
            0,
            definition("main", CoreKind::Content, "files", 1),
        )
        .unwrap();
    let before = state.clone();
    // Two writers both read generation 1; the first lands, the
    // second conflicts.
    state
        .create(
            &auth,
            &admin(),
            1,
            definition("wiki", CoreKind::Array, "docs", 1),
        )
        .unwrap();
    let conflict = state.create(
        &auth,
        &admin(),
        1,
        definition("lab", CoreKind::Content, "files", 1),
    );
    assert_eq!(
        conflict,
        Err(RegistryStateError::GenerationConflict {
            expected: 1,
            actual: 2,
        })
    );
    assert_ne!(state, before);
    assert_eq!(state.generation(), 2);
    assert!(state.resolve(&NamespaceId::new("lab").unwrap()).is_err());
}

#[test]
fn update_cannot_change_core() {
    let mut state = server_state();
    let auth = authorizer();
    state
        .create(
            &auth,
            &admin(),
            0,
            definition("main", CoreKind::Content, "files", 1),
        )
        .unwrap();
    let moved = state.update(
        &auth,
        &admin(),
        1,
        definition("main", CoreKind::Array, "files", 1),
    );
    assert_eq!(
        moved,
        Err(RegistryStateError::CoreChange {
            namespace: NamespaceId::new("main").unwrap(),
            expected: CoreKind::Content,
            found: CoreKind::Array,
        })
    );
    assert_eq!(state.generation(), 1);
    assert_eq!(
        state
            .resolve(&NamespaceId::new("main").unwrap())
            .unwrap()
            .core,
        CoreKind::Content
    );
}

#[test]
fn state_round_trips_through_canonical_bytes() {
    let mut state = server_state();
    let auth = authorizer();
    state
        .create(
            &auth,
            &admin(),
            0,
            definition("main", CoreKind::Content, "files", 1),
        )
        .unwrap();
    state
        .create(
            &auth,
            &admin(),
            1,
            definition("wiki", CoreKind::Array, "docs", 3),
        )
        .unwrap();
    state
        .revoke(&auth, &admin(), 2, &NamespaceId::new("main").unwrap())
        .unwrap();
    let bytes = state.encode().unwrap();
    let decoded = RegistryState::decode(&bytes).unwrap();
    assert_eq!(decoded, state);
    // Re-encoding the decoded state reproduces the bytes exactly:
    // the persisted form is canonical.
    assert_eq!(decoded.encode().unwrap(), bytes);
    // And the decoded state keeps mutating from its generation.
    let mut decoded = decoded;
    let next = decoded
        .create(
            &auth,
            &admin(),
            3,
            definition("lab", CoreKind::Content, "files", 1),
        )
        .unwrap();
    assert_eq!(next, 4);
}

#[test]
fn decode_rejects_unknown_fields_and_other_versions() {
    let state = server_state();
    let Value::Map(mut entries) = state.to_value() else {
        panic!("state value is a map");
    };
    let mut unknown = entries.clone();
    unknown.push((Value::Text("surprise".to_owned()), Value::UInt(1)));
    assert_eq!(
        RegistryState::from_value(&Value::Map(unknown)),
        Err(RegistryStateError::UnknownField {
            field: "surprise".to_owned(),
        })
    );
    for (key, value) in &mut entries {
        if *key == Value::Text("v".to_owned()) {
            *value = Value::UInt(2);
        }
    }
    assert_eq!(
        RegistryState::from_value(&Value::Map(entries)),
        Err(RegistryStateError::UnsupportedVersion { version: 2 })
    );
}

#[test]
fn registries_coexist_with_independent_state() {
    let auth = authorizer();
    let mut one = server_state();
    let mut two = RegistryState::new(RegistryId::handle("lab-rig").unwrap());
    one.create(
        &auth,
        &admin(),
        0,
        definition("main", CoreKind::Content, "files", 1),
    )
    .unwrap();
    two.create(
        &auth,
        &admin(),
        0,
        definition("main", CoreKind::Array, "docs", 1),
    )
    .unwrap();
    let name = NamespaceId::new("main").unwrap();
    assert_eq!(one.resolve(&name).unwrap().core, CoreKind::Content);
    assert_eq!(two.resolve(&name).unwrap().core, CoreKind::Array);
    // A mutation of one registry is invisible to the other.
    two.revoke(&auth, &admin(), 1, &name).unwrap();
    assert!(two.resolve(&name).is_err());
    assert!(one.resolve(&name).is_ok());
    assert_eq!(one.generation(), 1);
}

#[test]
fn authorizer_is_a_seam_not_a_singleton() {
    // A deployment-supplied authorizer with a different rule (here:
    // a tenant-bound one) plugs in without the state changing.
    struct TenantAuthorizer;
    impl RegistryAuthorizer for TenantAuthorizer {
        fn authorize(
            &self,
            context: &MutationContext,
            registry: &RegistryId,
            mutation: MutationKind,
        ) -> Result<(), RegistryStateError> {
            if context.tenant_scope() == "tenant-a" {
                Ok(())
            } else {
                Err(RegistryStateError::Unauthorized {
                    registry: registry.clone(),
                    mutation,
                    authorization_scope: context.authorization_scope().to_owned(),
                })
            }
        }
    }
    let mut state = server_state();
    let outsider = MutationContext::new("operator", "tenant-b", ADMIN_SCOPE).unwrap();
    let refused = state.create(
        &TenantAuthorizer,
        &outsider,
        0,
        definition("main", CoreKind::Content, "files", 1),
    );
    assert!(matches!(
        refused,
        Err(RegistryStateError::Unauthorized { .. })
    ));
    assert_eq!(state.generation(), 0);
}
