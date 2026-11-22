//! Client-side validation of a plain records query.
//!
//! Split from [`super`] (#12671) to keep `world_net` under the
//! production line limit.

use anyhow::{Context, Result};

/// Budget for validating records returned by a plain records query.
/// Enforces the same caps as `parse_records` and `read_records`.
#[derive(Default)]
pub(super) struct RecordsBudget {
    record_count: usize,
    last_key: Option<String>,
}

impl RecordsBudget {
    pub(super) fn validate_record(&mut self, key: &str, value: &serde_json::Value) -> Result<()> {
        // Check key length and non-empty
        anyhow::ensure!(
            !key.is_empty() && key.len() <= crate::world::MAX_RECORD_KEY_BYTES,
            "record key must be 1..={} bytes",
            crate::world::MAX_RECORD_KEY_BYTES
        );

        // Check for control characters
        anyhow::ensure!(
            !key.chars().any(char::is_control),
            "record key {key:?} contains a control character"
        );

        // Check value size
        let value_bytes = serde_json::to_vec(value)
            .context("measure record value")?
            .len();
        anyhow::ensure!(
            value_bytes <= crate::world::MAX_RECORD_VALUE_BYTES,
            "record {key:?} is {value_bytes} bytes; the limit is {} bytes",
            crate::world::MAX_RECORD_VALUE_BYTES
        );

        // Check total count
        self.record_count = self
            .record_count
            .checked_add(1)
            .context("record count overflow")?;
        anyhow::ensure!(
            self.record_count <= crate::world::MAX_RECORDS,
            "records query contains more than {} records; the limit is {}",
            self.record_count,
            crate::world::MAX_RECORDS
        );

        // Check ordering: keys must be strictly ascending
        if let Some(last) = &self.last_key {
            anyhow::ensure!(
                key > last.as_str(),
                "records are out of order: key {key:?} <= last key {last:?}"
            );
        }

        self.last_key = Some(key.to_owned());
        Ok(())
    }
}
