//! The demo world the reference app joins: an arena with the local
//! avatar and two beacons, hosted in its own process (`host` mode).
//!
//! It is the bridge's [`HostedWorldFixture`] with one world rule added at
//! the gate: a position outside the arena is rejected. That rule is what
//! makes the presenter's snap-back visible on demand: walk into the edge
//! and the predicted step is drawn, refused, and drawn back.

use id_bevy_bridge::{
    AppendGate, GateVerdict, HostedWorldFixture, Intent, JournalDelta, JournalSource,
    PositionBatch, Proposal,
};

use crate::app::AVATAR;
use crate::provider::Surface;
use crate::remote::HostedWorld;

/// Half the arena's width and height, in world units.
pub const ARENA_HALF_EXTENT: f64 = 3.0;

/// The gate's reason for a position outside the arena.
pub const OUTSIDE_ARENA: &str = "outside the arena";

/// The demo world: journal, gate and the providers it grants.
#[derive(Debug)]
pub struct ArenaHost {
    world: HostedWorldFixture,
    half_extent: f64,
    granted: Vec<String>,
}

impl ArenaHost {
    /// The demo arena, granting `granted` provider names.
    #[must_use]
    pub fn demo(granted: Vec<String>) -> Self {
        let world = HostedWorldFixture::new();
        for position in arena_genesis() {
            world.append(position);
        }
        Self {
            world,
            half_extent: ARENA_HALF_EXTENT,
            granted,
        }
    }

    /// The providers that enable every presenter surface.
    #[must_use]
    pub fn every_surface() -> Vec<String> {
        Surface::ALL
            .iter()
            .map(|surface| surface.provider().to_owned())
            .collect()
    }
}

/// The demo arena's first positions: the avatar at the centre, then two
/// beacons. Every host of the arena starts its journal with them.
#[must_use]
pub fn arena_genesis() -> Vec<Vec<JournalDelta>> {
    vec![
        vec![upsert(AVATAR, "You", [0.0, 0.0])],
        vec![
            upsert("beacon-north", "North beacon", [0.0, 2.0]),
            upsert("beacon-east", "East beacon", [2.0, 0.0]),
        ],
    ]
}

/// The arena's world rule: true when `frame` carries an intent that
/// moves something to a position more than `half_extent` from the
/// centre on either axis. A frame that is not an intent is left to the
/// gate's own decoding.
#[must_use]
pub fn outside_arena(frame: &serde_json::Value, half_extent: f64) -> bool {
    let Ok(Intent {
        proposal: Proposal::Upsert { values, .. },
        ..
    }) = Intent::from_input_frame(frame)
    else {
        return false;
    };
    values.iter().any(|(predicate, value)| {
        predicate == "position"
            && value.as_array().is_some_and(|pair| {
                pair.iter()
                    .any(|coordinate| coordinate.as_f64().is_none_or(|c| c.abs() > half_extent))
            })
    })
}

fn upsert(id: &str, name: &str, at: [f64; 2]) -> JournalDelta {
    JournalDelta::Upsert {
        position: 0,
        id: id.to_owned(),
        values: vec![
            ("name".to_owned(), serde_json::json!(name)),
            ("position".to_owned(), serde_json::json!(at)),
        ],
    }
}

impl JournalSource for ArenaHost {
    fn read_after(&self, after: Option<u64>, max: usize) -> Vec<PositionBatch> {
        self.world.read_after(after, max)
    }

    fn pending_after(&self, after: Option<u64>) -> u64 {
        self.world.pending_after(after)
    }
}

impl AppendGate for ArenaHost {
    fn submit(&self, frame: &serde_json::Value) -> GateVerdict {
        if outside_arena(frame, self.half_extent) {
            return GateVerdict::Rejected {
                reason: OUTSIDE_ARENA.to_owned(),
            };
        }
        self.world.submit(frame)
    }
}

impl HostedWorld for ArenaHost {
    fn granted(&self) -> Vec<String> {
        self.granted.clone()
    }
}
