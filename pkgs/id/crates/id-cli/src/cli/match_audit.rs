//! `id match-audit` subcommand definitions.

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id match-audit`.
#[cfg(feature = "sandbox")]
#[derive(Subcommand, Debug)]
pub enum MatchAuditCommand {
    /// Export a reference tic-tac-toe transcript using synthetic demo keys.
    #[command(name = "export-tic-tac-toe")]
    ExportTicTacToe {
        /// Write the binary v0 log to this path.
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify a v0 log against the pinned bundled tic-tac-toe program.
    Verify {
        /// Binary match-audit v0 log.
        log: PathBuf,
    },
}
