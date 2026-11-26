//! The namespace plane of the config document: `namespaces`.
//!
//! Sixth part of slice 9111-03 in the #9111 record: §3.1 (`namespaces` is
//! "a reference to the server-local namespace registry (#9116). Default:
//! built-in registry only.") and §7.4 (the namespace knobs sit in the same
//! document as the peer and link knobs, in their own section).
//!
//! Config only chooses *which* local registry this server resolves names
//! in. The registry's contents (each namespace's core, handler, encoding,
//! read and write policies, `shorthand_policy`, schema version) are signed
//! registry records owned by #9116, with their own generations and audit,
//! and are never written here. There is no global registry and no
//! fallback (#9116): a name the chosen registry lacks is unknown, even
//! when another registry defines it.
//!
//! The reference is one string, written the way #9116's `RegistryId`
//! displays itself:
//!
//! - `builtin`: the built-in registry only (the default);
//! - `server:<id>`: the registry of the server with that ID;
//! - `handle:<handle>`: an explicit registry handle.
//!
//! The ID or handle after the colon is 1 to 255 visible ASCII characters,
//! the same rule #9116 applies to a registry ID. An absent key and `null`
//! both take the default (§6.3), unknown keys fail closed (§6.4), and a
//! document without `namespaces`, or one that restates the default, keeps
//! its digest.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ConfigError;

/// The registry reference that names the built-in registry.
pub const BUILTIN_NAMESPACE_REGISTRY: &str = "builtin";

/// The longest registry ID or handle (the #9116 registry-ID limit).
pub const MAX_NAMESPACE_REGISTRY_ID_LEN: usize = 255;

/// `namespaces` as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Namespaces {
    /// Which local registry names resolve in. Default `builtin`.
    #[serde(default)]
    pub registry: Option<String>,
}

/// Which local namespace registry a server resolves names in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RegistryRef {
    /// The built-in registry only.
    Builtin,
    /// The registry of a server, by server ID.
    Server(String),
    /// An explicit registry handle.
    Handle(String),
}

impl RegistryRef {
    /// Parse a written registry reference.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidNamespaceRegistry`] for anything other than
    /// `builtin`, `server:<id>` or `handle:<handle>` with a valid ID.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let invalid = || ConfigError::InvalidNamespaceRegistry(text.to_owned());
        if text == BUILTIN_NAMESPACE_REGISTRY {
            return Ok(Self::Builtin);
        }
        let (kind, id) = text.split_once(':').ok_or_else(invalid)?;
        if !is_registry_id(id) {
            return Err(invalid());
        }
        match kind {
            "server" => Ok(Self::Server(id.to_owned())),
            "handle" => Ok(Self::Handle(id.to_owned())),
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for RegistryRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str(BUILTIN_NAMESPACE_REGISTRY),
            Self::Server(id) => write!(f, "server:{id}"),
            Self::Handle(id) => write!(f, "handle:{id}"),
        }
    }
}

fn is_registry_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_NAMESPACE_REGISTRY_ID_LEN
        && id.bytes().all(|byte| byte.is_ascii_graphic())
}

impl Namespaces {
    /// Check the written registry reference.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidNamespaceRegistry`] for a malformed reference.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(registry) = &self.registry {
            RegistryRef::parse(registry)?;
        }
        Ok(())
    }

    /// The resolved section: the written reference, or `builtin`. Call
    /// after [`Namespaces::validate`].
    #[must_use]
    pub fn resolved(&self) -> ResolvedNamespaces {
        ResolvedNamespaces {
            registry: self
                .registry
                .clone()
                .unwrap_or_else(|| BUILTIN_NAMESPACE_REGISTRY.to_owned()),
        }
    }
}

/// The resolved namespace plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedNamespaces {
    /// The registry reference, in its written form.
    pub registry: String,
}

impl Default for ResolvedNamespaces {
    fn default() -> Self {
        Self {
            registry: BUILTIN_NAMESPACE_REGISTRY.to_owned(),
        }
    }
}

impl ResolvedNamespaces {
    /// Whether this is the built-in default (the built-in registry only).
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.registry == BUILTIN_NAMESPACE_REGISTRY
    }

    /// The registry reference, parsed.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidNamespaceRegistry`] only for a value that did
    /// not come from a validated document.
    pub fn registry_ref(&self) -> Result<RegistryRef, ConfigError> {
        RegistryRef::parse(&self.registry)
    }
}
