//! The host posture (design §3, second posture; #13706): the Bevy
//! process also runs the world's append gate and journal.
//!
//! [`EmbeddedHost`] is a hosted world whose journal is a
//! [`JournalStore`] on disk. It is the same kind of thing a presenter
//! joins from outside: a [`JournalSource`], an [`AppendGate`] and a
//! [`HostedWorld`] with grants and admission. Hosting adds lifecycle and
//! nothing else:
//!
//! - **Lifecycle:** [`EmbeddedHost::create`] starts a journal with its
//!   genesis positions, [`EmbeddedHost::open`] resumes a stored one from
//!   its last position, the gate appends durably, and
//!   [`EmbeddedHost::close`] syncs it and stops admitting and appending.
//! - **Admission:** other presenters join through [`listen`] and are
//!   admitted by their v1 device manifest, up to a capacity.
//! - **No back door:** after genesis the only way into the journal is the
//!   gate, and the host's own Bevy side joins through
//!   [`crate::Session::host`], which connects the bridge to this world's
//!   journal and gate exactly as [`crate::Session::join`] connects it to
//!   a remote one. There is no method that writes world state.
//!
//! [`listen`]: crate::remote::listen

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use id_bevy_bridge::{
    AppendGate, GateVerdict, JournalDelta, JournalSource, PositionBatch, judge_intent,
};

use crate::host::{ARENA_HALF_EXTENT, OUTSIDE_ARENA, arena_genesis, outside_arena};
use crate::journal_store::{JournalStore, StoreError};
use crate::remote::HostedWorld;

/// The gate's reason once the world is closed.
pub const WORLD_CLOSED: &str = "the world is closed";

/// Presenters a world admits unless told otherwise.
pub const DEFAULT_CAPACITY: usize = 8;

/// The rules the world's gate keeps besides the bridge's own (stale
/// intents are always refused).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldRules {
    /// Refuse a position more than this far from the centre on either
    /// axis.
    pub half_extent: Option<f64>,
    /// Refuse any intent that writes one of these predicates.
    pub denied: Vec<String>,
}

impl WorldRules {
    /// The demo arena's rules: stay inside the arena.
    #[must_use]
    pub const fn arena() -> Self {
        Self {
            half_extent: Some(ARENA_HALF_EXTENT),
            denied: Vec::new(),
        }
    }
}

/// How a hosted world treats the presenters that join it.
#[derive(Debug, Clone, PartialEq)]
pub struct HostSettings {
    /// The gate's rules.
    pub rules: WorldRules,
    /// Provider names granted to every presenter, the host's own
    /// included.
    pub granted: Vec<String>,
    /// Most presenters admitted at once, the host's own not counted.
    pub capacity: usize,
}

impl HostSettings {
    /// The demo arena, granting `granted`.
    #[must_use]
    pub const fn arena(granted: Vec<String>) -> Self {
        Self {
            rules: WorldRules::arena(),
            granted,
            capacity: DEFAULT_CAPACITY,
        }
    }
}

#[derive(Debug)]
struct Core {
    /// The open journal; `None` once the world is closed.
    store: Option<JournalStore>,
    /// The journal as it was closed, still readable.
    closed: Vec<PositionBatch>,
    /// Presenters admitted and not yet departed.
    present: usize,
}

impl Core {
    fn batches(&self) -> &[PositionBatch] {
        self.store
            .as_ref()
            .map_or(self.closed.as_slice(), JournalStore::batches)
    }
}

/// A world hosted in this process, with its journal on disk.
#[derive(Debug)]
pub struct EmbeddedHost {
    core: Mutex<Core>,
    settings: HostSettings,
}

impl EmbeddedHost {
    fn with_store(store: JournalStore, settings: HostSettings) -> Self {
        Self {
            core: Mutex::new(Core {
                store: Some(store),
                closed: Vec::new(),
                present: 0,
            }),
            settings,
        }
    }

    /// Create a world at `path` whose journal starts with `genesis`, one
    /// position per entry. Fails if `path` exists.
    ///
    /// # Errors
    ///
    /// The journal could not be created or written.
    pub fn create(
        path: &Path,
        genesis: Vec<Vec<JournalDelta>>,
        settings: HostSettings,
    ) -> Result<Self, StoreError> {
        let mut store = JournalStore::create(path)?;
        for position in genesis {
            store.append(position)?;
        }
        Ok(Self::with_store(store, settings))
    }

