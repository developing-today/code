//! Local namespace registry record and the two-core constraint (slice
//! A9116-registry-record, issue #10243; parent #9116; design
//! `part-a-addressing/versioned-design.md` section A-9116.1, statements
//! S4.1–S4.7 and S4.19–S4.22).
//!
//! A [`NamespaceDefinition`] binds a stable local [`NamespaceId`] to
//! exactly one of the two address cores and to explicit handler and
//! configuration profiles:
//! `{core, handler_id, encoding_profile, read_policy, write_policy,
//! shorthand_policy, schema_version}`. Nothing has a default: every
//! field is named in the record.
//!
//! # Exactly two cores
//!
//! [`CoreKind`] has two variants, the immutable content-addressed core
//! (#9120) and the mutable array-addressed core (#9118). Names, numeric
//! registries, streams and signed worlds are derived namespaces, lenses
//! over one of these, never a third core. A record or shape that names
//! anything else, the derived shape included, is
//! [`NamespaceError::UnknownCore`]. A handler receives a typed
//! [`CoreAddress`], and [`NamespaceDefinition::admit`] refuses an
//! address of the other core.
//!
//! # Local registries, explicit context
//!
//! A [`LocalRegistry`] is one server's set of definitions, identified by
//! a [`RegistryId`] (a server or an explicit registry handle). The same
//! namespace name may be defined differently in different registries.
//! Resolution always names the registry to use: [`Resolver::resolve`]
//! takes a [`RegistryId`], and a name missing from that registry is
//! [`NamespaceError::UnknownNamespace`] even when another registry
//! defines it. There is no global registry and no fallback.
//!
//! Generations, authorization and audit of registry mutations belong to
//! slice A9116-registry-state.
//!
//! # Record encoding
//!
//! A definition encodes as a canonical CBOR map (A-9114.3) with text
//! keys `v` (record version, 1), `namespace`, `core` (`content` or
//! `array`), `handler`, `encoding`, `read`, `write`, `shorthand` and
//! `schema` (unsigned). Every key is critical: an unknown key, a missing
//! key or another version rejects.
//!
//! The crate-root export of this module belongs to slice 9110-02; until
//! that lands the integration test includes it, and the modules it uses,
//! by path.

use std::collections::BTreeMap;
use std::fmt;

use crate::array_address::ArrayAddress;
use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::id_shape::{ContentId, IdShape};

/// The record version this module reads and writes.
pub const RECORD_VERSION: u64 = 1;

/// Longest namespace name, handler, profile or policy token.
pub const MAX_TOKEN_LEN: usize = 63;

/// Longest registry identifier.
pub const MAX_REGISTRY_ID_LEN: usize = 255;

const FIELDS: [&str; 9] = [
    "v",
    "namespace",
    "core",
    "handler",
    "encoding",
    "read",
    "write",
    "shorthand",
    "schema",
];

/// Why a definition, registry or resolution failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceError {
    /// A name or token is not well formed.
    Invalid {
        /// Which field.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// A core other than `content` or `array` was named.
    UnknownCore {
        /// The core as written.
        core: String,
    },
    /// The record bytes are not canonical CBOR.
    Cbor(CborError),
    /// The record has another version.
    UnsupportedVersion {
        /// The version found.
        version: u64,
    },
    /// The record has a key this reader does not know.
    UnknownField {
        /// The key.
        field: String,
    },
    /// A core address of the other core was given to a definition.
    CoreMismatch {
        /// The definition's core.
        expected: CoreKind,
        /// The address's core.
        found: CoreKind,
    },
    /// The registry already defines this namespace.
    DuplicateNamespace {
        /// The namespace.
        namespace: NamespaceId,
    },
    /// The resolver already has a registry with this identity.
    DuplicateRegistry {
        /// The registry.
        registry: RegistryId,
    },
    /// The resolver has no registry with this identity.
    UnknownRegistry {
        /// The registry asked for.
        registry: RegistryId,
    },
    /// The registry does not define this namespace.
    UnknownNamespace {
        /// The registry searched.
        registry: RegistryId,
        /// The namespace asked for.
        namespace: NamespaceId,
    },
}

impl NamespaceError {
    /// A stable failure class: `malformed`, `unknown_core`,
    /// `unsupported_version`, `unknown_field`, `core_mismatch`,
    /// `duplicate`, `unknown_registry` or `unknown_namespace`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Invalid { .. } | Self::Cbor(_) => "malformed",
            Self::UnknownCore { .. } => "unknown_core",
            Self::UnsupportedVersion { .. } => "unsupported_version",
            Self::UnknownField { .. } => "unknown_field",
            Self::CoreMismatch { .. } => "core_mismatch",
            Self::DuplicateNamespace { .. } | Self::DuplicateRegistry { .. } => "duplicate",
            Self::UnknownRegistry { .. } => "unknown_registry",
            Self::UnknownNamespace { .. } => "unknown_namespace",
        }
    }
}

