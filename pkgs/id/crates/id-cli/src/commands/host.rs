//! `id host`: host mode over a data root of world directories (#10693).

use std::fmt::Write as _;

use anyhow::{Context, Result};
use id_worlds::host::{Census, HostFile, Supervisor, census_of};

use crate::cli::HostCommand;

/// Run one `id host` subcommand.
///
/// # Errors
///
/// `run` fails when the data root cannot be opened, a recorded-live
/// world fails replay, a `--start` world cannot start (every world
/// already live is drained first), or a world fails to drain on
/// Ctrl-C. `list` fails when the host file is unreadable or names
/// another data root.
pub async fn cmd_host(command: HostCommand) -> Result<()> {
    match command {
        HostCommand::Run { root, start, actor } => {
            let mut supervisor = Supervisor::open(&root)
                .await
                .with_context(|| format!("opening data root {}", root.display()))?;
            for name in &start {
                if let Err(error) = supervisor.start(name, &actor).await {
                    supervisor.shutdown().await?;
                    return Err(error);
                }
            }
            print!("{}", census_lines(&supervisor.census(), supervisor.file()));
            println!(
                "hosting {} live world(s) from {}; Ctrl-C to stop",
                supervisor.live_names().len(),
                root.display()
            );
            tokio::signal::ctrl_c()
                .await
                .context("waiting for Ctrl-C")?;
            supervisor.shutdown().await
        }
        HostCommand::List { root, json } => {
            let file = HostFile::read_or_rebuild(&root)
                .with_context(|| format!("reading data root {}", root.display()))?;
            let census = census_of(&root, &file);
            if json {
                println!("{}", serde_json::to_string_pretty(&census)?);
            } else {
                print!("{}", census_lines(&census, &file));
            }
            Ok(())
        }
    }
}

/// One line per world (name, state, bytes, last state change), then one
/// total line per lifecycle state.
fn census_lines(census: &Census, file: &HostFile) -> String {
    let mut out = String::new();
    for row in &census.worlds {
        let last = file
            .worlds
            .get(&row.name)
            .and_then(|record| record.history.last())
            .map_or_else(
                || "never".to_owned(),
                |change| format!("{} by {}", change.at_secs, change.actor),
            );
        let missing = if row.missing {
            " (directory missing)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{} {} journal={} modules={} other={} changed={last}{missing}",
            row.name, row.state, row.journal_bytes, row.modules_bytes, row.other_bytes
        );
    }
    for (state, totals) in [
        ("live", &census.live),
        ("asleep", &census.asleep),
        ("archived", &census.archived),
    ] {
        let _ = writeln!(
            out,
            "total {state}: worlds={} journal={} modules={} other={}",
            totals.worlds, totals.journal_bytes, totals.modules_bytes, totals.other_bytes
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use id_worlds::host::LifecycleState;

    #[tokio::test]
    async fn census_lines_name_every_world_and_every_state_total() {
        let root = tempfile::TempDir::new().unwrap();
        for name in ["alpha", "beta"] {
            let dir = root.path().join(name);
            std::fs::create_dir_all(dir.join("modules")).unwrap();
            std::fs::write(dir.join("journal.jsonl"), "").unwrap();
        }
        let mut supervisor = Supervisor::open(root.path()).await.unwrap();
        supervisor.archive("beta", "ops").unwrap();
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.worlds["beta"].state, LifecycleState::Archived);
        let text = census_lines(&census_of(root.path(), &file), &file);
        assert!(
            text.contains("alpha asleep journal=0 modules=0 other=0 changed="),
            "{text}"
        );
        assert!(
            text.contains("beta archived journal=0 modules=0 other=0 changed="),
            "{text}"
        );
        assert!(text.contains(" by ops\n"), "{text}");
        assert!(text.contains("total live: worlds=0"), "{text}");
        assert!(text.contains("total asleep: worlds=1"), "{text}");
        assert!(text.contains("total archived: worlds=1"), "{text}");
    }
}
