//! Source freeze acknowledgement gate (tracker #3404).
//!
//! Design: `pkgs/id/docs/2026-11-01_jurisdiction_aware_user_store_design.md`
//! §7 "Write fence". The coordinator must wait for the source
//! acknowledgement and never export if it cannot establish it; switch
//! is refused without freeze acknowledgement.
//!
//! This module is the pure decision function behind that rule: given
//! what the coordinator requires ([`FreezeRequirement`]) and what the
//! source durably acknowledged ([`FreezeAck`]), [`export_may_proceed`]
//! says whether an export (and therefore everything downstream of it:
//! copy, import, switch) may begin. Every refusal is a typed
//! [`FreezeError`] naming the rule that failed, and the gate is
//! fail-closed: anything missing or mismatched refuses.
//!
//! What is NOT here (follow-ons): persisting the ack in the directory,
//! the source-side drain handshake that produces the ack, and enforcing
//! this gate at the mutation entry points (WebSockets, alarms, RPC,
//! bridges). Those need the directory metadata (#3399) and the
//! restricted routing guards (#3400).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What the coordinator demands before an export may begin: the
/// migration it belongs to, the source jurisdiction it must come
/// from, and the minimum source op-sequence watermark the ack must
/// cover. Watermark `0` is genesis (a source with nothing applied);
/// a requirement of `0` accepts an ack of `0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeRequirement {
    /// Durable migration ID (`M` in the design runbook).
    pub migration_id: String,
    /// Jurisdiction label of the frozen source (e.g. `"us"`).
    pub source_jurisdiction: String,
    /// Lowest acceptable source watermark (inclusive).
    pub min_watermark: u64,
}

/// The source's durable freeze acknowledgement: the freeze watermark
/// is the last applied op-sequence included in the export snapshot,
/// and `in_flight_drained` records that every in-flight mutation was
/// serialized before the watermark was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeAck {
    /// Migration this ack belongs to; must equal the requirement's.
    pub migration_id: String,
    /// Source jurisdiction that froze; must equal the requirement's.
    pub source_jurisdiction: String,
    /// Last applied op-sequence covered by the snapshot.
    pub watermark: u64,
    /// True once all in-flight mutations were drained/serialized.
    pub in_flight_drained: bool,
}

/// Every way the freeze gate can refuse an export.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FreezeError {
    /// No source acknowledgement was presented at all.
    #[error("no source freeze acknowledgement for migration {0}")]
    MissingAck(String),
    /// The ack belongs to a different migration.
    #[error("freeze ack migration {got} does not match required {want}")]
    MigrationMismatch {
        /// The migration named by the presented ack.
        got: String,
        /// The migration the coordinator required.
        want: String,
    },
    /// The ack comes from a different source jurisdiction.
    #[error("freeze ack jurisdiction {got} does not match required {want}")]
    JurisdictionMismatch {
        /// The jurisdiction named by the presented ack.
        got: String,
        /// The source jurisdiction the coordinator required.
        want: String,
    },
    /// The ack watermark is below the required minimum: the snapshot
    /// would miss source writes.
    #[error("freeze ack watermark {got} is below required {need}")]
    StaleWatermark {
        /// The watermark the ack actually covers.
        got: u64,
        /// The watermark the coordinator required.
        need: u64,
    },
    /// The source froze with mutations still in flight: the snapshot
    /// is not a consistent cut.
    #[error("source froze with in-flight mutations undrained")]
    InFlightNotDrained,
}