impl fmt::Display for NamespaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { field, problem } => write!(f, "namespace {field} {problem}"),
            Self::UnknownCore { core } => write!(
                f,
                "{core:?} is not an address core; the cores are content and array"
            ),
            Self::Cbor(error) => write!(f, "namespace record is not canonical CBOR: {error:?}"),
            Self::UnsupportedVersion { version } => {
                write!(f, "namespace record version {version} is not supported")
            }
            Self::UnknownField { field } => {
                write!(f, "namespace record has unknown field {field:?}")
            }
            Self::CoreMismatch { expected, found } => write!(
                f,
                "namespace is on the {expected} core but the address is on the {found} core"
            ),
            Self::DuplicateNamespace { namespace } => {
                write!(
                    f,
                    "namespace {namespace} is already defined in this registry"
                )
            }
            Self::DuplicateRegistry { registry } => {
                write!(f, "registry {registry} is already known")
            }
            Self::UnknownRegistry { registry } => write!(f, "no registry {registry}"),
            Self::UnknownNamespace {
                registry,
                namespace,
            } => write!(
                f,
                "registry {registry} does not define namespace {namespace}"
            ),
        }
    }
}

impl std::error::Error for NamespaceError {}

impl From<CborError> for NamespaceError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

/// One of the two address cores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CoreKind {
    /// Immutable content-addressed objects (#9120).
    Content,
    /// Mutable array-addressed records (#9118).
    Array,
}

impl CoreKind {
    /// Both cores. There is no third.
    pub const ALL: [Self; 2] = [Self::Content, Self::Array];

    /// The core token, the same as its ID shape token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Array => "array",
        }
    }

    /// The core a token names.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::UnknownCore`] for anything but `content` or
    /// `array`.
    pub fn from_token(token: &str) -> Result<Self, NamespaceError> {
        match token {
            "content" => Ok(Self::Content),
            "array" => Ok(Self::Array),
            other => Err(NamespaceError::UnknownCore {
                core: other.to_owned(),
            }),
        }
    }

    /// The core an ID shape belongs to.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::UnknownCore`] for the derived shape: a derived
    /// namespace is a lens over a core, not a core.
    pub fn from_shape(shape: IdShape) -> Result<Self, NamespaceError> {
        match shape {
            IdShape::Content => Ok(Self::Content),
            IdShape::Array => Ok(Self::Array),
            IdShape::Derived => Err(NamespaceError::UnknownCore {
                core: shape.token().to_owned(),
            }),
        }
    }

    /// The ID shape of addresses on this core.
    #[must_use]
    pub const fn shape(self) -> IdShape {
        match self {
            Self::Content => IdShape::Content,
            Self::Array => IdShape::Array,
        }
    }
}

impl fmt::Display for CoreKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// A typed address on one of the two cores, as a handler receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreAddress {
    /// A content ID.
    Content(ContentId),
    /// An array address.
    Array(ArrayAddress),
}

impl CoreAddress {
    /// The core this address is on.
    #[must_use]
    pub const fn core(&self) -> CoreKind {
        match self {
            Self::Content(_) => CoreKind::Content,
            Self::Array(_) => CoreKind::Array,
        }
    }
}

fn is_token(text: &str, first: fn(u8) -> bool, rest: fn(u8) -> bool, max: usize) -> bool {
    let bytes = text.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= max
        && first(bytes[0])
        && bytes[1..].iter().all(|&byte| rest(byte))
}

const fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