    /// Open the world stored at `path`. It resumes from its last
    /// position; see [`JournalStore::open`].
    ///
    /// # Errors
    ///
    /// The journal could not be read or is not whole and ordered.
    pub fn open(path: &Path, settings: HostSettings) -> Result<Self, StoreError> {
        Ok(Self::with_store(JournalStore::open(path)?, settings))
    }

    /// The demo arena stored at `path`: opened if it exists, otherwise
    /// created with the arena's genesis.
    ///
    /// # Errors
    ///
    /// As [`Self::open`] and [`Self::create`].
    pub fn arena(path: &Path, granted: Vec<String>) -> Result<Self, StoreError> {
        let settings = HostSettings::arena(granted);
        if path.exists() {
            Self::open(path, settings)
        } else {
            Self::create(path, arena_genesis(), settings)
        }
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The whole journal.
    #[must_use]
    pub fn journal(&self) -> Vec<PositionBatch> {
        self.core().batches().to_vec()
    }

    /// The last position, if any.
    #[must_use]
    pub fn head(&self) -> Option<u64> {
        self.core().batches().last().map(|batch| batch.position)
    }

    /// Presenters admitted now, the host's own not counted.
    #[must_use]
    pub fn present(&self) -> usize {
        self.core().present
    }

    /// Whether the world has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.core().store.is_none()
    }

    /// Close the world: sync the journal, then refuse new presenters and
    /// every intent. The journal stays readable. Closing twice is fine.
    ///
    /// # Errors
    ///
    /// The final sync failed. The world is closed all the same.
    pub fn close(&self) -> Result<(), StoreError> {
        let mut core = self.core();
        let Some(store) = core.store.take() else {
            return Ok(());
        };
        core.closed = store.batches().to_vec();
        store.close()
    }
}

impl JournalSource for EmbeddedHost {
    fn read_after(&self, after: Option<u64>, max: usize) -> Vec<PositionBatch> {
        self.core()
            .batches()
            .iter()
            .filter(|batch| after.is_none_or(|after| batch.position > after))
            .take(max)
            .cloned()
            .collect()
    }

    fn pending_after(&self, after: Option<u64>) -> u64 {
        let pending = self
            .core()
            .batches()
            .iter()
            .filter(|batch| after.is_none_or(|after| batch.position > after))
            .count();
        u64::try_from(pending).unwrap_or(u64::MAX)
    }
}

impl AppendGate for EmbeddedHost {
    fn submit(&self, frame: &serde_json::Value) -> GateVerdict {
        // One lock for the checks and the append, so nothing lands in
        // between and positions stay in order.
        let mut core = self.core();
        let Some(store) = core.store.as_mut() else {
            return GateVerdict::Rejected {
                reason: WORLD_CLOSED.to_owned(),
            };
        };
        let rules = &self.settings.rules;
        if rules
            .half_extent
            .is_some_and(|half_extent| outside_arena(frame, half_extent))
        {
            return GateVerdict::Rejected {
                reason: OUTSIDE_ARENA.to_owned(),
            };
        }
        match judge_intent(store.batches(), &rules.denied, frame) {
            // The position is on disk before the presenter hears of it.
            Ok(delta) => match store.append(vec![delta]) {
                Ok(position) => GateVerdict::Accepted { position },
                Err(error) => GateVerdict::Rejected {
                    reason: format!("the journal could not take it: {error}"),
                },
            },
            Err(reason) => GateVerdict::Rejected { reason },
        }
    }
}

impl HostedWorld for EmbeddedHost {
    fn granted(&self) -> Vec<String> {
        self.settings.granted.clone()
    }

    fn admit(&self) -> Result<(), String> {
        let mut core = self.core();
        if core.store.is_none() {
            return Err(WORLD_CLOSED.to_owned());
        }
        if core.present >= self.settings.capacity {
            return Err(format!(
                "the world is full: it admits {} presenters",
                self.settings.capacity
            ));
        }
        core.present += 1;
        Ok(())
    }

    fn depart(&self) {
        let mut core = self.core();
        core.present = core.present.saturating_sub(1);
    }
}
