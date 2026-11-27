//! `id host`: host mode over a data root of world directories (#10693).

use std::fmt::Write as _;

use anyhow::{Context, Result};
use id_worlds::host::{
    Census, HostFile, Supervisor, WakeMode, WorldInspection, census_of, inspect_world,
    settings_body, snapshot_settings_body,
};
use id_worlds::host_snapshots::{SnapshotPolicy, parse_duration_secs};

use crate::cli::HostCommand;

/// Run one `id host` subcommand.
///
/// # Errors
///
/// `run` fails when another host already runs on the data root, the
/// data root cannot be opened, a recorded-live world fails replay, a
/// `--start` world cannot start (every world already live is drained
/// first), or a world fails to drain on Ctrl-C. `start`, `stop`,
/// `archive` and `restore` fail when no host runs on the data root or
/// the host refuses the act. `list` and `inspect` fail when the host
/// file is unreadable or names another data root; `inspect` also fails
/// for a world the host file does not record.
pub async fn cmd_host(command: HostCommand) -> Result<()> {
    match command {
        HostCommand::Run { root, start, actor } => {
            // Bind first: a second host on the same root must be refused
            // before it replays any world the first one holds.
            #[cfg(unix)]
            let listener = id_worlds::host_control::bind(&root)?;
            let mut supervisor = Supervisor::open(&root)
                .await
                .with_context(|| format!("opening data root {}", root.display()))?;
            match host_key(&root).await {
                Ok(key) => supervisor.set_signing_key(key),
                Err(error) => eprintln!("exports are off: no host key ({error:#})"),
            }
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
            #[cfg(unix)]
            {
                let served = id_worlds::host_control::serve(&mut supervisor, &listener, async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
                drop(listener);
                let _ = std::fs::remove_file(id_worlds::host_control::socket_path(&root));
                if let Err(error) = served {
                    supervisor.shutdown().await?;
                    return Err(error);
                }
            }
            #[cfg(not(unix))]
            tokio::signal::ctrl_c()
                .await
                .context("waiting for Ctrl-C")?;
            supervisor.shutdown().await
        }
        HostCommand::Start {
            root,
            world,
            actor,
            json,
        } => operator_act(&root, ControlAct::Start, world, actor, json).await,
        HostCommand::Stop {
            root,
            world,
            actor,
            json,
        } => operator_act(&root, ControlAct::Stop, world, actor, json).await,
        HostCommand::Archive {
            root,
            world,
            actor,
            json,
        } => operator_act(&root, ControlAct::Archive, world, actor, json).await,
        HostCommand::Restore {
            root,
            world,
            actor,
            json,
        } => operator_act(&root, ControlAct::Restore, world, actor, json).await,
        HostCommand::Pin {
            root,
            world,
            off,
            json,
        } => settings_act(&root, Setting::Pinned(!off), world, json).await,
        HostCommand::WakeMode {
            root,
            world,
            mode,
            json,
        } => settings_act(&root, Setting::Wake(mode.parse()?), world, json).await,
        HostCommand::Key { root } => {
            let key = host_key(&root).await?;
            println!("{}", hex_of(key.verifying_key().as_bytes()));
            Ok(())
        }
        HostCommand::Export {
            root,
            world,
            file,
            json,
        } => export_act(&root, world, &file, json).await,
        HostCommand::Import {
            root,
            file,
            source,
            actor,
            json,
        } => import_act(&root, &file, &source, actor, json).await,
        HostCommand::Snapshots {
            root,
            world,
            every,
            keep,
            max_age,
            off,
            inherit,
            json,
        } => {
            let change = snapshot_change(every.as_deref(), keep, max_age.as_deref(), off, inherit)?;
            snapshots_act(&root, world, change, json).await
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

/// The operator acts that need a running host.
#[derive(Clone, Copy, Debug)]
enum ControlAct {
    Start,
    Stop,
    Archive,
    Restore,
}

/// Send one operator act to the host running on `root` and print its
/// answer: `<world> <state>` (and the replay result after a start), or
/// the host's JSON with `--json`.
#[cfg(unix)]
async fn operator_act(
    root: &std::path::Path,
    act: ControlAct,
    world: String,
    actor: String,
    json: bool,
) -> Result<()> {
    use id_worlds::host_control::{ControlRequest, request};
    let message = match act {
        ControlAct::Start => ControlRequest::Start { world, actor },
        ControlAct::Stop => ControlRequest::Stop { world, actor },
        ControlAct::Archive => ControlRequest::Archive { world, actor },
        ControlAct::Restore => ControlRequest::Restore { world, actor },
    };
    let answer = request(root, &message).await?;
    if !answer.ok {
        anyhow::bail!(
            "{}",
            answer
                .error
                .unwrap_or_else(|| "the host refused without a reason".to_owned())
        );
    }
    let body = answer.body.unwrap_or_default();
    if json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print!("{}", act_line(&body));
    }
    Ok(())
}

#[cfg(not(unix))]
async fn operator_act(
    _root: &std::path::Path,
    _act: ControlAct,
    _world: String,
    _actor: String,
    _json: bool,
) -> Result<()> {
    anyhow::bail!("reaching a running host needs its Unix control socket")
}

/// The host's export signing key: `host.key` in the data root, created
/// owner-only when missing.
pub(crate) async fn host_key(root: &std::path::Path) -> Result<ed25519_dalek::SigningKey> {
    let path = root.join(id_worlds::host::HOST_KEY_NAME);
    let path = path
        .to_str()
        .with_context(|| format!("{} is not a UTF-8 path", path.display()))?;
    let key = id_store::store::load_or_create_keypair(path).await?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&key.to_bytes()))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// `exported|restored <world> entries=<n> source=<key> <file>`.
fn export_line(verb: &str, body: &serde_json::Value) -> String {
    format!(
        "{verb} {} entries={} source={} {}\n",
        body["world"].as_str().unwrap_or("?"),
        body["entries"].as_u64().unwrap_or(0),
        body["source"].as_str().unwrap_or("?"),
        body["path"].as_str().unwrap_or("?"),
    )
}

fn print_body(verb: &str, body: &serde_json::Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(body)?);
    } else {
        print!("{}", export_line(verb, body));
    }
    Ok(())
}