const fn is_name_rest(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

const fn is_token_start(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

const fn is_token_rest(byte: u8) -> bool {
    is_token_start(byte) || matches!(byte, b'-' | b'_' | b'.')
}

const fn is_registry_byte(byte: u8) -> bool {
    byte.is_ascii_graphic()
}

/// A stable namespace name, local to one registry:
/// `ALPHA *( ALPHA / DIGIT / "-" )`, at most [`MAX_TOKEN_LEN`] bytes,
/// compared exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamespaceId(String);

impl NamespaceId {
    /// A namespace name.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::Invalid`] when it is not well formed.
    pub fn new(name: impl Into<String>) -> Result<Self, NamespaceError> {
        let name = name.into();
        if is_token(&name, is_name_start, is_name_rest, MAX_TOKEN_LEN) {
            Ok(Self(name))
        } else {
            Err(NamespaceError::Invalid {
                field: "namespace",
                problem: "is not a letter followed by letters, digits or hyphens",
            })
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NamespaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A handler, profile or policy name: lowercase ASCII letters and
/// digits, then also `-`, `_` or `.`, at most [`MAX_TOKEN_LEN`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Token(String);

impl Token {
    /// A token for `field`.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::Invalid`] when it is not well formed.
    pub fn new(field: &'static str, text: impl Into<String>) -> Result<Self, NamespaceError> {
        let text = text.into();
        if is_token(&text, is_token_start, is_token_rest, MAX_TOKEN_LEN) {
            Ok(Self(text))
        } else {
            Err(NamespaceError::Invalid {
                field,
                problem: "is not a lowercase token",
            })
        }
    }

    /// The token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The resolver context: which local registry to resolve in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegistryId {
    /// The registry of a server, by server ID.
    Server(String),
    /// An explicit registry handle.
    Handle(String),
}

impl RegistryId {
    fn checked(text: &str) -> Result<(), NamespaceError> {
        if is_token(
            text,
            is_registry_byte,
            is_registry_byte,
            MAX_REGISTRY_ID_LEN,
        ) {
            Ok(())
        } else {
            Err(NamespaceError::Invalid {
                field: "registry",
                problem: "is not 1 to 255 visible ASCII characters",
            })
        }
    }

    /// A server's registry.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::Invalid`] for an empty or non-printable ID.
    pub fn server(server_id: impl Into<String>) -> Result<Self, NamespaceError> {
        let server_id = server_id.into();
        Self::checked(&server_id)?;
        Ok(Self::Server(server_id))
    }

    /// An explicit registry handle.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::Invalid`] for an empty or non-printable handle.
    pub fn handle(handle: impl Into<String>) -> Result<Self, NamespaceError> {
        let handle = handle.into();
        Self::checked(&handle)?;
        Ok(Self::Handle(handle))
    }
}

impl fmt::Display for RegistryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Server(id) => write!(f, "server:{id}"),
            Self::Handle(id) => write!(f, "handle:{id}"),
        }
    }
}

/// One namespace definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDefinition {
    /// The local namespace name.
    pub namespace: NamespaceId,
    /// The core its addresses are on.
    pub core: CoreKind,
    /// The handler that serves it.
    pub handler: Token,
    /// How addresses are written for it.
    pub encoding_profile: Token,
    /// Who may read.
    pub read_policy: Token,
    /// Who may write.
    pub write_policy: Token,
    /// Which shorthand forms it accepts.
    pub shorthand_policy: Token,
    /// The handler's schema version.
    pub schema_version: u64,
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

impl NamespaceDefinition {
    /// Hands `address` on only when it is on this namespace's core.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::CoreMismatch`] for an address on the other core.
    pub fn admit(&self, address: CoreAddress) -> Result<CoreAddress, NamespaceError> {
        if address.core() == self.core {
            Ok(address)
        } else {
            Err(NamespaceError::CoreMismatch {
                expected: self.core,
                found: address.core(),
            })
        }
    }

    /// The record as a canonical CBOR value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut entries = vec![
            (text("v"), Value::UInt(RECORD_VERSION)),
            (text("namespace"), text(self.namespace.as_str())),
            (text("core"), text(self.core.token())),
            (text("handler"), text(self.handler.as_str())),
            (text("encoding"), text(self.encoding_profile.as_str())),
            (text("read"), text(self.read_policy.as_str())),
            (text("write"), text(self.write_policy.as_str())),
            (text("shorthand"), text(self.shorthand_policy.as_str())),
            (text("schema"), Value::UInt(self.schema_version)),
        ];
        entries.sort_by_cached_key(|(key, _)| encode_canonical(key).unwrap_or_default());
        Value::Map(entries)
    }

    /// The canonical record bytes.
    ///
    /// # Errors
    ///
    /// None in practice; the value model is closed.
    pub fn encode(&self) -> Result<Vec<u8>, NamespaceError> {
        Ok(encode_canonical(&self.to_value())?)
    }

    /// Reads a record from canonical bytes.
    ///
    /// # Errors
    ///
    /// Non-canonical bytes, an unknown or missing field, another
    /// version, a malformed token or an unknown core.
    pub fn decode(bytes: &[u8]) -> Result<Self, NamespaceError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a record from a decoded value.
    ///
    /// # Errors
    ///
    /// As [`NamespaceDefinition::decode`].
    pub fn from_value(value: &Value) -> Result<Self, NamespaceError> {
        let Value::Map(entries) = value else {
            return Err(NamespaceError::Invalid {
                field: "record",
                problem: "is not a map",
            });
        };
        let mut fields: BTreeMap<&str, &Value> = BTreeMap::new();
        for (key, value) in entries {
            let Value::Text(key) = key else {
                return Err(NamespaceError::Invalid {
                    field: "record",
                    problem: "has a key that is not text",
                });
            };
            if !FIELDS.contains(&key.as_str()) {
                return Err(NamespaceError::UnknownField { field: key.clone() });
            }
            fields.insert(key.as_str(), value);
        }
        let uint = |field: &'static str| match fields.get(field) {
            Some(Value::UInt(number)) => Ok(*number),
            Some(_) => Err(NamespaceError::Invalid {
                field,
                problem: "is not an unsigned integer",
            }),
            None => Err(NamespaceError::Invalid {
                field,
                problem: "is missing",
            }),
        };
        let string = |field: &'static str| match fields.get(field) {
            Some(Value::Text(text)) => Ok(text.as_str()),
            Some(_) => Err(NamespaceError::Invalid {
                field,
                problem: "is not text",
            }),
            None => Err(NamespaceError::Invalid {
                field,
                problem: "is missing",
            }),
        };
        let version = uint("v")?;
        if version != RECORD_VERSION {
            return Err(NamespaceError::UnsupportedVersion { version });
        }
        Ok(Self {
            namespace: NamespaceId::new(string("namespace")?)?,
            core: CoreKind::from_token(string("core")?)?,
            handler: Token::new("handler", string("handler")?)?,
            encoding_profile: Token::new("encoding", string("encoding")?)?,
            read_policy: Token::new("read", string("read")?)?,
            write_policy: Token::new("write", string("write")?)?,
            shorthand_policy: Token::new("shorthand", string("shorthand")?)?,
            schema_version: uint("schema")?,
        })
    }
}

