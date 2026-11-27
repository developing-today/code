//! Deterministic inventory diffing and move reconciliation for re-imports (§6.1–§6.2).

use anyhow::Result;

use crate::{Inventory, Manifest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    New,
    Changed,
    Unchanged,
    MissingFromSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryChange {
    pub object_id: String,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReimportPlan {
    pub changes: Vec<InventoryChange>,
}

impl ReimportPlan {
    pub fn import_ids(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn is_noop(&self) -> bool {
        self.import_ids().is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovedDataSource {
    pub data_source_id: String,
    pub from_database_id: String,
    pub to_database_id: String,
}

pub fn plan_reimport(_previous: &Inventory, _current: &Inventory) -> Result<ReimportPlan> {
    Ok(ReimportPlan::default())
}

pub fn rehome_moved_sources(
    _previous: &Manifest,
    _current: &mut Manifest,
) -> Result<Vec<MovedDataSource>> {
    Ok(Vec::new())
}
