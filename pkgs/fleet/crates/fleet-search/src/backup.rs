//! `fleet search backup` — the daily backup of the canonical local
//! atlas.db (developing-today/code#14129, the #5670 §8 row that moves
//! `atlas-backup` into the fleet binary).
//!
//! A faithful port of the live shell original,
//! `nixos/home/atlas-backup.sh` (itself the 2026-10-30 laptop port of
//! the hatch-personal Python runner): each run snapshots the DB
//! consistently (`sqlite3 .backup`), gzips it deterministically
//! (mtime 0, no stored name), and files the SAME bytes to three
//! destinations —
//!
//!   1. laptop: `<root>/<DATE>/atlas-<DATE>.db.gz` (+ manifest)
//!   2. R2:     bucket `supabase-backups`, key `atlas/<DATE>/…`
//!   3. Drive:  `Cloud/cold-store/db-backups/local-atlas/` (rclone)
//!
//! Every offsite copy is sha256-verified against the local archive.
//! Dedupe is by content hash against the last OK verdict in
//! `<root>/backup-log.md`; rotation keeps the newest 7 dates on
//! laptop + R2 and the newest 4 archives on Drive, touching only this
//! routine's own artifacts. Verdict lines, log lines, the manifest
//! shape, and exit codes are the shell original's, unchanged, so the
//! systemd unit and anything scraping them see the same contract:
//!
//!   ATLAS-BACKUP <DATE> OK sha256=<hex> size=<n> laptop=ok r2=ok drive=ok …
//!   ATLAS-BACKUP <DATE> UNCHANGED sha256=<hex>
//!   ATLAS-BACKUP <DATE> FAILED legs=<names> sha256=<hex> size=<n> …
//!   ATLAS-BACKUP <DATE> FAILED early: <error>
//!
//! Exit 0 on OK/UNCHANGED (and on the single-instance SKIP), 1 if any
//! leg failed or an early failure fired, 2 on a usage error.
//!
//! Deliberate differences from the shell original, all mechanical:
//!   * the probe (integrity check, row counts, hashes, manifest) runs
//!     in-process instead of through an embedded Python step — the
//!     verb therefore needs `sqlite3`, `rclone`, and `bws` on hand,
//!     but not `python3`;
//!   * gzip bytes come from flate2 rather than CPython's gzip module,
//!     so the first verb run's archive hash differs from the shell
//!     original's even for an unchanged DB (one non-deduped upload,
//!     then dedupe is self-consistent again);
//!   * paths and tool locations can be overridden by environment
//!     (`ATLAS_BACKUP_DB`, `ATLAS_BACKUP_ROOT`, `ATLAS_BACKUP_R2_BUCKET`,
//!     `ATLAS_BACKUP_R2_PREFIX`, `ATLAS_BACKUP_DRIVE_DIR`,
//!     `ATLAS_BACKUP_SQLITE3`, `ATLAS_BACKUP_RCLONE`,
//!     `ATLAS_BACKUP_BWS`) — the defaults are the shell original's
//!     values, and the overrides exist so the contract test can run
//!     the verb end-to-end against stub tools.
//!
//! Credentials come from Bitwarden Secrets Manager at runtime,
//! referenced by key name only and never logged (the shell original's
//! rule): `cloudflare.r2-backups-access-key-id`,
//! `cloudflare.r2-backups-secret-access-key`, `cloudflare.r2-endpoint`.
//! The Drive leg uses the laptop's existing rclone `drive` remote.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

pub const BACKUP_USAGE: &str = "\
usage: fleet search backup [--date YYYY-MM-DD] [--skip-dedupe] [DATE]

