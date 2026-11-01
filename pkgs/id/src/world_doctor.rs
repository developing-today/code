//! `id world doctor`: mechanically diagnose a broken world setup and print
//! the fix for each problem.
//!
//! The checks cover the six things that fail most often and are diagnosable
//! without human judgement: the Roc toolchain, the platform build, capability
//! ceilings, the execution profile, the journal format, and the clock. Each
//! check is a pure function over a [`DoctorInputs`] value, so a failing case
//! can be produced in a unit test by pointing one input at something broken.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::world::{JOURNAL_VERSION, MAX_JOURNAL_VERSION};
use crate::world_limits::WorldExecutionProfile;

/// How bad a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Everything the check looks at is in order.
    Ok,
    /// Something is off but does not stop a world from running.
    Warn,
    /// Something is broken enough to block a world from running.
    Fail,
}

/// One check's verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable check id: `toolchain`, `platform`, `ceiling`, `profile`,
    /// `journal`, `clock`.
    pub check: String,
    /// How bad the finding is.
    pub severity: Severity,
    /// What was observed.
    pub detail: String,
    /// What to do about it, empty when nothing is wrong.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fix: String,
}

/// The whole `id world doctor` report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// One finding per check, in a fixed order.
    pub findings: Vec<Finding>,
}

impl DoctorReport {
    /// Whether every check passed with no warning or failure. This is a
    /// strict summary only; it is NOT the CLI exit condition — a `Warn`
    /// finding ("does not stop a world from running", see [`Severity::Warn`])
    /// must not fail `id world doctor`. Use [`DoctorReport::exit_result`]
    /// for the exit decision.
    #[must_use]
    pub fn healthy(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| finding.severity == Severity::Ok)
    }

    /// How many findings are failures (`Severity::Fail`). Warnings and
    /// passes are not problems for exit purposes.
    #[must_use]
    pub fn failure_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Fail)
            .count()
    }

    /// Whether any check failed.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failure_count() > 0
    }

    /// The CLI exit decision for a finished report: `Ok` when no check
    /// failed (warnings included), otherwise an error naming the number of
    /// FAILED checks — never the total number of checks run.
    ///
    /// # Errors
    ///
    /// Any finding has `Severity::Fail`.
    pub fn exit_result(&self) -> Result<()> {
        let failures = self.failure_count();
        anyhow::ensure!(failures == 0, "world doctor found {failures} problem(s)");
        Ok(())
    }

    /// Render the report as terminal text, one finding per line.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for finding in &self.findings {
            let tag = match finding.severity {
                Severity::Ok => "ok",
                Severity::Warn => "warn",
                Severity::Fail => "fail",
            };
            out.push_str(&format!(
                "{tag:4} {:<9} {}\n",
                finding.check, finding.detail
            ));
            if !finding.fix.is_empty() {
                out.push_str(&format!("      fix: {}\n", finding.fix));
            }
        }
        out
    }
}

/// The values each check reads. Every field is optional so a check can report
/// a missing input as its own failure rather than erroring out.
#[derive(Clone, Debug, Default)]
pub struct DoctorInputs {
    /// Roc compiler binary to check (a path or a bare name on `PATH`).
    pub roc_bin: Option<String>,
    /// The binary [`crate::world_compile::locate_roc`] resolved, injected by
    /// the caller so tests need not depend on the host's installed compiler.
    /// When `None`, the check resolves it itself.
    pub roc_resolved: Option<Result<String, String>>,
    /// World platform directory (`platform.roc` plus `targets/wasm32/host.wasm`).
    pub platform_dir: Option<PathBuf>,
    /// Server capability ceiling, if the server is configured with one.
    pub caps_ceiling: Option<Vec<String>>,
    /// World execution profile to validate, if one is configured.
    pub profile: Option<WorldExecutionProfile>,
    /// Path to a world journal (`journal.jsonl`) to check, if any.
    pub journal: Option<PathBuf>,
    /// Clock reading the check should use, in unix milliseconds. `None` (or
    /// zero) means the clock is broken.
    pub now_ms: Option<u64>,
}

fn ok(check: &str, detail: impl Into<String>) -> Finding {
    Finding {
        check: check.to_owned(),
        severity: Severity::Ok,
        detail: detail.into(),
        fix: String::new(),
    }
}

