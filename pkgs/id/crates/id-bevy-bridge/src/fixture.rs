//! Static journal fixture used to prove the bridge's schema and mapping
//! rules without a running world host (design §1, Q4).

/// A fixture delta at one journal position.
#[derive(Debug, Clone, PartialEq)]
pub enum JournalDelta {
    /// Create or update a journal entity with its predicate values.
    Upsert {
        /// Journal position of this delta.
        position: u64,
        /// Stable ID entity identifier.
        id: String,
        /// Predicate/value pairs; JSON is retained verbatim by projection.
        values: Vec<(String, serde_json::Value)>,
    },
    /// Retract an ID entity at a journal position.
    Retract {
        /// Journal position of this delta.
        position: u64,
        /// Stable ID entity identifier.
        id: String,
    },
}

impl JournalDelta {
    /// Journal position carried by this delta.
    #[must_use]
    pub fn position(&self) -> u64 {
        match self {
            Self::Upsert { position, .. } | Self::Retract { position, .. } => *position,
        }
    }
}

/// Example recorded prefix: Ada is projected at position 1 and retracted
/// at position 3. The unknown predicate intentionally demonstrates the
/// fail-closed schema boundary; it does not affect the projected state.
#[must_use]
pub fn static_fixture() -> Vec<JournalDelta> {
    vec![
        JournalDelta::Upsert {
            position: 1,
            id: "entity-ada".to_string(),
            values: vec![
                ("name".to_string(), serde_json::json!("Ada")),
                ("position".to_string(), serde_json::json!([1.5, 2.0])),
                (
                    "unregistered.secret".to_string(),
                    serde_json::json!("must not project"),
                ),
            ],
        },
        JournalDelta::Upsert {
            position: 2,
            id: "entity-babbage".to_string(),
            values: vec![("name".to_string(), serde_json::json!("Babbage"))],
        },
        JournalDelta::Retract {
            position: 3,
            id: "entity-ada".to_string(),
        },
    ]
}