Daily backup of the canonical local atlas.db: snapshot it
consistently, gzip it deterministically, and file the same bytes to
the laptop backups root, R2, and Google Drive, sha256-verified at
every destination (the atlas-backup routine, #14129).

  --date YYYY-MM-DD   backup date (default: today, local time); a
                      bare positional DATE means the same
  --skip-dedupe       file the snapshot even when its sha256 equals
                      the last filed snapshot's

Verdicts (stdout): ATLAS-BACKUP <DATE> OK | UNCHANGED | FAILED …
Exit codes: 0 ok/unchanged/skipped (another run holds the lock),
1 a leg failed or an early failure fired, 2 usage error.";

const DEFAULT_R2_BUCKET: &str = "supabase-backups";
const DEFAULT_R2_PREFIX: &str = "atlas";
const DEFAULT_DRIVE_DIR: &str = "Cloud/cold-store/db-backups/local-atlas";
const KEEP_LAPTOP_R2: usize = 7;
const KEEP_DRIVE: usize = 4;
const MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Everything a backup run needs, resolved from the environment by
/// [`BackupConfig::from_env`] (the CLI path) or built directly (the
/// contract test's path).
#[derive(Debug, Clone)]
pub struct BackupConfig {
    pub home: PathBuf,
    pub db_path: PathBuf,
    pub root: PathBuf,
    pub r2_bucket: String,
    pub r2_prefix: String,
    pub drive_dir: String,
    pub keep_laptop_r2: usize,
    pub keep_drive: usize,
    pub min_free_bytes: u64,
    pub sqlite3: Option<PathBuf>,
    pub rclone: Option<PathBuf>,
    pub bws: Option<PathBuf>,
}

impl BackupConfig {
    pub fn from_env() -> BackupConfig {
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let pick = |name: &str| env::var(name).ok().filter(|v| !v.is_empty());
        BackupConfig {
            db_path: pick("ATLAS_BACKUP_DB")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("atlas/turso-atlas/data/atlas.db")),
            root: pick("ATLAS_BACKUP_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("backups/atlas")),
            r2_bucket: pick("ATLAS_BACKUP_R2_BUCKET")
                .unwrap_or_else(|| DEFAULT_R2_BUCKET.to_string()),
            r2_prefix: pick("ATLAS_BACKUP_R2_PREFIX")
                .unwrap_or_else(|| DEFAULT_R2_PREFIX.to_string()),
            drive_dir: pick("ATLAS_BACKUP_DRIVE_DIR")
                .unwrap_or_else(|| DEFAULT_DRIVE_DIR.to_string()),
            keep_laptop_r2: KEEP_LAPTOP_R2,
            keep_drive: KEEP_DRIVE,
            min_free_bytes: MIN_FREE_BYTES,
            sqlite3: pick("ATLAS_BACKUP_SQLITE3").map(PathBuf::from),
            rclone: pick("ATLAS_BACKUP_RCLONE").map(PathBuf::from),
            bws: pick("ATLAS_BACKUP_BWS").map(PathBuf::from),
            home,
        }
    }
}

/// The CLI entry: `fleet search backup ...` (args include the
/// `backup` token at position 0).
pub fn run_cmd(raw: &[String]) -> i32 {
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{BACKUP_USAGE}");
        return 0;
    }
    let mut date: Option<String> = None;
    let mut skip_dedupe = false;
    let mut it = raw[1..].iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--date" => match it.next() {
                Some(v) => date = Some(v.clone()),
                // The shell original takes the (empty) next word and
                // fails date validation below; mirror that.
                None => date = Some(String::new()),
            },
            "--skip-dedupe" => skip_dedupe = true,
            other if other.starts_with("--") => {
                eprintln!("atlas-backup: unknown argument: {other}");
                return 2;
            }
            other => date = Some(other.to_string()),
        }
    }
    let cfg = BackupConfig::from_env();
    let date = date.unwrap_or_else(|| today_local(&cfg));
    run_backup(&cfg, &date, skip_dedupe)
}

/// Run one backup. Prints exactly one verdict line to stdout (plus
/// warnings to stderr) and returns the exit code.
pub fn run_backup(cfg: &BackupConfig, date: &str, skip_dedupe: bool) -> i32 {
    run_backup_w(cfg, date, skip_dedupe, &mut std::io::stdout())
}

