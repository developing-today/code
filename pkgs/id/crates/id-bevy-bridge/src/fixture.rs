//! Static journal fixture used to prove the bridge's schema and mapping
//! rules without a running world host (design §1, Q4).

/// A fixture delta at one journal position. Serialized with an `op`
/// tag, so a recorded journal reads as `{"op": "upsert", ...}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
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
    pub const fn position(&self) -> u64 {
        match self {
            Self::Upsert { position, .. } | Self::Retract { position, .. } => *position,
        }
    }
}

/// Example recorded prefix.
///
/// Ada is projected at position 1 and retracted at position 3. The unknown predicate intentionally demonstrates the
/// fail-closed schema boundary; it does not affect the projected state.
#[must_use]
pub fn static_fixture() -> Vec<JournalDelta> {
    vec![
        JournalDelta::Upsert {
            position: 1,
            id: "entity-ada".to_owned(),
            values: vec![
                ("name".to_owned(), serde_json::json!("Ada")),
                ("position".to_owned(), serde_json::json!([1.5, 2.0])),
                (
                    "unregistered.secret".to_owned(),
                    serde_json::json!("must not project"),
                ),
            ],
        },
        JournalDelta::Upsert {
            position: 2,
            id: "entity-babbage".to_owned(),
            values: vec![("name".to_owned(), serde_json::json!("Babbage"))],
        },
        JournalDelta::Retract {
            position: 3,
            id: "entity-ada".to_owned(),
        },
    ]
}

/// An in-memory hosted world for tests: a journal the bridge tails and an
/// append gate that journals accepted intents (design §2, slice 2).
///
/// The gate decodes the shell-contract `input` frame, rejects an intent
/// whose entity changed after its `based_on` position (it was proposed
/// against a stale view of that entity), rejects one that writes a denied
/// predicate, and otherwise journals it at the next position. An app that
/// is still catching up can therefore propose changes to entities nobody
/// else has touched since.
#[derive(Debug, Default)]
pub struct HostedWorldFixture {
    state: std::sync::Mutex<HostState>,
}

#[derive(Debug, Default)]
struct HostState {
    journal: Vec<crate::tail::PositionBatch>,
    denied: Vec<String>,
}

impl HostedWorldFixture {
    /// An empty hosted world.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, HostState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refuse every intent that writes `predicate`.
    pub fn deny_predicate(&self, predicate: impl Into<String>) {
        self.state().denied.push(predicate.into());
    }

    /// The last journaled position, if any.
    #[must_use]
    pub fn head(&self) -> Option<u64> {
        self.state().journal.last().map(|batch| batch.position)
    }

    /// Journal one position written by someone else; returns it. Each
    /// delta is restamped with the new position.
    pub fn append(&self, deltas: Vec<JournalDelta>) -> u64 {
        append_locked(&mut self.state(), deltas)
    }

    /// A copy of the whole journal, as recorded so far.
    #[must_use]
    pub fn journal(&self) -> Vec<crate::tail::PositionBatch> {
        self.state().journal.clone()
    }

    /// Journal a batch exactly as given, position and all, to model a
    /// misbehaving or corrupt journal.
    pub fn append_raw(&self, batch: crate::tail::PositionBatch) {
        self.state().journal.push(batch);
    }
}

fn append_locked(state: &mut HostState, deltas: Vec<JournalDelta>) -> u64 {
    let position = state.journal.last().map_or(1, |batch| batch.position + 1);
    let deltas = deltas
        .into_iter()
        .map(|delta| match delta {
            JournalDelta::Upsert { id, values, .. } => JournalDelta::Upsert {
                position,
                id,
                values,
            },
            JournalDelta::Retract { id, .. } => JournalDelta::Retract { position, id },
        })
        .collect();
    state
        .journal
        .push(crate::tail::PositionBatch { position, deltas });
    position
}

impl crate::tail::JournalSource for HostedWorldFixture {
    fn read_after(&self, after: Option<u64>, max: usize) -> Vec<crate::tail::PositionBatch> {
        let state = self.state();
        let start = after.map_or(0, |after| {
            state
                .journal
                .iter()
                .position(|batch| batch.position > after)
                .unwrap_or(state.journal.len())
        });
        state
            .journal
            .iter()
            .skip(start)
            .take(max)
            .cloned()
            .collect()
    }

    fn pending_after(&self, after: Option<u64>) -> u64 {
        let state = self.state();
        let pending = state
            .journal
            .iter()
            .filter(|batch| after.is_none_or(|after| batch.position > after))
            .count();
        u64::try_from(pending).unwrap_or(u64::MAX)
    }
}

impl crate::intent::AppendGate for HostedWorldFixture {
    fn submit(&self, frame: &serde_json::Value) -> crate::intent::GateVerdict {
        use crate::intent::GateVerdict;
        // One lock for check and append, so nothing lands in between.
        let mut state = self.state();
        match judge_intent(&state.journal, &state.denied, frame) {
            Ok(delta) => GateVerdict::Accepted {
                position: append_locked(&mut state, vec![delta]),
            },
            Err(reason) => GateVerdict::Rejected { reason },
        }
    }
}

/// The fixture gate's rules, for any hosted world that keeps them.
///
/// Decodes the shell-contract `input` frame and judges its intent against
/// `journal`: an intent whose entity changed after its `based_on`
/// position is stale, and one that writes a predicate in `denied` is
/// refused. An admitted intent comes back as the delta to journal, with
/// position 0 for the caller to stamp; a refused one as the reason. The
/// caller must hold its journal still between judging and appending.
///
/// # Errors
///
/// The reason the intent is refused.
pub fn judge_intent(
    journal: &[crate::tail::PositionBatch],
    denied: &[String],
    frame: &serde_json::Value,
) -> Result<JournalDelta, String> {
    use crate::intent::{Intent, Proposal};
    let intent = Intent::from_input_frame(frame).map_err(|error| error.to_string())?;
    let target = match &intent.proposal {
        Proposal::Upsert { id, .. } | Proposal::Retract { id } => id.clone(),
    };
    let touched_since = journal
        .iter()
        .filter(|batch| intent.based_on.is_none_or(|based| batch.position > based))
        .flat_map(|batch| batch.deltas.iter())
        .any(|delta| match delta {
            JournalDelta::Upsert { id, .. } | JournalDelta::Retract { id, .. } => *id == target,
        });
    if touched_since {
        return Err(format!(
            "stale: `{target}` changed after the intent's position"
        ));
    }
    match intent.proposal {
        Proposal::Upsert { id, values } => {
            if let Some((predicate, _)) = values
                .iter()
                .find(|(predicate, _)| denied.contains(predicate))
            {
                return Err(format!("denied predicate `{predicate}`"));
            }
            Ok(JournalDelta::Upsert {
                position: 0,
                id,
                values,
            })
        }
        Proposal::Retract { id } => Ok(JournalDelta::Retract { position: 0, id }),
    }
}
