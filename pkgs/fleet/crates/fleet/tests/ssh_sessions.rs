#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fleet-ssh-sessions-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fake_seat() -> (TestDir, PathBuf, String) {
    let dir = TestDir::new();
    let fake_bin = dir.0.join("bin");
    fs::create_dir(&fake_bin).expect("create fake bin directory");
    let fake_ssh = fake_bin.join("ssh");
    fs::write(
        &fake_ssh,
        "#!/bin/sh\nfor arg do last=$arg; done\nexec /bin/sh -c \"$last\"\n",
    )
    .expect("write fake ssh");
    let fake_tmux = fake_bin.join("tmux");
    fs::write(
        &fake_tmux,
        r##"#!/bin/sh
case "$1" in
  list-sessions)
    now=$(date +%s)
    case "$3" in
      *session_activity*)
        printf 'fleet-a\t%s\t0\nwork\t%s\t0\nfleet-attached\t%s\t1\nfleet-fresh\t%s\t0\n' "$((now - 3600))" "$((now - 3600))" "$((now - 3600))" "$now"
        ;;
      *) printf 'fleet-a\nwork\nfleet-attached\nfleet-b\nfleet-fresh\n' ;;
    esac
    ;;
  display-message)
    target=''
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "-t" ]; then target=$2; shift 2; else shift; fi
    done
    if [ "$target" = "=fleet-a" ] && [ "${FLEET_TEST_RECHECK_ATTACHED:-0}" = "1" ]; then
      printf '1\n'
    else
      printf '0\n'
    fi
    ;;
  kill-session)
    target=''
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "-t" ]; then target=$2; shift 2; else shift; fi
    done
    printf '%s\n' "$target" >> "$FLEET_TEST_KILLS"
    ;;
  *) echo "unexpected tmux command: $*" >&2; exit 98 ;;
esac
"##,
    )
    .expect("write fake tmux");
    for executable in [&fake_ssh, &fake_tmux] {
        let mut permissions = fs::metadata(executable)
            .expect("fake executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(executable, permissions).expect("make fake executable");
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", fake_bin.display(), old_path.to_string_lossy());
    let kill_log = dir.0.join("kills");
    (dir, kill_log, path)
}

fn run_fleet(
    dir: &TestDir,
    kill_log: &PathBuf,
    path: &str,
    args: &[&str],
    recheck_attached: bool,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(args)
        .env("PATH", path)
        .env("HOME", &dir.0)
        .env("HTTPS_PROXY", "http://127.0.0.1:3130")
        .env("FLEET_TEST_KILLS", kill_log)
        .env(
            "FLEET_TEST_RECHECK_ATTACHED",
            if recheck_attached { "1" } else { "0" },
        )
        .output()
        .expect("run fleet command")
}

#[test]
fn sessions_lists_only_fleet_owned_tmux_sessions() {
    let (dir, kill_log, path) = fake_seat();
    let output = run_fleet(&dir, &kill_log, &path, &["ssh", "sessions"], false);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "fleet-a\nfleet-attached\nfleet-b\nfleet-fresh\n"
    );
}

#[test]
fn sweep_kills_only_idle_detached_fleet_sessions() {
    let (dir, kill_log, path) = fake_seat();
    let output = run_fleet(
        &dir,
        &kill_log,
        &path,
        &["ssh", "sweep", "--idle-for", "60s"],
        false,
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "swept fleet-a\n"
    );
    assert_eq!(
        fs::read_to_string(kill_log).expect("read kill log"),
        "=fleet-a\n"
    );
}

#[test]
fn sweep_rechecks_attachment_before_killing() {
    let (dir, kill_log, path) = fake_seat();
    let output = run_fleet(
        &dir,
        &kill_log,
        &path,
        &["ssh", "sweep", "--idle-for=1m"],
        true,
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "skipped fleet-a (attached)\n"
    );
    assert!(!kill_log.exists(), "attached session must not be killed");
}

#[test]
fn sweep_dry_run_never_kills_sessions() {
    let (dir, kill_log, path) = fake_seat();
    let output = run_fleet(
        &dir,
        &kill_log,
        &path,
        &["ssh", "sweep", "--idle-for", "1h", "--dry-run"],
        false,
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "would sweep fleet-a\n"
    );
    assert!(!kill_log.exists(), "dry-run must not send a kill command");
}