/// Export a live world through the host running on `root`.
#[cfg(unix)]
async fn export_act(
    root: &std::path::Path,
    world: String,
    file: &std::path::Path,
    json: bool,
) -> Result<()> {
    use id_worlds::host_control::{ControlRequest, request};
    let path =
        std::path::absolute(file).with_context(|| format!("resolving {}", file.display()))?;
    let answer = request(root, &ControlRequest::Export { world, path }).await?;
    if !answer.ok {
        anyhow::bail!(
            "{}",
            answer
                .error
                .unwrap_or_else(|| "the host refused without a reason".to_owned())
        );
    }
    print_body("exported", &answer.body.unwrap_or_default(), json)
}

#[cfg(not(unix))]
async fn export_act(
    _root: &std::path::Path,
    _world: String,
    _file: &std::path::Path,
    _json: bool,
) -> Result<()> {
    anyhow::bail!("reaching a running host needs its Unix control socket")
}

/// Restore an export through the host running on `root`, or into the data
/// root directly when no host answers.
async fn import_act(
    root: &std::path::Path,
    file: &std::path::Path,
    source: &str,
    actor: String,
    json: bool,
) -> Result<()> {
    let path =
        std::path::absolute(file).with_context(|| format!("resolving {}", file.display()))?;
    #[cfg(unix)]
    {
        use id_worlds::host_control::{ControlRequest, parse_source, try_request};
        parse_source(source)?;
        let message = ControlRequest::Import {
            path: path.clone(),
            source: source.to_owned(),
            actor: actor.clone(),
        };
        if let Some(answer) = try_request(root, &message).await? {
            if !answer.ok {
                anyhow::bail!(
                    "{}",
                    answer
                        .error
                        .unwrap_or_else(|| "the host refused without a reason".to_owned())
                );
            }
            return print_body("restored", &answer.body.unwrap_or_default(), json);
        }
    }
    let pinned: [u8; 32] = id_worlds::world_session::decode_hex(source)
        .and_then(|bytes| bytes.try_into().ok())
        .with_context(|| format!("source {source:?} is not 64 hex digits"))?;
    let manifest = id_worlds::host::import_into_root(root, &path, pinned, &actor).await?;
    let mut body = serde_json::to_value(&manifest)?;
    body["path"] = serde_json::json!(path);
    print_body("restored", &body, json)
}