fn warn(check: &str, detail: impl Into<String>, fix: impl Into<String>) -> Finding {
    Finding {
        check: check.to_owned(),
        severity: Severity::Warn,
        detail: detail.into(),
        fix: fix.into(),
    }
}

fn fail(check: &str, detail: impl Into<String>, fix: impl Into<String>) -> Finding {
    Finding {
        check: check.to_owned(),
        severity: Severity::Fail,
        detail: detail.into(),
        fix: fix.into(),
    }
}

/// Run every check over `inputs`.
#[must_use]
pub fn run(inputs: &DoctorInputs) -> DoctorReport {
    DoctorReport {
        findings: vec![
            check_toolchain(inputs),
            check_platform(inputs),
            check_ceiling(inputs),
            check_profile(inputs),
            check_journal(inputs),
            check_clock(inputs),
        ],
    }
}

/// The compiler binary exists. When none is configured, the pinned nightly is
/// found by [`crate::world_compile::locate_roc`]; when one is, it must resolve
/// to a real file.
#[must_use]
pub fn check_toolchain(inputs: &DoctorInputs) -> Finding {
    let resolved = match inputs.roc_resolved.clone() {
        Some(resolved) => resolved,
        None => crate::world_compile::locate_roc().map_err(|error| error.to_string()),
    };
    let Some(roc_bin) = inputs.roc_bin.as_deref() else {
        return match resolved {
            Ok(found) => ok("toolchain", format!("Roc compiler {found}")),
            Err(_) => fail(
                "toolchain",
                "no Roc compiler configured",
                "set ID_ROC_BIN, or install the pinned Roc nightly and run id world doctor again",
            ),
        };
    };
    // A configured binary must exist. A bare name resolves through PATH.
    let exists = if roc_bin.contains('/') {
        Path::new(roc_bin).is_file()
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(roc_bin))
            .any(|candidate| candidate.is_file())
    };
    if !exists {
        return fail(
            "toolchain",
            format!("Roc compiler {roc_bin:?} is not available"),
            "install the pinned Roc nightly (see .opencode/plans/phase-11-foundations.md) or set ID_ROC_BIN",
        );
    }
    match resolved {
        Ok(found) if found == roc_bin => ok("toolchain", format!("Roc compiler {found}")),
        Ok(found) => warn(
            "toolchain",
            format!("using {found}, which differs from the configured {roc_bin}"),
            "set ID_ROC_BIN to the pinned nightly so builds are reproducible",
        ),
        Err(_) => warn(
            "toolchain",
            format!("{roc_bin} exists but is not the pinned nightly"),
            "use the pinned Roc nightly so module layouts match the platform",
        ),
    }
}

/// The platform directory is complete: `platform.roc` and the built
/// `targets/wasm32/host.wasm` are both present.
#[must_use]
pub fn check_platform(inputs: &DoctorInputs) -> Finding {
    let Some(platform_dir) = inputs.platform_dir.as_deref() else {
        return fail(
            "platform",
            "no world platform directory",
            "set ID_ROC_PLATFORM to a checkout of examples/roc-world",
        );
    };
    if !platform_dir.is_dir() {
        return fail(
            "platform",
            format!("{} is not a directory", platform_dir.display()),
            "set ID_ROC_PLATFORM to a checkout of examples/roc-world",
        );
    }
    if !platform_dir.join("platform.roc").is_file() {
        return fail(
            "platform",
            format!("{} has no platform.roc", platform_dir.display()),
            "check out examples/roc-world",
        );
    }
    if !platform_dir.join("targets/wasm32/host.wasm").is_file() {
        return fail(
            "platform",
            format!("{} has no targets/wasm32/host.wasm", platform_dir.display()),
            "run examples/roc-world/build-host.sh to build the platform host",
        );
    }
    ok("platform", format!("platform {}", platform_dir.display()))
}

/// The capability ceiling names only known capabilities and no `*`.
#[must_use]
pub fn check_ceiling(inputs: &DoctorInputs) -> Finding {
    let Some(ceiling) = inputs.caps_ceiling.as_deref() else {
        return ok("ceiling", "no capability ceiling (whole catalog)");
    };
    if ceiling.iter().any(|name| name == "*") {
        return fail(
            "ceiling",
            "the capability ceiling contains `*`",
            "a ceiling lists names only; `*` is a grant shorthand — remove it",
        );
    }
    if let Some(unknown) = ceiling
        .iter()
        .find(|name| !crate::world_caps::is_known(name))
    {
        return fail(
            "ceiling",
            format!("unknown capability {unknown:?} in the ceiling"),
            "remove it, or register the provider so the name is known",
        );
    }
    ok("ceiling", format!("{} capability names", ceiling.len()))
}

