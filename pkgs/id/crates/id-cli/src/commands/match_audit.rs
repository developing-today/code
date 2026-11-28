//! CLI for exporting and verifying match-audit v0 transcripts.

use anyhow::{Context, bail};
use p2panda_core::SigningKey;
use serde_json::{Value, json};

use crate::cli::MatchAuditCommand;
use crate::match_audit::{
    ActualValidityV0, AuditReportV0, RuntimeScopeV0, TechnicalVerdictV0,
    export_tic_tac_toe_reference, verify_tic_tac_toe_v0,
};

const MAX_LOG_BYTES: u64 = 128 * 1024 * 1024;

/// Run a match-audit command.
pub fn cmd_match_audit(command: MatchAuditCommand) -> anyhow::Result<()> {
    match command {
        MatchAuditCommand::ExportTicTacToe { output } => {
            // These reproducible demo identities are not production player keys.
            let players = [
                SigningKey::from_bytes(&[0x58; 32]),
                SigningKey::from_bytes(&[0x4f; 32]),
            ];
            let log = export_tic_tac_toe_reference(
                [&players[0], &players[1]],
                "reference-demo-match",
                "reference-demo-world",
                7,
            )?;
            let bytes = log.encode()?;
            std::fs::write(&output, &bytes)
                .with_context(|| format!("write match-audit log {}", output.display()))?;
            let report = verify_tic_tac_toe_v0(&bytes);
            print_report(&report)?;
            require_confirmation(&report)
        }
        MatchAuditCommand::Verify { log } => {
            let length = std::fs::metadata(&log)
                .with_context(|| format!("inspect match-audit log {}", log.display()))?
                .len();
            if length > MAX_LOG_BYTES {
                bail!("match-audit log is {length} bytes; the maximum is {MAX_LOG_BYTES}");
            }
            let bytes = std::fs::read(&log)
                .with_context(|| format!("read match-audit log {}", log.display()))?;
            let report = verify_tic_tac_toe_v0(&bytes);
            print_report(&report)?;
            require_confirmation(&report)
        }
    }
}

fn print_report(report: &AuditReportV0) -> anyhow::Result<()> {
    let value: Value = json!({
        "technical_verdict": technical_label(&report.technical),
        "technical": &report.technical,
        "actual_validity": actual_label(&report.actual),
        "actual": &report.actual,
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn require_confirmation(report: &AuditReportV0) -> anyhow::Result<()> {
    if matches!(
        &report.technical,
        TechnicalVerdictV0::ResultConfirmed {
            runtime: RuntimeScopeV0::PinnedRuntime | RuntimeScopeV0::CrossPlatform,
        }
    ) {
        Ok(())
    } else {
        bail!("{}", technical_label(&report.technical));
    }
}

fn technical_label(verdict: &TechnicalVerdictV0) -> String {
    match verdict {
        TechnicalVerdictV0::InvalidRecord { reason } => {
            format!("INVALID RECORD — {reason:?}")
        }
        TechnicalVerdictV0::ResultConfirmed {
            runtime: RuntimeScopeV0::PinnedRuntime,
        } => "RESULT CONFIRMED — full-history — valid on the pinned runtime".to_owned(),
        TechnicalVerdictV0::ResultConfirmed {
            runtime: RuntimeScopeV0::CrossPlatform,
        } => "RESULT CONFIRMED — full-history — cross-platform".to_owned(),
        TechnicalVerdictV0::ReplayDiverges {
            first_mismatching_interval,
        } => format!(
            "REPLAY DIVERGES — checkpoint interval [{},{}]",
            first_mismatching_interval.start_tick, first_mismatching_interval.end_tick
        ),
        TechnicalVerdictV0::PartiallyVerified {
            intervals,
            runtime: RuntimeScopeV0::PinnedRuntime,
        } => format!("PARTIALLY VERIFIED — intervals {intervals:?} — pinned runtime"),
        TechnicalVerdictV0::PartiallyVerified {
            intervals,
            runtime: RuntimeScopeV0::CrossPlatform,
        } => format!("PARTIALLY VERIFIED — intervals {intervals:?} — cross-platform"),
        TechnicalVerdictV0::Inconclusive { reason } => format!("INCONCLUSIVE — {reason:?}"),
    }
}

fn actual_label(validity: &ActualValidityV0) -> String {
    match validity {
        ActualValidityV0::InconclusiveNoObservedMetadata => {
            "INCONCLUSIVE — no observed metadata".to_owned()
        }
        ActualValidityV0::InconclusiveScreeningUnavailable => {
            "INCONCLUSIVE — screening unavailable".to_owned()
        }
        ActualValidityV0::NoFlags { depth } => format!("NO FLAGS at depth {depth:?}"),
        ActualValidityV0::Flagged { signals } => format!("FLAGGED — {signals:?}"),
    }
}
