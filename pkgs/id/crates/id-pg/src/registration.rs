//! Endpoint registration and owner configuration (design #5805, §2 and
//! §4; slice #13758).
//!
//! An endpoint is usable only after [`register`] accepts it. Registration
//! is two checks, both pure so they can be tested without a network:
//!
//! 1. **The owner's configuration** ([`EndpointConfig::validate`]): the
//!    source alias and the exposed schema are plain names, the base URL is
//!    HTTPS (plain `http://` only on a loopback host, §2) and carries no
//!    user info, query, or fragment, the credential reference is set, and
//!    every collection allowlist passes
//!    [`CollectionAllowlist::validate`] and names the exposed schema. This
//!    check needs nothing from the source, so a transport can run it before
//!    it sends a credential anywhere.
//! 2. **The live endpoint**, as reported by an [`EndpointProbe`] that the
//!    transport fills (#13763): the `PostgREST` `db-schemas` setting must
//!    list exactly the one exposed schema (§2, §4 layer 3), and the
//!    credential's role may hold nothing on that schema beyond `SELECT` on
//!    its relations and `USAGE` on the schema itself — `INSERT`,
//!    `UPDATE`, `DELETE`, `EXECUTE`, or any other privilege is a
//!    registration error (§4 layer 2, §9 invariant 9).
//!
//! A [`RegisteredEndpoint`] can only be built by [`register`], so holding
//! one is proof that both checks passed. Registration errors are for the
//! owner; none of them carries the credential reference.

use std::collections::BTreeSet;
use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::allowlist::{CollectionAllowlist, is_plain_identifier};
use crate::translate::Translator;

/// The owner's local configuration for one Postgres-over-HTTP endpoint.
///
/// It stays in local endpoint configuration and never goes into shared
/// provenance (§2): the ID-level reference
/// `id://foreign/postgres/<alias>/<schema>.<relation>` carries only the
/// alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    /// The source alias used in `id://foreign/postgres/<alias>/...`.
    pub alias: String,
    /// The `PostgREST` base URL. HTTPS, or plain HTTP on a loopback host.
    pub base_url: String,
    /// The one schema the endpoint exposes, sent as `Accept-Profile`.
    pub schema: String,
    /// A reference to the JWT or API key, resolved just before each
    /// request. Never the secret itself, and never placed in a URL, log,
    /// or error.
    pub credential_ref: String,
    /// The owner's collection allowlists. Each must name `schema`.
    pub collections: Vec<CollectionAllowlist>,
}

/// One kind of Postgres object a privilege can be granted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// The exposed schema itself.
    Schema,
    /// A table, view, or materialized view in the exposed schema.
    Relation,
    /// A function or procedure in the exposed schema.
    Function,
    /// A sequence in the exposed schema.
    Sequence,
    /// Any other object (a type, a domain, a large object, ...).
    Other,
}

/// A Postgres privilege, as reported by the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Privilege {
    /// `SELECT`.
    Select,
    /// `INSERT`.
    Insert,
    /// `UPDATE`.
    Update,
    /// `DELETE`.
    Delete,
    /// `TRUNCATE`.
    Truncate,
    /// `REFERENCES`.
    References,
    /// `TRIGGER`.
    Trigger,
    /// `EXECUTE`.
    Execute,
    /// `USAGE`.
    Usage,
    /// `CREATE`.
    Create,
    /// `MAINTAIN`.
    Maintain,
}

impl Privilege {
    /// The privilege's SQL keyword.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::References => "REFERENCES",
            Self::Trigger => "TRIGGER",
            Self::Execute => "EXECUTE",
            Self::Usage => "USAGE",
            Self::Create => "CREATE",
            Self::Maintain => "MAINTAIN",
        }
    }
}

/// One privilege the credential's role holds on an object in the exposed
/// schema, directly or through a role it inherits or `PUBLIC`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleGrant {
    /// What kind of object the grant is on.
    pub kind: ObjectKind,
    /// The object's name within the exposed schema (the schema's own name
    /// for [`ObjectKind::Schema`]).
    pub object: String,
    /// The privilege held.
    pub privilege: Privilege,
}

/// What the transport read from the live endpoint at registration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointProbe {
    /// The schemas the `PostgREST` endpoint serves (its `db-schemas`
    /// setting, or the schema list its schema probe returns).
    pub db_schemas: Vec<String>,
    /// Every privilege the credential's role holds on the exposed schema
    /// and the objects in it.
    pub role_grants: Vec<RoleGrant>,
}