/// [`run_backup`] with the verdict stream named — the contract
/// test's seam.
pub fn run_backup_w(cfg: &BackupConfig, date: &str, skip_dedupe: bool, out: &mut dyn Write) -> i32 {
    // --- DATE validation (the shell original's rule: a real
    // calendar date as YYYY-MM-DD; failure here precedes any log).
    if !valid_date(date) {
        eprintln!(
            "atlas-backup: invalid DATE '{date}' - expected a real calendar date as YYYY-MM-DD"
        );
        return 1;
    }
    let log_path = cfg.root.join("backup-log.md");
    if fs::create_dir_all(&cfg.root).is_err() {
        return early_fail(out, &log_path, date, "cannot create backups root", false);
    }

    // --- single instance (flock on the root's lock file).
    let lock_file = match fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(cfg.root.join(".atlas-backup.lock"))
    {
        Ok(f) => f,
        Err(_) => return early_fail(out, &log_path, date, "cannot open instance lock", true),
    };
    let locked = unsafe {
        libc::flock(
            <fs::File as std::os::unix::io::AsRawFd>::as_raw_fd(&lock_file),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    };
    if locked != 0 {
        let _ = writeln!(
            out,
            "atlas-backup: SKIP another atlas-backup run holds the lock"
        );
        return 0;
    }

    // --- tools.
    let sqlite3 = match resolve_tool(cfg.sqlite3.as_deref(), "sqlite3", &[]) {
        Some(p) => p,
        None => return early_fail(out, &log_path, date, "sqlite3 not in PATH", true),
    };
    let bws = match resolve_bws(cfg) {
        Some(p) => p,
        None => return early_fail(out, &log_path, date, "bws not in PATH", true),
    };
    let rclone = match resolve_rclone(cfg) {
        Some(p) => p,
        None => {
            return early_fail(
                out,
                &log_path,
                date,
                "rclone not found in PATH or nix store",
                true,
            );
        }
    };
    if !cfg.db_path.is_file() {
        return early_fail(
            out,
            &log_path,
            date,
            &format!("canonical atlas.db not found at {}", cfg.db_path.display()),
            true,
        );
    }

    // --- credentials from Bitwarden (names only in code/logs).
    let r2 = match r2_credentials(&bws, cfg) {
        Ok(c) => c,
        Err(m) => return early_fail(out, &log_path, date, &m, true),
    };

    // --- snapshot + probe.
    let db_dir = cfg.db_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let free = free_bytes(&db_dir);
    if free < cfg.min_free_bytes {
        return early_fail(
            out,
            &log_path,
            date,
            &format!("only {free} bytes free in atlas data dir"),
            true,
        );
    }
    let staging_root = db_dir.join(".backup-staging");
    let stage = staging_root.join(date);
    let _stage_guard = StageGuard(stage.clone());
    let _ = fs::remove_dir_all(&stage);
    if fs::create_dir_all(&stage).is_err() {
        return early_fail(out, &log_path, date, "cannot create staging dir", true);
    }
    // Bound the staging tree: a killed run leaves its date dir
    // behind; this run holds the instance lock, so reclaim every
    // OTHER date's staging dir here (the shell original's rule).
    if let Ok(entries) = fs::read_dir(&staging_root) {
        for e in entries.flatten() {
            if e.path() != stage {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    let snap = stage.join(format!("atlas-{date}.db"));
    if !sqlite_backup(&sqlite3, &cfg.db_path, &snap) {
        return early_fail(out, &log_path, date, "sqlite3 .backup failed", true);
    }
    let probe = match probe_snapshot(&sqlite3, &snap) {
        Ok(p) => p,
        Err(m) => return early_fail(out, &log_path, date, &m, true),
    };
    let gz_path = stage.join(format!("atlas-{date}.db.gz"));
    let manifest_path = stage.join(format!("atlas-{date}.manifest.json"));
    let (digest, md5, size) = match gzip_and_hash(&snap, &gz_path) {
        Ok(v) => v,
        Err(m) => return early_fail(out, &log_path, date, &m, true),
    };
    let _ = fs::remove_file(&snap);
    let manifest = json!({
        "dataset": "local-atlas-db",
        "date": date,
        "file": format!("atlas-{date}.db.gz"),
        "sha256": digest,
        "md5": md5,
        "size": size,
        "source": format!("laptop:{}", cfg.db_path.display()),
        "source_bytes": cfg.db_path.metadata().map(|m| m.size()).unwrap_or(0),
        "counts": {
            "sessions": probe.counts[0],
            "messages": probe.counts[1],
            "chunks": probe.counts[2],
        },
        "created_at": now_utc_string(),
        "issue": "developing-today/code#1515",
    });
    if fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).unwrap_or_default()
        ),
    )
    .is_err()
    {
        return early_fail(out, &log_path, date, "manifest write failed", true);
    }
    let counts_csv = format!(
        "sessions:{},messages:{},chunks:{}",
        count_str(probe.counts[0]),
        count_str(probe.counts[1]),
        count_str(probe.counts[2])
    );

    // --- dedupe against the last filed snapshot.
    let (last_sha, filed_dates) = read_filed(&log_path);
    if digest == last_sha && !skip_dedupe {
        let _ = append_log(
            &log_path,
            date,
            &format!("sha256={digest} | unchanged — legs skipped | size={size}"),
        );
        let _ = writeln!(out, "ATLAS-BACKUP {date} UNCHANGED sha256={digest}");
        return 0;
    }

    // --- legs: each records ok or a one-line FAIL note and the run
    // continues with the remaining legs.
    let outdir = cfg.root.join(date);
    let laptop = leg_laptop(&gz_path, &manifest_path, &outdir, date, &digest)
        .err()
        .map(|e| format!("FAIL: {}", short_err(&e)));
    let r2_leg = leg_rclone(
        &rclone,
        &r2.env,
        &gz_path,
        &manifest_path,
        &format!(
            "r2bak:{}/{}/{}/atlas-{date}.db.gz",
            cfg.r2_bucket, cfg.r2_prefix, date
        ),
        &format!(
            "r2bak:{}/{}/{}/atlas-{date}.manifest.json",
            cfg.r2_bucket, cfg.r2_prefix, date
        ),
        &digest,
        "R2",
    )
    .err()
    .map(|e| format!("FAIL: {}", short_err(&e)));
    let drive = leg_rclone(
        &rclone,
        &[],
        &gz_path,
        &manifest_path,
        &format!("drive:{}/atlas-{date}.db.gz", cfg.drive_dir),
        &format!("drive:{}/atlas-{date}.manifest.json", cfg.drive_dir),
        &digest,
        "Drive",
    )
    .err()
    .map(|e| format!("FAIL: {}", short_err(&e)));

    let leg_str = |v: &Option<String>| v.clone().unwrap_or_else(|| "ok".to_string());
    let leg_laptop_s = leg_str(&laptop);
    let leg_r2_s = leg_str(&r2_leg);
    let leg_drive_s = leg_str(&drive);

    // --- rotation (only this routine's own artifacts; warnings
    // never fail the run; a destination rotates only when its leg
    // succeeded).
    let mut all_dates: Vec<String> = filed_dates;
    if !all_dates.iter().any(|d| d == date) {
        all_dates.push(date.to_string());
    }
    all_dates.sort();
    let keep_set: BTreeSet<String> = all_dates
        .iter()
        .rev()
        .take(cfg.keep_laptop_r2)
        .cloned()
        .collect();
    if laptop.is_none() {
        rotate_laptop(cfg, &keep_set);
    }
    if r2_leg.is_none() {
        rotate_r2(cfg, &rclone, &r2.env, &keep_set);
    }
    if drive.is_none() {
        rotate_drive(cfg, &rclone, date);
    }

    // --- verdict.
    let mut failed: Vec<&str> = Vec::new();
    if laptop.is_some() {
        failed.push("laptop");
    }
    if r2_leg.is_some() {
        failed.push("r2");
    }
    if drive.is_some() {
        failed.push("drive");
    }
    if !failed.is_empty() {
        let _ = append_log(
            &log_path,
            date,
            &format!(
                "sha256={digest} | PARTIAL laptop={leg_laptop_s} r2={leg_r2_s} drive={leg_drive_s} | size={size}"
            ),
        );
        let _ = writeln!(
            out,
            "ATLAS-BACKUP {date} FAILED legs={} sha256={digest} size={size} laptop={leg_laptop_s} r2={leg_r2_s} drive={leg_drive_s} counts={counts_csv}",
            failed.join(",")
        );
        return 1;
    }
    let _ = append_log(
        &log_path,
        date,
        &format!("sha256={digest} | OK laptop/r2/drive | size={size} | counts {counts_csv}"),
    );
    let _ = writeln!(
        out,
        "ATLAS-BACKUP {date} OK sha256={digest} size={size} laptop=ok r2=ok drive=ok counts={counts_csv}"
    );
    0
}

/// Removes the staging dir on drop — the shell original's EXIT trap.
struct StageGuard(PathBuf);
impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn append_log(log_path: &Path, date: &str, line: &str) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(f, "{date} | {line}")
}

