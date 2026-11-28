//! Numeric registry write validation (slice A9122-numeric-registry, issue
//! #10277; design A-9122.1, S4.64-S4.73).
//!
//! A numeric registry is a derived namespace over the array-address core. Its
//! write boundary admits only the numeric kinds named by its policy and uses
//! the numeric comparator's operational budget. The domain remains unbounded:
//! a large value is accepted whenever its encoded parts fit the configured
//! budget.
//!
//! This module deliberately does not own namespace generations or
//! authorization. Callers first admit a key here, then perform the namespace
//! mutation through [`crate::namespace_registry::RegistryState`] with its
//! existing [`crate::namespace_registry::RegistryAuthorizer`] seam.

use std::fmt;

use id_core::numeric_order;
use id_core::numeric_value::{NumericError, NumericLimits, NumericValue};

/// The version of this numeric write-policy contract.
pub const POLICY_VERSION: u64 = 1;

/// The three stable [`NumericValue`] kind tags.
pub const ALL_KINDS: u8 = (1 << id_core::numeric_value::KIND_INTEGER)
    | (1 << id_core::numeric_value::KIND_DECIMAL)
    | (1 << id_core::numeric_value::KIND_BINARY_FLOAT);

/// Namespace-specific numeric write policy.
///
/// `allowed_kinds` is a bit mask over the stable numeric kind tags. The
/// `limits` field is operational only; it is never a semantic magnitude or
/// precision maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericNamespacePolicy {
    /// Version of the policy contract.
    pub version: u64,
    /// Allowed [`NumericValue`] kinds, represented as `1 << kind_tag`.
    pub allowed_kinds: u8,
    /// Parser/comparator resource budget.
    pub limits: NumericLimits,
}

impl NumericNamespacePolicy {
    /// A V3 ordered numeric policy admitting all three kinds.
    #[must_use]
    pub const fn ordered() -> Self {
        Self {
            version: POLICY_VERSION,
            allowed_kinds: ALL_KINDS,
            limits: NumericLimits {
                max_magnitude_bytes: id_core::numeric_value::DEFAULT_MAX_MAGNITUDE_BYTES,
            },
        }
    }

    /// Creates a policy with an explicit kind mask and operational budget.
    #[must_use]
    pub const fn with_limits(allowed_kinds: u8, limits: NumericLimits) -> Self {
        Self {
            version: POLICY_VERSION,
            allowed_kinds,
            limits,
        }
    }

    fn allows(&self, value: &NumericValue) -> bool {
        self.allowed_kinds & (1 << value.kind_tag()) != 0
    }
}

/// A validated numeric key ready for an array-core write.
///
/// The inner value is private so callers cannot construct a key without
/// passing through [`NumericRegistry::admit`]. Namespace generation and
/// authorization still belong to `RegistryState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericKey(NumericValue);

impl NumericKey {
    /// The admitted numeric value.
    #[must_use]
    pub const fn value(&self) -> &NumericValue {
        &self.0
    }
}

/// Why a numeric namespace write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumericRegistryError {
    /// The value's kind is not enabled by the namespace policy.
    KindNotAllowed {
        /// The stable kind tag that was presented.
        kind: u8,
    },
    /// The value exceeded an operational parser/comparator budget.
    Numeric(NumericError),
}

impl NumericRegistryError {
    /// Stable error-envelope class.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::KindNotAllowed { .. } => "kind_not_allowed",
            Self::Numeric(error) => error.code(),
        }
    }
}

impl fmt::Display for NumericRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KindNotAllowed { kind } => write!(f, "numeric kind {kind} is not allowed"),
            Self::Numeric(error) => write!(f, "numeric key rejected: {error}"),
        }
    }
}

impl std::error::Error for NumericRegistryError {}

impl From<NumericError> for NumericRegistryError {
    fn from(error: NumericError) -> Self {
        Self::Numeric(error)
    }
}

/// The numeric write boundary for one namespace policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericRegistry {
    policy: NumericNamespacePolicy,
}

impl NumericRegistry {
    /// Creates a validator for a namespace policy.
    #[must_use]
    pub const fn new(policy: NumericNamespacePolicy) -> Self {
        Self { policy }
    }

    /// The policy this validator applies.
    #[must_use]
    pub const fn policy(&self) -> NumericNamespacePolicy {
        self.policy
    }

    /// Admits a key for a namespace write.
    ///
    /// Kind validation happens before comparator work. The self-comparison
    /// then exercises the exact comparator's budget checks without imposing a
    /// semantic maximum on the numeric value. The caller must pass the
    /// returned key to its array write and use `RegistryState` for the
    /// authorized, versioned namespace mutation.
    ///
    /// # Errors
    ///
    /// [`NumericRegistryError::KindNotAllowed`] when the namespace policy
    /// rejects the stable kind, or [`NumericRegistryError::Numeric`] when an
    /// operational budget is exceeded.
    pub fn admit(&self, value: NumericValue) -> Result<NumericKey, NumericRegistryError> {
        if !self.policy.allows(&value) {
            return Err(NumericRegistryError::KindNotAllowed {
                kind: value.kind_tag(),
            });
        }
        numeric_order::compare_with(&value, &value, self.policy.limits)?;
        Ok(NumericKey(value))
    }
}