/// Why registration refused an endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The source alias is empty or has characters other than ASCII
    /// letters, digits, `-`, and `_`.
    InvalidAlias,
    /// The exposed schema is not a plain identifier.
    InvalidSchema,
    /// The base URL is neither HTTPS nor plain HTTP on a loopback host.
    InsecureBaseUrl,
    /// The base URL cannot be parsed as `scheme://host[:port][/path]`.
    MalformedBaseUrl(&'static str),
    /// The base URL carries user info, which would put a credential in a
    /// URL (§9 invariant 4).
    BaseUrlUserinfo,
    /// The base URL carries a query or a fragment.
    BaseUrlQueryOrFragment,
    /// The credential reference is empty.
    MissingCredentialRef,
    /// No collection is configured.
    NoCollections,
    /// A collection allowlist failed [`CollectionAllowlist::validate`].
    InvalidCollection {
        /// The collection's relation name.
        relation: String,
        /// The validation failure.
        reason: String,
    },
    /// A collection names a schema other than the exposed one.
    CollectionSchemaMismatch {
        /// The collection's relation name.
        relation: String,
    },
    /// The same relation is configured twice.
    DuplicateCollection {
        /// The repeated relation name.
        relation: String,
    },
    /// A collection declares a row cap of zero.
    ZeroLimit {
        /// The collection's relation name.
        relation: String,
    },
    /// The endpoint does not serve the exposed schema at all.
    ExposedSchemaNotServed,
    /// The endpoint serves schemas besides the exposed one (§9
    /// invariant 9).
    ExtraSchemas {
        /// The extra schemas, sorted.
        schemas: Vec<String>,
    },
    /// The role holds a privilege beyond read access (§4, §9 invariant 9).
    ForbiddenPrivilege {
        /// The object the privilege is on.
        object: String,
        /// The privilege.
        privilege: Privilege,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAlias => {
                f.write_str("source alias must be non-empty ASCII letters, digits, '-' or '_'")
            }
            Self::InvalidSchema => f.write_str("exposed schema is not a plain identifier"),
            Self::InsecureBaseUrl => {
                f.write_str("base URL must be https, or plain http only on a loopback host")
            }
            Self::MalformedBaseUrl(why) => write!(f, "base URL is malformed: {why}"),
            Self::BaseUrlUserinfo => {
                f.write_str("base URL must not carry user info; use credential_ref")
            }
            Self::BaseUrlQueryOrFragment => {
                f.write_str("base URL must not carry a query or a fragment")
            }
            Self::MissingCredentialRef => f.write_str("credential_ref is empty"),
            Self::NoCollections => f.write_str("no collection is configured"),
            Self::InvalidCollection { relation, reason } => {
                write!(f, "collection {relation:?} is invalid: {reason}")
            }
            Self::CollectionSchemaMismatch { relation } => write!(
                f,
                "collection {relation:?} names a schema other than the exposed schema"
            ),
            Self::DuplicateCollection { relation } => {
                write!(f, "collection {relation:?} is configured more than once")
            }
            Self::ZeroLimit { relation } => {
                write!(f, "collection {relation:?} declares a row cap of 0")
            }
            Self::ExposedSchemaNotServed => {
                f.write_str("the endpoint's db-schemas does not list the exposed schema")
            }
            Self::ExtraSchemas { schemas } => write!(
                f,
                "the endpoint's db-schemas lists schemas besides the exposed one: {}",
                schemas.join(", ")
            ),
            Self::ForbiddenPrivilege { object, privilege } => write!(
                f,
                "the role holds {} on {object:?}; only SELECT on relations and USAGE on the schema are allowed",
                privilege.name()
            ),
        }
    }
}

impl std::error::Error for RegistrationError {}

/// Whether `alias` is a usable source alias for an `id://` reference.
fn is_valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether `host` (as written in a URL authority) is a loopback host.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Check the base URL's transport rules (§2) without a URL parser.
fn check_base_url(url: &str) -> Result<(), RegistrationError> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(RegistrationError::MalformedBaseUrl(
            "contains whitespace or control characters",
        ));
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or(RegistrationError::MalformedBaseUrl("missing scheme"))?;
    let https = scheme.eq_ignore_ascii_case("https");
    if !https && !scheme.eq_ignore_ascii_case("http") {
        return Err(RegistrationError::InsecureBaseUrl);
    }
    if rest.contains('?') || rest.contains('#') {
        return Err(RegistrationError::BaseUrlQueryOrFragment);
    }
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return Err(RegistrationError::BaseUrlUserinfo);
    }
    let (host, port) = if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or(RegistrationError::MalformedBaseUrl("unclosed IPv6 host"))?;
        let (host, after) = authority.split_at(end + 1);
        match after.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if after.is_empty() => (host, None),
            None => {
                return Err(RegistrationError::MalformedBaseUrl(
                    "unexpected text after IPv6 host",
                ));
            }
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err(RegistrationError::MalformedBaseUrl("missing host"));
    }
    if let Some(port) = port
        && port.parse::<u16>().is_err()
    {
        return Err(RegistrationError::MalformedBaseUrl("invalid port"));
    }
    if !https && !is_loopback_host(host) {
        return Err(RegistrationError::InsecureBaseUrl);
    }
    Ok(())
}