/// The early-failure verdict: print it, best-effort log it (once
/// the root exists), exit 1 — the shell original's `early_fail`.
fn early_fail(out: &mut dyn Write, log_path: &Path, date: &str, msg: &str, log: bool) -> i32 {
    let _ = writeln!(out, "ATLAS-BACKUP {date} FAILED early: {msg}");
    if log {
        let _ = append_log(log_path, date, &format!("FAILED early: {msg}"));
    }
    1
}

fn short_err(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or("")
        .replace('|', "/")
        .chars()
        .take(200)
        .collect()
}

fn count_str(v: Option<i64>) -> String {
    v.map(|n| n.to_string())
        .unwrap_or_else(|| "None".to_string())
}

/// PATH lookup, honouring an explicit override first.
fn resolve_tool(override_: Option<&Path>, name: &str, extra_dirs: &[&Path]) -> Option<PathBuf> {
    if let Some(p) = override_ {
        return p.is_file().then(|| p.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(extra_dirs.iter().map(|d| d.to_path_buf()));
    for d in dirs {
        let cand = d.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn resolve_bws(cfg: &BackupConfig) -> Option<PathBuf> {
    let tools_bin = cfg.home.join("tools/bin");
    resolve_tool(cfg.bws.as_deref(), "bws", &[tools_bin.as_path()])
}

fn resolve_rclone(cfg: &BackupConfig) -> Option<PathBuf> {
    if let Some(p) = resolve_tool(cfg.rclone.as_deref(), "rclone", &[]) {
        return Some(p);
    }
    // The shell original's fallback: the newest nix-store build.
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir("/nix/store") {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.contains("-rclone-")
                && name
                    .rsplit("-rclone-")
                    .next()
                    .map(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
            {
                let cand = e.path().join("bin/rclone");
                if cand.is_file() {
                    cands.push(cand);
                }
            }
        }
    }
    cands.sort();
    let newest = cands.pop()?;
    // The shell original symlinks it into <root>/.bin and PATHs it;
    // the port invokes the resolved path directly (equivalent).
    let link_dir = cfg.root.join(".bin");
    let _ = fs::create_dir_all(&link_dir);
    let link = link_dir.join("rclone");
    let _ = fs::remove_file(&link);
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&newest, &link);
    Some(newest)
}

struct R2Creds {
    env: Vec<(String, String)>,
}

/// Fetch the three R2 values from Bitwarden by key name (values are
/// never logged) and shape them as the rclone env-config the shell
/// original exports for its `r2bak` remote.
fn r2_credentials(bws: &Path, cfg: &BackupConfig) -> Result<R2Creds, String> {
    let mut token = env::var("BWS_ACCESS_TOKEN").unwrap_or_default();
    if token.is_empty() {
        let token_file = cfg.home.join(".config/bws/access-token");
        if let Ok(contents) = fs::read_to_string(&token_file) {
            token = contents.trim().to_string();
        }
    }
    let out = Command::new(bws)
        .arg("secret")
        .arg("list")
        .env("BWS_ACCESS_TOKEN", &token)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "bws secret list failed".to_string())?;
    if !out.status.success() {
        return Err("bws secret list failed".to_string());
    }
    let list: Json =
        serde_json::from_slice(&out.stdout).map_err(|_| "bws secret list failed".to_string())?;
    let get = |key: &str| -> String {
        list.as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|s| s.get("key").and_then(|k| k.as_str()) == Some(key))
            })
            .and_then(|s| s.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let key_id = get("cloudflare.r2-backups-access-key-id");
    let secret = get("cloudflare.r2-backups-secret-access-key");
    let mut endpoint = get("cloudflare.r2-endpoint");
    if key_id.is_empty() || secret.is_empty() || endpoint.is_empty() {
        return Err("R2 credentials incomplete in Bitwarden".to_string());
    }
    if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
        endpoint = format!("https://{endpoint}");
    }
    let endpoint = endpoint.trim_end_matches('/').to_string();
    Ok(R2Creds {
        env: vec![
            ("RCLONE_CONFIG_R2BAK_TYPE".into(), "s3".into()),
            ("RCLONE_CONFIG_R2BAK_PROVIDER".into(), "Cloudflare".into()),
            ("RCLONE_CONFIG_R2BAK_ACCESS_KEY_ID".into(), key_id),
            ("RCLONE_CONFIG_R2BAK_SECRET_ACCESS_KEY".into(), secret),
            ("RCLONE_CONFIG_R2BAK_ENDPOINT".into(), endpoint),
            ("RCLONE_CONFIG_R2BAK_REGION".into(), "auto".into()),
            // The r2-backups token cannot HeadBucket (403), so
            // rclone must skip its bucket check (the standing fleet
            // rule for this remote).
            ("RCLONE_CONFIG_R2BAK_NO_CHECK_BUCKET".into(), "true".into()),
        ],
    })
}

