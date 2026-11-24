//! Versioned namespace registry state and the authorization seam
//! (slice A9116-registry-state, issue #10257; parent #9116; design
//! `part-a-addressing/versioned-design.md` section A-9116.1,
//! statements S4.2–S4.10 and S4.18).
//!
//! The record slice (A9116-registry-record, `id_core::
//! namespace_definition`) defines what a namespace definition *is*.
//! This slice defines how a local registry *changes*: a
//! [`RegistryState`] is a [`LocalRegistry`] plus a generation counter
//! and an audit log, and the only way to change it is a mutation.
//!
//! # The state machine
//!
//! A mutation is one of create, update or revoke. Every mutation:
//!
//! 1. is authorized first — the injected [`RegistryAuthorizer`]
//!    judges the injected [`MutationContext`] before anything else
//!    is checked, so an unauthorized caller learns nothing, not even
//!    whether its expected generation was right;
//! 2. names the generation it expects — a mismatch is
//!    [`RegistryStateError::GenerationConflict`] and changes nothing,
//!    which is what makes two writers racing one registry safe: the
//!    loser's next attempt conflicts instead of silently overwriting;
//! 3. applies through the record layer, so a create of a taken name
//!    is the record's `DuplicateNamespace` and a mutation of a
//!    missing name is its `UnknownNamespace`;
//! 4. on success, increments the generation by exactly one and
//!    appends an [`AuditEntry`] naming the new generation, the
//!    mutation, the namespace and the context that authorized it.
//!
//! A failed mutation changes nothing: not the definitions, not the
//! generation, not the audit log. An update may not change a
//! namespace's core — handlers cannot change core identity
//! (A-9116.1) — that is [`RegistryStateError::CoreChange`]; revoke
//! and re-create instead.
//!
//! # The authorization seam
//!
//! This slice does not define tenancy or authentication. The caller
//! injects a [`MutationContext`] — the principal and the tenant and
//! authorization scopes it acts under, the same scopes the port
//! contract's operation envelope carries (`id_core::port::
//! OperationEnvelope`) under the authenticated tenant/scope context
//! contract (#9083) — and a [`RegistryAuthorizer`] that judges it.
//! [`AdminScopeAuthorizer`] is the shipped judge: it authorizes a
//! mutation exactly when the context's authorization scope equals
//! the registry's configured admin scope. Deployments with a richer
//! authority model implement the trait; the state machine's ordering
//! (authorize, then generation, then apply) never changes.
//!
//! # Persistence
//!
//! A state encodes as one canonical CBOR map (A-9114.3): `v` (state
//! version, 1), `registry_kind` (`server` or `handle`), `registry`,
//! `generation`, `definitions` (record values, as the record slice
//! encodes them) and `audit`. Every key is critical: an unknown key,
//! a missing key or another version rejects. Decoding re-validates
//! every definition through the record layer. Persisting the bytes
//! is the caller's store concern; resolving through the last
//! verified generation is the rollback path.
//!
//! This state is the *local configuration* registry only. The
//! existing Iroh namespace registries are a different mechanism and
//! coexist untouched: nothing here reads or writes them, and no ID
//! minted under them changes meaning.

use std::fmt;

use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use id_core::namespace_definition::{
    CoreKind, LocalRegistry, NamespaceDefinition, NamespaceError, NamespaceId, RegistryId,
};

/// The state version this module reads and writes.
pub const STATE_VERSION: u64 = 1;

/// Longest principal or scope text in a [`MutationContext`].
pub const MAX_CONTEXT_LEN: usize = 255;

const STATE_FIELDS: [&str; 6] = [
    "v",
    "registry_kind",
    "registry",
    "generation",
    "definitions",
    "audit",
];

const AUDIT_FIELDS: [&str; 6] = [
    "generation",
    "mutation",
    "namespace",
    "principal",
    "tenant",
    "scope",
];

/// Which mutation a [`RegistryState`] transition performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MutationKind {
    /// Define a namespace that does not exist yet.
    Create,
    /// Replace the definition of an existing namespace.
    Update,
    /// Remove a namespace's definition.
    Revoke,
}

