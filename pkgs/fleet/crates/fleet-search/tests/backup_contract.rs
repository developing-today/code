//! Contract test for `fleet search backup` (#14129): the verb,
//! run end-to-end against stub `sqlite3` / `bws` / `rclone`
//! executables, must reproduce the shell original's
//! (`nixos/home/atlas-backup.sh`) observable contract —
//!
//!   * the same bytes land at all three destinations, sha256-equal
//!     to the manifest's `sha256`;
//!   * the manifest carries the shell original's fields;
//!   * the verdict + backup-log lines match its formats;
//!   * a second run over an unchanged DB dedupes to UNCHANGED;
//!   * rotation keeps the newest 7 laptop dates.
//!
//! The oracle is the shell script's documented behaviour (its
//! header comment + verdict/log formats), not this crate's own
//! internals: stubs implement the tool CLIs the script itself
//! drives, over a plain directory tree standing in for the remotes.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use fleet_search::backup::{BackupConfig, run_backup_w};

struct Fixture {
    base: PathBuf,
    cfg: BackupConfig,
    remote: PathBuf,
}

fn write_tool(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn fixture(tag: &str) -> Fixture {
    let base = std::env::temp_dir().join(format!(
        "iw288-backup-contract-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    let home = base.join("home");
    let data = home.join("atlas/turso-atlas/data");
    fs::create_dir_all(&data).unwrap();
    let db = data.join("atlas.db");
    fs::write(&db, b"canonical atlas db bytes v1").unwrap();
    // The snapshot the stub sqlite3 hands back for `.backup`.
    let snap_source = base.join("snap-source.db");
    fs::write(&snap_source, b"snapshot bytes: sessions and chunks").unwrap();
    let bin = base.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let remote = base.join("remote");
    fs::create_dir_all(&remote).unwrap();

    write_tool(
        &bin.join("sqlite3"),
        &format!(
            "#!/bin/sh\n\
             case \"$2\" in\n\
             .backup*) out=$(printf '%s' \"$2\" | sed \"s/.*'\\(.*\\)'.*/\\1/\"); cp \"{snap}\" \"$out\" ;;\n\
             *integrity_check*) echo ok ;;\n\
             *\"count(*) FROM sessions\"*) echo 3 ;;\n\
             *\"count(*) FROM messages\"*) echo 5 ;;\n\
             *\"count(*) FROM chunks\"*) echo 7 ;;\n\
             *) echo \"stub sqlite3: unhandled: $2\" >&2; exit 1 ;;\n\
             esac\n",
            snap = snap_source.display()
        ),
    );
    write_tool(
        &bin.join("bws"),
        "#!/bin/sh\n\
         echo '[{\"key\":\"cloudflare.r2-backups-access-key-id\",\"value\":\"test-key-id\"},\
         {\"key\":\"cloudflare.r2-backups-secret-access-key\",\"value\":\"test-secret\"},\
         {\"key\":\"cloudflare.r2-endpoint\",\"value\":\"r2.example.invalid\"}]'\n",
    );
    write_tool(
        &bin.join("rclone"),
        &format!(
            "#!/bin/sh\n\
             root=\"{remote}\"\n\
             map() {{ printf '%s/%s' \"$root\" \"${{1#*:}}\"; }}\n\
             cmd=\"$1\"; shift\n\
             case \"$cmd\" in\n\
             copyto) dest=$(map \"$2\"); mkdir -p \"$(dirname \"$dest\")\"; cp \"$1\" \"$dest\" ;;\n\
             cat) cat \"$(map \"$1\")\" ;;\n\
             lsf) if [ \"$1\" = \"--dirs-only\" ]; then p=$(map \"$2\"); [ -d \"$p\" ] && ls \"$p\" | while read -r d; do [ -d \"$p/$d\" ] && echo \"$d/\"; done; else p=$(map \"$1\"); [ -d \"$p\" ] && ls \"$p\"; fi ;;\n\
             purge) rm -rf \"$(map \"$1\")\" ;;\n\
             deletefile) rm -f \"$(map \"$1\")\" ;;\n\
             *) echo \"stub rclone: unhandled: $cmd\" >&2; exit 1 ;;\n\
             esac\n",
            remote = remote.display()
        ),
    );

    let cfg = BackupConfig {
        home: home.clone(),
        db_path: db,
        root: home.join("backups/atlas"),
        r2_bucket: "supabase-backups".to_string(),
        r2_prefix: "atlas".to_string(),
        drive_dir: "Cloud/cold-store/db-backups/local-atlas".to_string(),
        keep_laptop_r2: 7,
        keep_drive: 4,
        min_free_bytes: 0,
        sqlite3: Some(bin.join("sqlite3")),
        rclone: Some(bin.join("rclone")),
        bws: Some(bin.join("bws")),
    };
    Fixture { base, cfg, remote }
}

fn run(fx: &Fixture, date: &str, skip_dedupe: bool) -> (i32, String) {
    let mut out: Vec<u8> = Vec::new();
    let code = run_backup_w(&fx.cfg, date, skip_dedupe, &mut out);
    (code, String::from_utf8_lossy(&out).to_string())
}

fn sha256_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path).unwrap();
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn backup_contract_end_to_end() {
    let fx = fixture("e2e");
    let date = "2026-11-25";

    // --- first run: OK, identical bytes at all three destinations.
    let (code, verdict) = run(&fx, date, false);
    assert_eq!(code, 0, "verdict: {verdict}");
    assert!(
        verdict.starts_with(&format!("ATLAS-BACKUP {date} OK sha256=")),
        "verdict: {verdict}"
    );
    assert!(
        verdict.contains("laptop=ok r2=ok drive=ok"),
        "verdict: {verdict}"
    );
    assert!(
        verdict.contains("counts=sessions:3,messages:5,chunks:7"),
        "verdict: {verdict}"
    );

    let local_gz = fx.cfg.root.join(date).join(format!("atlas-{date}.db.gz"));
    let r2_gz = fx
        .remote
        .join(format!("supabase-backups/atlas/{date}/atlas-{date}.db.gz"));
    let drive_gz = fx.remote.join(format!(
        "Cloud/cold-store/db-backups/local-atlas/atlas-{date}.db.gz"
    ));
    let digest = sha256_of(&local_gz);
    assert_eq!(sha256_of(&r2_gz), digest);
    assert_eq!(sha256_of(&drive_gz), digest);
    assert!(
        verdict.contains(&format!("sha256={digest}")),
        "verdict: {verdict}"
    );

    // --- manifest shape (the shell original's fields).
    let manifest_text = fs::read_to_string(
        fx.cfg
            .root
            .join(date)
            .join(format!("atlas-{date}.manifest.json")),
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    assert_eq!(manifest["dataset"], "local-atlas-db");
    assert_eq!(manifest["date"], date);
    assert_eq!(manifest["file"], format!("atlas-{date}.db.gz"));
    assert_eq!(manifest["sha256"], digest);
    assert_eq!(manifest["counts"]["sessions"], 3);
    assert_eq!(manifest["counts"]["messages"], 5);
    assert_eq!(manifest["counts"]["chunks"], 7);
    assert_eq!(manifest["issue"], "developing-today/code#1515");

    // --- backup-log line (the dedupe record's OK shape).
    let log = fs::read_to_string(fx.cfg.root.join("backup-log.md")).unwrap();
    assert!(
        log.contains(&format!(
            "{date} | sha256={digest} | OK laptop/r2/drive | size="
        )),
        "log: {log}"
    );

    // --- second run, unchanged DB: UNCHANGED, no new log OK line.
    let (code, verdict) = run(&fx, "2026-11-26", false);
    assert_eq!(code, 0, "verdict: {verdict}");
    assert!(
        verdict.contains(&format!("UNCHANGED sha256={digest}")),
        "verdict: {verdict}"
    );
    assert!(
        !fx.cfg.root.join("2026-11-26").exists(),
        "deduped run must not file a new date dir"
    );

    // --- rotation: seed 8 older filed dates; a fresh (changed)
    // snapshot run keeps only the newest 7 laptop date dirs.
    let log_path = fx.cfg.root.join("backup-log.md");
    let mut log_text = fs::read_to_string(&log_path).unwrap();
    for d in 10..18 {
        let day = format!("2026-10-{d}");
        fs::create_dir_all(fx.cfg.root.join(&day)).unwrap();
        log_text.push_str(&format!(
            "{day} | sha256=old{d} | OK laptop/r2/drive | size=1 | counts sessions:0,messages:0,chunks:0\n"
        ));
    }
    // The seeded OK lines end with a different sha, so the next run
    // is not deduped.
    fs::write(&log_path, log_text).unwrap();
    let (code, verdict) = run(&fx, "2026-11-27", false);
    assert_eq!(code, 0, "verdict: {verdict}");
    let mut remaining: Vec<String> = fs::read_dir(&fx.cfg.root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.len() == 10 && n.starts_with("20"))
        .collect();
    remaining.sort();
    assert_eq!(remaining.len(), 7, "kept dirs: {remaining:?}");
    assert!(!remaining.contains(&"2026-10-10".to_string()));
    assert!(remaining.contains(&"2026-11-27".to_string()));

    let _ = fs::remove_dir_all(&fx.base);
}

#[test]
fn backup_contract_rejects_bad_dates() {
    let fx = fixture("dates");
    for bad in ["2026-02-29", "2026-13-01", "25-11-2026", ""] {
        let (code, _verdict) = run(&fx, bad, false);
        assert_eq!(code, 1, "date {bad:?} must fail");
    }
    let _ = fs::remove_dir_all(&fx.base);
}