fn free_bytes(dir: &Path) -> u64 {
    let cpath = match std::ffi::CString::new(dir.as_os_str().as_bytes()) {
        Ok(p) => p,
        Err(_) => return 0,
    };
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(cpath.as_ptr(), &mut st) != 0 {
            return 0;
        }
        st.f_bavail as u64 * st.f_frsize as u64
    }
}

fn sqlite_backup(sqlite3: &Path, db: &Path, snap: &Path) -> bool {
    let sql = format!(
        ".backup '{}'",
        snap.display().to_string().replace('\'', "''")
    );
    Command::new(sqlite3)
        .arg(db)
        .arg(sql)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sqlite_scalar(sqlite3: &Path, db: &Path, sql: &str) -> Option<String> {
    let out = Command::new(sqlite3)
        .arg(db)
        .arg(sql)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

struct Probe {
    counts: [Option<i64>; 3],
}

fn probe_snapshot(sqlite3: &Path, snap: &Path) -> Result<Probe, String> {
    match sqlite_scalar(sqlite3, snap, "PRAGMA integrity_check;") {
        Some(v) if v == "ok" => {}
        Some(v) => return Err(format!("snapshot probe failed: integrity_check: {v}")),
        None => return Err("snapshot probe failed".to_string()),
    }
    let mut counts = [None; 3];
    for (i, t) in ["sessions", "messages", "chunks"].iter().enumerate() {
        counts[i] = sqlite_scalar(sqlite3, snap, &format!("SELECT count(*) FROM {t};"))
            .and_then(|v| v.parse::<i64>().ok());
    }
    Ok(Probe { counts })
}

/// Deterministic gzip (mtime 0, no stored name, level 6 — the shell
/// original's parameters) + sha256/md5/size of the archive.
fn gzip_and_hash(snap: &Path, gz_path: &Path) -> Result<(String, String, u64), String> {
    let mut fin = fs::File::open(snap).map_err(|_| "snapshot probe failed".to_string())?;
    let fout = fs::File::create(gz_path).map_err(|_| "snapshot probe failed".to_string())?;
    let mut enc = flate2::write::GzEncoder::new(fout, flate2::Compression::new(6));
    // GzEncoder's default header already carries mtime 0 and no
    // filename (flate2 builds the header from its builder defaults
    // only via GzBuilder; the plain encoder writes mtime 0).
    let mut buf = [0u8; 1 << 20];
    loop {
        let n = fin
            .read(&mut buf)
            .map_err(|_| "snapshot probe failed".to_string())?;
        if n == 0 {
            break;
        }
        enc.write_all(&buf[..n])
            .map_err(|_| "snapshot probe failed".to_string())?;
    }
    enc.finish()
        .map_err(|_| "snapshot probe failed".to_string())?;
    let bytes = fs::read(gz_path).map_err(|_| "snapshot probe failed".to_string())?;
    Ok((sha256_hex(&bytes), md5_hex(&bytes), bytes.len() as u64))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|b| sha256_hex(&b))
}

/// MD5 (RFC 1321), implemented in-crate: the manifest carries an md5
/// field for the old runner's consumers and no md5 crate exists in
/// the workspace lock. Verified against the RFC test vectors in this
/// module's tests. (Integrity verification uses sha256; md5 is a
/// manifest nicety, never a check.)
fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 * abs(sin(i+1))), computed at runtime so the
    // table cannot be mistyped.
    let k: Vec<u32> = (0..64)
        .map(|i| (((i as f64 + 1.0).sin().abs() * 4294967296.0).floor() as u64) as u32)
        .collect();
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = tmp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = String::with_capacity(32);
    for v in [a0, b0, c0, d0] {
        for b in v.to_le_bytes() {
            out.push_str(&format!("{b:02x}"));
        }
    }
    out
}

