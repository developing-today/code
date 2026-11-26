//! Intent round-trip: Bevy → append gate → journal → Bevy (design §2.2).
//!
//! A system that wants to change world state proposes an [`Intent`]
//! through [`IntentOutbox`], carrying the applied position it was based
//! on. [`submit_intents`] sends each one to the [`AppendGate`] as a
//! shell-contract `input` frame. The change becomes world state only when
//! the tail applies the journal position that confirms it, for the
//! proposing app too: nothing here writes a world-state component.
//!
//! An app may show the change early as presentation state with
//! [`PredictedPosition`]. [`reconcile_predictions`] removes the prediction
//! once the intent is settled, so a rejected intent visibly snaps back to
//! the applied [`WorldPosition`](crate::map::WorldPosition).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::map::WorldPosition;
use crate::projection::applied_position;

/// The change an intent proposes for one ID entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Proposal {
    /// Create or update the entity with these predicate values.
    Upsert {
        /// The ID entity.
        id: String,
        /// Predicate/value pairs.
        values: Vec<(String, serde_json::Value)>,
    },
    /// Retract the entity.
    Retract {
        /// The ID entity.
        id: String,
    },
}

/// A proposed append, as submitted to the gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    /// The app's own number for this intent.
    pub intent_id: u64,
    /// The applied position the proposing system saw.
    pub based_on: Option<u64>,
    /// The proposed change.
    pub proposal: Proposal,
}

/// Why an `input` frame did not decode as an intent.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum IntentDecodeError {
    /// The frame was not `{"type": "input", "data_hex": ...}`.
    #[error("not a shell-contract input frame")]
    NotInput,
    /// `data_hex` was not even-length lowercase or uppercase hex.
    #[error("data_hex is not hex")]
    BadHex,
    /// The payload was not an intent.
    #[error("payload is not an intent: {0}")]
    BadPayload(String),
}

impl Intent {
    /// Encode as a shell-contract `input` event: the intent's JSON in
    /// `data_hex` (contract v1, `input` fields).
    #[must_use]
    pub fn to_input_frame(&self) -> serde_json::Value {
        let payload = serde_json::to_vec(self).unwrap_or_default();
        serde_json::json!({ "type": "input", "data_hex": hex_encode(&payload) })
    }

    /// Decode an intent from a shell-contract `input` event.
    pub fn from_input_frame(frame: &serde_json::Value) -> Result<Self, IntentDecodeError> {
        if frame.get("type").and_then(serde_json::Value::as_str) != Some("input") {
            return Err(IntentDecodeError::NotInput);
        }
        let data = frame
            .get("data_hex")
            .and_then(serde_json::Value::as_str)
            .ok_or(IntentDecodeError::NotInput)?;
        let bytes = hex_decode(data).ok_or(IntentDecodeError::BadHex)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| IntentDecodeError::BadPayload(error.to_string()))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |digit: u8| {
        char::from(digit)
            .to_digit(16)
            .and_then(|v| u8::try_from(v).ok())
    };
    let (pairs, _) = digits.as_chunks::<2>();
    pairs
        .iter()
        .map(|[high, low]| Some((nibble(*high)? << 4) | nibble(*low)?))
        .collect()
}

/// The append gate's answer to one submitted intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Accepted; the change is journaled at `position`.
    Accepted {
        /// The confirming journal position.
        position: u64,
    },
    /// Rejected; nothing was journaled.
    Rejected {
        /// The gate's reason.
        reason: String,
    },
}

/// The world's append gate, which alone decides what is journaled.
pub trait AppendGate: Send + Sync + 'static {
    /// Submit one shell-contract `input` frame carrying an intent.
    fn submit(&self, frame: &serde_json::Value) -> GateVerdict;
}

/// The gate intents go to.
#[derive(Resource, Clone)]
pub struct IntentGate(pub Arc<dyn AppendGate>);

impl fmt::Debug for IntentGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntentGate").finish_non_exhaustive()
    }
}

/// Intents proposed by systems, waiting for the next [`JournalSet`] run.
///
/// [`JournalSet`]: crate::tail::JournalSet
#[derive(Resource, Debug, Default)]
pub struct IntentOutbox {
    queued: Vec<Intent>,
    next_id: u64,
}

impl IntentOutbox {
    /// Queue a proposal based on `based_on` and return its intent id.
    pub fn propose(&mut self, based_on: Option<u64>, proposal: Proposal) -> u64 {
        self.next_id += 1;
        self.queued.push(Intent {
            intent_id: self.next_id,
            based_on,
            proposal,
        });
        self.next_id
    }