impl MutationKind {
    /// The mutation token, as written in audit entries and state bytes.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Revoke => "revoke",
        }
    }

    /// The mutation a token names.
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::Invalid`] for any other token.
    pub fn from_token(token: &str) -> Result<Self, RegistryStateError> {
        match token {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "revoke" => Ok(Self::Revoke),
            _ => Err(RegistryStateError::Invalid {
                field: "mutation",
                problem: "is not create, update or revoke",
            }),
        }
    }
}

impl fmt::Display for MutationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// The injected context a mutation is judged and audited under: who
/// acts, and the tenant and authorization scopes they act within.
/// Authentication of the principal is the caller's contract (#9083);
/// this type only requires the texts to be well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationContext {
    principal: String,
    tenant_scope: String,
    authorization_scope: String,
}

fn checked_context(field: &'static str, text: &str) -> Result<(), RegistryStateError> {
    if !text.is_empty()
        && text.len() <= MAX_CONTEXT_LEN
        && text.bytes().all(|byte| byte.is_ascii_graphic())
    {
        Ok(())
    } else {
        Err(RegistryStateError::InvalidContext {
            field,
            problem: "is not 1 to 255 visible ASCII characters",
        })
    }
}

impl MutationContext {
    /// A mutation context.
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::InvalidContext`] when any text is empty,
    /// overlong or not visible ASCII.
    pub fn new(
        principal: impl Into<String>,
        tenant_scope: impl Into<String>,
        authorization_scope: impl Into<String>,
    ) -> Result<Self, RegistryStateError> {
        let principal = principal.into();
        let tenant_scope = tenant_scope.into();
        let authorization_scope = authorization_scope.into();
        checked_context("principal", &principal)?;
        checked_context("tenant_scope", &tenant_scope)?;
        checked_context("authorization_scope", &authorization_scope)?;
        Ok(Self {
            principal,
            tenant_scope,
            authorization_scope,
        })
    }

    /// Who acts.
    #[must_use]
    pub fn principal(&self) -> &str {
        &self.principal
    }

    /// The tenant scope acted within.
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    /// The authorization scope asserted for the caller.
    #[must_use]
    pub fn authorization_scope(&self) -> &str {
        &self.authorization_scope
    }
}