/// The dedupe record: the last filed snapshot's sha (from the last
/// OK log line) and every filed date, parsed with the shell
/// original's rule.
fn read_filed(log_path: &Path) -> (String, Vec<String>) {
    let mut sha = String::new();
    let mut dates = Vec::new();
    if let Ok(contents) = fs::read_to_string(log_path) {
        for line in contents.lines() {
            let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
            if parts.len() >= 3
                && parts[1].contains("sha256")
                && parts[2].to_uppercase().starts_with("OK")
            {
                sha = if parts[1].contains('=') {
                    parts[1].split('=').nth(1).unwrap_or("").trim().to_string()
                } else {
                    parts[1].split_whitespace().last().unwrap_or("").to_string()
                };
                dates.push(parts[0].to_string());
            }
        }
    }
    (sha, dates)
}

fn leg_laptop(
    gz: &Path,
    manifest: &Path,
    outdir: &Path,
    date: &str,
    digest: &str,
) -> Result<(), String> {
    fs::create_dir_all(outdir).map_err(|_| format!("mkdir {} failed", outdir.display()))?;
    let tmp = outdir.join(".atlas.tmp");
    fs::copy(gz, &tmp).map_err(|_| "copy to laptop staging failed".to_string())?;
    let got = sha256_file(&tmp).unwrap_or_default();
    if got != digest {
        return Err(format!("laptop sha mismatch: {got} != {digest}"));
    }
    fs::rename(&tmp, outdir.join(format!("atlas-{date}.db.gz")))
        .map_err(|_| "laptop rename failed".to_string())?;
    fs::copy(manifest, outdir.join(format!("atlas-{date}.manifest.json")))
        .map_err(|_| "laptop manifest copy failed".to_string())?;
    Ok(())
}