impl EndpointConfig {
    /// Check the owner's configuration on its own, before anything is
    /// sent to the endpoint.
    pub fn validate(&self) -> Result<(), RegistrationError> {
        if !is_valid_alias(&self.alias) {
            return Err(RegistrationError::InvalidAlias);
        }
        if !is_plain_identifier(&self.schema) {
            return Err(RegistrationError::InvalidSchema);
        }
        check_base_url(&self.base_url)?;
        if self.credential_ref.trim().is_empty() {
            return Err(RegistrationError::MissingCredentialRef);
        }
        if self.collections.is_empty() {
            return Err(RegistrationError::NoCollections);
        }
        let mut seen = BTreeSet::new();
        for collection in &self.collections {
            collection
                .validate()
                .map_err(|reason| RegistrationError::InvalidCollection {
                    relation: collection.relation.clone(),
                    reason,
                })?;
            if collection.schema != self.schema {
                return Err(RegistrationError::CollectionSchemaMismatch {
                    relation: collection.relation.clone(),
                });
            }
            if collection.max_limit == Some(0) {
                return Err(RegistrationError::ZeroLimit {
                    relation: collection.relation.clone(),
                });
            }
            if !seen.insert(collection.relation.as_str()) {
                return Err(RegistrationError::DuplicateCollection {
                    relation: collection.relation.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Check what the probe read from the live endpoint against the exposed
/// schema.
fn check_probe(schema: &str, probe: &EndpointProbe) -> Result<(), RegistrationError> {
    if !probe.db_schemas.iter().any(|s| s == schema) {
        return Err(RegistrationError::ExposedSchemaNotServed);
    }
    let extra: BTreeSet<&str> = probe
        .db_schemas
        .iter()
        .map(String::as_str)
        .filter(|s| *s != schema)
        .collect();
    if !extra.is_empty() {
        return Err(RegistrationError::ExtraSchemas {
            schemas: extra.into_iter().map(str::to_owned).collect(),
        });
    }
    for grant in &probe.role_grants {
        let allowed = matches!(
            (grant.kind, grant.privilege),
            (ObjectKind::Relation, Privilege::Select) | (ObjectKind::Schema, Privilege::Usage)
        );
        if !allowed {
            return Err(RegistrationError::ForbiddenPrivilege {
                object: grant.object.clone(),
                privilege: grant.privilege,
            });
        }
    }
    Ok(())
}

/// An endpoint that passed registration. Only [`register`] builds one.
#[derive(Debug, Clone)]
pub struct RegisteredEndpoint {
    config: EndpointConfig,
}

impl RegisteredEndpoint {
    /// The source alias.
    pub const fn alias(&self) -> &str {
        self.config.alias.as_str()
    }

    /// The `PostgREST` base URL.
    pub const fn base_url(&self) -> &str {
        self.config.base_url.as_str()
    }

    /// The exposed schema the registration checked; the only value ever
    /// sent as `Accept-Profile` (§9 invariant 2).
    pub const fn schema(&self) -> &str {
        self.config.schema.as_str()
    }

    /// The credential reference, for the transport to resolve just before
    /// each request.
    pub const fn credential_ref(&self) -> &str {
        self.config.credential_ref.as_str()
    }

    /// The registered collection allowlists.
    pub const fn collections(&self) -> &[CollectionAllowlist] {
        self.config.collections.as_slice()
    }

    /// A query translator over the registered collections.
    pub fn translator(&self) -> Translator {
        Translator::new(self.config.collections.clone())
    }
}

/// Register an endpoint: check the owner's configuration, then check what
/// the probe read from the live endpoint. Every refusal happens here, so a
/// misconfigured endpoint never serves a query.
pub fn register(
    config: EndpointConfig,
    probe: &EndpointProbe,
) -> Result<RegisteredEndpoint, RegistrationError> {
    config.validate()?;
    check_probe(&config.schema, probe)?;
    Ok(RegisteredEndpoint { config })
}
