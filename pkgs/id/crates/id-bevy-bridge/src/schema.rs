//! Schema-as-data for the ID → Bevy projection (design §1.2–§1.3).

use std::collections::BTreeMap;

/// Authority/data class for a predicate (design §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClass {
    /// Journal-owned world state. Only this class is projected here.
    World,
    /// Bevy-owned presentation state; never journaled or projected.
    Presentation,
    /// Process-local Bevy state; never journaled or projected.
    Local,
}

/// Supported projection rule for a schema component (design §1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentRule {
    /// Journaled UTF-8 string projected verbatim.
    Name,
    /// Journaled two-element JSON array of numbers projected verbatim.
    Position2,
}

/// Classification and mapping for one predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredicateRule {
    /// Data authority class.
    pub class: DataClass,
    /// Component mapping and value decoding rule.
    pub component: ComponentRule,
}

/// Predicate → component/data class/project rule table.
///
/// Missing entries intentionally have no fallback: projection ignores an
/// unclassified predicate (design §1.2 total, fail-closed classification).
#[derive(Debug, Clone, Default)]
pub struct BridgeSchema {
    rules: BTreeMap<String, PredicateRule>,
}

impl BridgeSchema {
    /// Create an empty schema.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a predicate's explicit authority and projection rule.
    pub fn register(&mut self, predicate: impl Into<String>, rule: PredicateRule) {
        self.rules.insert(predicate.into(), rule);
    }

    /// Look up an explicit classification; `None` means fail closed.
    #[must_use]
    pub fn rule(&self, predicate: &str) -> Option<PredicateRule> {
        self.rules.get(predicate).copied()
    }
}