    /// Intents not yet submitted.
    #[must_use]
    pub fn queued(&self) -> &[Intent] {
        &self.queued
    }
}

/// How an intent ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentOutcome {
    /// The confirming position has been applied.
    Confirmed {
        /// The intent.
        intent_id: u64,
        /// The confirming journal position.
        position: u64,
    },
    /// The gate refused it.
    Rejected {
        /// The intent.
        intent_id: u64,
        /// The gate's reason.
        reason: String,
    },
}

impl IntentOutcome {
    /// The intent this outcome settles.
    #[must_use]
    pub const fn intent_id(&self) -> u64 {
        match self {
            Self::Confirmed { intent_id, .. } | Self::Rejected { intent_id, .. } => *intent_id,
        }
    }
}

/// Accepted intents waiting for their confirming position, and the
/// outcomes settled during the current tick.
#[derive(Resource, Debug, Default)]
pub struct IntentLedger {
    awaiting: BTreeMap<u64, u64>,
    settled: Vec<IntentOutcome>,
}

impl IntentLedger {
    /// The confirming position of an accepted intent not yet applied.
    #[must_use]
    pub fn awaiting(&self, intent_id: u64) -> Option<u64> {
        self.awaiting.get(&intent_id).copied()
    }

    /// Outcomes settled during the current tick.
    #[must_use]
    pub fn settled(&self) -> &[IntentOutcome] {
        &self.settled
    }
}

/// Presentation-class prediction of a position an intent proposes. It
/// never changes [`WorldPosition`]; it is removed when the intent settles.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct PredictedPosition {
    /// The intent this prediction belongs to.
    pub intent_id: u64,
    /// The predicted position.
    pub at: WorldPosition,
}

/// What a presenter should draw for an entity's position, and whether it
/// is a prediction rather than applied world state.
#[must_use]
pub fn presented_position(world: &World, entity: Entity) -> Option<(WorldPosition, bool)> {
    if let Some(predicted) = world.get::<PredictedPosition>(entity) {
        return Some((predicted.at, true));
    }
    world.get::<WorldPosition>(entity).map(|at| (*at, false))
}

/// Send queued intents to the gate. Starts the tick's settled outcomes
/// afresh; rejections settle at once.
pub fn submit_intents(world: &mut World) {
    if !world.contains_resource::<IntentLedger>() {
        world.insert_resource(IntentLedger::default());
    }
    world.resource_mut::<IntentLedger>().settled.clear();
    let Some(gate) = world.get_resource::<IntentGate>().cloned() else {
        return;
    };
    let queued = world
        .get_resource_mut::<IntentOutbox>()
        .map(|mut outbox| std::mem::take(&mut outbox.queued))
        .unwrap_or_default();
    let mut ledger = world.resource_mut::<IntentLedger>();
    for intent in queued {
        match gate.0.submit(&intent.to_input_frame()) {
            GateVerdict::Accepted { position } => {
                ledger.awaiting.insert(intent.intent_id, position);
            }
            GateVerdict::Rejected { reason } => ledger.settled.push(IntentOutcome::Rejected {
                intent_id: intent.intent_id,
                reason,
            }),
        }
    }
}

/// Confirm accepted intents whose position the tail has now applied.
pub fn confirm_intents(world: &mut World) {
    let Some(applied) = applied_position(world) else {
        return;
    };
    let Some(mut ledger) = world.get_resource_mut::<IntentLedger>() else {
        return;
    };
    let confirmed: Vec<(u64, u64)> = ledger
        .awaiting
        .iter()
        .filter(|(_, position)| **position <= applied)
        .map(|(intent, position)| (*intent, *position))
        .collect();
    for (intent_id, position) in confirmed {
        ledger.awaiting.remove(&intent_id);
        ledger.settled.push(IntentOutcome::Confirmed {
            intent_id,
            position,
        });
    }
}

/// Remove predictions whose intent settled this tick: on confirmation
/// the world state now shows the change; on rejection the presenter
/// snaps back to the applied world state.
pub fn reconcile_predictions(world: &mut World) {
    let Some(ledger) = world.get_resource::<IntentLedger>() else {
        return;
    };
    let settled: Vec<u64> = ledger
        .settled
        .iter()
        .map(IntentOutcome::intent_id)
        .collect();
    if settled.is_empty() {
        return;
    }
    let mut query = world.query::<(Entity, &PredictedPosition)>();
    let stale: Vec<Entity> = query
        .iter(world)
        .filter(|(_, predicted)| settled.contains(&predicted.intent_id))
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale {
        world.entity_mut(entity).remove::<PredictedPosition>();
    }
}