/// The authorization seam: judges whether a [`MutationContext`] may
/// perform a mutation on a registry. Implementations must be
/// fail-closed — any doubt is a refusal — and must not mutate
/// anything themselves.
pub trait RegistryAuthorizer {
    /// Authorizes, or refuses with
    /// [`RegistryStateError::Unauthorized`].
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::Unauthorized`] when the context does not
    /// hold authority over the registry for this mutation.
    fn authorize(
        &self,
        context: &MutationContext,
        registry: &RegistryId,
        mutation: MutationKind,
    ) -> Result<(), RegistryStateError>;
}

/// The shipped authorizer: a mutation is authorized exactly when the
/// context's authorization scope equals the admin scope configured
/// for the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminScopeAuthorizer {
    admin_scope: String,
}

impl AdminScopeAuthorizer {
    /// An authorizer for one admin scope.
    #[must_use]
    pub fn new(admin_scope: impl Into<String>) -> Self {
        Self {
            admin_scope: admin_scope.into(),
        }
    }

    /// The admin scope it requires.
    #[must_use]
    pub fn admin_scope(&self) -> &str {
        &self.admin_scope
    }
}

impl RegistryAuthorizer for AdminScopeAuthorizer {
    fn authorize(
        &self,
        context: &MutationContext,
        registry: &RegistryId,
        mutation: MutationKind,
    ) -> Result<(), RegistryStateError> {
        if context.authorization_scope() == self.admin_scope {
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

/// Why a registry-state operation failed. A failed mutation leaves
/// the state exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryStateError {
    /// The context does not hold authority over this registry.
    Unauthorized {
        /// The registry the mutation targeted.
        registry: RegistryId,
        /// The mutation refused.
        mutation: MutationKind,
        /// The authorization scope the context asserted.
        authorization_scope: String,
    },
    /// The mutation expected another generation: a concurrent
    /// mutation landed first, or the caller is behind.
    GenerationConflict {
        /// The generation the caller expected.
        expected: u64,
        /// The generation the state is actually at.
        actual: u64,
    },
    /// An update tried to move a namespace to the other core.
    CoreChange {
        /// The namespace.
        namespace: NamespaceId,
        /// The core it is defined on.
        expected: CoreKind,
        /// The core the update named.
        found: CoreKind,
    },
    /// A mutation context text is not well formed.
    InvalidContext {
        /// Which field.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// A state or audit value is malformed.
    Invalid {
        /// Which field.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// The state bytes have another version.
    UnsupportedVersion {
        /// The version found.
        version: u64,
    },
    /// The state bytes have a key this reader does not know.
    UnknownField {
        /// The key.
        field: String,
    },
    /// The record layer refused.
    Record(NamespaceError),
    /// The state bytes are not canonical CBOR.
    Cbor(CborError),
}

impl From<NamespaceError> for RegistryStateError {
    fn from(error: NamespaceError) -> Self {
        Self::Record(error)
    }
}

impl From<CborError> for RegistryStateError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl fmt::Display for RegistryStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized {
                registry,
                mutation,
                authorization_scope,
            } => write!(
                f,
                "scope {authorization_scope} holds no {mutation} authority over registry {registry}"
            ),
            Self::GenerationConflict { expected, actual } => {
                write!(f, "expected generation {expected}, state is at {actual}")
            }
            Self::CoreChange {
                namespace,
                expected,
                found,
            } => write!(
                f,
                "namespace {namespace} is defined on core {expected}; an update cannot move it to {found}"
            ),
            Self::InvalidContext { field, problem } => {
                write!(f, "context field {field} {problem}")
            }
            Self::Invalid { field, problem } => write!(f, "field {field} {problem}"),
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported registry state version {version}")
            }
            Self::UnknownField { field } => write!(f, "unknown registry state field {field}"),
            Self::Record(error) => write!(f, "{error}"),
            Self::Cbor(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RegistryStateError {}

/// One audited mutation: the generation it produced, what it did,
/// and the context it was authorized under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// The generation this mutation produced.
    pub generation: u64,
    /// What the mutation did.
    pub mutation: MutationKind,
    /// The namespace it touched.
    pub namespace: NamespaceId,
    /// Who acted.
    pub principal: String,
    /// The tenant scope acted within.
    pub tenant_scope: String,
    /// The authorization scope asserted.
    pub authorization_scope: String,
}

impl AuditEntry {
    fn to_value(&self) -> Value {
        let mut entries = vec![
            (text("generation"), Value::UInt(self.generation)),
            (text("mutation"), text(self.mutation.token())),
            (text("namespace"), text(self.namespace.as_str())),
            (text("principal"), text(&self.principal)),
            (text("tenant"), text(&self.tenant_scope)),
            (text("scope"), text(&self.authorization_scope)),
        ];
        entries.sort_by_cached_key(|(key, _)| encode_canonical(key).unwrap_or_default());
        Value::Map(entries)
    }

    fn from_value(value: &Value) -> Result<Self, RegistryStateError> {
        let fields = field_map(value, &AUDIT_FIELDS)?;
        Ok(Self {
            generation: uint_field(&fields, "generation")?,
            mutation: MutationKind::from_token(string_field(&fields, "mutation")?)?,
            namespace: NamespaceId::new(string_field(&fields, "namespace")?)?,
            principal: string_field(&fields, "principal")?.to_owned(),
            tenant_scope: string_field(&fields, "tenant")?.to_owned(),
            authorization_scope: string_field(&fields, "scope")?.to_owned(),
        })
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// The keys of a state or audit map, every one required and known.
fn field_map<'a>(
    value: &'a Value,
    known: &[&'static str],
) -> Result<std::collections::BTreeMap<&'a str, &'a Value>, RegistryStateError> {
    let Value::Map(entries) = value else {
        return Err(RegistryStateError::Invalid {
            field: "record",
            problem: "is not a map",
        });
    };
    let mut fields = std::collections::BTreeMap::new();
    for (key, value) in entries {
        let Value::Text(key) = key else {
            return Err(RegistryStateError::Invalid {
                field: "record",
                problem: "has a key that is not text",
            });
        };
        if !known.contains(&key.as_str()) {
            return Err(RegistryStateError::UnknownField { field: key.clone() });
        }
        fields.insert(key.as_str(), value);
    }
    for name in known {
        if !fields.contains_key(name) {
            return Err(RegistryStateError::Invalid {
                field: *name,
                problem: "is missing",
            });
        }
    }
    Ok(fields)
}

fn uint_field(
    fields: &std::collections::BTreeMap<&str, &Value>,
    field: &'static str,
) -> Result<u64, RegistryStateError> {
    match fields.get(field) {
        Some(Value::UInt(number)) => Ok(*number),
        _ => Err(RegistryStateError::Invalid {
            field,
            problem: "is not an unsigned integer",
        }),
    }
}

fn string_field<'a>(
    fields: &std::collections::BTreeMap<&'a str, &'a Value>,
    field: &'static str,
) -> Result<&'a str, RegistryStateError> {
    match fields.get(field) {
        Some(Value::Text(text)) => Ok(text.as_str()),
        _ => Err(RegistryStateError::Invalid {
            field,
            problem: "is not text",
        }),
    }
}

/// A local registry's versioned state: its definitions, the
/// generation they are at, and the audit log of how they got there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryState {
    registry: LocalRegistry,
    generation: u64,
    audit: Vec<AuditEntry>,
}

impl RegistryState {
    /// An empty registry state at generation 0.
    #[must_use]
    pub fn new(registry_id: RegistryId) -> Self {
        Self {
            registry: LocalRegistry::new(registry_id),
            generation: 0,
            audit: Vec::new(),
        }
    }

    /// The registry this state versions.
    #[must_use]
    pub fn registry_id(&self) -> &RegistryId {
        self.registry.id()
    }

    /// The definitions, as the record layer holds them.
    #[must_use]
    pub const fn local_registry(&self) -> &LocalRegistry {
        &self.registry
    }

    /// The current generation. Every successful mutation increments
    /// it by exactly one; nothing else changes it.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The definition of `namespace` at the current generation.
    ///
    /// # Errors
    ///
    /// The record's `UnknownNamespace` when this registry does not
    /// define it at this generation.
    pub fn resolve(
        &self,
        namespace: &NamespaceId,
    ) -> Result<&NamespaceDefinition, RegistryStateError> {
        Ok(self.registry.resolve(namespace)?)
    }

    /// Every definition at the current generation, by name.
    pub fn definitions(&self) -> impl Iterator<Item = &NamespaceDefinition> {
        self.registry.definitions()
    }

    /// The audit log, oldest first. One entry per successful
    /// mutation, in generation order.
    #[must_use]
    pub fn audit_log(&self) -> &[AuditEntry] {
        &self.audit
    }

    /// Creates a namespace definition.
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::Unauthorized`], then
    /// [`RegistryStateError::GenerationConflict`], then the record's
    /// `DuplicateNamespace` when the name is taken.
    pub fn create(
        &mut self,
        authorizer: &dyn RegistryAuthorizer,
        context: &MutationContext,
        expected_generation: u64,
        definition: NamespaceDefinition,
    ) -> Result<u64, RegistryStateError> {
        let namespace = definition.namespace.clone();
        self.mutate(
            authorizer,
            context,
            expected_generation,
            MutationKind::Create,
            move |registry| {
                registry.define(definition)?;
                Ok(namespace)
            },
        )
    }

    /// Replaces the definition of an existing namespace. The core
    /// may not change.
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::Unauthorized`], then
    /// [`RegistryStateError::GenerationConflict`], then the record's
    /// `UnknownNamespace` when the name is not defined, then
    /// [`RegistryStateError::CoreChange`] when the replacement names
    /// the other core.
    pub fn update(
        &mut self,
        authorizer: &dyn RegistryAuthorizer,
        context: &MutationContext,
        expected_generation: u64,
        definition: NamespaceDefinition,
    ) -> Result<u64, RegistryStateError> {
        let namespace = definition.namespace.clone();
        self.mutate(
            authorizer,
            context,
            expected_generation,
            MutationKind::Update,
            move |registry| {
                let existing = registry.resolve(&namespace)?;
                if existing.core != definition.core {
                    return Err(RegistryStateError::CoreChange {
                        namespace: namespace.clone(),
                        expected: existing.core,
                        found: definition.core,
                    });
                }
                *registry = rebuilt(registry, Some(&definition), None)?;
                Ok(namespace)
            },
        )
    }

    /// Removes a namespace's definition.
    ///
    /// # Errors
    ///
    /// [`RegistryStateError::Unauthorized`], then
    /// [`RegistryStateError::GenerationConflict`], then the record's
    /// `UnknownNamespace` when the name is not defined.
    pub fn revoke(
        &mut self,
        authorizer: &dyn RegistryAuthorizer,
        context: &MutationContext,
        expected_generation: u64,
        namespace: &NamespaceId,
    ) -> Result<u64, RegistryStateError> {
        let namespace = namespace.clone();
        self.mutate(
            authorizer,
            context,
            expected_generation,
            MutationKind::Revoke,
            move |registry| {
                registry.resolve(&namespace)?;
                *registry = rebuilt(registry, None, Some(&namespace))?;
                Ok(namespace)
            },
        )
    }

    /// The one transition every mutation goes through: authorize,
    /// check the generation, apply, then — only on success — step
    /// the generation once and audit.
    fn mutate(
        &mut self,
        authorizer: &dyn RegistryAuthorizer,
        context: &MutationContext,
        expected_generation: u64,
        mutation: MutationKind,
        apply: impl FnOnce(&mut LocalRegistry) -> Result<NamespaceId, RegistryStateError>,
    ) -> Result<u64, RegistryStateError> {
        authorizer.authorize(context, self.registry.id(), mutation)?;
        if expected_generation != self.generation {
            return Err(RegistryStateError::GenerationConflict {
                expected: expected_generation,
                actual: self.generation,
            });
        }
        let namespace = apply(&mut self.registry)?;
        self.generation += 1;
        self.audit.push(AuditEntry {
            generation: self.generation,
            mutation,
            namespace,
            principal: context.principal().to_owned(),
            tenant_scope: context.tenant_scope().to_owned(),
            authorization_scope: context.authorization_scope().to_owned(),
        });
        Ok(self.generation)
    }

    /// The state as a canonical CBOR value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let (registry_kind, registry_id) = match self.registry.id() {
            RegistryId::Server(id) => ("server", id.as_str()),
            RegistryId::Handle(id) => ("handle", id.as_str()),
        };
        let mut entries = vec![
            (text("v"), Value::UInt(STATE_VERSION)),
            (text("registry_kind"), text(registry_kind)),
            (text("registry"), text(registry_id)),
            (text("generation"), Value::UInt(self.generation)),
            (
                text("definitions"),
                Value::Array(
                    self.definitions()
                        .map(NamespaceDefinition::to_value)
                        .collect(),
                ),
            ),
            (
                text("audit"),
                Value::Array(self.audit.iter().map(AuditEntry::to_value).collect()),
            ),
        ];
        entries.sort_by_cached_key(|(key, _)| encode_canonical(key).unwrap_or_default());
        Value::Map(entries)
    }

    /// The canonical state bytes.
    ///
    /// # Errors
    ///
    /// None in practice; the value model is closed.
    pub fn encode(&self) -> Result<Vec<u8>, RegistryStateError> {
        Ok(encode_canonical(&self.to_value())?)
    }

    /// Reads a state from canonical bytes.
    ///
    /// # Errors
    ///
    /// Non-canonical bytes, an unknown or missing field, another
    /// version, or a definition or audit entry the record layer
    /// refuses.
    pub fn decode(bytes: &[u8]) -> Result<Self, RegistryStateError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a state from a decoded value.
    ///
    /// # Errors
    ///
    /// As [`RegistryState::decode`].
    pub fn from_value(value: &Value) -> Result<Self, RegistryStateError> {
        let fields = field_map(value, &STATE_FIELDS)?;
        let version = uint_field(&fields, "v")?;
        if version != STATE_VERSION {
            return Err(RegistryStateError::UnsupportedVersion { version });
        }
        let registry_id = match string_field(&fields, "registry_kind")? {
            "server" => RegistryId::server(string_field(&fields, "registry")?)?,
            "handle" => RegistryId::handle(string_field(&fields, "registry")?)?,
            _ => {
                return Err(RegistryStateError::Invalid {
                    field: "registry_kind",
                    problem: "is not server or handle",
                });
            }
        };
        let mut registry = LocalRegistry::new(registry_id);
        match fields.get("definitions") {
            Some(Value::Array(definitions)) => {
                for definition in definitions {
                    registry.define(NamespaceDefinition::from_value(definition)?)?;
                }
            }
            _ => {
                return Err(RegistryStateError::Invalid {
                    field: "definitions",
                    problem: "is not an array",
                });
            }
        }
        let mut audit = Vec::new();
        match fields.get("audit") {
            Some(Value::Array(entries)) => {
                for entry in entries {
                    audit.push(AuditEntry::from_value(entry)?);
                }
            }
            _ => {
                return Err(RegistryStateError::Invalid {
                    field: "audit",
                    problem: "is not an array",
                });
            }
        }
        Ok(Self {
            registry,
            generation: uint_field(&fields, "generation")?,
            audit,
        })
    }
}

/// A registry with one definition replaced and/or one removed,
/// rebuilt through the record layer so its invariants (unique
/// names, validated records) are re-checked on the way in.
fn rebuilt(
    registry: &LocalRegistry,
    replace: Option<&NamespaceDefinition>,
    remove: Option<&NamespaceId>,
) -> Result<LocalRegistry, RegistryStateError> {
    let mut next = LocalRegistry::new(registry.id().clone());
    for definition in registry.definitions() {
        if Some(&definition.namespace) == remove {
            continue;
        }
        if let Some(replacement) = replace.filter(|r| r.namespace == definition.namespace) {
            next.define(replacement.clone())?;
        } else {
            next.define(definition.clone())?;
        }
    }
    Ok(next)
}