/// A per-world setting an operator changes (#10694).
#[derive(Clone, Copy, Debug)]
enum Setting {
    Pinned(bool),
    Wake(WakeMode),
}

/// Change one world's setting on the host running on `root`, or in its
/// host file when no host answers, and print the world's settings.
async fn settings_act(
    root: &std::path::Path,
    setting: Setting,
    world: String,
    json: bool,
) -> Result<()> {
    // The host's answer, when a host answers: its body, or why it refused.
    #[cfg(unix)]
    let answered: Option<std::result::Result<serde_json::Value, String>> = {
        use id_worlds::host_control::{ControlRequest, try_request};
        let message = match setting {
            Setting::Pinned(pinned) => ControlRequest::SetPinned {
                world: world.clone(),
                pinned,
            },
            Setting::Wake(wake) => ControlRequest::SetWake {
                world: world.clone(),
                wake,
            },
        };
        try_request(root, &message).await?.map(|answer| {
            if answer.ok {
                Ok(answer.body.unwrap_or_default())
            } else {
                Err(answer
                    .error
                    .unwrap_or_else(|| "the host refused without a reason".to_owned()))
            }
        })
    };
    #[cfg(not(unix))]
    let answered: Option<std::result::Result<serde_json::Value, String>> = None;
    let body = match answered {
        Some(Ok(body)) => body,
        Some(Err(error)) => anyhow::bail!("{error}"),
        None => {
            let mut file = HostFile::read_or_rebuild(root)
                .with_context(|| format!("reading data root {}", root.display()))?;
            match setting {
                Setting::Pinned(pinned) => file.set_pinned(&world, pinned)?,
                Setting::Wake(wake) => file.set_wake(&world, wake)?,
            }
            file.save(root)?;
            settings_body(&file, &world)?
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print!("{}", settings_line(&body));
    }
    Ok(())
}

/// A snapshot-policy change asked for on the command line: `None` only
/// shows, `Some(None)` clears a world's own policy, `Some(Some(_))` sets.
type SnapshotChange = Option<Option<SnapshotPolicy>>;

fn snapshot_change(
    every: Option<&str>,
    keep: Option<u32>,
    max_age: Option<&str>,
    off: bool,
    inherit: bool,
) -> Result<SnapshotChange> {
    if inherit {
        return Ok(Some(None));
    }
    if off {
        return Ok(Some(Some(SnapshotPolicy::OFF)));
    }
    let Some(every) = every else {
        return Ok(None);
    };
    let policy = SnapshotPolicy {
        every_secs: parse_duration_secs(every)?,
        keep: keep.context("--every needs --keep")?,
        max_age_secs: max_age.map(parse_duration_secs).transpose()?,
    };
    anyhow::ensure!(
        policy.is_on(),
        "--every must be more than 0; use --off to stop"
    );
    policy.validate()?;
    Ok(Some(Some(policy)))
}

/// Show or change the snapshot policy on the host running on `root`, or
/// in its host file when no host answers, and print the settings.
async fn snapshots_act(
    root: &std::path::Path,
    world: Option<String>,
    change: SnapshotChange,
    json: bool,
) -> Result<()> {
    #[cfg(unix)]
    let answered: Option<std::result::Result<serde_json::Value, String>> = {
        use id_worlds::host_control::{ControlRequest, try_request};
        let message = match change {
            Some(policy) => ControlRequest::SetSnapshots {
                world: world.clone(),
                policy,
            },
            None => ControlRequest::Snapshots {
                world: world.clone(),
            },
        };
        try_request(root, &message).await?.map(|answer| {
            if answer.ok {
                Ok(answer.body.unwrap_or_default())
            } else {
                Err(answer
                    .error
                    .unwrap_or_else(|| "the host refused without a reason".to_owned()))
            }
        })
    };
    #[cfg(not(unix))]
    let answered: Option<std::result::Result<serde_json::Value, String>> = None;
    let body = match answered {
        Some(Ok(body)) => body,
        Some(Err(error)) => anyhow::bail!("{error}"),
        None => {
            let mut file = HostFile::read_or_rebuild(root)
                .with_context(|| format!("reading data root {}", root.display()))?;
            if let Some(policy) = change {
                file.set_snapshot_policy(world.as_deref(), policy)?;
                file.save(root)?;
            }
            snapshot_settings_body(root, &file, world.as_deref())?
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print!("{}", snapshot_lines(&body));
    }
    Ok(())
}

/// `snapshots <world|host> every=<secs> keep=<n> [max-age=<secs>] dir=<path>`
/// (or `off`), then one `set <time> entries=<n> bytes=<n> <path>` line per
/// set.
fn snapshot_lines(body: &serde_json::Value) -> String {
    let who = body["world"].as_str().unwrap_or("host").to_owned();
    let policy = &body["policy"];
    let every = policy["every_secs"].as_u64().unwrap_or(0);
    let mut out = if every == 0 {
        format!("snapshots {who} off")
    } else {
        let mut line = format!(
            "snapshots {who} every={every}s keep={}",
            policy["keep"].as_u64().unwrap_or(0)
        );
        if let Some(age) = policy["max_age_secs"].as_u64() {
            let _ = write!(line, " max-age={age}s");
        }
        line
    };
    if body["own_policy"] == serde_json::Value::Bool(false) {
        out.push_str(" (host default)");
    }
    let _ = writeln!(out, " dir={}", body["dir"].as_str().unwrap_or("?"));
    for set in body["sets"].as_array().map_or(&[][..], Vec::as_slice) {
        let _ = writeln!(
            out,
            "set {} entries={} bytes={} {}",
            set["at_secs"].as_u64().unwrap_or(0),
            set["entries"].as_u64().unwrap_or(0),
            set["bytes"].as_u64().unwrap_or(0),
            set["path"].as_str().unwrap_or("?")
        );
    }
    out
}

/// `<world> <state> pinned=<bool> wake=<mode>`.
fn settings_line(body: &serde_json::Value) -> String {
    let text = |key: &str| body[key].as_str().unwrap_or("?").to_owned();
    format!(
        "{} {} pinned={} wake={}\n",
        text("world"),
        text("state"),
        body["pinned"].as_bool().unwrap_or(false),
        text("wake")
    )
}

/// `<world> <state>`, plus the replay result when the act was a start.
fn act_line(body: &serde_json::Value) -> String {
    let text = |key: &str| body[key].as_str().unwrap_or("?").to_owned();
    let mut line = format!("{} {}", text("world"), text("state"));
    if let Some(entries) = body["journal_entries"].as_u64() {
        let _ = write!(
            line,
            " journal-entries={entries} replayed-inputs={}",
            body["replayed_inputs"].as_u64().unwrap_or(0)
        );
    }
    if let Some(error) = body["program_error"].as_str() {
        let _ = write!(line, " program-unavailable: {error}");
    }
    line.push('\n');
    line
}

/// `id host inspect` as lines: one fact per line, then one line per
/// recorded transition.
fn inspection_lines(seen: &WorldInspection) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "world {}", seen.name);
    let _ = writeln!(out, "state {}", seen.state);
    let _ = writeln!(out, "pinned {}", seen.pinned);
    let _ = writeln!(out, "wake {}", seen.wake);
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
    let ceilings = &seen.ceilings;
    let _ = writeln!(
        out,
        "ceilings participants={} chat-bytes={} input-bytes={} presentation-bytes={} \
         snapshot-bytes={} retained-events={} checkpoint-every={} memory-bytes={}",
        ceilings.participants,
        ceilings.chat_bytes,
        ceilings.input_bytes,
        ceilings.presentation_bytes,
        ceilings.snapshot_bytes,
        ceilings.retained_events,
        ceilings.checkpoint_every,
        ceilings.total_memory_bytes
    );
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
    match &seen.last_recovery {
        None => {
            let _ = writeln!(out, "recovery none");
        }
        Some(recovery) => {
            let _ = match &recovery.error {
                Some(error) => writeln!(
                    out,
                    "recovery {} failed torn-tail={}: {error}",
                    recovery.at_secs, recovery.torn_tail
                ),
                None => writeln!(
                    out,
                    "recovery {} replayed sequence={} entries={} inputs={} torn-tail={}",
                    recovery.at_secs,
                    recovery.sequence,
                    recovery.journal_entries,
                    recovery.replayed_inputs,
                    recovery.torn_tail
                ),
            };
            if let Some(error) = &recovery.program_error {
                let _ = writeln!(out, "recovery program unavailable: {error}");
            }
        }
    }
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
            "{} {} journal={} modules={} other={} snapshots={}:{} changed={last}{missing}",
            row.name,
            row.state,
            row.journal_bytes,
            row.modules_bytes,
            row.other_bytes,
            row.snapshot_sets,
            row.snapshot_bytes
        );
    }
    for (state, totals) in [
        ("live", &census.live),
        ("asleep", &census.asleep),
        ("archived", &census.archived),
    ] {
        let _ = writeln!(
            out,
            "total {state}: worlds={} journal={} modules={} other={} snapshots={}",
            totals.worlds,
            totals.journal_bytes,
            totals.modules_bytes,
            totals.other_bytes,
            totals.snapshot_bytes
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
            text.contains("alpha asleep journal=0 modules=0 other=0 snapshots=0:0 changed="),
            "{text}"
        );
        assert!(
            text.contains("beta archived journal=0 modules=0 other=0 snapshots=0:0 changed="),
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
        assert!(
            text.contains("\nceilings participants=256 chat-bytes=2048 input-bytes=16384 "),
            "{text}"
        );
        assert!(text.contains(" ops from asleep\n"), "{text}");
        let alpha = inspection_lines(&inspect_world(root.path(), &file, "alpha").unwrap());
        assert!(alpha.contains("history none\n"), "{alpha}");
        assert!(alpha.contains("\nrecovery none\n"), "{alpha}");
        let mut recovered = seen.clone();
        recovered.last_recovery = Some(id_worlds::host::RecoveryRecord {
            at_secs: 7,
            torn_tail: true,
            sequence: 3,
            journal_entries: 4,
            replayed_inputs: 2,
            program_error: Some("no module".to_owned()),
            error: None,
        });
        let text = inspection_lines(&recovered);
        assert!(
            text.contains(
                "\nrecovery 7 replayed sequence=3 entries=4 inputs=2 torn-tail=true\n\
                 recovery program unavailable: no module\n"
            ),
            "{text}"
        );
        recovered.last_recovery = Some(id_worlds::host::RecoveryRecord {
            at_secs: 9,
            error: Some("version 99".to_owned()),
            ..id_worlds::host::RecoveryRecord::default()
        });
        let text = inspection_lines(&recovered);
        assert!(
            text.contains("\nrecovery 9 failed torn-tail=false: version 99\n"),
            "{text}"
        );
        assert!(inspect_world(root.path(), &file, "gamma").is_err());
    }

    #[tokio::test]
    async fn snapshot_policy_edits_the_host_file_when_no_host_answers() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = root.path().join("w");
        std::fs::create_dir_all(dir.join("modules")).unwrap();
        std::fs::write(dir.join("journal.jsonl"), "").unwrap();
        let hourly = snapshot_change(Some("1h"), Some(3), Some("2d"), false, false).unwrap();
        snapshots_act(root.path(), None, hourly, false)
            .await
            .unwrap();
        let file = HostFile::load(root.path()).unwrap();
        let policy = SnapshotPolicy {
            every_secs: 3_600,
            keep: 3,
            max_age_secs: Some(172_800),
        };
        assert_eq!(file.snapshots, Some(policy));
        assert_eq!(file.snapshot_policy("w"), policy);

        let off = snapshot_change(None, None, None, true, false).unwrap();
        snapshots_act(root.path(), Some("w".to_owned()), off, false)
            .await
            .unwrap();
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.snapshot_policy("w"), SnapshotPolicy::OFF);
        let body = snapshot_settings_body(root.path(), &file, Some("w")).unwrap();
        let text = snapshot_lines(&body);
        assert!(text.starts_with("snapshots w off dir="), "{text}");
        let text = snapshot_lines(&snapshot_settings_body(root.path(), &file, None).unwrap());
        assert!(
            text.starts_with("snapshots host every=3600s keep=3 max-age=172800s dir="),
            "{text}"
        );

        let inherit = snapshot_change(None, None, None, false, true).unwrap();
        snapshots_act(root.path(), Some("w".to_owned()), inherit, true)
            .await
            .unwrap();
        let file = HostFile::load(root.path()).unwrap();
        assert_eq!(file.snapshot_policy("w"), policy);
        let text = snapshot_lines(&snapshot_settings_body(root.path(), &file, Some("w")).unwrap());
        assert!(text.contains(" (host default) dir="), "{text}");

        // Bad cadences are refused before anything is written.
        assert!(snapshot_change(Some("30s"), Some(1), None, false, false).is_err());
        assert!(snapshot_change(Some("0"), Some(1), None, false, false).is_err());
        assert!(snapshot_change(Some("6w"), Some(1), None, false, false).is_err());
        assert!(snapshot_change(Some("1h"), Some(0), None, false, false).is_err());
        assert_eq!(
            snapshot_change(None, None, None, false, false).unwrap(),
            None
        );
        // Only shows: the file stays as it was.
        snapshots_act(root.path(), None, None, false).await.unwrap();
        assert_eq!(HostFile::load(root.path()).unwrap(), file);
        assert!(
            snapshots_act(root.path(), Some("nobody".to_owned()), None, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn pin_and_wake_mode_edit_the_host_file_when_no_host_answers() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = root.path().join("w");
        std::fs::create_dir_all(dir.join("modules")).unwrap();
        std::fs::write(dir.join("journal.jsonl"), "").unwrap();
        // No host file yet: the first setting writes one.
        settings_act(root.path(), Setting::Pinned(true), "w".to_owned(), false)
            .await
            .unwrap();
        settings_act(
            root.path(),
            Setting::Wake(WakeMode::Operator),
            "w".to_owned(),
            true,
        )
        .await
        .unwrap();
        let file = HostFile::load(root.path()).unwrap();
        assert!(file.worlds["w"].pinned);
        assert_eq!(file.worlds["w"].wake, WakeMode::Operator);
        let text = inspection_lines(&inspect_world(root.path(), &file, "w").unwrap());
        assert!(text.contains("\npinned true\nwake operator\n"), "{text}");

        settings_act(root.path(), Setting::Pinned(false), "w".to_owned(), false)
            .await
            .unwrap();
        assert!(!HostFile::load(root.path()).unwrap().worlds["w"].pinned);
        assert!(
            settings_act(
                root.path(),
                Setting::Pinned(true),
                "nobody".to_owned(),
                false
            )
            .await
            .is_err()
        );
        assert_eq!(
            settings_line(&settings_body(&file, "w").unwrap()),
            "w asleep pinned=true wake=operator\n"
        );
    }

    #[tokio::test]
    async fn host_key_is_stable_and_import_refuses_a_bad_source() {
        let root = tempfile::TempDir::new().unwrap();
        let first = host_key(root.path()).await.unwrap();
        let again = host_key(root.path()).await.unwrap();
        assert_eq!(first.verifying_key(), again.verifying_key());
        assert_eq!(hex_of(first.verifying_key().as_bytes()).len(), 64);
        let refused = import_act(
            root.path(),
            &root.path().join("missing.export"),
            "not-hex",
            "ops".to_owned(),
            false,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(refused.contains("64 hex digits"), "{refused}");
        let body = serde_json::json!({
            "world": "w",
            "entries": 4,
            "source": "ab",
            "path": "/x/w.export",
        });
        assert_eq!(
            export_line("exported", &body),
            "exported w entries=4 source=ab /x/w.export\n"
        );
    }

    #[test]
    fn act_lines_name_the_world_its_state_and_any_replay_result() {
        let stopped = serde_json::json!({ "world": "w", "state": "asleep" });
        assert_eq!(act_line(&stopped), "w asleep\n");
        let started = serde_json::json!({
            "world": "w",
            "state": "live",
            "journal_entries": 3,
            "replayed_inputs": 2,
            "program_error": "no module",
        });
        assert_eq!(
            act_line(&started),
            "w live journal-entries=3 replayed-inputs=2 program-unavailable: no module\n"
        );
    }
}