fn rclone_run(rclone: &Path, envs: &[(String, String)], args: &[&str]) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(rclone);
    cmd.args(args).stdin(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// One offsite leg (R2 or Drive): upload, read back and sha-verify,
/// then upload the manifest — the shell original's leg shape.
fn leg_rclone(
    rclone: &Path,
    envs: &[(String, String)],
    gz: &Path,
    manifest: &Path,
    dest_gz: &str,
    dest_manifest: &str,
    digest: &str,
    label: &str,
) -> Result<(), String> {
    rclone_run(
        rclone,
        envs,
        &["copyto", &gz.display().to_string(), dest_gz],
    )
    .map_err(|_| format!("{label} upload failed"))?;
    // A failed read-back reads as empty in the shell original and
    // surfaces as a sha mismatch with an empty `got`.
    let got = match rclone_run(rclone, envs, &["cat", dest_gz]) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(_) => String::new(),
    };
    if got != digest {
        return Err(format!("{label} sha mismatch: got {got}"));
    }
    rclone_run(
        rclone,
        envs,
        &["copyto", &manifest.display().to_string(), dest_manifest],
    )
    .map_err(|_| format!("{label} manifest upload failed"))?;
    Ok(())
}

fn looks_like_date_dir(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 10
        && name.starts_with("20")
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn rotate_laptop(cfg: &BackupConfig, keep_set: &BTreeSet<String>) {
    if let Ok(entries) = fs::read_dir(&cfg.root) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir()
                && looks_like_date_dir(&name)
                && !keep_set.contains(&name)
                && fs::remove_dir_all(e.path()).is_err()
            {
                eprintln!("atlas-backup: WARN laptop rotation failed for {name}");
            }
        }
    }
}

fn rotate_r2(
    cfg: &BackupConfig,
    rclone: &Path,
    envs: &[(String, String)],
    keep_set: &BTreeSet<String>,
) {
    let base = format!("r2bak:{}/{}", cfg.r2_bucket, cfg.r2_prefix);
    match rclone_run(rclone, envs, &["lsf", "--dirs-only", &base]) {
        Ok(out) => {
            for line in String::from_utf8_lossy(&out).lines() {
                let day = line.trim_end_matches('/');
                if looks_like_date_dir(day) && !keep_set.contains(day) {
                    let target = format!("{base}/{day}");
                    if rclone_run(rclone, envs, &["purge", &target]).is_err() {
                        eprintln!("atlas-backup: WARN R2 rotation failed for {day}");
                    }
                }
            }
        }
        Err(_) => eprintln!("atlas-backup: WARN R2 rotation listing failed"),
    }
}