/// One server's namespace definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRegistry {
    id: RegistryId,
    definitions: BTreeMap<NamespaceId, NamespaceDefinition>,
}

impl LocalRegistry {
    /// An empty registry.
    #[must_use]
    pub const fn new(id: RegistryId) -> Self {
        Self {
            id,
            definitions: BTreeMap::new(),
        }
    }

    /// Its identity.
    #[must_use]
    pub const fn id(&self) -> &RegistryId {
        &self.id
    }

    /// Adds a definition.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::DuplicateNamespace`] when the name is taken.
    pub fn define(&mut self, definition: NamespaceDefinition) -> Result<(), NamespaceError> {
        if self.definitions.contains_key(&definition.namespace) {
            return Err(NamespaceError::DuplicateNamespace {
                namespace: definition.namespace,
            });
        }
        self.definitions
            .insert(definition.namespace.clone(), definition);
        Ok(())
    }

    /// The definition of `namespace` in this registry.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::UnknownNamespace`] when this registry does not
    /// define it.
    pub fn resolve(&self, namespace: &NamespaceId) -> Result<&NamespaceDefinition, NamespaceError> {
        self.definitions
            .get(namespace)
            .ok_or_else(|| NamespaceError::UnknownNamespace {
                registry: self.id.clone(),
                namespace: namespace.clone(),
            })
    }

    /// Every definition, by name.
    pub fn definitions(&self) -> impl Iterator<Item = &NamespaceDefinition> {
        self.definitions.values()
    }
}

/// The local registries a process knows, each reached only by naming it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolver {
    registries: BTreeMap<RegistryId, LocalRegistry>,
}

impl Resolver {
    /// A resolver with no registries.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            registries: BTreeMap::new(),
        }
    }

    /// Adds a registry.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::DuplicateRegistry`] when one with its identity
    /// is already known.
    pub fn add(&mut self, registry: LocalRegistry) -> Result<(), NamespaceError> {
        if self.registries.contains_key(registry.id()) {
            return Err(NamespaceError::DuplicateRegistry {
                registry: registry.id().clone(),
            });
        }
        self.registries.insert(registry.id().clone(), registry);
        Ok(())
    }

    /// The registry `context` names.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::UnknownRegistry`].
    pub fn registry(&self, context: &RegistryId) -> Result<&LocalRegistry, NamespaceError> {
        self.registries
            .get(context)
            .ok_or_else(|| NamespaceError::UnknownRegistry {
                registry: context.clone(),
            })
    }

    /// Resolves `namespace` in the registry `context` names, and only
    /// there.
    ///
    /// # Errors
    ///
    /// [`NamespaceError::UnknownRegistry`] or
    /// [`NamespaceError::UnknownNamespace`]; no other registry is
    /// consulted.
    pub fn resolve(
        &self,
        context: &RegistryId,
        namespace: &NamespaceId,
    ) -> Result<&NamespaceDefinition, NamespaceError> {
        self.registry(context)?.resolve(namespace)
    }
}
