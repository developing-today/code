//! `id petname`: read-only review of the petnames one owner gave (#4974).
//!
//! Petnames are private by construction. This reads the local journal and prints it; it never
//! writes, and nothing it reads is published or shared with peers.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::json;

use crate::cli::PetnameCommand;
use id_core::petname::Petnames;

/// Run a petname command against its journal.
///
/// # Errors
///
/// The journal cannot be read or does not load, or the owner is not 64 hex characters.
pub fn cmd_petname(command: PetnameCommand, json: bool) -> Result<()> {
    match command {
        PetnameCommand::List { file, owner } => list(&file, &owner, json),
    }
}

fn list(file: &Path, owner: &str, json: bool) -> Result<()> {
    if owner.len() != 64 || !owner.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--owner must be 64 hex characters");
    }
    let petnames = Petnames::open(file)
        .with_context(|| format!("opening the petnames journal {}", file.display()))?;
    let rows: Vec<(&str, &str)> = petnames.of_owner(owner).collect();
    if json {
        let array: Vec<_> = rows
            .iter()
            .map(|(account, name)| json!({ "account": account, "name": name }))
            .collect();
        println!("{}", serde_json::to_string(&array)?);
    } else {
        for (account, name) in rows {
            println!("account={account} name={name}");
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const ALICE: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    #[test]
    fn listing_reads_only_the_owners_petnames_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("petnames.jsonl");
        {
            let mut store = Petnames::open(&path).unwrap();
            store.set(OWNER, ALICE, "Ali", std::iter::empty()).unwrap();
            store
                .set(OTHER, ALICE, "Alice", std::iter::empty())
                .unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let store = Petnames::open(&path).unwrap();
        let rows: Vec<(&str, &str)> = store.of_owner(OWNER).collect();
        assert_eq!(rows, vec![(ALICE, "Ali")]);
        list(&path, OWNER, false).unwrap();
        list(&path, OWNER, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn owner_must_be_64_hex_characters() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("petnames.jsonl");
        assert!(list(&path, "not-an-account", false).is_err());
    }
}