/// Decide whether an export may begin. Returns `Ok(())` only when a
/// present ack names the required migration and source jurisdiction,
/// covers at least the required watermark, and confirms the drain.
/// Anything else — including a missing ack — refuses.
pub fn export_may_proceed(
    ack: Option<&FreezeAck>,
    required: &FreezeRequirement,
) -> Result<(), FreezeError> {
    let Some(ack) = ack else {
        return Err(FreezeError::MissingAck(required.migration_id.clone()));
    };

    if ack.migration_id != required.migration_id {
        return Err(FreezeError::MigrationMismatch {
            got: ack.migration_id.clone(),
            want: required.migration_id.clone(),
        });
    }
    if ack.source_jurisdiction != required.source_jurisdiction {
        return Err(FreezeError::JurisdictionMismatch {
            got: ack.source_jurisdiction.clone(),
            want: required.source_jurisdiction.clone(),
        });
    }
    if ack.watermark < required.min_watermark {
        return Err(FreezeError::StaleWatermark {
            got: ack.watermark,
            need: required.min_watermark,
        });
    }
    if !ack.in_flight_drained {
        return Err(FreezeError::InFlightNotDrained);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn requirement() -> FreezeRequirement {
        FreezeRequirement {
            migration_id: "m-01".to_string(),
            source_jurisdiction: "us".to_string(),
            min_watermark: 41,
        }
    }

    fn matching_ack() -> FreezeAck {
        FreezeAck {
            migration_id: "m-01".to_string(),
            source_jurisdiction: "us".to_string(),
            watermark: 41,
            in_flight_drained: true,
        }
    }

    #[test]
    fn golden_matching_ack_permits_export() {
        assert_eq!(
            export_may_proceed(Some(&matching_ack()), &requirement()),
            Ok(())
        );
    }

    #[test]
    fn export_refused_without_any_ack() {
        assert_eq!(
            export_may_proceed(None, &requirement()),
            Err(FreezeError::MissingAck("m-01".to_string()))
        );
    }

    #[test]
    fn stale_watermark_is_refused() {
        let ack = FreezeAck {
            watermark: 40,
            ..matching_ack()
        };
        assert_eq!(
            export_may_proceed(Some(&ack), &requirement()),
            Err(FreezeError::StaleWatermark { got: 40, need: 41 })
        );
    }

    #[test]
    fn wrong_migration_is_refused() {
        let ack = FreezeAck {
            migration_id: "m-02".to_string(),
            ..matching_ack()
        };
        assert_eq!(
            export_may_proceed(Some(&ack), &requirement()),
            Err(FreezeError::MigrationMismatch {
                got: "m-02".to_string(),
                want: "m-01".to_string(),
            })
        );
    }

    #[test]
    fn wrong_jurisdiction_is_refused() {
        let ack = FreezeAck {
            source_jurisdiction: "eu".to_string(),
            ..matching_ack()
        };
        assert_eq!(
            export_may_proceed(Some(&ack), &requirement()),
            Err(FreezeError::JurisdictionMismatch {
                got: "eu".to_string(),
                want: "us".to_string(),
            })
        );
    }

    #[test]
    fn undrained_in_flight_is_refused() {
        let ack = FreezeAck {
            in_flight_drained: false,
            ..matching_ack()
        };
        assert_eq!(
            export_may_proceed(Some(&ack), &requirement()),
            Err(FreezeError::InFlightNotDrained)
        );
    }

    #[test]
    fn genesis_watermark_zero_is_accepted_when_required() {
        let req = FreezeRequirement {
            min_watermark: 0,
            ..requirement()
        };
        let ack = FreezeAck {
            watermark: 0,
            ..matching_ack()
        };
        assert_eq!(export_may_proceed(Some(&ack), &req), Ok(()));
    }

    // Fail-closed oracle: `Ok` is returned exactly when the ack is
    // present, names the required migration and jurisdiction, covers
    // the watermark, and confirms the drain. Anything else refuses,
    // so no test double or mock can sneak an export past the gate.
    proptest! {
        #[test]
        fn gate_permits_export_exactly_on_full_match(
            ack_migration in "[a-z0-9-]{0,8}",
            ack_jurisdiction in "[a-z]{0,4}",
            ack_watermark in 0u64..100,
            ack_drained in proptest::bool::ANY,
            req_migration in "[a-z0-9-]{0,8}",
            req_jurisdiction in "[a-z]{0,4}",
            req_min in 0u64..100,
            present in proptest::bool::ANY,
        ) {
            let req = FreezeRequirement {
                migration_id: req_migration.clone(),
                source_jurisdiction: req_jurisdiction.clone(),
                min_watermark: req_min,
            };
            let ack = FreezeAck {
                migration_id: ack_migration.clone(),
                source_jurisdiction: ack_jurisdiction.clone(),
                watermark: ack_watermark,
                in_flight_drained: ack_drained,
            };
            let full_match = ack_migration == req_migration
                && ack_jurisdiction == req_jurisdiction
                && ack_watermark >= req_min
                && ack_drained;
            prop_assert_eq!(
                export_may_proceed(present.then_some(&ack), &req).is_ok(),
                present && full_match,
            );
        }
    }
}
