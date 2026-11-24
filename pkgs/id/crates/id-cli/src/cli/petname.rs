//! `id petname` subcommand definitions: list the petnames one owner gave.

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id petname`. Petnames are private to their owner and are read here only.
#[derive(Subcommand, Debug)]
pub enum PetnameCommand {
    /// List the petnames one owner gave, read-only, from a `petnames.jsonl` journal.
    ///
    /// Changes no petname state and shares nothing with peers.
    ///
    /// # Examples
    ///
    /// ```bash
    /// id petname list --file world/petnames.jsonl --owner <ACCOUNT>
    /// ```
    List {
        /// The `petnames.jsonl` journal to read.
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
        /// The owner whose petnames to list (64 hex characters).
        #[arg(long, value_name = "ACCOUNT")]
        owner: String,
    },
}
