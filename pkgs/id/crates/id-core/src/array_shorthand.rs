//! Per-namespace configured bare-string shorthand for array addresses.
//!
//! Slice A9119-shorthand-policy (issue #10261), design
//! `part-a-addressing/versioned-design.md` A-9119.1, statements
//! S4.29–S4.33.
//!
//! A bare UTF-8 string such as `name` is shorthand for an array address.
//! How it expands is configuration, never a constant:
//!
//! ```text
//! { default_slot: index, prefix_fill: null, text_encoding: UTF8 }
//! ```
//!
//! With `default_slot = 2` the string `name` expands to
//! `[null, null, utf8("name")]`, but 2 is only an example: nothing here
//! knows a default index. A policy is set per namespace and may be
//! overridden per server; the server override wins only where one is
//! configured. With neither, expansion fails as
//! `missing_shorthand_policy` and never falls back to any index.
//!
//! Shorthand only names array addresses, so a namespace bound to the
//! content core refuses it as `shorthand_requires_array_core`.

use std::collections::BTreeMap;
use std::fmt;

use crate::array_address::ArrayAddress;
use crate::namespace_definition::{CoreKind, NamespaceDefinition, NamespaceId, RegistryId};

/// What fills the slots before the default slot. The design admits only
/// null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrefixFill {
    /// Every earlier slot is a null slot.
    Null,
}

/// How the bare string becomes slot bytes. The design admits only
/// UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextEncoding {
    /// The string's own UTF-8 bytes, unchanged.
    Utf8,
}

/// One namespace's shorthand configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShorthandPolicy {
    default_slot: usize,
    prefix_fill: PrefixFill,
    text_encoding: TextEncoding,
}

impl ShorthandPolicy {
    /// A policy placing the string at `default_slot`, with null prefix
    /// fill and UTF-8 text, the only fill and encoding V2 admits.
    pub const fn new(default_slot: usize) -> Self {
        Self {
            default_slot,
            prefix_fill: PrefixFill::Null,
            text_encoding: TextEncoding::Utf8,
        }
    }

    /// The zero-based slot the string goes in.
    pub const fn default_slot(self) -> usize {
        self.default_slot
    }

    /// What fills the earlier slots.
    pub const fn prefix_fill(self) -> PrefixFill {
        self.prefix_fill
    }

    /// How the string becomes bytes.
    pub const fn text_encoding(self) -> TextEncoding {
        self.text_encoding
    }

    /// Expand `text` under this policy: `default_slot` prefix slots, then
    /// the encoded string. The result has exactly `default_slot + 1`
    /// slots. A policy whose slot cannot be allocated fails as
    /// `expansion_too_large` rather than aborting; there is no fixed
    /// maximum index.
    pub fn expand(self, text: &str) -> Result<ArrayAddress, ShorthandError> {
        let len = self
            .default_slot
            .checked_add(1)
            .ok_or(ShorthandError::ExpansionTooLarge)?;
        let mut slots: Vec<Option<Vec<u8>>> = Vec::new();
        slots
            .try_reserve_exact(len)
            .map_err(|_reserve| ShorthandError::ExpansionTooLarge)?;
        let fill = match self.prefix_fill {
            PrefixFill::Null => None,
        };
        slots.resize(self.default_slot, fill);
        let bytes = match self.text_encoding {
            TextEncoding::Utf8 => text.as_bytes().to_vec(),
        };
        slots.push(Some(bytes));
        Ok(ArrayAddress::new(slots))
    }
}

/// Why a bare string could not be expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShorthandError {
    /// Neither the server nor the namespace configures a policy.
    MissingShorthandPolicy {
        /// The namespace that has no policy.
        namespace: NamespaceId,
    },
    /// The namespace is bound to the content core, which has no slots.
    ShorthandRequiresArrayCore {
        /// The namespace that is not array-addressed.
        namespace: NamespaceId,
    },
    /// The configured slot index cannot be allocated.
    ExpansionTooLarge,
}

impl ShorthandError {
    /// The stable class name.
    pub const fn class(&self) -> &'static str {
        match self {
            Self::MissingShorthandPolicy { .. } => "missing_shorthand_policy",
            Self::ShorthandRequiresArrayCore { .. } => "shorthand_requires_array_core",
            Self::ExpansionTooLarge => "expansion_too_large",
        }
    }
}

impl fmt::Display for ShorthandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingShorthandPolicy { namespace }
            | Self::ShorthandRequiresArrayCore { namespace } => {
                write!(f, "{}: {}", self.class(), namespace.as_str())
            }
            Self::ExpansionTooLarge => f.write_str(self.class()),
        }
    }
}

impl std::error::Error for ShorthandError {}

/// Shorthand configuration: per-namespace defaults and per-server
/// overrides. Everything is unset until configured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShorthandConfig {
    namespace_defaults: BTreeMap<NamespaceId, ShorthandPolicy>,
    server_overrides: BTreeMap<(RegistryId, NamespaceId), ShorthandPolicy>,
}

impl ShorthandConfig {
    /// An empty configuration: every namespace is unset.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the namespace's own policy, returning the one it replaces.
    pub fn set_namespace_default(
        &mut self,
        namespace: NamespaceId,
        policy: ShorthandPolicy,
    ) -> Option<ShorthandPolicy> {
        self.namespace_defaults.insert(namespace, policy)
    }

    /// Unset the namespace's own policy, returning it.
    pub fn clear_namespace_default(&mut self, namespace: &NamespaceId) -> Option<ShorthandPolicy> {
        self.namespace_defaults.remove(namespace)
    }

    /// Set one server's override for a namespace, returning the one it
    /// replaces.
    pub fn set_server_override(
        &mut self,
        server: RegistryId,
        namespace: NamespaceId,
        policy: ShorthandPolicy,
    ) -> Option<ShorthandPolicy> {
        self.server_overrides.insert((server, namespace), policy)
    }

    /// Remove one server's override for a namespace, returning it.
    pub fn clear_server_override(
        &mut self,
        server: &RegistryId,
        namespace: &NamespaceId,
    ) -> Option<ShorthandPolicy> {
        self.server_overrides
            .remove(&(server.clone(), namespace.clone()))
    }

    /// The policy in force for `namespace` on `server`: that server's
    /// override where one is configured, otherwise the namespace's own
    /// policy, otherwise `missing_shorthand_policy`. Another server's
    /// override never applies.
    pub fn policy_for(
        &self,
        server: &RegistryId,
        namespace: &NamespaceId,
    ) -> Result<ShorthandPolicy, ShorthandError> {
        self.server_overrides
            .get(&(server.clone(), namespace.clone()))
            .or_else(|| self.namespace_defaults.get(namespace))
            .copied()
            .ok_or_else(|| ShorthandError::MissingShorthandPolicy {
                namespace: namespace.clone(),
            })
    }

    /// Expand `text` for a namespace defined by `definition`, on
    /// `server`. The namespace must be array-addressed and have a policy
    /// in force.
    pub fn expand(
        &self,
        server: &RegistryId,
        definition: &NamespaceDefinition,
        text: &str,
    ) -> Result<ArrayAddress, ShorthandError> {
        if definition.core != CoreKind::Array {
            return Err(ShorthandError::ShorthandRequiresArrayCore {
                namespace: definition.namespace.clone(),
            });
        }
        self.policy_for(server, &definition.namespace)?.expand(text)
    }
}