/// The execution profile's requested bounds are nonzero and above their
/// minimums, so `resolve` can accept them.
#[must_use]
pub fn check_profile(inputs: &DoctorInputs) -> Finding {
    let Some(profile) = inputs.profile else {
        return ok("profile", "no world execution profile (server defaults)");
    };
    let mut problems: Vec<&str> = Vec::new();
    if profile.wasm_fuel == Some(0) {
        problems.push("wasm_fuel is zero");
    }
    if profile
        .wasm_memory_bytes
        .is_some_and(|bytes| bytes < 64 * 1024)
    {
        problems.push("wasm_memory_bytes is below one Wasm page (64 KiB)");
    }
    if profile.wasm_message_bytes == Some(0) {
        problems.push("wasm_message_bytes is zero");
    }
    if profile
        .native_memory_bytes
        .is_some_and(|bytes| bytes < 16 * 1024 * 1024)
    {
        problems.push("native_memory_bytes is below 16 MiB");
    }
    if profile.native_cpu_seconds == Some(0) {
        problems.push("native_cpu_seconds is zero");
    }
    if profile.native_call_deadline_ms == Some(0) {
        problems.push("native_call_deadline_ms is zero");
    }
    if problems.is_empty() {
        return ok("profile", "execution profile bounds are valid");
    }
    fail(
        "profile",
        format!("execution profile is invalid: {}", problems.join("; ")),
        "set each bound to a positive value at or above its minimum",
    )
}

/// The journal parses, its first entry is `Created` and its version is one
/// this binary can read.
#[must_use]
pub fn check_journal(inputs: &DoctorInputs) -> Finding {
    let Some(journal) = inputs.journal.as_deref() else {
        return ok("journal", "no journal to check");
    };
    match inspect_journal_file(journal) {
        Ok(summary) if summary.version > MAX_JOURNAL_VERSION => fail(
            "journal",
            format!(
                "{}: journal format version {} is newer than this binary supports",
                journal.display(),
                summary.version
            ),
            format!(
                "use an id binary that supports journal format version {} before opening this world",
                summary.version
            ),
        ),
        Ok(summary) if summary.version < JOURNAL_VERSION => fail(
            "journal",
            format!(
                "{}: journal format version {} is older than this binary supports",
                journal.display(),
                summary.version
            ),
            "restore the journal from a backup, or recreate the world",
        ),
        Ok(summary) if summary.torn_tail => warn(
            "journal",
            format!(
                "{}: {} entries, format version {}; an unterminated final entry will be dropped on open",
                journal.display(),
                summary.entries,
                summary.version
            ),
            "opening the world will drop the incomplete final entry and continue",
        ),
        Ok(summary) => ok(
            "journal",
            format!(
                "{} entries, format version {}",
                summary.entries, summary.version
            ),
        ),
        Err(error) => fail(
            "journal",
            format!("{}: {error}", journal.display()),
            "restore the journal from a backup, or recreate the world",
        ),
    }
}

struct JournalSummary {
    entries: usize,
    version: u32,
    torn_tail: bool,
}

/// Parse a journal and record an incomplete final append without treating it
/// as corruption. The opener drops that tail before appending again.
fn inspect_journal_file(path: &Path) -> Result<JournalSummary> {
    let bytes =
        std::fs::read(path).map_err(|error| anyhow::anyhow!("cannot read journal: {error}"))?;
    let has_unterminated_tail = !bytes.is_empty() && !bytes.ends_with(b"\n");
    let last_line_index = bytes.iter().filter(|byte| **byte == b'\n').count();
    let mut entries = 0usize;
    let mut version = None;
    let mut torn_tail = false;
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if index == last_line_index && has_unterminated_tail && !line.is_empty() {
            torn_tail = true;
            break;
        }
        if line.is_empty() {
            continue;
        }
        let entry: crate::world::JournalEntry = serde_json::from_slice(line)
            .map_err(|error| anyhow::anyhow!("entry {} is malformed: {error}", index + 1))?;
        match &entry {
            crate::world::JournalEntry::Created {
                world_id: _,
                version: created_version,
            } => {
                anyhow::ensure!(
                    entries == 0,
                    "entry {} is `Created`, but it is not the first entry",
                    index + 1
                );
                version = Some(*created_version);
            }
            _ if entries == 0 => {
                anyhow::bail!("the first entry is not `Created`");
            }
            _ => {}
        }
        entries += 1;
    }
    anyhow::ensure!(entries > 0, "journal is empty");
    Ok(JournalSummary {
        entries,
        version: version.unwrap_or(JOURNAL_VERSION),
        torn_tail,
    })
}

