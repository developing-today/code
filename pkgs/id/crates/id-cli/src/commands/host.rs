//! `id host`: host mode over a data root of world directories (#10693).

use std::fmt::Write as _;

use anyhow::{Context, Result};
use id_worlds::host::{Census, HostFile, Supervisor, WorldInspection, census_of, inspect_world};

use crate::cli::HostCommand;

/// Run one `id host` subcommand.
///
/// # Errors
///
/// `run` fails when the data root cannot be opened, a recorded-live
/// world fails replay, a `--start` world cannot start (every world
/// already live is drained first), or a world fails to drain on
/// Ctrl-C. `list` and `inspect` fail when the host file is unreadable
/// or names another data root; `inspect` also fails for a world the
/// host file does not record.
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
        HostCommand::Inspect { root, world, json } => {
            let file = HostFile::read_or_rebuild(&root)
                .with_context(|| format!("reading data root {}", root.display()))?;
            let inspection = inspect_world(&root, &file, &world)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inspection)?);
            } else {
                print!("{}", inspection_lines(&inspection));
            }
            Ok(())
        }
    }
}

/// `id host inspect` as lines: one fact per line, then one line per
/// recorded transition.
fn inspection_lines(seen: &WorldInspection) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "world {}", seen.name);
    let _ = writeln!(out, "state {}", seen.state);
    let _ = writeln!(out, "data-root {}", seen.data_root.display());
    let _ = writeln!(out, "dir {}", seen.dir.display());
    match (&seen.journal, &seen.journal_error) {
        (Some(journal), _) => {
            let _ = writeln!(
                out,
                "journal entries={} version={} torn-tail={}",
                journal.entries, journal.version, journal.torn_tail
            );
        }
        (None, Some(error)) => {
            let _ = writeln!(out, "journal unreadable: {error}");
        }
        (None, None) => {
            let _ = writeln!(out, "journal unreadable");
        }
    }
    let _ = writeln!(out, "modules {}", seen.module_count);
    let missing = if seen.storage.missing {
        " (directory missing)"
    } else {
        ""
    };
    let _ = writeln!(
        out,
        "bytes journal={} modules={} other={}{missing}",
        seen.storage.journal_bytes, seen.storage.modules_bytes, seen.storage.other_bytes
    );
    if seen.history.is_empty() {
        let _ = writeln!(out, "history none");
    }
    for change in &seen.history {
        let _ = writeln!(
            out,
            "history {} {} from {}",
            change.at_secs, change.actor, change.prior
        );
    }
    out
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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

        let seen = inspect_world(root.path(), &file, "beta").unwrap();
        let text = inspection_lines(&seen);
        assert!(text.starts_with("world beta\nstate archived\n"), "{text}");
        assert!(text.contains("journal unreadable: "), "{text}");
        assert!(text.contains("modules 0\n"), "{text}");
        assert!(text.contains(" ops from asleep\n"), "{text}");
        let alpha = inspection_lines(&inspect_world(root.path(), &file, "alpha").unwrap());
        assert!(alpha.contains("history none\n"), "{alpha}");
        assert!(inspect_world(root.path(), &file, "gamma").is_err());
    }
}