fn rotate_drive(cfg: &BackupConfig, rclone: &Path, _date: &str) {
    let base = format!("drive:{}", cfg.drive_dir);
    match rclone_run(rclone, &[], &["lsf", &base]) {
        Ok(out) => {
            let names: Vec<String> = String::from_utf8_lossy(&out)
                .lines()
                .map(|l| l.to_string())
                .collect();
            let mut archives: Vec<String> = names
                .iter()
                .filter(|n| {
                    n.len() == "atlas-".len() + 10 + ".db.gz".len()
                        && n.starts_with("atlas-")
                        && n.ends_with(".db.gz")
                        && looks_like_date_dir(&n["atlas-".len()..n.len() - ".db.gz".len()])
                })
                .cloned()
                .collect();
            archives.sort();
            archives.reverse();
            let keep: BTreeSet<String> = archives.iter().take(cfg.keep_drive).cloned().collect();
            for name in &names {
                if name.starts_with("atlas-") && name.ends_with(".db.gz") && !keep.contains(name) {
                    let target = format!("{base}/{name}");
                    if rclone_run(rclone, &[], &["deletefile", &target]).is_err() {
                        eprintln!("atlas-backup: WARN Drive rotation failed for {name}");
                    }
                    let manifest_name = format!(
                        "{}.manifest.json",
                        name.strip_suffix(".db.gz").unwrap_or(name)
                    );
                    let mtarget = format!("{base}/{manifest_name}");
                    let _ = rclone_run(rclone, &[], &["deletefile", &mtarget]);
                }
            }
        }
        Err(_) => eprintln!("atlas-backup: WARN Drive rotation listing failed"),
    }
}

// --- calendar helpers (no chrono in this crate's dependency set) ---

/// days-from-civil / civil-from-days (Howard Hinnant's algorithms).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let y: i64 = s[0..4].parse().unwrap_or(-1);
    let m: u32 = s[5..7].parse().unwrap_or(0);
    let d: u32 = s[8..10].parse().unwrap_or(0);
    if y < 0 || !(1..=12).contains(&m) || d < 1 {
        return false;
    }
    civil_from_days(days_from_civil(y, m, d)) == (y, m, d)
}

fn today_local(_cfg: &BackupConfig) -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        )
    }
}

fn now_utc_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let tod = secs.rem_euclid(86400);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_rfc1321_vectors() {
        // The oracle is RFC 1321's own test suite.
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            md5_hex(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        assert_eq!(
            md5_hex(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn date_validation_matches_calendar() {
        assert!(valid_date("2026-11-25"));
        assert!(valid_date("2024-02-29")); // leap day
        assert!(!valid_date("2026-02-29"));
        assert!(!valid_date("2026-13-01"));
        assert!(!valid_date("2026-00-10"));
        assert!(!valid_date("2026-11-31"));
        assert!(!valid_date("2026-1-5"));
        assert!(!valid_date("25-11-2026"));
        assert!(!valid_date("2026/11/25"));
        assert!(!valid_date(""));
    }

    #[test]
    fn filed_parsing_follows_the_shell_rule() {
        // Only true OK lines count as filed; a PARTIAL line's leg
        // summary must never read as filed.
        let dir = env::temp_dir().join(format!("iw288-filed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("backup-log.md");
        fs::write(
            &log,
            "2026-11-20 | sha256=aaaa | OK laptop/r2/drive | size=10 | counts sessions:1,messages:2,chunks:3\n\
             2026-11-21 | sha256=bbbb | PARTIAL laptop=ok r2=FAIL: x drive=ok | size=11\n\
             2026-11-22 | sha256=cccc | unchanged — legs skipped | size=10\n\
             2026-11-23 | sha256=dddd | OK laptop/r2/drive | size=12 | counts sessions:1,messages:2,chunks:3\n",
        )
        .unwrap();
        let (sha, dates) = read_filed(&log);
        assert_eq!(sha, "dddd");
        assert_eq!(
            dates,
            vec!["2026-11-20".to_string(), "2026-11-23".to_string()]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gzip_is_deterministic_with_zero_mtime() {
        let dir = env::temp_dir().join(format!("iw288-gz-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let snap = dir.join("snap.db");
        fs::write(&snap, b"atlas fixture bytes").unwrap();
        let g1 = dir.join("a.gz");
        let g2 = dir.join("b.gz");
        let (s1, _, _) = gzip_and_hash(&snap, &g1).unwrap();
        let (s2, _, _) = gzip_and_hash(&snap, &g2).unwrap();
        assert_eq!(s1, s2);
        let bytes = fs::read(&g1).unwrap();
        // gzip header: magic, method 8, no stored name (FLG=0),
        // MTIME zero — the determinism contract.
        assert_eq!(&bytes[0..3], &[0x1f, 0x8b, 8]);
        assert_eq!(bytes[3], 0);
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
        let _ = fs::remove_dir_all(&dir);
    }
}