/// Parse a journal file and check its format, without opening it for append.
///
/// Returns the entry count and the `Created` version.
///
/// # Errors
///
/// A missing file, a first entry that is not `Created`, a version this binary
/// does not know, or any malformed complete entry. An unterminated final
/// append is ignored, matching `FileJournal::open`.
pub fn validate_journal_file(path: &Path) -> Result<(usize, u32)> {
    let summary = inspect_journal_file(path)?;
    anyhow::ensure!(
        (JOURNAL_VERSION..=MAX_JOURNAL_VERSION).contains(&summary.version),
        "journal format version {} is not one this binary supports ({JOURNAL_VERSION}..={MAX_JOURNAL_VERSION})",
        summary.version
    );
    Ok((summary.entries, summary.version))
}

/// The clock reads a positive unix time. A zero (or absurd) reading means the
/// system clock is before the epoch, which breaks every schedule and expiry.
#[must_use]
pub fn check_clock(inputs: &DoctorInputs) -> Finding {
    match inputs.now_ms {
        Some(now) if now > 0 => ok("clock", format!("{now} ms since the epoch")),
        _ => fail(
            "clock",
            "the system clock is before the unix epoch",
            "set the system clock (schedules and expiries depend on it)",
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn inputs() -> DoctorInputs {
        DoctorInputs {
            roc_bin: None,
            roc_resolved: None,
            platform_dir: None,
            caps_ceiling: None,
            profile: None,
            journal: None,
            now_ms: Some(1_700_000_000_000),
        }
    }

    /// Inputs for a host that has a real compiler and a real platform, used by
    /// the "healthy" test. The platform is a synthetic complete directory so
    /// the test does not depend on `examples/roc-world` being built.
    fn healthy_inputs() -> DoctorInputs {
        let dir = std::env::temp_dir().join(format!("id-doctor-healthy-{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("targets/wasm32"));
        let _ = std::fs::write(dir.join("platform.roc"), "platform");
        let _ = std::fs::write(dir.join("targets/wasm32/host.wasm"), "wasm");
        let roc_path = dir.join("roc");
        let _ = std::fs::write(&roc_path, "roc binary");
        #[cfg(unix)]
        let _ = std::fs::set_permissions(
            &roc_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        );
        let roc = Some(roc_path.to_string_lossy().into_owned());
        let roc_resolved = roc
            .clone()
            .map(Ok)
            .or_else(|| Some(Err("no roc binary".to_owned())));
        DoctorInputs {
            roc_bin: roc,
            roc_resolved,
            platform_dir: Some(dir),
            caps_ceiling: None,
            profile: None,
            journal: None,
            now_ms: Some(1_700_000_000_000),
        }
    }

    #[test]
    fn a_healthy_input_set_reports_all_ok() {
        let report = run(&healthy_inputs());
        assert!(report.healthy(), "{report:#?}");
        assert_eq!(report.findings.len(), 6);
    }

    #[test]
    fn a_missing_toolchain_fails_with_a_fix() {
        // A missing resolution is a failure, independent of the host's PATH.
        let finding = check_toolchain(&DoctorInputs {
            roc_resolved: Some(Err("no roc binary".to_owned())),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail);
        assert!(!finding.fix.is_empty());
    }

    #[test]
    fn an_incomplete_platform_fails_with_the_build_command() {
        let finding = check_platform(&inputs());
        assert_eq!(finding.severity, Severity::Fail);
        assert!(finding.fix.contains("ID_ROC_PLATFORM"), "{}", finding.fix);
    }

    #[test]
    fn a_ceiling_refuses_wildcards_and_unknown_names() {
        let finding = check_ceiling(&DoctorInputs {
            caps_ceiling: Some(vec!["*".to_owned()]),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail);

        let finding = check_ceiling(&DoctorInputs {
            caps_ceiling: Some(vec!["time.nwo".to_owned()]),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail);

        let finding = check_ceiling(&DoctorInputs {
            caps_ceiling: Some(vec!["time.now".to_owned(), "chat.say".to_owned()]),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Ok);
    }

    #[test]
    fn a_zero_profile_bound_fails() {
        let finding = check_profile(&DoctorInputs {
            profile: Some(WorldExecutionProfile {
                wasm_fuel: Some(0),
                ..WorldExecutionProfile::default()
            }),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail);
        assert!(finding.detail.contains("wasm_fuel"), "{}", finding.detail);

        assert_eq!(check_profile(&inputs()).severity, Severity::Ok);
    }

    #[test]
    fn a_journal_must_start_with_created_and_parse() {
        let dir = std::env::temp_dir().join(format!("id-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");

        // A valid journal.
        let created = serde_json::json!({
            "entry": "created",
            "world_id": "w",
            "version": JOURNAL_VERSION,
        });
        std::fs::write(&path, format!("{created}\n")).unwrap();
        let (entries, version) = validate_journal_file(&path).unwrap();
        assert_eq!((entries, version), (1, JOURNAL_VERSION));

        // Not a `Created` first entry.
        let bad = serde_json::json!({"entry": "caps_granted", "granted": []});
        std::fs::write(&path, format!("{bad}\n")).unwrap();
        assert!(validate_journal_file(&path).is_err());

        // A version this binary does not know.
        let future = serde_json::json!({
            "entry": "created",
            "world_id": "w",
            "version": MAX_JOURNAL_VERSION + 1,
        });
        std::fs::write(&path, format!("{future}\n")).unwrap();
        assert!(validate_journal_file(&path).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_journal_version_and_blank_lines_follow_open_rules() {
        // #1598 originally pinned doctor to the restore reader. Triage #2999
        // supersedes that for blank lines: all journal readers skip them.
        let dir = std::env::temp_dir().join(format!("id-doctor-1598-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        let created = |version: u32| {
            serde_json::json!({
                "entry": "created",
                "world_id": "w",
                "version": version,
            })
        };

        // Version 0 is below JOURNAL_VERSION: refused, like restore.
        std::fs::write(&path, format!("{}\n", created(0))).unwrap();
        assert!(validate_journal_file(&path).is_err());

        // Leading blank line.
        std::fs::write(&path, format!("\n{}\n", created(JOURNAL_VERSION))).unwrap();
        let (entries, version) = validate_journal_file(&path).unwrap();
        assert_eq!((entries, version), (1, JOURNAL_VERSION));

        // A blank line between valid records is ignored.
        let revoked = serde_json::json!({
            "entry": "revoked",
            "participant_id": 1,
            "by": null,
        });
        std::fs::write(
            &path,
            format!("{}\n\n{}\n", created(JOURNAL_VERSION), revoked),
        )
        .unwrap();
        let (entries, version) = validate_journal_file(&path).unwrap();
        assert_eq!((entries, version), (2, JOURNAL_VERSION));

        // Trailing blank lines are ignored too.
        std::fs::write(&path, format!("{}\n\n", created(JOURNAL_VERSION))).unwrap();
        let (entries, version) = validate_journal_file(&path).unwrap();
        assert_eq!((entries, version), (1, JOURNAL_VERSION));

        // A leading blank line does not hide a missing header either.
        let bad = serde_json::json!({"entry": "caps_granted", "granted": []});
        std::fs::write(&path, format!("\n{bad}\n")).unwrap();
        assert!(validate_journal_file(&path).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_torn_final_entry_is_a_recoverable_warning_and_not_rewritten() {
        let dir = std::env::temp_dir().join(format!("id-doctor-torn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        let created = serde_json::json!({
            "entry": "created",
            "world_id": "w",
            "version": JOURNAL_VERSION,
        });
        let mut bytes = format!("{created}\n").into_bytes();
        bytes.extend_from_slice(b"{\"entry\":\"revoked\",\"participant");
        std::fs::write(&path, &bytes).unwrap();

        let finding = check_journal(&DoctorInputs {
            journal: Some(path.clone()),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Warn, "{finding:#?}");
        assert!(
            finding.detail.contains("unterminated final entry"),
            "{finding:#?}"
        );
        assert!(finding.fix.contains("drop"), "{finding:#?}");
        assert!(!finding.fix.contains("restore"), "{finding:#?}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "doctor is read-only");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_future_journal_version_gets_unsupported_binary_advice() {
        let dir = std::env::temp_dir().join(format!("id-doctor-future-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        let future = serde_json::json!({
            "entry": "created",
            "world_id": "w",
            "version": MAX_JOURNAL_VERSION + 1,
        });
        std::fs::write(&path, format!("{future}\n")).unwrap();

        let finding = check_journal(&DoctorInputs {
            journal: Some(path),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail, "{finding:#?}");
        assert!(
            finding.fix.contains("supports journal format"),
            "{finding:#?}"
        );
        assert!(!finding.fix.contains("restore"), "{finding:#?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mid_file_journal_corruption_remains_a_failure() {
        let dir = std::env::temp_dir().join(format!("id-doctor-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        let created = serde_json::json!({
            "entry": "created",
            "world_id": "w",
            "version": JOURNAL_VERSION,
        });
        let revoked = serde_json::json!({
            "entry": "revoked",
            "participant_id": 1,
            "by": null,
        });
        std::fs::write(&path, format!("{created}\nnot-json\n{revoked}\n")).unwrap();

        let finding = check_journal(&DoctorInputs {
            journal: Some(path),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail, "{finding:#?}");
        assert!(finding.fix.contains("restore"), "{finding:#?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_warn_only_report_does_not_fail_the_exit_result() {
        // Healthy inputs, except the resolved compiler differs from the
        // configured one — the one Warn path in check_toolchain.
        let mut inputs = healthy_inputs();
        let exe = std::env::current_exe().unwrap();
        inputs.roc_bin = Some(exe.to_string_lossy().into_owned());
        inputs.roc_resolved = Some(Ok("/some/other/roc".to_owned()));
        let report = run(&inputs);
        assert!(!report.healthy(), "{report:#?}");
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|finding| finding.severity == Severity::Warn)
                .count(),
            1,
            "{report:#?}"
        );
        assert_eq!(report.failure_count(), 0);
        assert!(!report.has_failures());
        // The CLI-level decision the Doctor handler uses: Warn exits 0.
        assert!(report.exit_result().is_ok(), "{report:#?}");
    }

    #[test]
    fn a_failing_report_counts_only_the_failed_checks() {
        // Toolchain unresolvable + no platform directory: exactly 2 Fail
        // findings out of 6 checks.
        let report = run(&DoctorInputs {
            roc_resolved: Some(Err("no roc binary".to_owned())),
            ..inputs()
        });
        assert_eq!(report.findings.len(), 6, "{report:#?}");
        assert_eq!(report.failure_count(), 2, "{report:#?}");
        let error = report.exit_result().unwrap_err().to_string();
        assert!(error.contains("found 2 problem(s)"), "{error}");
        assert!(!error.contains('6'), "{error}");
    }

    #[test]
    fn a_bad_ceiling_and_a_zero_bound_profile_fail_through_run() {
        // The values the CLI now supplies via --ceiling / --profile reach
        // the same run() the Doctor handler calls.
        let report = run(&DoctorInputs {
            caps_ceiling: Some(vec!["*".to_owned()]),
            profile: Some(WorldExecutionProfile {
                wasm_fuel: Some(0),
                ..WorldExecutionProfile::default()
            }),
            roc_resolved: Some(Err("no roc binary".to_owned())),
            ..inputs()
        });
        let ceiling = &report.findings[2];
        assert_eq!(ceiling.check, "ceiling");
        assert_eq!(ceiling.severity, Severity::Fail, "{ceiling:#?}");
        let profile = &report.findings[3];
        assert_eq!(profile.check, "profile");
        assert_eq!(profile.severity, Severity::Fail, "{profile:#?}");
        assert!(report.exit_result().is_err());
    }

    #[test]
    fn a_clock_before_the_epoch_fails() {
        let finding = check_clock(&DoctorInputs {
            now_ms: Some(0),
            ..inputs()
        });
        assert_eq!(finding.severity, Severity::Fail);

        assert_eq!(check_clock(&inputs()).severity, Severity::Ok);
    }
}
