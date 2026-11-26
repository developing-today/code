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

#[test]
fn sessions_lists_only_fleet_owned_tmux_sessions() {
    let dir = TestDir::new();
    let fake_bin = dir.0.join("bin");
    fs::create_dir(&fake_bin).expect("create fake bin directory");
    let fake_ssh = fake_bin.join("ssh");
    fs::write(
        &fake_ssh,
        "#!/bin/sh\nfor arg do last=$arg; done\ncase \"$last\" in\n  *'tmux list-sessions'*) printf 'fleet-a\\nwork\\nfleet-b\\n'; exit 0 ;;\n  *) echo \"unexpected remote command: $last\" >&2; exit 99 ;;\nesac\n",
    )
    .expect("write fake ssh");
    let mut permissions = fs::metadata(&fake_ssh)
        .expect("fake ssh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_ssh, permissions).expect("make fake ssh executable");

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", fake_bin.display(), old_path.to_string_lossy());
    let output = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(["ssh", "sessions"])
        .env("PATH", path)
        .env("HOME", &dir.0)
        .env("HTTPS_PROXY", "http://127.0.0.1:3130")
        .output()
        .expect("run fleet ssh sessions");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        "fleet-a\nfleet-b\n"
    );
}
