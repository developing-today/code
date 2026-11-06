#![cfg(unix)]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("fleet-proxy-ssh-{}-{suffix}", std::process::id()));
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
fn proxy_ssh_execs_system_ssh_with_fixed_argv_and_stdio() {
    let dir = TestDir::new();
    let fake_bin = dir.0.join("bin");
    fs::create_dir(&fake_bin).expect("create fake bin directory");
    let capture = dir.0.join("argv");
    let fake_ssh = fake_bin.join("ssh");
    fs::write(
        &fake_ssh,
        "#!/bin/sh\nprintf '%s\\000' \"$@\" > \"$FLEET_SSH_CAPTURE\"\ncat\nexit 37\n",
    )
    .expect("write fake ssh");
    let mut permissions = fs::metadata(&fake_ssh)
        .expect("fake ssh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_ssh, permissions).expect("make fake ssh executable");

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", fake_bin.display(), old_path.to_string_lossy());
    let remote_command = format!("echo; touch {}/injected", dir.0.display());
    let mut child = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args([
            "proxy",
            "ssh",
            "--",
            "-L",
            "127.0.0.1:2222:localhost:22",
            "-t",
            "bash",
            "-s",
            &remote_command,
        ])
        .env("PATH", path)
        .env("FLEET_SSH_CAPTURE", &capture)
        .env("FLEET_TARGET_HOST", "100.99.113.103")
        .env("HOME", dir.0.join("home with spaces"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start fleet child");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"stdin-payload")
        .expect("write to child stdin");
    let output = child.wait_with_output().expect("wait for fleet child");

    assert_eq!(
        output.status.code(),
        Some(37),
        "exec must preserve ssh exit status"
    );
    assert_eq!(output.stdout, b"stdin-payload", "exec must preserve stdio");
    assert!(
        !dir.0.join("injected").exists(),
        "remote command is passed as argv, not evaluated locally"
    );

    let captured = fs::read(capture).expect("read captured ssh argv");
    let argv: Vec<&str> = captured
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| std::str::from_utf8(argument).expect("UTF-8 argument"))
        .collect();
    let exe = env!("CARGO_BIN_EXE_fleet");
    let proxy =
        format!("ProxyCommand='{exe}' proxy connect --stdio --host '100.99.113.103' --port %p");
    assert!(argv.contains(&"HostName=100.99.113.103"));
    assert!(argv.contains(&"User=user"));
    assert!(argv.contains(&"StrictHostKeyChecking=yes"));
    assert!(argv.contains(&"UserKnownHostsFile=~/.ssh/known_hosts_laptop"));
    assert!(argv.contains(&"~/.ssh/id_ed25519"));
    assert!(argv.contains(&"ControlPath=~/.ssh/cm-%r@%h:%p"));
    assert!(argv.contains(&proxy.as_str()));
    assert!(argv.contains(&"127.0.0.1:2222:localhost:22"));
    let target = argv
        .iter()
        .position(|argument| *argument == "user@100.99.113.103")
        .expect("fixed target must be present");
    assert_eq!(
        &argv[target + 1..],
        &["bash", "-s", remote_command.as_str()]
    );
}
